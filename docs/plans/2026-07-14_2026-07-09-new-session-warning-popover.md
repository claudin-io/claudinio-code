# Plan: New Session Warning Popover

## Context
O botão "+ New" no header do ChatPanel atualmente fica **desabilitado** (`disabled`) quando o status da sessão é "thinking" ou "awaiting_approval". O usuário quer que o botão **fique sempre ativo**, mas que exiba um popover de alerta ao ser clicado quando houver uma sessão em andamento, avisando que a sessão atual será parada e que é possível retomá-la pelo History.

## Solution Design

### 1. Novo Componente: `NewSessionPopover`
Criar em `src/components/NewSessionPopover.tsx` seguindo o padrão dos popovers existentes (`FileMentionPopover`, etc.):

- **Posicionamento:** Abaixo do botão, alinhado à esquerda
- **Largura:** ~320px fixa
- **Estilo:** `bg-surface-1`, `border border-border-subtle`, `shadow-lg`, `rounded-lg`
- **Ícone:** Material Symbols "warning" (triângulo com exclamação), SVG fornecido pelo usuário
- **Tom:** Atenção/aviso — borda ou detalhe em tom âmbar/warning
- **Backdrop:** Overlay transparente (`fixed inset-0 z-40`) para capturar clique externo
- **Fechamento:** Clique fora, tecla Escape
- **Botões:** "Create new" (cria nova sessão) e "Go back" (fecha popover)
- **Dimensões internas:** Padding `p-4`, ícone ~32px, texto descritivo ~13px

### 2. Mudanças no `ChatPanel.tsx`
- **Botão "+ New" (linha 1746):** Remover `disabled={status() === "thinking" || status() === "awaiting_approval"}`
- **Handler `startNewSession` (linha 1558):** Alterar lógica:
  - Se `isBusy(status())` (thinking, awaiting_approval, awaiting_input) → mostrar popover
  - Se `!isBusy(status())` (idle, done, error) → criar nova sessão diretamente (fluxo atual)
- **Estado local:** Adicionar `const [showNewPopover, setShowNewPopover] = createSignal(false)`
- **Renderização condicional:** `<Show when={showNewPopover()}>` com o componente `<NewSessionPopover>`
- **Passar posição do botão para o popover** via ref (`currentTarget.getBoundingClientRect()`)

### 3. Locale strings (`en-US.ts` e `pt-BR.ts`)
Adicionar entradas para o texto do popover e os botões.

#### en-US:
```
"chat.header.newPopover.title": "Session in progress",
"chat.header.newPopover.body": "Starting a new session will stop the current one. You can resume it later from History.",
"chat.header.newPopover.create": "Create new",
"chat.header.newPopover.goBack": "Go back",
```

#### pt-BR:
```
"chat.header.newPopover.title": "Sessão em andamento",
"chat.header.newPopover.body": "Iniciar uma nova sessão irá parar a atual. Você pode retomá-la depois pelo Histórico.",
"chat.header.newPopover.create": "Criar nova",
"chat.header.newPopover.goBack": "Voltar",
```

### 4. Ícone Alert
O SVG fornecido pelo usuário será registrado como um novo ícone no componente `Icon.tsx`, com o nome `"alert-triangle"` (já existe um com esse nome mas com paths diferentes — vamos substituir ou criar um novo nome como `"warning-triangle"`).

Na verdade, olhando o `Icon.tsx`, já existe `"alert-triangle"` com paths pixel-art. Vou adicionar o SVG do usuário como um novo ícone, talvez como `"alert"` para manter o caminho mais curto.

### 5. Fluxo de interação
```
Usuário clica "+ New"
  ├─ status está busy (thinking/awaiting_approval/awaiting_input)?
  │    └─ Sim → abre popover
  │         ├─ "Create new" → chama startNewSession() (para sessão atual, cria nova)
  │         └─ "Go back" → fecha popover
  └─ Não → cria nova sessão diretamente (fluxo atual)
```

## Risks
- O posicionamento do popover deve considerar que o botão fica no header e pode estar próximo à borda superior da janela — o popover abre abaixo do botão, que é seguro.
- Se a janela for muito estreita, o popover pode extravasar à esquerda/direita. O componente deve incluir viewport clamping (como o `ContextMenu` existente faz).

## Tasks Summary
1. Add alert icon SVG to `Icon.tsx`
2. Add locale strings (en-US, pt-BR)
3. Create `NewSessionPopover` component
4. Modify `ChatPanel.tsx` — button always active + popover logic


## Implementation Log — 2026-07-14 06:02
**Summary:** Always-active +New button + NewSessionPopover warning component
**Changed files:** M src/components/ChatPanel.tsx, M src/components/Icon.tsx, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-14_2026-07-09-new-session-warning-popover.md, ?? src/components/NewSessionPopover.tsx
**Commits:** _(git unavailable or none)_
**Journal:** Key decisions and gotchas:
- Created `alert-triangle-filled` as a new icon separate from the existing pixel-art `alert-triangle` since they have different visual styles.
- The button removal of `disabled` was straightforward but required extracting `e.currentTarget.getBoundingClientRect()` from the `onClick` event to position the popover correctly below the button.
- Used `isBusy()` from `workspaceStatus.ts` (already exported) which covers `thinking | awaiting_approval | awaiting_input` — exactly the states where a session is in progress.
- The `handleConfirmNew` function duplicates the session reset logic from `startNewSession` because the popover needed a different code path: it's triggered when busy and needs to stop the ongoing session before creating a new one. Extracting to a helper could be a future refactor but YAGNI applies for now.
- Import `DOMRect` type doesn't need explicit import in SolidJS — it's a standard browser type available globally.
- No TypeScript errors in changed files; only pre-existing test errors in subagentTimeline.test.ts.
- The `buttonRect` is cleared to `null` when popover closes, so `Show when={showNewPopover() && buttonRect()}` acts as a safety guard against rendering without position data.

**Task journal:**
- Add alert icon SVG to Icon.tsx: User provided specific SVG icon (Material Symbols Warning triangle); Added as 'alert-triangle-filled' entry in PATHS record in Icon.tsx
- Add locale strings for popover content: User confirmed English text; Added en-US: 'Starting a new session will stop the current one. You can resume it later from History.' + 'Create new' / 'Go back'; Added pt-BR: 'Iniciar uma nova sessão irá parar a atual. Você pode retomá-la depois pelo Histórico.' + 'Criar nova' / 'Voltar'
- Create NewSessionPopover component: Uses Portal from solid-js/web with transparent backdrop; Positioned below the button (top + height + 4px); ~320px width with amber border (border-amber-500/30); Escape key closes, backdrop click closes; Icon: alert-triangle-filled in amber; Buttons: 'Create new' (accent) and 'Go back' (ghost)
- Modify ChatPanel.tsx: always-active button + popover trigger: Removed disabled prop from '+ New' button; Added showNewPopover and buttonRect signals; startNewSession now shows popover if isBusy(), proceeds otherwise; handleConfirmNew: closes popover, then runs newSession flow; Popover receives position from e.currentTarget.getBoundingClientRect(); buttonRect is cleared on popover close
