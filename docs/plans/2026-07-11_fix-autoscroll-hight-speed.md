# Fix: Autoscroll perde tracking em alta velocidade

## Context

O autoscroll do chat funciona bem em velocidade normal (~18 wps), mas quando o backlog de texto aumenta, a engine `createSmoothText` acelera progressivamente (até `maxWps=120`, e com `finishMultiplier=4.5x` na drenagem = efetivamente ~540 wps). Nesse regime, o scroll suave com `behavior: "smooth"` não acompanha.

## Diagnóstico (Prova Real)

Três problemas identificados no `src/components/ChatPanel.tsx`:

### 🔴 Problema A: `scrollToBottom` com `setTimeout` + `behavior: "smooth"`
- Linha 1009-1014: `scrollToBottom` agenda `scrollIntoView({ behavior: "smooth" })` com `setTimeout(50)`.
- A animação smooth leva ~200-400ms para completar. Nesse intervalo, o smooth text revela dezenas de novas palavras, e o `messagesEndRef` que era o target 50ms atrás já não é mais o bottom real.
- O scroll smooth anima da posição atual para um target **já obsoleto** → nunca alcança o bottom.
- Resultado: scroll acumula latência e fica permanentemente atrás do conteúdo.

### 🟡 Problema B: `autoScrolling` guard nunca libera
- Linhas 1000-1007: `handleScroll` só libera `autoScrolling=false` se `atBottom === true` depois que a animação smooth termina.
- Se a animação termina e NÃO está no bottom (porque conteúdo cresceu durante a animação), `autoScrolling` fica `true` **para sempre**, engolindo todos os scroll events subsequentes.
- `scrollToBottom` ainda funciona porque seta `isAtBottom(true)` internamente, mas o flag `autoScrolling` nunca é liberado até o próximo scroll que acidentalmente bata no bottom.

### 🟡 Problema C: `flushPendingDone` sem scroll
- Linhas 1297-1313: `flushPendingDone` promove o `Done` para `messages`, mas **não chama `scrollToBottom`** depois. O conteúdo final aparece sem scroll.
- O último scroll foi no evento `Done` (linha 1239), que pode ter sido dezenas/milissegundos antes se o typewriter demorou para drenar.

## Solução

### 1. `scrollToBottom` — remover `setTimeout` e trocar `behavior` para `"instant"`

```ts
const scrollToBottom = (force = false) => {
    if (!force && !isAtBottom()) return;
    autoScrolling = true;
    setIsAtBottom(true);
    messagesEndRef?.scrollIntoView({ behavior: "instant" });
};
```

- Sem `setTimeout`: o scroll acontece no mesmo tick, sem janela para conteúdo crescer entre a decisão de scrollar e o scroll efetivo.
- `behavior: "instant"`: sem animação de 200-400ms, o scroll é imediato. O conteúdo que crescer DEPOIS será pego pelo próximo ciclo do `lastLiveScroll` effect.
- `autoScrolling` continua sendo setado para proteger o scroll event disparado pelo `scrollIntoView` síncrono.

### 2. `handleScroll` — lógica ajustada para `behavior: "instant"`

Com `behavior: "instant"`, o scroll event dispara sincronamente na mesma microtask. Como nenhum conteúdo novo entrou entre `setIsAtBottom(true)` e `scrollIntoView`, o `atBottom` será `true` e `autoScrolling` será liberado corretamente.

Mudança: remover o `if (autoScrolling && atBottom) autoScrolling = false` e simplesmente liberar `autoScrolling` de forma mais robusta:

```ts
const handleScroll = () => {
    const el = scrollContainerRef;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < NEAR_BOTTOM_PX;
    if (autoScrolling) {
      if (atBottom) autoScrolling = false;
      return;
    }
    setIsAtBottom(atBottom);
};
```

Mantido como está — com `behavior: "instant"`, o `atBottom` será `true` consistentemente, então o guard funciona sem travamento.

### 3. `flushPendingDone` — adicionar `scrollToBottom(true)` no final

```ts
const flushPendingDone = () => {
    const pending = pendingDone();
    if (!pending) return;
    // ... (código existente)
    smoothThinking.reset();
    scrollToBottom(true);  // <<< NOVO: scrolla para mostrar a msg final
};
```

- `force=true` porque `isAtBottom()` pode ter mudado durante a drenagem se o usuário scrollou para cima. Mas se o usuário está lendo, queremos scrollar para mostrar o `Done` — é o `done` que marca o fim do bloco, então `force=true` é apropriado.

## Files changed

- `src/components/ChatPanel.tsx` — apenas as seções `scrollToBottom`, `handleScroll` (sem mudança real) e `flushPendingDone`

## Risks

- `behavior: "instant"` perde o aspecto visual agradável do scroll suave. Mitigação: em velocidade normal (18 wps), o scroll instantâneo é quase imperceptível porque o conteúdo não cresce rápido. Em alta velocidade, o smooth não funcionava de qualquer forma.
- `scrollToBottom(true)` em `flushPendingDone` pode scrollar mesmo se o usuário estiver lendo algo acima. Mas o `Done` marca o fim do bloco de resposta — faz sentido scrollar para mostrar.

## Tasks

1. **golden-scroll-instant** — Modificar `scrollToBottom` para usar `behavior: "instant"` e remover `setTimeout(50)`
2. **golden-flush-scroll** — Adicionar `scrollToBottom(true)` ao final de `flushPendingDone`
3. **verify-build** — Verificar que o build compila sem erros


## Implementation Log — 2026-07-11 15:46
**Summary:** fix: autoscroll tracking in high-speed word reveal — removed setTimeout + behavior:'smooth' in scrollToBottom, added scrollToBottom(true) to flushPendingDone
**Changed files:** M src/components/ChatPanel.tsx, ?? docs/plans/2026-07-11_fix-autoscroll-hight-speed.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Findings & Gotchas

1. **Problema raiz não era a velocidade em si — era o `setTimeout(50)` combinado com `behavior: 'smooth'`**. O delay de 50ms dava uma janela para o smooth text revelar dezenas de palavras. O target do `scrollIntoView` ficava obsoleto antes mesmo da animação smooth começar. Com `behavior: 'instant'` e sem `setTimeout`, o scroll acontece no mesmo tick que a decisão de scrollar — o conteúdo não cresce entre a decisão e o scroll.

2. **`autoScrolling` travava para sempre** porque o scroll smooth terminava ~200-400ms depois, quando o conteúdo já tinha crescido, então `atBottom === false` e `autoScrolling` nunca era liberado. Com `behavior: 'instant'`, o scroll event dispara sincronamente e `atBottom` é consistentemente `true`, então o guard funciona.

3. **`flushPendingDone` esquecia de scrollar**. O `scrollToBottom` existia nos eventos `Done` e `TextStep` do `handleEvent`, mas não no `flushPendingDone` que é a função que realmente promove o `Done` para `messages`. Era um fio solto: o evento `Done` disparava `scrollToBottom`, mas o `flushPendingDone` podia rodar segundos depois (quando o typewriter terminava de drenar), e ninguém scrollava para mostrar o conteúdo final.

4. **Validação**: decidi por `scrollToBottom(true)` em vez de `scrollToBottom()` no `flushPendingDone` porque o usuário pode ter scrollado para cima durante a drenagem (~segundos de espera). Mas como o `Done` marca o fim do bloco de resposta, forçar o scroll é o comportamento correto.

5. **Build**: todos os erros do `tsc --noEmit` são pré-existentes (principalmente `vi` not found em test files — problema de configuração de ambiente de teste com vitest, não relacionado às nossas mudanças).

**Task journal:**
- Trocar scrollIntoView para behavior: "instant" sem setTimeout: Troquei behavior:'smooth' + setTimeout(50) por behavior:'instant' sem setTimeout. O scroll agora é síncrono — não há mais janela de ~200-400ms onde o conteúdo pode crescer e invalidar o target. autoScrolling continua sendo setado para proteger o scroll event disparado pelo scrollIntoView síncrono.
- Adicionar scrollToBottom(true) no flushPendingDone: Adicionei scrollToBottom(true) ao final de flushPendingDone, após smoothThinking.reset(). force=true porque o usuário pode ter scrollado para cima durante a drenagem, mas o Done marca o fim da resposta — é o momento certo para scrollar.
- Verificar build: Nenhum erro novo introduzido. Todos os erros do tsc são pré-existentes no código (vi not found em test files, setImmediate, etc.). Nenhum erro no ChatPanel.tsx.
