# Plan: `claudinio` puro abre o chat por default

> Implementado em `bf00d97` (ver `cli/src/main.rs:88`). Follow-up restante: documentação no README do npm.

## Context

O usuário pediu, em PT-BR: "gostaria que ao chamar claudinio, abrisse o chat por default e nao precisar de claudinio chat".

Hoje o binário `claudinio` SEM subcomando já abre a TUI de chat: `cli/src/main.rs:88` faz `match cli.command.unwrap_or(Command::Chat { path: None })`. A funcionalidade foi implementada em `bf00d97 feat(cli): add claudinio CLI + minimalist ratatui TUI over the shared core`, mas a documentação do pacote (`npm/claudinio/README.md:14`) ainda diz `npx claudinio chat  # TUI interativa` como caminho canônico, sem mencionar que rodar `claudinio` puro já é equivalente.

Decisão confirmada pelo usuário: **só documentar e nada mudar no código**.

## Solution Design

- Tornar o `command` da struct `Cli` opcional via `Option<Command>`.
- Adicionar `#[command(subcommand)]` ainda, mas `Option`.
- Em `main()`, se `command` for `None`, despachar para `commands::chat::run(None)` (cwd).
- Não adicionar `--path` no topo; o usuário que precisar de outro path usa `claudinio chat --path X` (decisão confirmada pelo usuário).
- Nenhuma outra mudança de comportamento.

## Risks

- **Baixo.** Mudança isolada em um arquivo (`cli/src/main.rs`). Nenhum impacto em outros subcomandos.
- Ajuda do clap (`claudinio --help`) ainda lista todos os subcomandos; apenas a execução sem subcomando muda.

## Non-goals

- Não adicionar `--path` no topo da CLI.
- Não alterar a TUI em si.
- Não mudar mensagens de ajuda/uso do clap.

## Low-Level Design

**Arquivo:** `cli/src/main.rs` (único arquivo alterado).

**Estado atual (trechos relevantes):**

```rust
struct Cli {
    #[command(subcommand)]
    command: Command,   // obrigatório
}
```

```rust
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Config { action } => commands::config::run(action),
        // ...
        Command::Chat { path } => commands::chat::run(path).await,
        // ...
    }
}
```

**Mudança 1 — tornar `command` opcional:**

```rust
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}
```

**Mudança 2 — despachar `None` para chat:**

```rust
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Chat { path: None }) {
        Command::Config { action } => commands::config::run(action),
        Command::Models => commands::config::run_models().await,
        Command::Index { path } => commands::index::run(path).await,
        Command::Search { query, path, limit } => commands::search::run(query, path, limit).await,
        Command::Run { message, mode, path, yes } => commands::run::run(message, mode, path, yes).await,
        Command::Chat { path } => commands::chat::run(path).await,
        Command::Auth { action } => commands::auth::run(action).await,
        Command::Sessions { action } => commands::sessions::run(action),
    }
}
```

`Command::Chat { path: None }` reutiliza o despacho existente; `commands::chat::run` já aceita `Option<String>` e delega para `crate::tui::run(path)` (`cli/src/commands/chat.rs:7`, `cli/src/tui/app.rs:137`), que usa `cwd` quando `path == None`. Sem novos arquivos, sem novas deps.

## Tasks summary

1. Editar `cli/src/main.rs`: `command: Option<Command>` + `unwrap_or(Command::Chat { path: None })` no `main`.
2. Buildar (`cargo build -p claudinio` ou equivalente) e verificar `claudinio --help` e execução sem subcomando.
