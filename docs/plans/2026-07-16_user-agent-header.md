# User-Agent `Claudinio-Code/X.Y.Z` em todas as chamadas HTTP

## Contexto

Atualmente nenhuma das 10 chamadas HTTP do Claudinio Code envia um header `User-Agent` personalizado. O `reqwest` usa o default (`reqwest/<versão>`), que não identifica a aplicação perante os serviços de destino (API do provedor, skills registry, web search, etc.).

**Evidência da investigação:** `grep -rn 'User-Agent\|user_agent' src-tauri/src/` retorna zero resultados. Nenhum dos 10 `Client::new()` ou `Client::builder()` define `.user_agent(...)`.

## Solution Design

**Meta:** Toda chamada HTTP originada pelo Claudinio Code deve incluir o header `User-Agent: Claudinio-Code/0.1.10` (versão dinâmica de `env!("CARGO_PKG_VERSION")`).

**Estratégia:** Criar um módulo `src-tauri/src/http.rs` com duas funções públicas (privadas à crate):

- `default_client() -> reqwest::Client` — substitui `reqwest::Client::new()` com o User-Agent já configurado.
- `default_client_builder() -> reqwest::ClientBuilder` — substitui `reqwest::Client::builder()` com o User-Agent já configurado, pronto para receber timeouts adicionais.

Registar o módulo em `lib.rs` e adaptar os 10 call sites.

## Riscos

- **Baixo:** `reqwest::ClientBuilder` é cloneable e stateful — `.user_agent()` é idempotente (último valor ganha). Nenhum call site atual define User-Agent, então não há conflito.
- **Baixo:** A API crate pública não expõe o módulo `http` — só `pub mod code_intel` está exposto, os outros são privados. Nenhum consumidor externo será afetado.

## Non-goals

- Não adicionar User-Agent nas chamadas do frontend (fetch do lado JS) — o foco é o backend Rust.
- Não expor o módulo `http` como API pública da crate.
- Não usar build script — `env!("CARGO_PKG_VERSION")` é resolvido em compile-time.

## Low-Level Design

### Novo ficheiro: `src-tauri/src/http.rs`

```rust
use reqwest::{Client, ClientBuilder};

const USER_AGENT: &str = concat!("Claudinio-Code/", env!("CARGO_PKG_VERSION"));

/// Returns a `reqwest::Client` pre-configured with the Claudinio-Code User-Agent.
/// Use instead of `reqwest::Client::new()`.
pub(crate) fn default_client() -> Client {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .expect("hardcoded user_agent always builds")
}

/// Returns a `reqwest::ClientBuilder` pre-configured with the Claudinio-Code User-Agent.
/// Use instead of `reqwest::Client::builder()` — callers can add timeouts before `.build()`.
pub(crate) fn default_client_builder() -> ClientBuilder {
    Client::builder().user_agent(USER_AGENT)
}
```

`concat!` resolve o User-Agent em compile-time como `Claudinio-Code/0.1.10`, sem overhead de runtime.

### Registo: `src-tauri/src/lib.rs`

Adicionar no topo:
```rust
pub(crate) mod http;
```

### Call sites a adaptar (10):

| # | Ficheiro | Linha | Antes | Depois | Notas |
|---|----------|-------|-------|--------|-------|
| 1 | `agent/provider.rs` | 553 | `reqwest::Client::builder()` | `crate::http::default_client_builder()` | + `connect_timeout(15s)` + `timeout(45s)` |
| 2 | `agent/provider.rs` | 626 | `reqwest::Client::builder()` | `crate::http::default_client_builder()` | + `connect_timeout(15s)` + `timeout(90s)` |
| 3 | `agent/provider.rs` | 714 | `reqwest::Client::builder()` | `crate::http::default_client_builder()` | + `connect_timeout(15s)` |
| 4 | `commands/agent.rs` | 634 | `reqwest::Client::new()` | `crate::http::default_client()` | `list_models` |
| 5 | `commands/auth.rs` | 183 | `reqwest::Client::new()` | `crate::http::default_client()` | OAuth code exchange |
| 6 | `commands/auth.rs` | 253 | `reqwest::Client::new()` | `crate::http::default_client()` | `validate_api_key` |
| 7 | `agent/tools/web_search.rs` | 50 | `reqwest::Client::new()` | `crate::http::default_client()` | web search |
| 8 | `agent/skills.rs` | 337 | `reqwest::Client::builder()` | `crate::http::default_client_builder()` | + `timeout(10s)` |
| 9 | `agent/skills.rs` | 468 | `reqwest::Client::builder()` | `crate::http::default_client_builder()` | + `timeout(15s)` |
| 10 | `code_intel/embeddings.rs` | 463 | `reqwest::Client::new()` | `crate::http::default_client()` | download de modelo |

### Verificação

1. **Compilação:** `cargo build` — deve compilar sem warnings.
2. **Unitários existentes:** `cargo test` — todos os testes devem continuar a passar.
3. **Grep do User-Agent:** `grep -r 'user_agent' src-tauri/src/http.rs` deve encontrar a definição.
4. **Grep do call sites:** `grep -rn 'default_client\|default_client_builder' src-tauri/src/` deve retornar 10+ ocorrências (10 call sites + 2 definições + import no lib.rs).
5. **Grep inverso:** `grep -rn 'reqwest::Client::new()\|reqwest::Client::builder()' src-tauri/src/ --include='*.rs'` deve retornar **zero** resultados (exceto no módulo `http.rs`).


## Implementation Log — 2026-07-16 23:06
**Summary:** Add User-Agent: Claudinio-Code/0.1.10 to all 10 HTTP call sites
**Changed files:** M src-tauri/src/agent/provider.rs, M src-tauri/src/agent/skills.rs, M src-tauri/src/agent/tools/web_search.rs, M src-tauri/src/code_intel/embeddings.rs, M src-tauri/src/commands/agent.rs, M src-tauri/src/commands/auth.rs, M src-tauri/src/lib.rs, ?? docs/plans/2026-07-16_user-agent-header.md, ?? src-tauri/src/http.rs
**Commits:** _(git unavailable or none)_
**Journal:** Implemented cleanly — 10 call sites adapted across 6 files with zero regressions (225 tests pass, no new warnings). Key decisions: (1) Used `concat!` macro to bake the version string at compile-time — zero runtime overhead. (2) Created two helpers: `default_client()` for raw `Client::new()` replacements and `default_client_builder()` for builder-chain replacements so callers can still add timeouts. (3) The `pub(crate)` visibility keeps the module internal to the crate. The only two raw `reqwest::Client` references left in the entire src tree are the doc comments inside http.rs itself — correct and intentional.

**Task journal:**
- Criar módulo http.rs com helpers de User-Agent: Created src-tauri/src/http.rs with USER_AGENT const, default_client() and default_client_builder()
- Registar módulo http em lib.rs: Added pub(crate) mod http; to lib.rs after mod commands;
- Adaptar call sites: agent/provider.rs (3 sites): Replaced 3 sites in provider.rs: lines 553 (classify_turn_completion), 626 (one_shot), 714 (stream_message). All kept their connect_timeout/timeout chains.
- Adaptar call sites: commands/agent.rs + commands/auth.rs (3 sites): Replaced 3 sites: agent.rs:634, auth.rs:183, auth.rs:253
- Adaptar call sites: web_search.rs, skills.rs, embeddings.rs (4 sites): Replaced 4 sites: web_search.rs:50, skills.rs:337, skills.rs:469, embeddings.rs:463
- Verificar compilação, testes e cobertura de call sites: Build: 0 errors. Tests: 225 passed, 0 failed, 3 ignored. default_client occurrences: 12 (10 call sites + 2 defs + import). Raw reqwest::Client::new()/builder(): zero outside http.rs comments.
