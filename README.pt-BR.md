<p align="center">
  <img src="docs/assets/logo.png" alt="Claudinio Code" width="128">
</p>

<h1 align="center">Claudinio Code</h1>

<p align="center">
  <strong>Um harness desktop nativo para agentes de IA.</strong><br>
  Inteligência de código de verdade, uma timeline visível do raciocínio e aprovação explícita para tudo que toca sua máquina.
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <a href="https://github.com/claudin-io/claudinio-code/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/claudin-io/claudinio-code/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/claudin-io/claudinio-code-releases/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/claudin-io/claudinio-code-releases?label=download"></a>
  <img alt="Platforms" src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey">
</p>

<p align="center">
  <a href="README.md">English</a> ·
  <a href="README.pt-BR.md">Português</a>
</p>

---

Claudinio Code é um aplicativo desktop — não um wrapper de terminal, não uma aba
do navegador, não uma extensão parafusada no editor de outra pessoa. Ele roda um
loop de agente em Rust, indexa seu workspace com tree-sitter e embeddings ONNX
locais, e renderiza cada pensamento, chamada de ferramenta e subagente em uma
timeline que você pode inspecionar.

Fala com qualquer API compatível com a Messages API da Anthropic.
[claudin.io](https://claudin.io) é o padrão; Anthropic, OpenRouter, DeepSeek e o
que mais estiver no catálogo do [models.dev](https://models.dev) também funcionam.

## Instalação

Baixe o instalador da sua plataforma em
[**Releases**](https://github.com/claudin-io/claudinio-code-releases/releases/latest):

| Plataforma | Artefato |
|---|---|
| macOS (Apple Silicon) | `Claudinio-Code-macOS-arm64-*.dmg` |
| Windows (x64) | `Claudinio-Code-Windows-x64-*.exe` / `*.msi` |
| Windows (ARM64) | `Claudinio-Code-Windows-arm64-*.exe` |
| Linux (x64) | `Claudinio-Code-Linux-x64-*.AppImage` / `*.deb` |
| Linux (ARM64) | `Claudinio-Code-Linux-arm64-*.AppImage` / `*.deb` |

O app se atualiza sozinho: os artefatos são assinados e o updater embutido
verifica a assinatura antes de instalar.

Build a partir do código-fonte está em [CONTRIBUTING.md](CONTRIBUTING.md).

## Primeiros passos

1. **Abra uma pasta.** A indexação começa na hora — a busca por palavra-chave já
   funciona de imediato, o ranking semântico entra assim que os embeddings terminam.
2. **Conecte um provedor.** Configurações → API key e base URL (padrão:
   `https://api.claudin.io`), ou faça login no OpenRouter via OAuth.
3. **Descreva a tarefa.** O agente planeja, chama ferramentas e transmite o
   raciocínio na timeline.
4. **Aprove o que importa.** Comandos de shell e edições de arquivo param para
   sua aprovação; edições mostram um diff no Monaco antes de escrever qualquer coisa.

## O que o torna diferente

### Brain e Builder

Dois modos com superfícies de ferramenta diferentes. O **Brain** explora em modo
somente-leitura, te entrevista sobre os requisitos e escreve um plano. O
**Builder** executa — escrevendo código, rodando comandos, verificando. O handoff
entre eles abre uma sessão nova semeada com o plano, para a execução não herdar
uma janela de contexto cheia de exploração.

### Ele lê o seu codebase de verdade

Não é só grep:

- **Indexação tree-sitter** em **77 linguagens** — símbolos, assinaturas, doc
  comments e relações de chamada, em SQLite.
- **Busca híbrida** — BM25 sobre código, docs e caminhos, fundido com embeddings
  semânticos MiniLM via reciprocal rank fusion. Acha uma fila de mensagens
  descrevendo o que ela faz, e acha `TOKENIZERS_PARALLELISM` pela grafia exata.
- **Embeddings rodam localmente.** O `all-MiniLM-L6-v2` vem dentro do instalador
  como resource ONNX. Seu código nunca é enviado a lugar nenhum para ser indexado.
- **LSP** — go-to-definition, find-references e hover via
  `typescript-language-server` e `rust-analyzer`.
- **Reindexação viva** — um watcher mantém o índice em dia enquanto você trabalha.

### Subagentes paralelos

O agente principal delega para subagentes paralelos (4 por padrão, configurável),
cada um com contexto fresco e sua própria timeline, que você pode abrir e ler.
Os modos são `explore` (somente leitura) ou `code`.

### Nada é caixa-preta

Cada turno do assistente vira uma timeline colapsável — fases, pensamento,
chamadas de ferramenta, resultados, subagentes, contagem de tokens e custo. As
sessões são persistidas em JSONL em `.claudinio/sessions/`, então dá para
reabrir e continuar qualquer conversa.

### Steering, não só interrupção

Digite enquanto o agente trabalha: sua mensagem é enfileirada e injetada como
orientação no meio do raciocínio. `Esc` interrompe de vez.

### Extensível

- **Skills** — jogue um `SKILL.md` em `.agents/skills/`, `.claudinio/skills/` ou
  `.claude/skills/` e o agente descobre sozinho.
- **MCP** — conecte servidores Model Context Protocol via stdio ou HTTP.

A interface e o agente são apenas em inglês. Os system prompts são escritos e
ajustados em inglês, e o agente pede que você também escreva em inglês — uma
casca localizada em volta de um agente que só fala inglês era pior do que ser
direto sobre isso. (Este README é uma cortesia de documentação, não uma
tradução da UI.)

## As ferramentas que o modelo pode chamar

| Ferramenta | Aprovação | O que faz |
|---|---|---|
| `read_file` | automático | Lê um arquivo de texto dentro do workspace |
| `list_dir` | automático | Lista um diretório, respeitando `.gitignore` |
| `grep` | automático | Busca regex (ripgrep) |
| `code_search` | automático | Busca full-text de símbolos (SQLite FTS5) |
| `semantic_search` | automático | Busca híbrida BM25 + embeddings |
| `symbol_lookup` | automático | Lookup exato de símbolo |
| `file_outline` | automático | Símbolos definidos em um arquivo |
| `go_to_definition` | automático | Definição via LSP |
| `find_references` | automático | Referências via LSP |
| `web_search` | automático | Busca na web |
| `ask_user` | automático | Faz uma pergunta a você, com opções |
| `tasks_get` / `tasks_set` | automático | Lê e atualiza a lista de tarefas |
| `write_plan` / `finalize_plan` | automático | Escreve e fecha um documento de plano |
| `enter_plan_mode` / `exit_plan_mode` | automático | Alterna entre Brain e Builder |
| `spawn_agents` | automático | Dispara subagentes paralelos |
| **`edit_file`** | **requer aprovação** | Propõe uma edição, exibida como diff |
| **`bash`** | **requer aprovação** | Roda um comando de shell (comandos de leitura são allowlisted) |

As ferramentas de arquivo ficam confinadas ao workspace aberto por um guard de
caminho que rejeita traversal. Uma denylist bloqueia comandos sabidamente
destrutivos. O modelo de ameaças completo — incluindo o que o projeto
explicitamente *não* defende — está em [SECURITY.md](SECURITY.md).

## Arquitetura

```
┌──────────────────────────────────────────────────────────┐
│  Frontend — SolidJS + TypeScript + Tailwind              │
│  timeline · aprovações · diffs Monaco · árvore de arquivos│
└───────────────────────────┬──────────────────────────────┘
                            │ Tauri IPC
┌───────────────────────────┴──────────────────────────────┐
│  Backend — Rust / Tauri v2                               │
│                                                          │
│  agent/        loop, provedores (SSE), ferramentas,      │
│                subagentes, permissões, skills, MCP       │
│  code_intel/   tree-sitter → SQLite FTS5 + vetores ONNX  │
│  lsp/          typescript-language-server, rust-analyzer │
│  commands/     superfície IPC exposta ao frontend        │
└──────────────────────────────────────────────────────────┘
```

O fluxo de uma mensagem: IPC `send_message` → loop de sessão → stream SSE do
provedor → `AgentEvent`s por um channel → timeline renderiza ao vivo → gates de
aprovação resolvem por oneshot channels → resultados alimentam o round seguinte
→ `Done`, persistido em JSONL.

## Construído com

[Tauri v2](https://v2.tauri.app) ·
[Rust](https://www.rust-lang.org) ·
[SolidJS](https://www.solidjs.com) ·
[TypeScript](https://www.typescriptlang.org) ·
[Tailwind CSS v4](https://tailwindcss.com) ·
[Monaco Editor](https://microsoft.github.io/monaco-editor/) ·
[tree-sitter](https://tree-sitter.github.io/tree-sitter/) ·
[SQLite / rusqlite](https://github.com/rusqlite/rusqlite) ·
[ONNX Runtime](https://github.com/pykeio/ort) ·
[tokio](https://tokio.rs)

## Ícones

Os ícones da interface são paths SVG portados à mão, obtidos pelo
[Icônes](https://icones.js.org), um navegador dos conjuntos de ícones
open-source agregados pelo [Iconify](https://iconify.design). Obrigado às
coleções e seus autores:

| Coleção | Autor | Licença |
| --- | --- | --- |
| [Lucide](https://icones.js.org/collection/lucide) | Lucide Contributors | ISC |
| [Codicons](https://icones.js.org/collection/codicon) | Microsoft | CC BY 4.0 |
| [Carbon](https://icones.js.org/collection/carbon) | IBM | Apache 2.0 |
| [Pixel Icon Library](https://icones.js.org/collection/pixel) | HackerNoon | CC BY 4.0 |
| [Octicons](https://icones.js.org/collection/octicon) | GitHub | MIT |
| [Bootstrap Icons](https://icones.js.org/collection/bi) | The Bootstrap Authors | MIT |
| [Hugeicons](https://icones.js.org/collection/hugeicons) | Hugeicons | MIT |
| [Pixelarticons](https://icones.js.org/collection/pixelarticons) | Gerrit Halfmann | MIT |
| [Dinkie Icons](https://icones.js.org/collection/dinkie) | atelierAnchor | MIT |
| [Game Icons](https://icones.js.org/collection/game-icons) | GameIcons | CC BY 3.0 |
| [Streamline Ultimate](https://icones.js.org/collection/streamline-ultimate) | Streamline | CC BY 4.0 |

## Contribuindo

Issues e pull requests são bem-vindos — comece pelo
[CONTRIBUTING.md](CONTRIBUTING.md) para o setup de desenvolvimento e os checks
que a CI roda. Problemas de segurança vão pelo [SECURITY.md](SECURITY.md), nunca
por uma issue pública. A participação é regida pelo
[Código de Conduta](CODE_OF_CONDUCT.md).

## Licença

[MIT](LICENSE) © Victor Carvalho Tavernari
