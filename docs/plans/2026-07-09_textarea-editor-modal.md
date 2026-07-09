# Solution Design: Botão de Edição com Modal Monaco Editor

## 1. Context / Problem Statement
O usuário quer um botão com ícone de "notebook-pen" ao lado do botão de anexar (paperclip) no ChatPanel. Ao clicar, abre uma modal com Monaco Editor (já instalado no projeto) contendo o texto atual do textarea. Ao fechar a modal (X, ESC, ou clique fora), o texto editado volta automaticamente para o textarea de input.

**CONFIRMADO pelo usuário:**
- Usar Monaco Editor (já é dependência)
- Botão ao lado do paperclip, sempre visível
- Fechar modal = salvar texto de volta (sem botão Aplicar/Cancelar)

## 2. Goal (Definition of Done)
- Botão com ícone `notebook-pen` visível ao lado do paperclip no input do ChatPanel
- Modal com Monaco Editor que carrega o conteúdo do textarea ao abrir
- Fechar modal por qualquer meio (X, ESC, backdrop click) restaura o texto no textarea
- Comportamento preservado: textarea continua funcionando normalmente, envio via Enter funciona igual

## 3. Key Findings (Prova Real)
- **Monaco Editor**: Já está em `package.json` como `"monaco-editor": "^0.55.1"` — não precisa instalar nada
- **Icon.tsx**: Usa sistema de SVG paths pixel-art, não Lucide. É necessário criar o path para `notebook-pen`
- **ChatPanel.tsx**: ~2782 linhas. O textarea está na seção final do JSX, dentro de um `<div class="flex items-center gap-2 rounded-lg border...">`. O botão paperclip é o primeiro filho.
- **SolidJS**: Framework usado. Componentes usam `createSignal`, `createEffect`, etc.
- **Modal existente**: Já existe `SubagentModal` no ChatPanel que serve como referência de estilo (backdrop, z-50, ESC handler, clique fora)

## 4. Authoritative Inputs
- Ícone de referência: `https://icones.js.org/collection/all?s=notebook&icon=lucide:notebook-pen` (Lucide) — precisamos adaptar para o estilo pixel-art
- Posição: ao lado esquerdo do paperclip, na barra de input

## 5. Changes (Steps)

### Step 1: Adicionar ícone `notebook-pen` no Icon.tsx
- **Target:** `src/components/Icon.tsx`
- **Mutation:** Adicionar entrada `"notebook-pen"` no objeto `PATHS` com paths SVG no estilo pixel-art, e adicionar ao tipo `IconName`
- **Why:** O sistema de ícones é customizado, precisa de um path SVG novo
- **Constraints:** Seguir o estilo pixel-art (grid-based) usado nos outros ícones

### Step 2: Criar componente `TextEditorModal.tsx`
- **Target:** `src/components/TextEditorModal.tsx` (novo arquivo)
- **Mutation:** Criar componente SolidJS que:
  - Recebe props: `initialText: string`, `onClose: (text: string) => void`
  - Renderiza Monaco Editor dentro de um container com altura fixa (~70vh)
  - Backdrop escuro com `bg-black/40`
  - Fecha no ESC, clique fora, e botão X — sempre chamando `onClose` com o texto atual
  - Usa tema dark (consistente com o app)
- **Why:** Componente reutilizável e isolado, fácil de testar
- **Constraints:** SolidJS `onMount`/`onCleanup` para lifecycle do Monaco

### Step 3: Integrar no ChatPanel.tsx
- **Target:** `src/components/ChatPanel.tsx`
- **Mutation:**
  - Importar `TextEditorModal`
  - Adicionar `createSignal<boolean>` para controlar visibilidade da modal
  - Adicionar botão `<button>` com `<Icon name="notebook-pen">` ANTES do botão paperclip (dentro do mesmo flex container)
  - Renderizar `<TextEditorModal>` condicionalmente com `<Show when={showEditor()}>`
  - Ao fechar: `setInput(text)` com o texto retornado
- **Why:** Integração mínima, sem alterar lógica existente
- **Constraints:** Não alterar comportamento de envio, atalhos, ou outros botões

## 6. Verification Plan

### Dry-run / Preview
- Revisar diff antes de aplicar

### Apply
- `pnpm build` deve compilar sem erros TypeScript
- `pnpm test` deve passar (testes existentes)

### End-to-end
1. Abrir o app, abrir um workspace
2. Digitar texto no textarea
3. Clicar no botão notebook-pen → modal abre com Monaco mostrando o texto
4. Editar o texto no Monaco
5. Fechar modal (testar: X, ESC, clique fora) → texto aparece de volta no textarea
6. Enviar mensagem → funciona normalmente

### Edge / no-op
- Abrir modal com textarea vazio → Monaco abre vazio, fecha sem erros
- Digitar no textarea enquanto modal está aberta (não é possível pois modal é modal)
- Re-abrir modal múltiplas vezes → sempre carrega o texto atual

### Regression
- Envio de mensagem (Enter / botão send) continua funcionando
- Paperclip, modo brain/builder, @-mentions, `<skill>` tags não são afetados

## 7. Tasks Summary
1. Add `notebook-pen` icon to Icon.tsx
2. Create `TextEditorModal.tsx` component
3. Integrate button + modal into ChatPanel.tsx
4. Build + test verification
