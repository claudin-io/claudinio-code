# Plan: Preencher todos os 16 locales vazios com traduções

## Context

O Claudinio Code tem um sistema i18n customizado (`src/lib/grill-me.ts`) com 18 locales. Apenas `en-US` e `pt-BR` estão populados (373 keys cada, paridade verificada por teste). Os outros 16 arquivos exportam dicts vazios (`{}`), caindo automaticamente para `en-US`.

**Objetivo**: Preencher todos os 16 locales com traduções completas (373 keys cada), usando o modelo Claude da própria API do Claudinio (mesmo endpoint usado pelo app).

**16 locales a preencher**:
`es-ES`, `fr-FR`, `de-DE`, `it-IT`, `ru-RU`, `tr-TR`, `ar-SA`, `hi-IN`, `bn-BD`, `ur-PK`, `zh-CN`, `ja-JP`, `ko-KR`, `vi-VN`, `id-ID`, `pt-PT`

## Solution Design

### Estratégia: Script Python de tradução em lote

Um script Python que:
1. Lê o arquivo `src/lib/locales/en-US.ts` e extrai as 373 keys com seus valores em inglês
2. Lê a config do app (`~/.config/claudinio-code/config.json`) para obter API key e base URL
3. Para cada um dos 16 locales, envia as 373 strings em batches para a API Claude (`/v1/messages`), pedindo tradução
4. Preserva rigorosamente placeholders como `{0}`, `{1}`, etc.
5. Gera os arquivos TypeScript no formato exato esperado
6. Atualiza o teste `src/lib/locales.test.ts` para verificar que os novos locales têm 373 keys (em vez de 0)

### Prompt de tradução

O prompt enviado ao modelo instrui:
- Traduzir do inglês para o idioma alvo
- Preservar EXATAMENTE placeholders `{0}`, `{1}`, etc.
- Preservar nomes próprios: "Claudinio Code", "Claudinio", "Monaco Editor", "CodeBERT", etc.
- Retornar JSON com formato `{"key": "translated value", ...}`
- Emojis (⚡, 💡) podem ser mantidos

### Edge cases
- Strings com `${0}` no original (ex: `"in ${0} · out ${1} · cache ${2}"`) — preservar
- Strings vazias (ex: `"chat.header.turn": ""`) — manter vazias
- Chaves que são iguais em qualquer idioma (ex: `"app.title": "Claudinio Code"`) — manter
- Temas: "Dracula", "Nord", "Catppuccin" — manter como nomes próprios
- `pt-PT` vs `pt-BR` — o pt-PT deve usar ortografia e expressões de Portugal

## Risks

- **Alto**: ~5.968 strings = muitas chamadas de API. Pode consumir bastante cota.
- **Médio**: Placeholders podem ser corrompidos se o modelo alucinar. O script deve validar cada resposta.
- **Médio**: Idiomas RTL (ar-SA, ur-PK) — traduções corretas mas sem alterações de layout (CSS não é escopo).
- **Baixo**: Idiomas com scripts não-latinos (zh-CN, ja-JP, ko-KR, ru-RU, hi-IN, bn-BD) — encoding UTF-8 garante compatibilidade.

## Non-goals

- NÃO alterar componentes ou adicionar suporte RTL no CSS
- NÃO adicionar novos locales além dos 16 existentes
- NÃO modificar o sistema i18n (`grill-me.ts`)
- NÃO traduzir strings que já estão em pt-BR
- NÃO alterar o arquivo en-US.ts

## Low-Level Design

### Architecture

O script Python `scripts/translate_locales.py` opera standalone (não integrado ao build do Tauri). Ele lê `en-US.ts` como source of truth, consulta a API do Claudinio, e escreve os 16 arquivos de locale + atualiza o teste.

```
┌──────────────────┐     ┌─────────────────────┐     ┌──────────────────────┐
│ en-US.ts (373    │────▶│ translate_locales.py │────▶│ 16 locale files      │
│ key-value pairs) │     │                     │     │ (es-ES.ts ... pt-PT) │
└──────────────────┘     │ ┌─────────────────┐ │     └──────────────────────┘
                         │ │ Claudinio API   │ │     ┌──────────────────────┐
                         │ │ /v1/messages    │ │────▶│ locales.test.ts      │
                         │ │ (batch translate)│ │     │ (updated assertions) │
                         │ └─────────────────┘ │     └──────────────────────┘
                         └─────────────────────┘
```

### Files to create/modify

| File | Action | Purpose |
|------|--------|---------|
| `scripts/translate_locales.py` | CREATE | Main translation script |
| `src/lib/locales/es-ES.ts` | OVERWRITE | Spanish translations |
| `src/lib/locales/fr-FR.ts` | OVERWRITE | French translations |
| `src/lib/locales/de-DE.ts` | OVERWRITE | German translations |
| `src/lib/locales/it-IT.ts` | OVERWRITE | Italian translations |
| `src/lib/locales/ru-RU.ts` | OVERWRITE | Russian translations |
| `src/lib/locales/tr-TR.ts` | OVERWRITE | Turkish translations |
| `src/lib/locales/ar-SA.ts` | OVERWRITE | Arabic translations |
| `src/lib/locales/hi-IN.ts` | OVERWRITE | Hindi translations |
| `src/lib/locales/bn-BD.ts` | OVERWRITE | Bengali translations |
| `src/lib/locales/ur-PK.ts` | OVERWRITE | Urdu translations |
| `src/lib/locales/zh-CN.ts` | OVERWRITE | Chinese Simplified translations |
| `src/lib/locales/ja-JP.ts` | OVERWRITE | Japanese translations |
| `src/lib/locales/ko-KR.ts` | OVERWRITE | Korean translations |
| `src/lib/locales/vi-VN.ts` | OVERWRITE | Vietnamese translations |
| `src/lib/locales/id-ID.ts` | OVERWRITE | Indonesian translations |
| `src/lib/locales/pt-PT.ts` | OVERWRITE | Portuguese (Portugal) translations |
| `src/lib/locales.test.ts` | MODIFY | Update "empty locales" test to verify 373 keys each |

### Data flow

1. **Extract**: Parse `en-US.ts` with regex to build `OrderedDict[str, str]` of 373 key-value pairs
2. **Load config**: Read `~/.config/claudinio-code/config.json` → extract `api_key`, `base_url`, `override_api_key`, `override_base_url`
3. **For each locale**: 
   a. Split 373 keys into batches of 30 (13 API calls per locale)
   b. For each batch, call Claudinio API with system prompt instructing translation to target language
   c. Parse JSON response, validate all keys present and placeholders intact
   d. Accumulate results
4. **Generate .ts file**: Write each locale file matching the exact template format with section comments
5. **Update test**: Replace the "empty locales" test assertion from `0 keys` to `373 keys`

### API call spec

**Endpoint**: `POST {effective_base_url}/v1/messages`
- `effective_base_url = override_base_url ?? base_url` (default: `https://api.claudin.io`)
- `effective_api_key = override_api_key ?? api_key`

**Headers**:
```
Content-Type: application/json
x-api-key: {effective_api_key}
anthropic-version: 2023-06-01
```

**Body** (non-streaming, `stream: false`):
```json
{
  "model": "claudinio",
  "max_tokens": 4096,
  "stream": false,
  "system": "You are a professional translator. Translate each English string to {target_language}. Preserve EXACTLY all placeholders like {0}, {1}, ${0}, etc. Do NOT translate proper names: Claudinio Code, Claudinio, Monaco Editor, CodeBERT, YOLO, MCP, JSON, API, IDE, LSP, FTS5, VS Code, Cursor, Anthropic, git, GitHub, ssh, bash, Brain (mode), Builder (mode), Golden loop, Dracula, Nord, Catppuccin, Monokai, One Dark, Tokyo Night, Gruvbox, Rose Pine, Everforest, Solarized. Keep emojis (⚡, 💡). Return ONLY valid JSON: {\"key\": \"translated string\", ...}",
  "messages": [
    {
      "role": "user",
      "content": "Translate these English UI strings to {target_language_name}:\n\n{json_batch}"
    }
  ]
}
```

### Script structure (translate_locales.py)

```python
#!/usr/bin/env python3
"""Translate en-US locale dict to all 16 target locales via Claudinio API."""

import json, os, re, sys, time
from pathlib import Path
from collections import OrderedDict
import httpx  # pip install httpx

PROJECT_ROOT = Path(__file__).resolve().parent.parent
LOCALES_DIR = PROJECT_ROOT / "src" / "lib" / "locales"
EN_US_PATH = LOCALES_DIR / "en-US.ts"
CONFIG_PATH = Path.home() / ".config" / "claudinio-code" / "config.json"
BATCH_SIZE = 30

TARGET_LOCALES = {
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

# Section comments from en-US.ts (extracted during parse)
SECTION_COMMENTS = [
    "// ── App ─────",
    "// ── EmptyState ─────",
    "// ── ChatPanel - Header ─────",
    "// ── ChatPanel - Git ─────",
    "// ── Network activity indicator ─────",
    "// ── Askpass (git/ssh credential prompts) ─────",
    "// ── CommitPush Modal ─────",
    "// ── ChatPanel - Status ─────",
    "// ── ChatPanel - Messages ─────",
    "// ── ChatPanel - Auth Card ─────",
    "// ── ChatPanel - Phases ─────",
    "// ── ChatPanel - Timeline ─────",
    "// ── ChatPanel - Subagent ─────",
    "// ── ChatPanel - Input ─────",
    "// ── ChatPanel - Approval ─────",
    "// ── ChatPanel - Question ─────",
    "// ── ChatPanel - Context Footer ─────",
    "// ── ChatPanel - Compaction ─────",
    "// ── ChatPanel - Archived ─────",
    "// ── ChatPanel - Drop overlay ─────",
    "// ── Mention popovers ─────",
    "// ── Text Editor Modal ─────",
    "// ── Prompt Enhancement ─────",
    "// ── File Editor Modal ─────",
    "// ── Tasks Panel ─────",
    "// ── Context Warning ─────",
    "// ── Session mode (Brain / Builder) ─────",
    "// ── Onboarding Wizard ─────",
    "// ── Theme ─────",
    "// ── Content Viewer ─────",
    "// ── Context Menu ─────",
]


def extract_en_us_dict() -> OrderedDict:
    """Parse en-US.ts and return ordered dict of key -> english value."""
    ...

def load_config() -> dict:
    """Load Claudinio config.json, return dict with api_key, base_url, etc."""
    ...

def translate_batch(client: httpx.Client, batch: dict, locale_code: str, 
                   locale_name: str, base_url: str, api_key: str) -> dict:
    """Send one batch of keys to Claudinio API, return translated dict."""
    ...

def generate_locale_file(locale_code: str, translations: OrderedDict, 
                         section_map: dict) -> str:
    """Generate the full .ts file content matching en-US format."""
    ...

def update_test_file():
    """Update locales.test.ts: change empty-locale assertions from 0 to 373 keys."""
    ...

def main():
    ...
```

### locales.test.ts changes

Current assertion (line ~106):
```typescript
expect(Object.keys(dict).length, `${code} should have 0 keys`).toBe(0);
```

New assertion:
```typescript
expect(Object.keys(dict).length, `${code} should have 373 keys`).toBe(373);
```

Additionally, add assertions that each locale has all the same keys as en-US (same as the pt-BR parity test).

### Verification

1. Run `python3 scripts/translate_locales.py` — generates 16 .ts files
2. Run `npx vitest run src/lib/locales.test.ts` — all tests pass (373 keys each, no key mismatches)
3. Run `npx tsc --noEmit` — no TypeScript errors in locale files
4. Manual spot-check: open `es-ES.ts`, verify section comments, key structure, placeholder preservation

## Tasks summary

1. Create `scripts/translate_locales.py` — Python script with extraction, API calling, file generation, and test update logic
2. Run the script to translate all 16 locales via Claudinio API
3. Verify: run `vitest src/lib/locales.test.ts` and `tsc --noEmit`
4. Fix any translation issues found during verification (missing keys, broken placeholders)