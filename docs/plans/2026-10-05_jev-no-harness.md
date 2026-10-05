# Jev no harness do Claudinio Code: estudo e plano

*2026-10-05. Este documento continua o `JEV_SYSTEM_ONE_STUDY.md` do repositório
claudinio-litellm, que é o estudo do Jev do lado do proxy. Fontes: o survey
online feito a 2026-10-05 (links no §2) e o mapa do código do claudinio-code no
HEAD `5f4c109` (0.3.1). Ainda não há nenhum número medido no nosso tráfego; o
§5 diz o que medir antes de ligar cada fase.*

## 1. O Jev em 5 linhas

O Jev é o modelo "System One" da TypeSafe. **Não gera texto.** Recebe um
`state` e perguntas tipadas (`noul` sim/não, `choice` e `score`) e devolve
probabilidades calibradas.

- Preço: $0,042/M de input, output grátis.
- Contexto: 64k, dos quais 32k para o state.
- Latência real: 0,3 a 0,7 s.
- Versão: continua a não haver nada depois da `jev-1.13`. O `jev-preview` é um
  alias da mesma versão. No OpenRouter usa-se
  `POST /api/alpha/decisions` com o modelo `typesafe/jev-1.13-20260917`
  (fixado).
- Limites conhecidos: não faz contas, distrai-se com state irrelevante, é
  injetável a partir do state e não sabe distinguir quem escreveu o texto.
- Precisão em benchmarks independentes: 67 a 68%, contra 73% do Opus 5. É
  "bom o suficiente a 1/440 do preço", não melhor.

**Novidade de 25-09: `typesafe/jev-router` no OpenRouter.** Escolhe modelo e
effort por pedido, já com o custo de perder a cache incluído na decisão.
**Não nos serve:** só escolhe entre modelos do OpenRouter, e os modelos do
claudin.io não estão lá.

## 2. O que o ecossistema mediu em harnesses de coding

| padrão | projeto | resultado medido |
|---|---|---|
| effort por passo, só para baixo | ifoster01/jev-effort | em `high`: custo −1,1% (thinking −46%). Em `max`: custo −55%. Numa sessão real, −4 a 9%, porque a cache é 82% do custo. Cache intacta (99,1%) |
| effort por pedido | robertn702/opencode-jev-router | 44/44 tarefas resolvidas nos dois braços. −14% de tokens de output, −9% de tempo |
| modelo do subagente | 0x7067/claude-jev | em 300 spawns: concorda 160 vezes, escolhe mais barato 51 e mais caro 89. Misto |
| compactação por seleção verbatim | claude-jev, jevmate | ~1 s contra ~117 s do summarizer. Restrições plantadas sobrevivem 100% |
| corte de output de tool grande | jevmate | perdeu 2 de 828 linhas com sinal. O original fica guardado em disco |
| guard de conclusão (Stop) | jev-belay | 7 de 8 paragens corretas, mas foi **removido** por colidir com outro verificador |
| detecção de loop | paseo_drinking_bird | sem número |
| skill router | shimo4228 | **negativo**: 28 de 539 sugestões usadas. Removido |
| dicas de routing por prompt | claude-jev | +5,2 pontos sobre o baseline. Desligado |

Três lições valem para nós:

1. **Mudar de modelo ou o catálogo de tools a meio da sessão parte a cache.**
   A cache é a maior fatia da conta, e o proxy mede ~96% do input do
   executor como cache read.
2. **Juntar o Jev a um loop que já tem verificadores dá conflito.** Foi o
   caso do shimo. O Jev compensa quando **substitui** uma decisão que já
   existe, e menos quando acrescenta uma.
3. **Regras determinísticas primeiro, Jev só para o que é juízo de
   significado.** É o mesmo princípio do `loop_watch` do proxy: "a aritmética
   decide QUANDO olhar, o Jev decide SE".

## 3. O que o nosso harness tem hoje (HEAD `5f4c109`)

Os caminhos são relativos a `src-tauri/src/`.

- **Juiz de conclusão com o modelo mais caro.**
  - Em cada turno terminal sem tool calls, `session.rs:2341` chama
    `provider::classify_turn_completion` (`provider.rs:1017`) com o
    **`brain_model` (claudius)**.
  - É um pedido frio (system prompt próprio, sem cache) que pode levar até
    1024 tokens de output e devolve uma única palavra: CONTINUE ou DONE.
  - **O usage é descartado**: o custo não entra no `CostLedger`, mas o cliente
    paga-o à mesma.
  - **Já é um classificador binário. É o encaixe mais direto do Jev que
    existe.**
- **Não há nenhuma detecção de loop.**
  - `max_rounds` é infinito por defeito, na sessão principal e nos
    subagentes.
  - Não há deteção de tool calls repetidas.
  - O proxy tem o `loop_watch` em sombra desde 2026-09-21, mas medido hoje
    tem **0 decisões em 336 h** com 74 sessões rastreadas: a porta aritmética
    nunca disparou.
  - O proxy só vê deltas de tokens. O harness vê os argumentos e os
    resultados das tools, de forma exata.
- **Thinking global e fixo.**
  - Todos os pedidos, incluindo os de subagentes Explore, levam
    `thinking.budget_tokens` de 4k a 30k e `max_tokens` de 32k.
  - Não há effort por passo.
- **Subagentes.** Contexto novo (não herdam o prefixo do pai), sempre
  `builder_model` com o thinking da sessão, sem compactação.
- **Corte de tool results.** São cortados a 24 000 chars (`tool_result_block`,
  `session.rs:4345`). É um corte cego: não escolhe o que guardar.
- **Compactação.** É um subagente summarizer no `builder_model`, que lê o
  JSONL. É raro (só acima de 150k tokens).
- **Permissões.** Manual, ou YOLO. Não há meio-termo.

## 4. Plano por fases

O Jev entra como **infra única com três modos**: `off`, `shadow` e `on`, com
fail-open em tudo (sem chave, timeout, 429/529 ou 4xx → a decisão antiga
mantém-se). Cada fase segue TDD: primeiro os testes vermelhos com o stub HTTP
`spawn_stub` (`session.rs:5345`).

### F0. Infra: cliente Jev e para onde vai o pedido

- **Cliente.** `agent/jev.rs`: `decide(state, questions) -> Option<Decisions>`,
  com timeout de 2 s, `NetSource::Jev`, um `SessionRecord::JevDecision` no
  JSONL e o custo somado num campo novo do `CostLedger`.
- **Rota recomendada.** Um endpoint `POST api.claudin.io/v1/decisions` no
  proxy, autenticado com a chave do cliente. Reutiliza o
  `claudin_router/jev.py` e encaminha com a nossa chave do OpenRouter. Três
  razões:
  1. o utilizador não precisa de conta no OpenRouter;
  2. **é o único sítio onde a sombra é medível**, porque o JSONL vive na
     máquina do cliente e nós não o vemos;
  3. podemos fazer rate-limit e trocar de backend (Jev ou réplica local)
     sem lançar uma versão nova do app.
- **Quem usa OpenRouter.** Se o utilizador tiver OpenRouter configurado,
  chamamos o OpenRouter diretamente com a chave dele.
- **Decisão em aberto: privacidade.** O state leva código do cliente para a
  TypeSafe. O proxy já o faz hoje com `CLAUDIN_BRANCHES=on`, mas o DPA nunca
  foi lido. A flag tem de ser exposta nas Definições.

### F1. Juiz de conclusão → Jev. Substitui, não acrescenta

- **Perguntas** (`noul`, fan-out, numa só chamada):
  - `announced_unfinished_step`: anunciou um passo e não o deu;
  - `asks_user_in_prose`: pergunta algo ao utilizador em prosa, sem usar
    `ask_user`.
- **State:** o texto final do turno, mais o nome da última tool chamada.
- **Shadow:** corre o claudius e o Jev em paralelo e grava os dois
  veredictos. O veredicto que conta continua a ser o do claudius.
- **On:** passa a contar o Jev. Na zona cinzenta (0,35 < p < 0,65) cai para o
  claudius, e o backstop por pontuação (`should_nudge_terminal`) mantém-se.
- **O que se ganha:** um pedido frio ao claudius por turno terminal passa a
  ~$0,00003. A latência desce de segundos para ~0,4 s, e esta latência é
  exatamente o tempo que o utilizador espera para ver a run acabar.
- **Fixtures:** os casos reais citados no código (sessão 912bb460, "vou
  spawnar subagentes" sem os spawnar) e um conjunto rotulado de 40 a 60 finais.
  O limiar fica fixado por teste, à maneira do jevcal, e não por um número
  escolhido à mão.

### F2. Quebra-ciclos no harness

O caso que justifica esta fase: o cliente 7cd4fe45 fez 103 voltas da mesma tool
call, pagou US$1,03 por zero trabalho, e é o 6.º ticket dele.

- **Porta determinística.** Hash de (tool, args) repetido ≥3 vezes numa janela
  de 8, **ou** hash do resultado igual N vezes seguidas. É barata e exata, e o
  proxy não a consegue ter.
- **Pergunta ao Jev.** Só quando a porta dispara: `noul` "está a repetir sem
  progredir?" sobre os últimos K passos (tool, args e resumo do resultado).
- **Ação em `on`.**
  1. Primeiro, um nudge único: "estás a repetir X; muda de abordagem ou
     pergunta ao utilizador".
  2. Se repetir outra vez, a run **pausa** e chama `ask_user`. **Nunca mata
     a run em silêncio**: um falso positivo mata trabalho real a meio
     (`loop_watch`, §12.5 do estudo do proxy).
- **Bónus sem Jev:** um `max_rounds` por defeito alto (200, por exemplo) como
  rede de segurança, com aviso em vez de corte.

### F3. Corte inteligente de tool results grandes

- **Hoje:** acima de 24k chars, corte cego.
- **Proposta:** acima de um limiar, o Jev escolhe que linhas manter
  (`choice`/`score` por bloco, segundo o padrão do jevmate). O output completo
  vai para um ficheiro, e o modelo recebe o caminho para o poder ler com
  `read_file`.
- **Cache:** é seguro, porque mexe só na **cauda** (o resultado que acabou de
  chegar). O prefixo não se move.
- **O que se ganha:** sobretudo qualidade, ao trocar um corte cego por um corte
  com sinal. Em tokens é modesto: um resultado de 24k chars são ~8k tokens
  frios, uma vez.
- **Medir em fixtures:** linhas com sinal perdidas (erros, falhas de teste,
  paths). O alvo é ≤0,5%, a mesma ordem de grandeza do jevmate.

### F4. Effort dos subagentes. Primeiro sem Jev

- **Regra determinística:** os subagentes Explore correm com effort `low`.
  Ler e procurar não precisa de 16k de thinking.
- **Antes de pôr o Jev a escolher**, há três números para medir:
  1. **Que fatia da conta do claudinio-code é thinking?** Lê-se no Langfuse,
     filtrado pelo user-agent do claudinio-code.
  2. **Um budget menor encolhe mesmo o raciocínio nos nossos upstreams?** O
     brain tem um negativo **medido**: no DeepSeek, `high` não ficou mais curto
     que o default (mediana de 20,9k contra 18,6k, `deepseek-v4-flash
     raciocinio_sem_dial_medido`). Se o dial não morde, toda esta fase vale
     zero.
  3. **Mudar o budget a meio da sessão parte a cache do upstream?** Mede-se
     com dois pedidos controlados.

### F5 (condicional a F4). Effort por passo no loop principal

É o padrão jev-effort: o Jev **só pode baixar** o effort, nunca subir acima do
da sessão.

- **Só avança se o F4 mostrar** que o thinking pesa na conta **e** que o dial
  morde.
- O ganho medido lá fora é de ~1% em `high` e até 55% em `max`. O nosso
  default não é `max`, por isso a expectativa é baixa.

### F6 (produto, não custo). Auto-mode entre o manual e o YOLO

Um `noul` de risco por tool call aprovável: "destrutiva?", "fora do âmbito?".
Decide a partir de que valor se aprova sozinha. Nunca relaxa a deny-list.

- **Calibração honesta do pi-jev:** um edit normal pontua 0,85 em
  "destrutivo", por isso o limiar de aprovação automática fica em 0,9 ou
  acima.
- Primeiro só em sombra.

## 5. Portões: nada passa a `on` sem estes números

| fase | número para ligar |
|---|---|
| F1 | concordância Jev vs claudius ≥90% em sombra (n ≥ 200 turnos terminais), 0 regressões nas fixtures, e a distribuição de p **bimodal**. Um borrão no meio pede um state melhor, não um limiar |
| F2 | quantas vezes a porta dispara (alvo <1% das rondas), as 15 sessões do topo lidas à mão, e 0 falsos positivos nas sessões do Victor |
| F3 | ≤0,5% de linhas com sinal perdidas nas fixtures |
| F4/F5 | os três números do F4, e nunca agregar numa janela que atravesse uma mudança de config |

## 6. O que NÃO fazer

Cada um destes já foi medido como negativo ou parte a cache:

- **Trocar de modelo por turno, ou usar o `jev-router`.** Parte a cache, e os
  nossos modelos não estão no OpenRouter.
- **Filtrar o catálogo de tools por turno.** Parte a cache. Se algum dia se
  fizer, é só no arranque da run.
- **Podar ou reescrever o histórico a meio da sessão.** É o anti-caso do §7.1
  do estudo do proxy.
- **Skill router e dicas de routing por prompt.** Negativo medido por
  terceiros.
- **Pôr o Jev como mais um guard ao lado do juiz atual.** A forma é
  substituir (F1), não empilhar.

## 7. Ordem recomendada

1. **F0 + F1:** substitui um custo e uma latência que já existem, e a sombra é
   barata.
2. **F2:** o único com um cliente real a perder dinheiro.
3. **F3.**
4. **Medição do F4**, que diz se F4 e F5 valem a pena de todo.
5. **F6:** quando houver procura.

## 8. Implementado (2026-10-05, branch `feat/jev-harness`, TDD)

O que foi pedido: "tudo o que der e trouxer benefício". O utilizador usa a
própria chave TypeSafe ou o OpenRouter, e a conta claudin.io também dá acesso,
incluído no plano.

**Backends, por ordem** (`agent/jev.rs`):

1. a chave TypeSafe do utilizador;
2. o plano claudin.io, em `/api/app/decisions` (claudinio-litellm
   `dashboard/app_decisions.py`), assinado como o `web_search` e sem chave
   externa;
3. o OpenRouter ligado.

Um 401/403 do plano fica memorizado durante 1 h e o pedido passa ao backend
seguinte. Tudo é fail-open.

**As três fases que mostraram benefício, com os limiares tirados de sondas ao
vivo a 2026-10-05:**

| fase | o que faz | sonda |
|---|---|---|
| F1 | o juiz de conclusão pergunta primeiro ao Jev (`unfinished`, `asks_user`). Com ≥0,7 continua, com ambos ≤0,3 termina, e o resto vai para o juiz LLM | 16/16 corretos; finais concluídos dão 0,03–0,05 e os outros 0,61–0,99. ~0,2 s, $2,1e-5 |
| F2 | `loop_watch` na sessão e nos subagentes. A porta exata dispara com (tool, args, resultado) ×3 ou (tool, args) ×3; o Jev julga com `stuck` ≥0,85; dois avisos e depois `tool_loop` | 4 ciclos dão 0,89–0,96; trabalho saudável 0,03–0,05; polling de CI 0,72 |
| F3 | `output_trim` no bash: acima de 20k chars ficam a cabeça e o fim, o Jev ordena os blocos do meio, e o output completo vai para um ficheiro temporário | sinal 0,87–0,99, ruído 0,04–0,12 |

**Um bug à parte, encontrado pelo teste do F3:** o bash lia o stdout só depois
de o processo terminar. Com um output acima de ~64 KB, o pipe enchia, o
processo filho bloqueava e o comando acabava em timeout de 30 s. Foi corrigido
e tem teste de regressão.

**O que ficou de fora, e porquê:** o F4 e o F5 (effort por passo) estão parados
até haver medição (o brain tem um negativo medido no DeepSeek). O F6
(auto-aprovação) é produto, não custo, e precisa de sombra antes.

**Para medir:** o custo vai para o `CostLedger` (`jev_cost`). O
`ContinuationJudge` grava `judge: jev|llm`. No servidor, o hash
`claudinio:jev:app:stats:<dia>` tem as chamadas e o custo diário do plano.

## 9. Compactação literal (port do `fast-jev-compaction`, 2026-10-05)

Pedido do Victor: trazer para o harness, de raiz, o que o
[tamaratran/fast-jev-compaction](https://github.com/tamaratran/fast-jev-compaction)
faz.

**O que o repo faz.** Em vez de pedir um resumo a um LLM, o Jev lê a conversa
inteira (resultados de tools substituídos por uma nota, encaixada por estágios)
e responde, por cada tool call antiga, duas perguntas: a chamada fica? o output
fica literal? O que não fica sai: o output é cortado a uma cabeça de 300 chars,
ou a chamada sai com o resultado. O texto do utilizador e do assistente nunca
muda.

**O que trouxemos** (`agent/prune.rs`, registo `SessionRecord::Pruned`):

- **Antes do handoff e da compactação**, nos dois pontos do loop, corre a
  compactação literal. Só é aceite se libertar ≥25% dos chars e deixar o
  contexto abaixo de 80% do limite. Caso contrário cai no handoff ou no resumo
  de sempre, exatamente como antes.
- **Persistida:** o registo `Pruned` é reaplicado em `history_from_records`, por
  isso uma sessão retomada é a mesma sessão. Os `Turn` nunca são apagados, e a
  UI continua a mostrar tudo.
- **Subagentes:** não tinham compactação nenhuma. Agora, acima da linha de
  handoff, a mesma poda corre em memória.
- **Limites ajustados aos nossos backends:** estado ≤56k chars e 8 perguntas por
  pedido (o teto do endpoint do plano), no máximo 160 calls julgadas.
- **Diagnóstico:** o registo grava pedidos, estágio, redução, custo e as duas
  probabilidades de cada call.

**Limiares próprios, não os do upstream (0,5), medidos em duas sondas ao vivo:**

| call | P(call) | P(result) | decisão |
|---|---|---|---|
| schema de que a tarefa atual depende | 0,50 | 0,41 | fica inteiro |
| ficheiro do bug, já corrigido | 0,31 | 0,23 | call fica, output cortado |
| teste que falhou, já tratado | 0,31 | 0,16 | call fica, output cortado |
| log de build | 0,23 | 0,14 | call fica, output cortado |
| README / UI não relacionada | 0,11–0,13 | 0,07–0,11 | sai |

Com 0,5, o schema ainda necessário teria sido cortado. Ficou
`KEEP_RESULT_THRESHOLD = 0,35` e `KEEP_CALL_THRESHOLD = 0,2`. Apagar um output
necessário custa uma releitura; manter um obsoleto só custa contexto.

**Porque vale aqui, quando no proxy era um anti-caso (§7.1 do estudo do
proxy):** no proxy, podar contexto quente parte a cache sem necessidade. Aqui a
poda só corre quando o harness **já ia** reescrever a história (handoff ou
resumo), por isso o custo de cache é o mesmo. Muda o que sobrevive: as palavras
exatas em vez de prosa, e ~1 s em vez de um subagente summarizer que relê o
JSONL inteiro.

**O que não trouxemos:** o gatilho por percentagem (`compactAtPercent=60`),
porque o nosso já é o limiar de handoff; e o estimador de tokens deles, porque
os nossos limites são em chars.
