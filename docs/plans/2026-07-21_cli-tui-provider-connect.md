# Conectar providers no TUI/CLI — Solução de Design

## Contexto

O desktop (Tauri) já suporta 3 tiers de providers:
1. **Claudinio** — nativo, "Recommended", OAuth via core
2. **OpenRouter** — OAuth PKCE, badge "Experimental", implementado só no Tauri
3. **Catálogo models.dev** — providers manuais via `connect_provider` Tauri command

No CLI/TUI:
- `claudinio auth login --provider claudinio` funciona (OAuth PKCE)
- `claudinio auth login --provider openrouter` **baila** com `"ainda não implementado"`
- **Não existe** comando para adicionar providers manuais
- `/model` no TUI já lista providers conectados (se existirem em config.json) — mas não tem UI de conexão

O core (`claudinio_core::agent::provider::resolve_provider`, `ProviderEntry`, `save_config`) é **compartilhado** — o suporte a providers já existe no runtime, falta a CLI/TUI para conectá-los.

## Solução de Design

### Hierarquia de prioridade (confirmada)

1. **Claudinio** — destaque visual "Recommended", OAuth PKCE, funcionalidades exclusivas (web_search, budget tracking). Já implementado.
2. **OpenRouter** — OAuth PKCE via CLI, badge "Experimental". Hoje baila — será implementado.
3. **Manual (provider add)** — providers do catálogo models.dev (DeepSeek, Anthropic, etc.) ou qualquer provider com API key + base_url. `provider add` via CLI + slash command no TUI.

### Escopo

| Feature | CLI subcomando | TUI slash |
|---|---|---|
| OpenRouter OAuth | `auth login --provider openrouter` | N/a (OAuth abre browser, sai do TUI) |
| Provider manual | `provider add <id>` (flags + interactive) | `/provider add` |
| Listar conectados | `provider list` | `/provider list` |
| Remover | `provider remove <id>` | `/provider remove` |

### Como funciona (fluxo)

**Claudinio** → já existe em `core::auth::login_claudinio` + `cli/src/commands/auth.rs`

**OpenRouter OAuth** → portar o mesmo PKCE do Tauri para o CLI:
1. Bind TcpListener 127.0.0.1:0
2. `random_hex(32)` verifier → base64url(SHA256) challenge
3. Abrir `https://openrouter.ai/auth?callback_url=...&code_challenge=...&code_challenge_method=S256`
4. `wait_for_callback(listener, None)` — captura o code
5. POST `https://openrouter.ai/api/v1/auth/keys` com `{code, code_verifier, code_challenge_method:"S256"}`
6. Salvar `config.providers["openrouter"] = ProviderEntry { ... }`
7. Mostrar modelos disponíveis

**Provider manual** → novo subcomando `provider`:
- `provider add <id>`:
  - Flags: `--api-key` (obrigatório), `--base-url` (opcional), `--protocol [openai|anthropic]` (opcional, default openai), `--label` (opcional)
  - Se alguma flag faltar, prompt interativo pergunta
  - Cria `ProviderEntry` com os dados fornecidos
  - Chama `save_config(&cfg)` — mesma função que o desktop usa
  - Sem dependência do catálogo models.dev (dados fornecidos pelo usuário)
- `provider list`:
  - Itera `config.providers` e exibe id, label, protocolo, nº modelos
- `provider remove <id>`:
  - Remove do map, fallback brain→claudius / builder→claudinio, save_config

**TUI (slash commands)**:
- `provider list` → commit_notice com tabela de providers
- `provider add <id>` → prompt interativo inline (reusa sistema de `ask_user` ou editor input)
- `provider remove <id>` → confirma e remove

### Não Escopo (Non-goals)

- Catálogo models.dev navegável no TUI (seria fase futura)
- Provider `edit` (trocar API key sem recriar) — pode ser `provider add` sobreescrevendo
- Login provider no TUI via OAuth (não faz sentido — OAuth abre browser)
- Mudanças no core (provider resolution, ProviderEntry, save_config) — já existe tudo

## Risco e Mitigação

| Risco | Mitigação |
|---|---|
| OpenRouter mudar endpoint OAuth | Mesmo endpoint usado no desktop — funcional hoje |
| CLI sem `open::that()` em headless | Printar URL como fallback (mesmo pattern do login claudinio) |
| Sem catálogo, modelos externos sem pricing no TUI | ProviderEntry aceita model_pricing vazio; custo só não aparece |
| TUI `/provider add` interativo com prompts | Reusa o sistema `PendingQuestion` (já implementado) ou editor inline |

## Low-Level Design

### 1. `cli/src/commands/provider.rs` — novo comando

```
src/cli/commands/
├── mod.rs         → pub mod provider;
├── provider.rs    → NOVO
```

**Subcomandos:**
```rust
#[derive(Subcommand)]
pub enum ProviderAction {
    /// Adiciona um provider manual (DeepSeek, Anthropic, etc.)
    Add {
        id: String,
        #[arg(long)]
        api_key: Option<String>,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long, default_value = "openai")]
        protocol: String,
        #[arg(long)]
        label: Option<String>,
    },
    /// Lista providers conectados
    List,
    /// Remove um provider conectado
    Remove { id: String },
}
```

**Implementação:**

`run_add(id, api_key, base_url, protocol, label)`:
- Se `api_key` for None, prompt interativo: `rpassword::read_password_from_stdin` ou `dialoguer::Password`
- Se `base_url` for None, prompt: `dialoguer::Input`
- Construir `ProviderEntry { api_key, base_url, protocol, label, ..default }`
- `let mut cfg = provider::load_config(); cfg.providers.insert(id, entry); provider::save_config(&cfg);`
- Printar sucesso

`run_list()`:
- `let cfg = provider::load_config();`
- Se vazio: "Nenhum provider conectado."
- Iterar, printar tabela: id | label | protocol | models

`run_remove(id)`:
- `let mut cfg = provider::load_config(); cfg.providers.remove(&id);`
- Se brain_model ou builder_model starta com "{id}/", resetar para claudius/claudinio
- `provider::save_config(&cfg);`

**Flags de prompt interativo:**
- `dialoguer` já é dependência transitiva via `clap` (basta adicionar)
- Ou usar `rpassword` para API key (mais seguro)
- Ou usar stdin simples como o resto do CLI faz

Minimal: Para não adicionar deps novas, usar `std::io::stdin().read_line()` + `rpassword`. Rpassword já está no Cargo.lock? Vou verificar — se não, adicionar.

**Falha segura:** Se `id` já existe no config, confirmar overwrite.

### 2. `cli/src/commands/auth.rs` — desbloquear OpenRouter

Trocar:
```rust
"openrouter" => {
    anyhow::bail!("login openrouter pelo CLI ainda não implementado; use o app por ora.")
}
```
Por implementação completa:

```rust
"openrouter" => {
    println!("Abrindo browser para autorizar OpenRouter…");
    let key = openrouter_login_cli().await?;
    println!("✓ OpenRouter conectado. Chave: …{}", &key[key.len()-4..]);

    // Buscar modelos (live ou vazio)
    let models = list_openrouter_models_cli().await.unwrap_or_default();
    println!("{} modelos disponíveis", models.len());

    let mut cfg = provider::load_config();
    cfg.providers.insert("openrouter".into(), ProviderEntry {
        api_key: key,
        base_url: "https://openrouter.ai/api/v1".into(),
        protocol: "openai".into(),
        enabled_models: Vec::new(),
        label: Some("OpenRouter".into()),
        model_pricing: HashMap::new(),  // sem catalog
        model_output_limits: HashMap::new(),
    });
    provider::save_config(&cfg);
    println!("✓ Provider OpenRouter salvo em config.json");
}
```

`openrouter_login_cli()` — port do Tauri para CLI:
```rust
async fn openrouter_login_cli() -> Result<String, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let verifier = random_hex(32);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let authorize_url = format!(
        "https://openrouter.ai/auth?callback_url=http%3A%2F%2F127.0.0.1%3A{port}%2Fcallback&code_challenge={challenge}&code_challenge_method=S256"
    );
    println!("Abrindo browser… Se não abrir, acesse:\n  {authorize_url}");
    open::that(&authorize_url).map_err(|e| format!("falha ao abrir browser: {e}"))?;
    let code = wait_for_callback(listener, None).await?;
    // key exchange POST
    let client = claudinio_core::http::default_client();
    let resp = client.post("https://openrouter.ai/api/v1/auth/keys")
        .json(&serde_json::json!({"code": code, "code_verifier": verifier, "code_challenge_method": "S256"}))
        .send().await?;
    // parse key from response
    let parsed: Value = resp.json().await?;
    let key = parsed["key"].as_str()...;
    Ok(key.to_string())
}
```

**Dependências adicionar em `cli/Cargo.toml`:**
- `base64` (já existe em `src-tauri`, verificar no workspace)
- `sha2` (já existe em `core`)
- `claudinio_core::http` (já exportado)
- `serde_json::Value` (já em core)

### 3. `cli/src/tui/app.rs` — slash command `/provider`

Adicionar em `overlays.rs::COMMANDS`:
```rust
SlashCmd { name: "provider", desc: "add / list / remove external providers" },
```

Adicionar match arm em `app.rs::run_command`:
```rust
"provider" => {
    let parts: Vec<&str> = arg.split_whitespace().collect();
    match parts.first().copied() {
        Some("list") => {
            let cfg = provider::load_config();
            if cfg.providers.is_empty() {
                app.commit_notice("Nenhum provider conectado.", theme.muted);
            } else {
                for (id, p) in &cfg.providers {
                    let label = p.label.as_deref().unwrap_or(id);
                    app.commit_notice(format!("  {label} ({id}) — {protocol}", protocol = p.protocol), theme.accent);
                }
            }
        }
        Some("add") => {
            // Tentar parsear flags do próprio arg
            // Se faltar info, abrir prompt inline similar a ask_user
            // Ou simplesmente: "use `claudinio provider add <id>` no terminal"
            app.commit_notice("Use `claudinio provider add <id> --api-key ...` no terminal", theme.warning);
        }
        Some("remove") => {
            let id = parts.get(1).unwrap_or(&"");
            if id.is_empty() { app.commit_notice("Use: /provider remove <id>", theme.warning); return; }
            let mut cfg = provider::load_config();
            cfg.providers.remove(*id);
            // fallback
            provider::save_config(&cfg);
            app.commit_notice(format!("Provider {id} removido."), theme.success);
        }
        _ => { app.commit_notice("/provider: add | list | remove", theme.muted); }
    }
}
```

### 4. `cli/src/commands/mod.rs` — adicionar `pub mod provider`

Adicionar: `pub mod provider;`

### 5. `cli/src/main.rs` — adicionar subcomando

```rust
/// Adiciona/Lista/Remove providers externos (OpenRouter, DeepSeek, etc).
Provider {
    #[command(subcommand)]
    action: commands::provider::ProviderAction,
},
```

E no match: `Command::Provider { action } => commands::provider::run(action).await,`

### 6. Dependências

Verificar se `base64` e `sha2` estão disponíveis para o crate `cli`:
- `sha2` — já disponível via `claudinio_core` (re-export ou dep direta)
- `base64` — verificar workspace Cargo.toml

Se não disponíveis, adicionar ao `cli/Cargo.toml`:
```toml
base64 = "0.22"
sha2 = "0.10"
rpassword = "7"  # para input de API key sem echo
```

### Arquivos alterados (8):

| File | Operação |
|---|---|
| `cli/src/commands/auth.rs` | Modificar — implementar OpenRouter OAuth |
| `cli/src/commands/provider.rs` | **Novo** — subcomando provider add/list/remove |
| `cli/src/commands/mod.rs` | Modificar — adicionar `pub mod provider` |
| `cli/src/main.rs` | Modificar — adicionar `Provider` subcomando |
| `cli/src/tui/overlays.rs` | Modificar — adicionar `provider` à COMMANDS |
| `cli/src/tui/app.rs` | Modificar — adicionar match arm `"provider"` em run_command |
| `cli/Cargo.toml` | Modificar — adicionar deps (base64, sha2, rpassword) |
| Nenhum core | Core stays untouched |

## Tasks

- **T1**: `cli/src/commands/auth.rs` — implementar OpenRouter OAuth login (PKCE flow + key exchange + save config)
- **T2**: `cli/src/commands/provider.rs` (novo) — subcomandos `add`, `list`, `remove` com flags + prompts interativos
- **T3**: `cli/src/commands/mod.rs` + `cli/src/main.rs` — registrar `provider` subcomando e dispatch
- **T4**: `cli/src/tui/overlays.rs` + `cli/src/tui/app.rs` — slash command `/provider` com subcomandos list/remove
- **T5**: `cli/Cargo.toml` — adicionar dependências necessárias (base64, sha2, rpassword)
