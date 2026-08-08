#!/usr/bin/env python3
"""Compute the pinned Chromium table for src-tauri/src/browser/provision.rs.

Chrome for Testing publishes immutable per-version download URLs but no
checksums, so we hash the artifacts ourselves and commit the result — the same
arrangement as the embedding model in fetch_embedding_model.py.

    python3 scripts/pin_chromium.py              # latest Stable
    python3 scripts/pin_chromium.py 141.0.7390.54

Prints a Rust literal to paste into provision.rs. Streams each artifact and
hashes it without keeping it on disk (~170 MB per platform, 4 platforms).
Stdlib only.

After bumping the pin, re-run the golden-pixel screenshot test: the exact
semantics of Page.captureScreenshot `clip` + `captureBeyondViewport` have
changed between Chrome versions before.
"""

import hashlib
import json
import sys
import urllib.request

LAST_KNOWN = (
    "https://googlechromelabs.github.io/chrome-for-testing/"
    "last-known-good-versions-with-downloads.json"
)
ARTIFACT = (
    "https://storage.googleapis.com/chrome-for-testing-public/"
    "{version}/{platform}/chrome-{platform}.zip"
)

# platform -> path of the executable inside the zip.
PLATFORMS = {
    "mac-arm64": (
        "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/"
        "Google Chrome for Testing"
    ),
    "mac-x64": (
        "chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/"
        "Google Chrome for Testing"
    ),
    "win64": "chrome-win64/chrome.exe",
    "linux64": "chrome-linux64/chrome",
}


def resolve_stable_version() -> str:
    with urllib.request.urlopen(LAST_KNOWN, timeout=60) as r:
        data = json.load(r)
    return data["channels"]["Stable"]["version"]


def hash_artifact(version: str, platform: str) -> tuple[str, int]:
    url = ARTIFACT.format(version=version, platform=platform)
    digest = hashlib.sha256()
    size = 0
    with urllib.request.urlopen(url, timeout=300) as r:
        while chunk := r.read(1 << 20):
            digest.update(chunk)
            size += len(chunk)
            print(f"  {platform}: {size / 1e6:7.1f} MB", end="\r", file=sys.stderr)
    print(f"  {platform}: {size / 1e6:7.1f} MB  done", file=sys.stderr)
    return digest.hexdigest(), size


def main() -> int:
    version = sys.argv[1] if len(sys.argv) > 1 else resolve_stable_version()
    print(f"Chrome for Testing {version}", file=sys.stderr)

    rows = []
    for platform, exe in PLATFORMS.items():
        sha, size = hash_artifact(version, platform)
        rows.append((platform, sha, size, exe))

    print()
    print(f'pub const CHROMIUM_VERSION: &str = "{version}";')
    print()
    print("pub const CHROMIUM_BUILDS: &[ChromiumBuild] = &[")
    for platform, sha, size, exe in rows:
        print("    ChromiumBuild {")
        print(f'        platform: "{platform}",')
        print(f'        sha256: "{sha}",')
        print(f"        size: {size},")
        print(f'        exe_in_zip: "{exe}",')
        print("    },")
    print("];")
    return 0


if __name__ == "__main__":
    sys.exit(main())
