# Plano: CLI + TUI reaproveitando o backend Rust (`feat/cli-tui`)

> ## Status de implementação (branch `feat/cli-tui`)
> **Fases 0–7 implementadas.** Core `claudinio-core` extraído e 100% Tauri-free
> (0 deps de tauri); app Tauri delega ao core (sem regressão). CLI `claudinio`
> com `config/models/index/search/run/chat/auth/sessions` — `index`, `search` e
> `run` (brain/builder streaming) verificados de ponta a ponta com API real; TUI
> `chat` compila (testar em terminal real). Testes: 282 core + 2 desktop, verdes.
>
> **Distribuição npm:** launcher `npm/claudinio` + `scripts/build-npm.mjs` +
> job `publish-npm` no `release.yml` (build do CLI na matriz existente).
> Mecanismo validado localmente (shim resolve o pacote de plataforma e executa).
> **Pré-requisitos p/ publicar:** secret `NPM_TOKEN`, org npm `@claudinio`, e o
> nome `claudinio` livre (`npm view claudinio`).
>
> **Desvios do plano:**
> - Modelo MiniLM **não** foi movido para `core/models` (evita churn no bundle
>   Tauri, sem ganho funcional): o CLI resolve via `../src-tauri` em dev e baixa
>   em produção (`ensure_model_downloaded`). Fonte única efetiva: `src-tauri/models`.
> - Binário do CLI ≈ **183 MB** (onnxruntime estático + ~90 gramáticas
>   tree-sitter + tokenizers). `strip` aplicado no profile release. Reduzir mais
>   exige feature-gate das gramáticas — **follow-up**.


## Contexto

O Claudinio Code hoje é um app Tauri 2 + SolidJS. Todo o "cérebro" (harness de
agente, indexação, busca híbrida, auth, handoff) já vive em Rust dentro de
`src-tauri/`. Queremos um segundo frontend — um **CLI com TUI minimalista** —
para quem só precisa de algo prático, sem depender de JS/webview. O app visual
continua existindo para quem prefere a experiência gráfica.

Regra de ouro do projeto: **nada duplicado**. O CLI e o app devem compartilhar
o mesmo core; melhoria no core flui para os dois automaticamente. Todas as core
features precisam existir no CLI: indexação, brain mode, builder mode, busca
semântica, auth (claudinio + outros providers), handoff, brain effort.

### O que a exploração confirmou

O core já é **quase headless**. A única amarra real de Tauri no caminho de
execução do agente é o tipo `tauri::ipc::Channel<AgentEvent>`, usado apenas como
_sink_ (`chan.send(ev)`) em 5 arquivos: `agent/session.rs`, `agent/provider.rs`,
`agent/provider/openai.rs`, `agent/subagent.rs`, `agent/transition.rs`. Não há
`AppHandle`, `app.emit` nem `tauri::State` dentro de `agent/`.

- `code_intel/db.rs` (`IndexDb::search_hybrid`, BM25+vetor+RRF) e
  `code_intel/embeddings.rs` (ONNX/MiniLM-L6) são **100% Tauri-free** — operam
  sobre `&Path`.
- `code_intel/indexer.rs` (`scan_workspace`, `generate_all_embeddings`) já recebe
  `app_handle: Option<&AppHandle>` e `Channel` como **opcionais** — passar `None`
  funciona hoje; o progresso Tauri-free sai por `shared_progress: &Mutex<Option<IndexProgress>>`.
- `agent/provider.rs`: `config_path()`, `load_config()`, `save_config()`,
  `AgentConfig` (brain_model/builder_model, `thinking_effort`, providers) — sem Tauri.
  Effort buckets: `thinking_budget_tokens()` (Anthropic) e `reasoning_effort()` (OpenAI/OpenRouter).
- `state.rs`: `AppState`/`WorkspaceState` são structs planos (`Arc`/`Mutex`/`HashMap`),
  sem tipos Tauri. `AppState::new()` é chamável direto.
- OAuth (claudinio e OpenRouter) usa **loopback `TcpListener`**, não webview — a
  única chamada Tauri é abrir o browser (`app.opener().open_url`).
- O driver de handoff é `spawn_run_loop` em `commands/agent.rs:391-482` — ~90
  linhas de Rust puro que seguem `RunOutcome::Handoff` via `transition::link_session`;
  precisa ser levantado para o core.

## Decisões (confirmadas com o usuário)

1. **Estrutura**: Cargo workspace com crate `core` compartilhada (não segundo bin).
2. **Forma do CLI**: subcomandos scriptáveis + um comando `chat` interativo (TUI ratatui).

## Arquitetura alvo

```
claudinio_code/
├─ Cargo.toml            # [workspace] members = ["core", "desktop", "cli"]
├─ .cargo/config.toml    # (mantém +crt-static Windows, agora nível workspace)
├─ core/                 # claudinio-core: agent/, code_intel/, state, provider,
│  ├─ Cargo.toml         #   persist, http, auth (loopback), paths, EventSink trait
│  ├─ src/
│  └─ models/            # MiniLM-L6 bundlado (movido de src-tauri/models)
├─ desktop/              # = src-tauri atual: Tauri builder + commands/ + adapters
│  └─ (depends on claudinio-core; impl EventSink -> Channel)
└─ cli/                  # claudinio-cli: clap + ratatui + adapters stdout/TUI
   └─ (depends on claudinio-core; impl EventSink -> terminal/TUI)
```

Binários: `desktop` continua gerando o app Tauri; `cli` gera o executável
`claudinio`.

## Plano de execução

### Fase 0 — Scaffold do workspace (mecânico, sem mudança de lógica)
- Criar `Cargo.toml` raiz `[workspace]` com `members = ["core", "desktop", "cli"]`
  e `resolver = "2"`. Mover `.cargo/config.toml` continua na raiz (já está).
- Renomear/mover `src-tauri` → `desktop` (ou manter o nome de pasta `src-tauri`
  como `desktop` crate — manter a pasta reduz churn de `tauri.conf.json`,
  `capabilities/`, `gen/`, ícones e do `pnpm tauri` no `package.json`). **Recomendo
  manter a pasta `src-tauri`** e apenas renomear o package para `desktop` no seu
  `Cargo.toml`, ajustando o `[[bin]]`/`lib` conforme necessário.
- Criar `core/` e `cli/` vazios com `Cargo.toml` mínimos.
- **Checkpoint**: `pnpm tauri build`/`cargo build` do desktop ainda funcionam.

### Fase 1 — Extrair a crate `core`
- Mover para `core/src/`: `agent/` (inteiro), `code_intel/` (inteiro), `state.rs`,
  `http.rs`, e as deps pesadas do `Cargo.toml` (ort, tokenizers[onig], rusqlite,
  tree-sitter*, reqwest, tokio, rmcp, dirs, tiktoken-rs, xxhash, notify, sha2,
  serde*, uuid, chrono, regex, lru, image, base64). Preservar EXATAMENTE a config
  `tokenizers = { default-features=false, features=["onig"] }` e
  `ort features=["download-binaries"]` (comentário em `src-tauri/Cargo.toml:120`).
- `desktop/Cargo.toml` passa a depender de `claudinio-core` + só os deps Tauri
  (`tauri`, `tauri-plugin-*`, `tauri-build`, `keepawake`, `sys-locale`, `sysinfo`,
  `machine-uid`, `windows-sys`).
- `lsp/` pode ir para `core` (opcional) ou ficar no desktop; o CLI não precisa dele
  no dia 1. Decisão: mover para `core` mas não expor no CLI ainda.
- **Checkpoint**: `cargo build -p claudinio-core` compila sem nenhum crate Tauri;
  desktop continua compilando.

### Fase 2 — `EventSink` trait (a única refatoração de verdade)
- Em `core`, definir:
  ```rust
  pub trait EventSink: Send + Sync + 'static {
      fn send(&self, ev: AgentEvent);
  }
  ```
  (assinatura casa com o uso atual `let _ = chan.send(ev)` — engole erro).
- Substituir o parâmetro `event_tx: &Channel<AgentEvent>` por `&dyn EventSink`
  (object-safe) nos 5 arquivos: `agent/session.rs`, `agent/provider.rs`,
  `agent/provider/openai.rs`, `agent/subagent.rs`, `agent/transition.rs`.
- `agent/subagent.rs:109-124` hoje constrói `Channel::new(closure)` para embrulhar
  eventos de subagentes → trocar por um `struct WrappingSink { parent: Arc<dyn EventSink>, ... }`
  que reencaminha/rotula e implementa `EventSink`.
- **desktop**: `impl EventSink for TauriSink(Channel<AgentEvent>)` chamando
  `self.0.send(ev)`; adaptar `commands/agent.rs` para passar `&TauriSink`.
- **Checkpoint**: desktop compila e roda idêntico ao de antes (mesma stream de eventos).

### Fase 3 — Helpers compartilhados no core (path/workspace/run/auth)
Extrair a orquestração fina que hoje vive em `commands/` para funções de `core`
que ambos os frontends chamam (elimina duplicação):
- `core::paths` — versões puras de `resolve_model_dir`, `cache_model_dir`,
  `index_db_path` recebendo `config_dir`/`data_dir` explícitos (desktop passa dirs
  Tauri; CLI passa `dirs::config_dir()`/`dirs::data_dir()`). Substitui os helpers
  Tauri de `commands/code_intel.rs:31-76`.
- `core::workspace::open(root, data_dir, &config, embedder, progress_sink)` —
  levanta a orquestração de 5 fases de `open_workspace` (scan → load model → embed)
  para construir `WorkspaceState` sem Tauri.
- `core::run::drive(args, sink)` — levantar o loop de handoff de
  `commands/agent.rs:391-482` (`spawn_run_loop`) para o core, parametrizado por
  `&dyn EventSink`. Desktop e CLI passam a chamá-lo. Este é o ponto que destrava
  brain/builder + handoff no CLI.
- `core::auth` — extrair `wait_for_callback`/`parse_callback_query`/exchange
  (já `pub(crate)`) para uma função que retorna a chave; o frontend só decide como
  abrir o browser. Desktop usa `opener`; CLI usa o crate `open` (ou imprime a URL).
- **Checkpoint**: desktop refatorado para usar esses helpers e continua idêntico.

### Fase 4 — Crate `cli`: subcomandos não-interativos
Deps: `clap` (derive), `tokio`, `ratatui` + `crossterm`, `open`, `anyhow`,
`indicatif` (barra de progresso), `claudinio-core`.

Subcomandos (nome do binário: `claudinio`):
- `auth login [--provider claudinio|openrouter]` — loopback + `open` no browser
  (fallback: imprime URL). `auth logout`, `auth status`.
- `config get|set <key> [value]` — brain_model, builder_model, effort, base_url,
  api_key (via `load_config`/`save_config`).
- `models` — lista modelos (reusa `list_models`/catálogo do provider).
- `index [path] [--watch]` — chama `core::workspace::open`; renderiza progresso via
  `shared_progress`/`IndexProgress` com `indicatif`.
- `search <query> [--limit N]` — `IndexDb::open` + `encode_query` + `search_hybrid`;
  imprime resultados (path:line + score), respeitando o envelope `{mode,note,results}`.
- `run -m brain|builder "mensagem" [--effort low..max] [--yes]` — one-shot
  não-interativo: chama `core::run::drive` com um `StdoutSink` que renderiza
  `AgentEvent` linha-a-linha (`[thinking]`, `[tool]`, texto em streaming). `--yes`
  auto-aprova tools.
- `sessions list|load <id>` — reusa `persist::list_sessions`/`load_records`.
- **Checkpoint**: `claudinio auth status`, `index .`, `search`, `run -m brain "..."`
  funcionam ponta a ponta.

### Fase 5 — Comando `chat`: TUI ratatui minimalista
Uma view só, minimalista:
- **Layout**: painel de transcript rolável (topo) + linha de input (baixo) +
  barra de status (mode brain/builder, modelo, effort, tokens/context warning).
- **Stream**: um `TuiSink` empurra `AgentEvent` para um `tokio::mpsc`; o event loop
  do ratatui consome e renderiza: `TextDelta` (texto), `Thinking` (dim/colapsável),
  `ToolCall`/`ToolResult` (colapsado, expande com tecla), `AskUser` (prompt inline),
  `SessionLinked` (banner de handoff — brain→builder aparece como uma thread só),
  `GoldenLoop`, `SessionStats`.
- **Interação**: Enter envia; aprovação de tool = prompt y/n inline (ou `--yolo`);
  Ctrl+C = interrupt (via `SteeringCtl`); comando/atalho para trocar mode e effort;
  Esc/`:q` sai. Steering: digitar enquanto roda enfileira via `queue_steering`.
- Handoff e effort saem de graça: `core::run::drive` já segue handoffs; effort é o
  `AgentConfig.thinking_effort` global (ajustável em runtime pela barra de status).
- **Checkpoint**: `claudinio chat` roda uma sessão brain que planeja, faz handoff
  para builder e executa, com streaming visível.

### Fase 6 — Modelo de embeddings + build/dist
- Mover `src-tauri/models/all-MiniLM-L6-v2/` para `core/models/` (fonte única);
  `tauri.conf.json` do desktop aponta para o novo caminho como resource.
- CLI resolve o modelo via `core::paths`: dev usa `CARGO_MANIFEST_DIR/../core/models`;
  senão cache em `dirs::data_dir()`; senão `embeddings::ensure_model_downloaded`
  (fetch sha256-pinado já existente). No dia 1, download-on-first-run é suficiente.
- `.cargo/config.toml` (`+crt-static` no `x86_64-pc-windows-msvc`) permanece na
  raiz e passa a valer para o workspace todo — necessário para o link ORT/CRT.
- Distribuição do CLI: `cargo build --release -p claudinio-cli`. (Bundlar o modelo
  no instalador do CLI é um follow-up; ver memória `onnx-embeddings-gotchas`.)

## Arquivos-chave

**A modificar / mover:**
- `Cargo.toml` (raiz, novo `[workspace]`)
- `src-tauri/Cargo.toml` → deps do core saem, vira `desktop`
- `src-tauri/src/agent/{session,provider,provider/openai,subagent,transition}.rs`
  → `Channel<AgentEvent>` → `&dyn EventSink`
- `src-tauri/src/commands/agent.rs:391-482` (`spawn_run_loop`) → `core::run::drive`
- `src-tauri/src/commands/code_intel.rs:31-76` (helpers de path) → `core::paths`
- `src-tauri/src/commands/{auth,providers}.rs` (loopback OAuth) → `core::auth`

**A criar:**
- `core/Cargo.toml`, `core/src/lib.rs` (re-export de agent, code_intel, state,
  provider, paths, run, auth, `EventSink`)
- `cli/Cargo.toml`, `cli/src/main.rs` (clap), `cli/src/tui/` (ratatui),
  `cli/src/sink.rs` (`StdoutSink`, `TuiSink`), `cli/src/commands/*`

**Reusar como estão (sem tocar na lógica):**
- `code_intel/db.rs` (`search_hybrid`), `code_intel/embeddings.rs`,
  `code_intel/indexer.rs` (params `Option` já suportam `None`)
- `agent/provider.rs` (config, effort buckets), `agent/persist.rs`, `state.rs`,
  `agent/tools/mod.rs` (`ToolContext` sem campos Tauri)

## Verificação (ponta a ponta)

1. **Core isolado**: `cargo build -p claudinio-core` compila sem nenhum crate Tauri
   na árvore (`cargo tree -p claudinio-core | grep -i tauri` = vazio).
2. **Desktop não regrediu**: `pnpm tauri build` (ou `cargo build -p desktop`) OK;
   rodar o app e confirmar que uma sessão brain→builder com handoff ainda streama
   idêntico (mesma `AgentEvent`).
3. **Testes existentes**: `cargo test` do workspace passa (grill-me cache, eval de
   busca híbrida com `--sweep`, etc.).
4. **CLI subcomandos**:
   - `claudinio auth login` → abre browser, completa OAuth, `auth status` mostra logado.
   - `claudinio index .` → barra de progresso, cria `index.db` em `data_dir/indexes/`.
   - `claudinio search "hybrid retrieval"` → resultados com path:line + score.
   - `claudinio run -m brain "explique X" --yes` → streaming de thinking/tool/texto.
5. **TUI**: `claudinio chat` → sessão brain que gera plano, faz handoff para builder
   e executa; aprovação de tool interativa; interrupt com Ctrl+C; troca de effort na
   barra de status reflete no próximo turno.
6. **Paridade de melhoria**: uma mudança pontual em `core` (ex.: novo min_cos na busca)
   aparece tanto no `claudinio search` quanto no app sem edição duplicada.

## Fase 7 — Distribuição via `npx` (npm)

Objetivo: `npx claudinio` funciona em macOS/Linux/Windows sem o usuário ter Rust.
Padrão usado por esbuild/Biome/turbo: **pacote launcher + binários pré-compilados
por plataforma como `optionalDependencies`** (não postinstall-download, que quebra
com `npm --ignore-scripts` e ambientes corporativos).

### Estrutura de pacotes npm
- **`claudinio`** (pacote principal, publicado): `package.json` com `bin` apontando
  para um shim JS de ~20 linhas, e todas as plataformas como `optionalDependencies`.
  O npm instala só a que casa com `os`/`cpu`.
- **`@claudinio/cli-<os>-<cpu>`** (um por target): contém APENAS o binário
  pré-compilado + um `package.json` mínimo com `"os"` e `"cpu"` (filtro do npm):
  - `@claudinio/cli-darwin-arm64`, `-linux-x64`, `-linux-arm64`,
    `-win32-x64`, `-win32-arm64` (as 5 do matrix atual do `release.yml`).
- **Shim JS** (`bin/claudinio.mjs`): resolve `require.resolve('@claudinio/cli-'+
  plat+'/bin/claudinio'+ext)` e faz `spawnSync` passando `process.argv`/stdio.
  Assim `npx claudinio ...` e o `claudinio` instalado global funcionam igual.

Nome: **`claudinio` (nome puro)** — decidido. `npx claudinio` funciona direto, sem
escopo. Pré-requisito: `npm view claudinio` para confirmar disponibilidade e
registrar o nome antes do primeiro publish. Se estiver ocupado, fallback para
`@claudinio/cli` com `bin: { claudinio }`.

### Modelo e tamanho do pacote
- **Não bundlar o MiniLM (~23MB) no npm.** No primeiro uso, `embeddings::
  ensure_model_downloaded` (já existe, sha256-pinado) baixa para `dirs::data_dir()`.
  Comando `claudinio index` dispara o download com barra de progresso. Mantém os
  pacotes npm pequenos.
- **ONNX Runtime**: `ort` com `download-binaries` (sem `load-dynamic`) linka o
  onnxruntime **estaticamente** → binário auto-contido (verificar no build; se for
  dinâmico, incluir a `.dylib/.so/.dll` ao lado do binário em cada pacote de plataforma).
- O binário linka ~90 gramáticas tree-sitter + onnxruntime → provável 50–100MB por
  pacote de plataforma. Aceitável (o npm só instala 1). Feature-gate de gramáticas
  é um follow-up opcional se quiser enxugar.

### Mudanças no CI (`.github/workflows/release.yml`)
- No job `build` (matrix já cobre os 5 targets), adicionar step:
  `cargo build --release -p claudinio-cli --target ${{ matrix.target }}` e
  `upload-artifact` do binário cru (`claudinio`/`claudinio.exe`).
- Novo job `publish-npm` (`needs: build`): baixa os 5 binários, monta os pacotes
  `@claudinio/cli-*` + o launcher `claudinio` (versão = tag, igual ao resto do
  release), e `npm publish --access public`. Requer secret `NPM_TOKEN` e uma org/scope
  `@claudinio` no npm.
- macOS: binário via npm não pega quarantine do Gatekeeper (não vem do browser),
  mas reaproveitar a assinatura Apple já existente no CI é recomendável.

### Verificação (Fase 7)
- `npm pack` local de cada pacote e `npx ./claudinio-*.tgz --version`.
- Após publish de teste (tag `-rc`): `npx claudinio@<rc> search "..."` numa máquina
  limpa sem Rust, em macOS arm64 e Linux x64.

## Riscos / notas

- A refatoração `EventSink` (Fase 2) é o item de maior risco — toca no hot path do
  agente. Fazer com desktop rodando lado a lado para diffar a stream.
- `tauri-build`/`generate_context!` precisam ficar SÓ na crate `desktop`; garantir
  que nada em `core` referencie macros Tauri.
- Preservar as flags de Windows: `tokenizers[onig]` sem `esaxx_fast` e o
  `+crt-static` — regressão aqui quebra o link do ORT no MSVC.
- ONNX via `ort download-binaries`: o CLI herda o download em build-time de graça;
  runtime resolve/baixa o MiniLM.


## Implementation Log — 2026-07-20 22:33
**Summary:** Commit 2 changes: extract attachments to core, add interactive TUI module
**Changed files:** M	Cargo.lock, M	cli/Cargo.toml, M	cli/src/commands/chat.rs, M	cli/src/main.rs, A	cli/src/tui/app.rs, A	cli/src/tui/diff.rs, A	cli/src/tui/editor.rs, A	cli/src/tui/event.rs, A	cli/src/tui/footer.rs, A	cli/src/tui/markdown.rs, A	cli/src/tui/mod.rs, A	cli/src/tui/overlays.rs, A	cli/src/tui/render.rs, A	cli/src/tui/theme.rs, A	cli/src/tui/transcript.rs, A	core/src/agent/attachments.rs, M	core/src/agent/mod.rs, M	src-tauri/src/commands/agent.rs
**Commits:** fb58594 feat(cli/tui): add inline-render interactive chat TUI, 59214e7 refactor(attachments): move attachment building from Tauri layer to core
**Journal:** Split into two atomic commits: (1) refactor extracting attachment processing from Tauri layer into core::agent::attachments — the shared function means CLI/TUI and desktop app build identical content blocks, no duplicated logic. (2) feat adding the full TUI module (11 new files) replacing inline chat.rs with delegation, adding tui-textarea dep, defaulting to Chat when no subcommand given. Build verified clean before both commits, working tree clean after.

**Task journal:**
- Commit attachment extraction to core: Commit 59214e7 — 3 files changed, +221/-143
- Commit new TUI module + deps: Commit fb58594 — 15 files changed, +3144/-468 (11 new TUI module files)
