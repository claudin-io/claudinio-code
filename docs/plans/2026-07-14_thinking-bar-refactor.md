# Thinking Bar — Replace Inline Thinking with Fixed Bar

## Context

Atualmente, quando o modelo está a "pensar", o texto do pensamento aparece inline dentro da área de scroll do chat (via `ThinkingRow` no `TimelineSteps`). Como o texto é revelado palavra-a-palavra a alta velocidade, o conteúdo "pula" constantemente e o scroll fica instável, mesmo com a lógica `isAtBottom`. O user quer substituir esta experiência por uma barra fixa entre o chat e o input que indica "Thinking..." sem mover o conteúdo já visível.

Decisões do user confirmadas:
- **Layout**: scrolling de mensagens | thinking bar fixa (sempre visível) | input area
- **Durante thinking**: Linha fina com SVG animado + "Thinking..." + hover tooltip (scrollável) com o texto completo do pensamento
- **Após terminar**: Barra some; o pensamento fica como step "Thought" colapsável no Trajectory (comportamento atual inalterado)
- **Tooltip**: Completo, scrollável
- **SVG animado**: O fornecido pelo user (3 bolinhas com blur + glow que rodam e oscilam), com cor alternante (accent)

## Solution Design

### 1. Novo componente: `ThinkingBar`

Um componente colocado **fora** do `scrollContainerRef`, entre o fim da scroll area e o input area. Renderizado apenas quando `status() === "thinking"` e a última step é de thinking.

**Estrutura visual:**
- Container: `h-10` (thin bar), fundo `surface-1`, bordo `border-t border-border-subtle`, padding horizontal `px-6`
- Esquerda: SVG spinner (o SVG fornecido pelo user, com animação de cor alternando entre `var(--accent)` e `var(--accent-strong)` via CSS `animation` com `filter: hue-rotate` ou cor animada)
- Meio: Texto "Thinking..." (i18n: `chat.status.thinking`)
- Tooltip no hover: o full container da barra tem `group`, e no `group-hover` mostra um tooltip com o texto completo via `smoothThinking.displayed`, com `max-h-[50vh] overflow-y-auto` e fundo `surface-2`, borda, sombra

**Cor alternante no SVG:**
O SVG fornecido usa `currentColor`. Podemos animar a cor com:
```css
@keyframes thinking-hue {
  0%, 100% { color: var(--accent); }
  50% { color: var(--accent-strong); }
}
```
Ou alternar via Tailwind `group` + CSS custom property. O SVG será envolvido num `<span>` com a classe `animate-thinking-color`.

### 2. Modificações no `ChatPanel.tsx`

#### a) JSX do live area (linhas ~1954-1972)
Remover o `ThinkingRow` da `TimelineSteps` quando é live e o step é de thinking. Abordagem:
- No `TimelineSteps`, quando `isLive === true`, filtrar os steps de thinking que são o último step (ou todos os thinking steps, já que só interessam na barra)
- Alternativa: passar `steps` filtrados para `TimelineSteps` no live mode

**Plano concreto:** Modificar o JSX do live area para filtrar steps `thinking` quando `isLive === true`. O `TimelineSteps` continua a renderizar todos os outros steps (tool calls, subagents, phases, etc.). A lógica de tracking dos steps (`currentSteps()`) permanece **inalterada** — os thinking steps ainda são adicionados para uso futuro no Trajectory quando a mensagem é promovida.

#### b) Adicionar `<ThinkingBar>` no layout
Entre o fim do scroll container e o input area:
```tsx
<Show when={liveThinkingActive()}>
  <ThinkingBar text={smoothThinking.displayed} />
</Show>
```
Usa o sinal `liveThinkingActive()` já existente.

### 3. CSS para tooltip e animação

Adicionar ao `App.css`:
- `.thinking-bar-tooltip`: Tooltip posicionado acima da barra, com fundo `surface-2`, bordo, `max-h-[50vh] overflow-y-auto`, transição de opacidade
- `.animate-thinking-color`: Animação de cor entre accent e accent-strong
- `.thinking-bar-spinner`: Wrapper do SVG com tamanho e alinhamento

### 4. i18n

Usar a chave `chat.status.thinking` já existente nos locales. O tooltip não precisa de chave nova — é o conteúdo do pensamento.

### 5. Comportamento de transição

- **Thinking inicia** (`Thinking` event → `liveThinkingActive()` fica true): Barra aparece
- **Tool call / texto aparece** (thinking step deixa de ser o último): `liveThinkingActive()` fica false → barra desaparece. Os thinking steps continuam em `currentSteps()`.
- **Done**: Thinking steps ganham `endedAt` na promoção. Quando a mensagem é promovida para `messages[]`, o Trajectory mostra o step "Thought" colapsável (comportamento existente).
- **Scroll**: A barra está fora do scroll container, nunca é afetada pelo scroll do chat.

### 6. O que NÃO muda

- O tracking de `currentSteps()` para thinking steps (continua igual)
- O `ThinkingRow` para mensagens históricas (já promovidas para `messages[]`)
- O `Trajectory` com steps colapsáveis
- O `createSmoothText` para `smoothThinking` (continua a alimentar a tooltip)
- A lógica de scroll (`isAtBottom`, `scrollToBottom`, etc.)

## Risks

| Risco | Mitigação |
|-------|-----------|
| Tooltip sobrepõe input area | Posicionar tooltip **acima** da barra (com `bottom-full`) usando um wrapper `relative` |
| Tooltip corta no topo do viewport | Usar `max-h-[50vh]` e `overflow-y-auto`, e possivelmente `top-auto bottom-full` com fallback |
| SVG animado causa performance | SVG é leve (3 círculos, filter simples). Sem `transform` pesado. |
| Quebra de layout no sepia/light | Usar variáveis CSS (`--surface-*`, `--border-*`) existentes |
| Thinking bar aparece durante `awaiting_input` | `liveThinkingActive()` já só retorna true quando `status() === "thinking"` |

## Tasks summary

1. **Adicionar CSS** para a thinking bar, tooltip, animação de cor
2. **Criar componente ThinkingBar** (ou inline) com SVG + texto + tooltip hover
3. **Modificar JSX do live area** para filtrar thinking steps do TimelineSteps
4. **Adicionar ThinkingBar ao layout** entre scroll e input
5. **Verificar transições** (thinking → tool call → done)


## Implementation Log — 2026-07-14 10:15
**Summary:** Replace inline thinking steps with a fixed ThinkingBar between scroll area and input
**Changed files:** M src/App.css, M src/components/ChatPanel.tsx, ?? docs/plans/2026-07-14_thinking-bar-refactor.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Key decisions and findings

1. **Minimal approach**: Instead of creating a separate component file, the ThinkingBar is defined inline in ChatPanel.tsx (consistent with the existing pattern — ThinkingRow, ToolRow, etc. are all local components). The SVG is stored as a constant `thinkingSvgSpinner` to avoid recreating JSX.

2. **Filter strategy**: The thinking steps filter is applied inline in the JSX: `status() === "thinking" ? currentSteps().filter((s) => s.type !== "thinking") : currentSteps()`. This is the minimal change that achieves the goal — no new component state, no new signals, no complex orchestration. The `currentSteps()` tracking is untouched, so thought steps still get promoted to Trajectory correctly when the message ends.

3. **CSS-only tooltip**: The tooltip uses pure CSS (`.thinking-bar-wrapper:hover .thinking-bar-tooltip`) with opacity transition. No JS needed, no `onMouseEnter`/`onMouseLeave`. The tooltip is positioned above the bar with `bottom: calc(100% + 6px)` and has `max-height: 50vh; overflow-y: auto` for the scrollable preview.

4. **SVG color pulse**: The user's SVG uses `currentColor`, so the animation is just a `@keyframes thinking-color-pulse` cycling `var(--accent)` → `var(--accent-strong)` applied to the `.thinking-bar-spinner` wrapper. Respects `prefers-reduced-motion: reduce` by disabling the animation and keeping the accent color static.

5. **Scroll unaffected**: The ThinkingBar is placed between the `</div>` closing `scrollContainerRef` and the `<Show when={openSubagent()}>`. It's inside the outer `relative flex flex-col overflow-hidden` container but completely outside the scroll area — zero impact on scroll behaviour.

6. **Build clean**: 1455 modules transformed, 0 errors. All 628 existing tests pass (the 11 uncaught `ResizeObserver` errors are pre-existing in the test environment, unrelated to this change).

**Task journal:**
- Add CSS for ThinkingBar, tooltip, and color animation: Added `.thinking-bar`, `.thinking-bar-spinner`, `.thinking-bar-label`, `.thinking-bar-tooltip`, `.thinking-bar-wrapper` classes; Added `@keyframes thinking-color-pulse` cycling accent ↔ accent-strong; Added `prefers-reduced-motion: reduce` fallback
- Create ThinkingBar component with SVG + text + hover tooltip: Created `ThinkingBar` component before `ThinkingRow`; Uses user's SVG spinner with `.thinking-bar-spinner` color-pulse class; Tooltip shows full thinking text via `smoothThinking.displayed`; `thinkingSvgSpinner` constant stores the SVG with glow filter animation
- Filter thinking steps from TimelineSteps during live mode: Filter added inline: `status() === 'thinking' ? currentSteps().filter((s) => s.type !== 'thinking') : currentSteps()`; Thinking steps are only filtered from display during live mode — tracking in `currentSteps()` stays unchanged
- Insert ThinkingBar between scroll container and input area: Added `<Show when={liveThinkingActive()}><ThinkingBar text={smoothThinking.displayed} /></Show>` right after scroll container div closes, before SubagentModal
- Verify all transitions and visual correctness: Build: `vite build` completed successfully (1455 modules, 0 errors); Tests: 628 passed across 34 suites (ResizeObserver errors are pre-existing test env issue); Transitions verified by code analysis: (1) bar shows via `liveThinkingActive()`, (2) tooltip via CSS hover with `.thinking-bar-tooltip`, (3) bar hidden when last step !== thinking or status !== thinking, (4) thought steps remain in `currentSteps()` for Trajectory promotion, (5) filter only applies during `status() === 'thinking'` — non-thinking steps display inline, (6) scroll container unchanged — bar is outside it
