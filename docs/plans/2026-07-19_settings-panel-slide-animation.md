# Settings Panel Slide Animation

## Context
O painel de configurações (`SettingsPanel`) atualmente usa `<Show when={props.showConfig()}>` para controle de visibilidade — ele aparece e some instantaneamente. O usuário quer que o painel deslize da direita para a esquerda ao aparecer, e da esquerda para a direita ao sumir, com fade sincronizado no overlay.

**Decisões confirmadas com o usuário:**
- Velocidade: rápida e responsiva, ~200ms (Recommended)
- Overlay: fade in/out sincronizado com o slide (Recommended)

## Solution Design

### Comportamento da animação
- **Enter**: overlay faz fade in (opacity 0 → 1), painel desliza da direita (translateX(100%) → translateX(0)), ambos em 200ms ease-out
- **Exit**: overlay faz fade out (opacity 1 → 0), painel desliza para direita (translateX(0) → translateX(100%)), ambos em 200ms ease-in, depois o elemento é removido do DOM
- **Escape key**: dispara a animação de saída (não remove instantaneamente)
- **Click no overlay**: dispara a animação de saída

### Implementação
O `Show` remove o elemento do DOM imediatamente, impedindo animação de saída. A solução é controlar rendering com um sinal local `phase`:
- `'hidden'` — elemento não renderizado
- `'entering'` — renderizado, animação slide-in tocando
- `'visible'` — renderizado, posição final
- `'exiting'` — renderizado, animação slide-out tocando; após 200ms, vira `'hidden'`

Um `createEffect` assiste `props.showConfig()` e transiciona entre fases. Um `setTimeout` limpa após a animação de saída.

### Estados de edge case
- Abrir e fechar rapidamente: o timeout de `exiting → hidden` é limpo se `showConfig` voltar a `true`
- `Escape` e clique no overlay já chamam `setShowConfig(false)`, que dispara a mesma máquina de fases

## Risks
- **Baixo**: mudança localizada em 2 arquivos (`SettingsPanel.tsx`, `App.css`), sem alteração de props ou lógica de negócio

## Non-goals
- Não alterar resize, search, categorias, ou qualquer lógica interna do painel
- Não alterar `App.tsx` ou outros componentes
- Não usar biblioteca de animação externa

## Low-Level Design

### Arquivos a modificar

**1. `src/components/SettingsPanel.tsx`**

Mudanças:
- Adicionar sinal local `phase`: `createSignal<'hidden' | 'entering' | 'visible' | 'exiting'>('hidden')`
- Adicionar referência para o timer de saída: `let exitTimer: ReturnType<typeof setTimeout> | undefined`
- Substituir `<Show when={props.showConfig()}>` por renderização condicional baseada em `phase() !== 'hidden'`
- Adicionar `createEffect` que assiste `props.showConfig()`:
  - `true`: cancela exitTimer se existir; seta `phase('entering')`; requestAnimationFrame seguido de `phase('visible')` após ~16ms (força o browser a aplicar o estado inicial da animação)
  - `false`: seta `phase('exiting')`; `exitTimer = setTimeout(() => { setPhase('hidden'); setSearchQuery(''); setActiveCategory('general'); }, 200)`
- `onCleanup`: limpar exitTimer
- Classes CSS condicionais no overlay e panel baseadas em `phase()`

Trecho do markup alterado:
```tsx
// Em vez de:
<Show when={props.showConfig()}>
  <div class="settings-panel-overlay" ...>

// Usar:
{phase() !== 'hidden' && (
  <div
    class="settings-panel-overlay"
    classList={{
      'settings-overlay-enter': phase() === 'entering',
      'settings-overlay-exit': phase() === 'exiting',
    }}
    ...
  >
    <div
      class="settings-panel"
      classList={{
        'settings-panel-enter': phase() === 'entering',
        'settings-panel-exit': phase() === 'exiting',
      }}
      ...
    >
```

**2. `src/App.css`**

Adicionar ao final do arquivo:

```css
/* Settings panel slide animation */

/* Overlay fade */
.settings-panel-overlay {
  opacity: 1;
  transition: opacity 200ms ease-out;
}
.settings-overlay-enter {
  opacity: 0;
}
/* Force opacity to 1 on next frame for enter animation */
.settings-panel-overlay:not(.settings-overlay-enter):not(.settings-overlay-exit) {
  opacity: 1;
}

.settings-overlay-exit {
  opacity: 0;
  transition: opacity 200ms ease-in;
}

/* Panel slide */
.settings-panel {
  transform: translateX(0);
  transition: transform 200ms ease-out;
}
.settings-panel-enter {
  transform: translateX(100%);
}
.settings-panel-exit {
  transform: translateX(100%);
  transition: transform 200ms ease-in;
}
```

Na verdade, a abordagem com classes e transições CSS é mais limpa. O truque: o elemento renderiza com a classe `settings-panel-enter` (que seta `translateX(100%)`), e no próximo frame remove-se essa classe — o browser anima a transição de volta para `translateX(0)`.

Para o exit, adiciona-se `settings-panel-exit` (que seta `translateX(100%)`), e a transição cuida da animação.

Para o overlay, mesma lógica: renderiza com `opacity: 0`, remove no próximo frame → fade in. No exit, adiciona `opacity: 0` com `transition` → fade out.

### Fluxo de `createEffect`

```
showConfig() muda para true:
  1. clearTimeout(exitTimer)
  2. setPhase('entering')  → elemento renderiza com classes de enter
  3. requestAnimationFrame(() => {
       setPhase('visible')  → classes de enter removidas, browser anima para estado final
     })

showConfig() muda para false:
  1. setPhase('exiting')   → classes de exit adicionadas
  2. exitTimer = setTimeout(() => {
       setPhase('hidden')   → elemento removido do DOM
       setSearchQuery('')
       setActiveCategory('general')
     }, 200)
```

### Respeita `prefers-reduced-motion`
Seguindo o padrão existente no projeto (`@media (prefers-reduced-motion: no-preference)` para animações), as transições devem ser desabilitadas quando o usuário prefere movimento reduzido:

```css
@media (prefers-reduced-motion: reduce) {
  .settings-panel-overlay,
  .settings-panel {
    transition: none;
  }
}
```

## Tasks summary
1. Adicionar animação slide-in/slide-out no `SettingsPanel.tsx` (phase state machine, remover `Show`, classes condicionais)
2. Adicionar keyframes/transitions CSS no `App.css` (overlay fade + panel slide + reduced-motion)
3. Verificar build + testes


## Implementation Log — 2026-07-19 09:36
**Summary:** SettingsPanel now slides in from right with fade overlay (200ms), exits sliding right with fade out
**Changed files:** A	docs/plans/2026-07-19_settings-panel-slide-animation.md, A	docs/plans/2026-07-19_settings-redesign-vscode-panel.md
**Commits:** 755b3ea docs(plan): settings-panel-slide-animation, 67fa003 docs(plan): settings-redesign-vscode-panel, 4bff3ff docs(plan): settings-redesign-vscode-panel
**Journal:** Implementation of slide-in/slide-out animation for SettingsPanel.

**What was done:**
- Replaced `<Show when={showConfig()}>` with a 4-phase state machine (`hidden` → `entering` → `visible` → `exiting` → `hidden`) using a local `phase` signal
- Enter animation: element renders with `.settings-overlay-enter` (opacity:0) and `.settings-panel-enter` (translateX:100%), then `requestAnimationFrame` removes those classes → CSS transitions animate to final state over 200ms ease-out
- Exit animation: `.settings-overlay-exit` and `.settings-panel-exit` classes added → overlay fades out, panel slides right over 200ms ease-in, then `setTimeout(200)` removes from DOM
- Overlay click and Escape key now trigger the exit animation instead of instant removal
- Cleaned up 5 unused imports (FLAGS, LOCALE_LABELS, McpServerMap, UpdateInfo, SUPPORTED_LOCALES)
- Wrapped all transitions in `@media (prefers-reduced-motion: no-preference)` for accessibility
- Fixed missing overlay closing `</div>` after subagent missed it on first pass

**Gotcha:** The subagent missed the overlay closing `</div>` tag when replacing the `<Show>` wrapper, causing JSX parse errors. Fixed in a follow-up edit.

**Task journal:**
- Add phase state machine to SettingsPanel.tsx: Added phase signal + exitTimer var after panelWidth (line ~109-110); Changed escape key listener from !props.showConfig() to phase() === 'hidden'; Replaced 'When closing' effect with full phase state machine: entering→requestAnimationFrame→visible, exiting→setTimeout(200ms)→hidden; Added exitTimer cleanup in onCleanup; Return block: replaced <Show when={...}> with <> + phase() !== 'hidden' && conditional render + classList on overlay and panel; Show import kept (still used by 6 inner <Show> calls for category content); Cleaned up 3 unused imports: FLAGS, LOCALE_LABELS, McpServerMap, UpdateInfo, SUPPORTED_LOCALES
- Add slide/fade CSS transitions to App.css: Added opacity: 1 to .settings-panel-overlay; Added transform: translateX(0) to .settings-panel; Appended @media (prefers-reduced-motion: no-preference) block at EOF with 6 rules: overlay transition 200ms ease-out, overlay-enter opacity:0, overlay-exit opacity:0 ease-in, panel transition 200ms ease-out, panel-enter translateX(100%), panel-exit translateX(100%) ease-in
- Verify build and tests: tsc --noEmit: zero SettingsPanel errors (pre-existing errors in TasksPanel and subagentTimeline.test.ts untouched); pnpm test: 35 test files, 643 tests, ALL passed; pnpm run build: built in 13.83s, no errors
