# Plan: Botão de Planos no Header + Botão "Ler Plano" no Brain

## Context / Problem Statement

O usuário quer dois novos botões:
1. **Botão "Planos" ao lado de "History"** no header do ChatPanel — abre um dropdown com a lista de planos (arquivos `.md` no diretório de planos), ordenados por data de criação, permitindo clicar para abrir e ler o plano.
2. **Botão "Ler Plano" ao lado de "Continuar com Builder"** — abre o plano mais recente no ContentViewerModal para leitura antes de aprovar.

Atualmente:
- O backend tem `write_plan` (escreve planos) e `check_plan_exists` (booleano), mas **não** tem `list_plans` nem `read_plan`.
- O frontend já tem `ContentViewerModal` que renderiza arquivos `.md` no Monaco Editor com syntax highlighting.
- O frontend já tem `viewerFile` signal + `setViewerFile` no ChatPanel para abrir o modal.
- O dropdown de History (sessions) serve como molde para o dropdown de planos.

## Goal (Definition of Done)

1. Backend: comando `list_plans` retorna lista de planos (nome, caminho, data de modificação) ordenados do mais recente para o mais antigo.
2. Frontend: botão "Planos" ao lado de "History" no header do ChatPanel com dropdown espelhando o dropdown de sessions.
3. Frontend: botão "Ler Plano" ao lado de "Continuar com Builder" que abre o plano mais recente no ContentViewerModal.
4. i18n: chaves em `en-US.ts` e `pt-BR.ts` para os novos textos.

## Key Findings (Prova Real)

| Finding | Source | Traceability |
|---------|--------|-------------|
| `plans_dir()` é público em `write_plan.rs:40` e reutilizável | `src-tauri/src/agent/tools/write_plan.rs:40` | Pode ser importado no comando `list_plans` |
| Comandos Tauri são registrados em `lib.rs:15-61` | `src-tauri/src/lib.rs` | `invoke_handler` com `.invoke_handler(tauri::generate_handler![...])` |
| Comandos de agent ficam em `commands/agent.rs` | `src-tauri/src/commands/agent.rs:813` | `check_plan_exists` está lá |
| `readFile` IPC já existe e lê qualquer arquivo | `src/lib/ipc.ts:15-17` | Não precisa de novo comando `read_plan` |
| `viewerFile` signal no ChatPanel já abre ContentViewerModal | `src/components/ChatPanel.tsx:510,2352-2359` | Padrão: `setViewerFile({ type: "text", path, title })` |
| Sessions dropdown é o molde exato | `src/components/ChatPanel.tsx:1732-1745` | Classes CSS, estrutura, posicionamento |
| History button está na linha ~1725 | `src/components/ChatPanel.tsx:1725-1731` | Botão `clock` + texto "History" |
| "Continue with Builder" está na linha ~1839 | `src/components/ChatPanel.tsx:1839-1850` | Condição `mode() === "brain" && modeOrigin() === "human" && status() === "done" && hasPlanBeenWritten()` |
| `latest_plan_file()` já existe em `finalize_plan.rs:165` | `src-tauri/src/agent/tools/finalize_plan.rs:165` | Retorna o `.md` mais recente por `mtime` |
| Planos são salvos como `YYYY-MM-DD_<slug>.md` | `src-tauri/src/agent/tools/write_plan.rs:65` | Convenção de nomenclatura |

## Authoritative Inputs

| Input | Value | Source |
|-------|-------|--------|
| Plan directory resolver | `plans_dir(workspace_root, plan_save_path)` | `write_plan.rs:40` — reusável |
| Dropdown CSS classes | `absolute right-4 top-9 z-20 max-h-80 w-80 overflow-y-auto rounded-lg border border-border-subtle bg-surface-1 py-1 shadow-lg` | `ChatPanel.tsx:1734` |
| ContentViewerModal props | `contentType: "text"`, `filePath`, `title`, `workspace`, `onClose` | `ContentViewerModal.tsx:12-18` |
| Ícone para planos | `notebook` (lucide:notebook-pen) ou `document` | Preciso verificar Icon.tsx para ícones disponíveis |

## Changes (Steps)

### Step 1: Backend — `list_plans` Tauri command

**Target:** `src-tauri/src/commands/agent.rs` (novo comando)
**Target:** `src-tauri/src/lib.rs` (registro)

**Mutation:**
- Adicionar comando `list_plans(workspace: String, state: State<AppState>) -> Result<Vec<PlanEntry>, String>`
- `PlanEntry` struct: `{ name: String, path: String, modified_at: u64 }` (UNIX timestamp em segundos)
- Usar `plans_dir()` do `write_plan.rs` para resolver o diretório
- Resolver `plan_save_path` com a mesma cascata do `check_plan_exists` (workspace config → global config)
- Listar `*.md`, extrair metadados, ordenar por `modified_at` descendente
- Registrar em `lib.rs`

**Why:** Frontend precisa listar planos para o dropdown. O comando `check_plan_exists` só retorna booleano.

**Constraints:** Reutilizar `plans_dir()` público. Seguir o padrão de cascata de config igual ao `check_plan_exists`.

### Step 2: Frontend IPC — `listPlans` function + type

**Target:** `src/lib/ipc.ts`

**Mutation:**
- Adicionar interface `PlanEntry { name: string; path: string; modifiedAt: number }`
- Adicionar `export function listPlans(workspace: string): Promise<PlanEntry[]>`

**Why:** Ponte entre backend e UI.

### Step 3: Frontend — Plans state + dropdown no ChatPanel

**Target:** `src/components/ChatPanel.tsx`

**Mutation:**
- Adicionar signals: `showPlans`, `plans` (similar a `showSessions`, `sessions`)
- Adicionar função `togglePlans` (espelha `toggleSessions`, mas chama `listPlans`)
- Adicionar função `openPlan(path: string, title: string)` que chama `setViewerFile({ type: "text", path, title })`
- Adicionar botão "Planos" ao lado do botão "History" no header (linha ~1731):
  ```tsx
  <button onClick={togglePlans} class="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-ink-muted hover:bg-surface-2" title={t("chat.header.plans")}>
    <Icon name="notebook" class="h-3.5 w-3.5" />
    {t("chat.header.plans")}
  </button>
  ```
- Adicionar dropdown (espelha o dropdown de sessions, linhas 1732-1745):
  ```tsx
  <Show when={showPlans()}>
    <div ref={plansRef} class="absolute right-4 top-9 z-20 max-h-80 w-80 overflow-y-auto rounded-lg border border-border-subtle bg-surface-1 py-1 shadow-lg">
      <Show when={plans().length > 0} fallback={<div class="px-3 py-2 text-[12px] text-ink-faint">{t("chat.header.noPlans")}</div>}>
        <For each={plans()}>
          {(p) => (
            <button onClick={() => { setShowPlans(false); openPlan(p.path, p.name); }} class="flex w-full flex-col gap-0.5 px-3 py-1.5 text-left hover:bg-surface-2">
              <span class="truncate text-[12px] text-ink">{p.name}</span>
              <span class="font-mono text-[10px] text-ink-faint">{new Date(p.modifiedAt * 1000).toLocaleString()}</span>
            </button>
          )}
        </For>
      </Show>
    </div>
  </Show>
  ```
- Adicionar `plansRef` para click-outside (se sessions já tem, seguir mesmo padrão)
- Adicionar cleanup no click-outside handler existente

**Why:** UI para o usuário acessar planos.

**Constraints:** Seguir exatamente o padrão do dropdown de sessions (classes CSS, estrutura, posicionamento, z-index).

### Step 4: Frontend — Botão "Ler Plano" ao lado de "Continuar com Builder"

**Target:** `src/components/ChatPanel.tsx`, linha ~1839

**Mutation:**
- Adicionar botão "Ler Plano" dentro do mesmo `<Show>` do "Continuar com Builder" (ou em um container flex com gap), ANTES do botão "Continuar com Builder":
  ```tsx
  <div class="mb-6 flex justify-center gap-3">
    <button
      onClick={readLatestPlan}
      class="inline-flex items-center gap-2 rounded-full border border-border-subtle bg-surface-1 px-5 py-2.5 text-sm font-semibold text-ink-muted transition-all hover:bg-surface-2 hover:text-ink active:scale-[0.98]"
    >
      <Icon name="notebook" class="h-4 w-4" />
      {t("mode.readPlan")}
    </button>
    <button onClick={continueWithBuilder} ...>
      {/* existing Continue with Builder button */}
    </button>
  </div>
  ```
- Criar função `readLatestPlan` que:
  1. Chama `listPlans(props.workspace)`
  2. Pega o primeiro item (mais recente)
  3. Chama `setViewerFile({ type: "text", path: plan.path, title: plan.name })`

**Why:** Usuário quer ler o plano antes de decidir se aprova/continua.

**Constraints:** Botão "Ler Plano" deve ter estilo secundário (outline/border) para não competir visualmente com o CTA principal "Continuar com Builder". Aparece nas mesmas condições do "Continuar com Builder".

### Step 5: i18n — chaves de tradução

**Target:** `src/lib/locales/en-US.ts`
**Target:** `src/lib/locales/pt-BR.ts`

**Mutation:**
- Adicionar após `chat.header.history` / `chat.header.turn`:
  - `"chat.header.plans": "Plans"` / `"Planos"`
  - `"chat.header.noPlans": "No plans found."` / `"Nenhum plano encontrado."`
- Adicionar após `mode.continueWithBuilder`:
  - `"mode.readPlan": "Read Plan"` / `"Ler Plano"`

**Why:** Suporte a EN e PT-BR.

## Verification Plan

1. **Backend `list_plans`:**
   - Rodar `cargo test` no backend para garantir que não quebrou nada
   - Verificar que o comando está registrado em `lib.rs`

2. **Frontend build:**
   - Rodar `pnpm build` (ou `pnpm tsc --noEmit`) para verificar TypeScript
   - Garantir que não há erros de tipo

3. **Funcional (manual):**
   - Criar um plano via Brain mode (usando `write_plan`)
   - Verificar que o botão "Planos" aparece ao lado de "History"
   - Clicar em "Planos" → dropdown mostra o plano criado
   - Clicar no plano → ContentViewerModal abre com o conteúdo
   - No Brain mode após `write_plan` → botão "Ler Plano" aparece ao lado de "Continuar com Builder"
   - Clicar "Ler Plano" → abre o plano mais recente

4. **Edge cases:**
   - Dropdown vazio (sem planos) → mostra "Nenhum plano encontrado."
   - Planos com nomes longos → truncados com `truncate`
   - Múltiplos planos → ordenados do mais recente ao mais antigo
   - Clicar fora do dropdown → fecha (click-outside)

## Risks / Edge Cases

- **Ícone `notebook`:** Verificar se existe no `Icon.tsx`. Se não existir, usar `document` ou adicionar o path SVG.
- **Plan save path customizado:** O `list_plans` precisa resolver o `plan_save_path` com a mesma lógica do `check_plan_exists` (workspace config → global config → default).
- **Click-outside:** O dropdown de sessions já tem `sessionsRef` com click-outside handler. Preciso seguir o mesmo padrão com `plansRef`.

## Tasks Summary

1. Backend: `list_plans` command + registro
2. Frontend IPC: `listPlans` + `PlanEntry` type
3. Frontend: Plans dropdown button + dropdown no ChatPanel header
4. Frontend: "Ler Plano" button ao lado de "Continuar com Builder"
5. i18n: chaves EN + PT-BR
6. Verificar ícone `notebook` no Icon.tsx


## Implementation Log — 2026-07-11 16:10
**Summary:** Add 'Planos' dropdown button next to History, and 'Ler Plano' button next to Continue with Builder
**Changed files:** M src-tauri/src/commands/agent.rs, M src-tauri/src/lib.rs, M src/components/ChatPanel.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-11_plans-button-and-read-plan.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Key Decisions & Implementation Notes

### Backend
- The `list_plans` command follows the exact same config cascade pattern as `check_plan_exists` (global config → workspace config → default `.claudinio/plans`), reusing `plans_dir()` from `write_plan.rs`.
- PlanEntry returns `modifiedAt` as UNIX epoch seconds (u64), sorted descending — newest first in the dropdown.
- Using `#[serde(rename_all = "camelCase")]` so the frontend receives camelCase (modifiedAt).

### Frontend — Plans Dropdown
- Mirrors the sessions dropdown exactly: same absolute positioning (`right-4 top-9`), same dimensions (`w-80 max-h-80`), same border/shadow classes, same click-outside handler pattern.
- Uses `notebook-pen` icon (already existed in Icon.tsx).
- `openPlan` function uses the existing `setViewerFile` signal pattern to open ContentViewerModal with `type: "text"`.

### Frontend — "Ler Plano" Button
- Appears beside "Continuar com Builder" inside the same conditional `<Show>` block.
- Uses a secondary/outline style (`border border-border-subtle bg-surface-1`) to visually complement the primary accent-colored CTA.
- `readLatestPlan` calls `listPlans` and opens the first result (most recent) — no need for a separate "read latest plan" IPC.
- Both buttons wrapped in `flex justify-center gap-3` for proper spacing.

### i18n
- Keys: `chat.header.plans`, `chat.header.noPlans`, `mode.readPlan` in both en-US and pt-BR.
- No trailing commas issue — used consistent formatting with the existing files.

### Verification
- `pnpm tsc --noEmit` — no new errors in modified files (all existing errors are pre-existing test/Monaco issues).
- `cargo check` — compiled cleanly.

### Gotchas
- The `ChatPanel.tsx` file is ~3300+ lines, making precise line-number edits tricky. Used pattern matching (find existing code blocks) rather than line numbers.
- Both dropdowns share the same positioning (`absolute right-4 top-9`). If both are open simultaneously they'd overlap — but the click-outside handler closes one when clicking the other's button, which is acceptable UX.

**Task journal:**
- Backend: list_plans Tauri command: Added PlanEntry struct with #[serde(rename_all='camelCase')]; Added list_plans command with plan_save_path cascade matching check_plan_exists; Registered in lib.rs
- Frontend IPC: listPlans + PlanEntry type: Added PlanEntry interface; Added listPlans function invoking list_plans Tauri command
- Frontend: Plans dropdown button + dropdown: Added listPlans import + PlanEntry type import; Added plansRef ref variable; Added showPlans/plans signals; Added togglePlans + openPlan functions; Added plans click-outside handler; Added 'Planos' button after History button (icon: notebook-pen); Added plans dropdown mirroring sessions dropdown pattern
- Frontend: 'Ler Plano' button next to 'Continue with Builder': Added readLatestPlan function (calls listPlans, takes first, opens viewer); Changed Continue with Builder container to flex with gap-3; Added 'Ler Plano' secondary/outline button before the primary CTA; Uses same <Show> condition as Continue with Builder
- i18n: EN + PT-BR translation keys: Added keys: chat.header.plans, chat.header.noPlans, mode.readPlan in both en-US and pt-BR
