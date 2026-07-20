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
- Não introduzir nova flag nem subcomando.

## Risks

- **Mudança de UX para usuários existentes que rodam `claudinio` por engano.**
  Antes: mensagem "Subcommand required" do clap. Agora: abre TUI.
  `claudinio config/run/auth/...` segue exigindo o subcomando; só `claudinio`
  solto muda — alinhado com o pedido.
- **Empacotamento npm** (`npm/claudinio/bin/claudinio.mjs`) passa argv ao
  binário; rodar `npx claudinio` sem args dispara TUI em vez de erro.
  Comportamento desejado.
- **Auto-commit do plano** já gravou `docs/plans/2026-07-20_claudinio-default-chat.md`
  no HEAD antes da edição. O commit do código Rust deve ser separado.

## Low-Level Design

### Arquivo único a tocar
`cli/src/main.rs`

### Símbolos
- `struct Cli` (`cli/src/main.rs:16-21`): trocar `command: Command` →
  `command: Option<Command>`. Atributo `#[command(subcommand)]` continua;
  opcionalidade vem do tipo.
- `fn main` (`cli/src/main.rs:80-91`): inserir um braço `None` no `match`
  que despacha `commands::chat::run(None).await`; envolver cada `Command::X`
  em `Some(...)`.
- `enum Command::Chat` (`cli/src/main.rs:54-58`): atualizar o `///` doc
  para mencionar que também é o default quando o usuário não passa
  subcomando. Nenhuma mudança de shape.

### Pontos de reuso (não mexer)
- `commands::chat::run` (`cli/src/commands/chat.rs:6`) já delega para
  `crate::tui::run(path).await`. `None` abre no cwd.
- `tui::run` (`cli/src/tui/mod.rs`) não muda.
- `npm/claudinio/bin/claudinio.mjs`, `scripts/build-npm.mjs` e
  `npm/claudinio/README.md:14` não mudam — `chat` segue sendo subcomando.

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
2. **Default dispara chat:** rodar `cargo run -p claudinio-cli --` com stdout
   pipe (sem TTY) → confirmar que entra em `commands::chat::run` (path de
   erro do TUI: "stdout não é um TTY"). Esse erro é a prova de que o despacho
   foi para o chat (único comando que exige TTY via `ratatui::init`).
3. **`claudinio chat` segue funcionando:** `cargo run -p claudinio-cli --
   chat` → mesmo erro de TTY em stdout pipe.
4. **Outros subcomandos inalterados:** `cargo run -p claudinio-cli -- --help`
   lista `config, models, index, search, run, chat, auth, sessions` na mesma
   ordem/forma de antes.
5. **Version inalterada:** `cargo run -p claudinio-cli -- --version` imprime
   a versão do `Cargo.toml`.

## Tasks summary

- `cli-chat-default-1` — Em `cli/src/main.rs`: `Cli.command: Option<Command>`;
  doc do `Command::Chat`; novo braço `None` no `match` despachando
  `commands::chat::run(None)`; demais variantes envoltas em `Some(...)`.
- `cli-chat-default-2` — Rodar `cargo check -p claudinio-cli`; em stdout
  pipe confirmar (2) e (3) acima; `--help` (4); `--version` (5).
