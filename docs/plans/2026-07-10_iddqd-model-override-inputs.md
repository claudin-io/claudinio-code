# Plan: Easter egg "iddqd" — Brain/Builder models viram inputs customizáveis

## Context
Quando o usuário ativa o Easter egg (digita "iddqd" no modal de settings), os campos de override de URL e API Key aparecem. Como agora o usuário quer usar um endpoint arbitrário (não o claudin.io), os modelos disponíveis via API (dropdown `select`) não são mais relevantes — ele precisa digitar o nome do modelo manualmente.

## Goal
Quando `easterEggActive()` está ativo, os dois `<select>` de Brain Model e Builder Model viram `<input type="text">`, permitindo que o usuário digite qualquer nome de modelo.

## Changes

### 1. App.tsx — Brain Model selector (~linha 595)
**Mutation:** Envolver o `<select>` num `<Show when={easterEggActive()} fallback={<select>... </select>}>`, com um `<input type="text">` como conteúdo ativo.

**Antes:**
```tsx
<select
  value={configBrainModel()}
  onChange={(e) => setConfigBrainModel(e.currentTarget.value)}
  disabled={workspaceConfigFields().has("brain_model")}
  class="mb-4 w-full ..."
>
  <For each={availableModels()}>
    {(m) => <option value={m} selected={configBrainModel() === m}>{m}</option>}
  </For>
</select>
```

**Depois:**
```tsx
<Show
  when={easterEggActive()}
  fallback={
    <select ...> ... </select>
  }
>
  <input
    type="text"
    value={configBrainModel()}
    onInput={(e) => setConfigBrainModel(e.currentTarget.value)}
    placeholder="claude-sonnet-4-20250514"
    class="mb-4 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
  />
</Show>
```

### 2. App.tsx — Builder Model selector (~linha 620)
**Mutation:** Mesmo padrão do Brain Model.

## Verification
1. `pnpm exec vite build` — sem erros
2. Abrir settings, digitar "iddqd" → Brain e Builder viram inputs de texto
3. Digitar no input (ex: `claude-sonnet-4-20250514`), salvar, reabrir settings → valor ainda presente
