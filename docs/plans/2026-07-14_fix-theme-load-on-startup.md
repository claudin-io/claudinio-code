# Corrigir carregamento do tema salvo na inicialização

## Context

O usuário reportou que o tema escolhido (ex: Dracula, Nord, Tokyo Night) não é aplicado ao abrir o app. O tema só carrega corretamente quando ele abre o Settings (ThemePicker). O comportamento sugere que o estado do tema só é inicializado sob demanda (lazy) ao invés de na carga inicial da aplicação.

## Investigation Findings (Real Proof)

1. **Problema 1 — index.html inline script obsoleto** ([`index.html`](index.html):6-17): O script que executa **antes do JS carregar** só reconhece valores legados `"dark"`, `"light"`, `"sepia"`. Temas como `"dracula"`, `"nord"`, `"tokyo-night"` não são reconhecidos e caem no fallback do OS (`matchMedia`). O `data-theme` inicial fica incorreto.

2. **Problema 2 — initState() nunca chamado na carga inicial** ([`src/lib/theme.ts`](src/lib/theme.ts):289-295): O `createThemeState()` (que lê `localStorage` corretamente e aplica `data-theme` real) só é invocado via `createRoot()` **lazily** na primeira chamada das funções exportadas (`theme()`, `preference()`, `resolvedTheme()`). Nenhuma dessas funções é chamada durante a renderização inicial da `App`. O `ThemePicker` (Settings) é quem primeiro chama `preference()` / `setThemePreference`, corrigindo o tema só ao abrir Settings.

3. **Fluxo confirmado**: Usuário seleciona → `setThemePreference('dracula')` → `localStorage.setItem('claudinio_theme', 'dracula')` → app reinicia → index.html lê `'dracula'` → não reconhecido → fallback do OS → `data-theme` errado → `createThemeState()` nunca chamado → resultado visual incorreto até abrir Settings.

## Solution Design

### Correção 1 — index.html (script inline)

**O que mudar:** O script `IIFE` no `<head>` do `index.html` deve aceitar **qualquer valor** armazenado em `localStorage`, não apenas os valores legados. A validação deve ser: se existe um valor armazenado e ele é uma string não vazia, use-o diretamente. Os valores legacy `"dark"`/`"light"`/`"sepia"` devem ser mapeados para os IDs atuais como fallback.

```
Alteração: substituir o if de validação restrito por um que aceite qualquer string
- Se stored existe → usa stored (com mapeamento legacy)
- Senão → fallback matchMedia
```

A IIFE continuará executando antes do React carregar, prevenindo o flash de tema incorreto.

### Correção 2 — initState() na inicialização da App

**O que mudar:** Forçar `initState()` a ser chamado **assim que o módulo de tema for importado**, durante a renderização inicial da App. A abordagem mais limpa é importar e chamar `resolvedTheme()` no topo do componente `App()` ou via um efeito de montagem, garantindo que `createThemeState()` execute seu `createMemo` que sincroniza `data-theme`.

**Opção escolhida:** Importar `theme()` no `App.tsx` e chamá-la dentro da App, de modo que o SolidJS reaja e o `createRoot` seja disparado. A forma mais segura é adicionar `initState()` (ou `resolvedTheme()`) dentro de `createEffect` ou diretamente no corpo da `App` como um acesso de leitura que dispara o lazy init.

Melhor abordagem: adicionar `import { resolvedTheme } from "./lib/theme";` em `App.tsx` e chamar `resolvedTheme()` dentro de `onMount` — isso força o `createRoot` + `createThemeState()` a executar, disparando o `createMemo` que seta `data-theme` no documento.

## Risks

- **Flash de tema incorreto**: O script inline do index.html (Correção 1) é a defesa principal contra flash. Se houver alguma condição de corrida entre o script inline e o SolidJS, pode ocorrer um breve flash. O script inline é síncrono e executa antes de qualquer import, então não há risco aqui.
- **Regressão em temas legacy**: IDs `"dark"`/`"light"`/`"sepia"` ainda podem estar armazenados de sessões antigas. O mapeamento deve ser preservado.
- **Side-effect duplicado**: Após a correção 2, `createThemeState()` vai setar `data-theme` novamente com o mesmo valor que o script inline já definiu. Isso é benigno — SolidJS detectará que é o mesmo valor e não causará re-renderização desnecessária.

## Non-goals

- Não vamos refatorar o sistema de lazy init do `theme.ts` — apenas garantir que ele seja chamado na inicialização.
- Não vamos alterar como o `data-theme` é usado no CSS ou nos componentes.
- Não vamos modificar o `createThemeState()` internamente.

## Low-Level Design

### Arquivos a modificar

#### 1. `index.html` (linhas 8-17)

**Mudança:** Substituir o script inline IIFE.

**Antes:**
```js
var stored = localStorage.getItem("claudinio_theme");
if (stored === "dark" || stored === "light" || stored === "sepia") {
  theme = stored;
}
```

**Depois:**
```js
var stored = localStorage.getItem("claudinio_theme");
if (stored) {
  if (stored === "dark") theme = "claudinio";
  else if (stored === "light") theme = "claudinio-light";
  else if (stored === "sepia") theme = "claudinio-sepia";
  else theme = stored;
}
```

Isso aceita qualquer tema salvo e faz o mapeamento legacy.

#### 2. `src/App.tsx` — garantir initState() na montagem

**Mudança:** Adicionar import de `resolvedTheme` e chamá-la (descartando o valor) no `onMount` da App.

**Onde:** No topo, junto com outros imports:
```ts
import { resolvedTheme } from "./lib/theme";
```

**Onde:** Dentro do `onMount` existente (linha ~256), adicionar:
```ts
// Force theme state initialization — reads localStorage and applies data-theme
resolvedTheme();
```

Ou, alternativamente, adicionar um `createEffect` vazio que acessa `resolvedTheme()`.

**Melhor:** Adicionar diretamente no `onMount` existente que já carrega config, para não criar um novo efeito. O `onMount` já existe e executa uma vez na montagem — perfeito para isso.

```ts
onMount(async () => {
  // Initialize theme state — reads localStorage and applies correct data-theme
  resolvedTheme();
  
  try {
    const cfg = await getConfig();
    ...
  }
  ...
});
```

### Dados e schemas

Nenhuma alteração de schema ou tipo. `localStorage` key continua `"claudinio_theme"`. Os valores aceitos continuam sendo `ThemePreference` = `"system" | ThemeId`.

### Padrões existentes a reutilizar

- O `createThemeState()` já faz `createMemo` que sincroniza `document.documentElement.dataset.theme` (linha 267-269);
- O lazy init via `initState()` + `createRoot` (linha 289-295) é preservado — apenas garantimos que ele execute;
- O `ThemePicker` continua funcionando sem alterações.

### Integração

O fluxo completo pós-fix:
1. App inicia → script inline no `<head>` lê `localStorage`, aceita qualquer tema, seta `data-theme` imediatamente
2. SolidJS monta a App → `onMount` chama `resolvedTheme()` → `initState()` dispara → `createThemeState()` lê o mesmo valor do localStorage, seta `data-theme` novamente (mesmo valor, sem efeito colateral)
3. Tema correto desde o primeiro frame visual

## Tasks Summary

1. **golden-fix-index-html**: Ajustar script inline no index.html para aceitar qualquer tema salvo (não só valores legacy)
2. **golden-fix-app-init**: Garantir initState() do tema na montagem inicial da App
3. **golden-verify**: Verificar que o build não quebra e que o tema carrega corretamente


## Implementation Log — 2026-07-14 14:02
**Summary:** Corrige carregamento do tema salvo na inicialização — index.html aceita qualquer tema (não só legacy) + initState() chamado no onMount da App
**Changed files:** M index.html, M src/App.tsx, ?? docs/plans/2026-07-14_fix-theme-load-on-startup.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Findings & Decisions

### Root cause — two independent failures

**1. Script inline do index.html (antes do React):**
O script que executa síncrono no `<head>` para prevenir flash de tema incorreto só aceitava valores legacy `"dark"`/`"light"`/`"sepia"`. Temas como `"dracula"`, `"nord"`, `"tokyo-night"` eram ignorados e caíam no fallback `matchMedia` do sistema operacional. Isso significa que o `data-theme` inicial (antes do React montar) SEMPRE estava errado para qualquer tema não-legacy.

**2. Estado lazy do tema no SolidJS:**
O `createThemeState()` em `theme.ts` usa `createRoot` com lazy-init — só é executado quando alguém chama `theme()`, `preference()` ou `resolvedTheme()`. Nenhum código na renderização inicial da `App.tsx` chamava essas funções. O `ThemePicker` (Settings) era o único lugar que as chamava, por isso o tema só corrigia ao abrir Settings.

### Decisions

- **Fix no script inline:** Mudança mínima — de um `if` com 3 valores fixos para uma cadeia `if/else if/else` que aceita qualquer string, com mapeamento legacy. Preserva compatibilidade com usuários que ainda têm valores legacy no localStorage.
- **Fix no App.tsx:** Chamar `resolvedTheme()` dentro do `onMount` existente que já carrega config. Isso força o `initState()` a executar e o `createMemo` que sincroniza `data-theme` dispara. Como o script inline já setou o mesmo valor, não há flash nem re-renderização.

### Gotchas

- O `tsc --noEmit` tem erros pré-existentes em outros arquivos (`ContentViewerModal.test.tsx` com tipos legacy `"sepia"`/`"light"` e `subagentTimeline.test.ts` com propriedade `cost`). Nenhum relacionado às mudanças.
- A indentação do `index.html` usa 10 espaços (não 8 como é mais comum) — foi descoberto empiricamente com `cat -vet`.

**Task journal:**
- Ajustar script inline do index.html para aceitar qualquer tema salvo: diagnóstico: script inline só aceita valores legacy 'dark'/'light'/'sepia'; tema 'dracula' salvo no localStorage cai no fallback OS matchMedia; causa: o if restritivo no script inline do index.html; fix: substituído por lógica que aceita qualquer tema salvo + mapeamento legacy
- Garantir initState() do tema na montagem inicial da App: diagnóstico: initState() nunca é chamado na carga inicial da App; ThemePicker em Settings é o único lugar que chama preference()/setThemePreference(); solução: chamar resolvedTheme() no onMount da App; fix: import adicionado + chamada no onMount
- Verificar build, testes e fluxo completo: ✅ 645 testes passaram (35 arquivos); ✅ tsc mostra apenas erros pré-existentes (não relacionados às mudanças); ✅ fluxo index.html: 10/10 cenários passaram na simulação; ✅ legacy 'dark'/'light'/'sepia' corretamente mapeados para ids atuais; ✅ temas reais ('dracula','nord','tokyo-night') preservados diretamente
- Verificação de regressão visual — flash de tema: O script inline executado antes do React/SolidJS carregar é a defesa primária contra flash; Com a correção, ele agora aceita QUALQUER tema salvo — então data-theme já vem correto no primeiro frame; O initState() da App roda no onMount e aplica o mesmo valor novamente — não causa flash porque o data-theme já está correto; Sem risco de flash de tema incorreto
