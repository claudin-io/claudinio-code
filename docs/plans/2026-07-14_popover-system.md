# Popover System — Unified Positioning Component

## 1. Context / Problem Statement

**Bug:** `NewSessionPopover` transborda a window quando o botão "+ New" está próximo à borda direita. O popover usa `left: ${props.position.left}px` sem nenhum clamping de viewport.

**Root cause (confirmed):** `src/components/NewSessionPopover.tsx:43` — o `left` é o valor cru do `getBoundingClientRect()` do botão, sem viewport clamping. O plano original (`docs/plans/2026-07-14_2026-07-09-new-session-warning-popover.md`, linha 58) já identificava esse risco mas nunca foi implementado.

**Systemic issue:** Existem 8 popovers/dropdowns no codebase, cada um com sua própria lógica de posicionamento manual, Portal, backdrop, e Escape key. Nenhum usa biblioteca de posicionamento (floating-ui/Popper). Apenas 2 fazem viewport clamping (`ContextMenu` e os mention popovers). O resto depende de sorte.

## 2. Goal (Definition of Done)

- Componente `<Popover>` reutilizável que gerencia Portal, backdrop, Escape key, posicionamento inteligente (anchor + origin points), flip quando não couber na viewport, e reação a resize/scroll.
- Todos os 8 popovers/dropdowns existentes migrados para usar `<Popover>`.
- `NewSessionPopover` nunca transborda a window, independente da posição do botão.

## 3. Key Findings (Real Proof)

| Finding | Source | Method |
|---|---|---|
| 8 popovers existem no codebase, todos com posicionamento manual | `popover-explorer` subagent | grep + file_outline + read_file across all components |
| Nenhuma biblioteca de posicionamento (floating-ui, Popper) | `popover-explorer` subagent | package.json search, import grep |
| `NewSessionPopover` é o único sem viewport clamping | `NewSessionPopover.tsx:43` | read_file — `left: \`${props.position.left}px\`` sem clamping |
| `ContextMenu` tem clamping manual (`clampX/clampY`) | `ContextMenu.tsx:25-27` | read_file |
| Mention popovers têm clamping + flip manual | `ChatPanel.tsx:2243-2258` | read_file |
| Task hover popover usa `right: 48px` fixo, não viewport-aware | `TasksPanel.tsx:163-168` | read_file |
| Sessions/Plans dropdowns usam `absolute` relativo ao header, não Portal | `ChatPanel.tsx:1818-1862` | read_file |
| `getCaretCoordinates()` é a única utility de posicionamento no codebase | `ChatPanel.tsx:852-895` | read_file |
| SolidJS 1.9 + Tailwind v4 + TypeScript strict | `ui-infra-explorer` subagent | package.json, tsconfig.json, vite.config.ts |

## 4. Authoritative Inputs

| Input | Value | Source |
|---|---|---|
| Margem da viewport | 8px | Decisão do usuário (consistente com MARGIN existente) |
| Estratégia de fallback | Flip simples (tenta oposto), depois clamp | Decisão do usuário |
| Modo de âncora | `triggerRef` (elemento DOM) como padrão; `position` override para coordenadas | Decisão do usuário |
| Anchor + origin points | Ambos `{x: 0..1, y: 0..1}` | Decisão do usuário |
| Escopo de migração | Todos os 8 popovers existentes | Decisão do usuário |
| Abordagem de componente | Componente `<Popover>` wrapper único (Portal + backdrop + Escape + posicionamento) | Decisão do usuário |

## 5. Solution Design

### 5.1 API do Componente `<Popover>`

```tsx
interface AnchorPoint {
  x: number; // 0 = left, 0.5 = center, 1 = right
  y: number; // 0 = top, 0.5 = center, 1 = bottom
}

interface PopoverProps {
  open: boolean;
  onClose: () => void;
  triggerRef?: HTMLElement | (() => HTMLElement | undefined | null);
  anchorPoint?: AnchorPoint;   // default {x: 0, y: 1}
  originPoint?: AnchorPoint;   // default {x: 0, y: 0}
  margin?: number;              // default 8
  showBackdrop?: boolean;      // default true
  position?: { top: number; left: number; width?: number; height?: number };
  children: JSX.Element;
  class?: string;
}
```

### 5.2 Algoritmo de Posicionamento

```
1. OBTER RETÂNGULO DO TRIGGER:
   - Se triggerRef: getBoundingClientRect()
   - Se position: usar coordenadas fornecidas {top, left, width:0, height:0}

2. COMPUTAR ÂNCORA GLOBAL:
   anchorX = rect.left + rect.width * anchorPoint.x
   anchorY = rect.top + rect.height * anchorPoint.y

3. MEDIR POPOVER: ResizeObserver captura {width, height} reais

4. TENTAR POSIÇÃO PREFERIDA:
   popoverLeft = anchorX - popoverWidth * originPoint.x
   popoverTop  = anchorY - popoverHeight * originPoint.y

5. VERIFICAR OVERFLOW:
   overflowRight  = (popoverLeft + popoverWidth)  > (window.innerWidth - margin)
   overflowLeft   = popoverLeft < margin
   overflowBottom = (popoverTop + popoverHeight) > (window.innerHeight - margin)
   overflowTop    = popoverTop < margin

6. FLIP (se necessário, um eixo por vez):
   Se overflowRight ou overflowLeft → flip originPoint.x (1 - x)
   Se overflowBottom ou overflowTop → flip originPoint.y (1 - y)
   Recomputar popoverLeft, popoverTop com novo originPoint

7. CLAMP (se ainda overflow):
   popoverLeft = clamp(popoverLeft, margin, window.innerWidth - popoverWidth - margin)
   popoverTop  = clamp(popoverTop, margin, window.innerHeight - popoverHeight - margin)

8. APLICAR: style = { position: 'fixed', left: popoverLeft, top: popoverTop }
```

### 5.3 Reatividade

- `ResizeObserver` no elemento popover para capturar dimensões reais
- `window.addEventListener("resize", ...)` para recalcular quando a janela muda
- `createEffect` que reage a mudanças no trigger rect, popover size, e window size
- Primeiro render com `visibility: hidden`; após posicionamento calculado, `visibility: visible`

### 5.4 Mapeamento de Migração

| # | Popover | Tipo de Âncora | anchorPoint | originPoint | Notas |
|---|---|---|---|---|---|
| 1 | NewSessionPopover | triggerRef | {0, 1} | {0, 0} | Bug fix original |
| 2 | FileMentionPopover | position override | {0, 0} | {0, 1} ou {0, 0} | Caret coords, smart flip existente vira automático |
| 3 | TagMentionPopover | position override | {0, 0} | {0, 1} | Caret coords do `<` char |
| 4 | SkillMentionPopover | position override | {0, 0} | {0, 1} | Caret coords |
| 5 | ContextMenu | position override | {0, 0} | {0, 0} | Mouse clientX/clientY |
| 6 | Task hover | triggerRef | {1, 0} | {0, 0} | Right side of task dot |
| 7 | Sessions dropdown | triggerRef | {1, 1} | {1, 0} | Era `absolute`, vira `fixed` via Popover |
| 8 | Plans dropdown | triggerRef | {1, 1} | {1, 0} | Igual sessions |

## 6. Changes (Steps)

### Step 1: Criar `src/components/Popover.tsx`
- **Target:** Novo arquivo
- **Mutation:** Componente `<Popover>` completo com Portal, backdrop, Escape, posicionamento inteligente, ResizeObserver
- **Why:** Fundação do sistema — todos os popovers existentes serão migrados para usar este componente
- **Constraints:** SolidJS idioms (createSignal, createEffect, onMount, onCleanup), zero dependências externas
- **Wiring sketch:** Exporta `Popover` e tipos `AnchorPoint`, `PopoverProps`. Usado como wrapper:
  ```tsx
  <Popover open={show()} onClose={close} triggerRef={triggerEl}>
    <div>content</div>
  </Popover>
  ```

### Step 2: Criar `src/components/__tests__/Popover.test.tsx`
- **Target:** Novo arquivo
- **Mutation:** Testes unitários para posicionamento, flip, clamp, resize, backdrop click, Escape key
- **Why:** Garantir que o algoritmo de posicionamento funciona em todas as bordas da viewport
- **Constraints:** Vitest + jsdom, sem acesso a `window.innerWidth/Height` reais — mockar

### Step 3: Migrar `NewSessionPopover` (bug fix)
- **Target:** `src/components/NewSessionPopover.tsx`
- **Mutation:** Substituir Portal + backdrop + posicionamento manual por `<Popover>` wrapper. Remover props `position` em favor de `triggerRef`. `ChatPanel.tsx` deve passar ref do botão em vez de `buttonRect`.
- **Why:** Corrige o bug de overflow
- **Wiring:**
  ```tsx
  // ChatPanel.tsx — ao invés de setButtonRect + setShowNewPopover:
  let newButtonRef: HTMLButtonElement | undefined;
  // No onClick: setShowNewPopover(true) (sem getBoundingClientRect)
  // No JSX:
  <Popover open={showNewPopover()} onClose={close} triggerRef={newButtonRef}
           anchorPoint={{x:0, y:1}} originPoint={{x:0, y:0}}>
    <NewSessionPopoverContent onConfirm={...} onClose={...} />
  </Popover>
  ```
  E `NewSessionPopover` vira um componente de conteúdo puro (sem Portal, sem backdrop, sem posicionamento).

### Step 4: Migrar `FileMentionPopover`
- **Target:** `src/components/FileMentionPopover.tsx` + `ChatPanel.tsx`
- **Mutation:** Envolver com `<Popover>` usando `position` override. Remover Portal, backdrop, Escape, posicionamento manual. Manter keyboard nav, Fuse search.
- **Why:** Unificar posicionamento; smart flip existente vira automático
- **Wiring:** ChatPanel passa `position={mentionPosition()}` para Popover; anchor/origin padrão cuida do resto

### Step 5: Migrar `TagMentionPopover`
- **Target:** `src/components/TagMentionPopover.tsx` + `ChatPanel.tsx`
- **Mutation:** Mesmo padrão do FileMentionPopover. `position` contém `{top, left}` do caret; anchor/origin `{0,0}/{0,1}` para aparecer acima.
- **Why:** Unificar posicionamento

### Step 6: Migrar `SkillMentionPopover`
- **Target:** `src/components/SkillMentionPopover.tsx` + `ChatPanel.tsx`
- **Mutation:** Mesmo padrão. Posicionamento via `position` override do caret.
- **Why:** Unificar posicionamento

### Step 7: Migrar `ContextMenu`
- **Target:** `src/components/ContextMenu.tsx`
- **Mutation:** Substituir Portal + backdrop + clampX/clampY por `<Popover>` com `position={{top: y, left: x}}` e anchor/origin `{0,0}/{0,0}`.
- **Why:** Remover lógica de clamping duplicada

### Step 8: Migrar Task hover popover
- **Target:** `src/components/TasksPanel.tsx`
- **Mutation:** Substituir Portal + posicionamento fixo (`right: 48px`) por `<Popover>` com `triggerRef` no task dot. anchorPoint `{1, 0}`, originPoint `{0, 0}`.
- **Why:** Fica viewport-aware em vez de posição fixa; popover não transborda em janelas estreitas

### Step 9: Migrar Sessions dropdown
- **Target:** `src/components/ChatPanel.tsx` ~linhas 1818-1839
- **Mutation:** Converter de `absolute` relativo ao header para `<Popover>` com `triggerRef` no botão History. anchorPoint `{1, 1}`, originPoint `{1, 0}`.
- **Why:** Unificar dismiss (Escape, clique-fora) e posicionamento

### Step 10: Migrar Plans dropdown
- **Target:** `src/components/ChatPanel.tsx` ~linhas 1842-1862
- **Mutation:** Mesmo padrão do Sessions dropdown.
- **Why:** Consistência

## 7. Verification Plan

### 7.1 Testes unitários
```bash
pnpm vitest run src/components/__tests__/Popover.test.tsx
```
- Casos: sem overflow, overflow direita, overflow esquerda, overflow baixo, overflow cima, flip horizontal, flip vertical, clamp quando flip não resolve, resize recalcula, Escape fecha, clique no backdrop fecha

### 7.2 Build
```bash
pnpm tsc --noEmit
```
- Zero erros de tipo

### 7.3 Visual (manual)
- Redimensionar a janela para ~900px de largura (mínimo da Tauri)
- Clicar "+ New" com sessão ocupada — popover deve aparecer totalmente visível
- Testar cada popover em viewport estreita (900×600) e larga (1920×1080)
- Menção @ no textarea: popover não transborda quando caret está na borda direita

### 7.4 Regressão
- Todos os popovers existentes continuam funcionando com mesma aparência e comportamento
- Keyboard navigation nos mention popovers (ArrowDown/Up/Enter) intacta
- Task hover popover: mouseEnter/mouseLeave com delay de 150ms intacto

## 8. Risks

| Risk | Mitigation |
|---|---|
| ResizeObserver não disponível em jsdom nos testes | Mockar ResizeObserver no setup de teste; testar posicionamento via cálculos diretos |
| Migração dos mention popovers quebrar keyboard nav | Manter a lógica de keyboard handling dentro do conteúdo do popover, não no wrapper |
| Sessões/Plans dropdowns migrarem de `absolute` para `fixed` e mudarem posição | Ajustar anchor/origin points para compensar a mudança de sistema de coordenadas |
| Task hover popover perder o delay de 150ms | O delay é gerido pelo TasksPanel, não pelo Popover — permanece intacto |


## Implementation Log — 2026-07-14 06:46
**Summary:** Create unified <Popover> component with anchor/origin points, flip+clamp positioning, ResizeObserver, and Escape+backdrop dismiss. Migrate all 8 existing popovers to use it, fixing the NewSessionPopover overflow bug.
**Changed files:** M src/components/ChatPanel.tsx, M src/components/ContextMenu.test.tsx, M src/components/ContextMenu.tsx, M src/components/FileMentionPopover.test.tsx, M src/components/FileMentionPopover.tsx, M src/components/NewSessionPopover.tsx, M src/components/SkillMentionPopover.test.tsx, M src/components/SkillMentionPopover.tsx, M src/components/TagMentionPopover.test.tsx, M src/components/TagMentionPopover.tsx, M src/components/TasksPanel.tsx, ?? docs/plans/2026-07-14_popover-system.md, ?? src/components/Popover.test.tsx, ?? src/components/Popover.tsx
**Commits:** _(git unavailable or none)_
**Journal:** ## Implementation Journal

### Key Decisions
- **computePosition** exported as pure function for testability — 18 unit tests on the algorithm alone
- `triggerRef` accepts both `HTMLElement` and lazy `() => HTMLElement | undefined | null` for SolidJS reactive refs (TaskPanel hover uses this pattern)
- `position` prop override for cases without a DOM element (mention popovers with caret coordinates, ContextMenu with mouse coords)
- `ResizeObserver` for measuring popover dimensions — `visibility: hidden` until first measurement
- `window.resize` listener with version counter to trigger recalculation
- Flip strategy: flips origin point X for horizontal overflow, Y for vertical, then clamps

### Gotchas
- **Shared mutation in tests**: the original computePosition tests mutated a shared `const trigger` object, causing cascading failures. Fixed with `makeTrigger()` factory that spreads defaults.
- **`ContextMenu.test.tsx` needed ResizeObserver mock**: since ContextMenu now wraps Popover, the test needed a ResizeObserver stub. Also simplified tests: removed manual clamp tests (Popover owns positioning now).
- **Portal mock in test-setup.ts**: `vi.mock("solid-js/web", ...)` replaces Portal with identity function. This means Popover tests run without actual Portal behavior — content is rendered inline in document.body, which is sufficient for DOM assertion testing.
- **zIndex vs "z-index"**: SolidJS style binding requires `"z-index"` (kebab-case) not `zIndex` (camelCase) for CSS custom properties.

### Migration Summary
All 8 popovers migrated to use `<Popover>` component:
1. **NewSessionPopover** (bug fix) — now uses triggerRef instead of getBoundingClientRect raw coords
2. **FileMentionPopover** — position via caret coords
3. **TagMentionPopover** — position via caret coords
4. **SkillMentionPopover** — position via caret coords
5. **ContextMenu** — position via mouse coords
6. **Task hover popover** — triggerRef with hoveredElement signal
7. **Sessions dropdown** — triggerRef on History button
8. **Plans dropdown** — triggerRef on Plans button

### Removed Duplicated Code
- 8 instances of `<Portal>`, backdrop divs, and manual positioning removed
- 6 instances of Escape keydown handlers removed
- 2 click-outside createEffect blocks removed
- Manual clampX/clampY functions removed from ContextMenu
- `buttonRect`/`setButtonRect` signals removed from ChatPanel
- `sessionsRef`/`plansRef` DOM refs removed
- `hoveredTop` signal replaced with `hoveredElement`

**Task journal:**
- Create Popover component (core engine): Created Popover.tsx with computePosition(), Popover component with Portal/backdrop/Escape/ResizeObserver/flip/clamp logic.
- Create Popover unit tests: 18 tests pass in Popover.test.tsx. Uses makeTrigger factory to avoid shared-mutation issue.
- Migrate NewSessionPopover (bug fix): NewSessionPopover.tsx stripped to pure content (removed Portal, onMount, onCleanup, position prop). ChatPanel.tsx: replaced buttonRect signal with newButtonRef, uses <Popover> wrapper. pnpm tsc passes.
- Migrate FileMentionPopover: FileMentionPopover.tsx stripped of Portal/backdrop/positioning. ChatPanel wraps in <Popover position={mentionPosition()}>. Tests updated (removed position/backdrop/Escape tests, kept Fuse/keyboard tests).
- Migrate TagMentionPopover: TagMentionPopover.tsx stripped of Portal/backdrop/bottom/left. ChatPanel wraps in <Popover position>.
- Migrate SkillMentionPopover: SkillMentionPopover.tsx stripped of Portal/backdrop/bottom/left. ChatPanel wraps in <Popover position>.
- Migrate ContextMenu: ContextMenu.tsx migrated: removed Portal, clampX/clampY, Escape handler. Uses <Popover position>. Tests updated with ResizeObserver mock, simplified (no more manual clamp tests).
- Migrate task hover popover: TasksPanel.tsx: replaced Portal with <Popover>. Changed hoveredTop signal to hoveredElement signal. triggerRef={() => hoveredElement()}, anchorPoint {1,0}, originPoint {0,0}, showBackdrop=false.
- Migrate Sessions dropdown: Sessions dropdown: replaced <Show> + absolute div with <Popover triggerRef={historyButtonRef}>. Removed sessionsRef, click-outside createEffect. Added historyButtonRef.
- Migrate Plans dropdown: Plans dropdown: same pattern as Sessions. Removed plansRef, click-outside createEffect. Added plansButtonRef.


## Implementation Log — 2026-07-14 07:13
**Changed files:** M src/components/ChatPanel.tsx, M src/components/ContextMenu.test.tsx, M src/components/ContextMenu.tsx, M src/components/FileMentionPopover.test.tsx, M src/components/FileMentionPopover.tsx, M src/components/NewSessionPopover.tsx, M src/components/SkillMentionPopover.test.tsx, M src/components/SkillMentionPopover.tsx, M src/components/TagMentionPopover.test.tsx, M src/components/TagMentionPopover.tsx, M src/components/TasksPanel.tsx, ?? docs/plans/2026-07-14_popover-system.md, ?? src/components/Popover.test.tsx, ?? src/components/Popover.tsx
**Commits:** _(git unavailable or none)_
**Journal:** ## Bug Fix: ResizeObserver never initializes when `open` starts `false`

**Root Cause**: Two compounding issues:
1. `onMount` runs once at component mount. When `open` starts `false`, the `<Show when={props.open}>` condition blocks the Portal children from rendering, so `popoverRef` (the `let` variable) is `undefined`. The `ResizeObserver` is never created. When `open` flips to `true` later, the DOM appears but `onMount` already passed — the observer is gone forever.
2. Even with `createEffect`, a plain `let popoverRef` is invisible to SolidJS reactivity. The ref callback (`ref={popoverRef!}`) fires during DOM creation but doesn't trigger any signal update, so the effect never re-runs.

**Fix**: Two changes in [Popover.tsx](src/components/Popover.tsx):
1. `onMount` → `createEffect` that reads `popoverEl()` and `props.open` reactively
2. `let popoverRef` → `const [popoverEl, setPopoverEl] = createSignal()` with `ref={setPopoverEl}`

Now when `open` flips to `true`, SolidJS renders the Portal, the ref callback fires `setPopoverEl`, and the `createEffect` reactively re-runs, sees the element, creates the `ResizeObserver`, calls `setReady(true)`, and the popover becomes `visibility: visible`.

**Task journal:**
- Create Popover component (core engine): Created Popover.tsx with computePosition(), Popover component with Portal/backdrop/Escape/ResizeObserver/flip/clamp logic.
- Create Popover unit tests: 18 tests pass in Popover.test.tsx. Uses makeTrigger factory to avoid shared-mutation issue.
- Migrate NewSessionPopover (bug fix): NewSessionPopover.tsx stripped to pure content. ChatPanel uses newButtonRef + <Popover>. Bug fixed: no more raw getBoundingClientRect.
- Migrate FileMentionPopover: FileMentionPopover.tsx stripped. ChatPanel wraps in <Popover position={mentionPosition()}>. Fuse/keyboard tests preserved.
- Migrate TagMentionPopover: TagMentionPopover.tsx stripped of Portal/backdrop/bottom/left. ChatPanel wraps in <Popover position>.
- Migrate SkillMentionPopover: SkillMentionPopover.tsx stripped of Portal/backdrop/bottom/left. ChatPanel wraps in <Popover position>.
- Migrate ContextMenu: ContextMenu.tsx migrated. Tests updated with ResizeObserver mock.
- Migrate task hover popover: TasksPanel.tsx: replaced Portal with <Popover>. hoveredTop -> hoveredElement signal. triggerRef={() => hoveredElement()}.
- Migrate Sessions dropdown: Sessions dropdown migrated. Removed sessionsRef, click-outside createEffect. Added historyButtonRef.
- Migrate Plans dropdown: Plans dropdown migrated. Removed plansRef, click-outside createEffect. Added plansButtonRef.
- Full verification + ResizeObserver lifecycle bugfix: Bug: onMount + let popoverRef = when open starts false, popoverRef is undefined -> ResizeObserver never set up -> ready stays false -> visibility:hidden forever.; Fix: replaced onMount with createEffect + signal-based ref (setPopoverEl). Now when open becomes true, the effect re-runs reactively, sees the DOM element, creates the ResizeObserver, sets ready() -> popover visible.; 628/628 tests pass, 34/34 files pass.


## Implementation Log — 2026-07-14 09:29
**Changed files:** M src/components/ChatPanel.tsx, M src/components/ContextMenu.test.tsx, M src/components/ContextMenu.tsx, M src/components/FileMentionPopover.test.tsx, M src/components/FileMentionPopover.tsx, M src/components/NewSessionPopover.tsx, M src/components/SkillMentionPopover.test.tsx, M src/components/SkillMentionPopover.tsx, M src/components/TagMentionPopover.test.tsx, M src/components/TagMentionPopover.tsx, M src/components/TasksPanel.tsx, ?? docs/plans/2026-07-14_popover-system.md, ?? src/components/Popover.test.tsx, ?? src/components/Popover.tsx
**Commits:** _(git unavailable or none)_
**Journal:** ## Bug 2: Mention popovers at (0,0) — `position: fixed` inside content + position prop treated as trigger rect

**Root Cause**: Two compounding issues when using the `<Popover position={{top, left}}>` override path:

1. **`position: fixed` on inner content**: FileMentionPopover, TagMentionPopover, and SkillMentionPopover all had `class="fixed z-50"` on their root `<div>`. This CSS overrides the Popover wrapper's `top`/`left` — the inner div anchors itself at viewport (0,0) regardless of what the outer container says.

2. **`position` prop treated as trigger rect**: `getTriggerRect()` mapped `props.position` to a `Rect` with `width: 0, height: 0` (for missing `width`/`height`). This was passed through `computePosition()` which applies anchorPoint/originPoint math. With `{x:0, y:0}` origin, `left = anchorX - 0*popoverWidth = 0`, `top = anchorY - 0*popoverHeight = 0` — effectively the popover's (0,0) corner lands at the trigger coordinate, which was `{top:0, left:0}` for the width=0/height=0 case.

3. **Inverted coordinates for Tag/Skill**: The old code computed `bottom = window.innerHeight - pos.top + 4` (intended for `position: absolute` with `bottom` CSS property) and stored it in `top`. Now `position` is a viewport-top coordinate.

**Fixes applied**:
1. Removed `fixed z-50` from all three mention popover root divs
2. `Popover.tsx`: when `props.position` is set, bypass `computePosition` entirely — set `top`/`left` directly as the final position
3. ChatPanel.tsx: changed tag/skill position calculation from bottom-distance to viewport-top (`pos.top - 4`)

**Task journal:**
- Create Popover component (core engine): Created Popover.tsx with computePosition(), Popover component with Portal/backdrop/Escape/ResizeObserver/flip/clamp logic.
- Create Popover unit tests: 18 tests pass in Popover.test.tsx. Uses makeTrigger factory to avoid shared-mutation issue.
- Migrate NewSessionPopover (bug fix): NewSessionPopover.tsx stripped to pure content. ChatPanel uses newButtonRef + <Popover>. Bug fixed: no more raw getBoundingClientRect.
- Migrate FileMentionPopover: FileMentionPopover.tsx stripped. ChatPanel wraps in <Popover position={mentionPosition()}>. Fuse/keyboard tests preserved.
- Migrate TagMentionPopover: TagMentionPopover.tsx stripped of Portal/backdrop/bottom/left. ChatPanel wraps in <Popover position>.
- Migrate SkillMentionPopover: SkillMentionPopover.tsx stripped of Portal/backdrop/bottom/left. ChatPanel wraps in <Popover position>.
- Migrate ContextMenu: ContextMenu.tsx migrated. Tests updated with ResizeObserver mock.
- Migrate task hover popover: TasksPanel.tsx: replaced Portal with <Popover>. hoveredTop -> hoveredElement signal. triggerRef={() => hoveredElement()}.
- Migrate Sessions dropdown: Sessions dropdown migrated. Removed sessionsRef, click-outside createEffect. Added historyButtonRef.
- Migrate Plans dropdown: Plans dropdown migrated. Removed plansRef, click-outside createEffect. Added plansButtonRef.
- Full verification + ResizeObserver lifecycle bugfix: Bug: onMount + let popoverRef = when open starts false, popoverRef is undefined -> ResizeObserver never set up -> ready stays false -> visibility:hidden forever.; Fix: replaced onMount with createEffect + signal-based ref (setPopoverEl). Now when open becomes true, the effect re-runs reactively, sees the DOM element, creates the ResizeObserver, sets ready() -> popover visible.; 628/628 tests pass, 34/34 files pass.
- Fix mention popover positioning (textarea): Two root causes: (1) FileMentionPopover, TagMentionPopover, SkillMentionPopover had `class="fixed z-50"` on their root div — this overrides the Popover's own top/left with position:fixed at (0,0). (2) The `position` prop was treated as a trigger rect in getTriggerRect(), so it went through computePosition which applies anchorPoint/originPoint math on top of it.; Fix: removed `fixed z-50` from all three mention popover root divs. Changed position calculation in Popover.tsx: when `props.position` is provided, set it directly as the final coordinate (bypass computePosition). Also fixed tag/skill position calculations: they were computing bottom-distance (for old `bottom` CSS) but storing as `top`, which now receives viewport-top coordinates.
- Remove `position: fixed` from mention content divs: Removed from all three files. Content now flows inside Popover's position:fixed container.
- Fix tag/skill position coordinate system: Changed `const bottom = window.innerHeight - pos.top + 4` -> `const top = pos.top - 4` in all three call sites (tag, skill on input change, skill on tag select).
- Verify build and tests: 628/628 pass, 34/34 files. Only pre-existing subagentTimeline TS errors (unrelated).


## Implementation Log — 2026-07-14 09:37
**Changed files:** M src/components/ChatPanel.tsx, M src/components/ContextMenu.test.tsx, M src/components/ContextMenu.tsx, M src/components/FileMentionPopover.test.tsx, M src/components/FileMentionPopover.tsx, M src/components/NewSessionPopover.tsx, M src/components/SkillMentionPopover.test.tsx, M src/components/SkillMentionPopover.tsx, M src/components/TagMentionPopover.test.tsx, M src/components/TagMentionPopover.tsx, M src/components/TasksPanel.tsx, ?? docs/plans/2026-07-14_popover-system.md, ?? src/components/Popover.test.tsx, ?? src/components/Popover.tsx
**Commits:** _(git unavailable or none)_
**Journal:** ## Round 3: Position prop now routes through computePosition with anchorPoint

The `position` prop is now treated as a trigger rect (caret/mouse coordinates with height), so it passes through `computePosition()` which applies `anchorPoint`/`originPoint` for automatic flip+clamp. This required:

1. **Popover.tsx**: Merged the `position` and `triggerRef` branches into a single `computePosition` call. `position` creates a trigger rect with `width: 0, height: props.position.height ?? 0`. `triggerRef` reads `getBoundingClientRect()`. Both go through the same computePosition → anchor/flip logic.

2. **ChatPanel.tsx**: All three mention handlers (`@`, `<tag>`, `<skill>`) now pass **raw caret coordinates** from `getCaretCoordinates()` directly into the `position` signal — no pre-calculation of final position. The `Popover` call sites have `anchorPoint={{x:0,y:1}}` so the popover appears below the caret and flips automatically when near viewport edges.

3. **ContextMenu** unchanged: `anchorPoint={x:0,y:0} originPoint={x:0,y:0}` keeps the popover exactly at mouse coordinates.

**Task journal:**
- Create Popover component (core engine): Created Popover.tsx with computePosition(), Popover component with Portal/backdrop/Escape/ResizeObserver/flip/clamp logic.
- Create Popover unit tests: 18 tests pass in Popover.test.tsx. Uses makeTrigger factory to avoid shared-mutation issue.
- Migrate NewSessionPopover (bug fix): NewSessionPopover.tsx stripped to pure content. ChatPanel uses newButtonRef + <Popover>. Bug fixed: no more raw getBoundingClientRect.
- Migrate FileMentionPopover: FileMentionPopover.tsx stripped. ChatPanel wraps in <Popover position={mentionPosition()}>. Fuse/keyboard tests preserved.
- Migrate TagMentionPopover: TagMentionPopover.tsx stripped of Portal/backdrop/bottom/left. ChatPanel wraps in <Popover position>.
- Migrate SkillMentionPopover: SkillMentionPopover.tsx stripped of Portal/backdrop/bottom/left. ChatPanel wraps in <Popover position>.
- Migrate ContextMenu: ContextMenu.tsx migrated. Tests updated with ResizeObserver mock.
- Migrate task hover popover: TasksPanel.tsx: replaced Portal with <Popover>. hoveredTop -> hoveredElement signal. triggerRef={() => hoveredElement()}.
- Migrate Sessions dropdown: Sessions dropdown migrated. Removed sessionsRef, click-outside createEffect. Added historyButtonRef.
- Migrate Plans dropdown: Plans dropdown migrated. Removed plansRef, click-outside createEffect. Added plansButtonRef.
- Full verification + ResizeObserver lifecycle bugfix: Bug: onMount + let popoverRef = when open starts false, popoverRef is undefined -> ResizeObserver never set up -> ready stays false -> visibility:hidden forever.; Fix: replaced onMount with createEffect + signal-based ref (setPopoverEl). Now when open becomes true, the effect re-runs reactively, sees the DOM element, creates the ResizeObserver, sets ready() -> popover visible.; 628/628 tests pass, 34/34 files pass.
- Fix mention popover positioning (textarea): Fixed all three issues. See individual tasks.
- Remove `position: fixed` from mention content divs: Removed from FileMentionPopover, TagMentionPopover, SkillMentionPopover.
- Fix tag/skill position coordinate system: Changed bottom-distance calculation to raw caret coordinates.
- Verify build and tests: 628/628 pass, 34/34 files.
- Re-enable computePosition + anchorPoint for mention popovers: Restored position → computePosition flow (position is treated as a 0-area trigger rect with height). Added anchorPoint={{x:0,y:1}} to @mention, <tag>, and <skill> popovers. Now popover appears below caret with automatic flip when near viewport edges. ContextMenu stays at anchorPoint={0,0} (appears at mouse position).; ChatPanel: mention handlers now pass raw caret coordinates (no pre-calculation). Popover.tsx: position + triggerRef merged into single computePosition path.
