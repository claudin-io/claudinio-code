# Settings Window with Sidebar

## Context

The Settings modal in `App.tsx` has grown to ~12 sections with ~16 configurable fields, all in a single 680px-wide scrollable column. The user wants a wider layout with a sidebar grouping settings into 6 categories, and to open settings in a separate OS window instead of a modal overlay.

Confirmed by user: Window 900x650px centered, sidebar 200px fixed, 6 categories (General settings / Account user / Models brain / Agent Behavior spawn-swarm / MCP terminal / Advanced goal), always separate window replacing the modal completely, one category active at a time.

## Solution Design

The gear icon in the header creates a Tauri WebviewWindow (900x650px, centered) that loads the same SPA with a `?window=settings` query param. The App component detects this at entry and renders `<SettingsWindow />` instead of the full app layout. The settings window loads config independently via the same `getConfig()` + `listModels()` IPC, manages its own SolidJS signals, and on Save calls `setConfig()` then emits a `settings-changed` event to the main window via Tauri events. The main window listens for this event and reloads its config signals. Cancel or OS-chrome-close simply closes the window without saving.

Edge cases: duplicate window prevention (check `getByLabel` first), API key validation failure keeps window open with error, invalid MCP JSON keeps window open, workspace context passed via URL param for workspace-scoped config merging.

## Non-Goals

No separate Vite entry point, no keyboard shortcut, no window position persistence, no resizable sidebar, no search/filter.

## Risks

- Tauri permissions may need individual allow-list entries beyond `core:window:default` — test immediately
- Query param `/?window=settings` may not survive Vite/Tauri production build — fallback to hash route if needed
- Two SolidJS instances share no signals — communicate only via events and Rust backend; this is intentional and safer

## Low-Level Design

This section details exactly which files to create/modify, the component architecture, data flow between windows, IPC patterns reused from the existing codebase, and concrete symbols and APIs to use.

### Files to create

- `src/components/SettingsWindow.tsx` — self-contained settings component with sidebar and 6 category content panels

### Files to modify

- `src/components/Icon.tsx` — add `user` pixel-art icon to PATHS
- `src/lib/locales/en-US.ts` and `src/lib/locales/pt-BR.ts` — add `app.config.categories.*` keys
- `src-tauri/capabilities/default.json` — add `core:window:default` and `core:webview:default`
- `src/App.tsx` — detect `?window=settings` at entry, replace gear button modal logic with `openSettingsWindow()`, listen for `settings-changed` event, remove old modal JSX

### SettingsWindow component internals

SettingsWindow is a function component with no props. It reads `workspace` from URL search params. Internal signals replicate the 16 config signals from App.tsx: `configBrainModel`, `configBuilderModel`, `configMaxRounds`, `configSubMaxRounds`, `configMaxGoldenCycles`, `configMaxGoldenStalls`, `configMaxParallelAgents`, `configYoloMode`, `configYoloBlacklist`, `configPlanSavePath`, `configOverrideBaseUrl`, `configOverrideApiKey`, `configMcpJson`, `accountLogin`, `accountTier`, `configApiKey`, `availableModels`, plus `activeCategory` (default `'general'`).

Lifecycle: onMount calls `getConfig(workspaceParam)` + `listModels()` + `listMcpServers()` — identical pattern to the existing `openConfig()` in App.tsx. Theme/locale state via `createThemeState()` and `createLocaleState()`.

Layout: `flex h-screen flex-col`. Top row is `flex flex-1 min-h-0` containing a 200px `<aside>` sidebar and a `flex-1 overflow-y-auto` `<main>` content area. Bottom is a `<footer>` with Cancel and Save buttons.

Categories array: `{ id: 'general', label: '...', icon: 'settings' }`, `{ id: 'account', label: '...', icon: 'user' }`, `{ id: 'models', label: '...', icon: 'brain' }`, `{ id: 'agent', label: '...', icon: 'spawn-swarm' }`, `{ id: 'mcp', label: '...', icon: 'terminal' }`, `{ id: 'advanced', label: '...', icon: 'goal' }`. Labels use the `t()` function with keys `app.config.categories.general` etc.

Content panels: `<Switch>` with `<Match>` per category. Each panel contains the exact same field components currently in the modal, just distributed by category:
- **General**: language dropdown + ThemePicker
- **Account**: sign-in/signed-in + API key fallback
- **Models**: brain model + builder model dropdowns
- **Agent**: parallel subagents slider, max rounds inputs, golden cycles/stalls, YOLO checkbox + blacklist
- **MCP**: MCP servers JSON textarea + add/test buttons + live status
- **Advanced**: plan save path + URL/API key overrides (behind easter egg check)

Save handler: validate API key if changed (non-OAuth), parse MCP JSON, call `setConfig()` with all signal values, call `setWorkspaceConfig()` if workspace active and plan_save_path changed, emit `settings-changed` to main window label, close window.

Cancel handler: just close window.

### App.tsx modifications

Entry detection at the very top of the App function component: check `new URLSearchParams(window.location.search).get('window') === 'settings'` — if true, return `<SettingsWindow />`. Import SettingsWindow and WebviewWindow.

New `openSettingsWindow()` function: check `WebviewWindow.getByLabel('settings')` — if exists, focus it and return. Otherwise construct `new WebviewWindow('settings', { url, title: 'Settings', width: 900, height: 650, center: true, resizable: true, minimizable: true, maximizable: false, focus: true })`. URL includes `?window=settings&workspace=<encoded>` if a workspace is active.

Gear button onClick: replace `openConfig` with `openSettingsWindow`.

Settings-changed listener: `getCurrentWebviewWindow().listen('settings-changed', ...)` reloads all config signals from `getConfig()` and `listModels()`. Cleaned up via `onCleanup`.

Remove: `showConfig` signal, `openConfig()` function, the entire `<Show when={showConfig()}>` modal block (approximately lines 656-1060 in current App.tsx).

### Capabilities

Add to `src-tauri/capabilities/default.json` permissions array: `"core:window:default"`, `"core:webview:default"`.

### User icon

Add to PATHS in Icon.tsx (alphabetically before `x`): pixel-art person silhouette — head at top (9-15,3-9), body below (5-17,11-19).

### i18n keys

en-US: `app.config.categories.general: "General"`, `app.config.categories.account: "Account"`, `app.config.categories.models: "Models"`, `app.config.categories.agent: "Agent Behavior"`, `app.config.categories.mcp: "MCP Servers"`, `app.config.categories.advanced: "Advanced"`.

pt-BR: `app.config.categories.general: "Geral"`, `app.config.categories.account: "Conta"`, `app.config.categories.models: "Modelos"`, `app.config.categories.agent: "Agente"`, `app.config.categories.mcp: "Servidores MCP"`, `app.config.categories.advanced: "Avançado"`.

### IPC functions reused

All from `src/lib/ipc.ts`: `getConfig(workspace?)`, `setConfig(args: SetConfigArgs)`, `setWorkspaceConfig(workspace, planSavePath)`, `listModels()`, `listMcpServers(workspace?)`, `validateApiKey(key)`. Also `parseMcpJson` and `mcpMapToJsonText` from existing utilities.

### Tauri APIs reused

From `@tauri-apps/api/webviewWindow`: `WebviewWindow` constructor, `getByLabel`, `getCurrent`. Event methods: `listen`, `emitTo`, `close`.

## Tasks

1. Add `user` pixel-art icon to `src/components/Icon.tsx` PATHS object
2. Add i18n category labels to `src/lib/locales/en-US.ts` and `src/lib/locales/pt-BR.ts` under `app.config.categories`
3. Add `core:window:default` and `core:webview:default` to `src-tauri/capabilities/default.json` permissions
4. Create `src/components/SettingsWindow.tsx` — sidebar with 6 categories, content panels, save/cancel handlers, config loading
5. Modify `src/App.tsx` — detect settings window entry, replace gear button with `openSettingsWindow()`, add `settings-changed` listener, remove old modal code
6. Build and verify end-to-end — test window creation, config load/save, event communication, error handling
