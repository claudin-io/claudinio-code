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

**Em CPU sem AVX** — Intel Celeron e Pentium N/J, Atom, Core anteriores a 2013 —
os builds x64 acima abortam no startup (`Illegal instruction` no Linux,
`0xc000001d` no Windows). O ONNX Runtime, que move a busca semântica, traz
kernels que essas CPUs não conseguem decodificar. Baixe o artefato `-baseline`:

| Plataforma | Artefato |
|---|---|
| Windows (x64, sem AVX) | `Claudinio-Code-Windows-x64-baseline-*.exe` |
| Linux (x64, sem AVX) | `Claudinio-Code-Linux-x64-baseline-*.AppImage` / `*.deb` |

É o mesmo app com um backend de embeddings em Rust puro — mesmo modelo, mesmos
resultados, indexação mais lenta — e ele não é servido pelo updater automático,
então baixe de novo para atualizar.

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
somente-leitura, te entrevista sobre os requisitos e escreve um plano, sem nunca
gravar um arquivo. O **Builder** executa — escrevendo código, rodando comandos,
verificando. O handoff entre eles abre uma sessão nova semeada com o plano, para
a execução não herdar uma janela de contexto cheia de exploração.

O Builder edita os arquivos ele mesmo: cada `edit_file` aparece para você como
um diff a aprovar, e um comando de shell que grava um arquivo é perguntado do
mesmo jeito, mesmo quando o prefixo está na allowlist. Ele só passa trabalho a
subagentes quando compensa — um subagente explore para uma investigação ampla
somente-leitura ou pesquisa em paralelo, subagentes code para duas ou mais
mudanças em arquivos disjuntos — porque uma edição delegada custa um segundo
prompt, uma segunda exploração e a especificação escrita duas vezes.

Você escolhe o modo, ou deixa o harness escolher. Uma sessão nova começa em
**Auto** onde o modelo de decisão por trás dele (o Jev) estiver disponível: a
primeira mensagem é roteada — um pedido que precisa de decisões e de um design
começa em Brain, todo o resto em Builder — e você ainda pode escolher qualquer
um dos dois na mão. O Auto envia essa primeira mensagem ao Jev; em Settings ›
Agent dá para desligá-lo, ou passá-lo para *Shadow*, que só registra o que o
Auto teria escolhido ao lado do que você escolheu.

### Ele se ajusta ao modelo que você escolheu

Todo limite de uma execução vem da janela do modelo com que ela está falando,
e não de um número único para todos. Um modelo de 200k se comporta como sempre.
Um modelo local de 32k recebe limites na escala de 32k — quando descartar
resultados antigos de ferramentas, quando compactar, o tamanho máximo de uma
leitura de arquivo — em vez de requisições várias vezes maiores que ele.

- **O contexto é enxugado antes de ser resumido.** Primeiro o que é cópia de
  algo mais novo (uma leitura anterior do mesmo arquivo, uma lista de tarefas
  anterior), depois o que um modelo de decisão pequeno julga não ser mais
  necessário, depois simplesmente os resultados de ferramenta mais antigos.
  Nessa etapa nem as suas palavras nem as do modelo são cortadas; só quando ela
  não basta a sessão faz handoff ou compacta.
- **Janela pequena recebe prompt pequeno.** Abaixo de 64k tokens, uma sessão
  Builder roda um perfil compacto — um prompt curto e dez ferramentas, sem
  subagentes e sem planos — cerca de 2,4k tokens antes de a conversa começar, onde o prompt e as ferramentas do perfil Standard ocupam
  cerca de 4,8k.
- **Ferramentas e skills chegam quando são necessárias.** Uma sessão Builder
  Standard envia treze ferramentas centrais; as demais — as buscas do índice e do LSP, as
  ferramentas de web e browser, a troca para o modo de plano, a leitura completa
  das tarefas e toda ferramenta MCP — aparecem no prompt em uma linha cada, e o modelo carrega as que a
  tarefa pede (`load_tools`, ou simplesmente chamando uma). Uma ferramenta
  carregada é acrescentada ao fim e fica pela sessão, então o prefixo em cache
  só cresce no final. As skills são listadas do mesmo jeito, uma linha cada, e
  lidas com a ferramenta `skills`. Com dez skills e um servidor MCP de vinte
  ferramentas o prefixo fica em cerca de 6,3k tokens, onde antes passava de 17k.
- **A lista de tarefas vem até o agente, e não o contrário.** A mensagem de
  cada execução termina com a lista de tarefas da sessão, uma linha por tarefa
  (`id · status · título`), então nenhum pedido é gasto relendo-a; o agente
  marca uma tarefa como em andamento no mesmo pedido da primeira chamada de
  ferramenta dela, e move várias tarefas em um só `tasks_update`. Trabalho de
  um passo só não precisa de lista, e o painel de tarefas mostra *Single step*.
  Uma compactação repete a lista depois do resumo, e um turno que termina com
  uma tarefa ainda em andamento recebe um lembrete antes de poder encerrar.
- **Ferramentas por modelo.** Em Settings › Models um modelo pode ser posto em
  *Lean*, que deixa de fora as três ferramentas de navegação por LSP e as
  linhas do prompt sobre elas. Nada é Lean por padrão: se menos ferramentas
  ajudam é uma propriedade do modelo, e precisa ser testado nele.

`scripts/session-stats.py` lê o seu próprio `.claudinio/sessions/` e mostra
para onde o contexto foi — quais resultados foram carregados por mais tempo, o
tamanho do prefixo fixo, quantas vezes um arquivo foi lido duas vezes — para
que essas escolhas sejam feitas sobre o que as suas sessões realmente fizeram.

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

### Ele enxerga sua aplicação rodando

Ler o código só resolve metade. O agente dirige um browser de verdade: muda um
componente, olha o que renderizou, lê o erro no console e corrige — sem você
ficar copiando e colando de um lado para o outro.

<p align="center">
  <img src="docs/assets/browser.png" alt="O agente navegando numa página e devolvendo um screenshot inline na timeline" width="880">
</p>

- **Screenshots voltam como imagem**, não como caminho de arquivo — o modelo
  enxerga o layout de fato. Captura o viewport, a página inteira, um seletor CSS
  ou uma região arbitrária.
- **Console e network** — `console.*`, exceções não capturadas, violações de CSP
  e subrecursos que falharam, mais cada request com status, tamanho e tempo. As
  leituras trazem só o que é novo desde a última, então ler em loop sai barato.
- **Interação** — clicar, digitar, teclar, focar, rolar até um elemento, esperar
  um aparecer.
- **Um browser dedicado, nunca o seu.** Um build fixado do Chrome for Testing,
  verificado por checksum, rodando com diretório de perfil próprio. Seu perfil do
  Chrome — e as sessões logadas nele — nunca fica exposto a uma página que o
  agente visita.
- **Navegação é barrada por origem.** `localhost` e sua rede local são
  automáticos; qualquer outra coisa pede aprovação. `file://` e `javascript:` são
  recusados, e não existe válvula de escape para JS arbitrário.

O browser é um download único de ~190 MB, iniciado em **Configurações › Browser**
— nunca silenciosamente por uma tool call. Desligue a feature ali e as tools
somem inteiramente do prompt.

### Subagentes paralelos

O agente principal pode delegar para subagentes paralelos (4 por padrão,
configurável), cada um com contexto fresco e sua própria timeline, que você pode
abrir e ler. Os modos são `explore` (somente leitura) ou `code`. Um subagente
explore é instruído a entregar um relatório curto — achados, referências
`caminho:linha`, perguntas em aberto — e o contexto do agente principal carrega
no máximo 6.000 caracteres de qualquer relatório (o meio de um mais longo é
cortado; a timeline guarda tudo). Todo subagente code de uma chamada parte do
mesmo briefing: a tarefa em andamento, o caminho do plano e os arquivos que a
sessão já leu, como caminhos.

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
- **Plugins** — instale [Agent Plugins](https://agent-plugins.org), pacotes
  portáveis que juntam skills e servidores MCP.

#### Agent Plugins

O Claudinio Code é um cliente conformante do Agent Plugins v1.0.0. Um plugin é um
diretório com o manifesto `plugin.json` e dois locais de componentes opcionais:

```text
meu-plugin/
├── plugin.json          # manifesto obrigatório
├── skills/              # cada subdiretório com SKILL.md é uma skill
│   └── summarize/SKILL.md
└── mcp.json             # servidores stdio e Streamable HTTP
```

Abra **Settings › Plugins** para ver o que está instalado e adicionar mais:

- **Install from folder** — aponte para um diretório no disco.
- **Install from URL** — uma URL git ou GitHub. Uma URL de *tree* como
  `https://github.com/owner/repo/tree/main/plugins/deploy` já carrega o branch e
  o subdiretório, então plugins dentro de um monorepo instalam sem esforço extra.
- **Create plugin** — gera um pacote em conformidade com a spec. Para algo mais
  elaborado, peça ao agente: a skill embutida `plugin-crafter` escreve e valida.

Plugins ficam em `~/.claudinio/plugins/` (ou no `.claudinio/plugins/` do projeto)
e também são descobertos em `.agents/plugins/` e `.claude/plugins/`. As skills do
plugin entram no catálogo; os servidores MCP conectam como `<plugin>.<servidor>`,
cada um com `PLUGIN_ROOT` e um `PLUGIN_DATA` persistente. Um plugin desativado
não contribui com nada, e o estado de ativação vive na sua config — nunca nos
arquivos do autor do plugin. Um plugin também pode vir desligado, declarando
`enabledByDefault` no namespace de extensão `io.claudin.claudinio`.

O que um plugin contribui sempre fica abaixo do que você mesmo escreveu: as
skills dele ficam atrás das suas skills de projeto, subpasta e usuário, então um
plugin nunca sobrepõe uma sua, e um servidor que você configurou à mão vence um
servidor de plugin com o mesmo nome. Instalar ou alternar um plugin reconecta o
MCP sozinho — sem reiniciar.

Falhas são contidas como a spec exige: um manifesto inválido rejeita só aquele
plugin, enquanto uma skill ou um servidor inválido pula apenas aquela entrada. O
explorador mostra exatamente o que foi pulado e por quê.

A interface é em inglês. O agente não: escreva para ele em qualquer idioma e ele
responde no idioma da sua última mensagem. (Este README é uma cortesia de
documentação, não uma tradução da UI.)

### Ele roda os seus hooks

O Claudinio implementa o **protocolo de hooks do Claude Code** — os nove
eventos, campo por campo — então um bloco `hooks` que você já escreveu funciona
aqui sem edição. Ele lê `~/.claude/settings.json`, o `.claude/settings.json` e o
`settings.local.json` do projeto, uma chave `hooks` no `.claudinio.json` e o
`hooks/hooks.json` de qualquer plugin instalado. Toda fonte contribui; nenhuma
sobrepõe a outra.

`PreToolUse` pode permitir, perguntar ou negar uma chamada de ferramenta.
`UserPromptSubmit` e `SessionStart` podem injetar contexto. `PreCompact` dispara
antes de uma compactação *e* antes de um handoff de contexto, que é o momento de
que um hook de memória realmente precisa. Matchers escritos com os nomes de
ferramenta do Claude Code (`Edit|Write`, `Bash`, `Task`) selecionam as
ferramentas certas do Claudinio por uma tabela de aliases.

Como um hook é código arbitrário que um repositório pode trazer junto, nada roda
até você aprovar aquele conjunto exato de comandos uma vez — a aprovação é um
hash, então editar um comando a revoga. Um hook pode dispensar um pedido de
aprovação; nunca pode dispensar uma política: um `allow` sobre um comando de
shell na denylist continua negado.

Referência completa em [docs/HOOKS.md](docs/HOOKS.md).

### Ele pode rodar o modelo na sua máquina

Ajustes → **Local models** baixa o llama.cpp e um modelo GGUF do Hugging Face, e
o modelo passa a aparecer nos seletores de Brain e Builder em *Local*. Nada sai
do computador e nada é cobrado — útil para código que não pode ir para terceiros,
para trabalhar offline e para não pagar por token enquanto você experimenta.

A lista recomendada é curta de propósito: cada modelo nela foi verificado como
capaz de conduzir o loop de ferramentas do agente, algo sobre o qual a maioria
dos repositórios GGUF não diz nada. Você também pode buscar qualquer repositório
GGUF direto no Hub; a tabela de quantizações mostra o tamanho de cada opção
contra a memória da sua máquina e pré-seleciona a melhor que cabe. Os downloads
são verificados contra os próprios checksums do Hugging Face e retomam arquivo a
arquivo.

Seja honesto quanto à troca: um 8B local não é um modelo de fronteira hospedado,
e um modelo que não sustenta um loop de tool calling vai patinar. O botão
**Test**, ao lado de um modelo instalado, diz se ele funciona antes de você
descobrir no meio da tarefa.

No Apple Silicon ele roda com **MLX**, o framework da própria Apple, que gera
mais rápido que o llama.cpp com os mesmos pesos; em qualquer outro lugar — e no
Mac, se você preferir — o llama.cpp lê GGUF. O engine é uma configuração, e cada
um tem seus próprios downloads, porque usam formatos de modelo diferentes.

A barra de status mostra o que um modelo carregado está custando enquanto roda:
memória residente, contexto usado contra contexto servido, e a taxa de tokens.

## As ferramentas que o modelo pode chamar

| Ferramenta | Aprovação | Enviada | O que faz |
|---|---|---|---|
| `read_file` | automático | sempre | Lê um arquivo de texto dentro do workspace |
| `list_dir` | automático | sempre | Lista um diretório, respeitando `.gitignore` |
| `grep` | automático | sempre | Busca regex (ripgrep) |
| `code_search` | automático | adiada | Busca full-text de símbolos (SQLite FTS5) |
| `semantic_search` | automático | adiada | Busca híbrida BM25 + embeddings |
| `symbol_lookup` | automático | adiada | Lookup exato de símbolo |
| `file_outline` | automático | adiada | Símbolos definidos em um arquivo |
| `go_to_definition` | automático | adiada | Definição via LSP |
| `find_references` | automático | adiada | Referências via LSP |
| `web_search` | automático | adiada | Busca na web |
| `browser_inspect` | automático | adiada | Lê console, network, texto ou HTML da página aberta |
| `browser_screenshot` | automático | adiada | Captura o viewport, a página inteira, um elemento ou uma região |
| `ask_user` | automático | sempre | Faz uma pergunta a você, com opções |
| `tasks_get` | automático | adiada | Descrição e diário de cada tarefa (cada execução já traz a lista) |
| `tasks_set` / `tasks_update` | automático | sempre | Reescreve a lista de tarefas, ou altera uma ou várias tarefas numa chamada |
| `run_quality` | automático | sempre | Roda os testes (e a cobertura) do projeto e registra o resultado |
| `write_plan` / `exit_plan_mode` | automático | sempre (Brain) | Escreve o plano; volta ao Builder |
| `finalize_plan` | automático | sempre (Builder) | Fecha o plano |
| `enter_plan_mode` | automático | adiada (Builder) | Passa para o Brain |
| `spawn_agents` | automático | sempre | Dispara subagentes paralelos |
| `skills` | automático | sempre | Lista as skills instaladas ou lê uma |
| `load_tools` | automático | sempre | Acrescenta ferramentas adiadas à lista pelo resto da sessão |
| Ferramentas MCP | **requer aprovação** | adiada | O que os servidores conectados oferecem |
| **`browser`** | **barrado por origem** | adiada | Navega e interage com uma página (endereços locais são automáticos; outras origens perguntam) |
| **`edit_file`** | **requer aprovação** | sempre (Builder, subagentes code) | Propõe uma edição, exibida como diff |
| **`bash`** | **requer aprovação** | sempre | Roda um comando de shell (comandos de leitura são allowlisted) |

*Enviada* é o que uma sessão Standard põe em todo pedido. Uma ferramenta
*adiada* aparece no prompt de sistema em uma linha, e sua definição entra no
pedido quando o modelo a carrega com `load_tools` — ou a chama, o que também a
carrega. Subagentes não adiam nada; só deixam de fora as três ferramentas LSP.

As chamadas às ferramentas só de leitura (`read_file`, `list_dir`, `grep`, as
buscas do índice e do LSP, `web_search` e `tasks_get`) que o modelo faz lado a
lado rodam ao mesmo tempo, no máximo quatro; todo o resto roda uma chamada de
cada vez, na ordem em que o modelo pediu.

As ferramentas de arquivo ficam confinadas ao workspace aberto por um guard de
caminho que rejeita traversal. Uma denylist bloqueia comandos sabidamente
destrutivos. Tudo que o browser lê de uma página é embrulhado e marcado como
dado não-confiável, porque uma página pode conter texto escrito para manipular o
modelo. O modelo de ameaças completo — incluindo o que o projeto explicitamente
*não* defende — está em [SECURITY.md](SECURITY.md).

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
