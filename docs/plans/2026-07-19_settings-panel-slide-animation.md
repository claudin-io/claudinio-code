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
