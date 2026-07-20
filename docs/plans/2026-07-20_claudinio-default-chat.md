# `claudinio` (sem args) deve abrir o chat por default

## Context

Hoje o CLI exige `claudinio chat` para abrir a TUI interativa. O usuário
quer que `claudinio` (sem subcomando) já abra o chat, sem precisar digitar
`chat`. `claudinio chat` continua funcionando.

Restrição: manter todos os outros subcomandos (`config`, `models`, `index`,
`search`, `run`, `auth`, `sessions`) inalterados, e manter `claudinio chat`
como caminho explícito válido (retrocompatibilidade para a linha
`npx claudinio chat` documentada em `npm/claudinio/README.md:14`).

## Solution Design

### Decisão
Tornar o campo `command` em `Cli` opcional (`Option<Command>`). Quando vier
`None` (usuário rodou `claudinio` sozinho), despachar `commands::chat::run(None)`
— equivalente a `claudinio chat` no diretório atual. Quando vier
`Some(Command::Chat { path })`, idem. Demais subcomandos, idem.

Isso é o padrão idiomático do `clap` derive para "default subcommand" sem
precisar de `clap::Command::default_subcommand` (que exige API imperativa, não
derive) e não exige introduzir uma flag nova.

### UX
- `claudinio` → TUI abre (mesmo path/estado de `claudinio chat`).
- `claudinio chat` → TUI abre (compat).
- `claudinio chat --path <dir>` → TUI com workspace fixo (compat).
- `claudinio --help` → continua listando `config/models/index/search/run/chat/auth/sessions`.
- `claudinio run ...`, `claudinio auth ...`, etc. → inalterados.

### Edges
- `--version` continua funcionando (vem do `#[command(version, about)]` no `Cli`).
- Sem subcomando e sem flag → cai no default (chat).
- Stream stdin/TTY: o `tui::run` já exige TTY; o launcher npm
  (`npm/claudinio/bin/claudinio.mjs`) já passa `-T`/pseudo-tty quando preciso,
  nada muda aqui.

### Non-goals
- Não mudar layout/visual da TUI.
- Não trocar o nome do binário.
- Não mudar `Chat { path }` — o campo `path: Option<String>` segue valendo.
- Não promover `chat` ao único subcomando (mantemos os outros).

## Low-Level Design

### Arquivo único a tocar
`cli/src/main.rs`

### Símbolos
- `struct Cli` (atual `cli/src/main.rs:16-21`): trocar `command: Command` →
  `command: Option<Command>`. O atributo `#[command(subcommand)]` continua;
  opcionalidade vem do tipo.
- `fn main` (`cli/src/main.rs:80-91`): inserir um braço `None` no `match`
  antes do `Some(Command::Chat)` que despacha `commands::chat::run(None).await`,
  e trocar o acesso `cli.command` por `cli.command` (já é `Option`, só
  ajustar o pattern).
- `enum Command` (`cli/src/main.rs:23-77`): atualizar o `///` doc do
  `Command::Chat` para mencionar que também é o default quando o usuário
  não passa subcomando. Nenhuma mudança de shape.

### Pontos de reuso (não mexer)
- `commands::chat::run` (`cli/src/commands/chat.rs:6`) já delega para
  `crate::tui::run(path).await`. Recebe `None` hoje (`claudinio chat` sem
  `--path`) e abre no cwd — comportamento desejado.
- `tui::run` (`cli/src/tui/mod.rs`) não muda.
- `npm/claudinio/bin/claudinio.mjs` e `scripts/build-npm.mjs` não mudam —
  eles só repassam argv ao binário; o default é decidido no Rust.
- `npm/claudinio/README.md:14` (`npx claudinio chat`) continua válido porque
  `chat` segue registrado como subcomando explícito.

### Wiring sketch (em `cli/src/main.rs`)

```rust
#[derive(Parser)]
#[command(
    name = "claudinio",
    version,
    about = "Claudinio Code — agente de código no terminal (brain/builder, busca semântica)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

// no fn main:
match cli.command {
    None => commands::chat::run(None).await,                 // default: chat
    Some(Command::Config { action }) => commands::config::run(action),
    Some(Command::Models) => commands::config::run_models().await,
    Some(Command::Index { path }) => commands::index::run(path).await,
    Some(Command::Search { query, path, limit }) => commands::search::run(query, path, limit).await,
    Some(Command::Run { message, mode, path, yes }) => commands::run::run(message, mode, path, yes).await,
    Some(Command::Chat { path }) => commands::chat::run(path).await,
    Some(Command::Auth { action }) => commands::auth::run(action).await,
    Some(Command::Sessions { action }) => commands::sessions::run(action),
}
```

E no `Command::Chat`:
```rust
/// TUI de chat interativa (brain/builder com handoff).
/// Este também é o subcomando executado quando `claudinio` é invocado sem
/// subcomando.
Chat {
    /// Diretório do workspace (padrão: diretório atual).
    #[arg(long)]
    path: Option<String>,
},
```

### Verification
1. **Compila:** `cargo check -p claudinio-cli` → exit 0, sem novos warnings
   introduzidos por esta mudança (warnings preexistentes em `core/agent/tools/*`
   permanecem, não são escopo).
2. **Default dispara chat:** rodar `cargo run -p claudinio-cli --` em stdin
   pipe para isolar do TTY e confirmar que a chamada entra em
   `commands::chat::run` (path de erro do TUI: "stdout não é um TTY").
   Esse erro é a prova de que o despacho foi para o chat (único comando que
   exige TTY via `ratatui::init`).
3. **`claudinio chat` segue funcionando:** `cargo run -p claudinio-cli --
   chat` → mesmo erro de TTY em stdout pipe (prova que o caminho explícito
   também despacha `commands::chat::run`).
4. **Outros subcomandos inalterados:** `cargo run -p claudinio-cli -- --help`
   lista `config, models, index, search, run, chat, auth, sessions` na mesma
   ordem/forma de antes.
5. **Version inalterada:** `cargo run -p claudinio-cli -- --version` imprime
   a versão do `Cargo.toml`.
