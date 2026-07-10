# Plan: Easter Egg "iddqd" — Override de URL Anthropic e API Key no Settings

## Context / Problem Statement

O usuário quer um Easter egg no painel de settings: ao digitar a sequência "iddqd" (referência ao cheat code de DOOM) no modal de settings, dois novos campos ocultos são revelados:

1. **Anthropic URL** — override da `base_url` (default: `https://api.claudin.io`)
2. **API Key Override** — override da `api_key`

Estes campos, quando preenchidos, fazem override do sign-in/auth para **apenas** as chamadas de inferência ao LLM (`stream_message`, `classify_turn_completion`, `one_shot`). O OAuth sign-in, websearch, e `list_models` continuam usando a config normal.

### Comportamento acordado:
- **Persistência**: os valores de override são salvos no `config.json` e sobrevivem a reinícios do app
- **Escopo do override**: apenas chamadas de inferência LLM — OAuth, websearch, e `list_models` NÃO usam override
- **Visibilidade**: o Easter egg precisa ser reativado a cada vez que o modal abre (digitar "iddqd" novamente); uma vez revelados, os campos ficam visíveis até o modal fechar

## Goal (Definition of Done)

1. Dois novos campos no `AgentConfig`: `override_base_url: Option<String>` e `override_api_key: Option<String>`
2. Backend `set_config` aceita e persiste os novos campos
3. Backend `get_config` retorna os novos campos
4. `stream_message`, `classify_turn_completion`, `one_shot` usam os overrides quando presentes (fallback para config normal)
5. `list_models`, websearch, e OAuth NÃO são afetados
6. Frontend: detector de keystrokes "iddqd" no modal de settings
7. Frontend: dois novos inputs (URL + API Key) que aparecem após o Easter egg e persistem seus valores

## Key Findings (Prova Real)

| Finding | Source |
|---------|--------|
| `AgentConfig` struct definida em `src-tauri/src/agent/provider.rs:41-85`, com `base_url: String` (default `https://api.claudin.io`) e `api_key: String` | `provider.rs` lines 41-85 |
| `stream_message` em `provider.rs:570` usa `config.base_url` (linha 597) e `config.api_key` (linha 602) | `provider.rs:597,602` |
| `classify_turn_completion` em `provider.rs:442` usa `config.base_url` (linha 469) e `config.api_key` (linha 473) | `provider.rs:469,473` |
| `one_shot` em `provider.rs:511` usa `config.base_url` (linha 534) e `config.api_key` (linha 538) | `provider.rs:534,538` |
| `list_models` em `commands/agent.rs:536` usa `cfg.base_url` e `cfg.api_key` separadamente — NÃO deve usar override | `commands/agent.rs:536-543` |
| `web_search` em `tools/web_search.rs:54` usa `config.api_key` — NÃO deve usar override | `web_search.rs:54` |
| `set_config` em `commands/agent.rs:455-498` — `SetConfigArgs` struct linhas 439-450 | `commands/agent.rs:439-498` |
| `get_config` em `commands/agent.rs:499-536` — retorna JSON com `baseUrl`, `hasApiKey`, etc. | `commands/agent.rs:499-536` |
| Settings modal inline em `App.tsx:422-690`, componente `Show when={showConfig()}` | `App.tsx` lines 422-690 |
| Frontend IPC interfaces em `src/lib/ipc.ts`: `AgentConfig` (linhas 55-67), `SetConfigArgs` (linhas 69-80) | `ipc.ts:55-80` |
| `openConfig()` em `App.tsx:157-190` popula signals a partir de `getConfig()` | `App.tsx:157-190` |
| `saveConfig()` em `App.tsx:192-235` chama `setConfig()` com os valores dos signals | `App.tsx:192-235` |

## Changes (Steps)

### 1. Rust: Adicionar campos de override ao `AgentConfig` (`src-tauri/src/agent/provider.rs`)

**Target:** `src-tauri/src/agent/provider.rs`, struct `AgentConfig` (após linha ~85, antes do `install_fallback_seed`)

**Mutation:** Adicionar dois campos:
```rust
/// Override base URL for LLM inference. When set, used instead of `base_url`
/// for /v1/messages calls (stream_message, classify_turn_completion, one_shot).
/// Does NOT affect login, websearch, or list_models.
#[serde(default)]
pub override_base_url: Option<String>,
/// Override API key for LLM inference. When set, used instead of `api_key`
/// for /v1/messages calls. Does NOT affect login, websearch, or list_models.
#[serde(default)]
pub override_api_key: Option<String>,
```

**Why:** Persistir os valores de override no config.json

**Constraints:** Campos são `Option<String>` com `#[serde(default)]` para backward compat com configs existentes. Adicionar defaults `None` no `impl Default`.

### 2. Rust: Modificar `stream_message` para usar overrides (`src-tauri/src/agent/provider.rs`)

**Target:** `stream_message` function (~linha 570)

**Mutation:** Substituir `&config.base_url` e `&config.api_key` por:
```rust
let effective_base_url = config.override_base_url.as_deref().unwrap_or(&config.base_url);
let effective_api_key = config.override_api_key.as_deref().unwrap_or(&config.api_key);
```
E usar `effective_base_url` e `effective_api_key` no lugar de `config.base_url` e `config.api_key`.

**Why:** Apenas chamadas LLM usam o override

### 3. Rust: Modificar `classify_turn_completion` para usar overrides (`src-tauri/src/agent/provider.rs`)

**Target:** `classify_turn_completion` function (~linha 442)

**Mutation:** Mesmo padrão — extrair `effective_base_url` e `effective_api_key` dos overrides.

### 4. Rust: Modificar `one_shot` para usar overrides (`src-tauri/src/agent/provider.rs`)

**Target:** `one_shot` function (~linha 511)

**Mutation:** Mesmo padrão.

### 5. Rust: Atualizar `SetConfigArgs` e `set_config` (`src-tauri/src/commands/agent.rs`)

**Target:** `SetConfigArgs` struct (linhas 439-450) e `set_config` function (linhas 455-498)

**Mutation:** Adicionar a `SetConfigArgs`:
```rust
pub override_base_url: Option<String>,
pub override_api_key: Option<String>,
```
E em `set_config`:
```rust
if let Some(url) = args.override_base_url {
    cfg.override_base_url = if url.is_empty() { None } else { Some(url) };
}
if let Some(key) = args.override_api_key {
    cfg.override_api_key = if key.is_empty() { None } else { Some(key) };
}
```

**Why:** Permitir que o frontend salve os valores de override

### 6. Rust: Atualizar `get_config` para retornar overrides (`src-tauri/src/commands/agent.rs`)

**Target:** `get_config` function (linhas 499-536)

**Mutation:** Adicionar ao JSON de resposta:
```rust
"overrideBaseUrl": cfg.override_base_url,
"overrideApiKey": cfg.override_api_key,
```

### 7. Frontend: Atualizar interfaces IPC (`src/lib/ipc.ts`)

**Target:** `AgentConfig` interface (linhas 55-67) e `SetConfigArgs` interface (linhas 69-80)

**Mutation:** Adicionar:
- `AgentConfig`: `overrideBaseUrl?: string | null; overrideApiKey?: string | null;`
- `SetConfigArgs`: `overrideBaseUrl?: string; overrideApiKey?: string;`

### 8. Frontend: Adicionar detector de "iddqd" + campos ocultos (`src/App.tsx`)

**Target:** `App` component, dentro da função `App()` e dentro do modal de settings

**Mutation:**
1. Adicionar signals (após linha ~79):
   ```typescript
   const [easterEggActive, setEasterEggActive] = createSignal(false);
   const [keystrokeBuf, setKeystrokeBuf] = createSignal("");
   const [configOverrideBaseUrl, setConfigOverrideBaseUrl] = createSignal("");
   const [configOverrideApiKey, setConfigOverrideApiKey] = createSignal("");
   ```

2. No `openConfig()`: resetar `setEasterEggActive(false)` e `setKeystrokeBuf("")`, popular `configOverrideBaseUrl` e `configOverrideApiKey` de `cfg.overrideBaseUrl` / `cfg.overrideApiKey`.

3. No modal div (após o `<div class="w-[400px]...">`), adicionar `onKeyDown` handler:
   ```typescript
   onKeyDown={(e) => {
     if (easterEggActive()) return;
     // Only track when no input/select/textarea is focused
     if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement || e.target instanceof HTMLTextAreaElement) return;
     const next = keystrokeBuf() + e.key;
     if ("iddqd".startsWith(next)) {
       setKeystrokeBuf(next);
       if (next === "iddqd") setEasterEggActive(true);
     } else if ("iddqd".startsWith(e.key)) {
       setKeystrokeBuf(e.key);
     } else {
       setKeystrokeBuf("");
     }
   }}
   ```

4. Adicionar dois campos (após a seção de API Key / Account, antes do `<hr>` do plan save path):
   ```tsx
   <Show when={easterEggActive()}>
     <hr class="mb-4 border-border-subtle" />
     <label class="mb-1 block text-xs text-ink-muted">{t("app.config.overrideBaseUrl")}</label>
     <input type="text" value={configOverrideBaseUrl()} onInput={(e) => setConfigOverrideBaseUrl(e.currentTarget.value)}
       placeholder="https://api.anthropic.com"
       class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent" />
     <p class="mb-3 text-[11px] text-ink-faint">{t("app.config.overrideBaseUrlHint")}</p>

     <label class="mb-1 block text-xs text-ink-muted">{t("app.config.overrideApiKey")}</label>
     <input type="password" value={configOverrideApiKey()} onInput={(e) => setConfigOverrideApiKey(e.currentTarget.value)}
       placeholder="sk-ant-..."
       class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent" />
     <p class="mb-3 text-[11px] text-ink-faint">{t("app.config.overrideApiKeyHint")}</p>
     <hr class="mb-4 border-border-subtle" />
   </Show>
   ```

5. No `saveConfig()`: incluir `overrideBaseUrl` e `overrideApiKey` na chamada `setConfig()`.

### 9. Frontend: Adicionar strings de tradução (`src/lib/locales/en-US.ts` e `pt-BR.ts`)

**Target:** Ambos os arquivos de locale

**Mutation:** Adicionar chaves no namespace `app.config`:
- `overrideBaseUrl`: "Anthropic URL Override" / "URL Anthropic (Override)"
- `overrideBaseUrlHint`: "Overrides the API endpoint for LLM calls only. Leave empty to use default." / "Substitui o endpoint da API apenas para chamadas ao LLM. Deixe vazio para usar o padrão."
- `overrideApiKey`: "API Key Override" / "Chave API (Override)"
- `overrideApiKeyHint`: "Overrides the API key for LLM calls only. Leave empty to use the signed-in key." / "Substitui a chave API apenas para chamadas ao LLM. Deixe vazio para usar a chave do sign-in."

## Risks

1. **Risco baixo:** Os novos campos `override_base_url` e `override_api_key` são `Option<String>` com `#[serde(default)]`, então configs existentes sem esses campos carregam normalmente (default = None).
2. **Risco baixo:** O `list_models` e `web_search` não são alterados, mantendo comportamento existente.
3. **Risco médio:** Se o usuário digitar "iddqd" enquanto foca um input que contenha essas letras (ex: input number "max rounds" tendo um "d" no placeholder), isso não ativa o Easter egg — mas também não atrapalha. O handler já ignora inputs focados.

## Verification Plan

1. **Build check:** `cd src-tauri && cargo check` — compila sem erros
2. **Frontend build:** `pnpm run build` ou `pnpm exec vite build` — sem erros TS
3. **Unit test (Rust):** Verificar que `AgentConfig` com `override_base_url` setado faz `stream_message` usar a URL override (se houver testes existentes)
4. **Manual test:** Abrir settings, digitar "iddqd" (sem foco em input) → campos aparecem
5. **Manual test:** Preencher override URL e API Key, salvar, reabrir settings → Easter egg resetado (campos ocultos), mas após digitar "iddqd" novamente, valores preenchidos anteriormente aparecem
6. **Manual test:** Enviar mensagem no chat com overrides configurados → requisição vai para a URL override usando a API key override

## Tasks Summary

10 atomic tasks: 5 backend (Rust), 4 frontend (TypeScript/SolidJS), 1 i18n (locales)


## Implementation Log — 2026-07-10 23:07
**Summary:** Easter egg "iddqd" no settings: digitar a sequência revela campos de override de URL Anthropic e API Key para chamadas LLM
**Changed files:** M docs/plans/2026-07-10_untested-pure-functions-coverage.md, M src-tauri/src/agent/provider.rs, M src-tauri/src/commands/agent.rs, M src/App.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-09_deploy-tag-0-1-1.md, ?? docs/plans/2026-07-10_iddqd-easter-egg-override.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Implementation Journal — Easter Egg "iddqd"

### Key decisions
1. **Override scope**: only `stream_message`, `classify_turn_completion`, and `one_shot` use the overrides. `list_models`, `web_search`, and OAuth sign-in are NOT affected — they continue using the normal `base_url`/`api_key`.
2. **Visibility**: Easter egg resets every time the settings modal closes. Must type `iddqd` again on each re-open. Values persist in config.json and survive restarts.
3. **Empty string = None**: both in the Rust `set_config` handler and in the Typescript save logic, an empty string is treated as None — clearing the override without sending garbage to the backend.

### Rust gotchas
- `config.override_api_key.as_deref()` returns `Option<&str>`. When used with `.unwrap_or(&config.api_key)`, the result is `&str`. Passing `&effective_api_key` to `.header()` creates `&&str`, which the reqwest `HeaderValue` trait can't convert. The fix: pass `effective_api_key` directly (no extra `&`).
- The `#[serde(default)]` attribute on `Option<String>` fields ensures backward compatibility — existing config files load fine without these fields.

### Frontend notes
- The `onKeyDown` handler is placed on the **outer modal overlay div**, with a guard that skips tracking when the event target is an input/select/textarea. This prevents accidental activation while typing in settings fields.
- The keystroke buffer matches case-insensitively (`e.key.toLowerCase()`) — works whether the user types lowercase or with Caps Lock.
- The Easter egg appears as an inline `<Show when={easterEggActive()}>` block between the Account section and the Plan save path, with `<hr>` separators above and below.

### Files changed
- `src-tauri/src/agent/provider.rs` — AgentConfig fields + effective_base_url/effective_api_key in 3 functions
- `src-tauri/src/commands/agent.rs` — SetConfigArgs + set_config + get_config
- `src/lib/ipc.ts` — AgentConfig + SetConfigArgs TypeScript interfaces
- `src/lib/locales/en-US.ts` — 4 i18n strings
- `src/lib/locales/pt-BR.ts` — 4 i18n strings
- `src/App.tsx` — Easter egg logic + override fields in settings modal

**Task journal:**
- Rust: Add override fields to AgentConfig struct: Added override_base_url and override_api_key fields with #[serde(default)]; Added None defaults in impl Default
- Rust: Use overrides in stream_message, classify_turn_completion, one_shot: classify_turn_completion: added effective_base_url/effective_api_key, updated URL + header; one_shot: added effective_base_url/effective_api_key, updated URL + header; stream_message: added effective_base_url/effective_api_key, updated URL + header
- Rust: Update SetConfigArgs and set_config for overrides: Added fields to SetConfigArgs; Added handling in set_config with empty-string-to-None logic
- Rust: Return override fields in get_config: Added overrideBaseUrl and overrideApiKey to the JSON response in get_config
- Rust: cargo check verification: cargo check passed after fixing &&str issue (had to deref effective_api_key properly)
- Frontend: Update IPC TypeScript interfaces: Added overrideBaseUrl and overrideApiKey to both AgentConfig and SetConfigArgs interfaces
- Frontend: Easter egg detector + override fields in settings modal: Added 5 signals (easterEggActive, keystrokeBuf, configOverrideBaseUrl, configOverrideApiKey); openConfig: reset easter egg, populate override values from cfg; saveConfig: include overrideBaseUrl and overrideApiKey in setConfig call; onKeyDown handler on modal overlay — tracks 'iddqd' keystrokes, skips when input/select/textarea focused; <Show when={easterEggActive()}> block with override URL (text) and API key (password) inputs
- Frontend: Add i18n translation strings: Added 4 keys to en-US.ts and pt-BR.ts
- Frontend: Build verification: pnpm exec vite build passed — 0 errors
