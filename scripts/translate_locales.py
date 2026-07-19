#!/usr/bin/env python3
"""
translate_locales.py — Translate en-US locale strings into 16 target locales
using the Claudinio API. Generates .ts locale files and updates the test suite.

Usage:
    python scripts/translate_locales.py
"""

import json
import re
import os
import sys
import time
import argparse
import traceback
from pathlib import Path
from collections import OrderedDict

import httpx

# ── Constants ───────────────────────────────────────────────────────────────

ROOT = Path(__file__).resolve().parent.parent
EN_US_PATH = ROOT / "src" / "lib" / "locales" / "en-US.ts"
LOCALES_DIR = ROOT / "src" / "lib" / "locales"
TEST_PATH = ROOT / "src" / "lib" / "locales.test.ts"

TARGET_LOCALES: dict[str, str] = {
    "es-ES": "Spanish (Spain)",
    "fr-FR": "French (France)",
    "de-DE": "German (Germany)",
    "it-IT": "Italian (Italy)",
    "ru-RU": "Russian (Russia)",
    "tr-TR": "Turkish (Turkey)",
    "ar-SA": "Arabic (Saudi Arabia)",
    "hi-IN": "Hindi (India)",
    "bn-BD": "Bengali (Bangladesh)",
    "ur-PK": "Urdu (Pakistan)",
    "zh-CN": "Chinese Simplified (China)",
    "ja-JP": "Japanese (Japan)",
    "ko-KR": "Korean (South Korea)",
    "vi-VN": "Vietnamese (Vietnam)",
    "id-ID": "Indonesian (Indonesia)",
    "pt-PT": "Portuguese (Portugal)",
}

BATCH_SIZE = 30
RETRY_COUNT = 3
RETRY_BACKOFF = 2.0  # seconds
BATCH_DELAY = 0.2  # seconds between batches

DEFAULT_API_URL = "https://api.claudin.io/v1/chat/completions"

SYSTEM_PROMPT_TEMPLATE = (
    "You are a professional translator. Translate each English string to {language_name}. "
    "Preserve EXACTLY all placeholders like {{0}}, {{1}}, ${{0}}, etc. "
    "Do NOT translate proper names: Claudinio Code, Claudinio, Monaco Editor, CodeBERT, YOLO, "
    "MCP, JSON, API, IDE, LSP, FTS5, VS Code, Cursor, Anthropic, git, GitHub, ssh, bash, "
    "Brain (mode), Builder (mode), Golden loop, Dracula, Nord, Catppuccin, Monokai, One Dark, "
    "Tokyo Night, Gruvbox, Rose Pine, Everforest, Solarized, Claudinio Light, Claudinio Sepia. "
    "Keep emojis (⚡, 💡). Return ONLY valid JSON: {{\"key\": \"translated string\", ...}}"
)


# ── Section/Key Parsing ─────────────────────────────────────────────────────

# Regex for section comments:  // ── Section Name ──────...
SECTION_RE = re.compile(r"^\s*//\s*──\s*(.+?)\s*──+")

# Combined regex for string values with escaped content or empty strings
KEY_VAL_RE = re.compile(
    r'^\s*"([^"]+)"\s*:\s*("(?:[^"\\]|\\.)*")\s*,?\s*$'
)


def parse_en_us(path: Path) -> tuple[OrderedDict, list[tuple[str, int]]]:
    """
    Parse en-US.ts and return:
      - OrderedDict of key → English value
      - List of (section_name, key_index) pairs indicating where section
        comments should appear in the output.
    """
    lines = path.read_text(encoding="utf-8").splitlines()

    ordered_keys: OrderedDict[str, str] = OrderedDict()
    sections: list[tuple[str, int]] = []  # (section_name, index_before_this_key)

    for line in lines:
        # Check for section comment
        sec_match = SECTION_RE.match(line)
        if sec_match:
            section_name = sec_match.group(1).strip()
            sections.append((section_name, len(ordered_keys)))
            continue

        # Check for key-value pair
        kv_match = KEY_VAL_RE.match(line)
        if kv_match:
            key = kv_match.group(1)
            raw_value = kv_match.group(2)
            # Parse the JSON string literal (handles escape sequences properly)
            try:
                value = json.loads(raw_value)
            except json.JSONDecodeError:
                print(f"  WARNING: Could not parse value for key '{key}', skipping")
                continue
            ordered_keys[key] = value

    return ordered_keys, sections


# ── Config Loading ──────────────────────────────────────────────────────────


def load_config() -> dict:
    """Load config from ~/.config/claudinio-code/config.json or fallback."""
    config_paths = [
        Path.home() / ".config" / "claudinio-code" / "config.json",
        Path.home() / ".config" / "claudinio" / "config.json",
    ]

    for cp in config_paths:
        if cp.exists():
            try:
                config = json.loads(cp.read_text(encoding="utf-8"))
                print(f"Loaded config from {cp}")
                return config
            except (json.JSONDecodeError, OSError) as exc:
                print(f"WARNING: Failed to parse {cp}: {exc}")

    print("ERROR: No config file found at:")
    for cp in config_paths:
        print(f"  {cp}")
    sys.exit(1)


def get_effective_url(config: dict, cli_url: str | None = None) -> str:
    """Determine the API URL. Priority: CLI arg > env var > config > default."""
    if cli_url:
        print(f"Using --api-url override -> {cli_url}")
        return cli_url
    env_url = os.environ.get("CLAUDINIO_API_URL", "").strip()
    if env_url:
        print(f"Using CLAUDINIO_API_URL env var -> {env_url}")
        return env_url
    # Check config for override_base_url or api_url (for backward compat)
    override_url = config.get("override_base_url", "").strip()
    if override_url:
        url = override_url.rstrip("/") + "/v1/chat/completions"
        print(f"Using override_base_url from config -> {url}")
        return url
    api_url = config.get("api_url", "").strip()
    if api_url:
        url = api_url.rstrip("/") + "/chat/completions"
        print(f"Using api_url from config -> {url}")
        return url
    print(f"Using default API URL -> {DEFAULT_API_URL}")
    return DEFAULT_API_URL


def get_effective_key(config: dict, cli_key: str | None = None) -> str:
    """Resolve API key. Priority: CLI arg > env var > config."""
    if cli_key:
        print("Using --api-key override")
        return cli_key
    env_key = os.environ.get("CLAUDINIO_API_KEY", "").strip()
    if env_key:
        print("Using CLAUDINIO_API_KEY env var")
        return env_key
    # Check config for override_api_key first, then api_key
    key = config.get("override_api_key", "").strip()
    if key:
        print("Using override_api_key from config")
        return key
    key = config.get("api_key", "").strip()
    if key:
        print("Using api_key from config")
        return key
    print("ERROR: No API key found. Provide via --api-key, CLAUDINIO_API_KEY env var, or config file.")
    sys.exit(1)


# ── API Call ────────────────────────────────────────────────────────────────


def call_api(
    client: httpx.Client,
    url: str,
    api_key: str,
    system_prompt: str,
    user_message: str,
) -> dict[str, str]:
    """
    POST to the OpenAI-compatible API, parse response, return key->translation dict.
    Retries up to RETRY_COUNT times on failure.
    """
    headers = {
        "Content-Type": "application/json",
        "Authorization": f"Bearer {api_key}",
    }

    body = {
        "model": "claudinio",
        "max_tokens": 4096,
        "temperature": 0.2,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_message},
        ],
    }

    last_error = None
    for attempt in range(1, RETRY_COUNT + 1):
        try:
            resp = client.post(url, json=body, headers=headers, timeout=120)
            resp.raise_for_status()
            data = resp.json()

            # Parse: response['choices'][0]['message']['content'] -> JSON string -> dict
            choices = data.get("choices", [])
            if not choices:
                raise ValueError("API response has empty 'choices' array")

            raw_text = choices[0].get("message", {}).get("content", "")
            if not raw_text:
                raise ValueError("API response has empty content in first choice")

            # Strip markdown code fences if present
            raw_text = raw_text.strip()
            if raw_text.startswith("```"):
                # Remove opening ```json or ``` and closing ```
                raw_text = re.sub(r"^```(?:json)?\s*", "", raw_text)
                raw_text = re.sub(r"\s*```$", "", raw_text)
                raw_text = raw_text.strip()

            result = json.loads(raw_text)
            if not isinstance(result, dict):
                raise ValueError(f"Expected JSON object, got {type(result).__name__}")

            return result

        except (httpx.HTTPError, json.JSONDecodeError, ValueError, KeyError) as exc:
            last_error = exc
            if attempt < RETRY_COUNT:
                wait = RETRY_BACKOFF * attempt
                print(f"    Attempt {attempt}/{RETRY_COUNT} failed: {exc}. Retrying in {wait}s...")
                time.sleep(wait)
            else:
                print(f"    All {RETRY_COUNT} attempts failed. Last error: {exc}")

    raise RuntimeError(f"API call failed after {RETRY_COUNT} attempts: {last_error}")


# ── File Generation ─────────────────────────────────────────────────────────


def generate_locale_file(
    code: str,
    translations: dict[str, str],
    sections: list[tuple[str, int]],
    en_keys: list[str],
    partial: bool = False,
    error_note: str = "",
) -> None:
    """
    Write a locale .ts file with translations in the same order as en-US.
    Sections are inserted at the boundaries tracked during parsing.
    """
    output_path = LOCALES_DIR / f"{code}.ts"

    # Build section lookup: key_index -> section_name (only first time seen)
    section_by_index: dict[int, str] = {}
    for sec_name, idx in sections:
        if idx not in section_by_index:
            section_by_index[idx] = sec_name

    lines_out: list[str] = []
    lines_out.append('import type { LocaleDict } from "../grill-me";')
    lines_out.append("")

    if partial:
        lines_out.append("// ⚠ PARTIAL TRANSLATION — API errors during generation")
        safe_note = error_note.replace('\n', ' | ').replace('\r', '')[:100]
        lines_out.append(f"// {safe_note}")
        lines_out.append("")

    lines_out.append("const dict: LocaleDict = {")

    key_index = 0
    for key in en_keys:
        # Insert section comment if one belongs here
        if key_index in section_by_index:
            sec = section_by_index[key_index]
            dashes_needed = 68 - len(sec) - 7  # "  // -- " + sec + " --"
            if dashes_needed < 2:
                dashes_needed = 2
            lines_out.append(f"  // ── {sec} {'─' * dashes_needed}")

        value = translations.get(key)
        if value is None:
            print(f"    WARNING: key '{key}' missing from translation, using empty fallback")
            lines_out.append(f'  "{key}": "",')
        else:
            escaped = json.dumps(value, ensure_ascii=False)
            lines_out.append(f'  "{key}": {escaped},')

        key_index += 1

    lines_out.append("};")
    lines_out.append("")
    lines_out.append("export default dict;")
    lines_out.append("")

    output_path.write_text("\n".join(lines_out), encoding="utf-8")
    print(f"    Wrote {output_path} ({len(translations)} keys)")


# ── Test File Update ────────────────────────────────────────────────────────


def update_test_file(en_key_count: int) -> None:
    """
    Update locales.test.ts:
    1. Replace the empty-locale assertion with the correct key count.
    2. Add a key-parity test for all 16 locales after the empty-locales test.
    """
    content = TEST_PATH.read_text(encoding="utf-8")

    # Replace: expect(Object.keys(dict).length, `${code} should have 0 keys`).toBe(0);
    old_line = (
        'expect(Object.keys(dict).length, `${code} should have 0 keys`).toBe(0);'
    )
    new_line = (
        f'expect(Object.keys(dict).length, `${{code}} should have {en_key_count} keys`).toBe(enKeys.length);'
    )

    if old_line not in content:
        print("WARNING: Could not find the empty-locale assertion line to replace.")
        print(f"  Expected: {old_line}")
    else:
        content = content.replace(old_line, new_line)
        print(f"  Replaced empty-locale assertion with {en_key_count}-key check.")

    # Add parity test block for all 16 locales before the resolveLocale describe
    marker = '  describe("resolveLocale"'
    if marker in content:
        parity_test = f"""  describe("key parity — all 16 translated locales", () => {{
    const enKeys = Object.keys(enUS);

    const allLocales: Record<string, Record<string, unknown>> = {{
      "es-ES": esES, "fr-FR": frFR, "de-DE": deDE, "it-IT": itIT,
      "ru-RU": ruRU, "tr-TR": trTR, "ar-SA": arSA, "hi-IN": hiIN,
      "bn-BD": bnBD, "ur-PK": urPK, "zh-CN": zhCN, "ja-JP": jaJP,
      "ko-KR": koKR, "vi-VN": viVN, "id-ID": idID, "pt-PT": ptPT,
    }};

    for (const [code, dict] of Object.entries(allLocales)) {{
      it(`${{code}} has the same keys as en-US`, () => {{
        expect(Object.keys(dict).length, `${{code}} should have ${{enKeys.length}} keys`).toBe(enKeys.length);
        for (const key of enKeys) {{
          expect(dict, `${{code}} missing key "${{key}}"`).toHaveProperty(key);
        }}
      }});
    }}
  }});

  """
        content = content.replace(marker, parity_test + marker)
        print("  Added key-parity test block for all 16 locales.")
    else:
        print("WARNING: Could not find insertion point for parity test block.")

    TEST_PATH.write_text(content, encoding="utf-8")
    print(f"  Updated {TEST_PATH}")


# ── Main Flow ───────────────────────────────────────────────────────────────


def main() -> None:
    print("=" * 70)
    print("  Claudinio Code — Locale Translation Script")
    print("=" * 70)

    # 0. Parse CLI args
    parser = argparse.ArgumentParser(description="Translate en-US locale to 16 target locales")
    parser.add_argument("--api-key", default=None, help="API key (overrides CLAUDINIO_API_KEY env var and config file)")
    parser.add_argument("--api-url", default=None, help="API URL (overrides CLAUDINIO_API_URL env var and config file)")
    parser.add_argument("--skip-existing", action="store_true", help="Skip locales whose .ts file already has >= 370 translated keys")
    parser.add_argument("--locales", default=None, help="Comma-separated list of locale codes to translate (overrides TARGET_LOCALES order)")
    args = parser.parse_args()

    # 1. Parse en-US.ts
    print("\n[1/5] Parsing en-US.ts...")
    if not EN_US_PATH.exists():
        print(f"ERROR: {EN_US_PATH} not found")
        sys.exit(1)

    en_keys_ordered, sections = parse_en_us(EN_US_PATH)
    en_key_count = len(en_keys_ordered)
    section_count = len(sections)
    print(f"  Found {en_key_count} keys across {section_count} sections")

    if en_key_count == 0:
        print("ERROR: No keys parsed from en-US.ts")
        sys.exit(1)

    en_keys_list = list(en_keys_ordered.keys())
    target_locales = dict(TARGET_LOCALES)

    # --locales filtering
    if args.locales:
        requested = [c.strip() for c in args.locales.split(",") if c.strip()]
        invalid = [c for c in requested if c not in TARGET_LOCALES]
        if invalid:
            print(f"  WARNING: Unknown locale codes in --locales: {invalid}")
        target_locales = {c: TARGET_LOCALES[c] for c in requested if c in TARGET_LOCALES}
        if not target_locales:
            print("ERROR: No valid locales after --locales filtering")
            sys.exit(1)
        print(f"  Filtered to {len(target_locales)} locale(s): {', '.join(target_locales)}")

    # --skip-existing filtering
    if args.skip_existing:
        skipped: list[str] = []
        remaining: dict[str, str] = {}
        MIN_KEYS = 370
        for code, name in target_locales.items():
            ts_path = LOCALES_DIR / f"{code}.ts"
            if ts_path.exists():
                # Count non-empty translated keys in the file
                content = ts_path.read_text(encoding="utf-8")
                key_count = len([l for l in content.splitlines() if KEY_VAL_RE.match(l)])
                if key_count >= MIN_KEYS:
                    print(f"  Skipping {code} ({name}) — already has {key_count} keys (>= {MIN_KEYS})")
                    skipped.append(code)
                    continue
                else:
                    print(f"  NOT skipping {code} ({name}) — only {key_count} keys (< {MIN_KEYS})")
            else:
                print(f"  NOT skipping {code} ({name}) — file not found")
            remaining[code] = name
        target_locales = remaining
        if skipped:
            print(f"  Skipped {len(skipped)} locale(s): {', '.join(skipped)}")
        if not target_locales:
            print("All locales already have >= 370 keys. Nothing to do.")
            sys.exit(0)

    # 2. Load config
    print("\n[2/5] Loading config...")
    config = load_config()
    effective_url = get_effective_url(config, args.api_url)
    effective_key = get_effective_key(config, args.api_key)
    if len(effective_key) > 12:
        print(f"  API key: {effective_key[:8]}...{effective_key[-4:]}")
    else:
        print(f"  API key: {effective_key}")

    # 3. Prepare HTTP client
    print("\n[3/5] Translating locales...")
    client = httpx.Client(http2=False)

    total_locales = len(target_locales)
    locale_errors: list[str] = []

    for idx, (code, language_name) in enumerate(target_locales.items(), start=1):
        print(f"\n{'─' * 50}")
        print(f"  [{idx}/{total_locales}] Locale {code} ({language_name})")

        system_prompt = SYSTEM_PROMPT_TEMPLATE.format(language_name=language_name)

        # Split into batches
        batches: list[dict[str, str]] = []
        for i in range(0, en_key_count, BATCH_SIZE):
            batch_keys = en_keys_list[i : i + BATCH_SIZE]
            batch = OrderedDict((k, en_keys_ordered[k]) for k in batch_keys)
            batches.append(batch)

        total_batches = len(batches)
        all_translations: dict[str, str] = {}

        partial_failure = False
        failure_note = ""

        for bi, batch in enumerate(batches, start=1):
            user_msg = (
                f"Translate these English UI strings to {language_name}:\n\n"
                + json.dumps(batch, ensure_ascii=False, indent=2)
            )

            try:
                batch_result = call_api(
                    client, effective_url, effective_key, system_prompt, user_msg
                )
                all_translations.update(batch_result)
                print(f"    batch {bi}/{total_batches} \u2713 ({len(batch_result)} keys)")
            except Exception as exc:
                print(f"    batch {bi}/{total_batches} \u2717 FAILED: {exc}")
                traceback.print_exc()
                partial_failure = True
                # Sanitize: strip newlines, truncate to 100 chars for a safe comment line
                note_raw = str(exc).replace('\n', ' | ').replace('\r', '')
                if len(note_raw) > 100:
                    note_raw = note_raw[:97] + '...'
                failure_note = f"Batch {bi}/{total_batches} failed: {note_raw}"

            if bi < total_batches:
                time.sleep(BATCH_DELAY)

        # Validate
        received = len(all_translations)
        missing = sorted(set(en_keys_list) - set(all_translations.keys()))
        extra = sorted(set(all_translations.keys()) - set(en_keys_list))

        if missing:
            preview = missing[:5]
            print(f"    \u26a0 {len(missing)} missing keys: {preview}{'...' if len(missing) > 5 else ''}")
        if extra:
            preview = extra[:5]
            print(f"    \u26a0 {len(extra)} extra keys: {preview}{'...' if len(extra) > 5 else ''}")
        print(f"    Total: {received}/{en_key_count} keys received")

        if received == 0:
            print(f"    ERROR: No translations received for {code} — skipping file generation")
            locale_errors.append(f"{code}: all batches failed")
            continue

        # 4. Generate .ts file
        generate_locale_file(
            code=code,
            translations=all_translations,
            sections=sections,
            en_keys=en_keys_list,
            partial=partial_failure,
            error_note=failure_note,
        )

        if partial_failure:
            locale_errors.append(f"{code}: partial ({received}/{en_key_count} keys, {len(missing)} missing)")

    client.close()

    # 5. Update test file
    print(f"\n[4/5] Updating locales.test.ts...")
    update_test_file(en_key_count)

    # Summary
    print(f"\n[5/5] {'=' * 50}")
    print(f"  Summary")
    print(f"  {'─' * 40}")
    print(f"  Locales processed: {total_locales}")
    if locale_errors:
        print(f"  Errors ({len(locale_errors)}):")
        for err in locale_errors:
            print(f"    - {err}")
    else:
        print(f"  All {total_locales} locales translated successfully.")

    print(f"\n  Done. {total_locales} locales written.")
    print(f"  Run: npx vitest run src/lib/locales.test.ts")


if __name__ == "__main__":
    main()
