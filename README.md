# Claudinio Code 🧠⚡

**Claudinio Code** é um IDE agentivo — um aplicativo desktop nativo (Tauri v2) que integra um **agente de IA** diretamente no seu editor de código, com inteligência de código rica (busca semântica, LSP, indexação tree-sitter) e uma interface de chat com timeline visual, aprovação de ferramentas e suporte a subagentes paralelos.

> Stack: [Tauri v2](https://v2.tauri.app) + [SolidJS](https://www.solidjs.com) + [TypeScript](https://www.typescriptlang.org) + [Rust](https://www.rust-lang.org)

---

## Sumário

- [Funcionalidades](#funcionalidades)
  - [Interface do Agente de IA](#interface-do-agente-de-ia)
  - [Gerenciamento de Projetos](#gerenciamento-de-projetos)
  - [Inteligência de Código](#inteligência-de-código)
  - [Sistema de Agentes](#sistema-de-agentes)
  - [Ferramentas do Agente](#ferramentas-do-agente)
  - [Tema e Design](#tema-e-design)
- [Arquitetura](#arquitetura)
- [Começando](#começando)
- [Como Usar](#como-usar)
- [Tecnologias](#tecnologias)
- [Licença](#licença)

---

## Funcionalidades

### Interface do Agente de IA

- **Chat com timeline visual** — Cada resposta do assistente mostra uma **linha do tempo colapsável** com fases (Planejamento → Execução → Sumário), steps de texto, pensamentos, chamadas de ferramenta e resultados.
- **Streaming em tempo real** — O agente transmite texto, pensamento e ferramentas via SSE (Server-Sent Events) — você vê o raciocínio acontecer.
- **Aprovação de ferramentas** — Comandos `bash` e edições de arquivo exigem aprovação do usuário. O diff é exibido no Monaco Editor antes de aplicar.
- **Ask User** — O agente pode fazer perguntas interativas com opções de seleção única/múltipla e campo "Outra resposta".
- **Steering (orientação em tempo real)** — Enquanto o agente pensa, você pode digitar mensagens que são enfileiradas e injetadas como orientação no meio do raciocínio.
- **Interrupção** — Pressione `Esc` para interromper o agente a qualquer momento.
- **Subagentes** — O agente principal pode delegar tarefas para **até 4 subagentes paralelos**, cada um com seu próprio contexto e timeline. Você pode clicar em um subagente para ver sua timeline completa em um modal.
- **Histórico de sessões** — Todas as conversas são persistidas em JSONL. Você pode listar, reabrir e continuar sessões anteriores.
- **Markdown completo** — As respostas são renderizadas com Markdown (`marked` + `highlight.js`), incluindo tabelas, listas, citações, imagens e blocos de código com syntax highlighting.

### Gerenciamento de Projetos

- **Abrir workspace** — Selecione uma pasta do sistema via seletor nativo. O projeto é automaticamente indexado.
- **Explorador de arquivos** — Árvore de diretórios lazy-loaded (expansão sob demanda) com ícones e seleção.
- **Projetos recentes** — Até 10 projetos recentes persistidos no `localStorage`, exibidos na tela inicial e na sidebar.
- **Barra de progresso de indexação** — Mostra status em tempo real: carregamento do modelo, arquivos indexados, símbolos extraídos, embeddings gerados.

### Inteligência de Código

- **Busca textual de símbolos (FTS5)** — Busca full-text sobre nomes e assinaturas de símbolos usando SQLite FTS5.
- **Busca semântica (CodeBERT)** — Busca por conceito usando o modelo **LateOn-Code-edge** (ColBERT-style, ONNX INT8). Entenda o que o código *faz*, não apenas como se chama.
- **Lookup exato de símbolo** — Encontre um símbolo pelo nome exato.
- **Outline de arquivo** — Lista todos os símbolos definidos em um arquivo com posições.
- **Go to Definition** (LSP) — Navegue para a definição de um símbolo via LSP (TypeScript/JavaScript e Rust).
- **Find References** (LSP) — Encontre todas as referências a um símbolo.
- **Hover Info** (LSP) — Obtenha informações detalhadas sobre um símbolo sob o cursor.
- **Indexação tree-sitter** — Parsers para TypeScript/JS, Rust e Python extraem símbolos, assinaturas, doc comments e relações de chamada.
- **Watcher de arquivos** — O workspace é monitorado em tempo real. Arquivos alterados são reindexados automaticamente.
- **Relações entre símbolos** — A tabela `relations` armazena chamadas de função (quem chama quem).

### Sistema de Agentes

O núcleo do Claudinio Code é um **loop agente-ferramenta** completo implementado em Rust:

| Componente | Descrição |
|---|---|
| **Provider** | Conecta-se a uma API compatível com Anthropic Messages API (SSE streaming), com fallback, parsing de `content_block_delta` para tool calls parciais, e suporte a interrupção atômica. |
| **Session Loop** | Até 30 rounds de agente-ferramenta com injeção de steering, verificação de interrupção, merging de roles de usuário e persistência JSONL. |
| **Subagentes** | Até 4 agentes paralelos (modos `explore` — só leitura, ou `code` — pode editar). Cada um tem contexto fresco e limite de 15 rounds. |
| **Persistência** | Sessões salvas em `.claudinio/sessions/<id>.jsonl` (JSONL). Cada registro é um `SessionRecord` com kind (meta, user, turn, steering, phase, done, error). |
| **Permissões** | Três níveis: `Auto` (leitura), `RequiresApproval` (bash, edit), `Denied` (blacklist de comandos perigosos). Bash tem allowlist para comandos read-only. |

### Ferramentas do Agente

O agente tem acesso a **13 ferramentas** integradas:

| Ferramenta | Permissão | Descrição |
|---|---|---|
| `read_file` | ✅ Auto | Lê arquivo de texto (max 2MB, workspace apenas) |
| `list_dir` | ✅ Auto | Lista diretório (respeita .gitignore) |
| `grep` | ✅ Auto | Busca regex com ripgrep |
| `edit_file` | 🛡️ Requer aprovação | Proposta de diff com visualização no Monaco |
| `bash` | 🛡️ Requer aprovação | Comando shell (allowlist para leitura) |
| `code_search` | ✅ Auto | Busca textual FTS5 |
| `symbol_lookup` | ✅ Auto | Lookup exato de símbolo |
| `file_outline` | ✅ Auto | Outline de símbolos de um arquivo |
| `go_to_definition` | ✅ Auto | LSP go-to-definition |
| `find_references` | ✅ Auto | LSP find references |
| `semantic_search` | ✅ Auto | Busca semântica (CodeBERT) |
| `ask_user` | ✅ Auto | Perguntas interativas ao usuário |
| `spawn_agents` | ✅ Auto | Dispara subagentes paralelos |

### Tema e Design

- **Tema dark/light** — Detecta automaticamente a preferência do sistema operacional. Tema claro com fundo bege/warm, tema escuro com fundo `#141210`.
- **Design system completo** — 4 níveis de superfície, 3 níveis de tinta, cores de accent (laranja quente), success (verde) e danger (vermelho), sombras arredondadas.
- **Monaco Editor customizado** — Temas `claudinio-dark` e `claudinio-light` combinando com o design system.
- **Fontes auto-hospedadas** — Inter Variable (sans) e JetBrains Mono (mono).
- **Animações suaves** — `prefers-reduced-motion` respeitado, animações de entrada, pulso de status, cursor piscante.
- **Integração macOS** — Padding para botões de tráfego (`pl-[78px]`), `data-tauri-drag-region` para arrastar janela.

---

## Arquitetura

```
┌─────────────────────────────────────────────────────┐
│                   Frontend (SolidJS)                 │
│  ┌───────────┐ ┌──────────────┐ ┌────────────────┐ │
│  │ ChatPanel  │ │  App Shell   │ │  DiffViewer    │ │
│  │ (agent UI) │ │ (sidebar,    │ │  (Monaco diff) │ │
│  │ timeline,  │ │  config,     │ │                │ │
│  │ approvals) │ │  file tree)  │ │                │ │
│  └─────┬─────┘ └──────┬───────┘ └────────────────┘ │
│        │              │                             │
│        └──────┬───────┘                             │
│               │ IPC (Tauri commands)                 │
└───────────────┼─────────────────────────────────────┘
                │
┌───────────────┼─────────────────────────────────────┐
│    Backend (Rust / Tauri v2)                        │
│               │                                     │
│  ┌────────────▼──────────────────────────────┐      │
│  │         AppState (state.rs)               │      │
│  │  config, index_db, lsp_manager,           │      │
│  │  active_session, steering, embeddings     │      │
│  └───┬──────────┬──────────┬──────────┬──────┘      │
│      │          │          │          │              │
│  ┌───▼──┐ ┌─────▼────┐ ┌──▼───┐ ┌───▼────────┐     │
│  │Agent │ │Code Intel│ │ LSP  │ │ File System │     │
│  │system│ │ FTS5 +   │ │ TS + │ │ read/write/ │     │
│  │      │ │Semantic  │ │ Rust │ │ list        │     │
│  │      │ │Embeddings│ │ Analy│ │             │     │
│  │      │ │(ONNX)    │ │zer   │ │             │     │
│  └──────┘ └──────────┘ └──────┘ └─────────────┘     │
└──────────────────────────────────────────────────────┘
```

### Fluxo de uma mensagem

```
1. Usuário digita → Tauri command `send_message`
2. `session::run_workflow()` inicia
3. → Provider faz streaming SSE da API
4. → Eventos (texto, pensamento, tool calls) enviados via Channel<AgentEvent>
5. → Frontend renderiza timeline em tempo real
6. → Se tool call requer aprovação: frontend mostra modal, aguarda
7. → Usuário aprova/rejeita → oneshot channel resolve
8. → Resultado volta para o modelo → próximo round
9. → Quando turno terminal: Done emitido, sessão persistida
```

---

## Começando

### Pré-requisitos

- [Node.js](https://nodejs.org/) (v18+)
- [pnpm](https://pnpm.io/) (v9+)
- [Rust](https://rustup.rs/) (edição stable 2024+)
- [Tauri CLI](https://v2.tauri.app/start/cli/) (`cargo install tauri-cli`)

### Instalação

```bash
# Clone o repositório
git clone <url-do-repositorio>
cd claudinio-code

# Instale dependências do frontend
pnpm install

# Execute em modo de desenvolvimento
pnpm tauri dev
```

### Build

```bash
pnpm tauri build
```

---

## Como Usar

1. **Abra um projeto** — Clique em "Abrir pasta" na tela inicial ou selecione um projeto recente.
2. **Aguarde a indexação** — O Claudinio Code vai escanear, extrair símbolos (tree-sitter) e gerar embeddings (LateOn-Code ONNX). A barra de progresso mostra o status.
3. **Configure a API** — Clique no ícone de engrenagem e configure sua API Key e Base URL (padrão: `https://api.claudin.io`).
4. **Converse com o agente** — Digite sua tarefa em linguagem natural. O agente planeja, executa ferramentas e mostra tudo na timeline.
5. **Aprove ações** — Comandos bash e edições de arquivo aparecem para aprovação. Veja o diff e decida.
6. **Use subagentes** — Para tarefas complexas, o agente pode disparar subagentes — acompanhe cada um em seu próprio modal.
7. **Retome sessões** — O histórico fica salvo. Clique em "Histórico" para reabrir conversas passadas.

---

## Tecnologias

### Frontend

| Tecnologia | Uso |
|---|---|
| [SolidJS](https://www.solidjs.com) | UI reativa (sem Virtual DOM) |
| [TypeScript](https://www.typescriptlang.org) | Tipagem estática |
| [Tailwind CSS v4](https://tailwindcss.com) | Estilização utility-first |
| [Monaco Editor](https://microsoft.github.io/monaco-editor/) | Diffs e visualização de código |
| [highlight.js](https://highlightjs.org) | Syntax highlighting |
| [marked](https://marked.js.org) | Renderização Markdown |
| [Vite](https://vitejs.dev) | Build tool |

### Backend

| Tecnologia | Uso |
|---|---|
| [Tauri v2](https://v2.tauri.app) | Framework desktop nativo |
| [Rust](https://www.rust-lang.org) | Linguagem do backend |
| [SQLite (rusqlite)](https://github.com/rusqlite/rusqlite) | Banco de índice (FTS5) |
| [Tree-sitter](https://tree-sitter.github.io/tree-sitter/) | Parsers de código (TS, JS, RS, PY) |
| [ONNX Runtime (ort)](https://github.com/pykeio/ort) | Inferência do modelo de embeddings |
| [LateOn-Code-edge](https://huggingface.co/lightonai/LateOn-Code-edge) | Modelo de embeddings semânticos (ColBERT-style) |
| [tokio](https://tokio.rs) | Runtime assíncrono |
| [reqwest](https://docs.rs/reqwest) | Cliente HTTP (API do provedor + download) |
| [ripgrep (através de grep)](https://github.com/BurntSushi/ripgrep) | Busca regex |
| [notify](https://github.com/notify-rs/notify) | Watcher de arquivos |
| [diffy](https://github.com/notify-rs/diffy) | Geração de diffs textuais |

---

## Licença

MIT
