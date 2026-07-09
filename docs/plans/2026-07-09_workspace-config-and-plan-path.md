# Plan: Workspace Config (.claudinio.json) + Custom Plan Save Path

## 1. Context / Problem Statement

**Problem 1 — Hardcoded plan directory:** Plans are always saved to `<workspace>/.claudinio/plans/`, which is gitignored. Users want to save plans to a configurable path (e.g. `docs/plans`) so they can be committed to the project repo for team visibility.

**Problem 2 — No team-shareable config:** All settings (brain_model, builder_model, yolo_mode, etc.) are stored only in the machine-local `~/.config/claudinio-code/config.json`. Teams sharing a project have no way to standardize these settings — every developer must configure them individually.

**Solution:** Introduce a `.claudinio.json` file at the workspace root as a team-shareable config layer. It holds: `plan_save_path`, `brain_model`, `builder_model`, `max_rounds`, `sub_max_rounds`, `yolo_mode`, `yolo_blacklist`. Add a `plan_save_path` field (text input + folder picker icon button) to the Settings UI.

### Decisions confirmed with user:

| Decision | Value |
|----------|-------|
| Config file location | Workspace root: `<workspace>/.claudinio.json` |
| Fields in `.claudinio.json` | `plan_save_path`, `brain_model`, `builder_model`, `max_rounds`, `sub_max_rounds`, `yolo_mode`, `yolo_blacklist` |
| Priority when both exist | Workspace config wins; local `~/.config` is fallback for missing fields |
| `plan_save_path` interpretation | Relative to workspace root (e.g. `docs/plans`) |
| Behavior when set | Replaces `.claudinio/plans` entirely (saves ONLY to custom path) |
| Behavior when empty/unset | Defaults to `.claudinio/plans` |
| Settings UI layout | All fields together with source badges ("Workspace" / "Local") |
| Workspace fields editability | Read-only in Settings, EXCEPT `plan_save_path` which IS editable |
| `plan_save_path` UI | Text input + folder picker icon button (opens native Tauri folder dialog). When empty: show "default" badge. When set: show reset (x) button to clear back to default |
| Directory creation | Auto-create on plan save if path doesn't exist; never overwrite, always coexist |

## 2. Goal (Definition of Done)

1. When a workspace has a `.claudinio.json`, its values merge into the agent config (workspace wins over local).
2. Settings modal shows all fields with source badges; workspace fields are read-only except `plan_save_path`.
3. `plan_save_path` has a text input + folder icon button to pick a folder via native dialog.
4. `write_plan` tool respects the custom `plan_save_path`; creates directories on demand.
5. When no custom path is set, behavior is unchanged (saves to `.claudinio/plans`).

## 3. Key Findings (Prova Real)

| Finding | Source |
|---------|--------|
| `plans_dir()` is hardcoded in `src-tauri/src/agent/tools/write_plan.rs:35-37` | file read |
| `AgentConfig` struct is in `src-tauri/src/agent/provider.rs:19-65` | file read |
| `set_config`/`get_config` commands in `src-tauri/src/commands/agent.rs:420-490` | file read |
| `ToolContext` struct is in `src-tauri/src/agent/tools/mod.rs:21-38` | file read |
| `ToolContext` constructed in `src-tauri/src/commands/agent.rs:153-163` | file read |
| System prompt references `.claudinio/plans` in `src-tauri/src/agent/session.rs:354,401` | grep |
| Tauri `dialog:default` permission already available in `capabilities/default.json` | file read |
| `pickFolder()` already exists in `src/lib/ipc.ts:19-22` using `@tauri-apps/plugin-dialog` | file read |
| No `.claudinio.json` exists anywhere in the codebase | code_search + grep |
| Icons: `folder-open` exists in `Icon.tsx`, `x` exists for reset | file read |
| Settings modal defined inline in `App.tsx` at ~line 309-475 | file read |
| `SetConfigArgs` in `ipc.ts:59-71` maps to Rust `SetConfigArgs` in `commands/agent.rs:420-434` | file read |
| `config_path()` in `provider.rs:108-112` resolves to `dirs::config_dir()/claudinio-code/config.json` | file read |
| Brain prompt constant references plan path in `session.rs:354` (`write_plan`) and `session.rs:401` (`EXPLORE...`) | grep |
| `run_workflow` creates `ToolContext` with `workspace_root` — this is where `.claudinio.json` would be loaded | `session.rs:969` / `commands/agent.rs:153` |

## 4. Authoritative Inputs

| Input | Value | Source |
|-------|-------|--------|
| `.claudinio.json` schema fields | `plan_save_path` (string\|null), `brain_model` (string), `builder_model` (string), `max_rounds` (number\|null), `sub_max_rounds` (number\|null), `yolo_mode` (boolean), `yolo_blacklist` (string[]) | per user |
| `plan_save_path` default | `null` → means `.claudinio/plans` | per user |
| Folder dialog mode | `{ directory: true, multiple: false }` | matches existing `pickFolder()` |
| Workspace label badge style | `text-[10px] rounded border px-1.5 py-px` with color differentiation | inferred from existing UI patterns |
| Read-only field treatment | `bg-surface-2` with `pointer-events-none` or `readonly` attribute | inferred |

## 5. Changes (Steps)

### Change 1 — Rust: Add `plan_save_path` to `AgentConfig` and `SetConfigArgs`

**Target:** `src-tauri/src/agent/provider.rs`
**Mutation:**
- Add `pub plan_save_path: Option<String>` field to `AgentConfig` struct (with `#[serde(default)]`)
- Add it to `Default` impl as `None`
- Add it to `save_config` (serialized automatically via serde)
- Add it to `load_config` (deserialized automatically; if missing from old config, `None`)

**Why:** Core data model must carry the optional custom plan path.

---

### Change 2 — Rust: Add `plan_save_path` to `SetConfigArgs` and `set_config`/`get_config`

**Target:** `src-tauri/src/commands/agent.rs`
**Mutation:**
- Add `pub plan_save_path: Option<String>` to `SetConfigArgs`
- In `set_config`: if `args.plan_save_path` is `Some`, set `cfg.plan_save_path = Some(args.plan_save_path)`. If the option is `None`, don't modify.
- In `get_config`: include `"planSavePath": cfg.plan_save_path` in the JSON response

**Why:** The frontend needs to read/write this field via IPC. Note: `save_plan_path` is the ONLY workspace field editable in Settings — frontend sends it to `set_config` which saves to LOCAL config, not `.claudinio.json` (the workspace file is read-only in Settings).

---

### Change 3 — Rust: Read `.claudinio.json` and merge into config

**Target:** `src-tauri/src/agent/provider.rs` (new function)
**Mutation:**
- Add `pub fn read_workspace_config(workspace_root: &str) -> Option<Value>` that reads `<workspace_root>/.claudinio.json` and returns parsed JSON or `None` if file doesn't exist or is invalid.
- Add `pub fn merge_workspace_config(cfg: &mut AgentConfig, ws_config: &Value)` that overlays workspace values onto the config for these fields ONLY:
  - `plan_save_path` (as string or null)
  - `brain_model` (as string)
  - `builder_model` (as string)
  - `max_rounds` (as u64 or null)
  - `sub_max_rounds` (as u64 or null)
  - `yolo_mode` (as bool)
  - `yolo_blacklist` (as array of strings)

**Why:** This is the merge layer. When a workspace is opened, the config used by the agent must reflect workspace overrides. The config loaded at startup (`load_config()`) is the base; workspace config is layered on top per-workspace.

**Constraints:**
- IMPORTANT: `base_url`, `api_key`, `account_login`, `account_tier`, `max_golden_cycles`, `max_golden_stalls`, `services_url` must NOT be read from `.claudinio.json` — stay machine-local only.
- If `.claudinio.json` is missing or malformed, silently skip (no error modal).

---

### Change 4 — Rust: Expose workspace config to frontend via `get_config`

**Target:** `src-tauri/src/commands/agent.rs` — `get_config` handler
**Mutation:**
- Add a new field `"workspaceConfig"` to the `get_config` JSON response. Value is the raw parsed `.claudinio.json` object (or `null` if no file). This lets the frontend determine which fields have workspace overrides and show source badges.

**Why:** The frontend needs to know which values came from the workspace config to display source badges and enforce read-only state.

**Constraints:** The response must still include the merged (effective) values. Add `workspaceConfig` as an additional field, not a replacement.

---

### Change 5 — Rust: Add `plan_save_path` to `ToolContext`

**Target:** `src-tauri/src/agent/tools/mod.rs`
**Mutation:**
- Add `pub plan_save_path: Option<String>` to `ToolContext` struct
- Update all `ToolContext` construction sites (in `commands/agent.rs` lines 153 and 623) to include the value: `plan_save_path: config.plan_save_path.clone()`

**Why:** `write_plan` needs access to the custom path. Currently `write_plan` only receives `workspace_root` from the context. By adding `plan_save_path` to `ToolContext`, the tool can resolve the effective plan directory.

---

### Change 6 — Rust: Modify `write_plan` to use custom path

**Target:** `src-tauri/src/agent/tools/write_plan.rs`
**Mutation:**
- Modify `plans_dir()` → accept `workspace_root: &str` and `plan_save_path: Option<&str>`:
  - If `plan_save_path` is `Some(path)`, use `PathBuf::from(workspace_root).join(path)`
  - Otherwise, use the current default: `PathBuf::from(workspace_root).join(".claudinio").join("plans")`
- In `execute()`:
  - Call the new `plans_dir(root, ctx.plan_save_path.as_deref())`
  - Use `std::fs::create_dir_all(&dir)` (already does this — verify it creates intermediate dirs for custom paths like `docs/plans`)
  - Update the success message to reflect the actual path used

**Why:** This is the core feature — the plan directory is no longer hardcoded.

**Constraints:** `create_dir_all` already creates all parent directories. The function creates, never overwrites files (each write creates a timestamped filename). Coexistence is guaranteed.

---

### Change 7 — Rust: Update system prompt to reference configurable path

**Target:** `src-tauri/src/agent/session.rs` — `system_prompt()` function and `BRAIN_PROMPT` constant
**Mutation:**
- The `system_prompt()` function already receives `workspace_root: Option<&str>`. Add a parameter `plan_save_path: Option<&str>`.
- In the prompt text, replace the hardcoded `.claudinio/plans/*.md` references with the effective path:
  - If `plan_save_path` is `Some(p)`, use `{workspace_root}/{p}/*.md`
  - Otherwise, use the current `.claudinio/plans/*.md`

**Why:** The agent's system prompt tells it where plans are saved. If the user configures a custom path, the prompt must match so the agent knows the correct location.

---

### Change 8 — Rust: Ensure `.claudinio.json` is loaded at session start

**Target:** `src-tauri/src/commands/agent.rs` — `send_message` handler (around line 98)
**Mutation:**
- After creating `workspace_root`, call `provider::read_workspace_config(root)` to load `.claudinio.json`
- If found, merge into the `config` used for the session
- Pass any `plan_save_path` from the (now merged) config into `ToolContext`

**Why:** Currently config is loaded once at app startup from the local file. With workspace config, we must re-merge when each workspace session starts.

---

### Change 9 — TypeScript: Update IPC interfaces

**Target:** `src/lib/ipc.ts`
**Mutation:**
- Add `planSavePath?: string | null` to `AgentConfig` interface
- Add `planSavePath?: string | null` to `SetConfigArgs` interface
- Add `workspaceConfig?: Record<string, unknown> | null` to `AgentConfig` interface (the raw unmasked `.claudinio.json` content)

**Why:** Type safety for the frontend.

---

### Change 10 — SolidJS: Add `plan_save_path` signal and Settings UI

**Target:** `src/App.tsx`
**Mutation:**
- Add signal: `const [configPlanSavePath, setConfigPlanSavePath] = createSignal<string>("");`
- Add signal: `const [workspaceConfigFields, setWorkspaceConfigFields] = createSignal<Set<string>>(new Set());` (tracks which fields have workspace overrides)
- In `openConfig()`:
  - Populate `configPlanSavePath` from `cfg.planSavePath ?? ""`
  - Populate `workspaceConfigFields` from `cfg.workspaceConfig` keys
- In `saveConfig()`:
  - Include `planSavePath: configPlanSavePath() || undefined`
- Add UI section **before the Brain model selector** (logical placement — it's a general setting):

```tsx
{/* Plan save path */}
<label class="mb-1 block text-xs text-ink-muted">{t("app.config.planSavePath")}</label>
<div class="mb-1 flex gap-1">
  <div class="relative flex-1">
    <input
      type="text"
      value={configPlanSavePath()}
      onInput={(e) => setConfigPlanSavePath(e.currentTarget.value)}
      placeholder=".claudinio/plans"
      readonly={workspaceConfigFields().has("plan_save_path")}
      class="w-full rounded-md border border-border-subtle bg-surface-0 p-2 pr-8 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
      classList={{ "bg-surface-2 text-ink-muted pointer-events-none": workspaceConfigFields().has("plan_save_path") && !isSavePathEditable() }}
    />
    <Show when={configPlanSavePath() && !workspaceConfigFields().has("plan_save_path")}>
      <button
        onClick={() => setConfigPlanSavePath("")}
        class="absolute right-2 top-1/2 -translate-y-1/2 text-ink-faint hover:text-ink"
        title={t("app.config.resetToDefault")}
      >
        <Icon name="x" class="h-3.5 w-3.5" />
      </button>
    </Show>
  </div>
  <button
    onClick={pickPlanPath}
    class="flex h-9 w-9 shrink-0 items-center justify-center rounded-md border border-border-subtle text-ink-muted hover:bg-surface-2 hover:text-ink"
    title={t("app.config.browseFolder")}
  >
    <Icon name="folder-open" class="h-4 w-4" />
  </button>
</div>
<div class="mb-4 flex items-center gap-2">
  <Show when={workspaceConfigFields().has("plan_save_path")}>
    <span class="rounded border border-accent/40 bg-accent/10 px-1.5 py-px text-[10px] font-medium text-accent">{t("app.config.sourceWorkspace")}</span>
  </Show>
  <Show when={!configPlanSavePath()}>
    <span class="rounded border border-border-subtle bg-surface-2 px-1.5 py-px text-[10px] text-ink-faint">{t("app.config.default")}</span>
  </Show>
  <p class="text-[11px] text-ink-faint">{t("app.config.planSavePathHint")}</p>
</div>
```

- Add `pickPlanPath` async function:
```tsx
const pickPlanPath = async () => {
  const folder = await pickFolder();
  if (!folder) return;
  const ws = activeWorkspace();
  if (!ws) { setConfigPlanSavePath(folder); return; }
  // Convert absolute path to relative (relative to workspace root)
  if (folder.startsWith(ws)) {
    let rel = folder.slice(ws.length);
    if (rel.startsWith("/") || rel.startsWith("\\")) rel = rel.slice(1);
    setConfigPlanSavePath(rel || ".");
  } else {
    // Selected folder is outside workspace — use absolute as fallback
    setConfigPlanSavePath(folder);
  }
};
```

- Add source badges to ALL workspace-configurable fields (brain_model, builder_model, max_rounds, sub_max_rounds, yolo_mode, yolo_blacklist):
  - After each field's label, show `{workspaceConfigFields().has("brain_model") ? <span ...sourceWorkspace> : <span ...sourceLocal>}`.
  - When field has workspace override, make it read-only: add `disabled` attribute or `pointer-events-none bg-surface-2` styling.

**Why:** The UI must clearly show which fields come from the team config vs local config, and enforce read-only for workspace fields (except plan_save_path).

**Constraint:** `plan_save_path` is the EXCEPTION — it is ALWAYS editable in Settings (the user explicitly chose this). Even when it comes from workspace config, the user can still type in the field. The save goes to `.claudinio.json` via a separate IPC or via `set_config` which will write to the workspace file in this specific case.

Wait — per the interview, `.claudinio.json` is "read-only in Settings" generally, but `plan_save_path` is the exception. This means when the user edits this field and saves, the value needs to go to `.claudinio.json` (not local config). This requires special handling.

**Revised approach for plan_save_path editability:**
- When `plan_save_path` is edited in Settings and saved, call a new IPC `set_workspace_config({ plan_save_path: value })` that writes directly to `.claudinio.json`
- OR: `set_config` with `planSavePath` writes to `.claudinio.json` instead of local config
- Simplest: Add a separate IPC command `set_workspace_config_value(key, value)` that reads `.claudinio.json`, updates one key, and writes back

### Change 11 — Rust: Add IPC to write `.claudinio.json` fields

**Target:** `src-tauri/src/commands/agent.rs`
**Mutation:**
- Add new command `set_workspace_config` that:
  - Takes `workspace_root: String` and `plan_save_path: Option<String>`
  - Reads existing `.claudinio.json` (or creates fresh object)
  - Updates `plan_save_path` key
  - Writes back with `serde_json::to_string_pretty`
  - Returns `Ok(())`
- Register command in `lib.rs`

**Why:** The `plan_save_path` field in Settings edits the workspace config directly. Other workspace fields are read-only so they don't need a write path.

### Change 12 — Frontend: Wire `saveConfig` for plan_save_path to workspace config IPC

**Target:** `src/App.tsx`
**Mutation:**
- In `saveConfig()`, after the existing `setConfig` call, if `configPlanSavePath` changed from its original value, also call `setWorkspaceConfig(activeWorkspace(), configPlanSavePath() || null)`
- In `openConfig()`, store the original `planSavePath` value for comparison

### Change 13 — i18n: Add translation keys

**Target:** `src/lib/locales/en-US.ts` and `src/lib/locales/pt-BR.ts`
**Mutation:** Add these keys:

| Key | en-US | pt-BR |
|-----|-------|-------|
| `app.config.planSavePath` | "Plan save path" | "Caminho para salvar planos" |
| `app.config.planSavePathHint` | "Relative to workspace root. Leave empty to use default (.claudinio/plans)." | "Relativo à raiz do workspace. Deixe vazio para usar o padrão (.claudinio/plans)." |
| `app.config.browseFolder` | "Browse folder" | "Procurar pasta" |
| `app.config.resetToDefault` | "Reset to default" | "Voltar ao padrão" |
| `app.config.default` | "default" | "padrão" |
| `app.config.sourceWorkspace` | "Workspace" | "Workspace" |
| `app.config.sourceLocal` | "Local" | "Local" |

## 6. Verification Plan

### Rust compilation
```bash
cd src-tauri && cargo check 2>&1
```
Expected: no errors.

### TypeScript compilation
```bash
npx tsc --noEmit 2>&1
```
Expected: no errors.

### Unit: write_plan with custom path
Create a temp workspace, write a `.claudinio.json` with `plan_save_path: "my-plans"`, run `write_plan` tool, verify file lands in `<workspace>/my-plans/YYYY-MM-DD_<slug>.md`.

### Unit: write_plan default behavior
Without `.claudinio.json`, verify plan lands in `.claudinio/plans/` as before. Regresion check.

### Unit: auto-create directories
Set `plan_save_path` to `deeply/nested/plans`, run `write_plan`, verify directories are created and plan is written.

### Unit: `.claudinio.json` merge
Write a `.claudinio.json` with `{"brain_model": "custom-model"}`, open workspace, verify `get_config` returns `brainModel: "custom-model"` and `workspaceConfig.brain_model: "custom-model"`.

### Unit: config priority
Set local `brain_model = "local-model"` in `~/.config`, and workspace `brain_model = "workspace-model"` in `.claudinio.json`. Verify effective config uses `"workspace-model"`. Remove `brain_model` from `.claudinio.json`, verify it falls back to `"local-model"`.

### UI: Settings modal renders
Open Settings with a workspace open. Verify:
- `plan_save_path` field shows text input + folder icon button
- Empty state shows "default" badge
- When workspace has `.claudinio.json` fields, source badges appear
- Workspace fields (brain_model, etc.) are read-only

### UI: Folder picker interaction
Click folder icon button, select a folder. Verify:
- Text input populates with relative path (if inside workspace) or absolute path
- Reset (x) button appears

### UI: Reset to default
Click (x) reset button. Verify:
- Text input clears
- "default" badge reappears

### E2E: Full workflow
1. Create `.claudinio.json` with `plan_save_path: "team-plans"`
2. Open workspace, open Settings, verify path shown
3. Run a Brain session, trigger write_plan, verify file lands in `team-plans/`
4. Edit `plan_save_path` in Settings, save
5. Verify `.claudinio.json` updated on disk
6. Run another Brain session, trigger write_plan, verify file lands in new path

## 7. Risks

| Risk | Mitigation |
|------|-----------|
| `.claudinio.json` parse error crashes the app | Use `serde_json::from_str` with match; treat invalid JSON as "no workspace config" (silent fallback) |
| Custom path is absolute | The `plans_dir()` function joins with workspace root. If `plan_save_path` is absolute, `PathBuf::join` replaces. We should detect absolute paths and use them directly. |
| Folder dialog returns absolute path | `pickPlanPath()` in frontend converts to relative when inside workspace; otherwise uses absolute fallback |
| Race condition: `.claudinio.json` edited externally | Write uses atomic `std::fs::write` (overwrites full file). Read on each session start. |
| Old config.json without `plan_save_path` | Serde `#[serde(default)]` handles this — deserializes to `None` |

## 8. Tasks Summary

1. Add `plan_save_path` to `AgentConfig` struct
2. Add `plan_save_path` to `SetConfigArgs` and `set_config`/`get_config` commands
3. Implement `.claudinio.json` read + merge in `provider.rs`
4. Expose `workspaceConfig` in `get_config` response
5. Add `plan_save_path` to `ToolContext`
6. Modify `write_plan` to use custom path from `ToolContext`
7. Update system prompt to reference configurable path
8. Load `.claudinio.json` at session start in `send_message`
9. Add `set_workspace_config` IPC command
10. Update TypeScript IPC interfaces
11. Add Settings UI for `plan_save_path` with folder picker, badges, read-only states
12. Wire `saveConfig` to write `plan_save_path` to workspace config
13. Add i18n keys (en-US + pt-BR)
14. Verification: cargo check, tsc, manual E2E
