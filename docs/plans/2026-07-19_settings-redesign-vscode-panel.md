# Settings Redesign: VS Code-Style Panel

## Context

The current settings UI lives inline inside `App.tsx` (~650 lines of settings JSX in a 1400+ line file). It opens as a fixed 680px wide modal with all settings in a flat, scrollable list. There is no search, no categorization, and settings are spread across the modal in an ad-hoc order. Users struggle to find specific settings quickly.

The user confirmed: the primary goal is **usability — settings are hard to find and navigate**. The redesign will introduce a sidebar + content area panel (VS Code-style), search filtering, logical categories, and also extract the settings code from App.tsx into proper components.

## Solution Design

### Navigation Structure

Replace the current 680px fixed modal with a **resizable panel** that slides in from the right side of the window. It has two zones:

**Left sidebar (~220px):**
- Category list with icons: General, Models, Account, Agent, MCP
- Active category highlighted with accent color
- Badge counts for non-default settings per category (e.g., "2 changed" indicator)

**Right content area:**
- Scrollable, shows all settings for the active category
- Each setting has its label, input, hint text, and source badge (Local vs Workspace)

### Search

A search bar sits at the top of the sidebar. As the user types:
- Categories with no matching settings collapse/hide
- Only matching settings remain visible in the content area
- Matched text is highlighted in both the sidebar (category name match) and content area (label match)
- Clearing the search restores full visibility

### Sizing & Resize

- Default width: **850px**
- Panel sits on the right side with a 4px drag handle on its **left edge**
- Minimum width: 480px, Maximum width: 90vw
- Height: full viewport (covers the entire window, no backdrop)
- A subtle backdrop blur remains to separate from the main UI

### Categories & Settings Mapping

**General** (icon: `sliders` — will add to Icon):
- Language (select)
- Theme (ThemePicker grid)
- Keep Awake (checkbox)
- Plan Save Path (text input + browse button)
- Preferred IDE (select)
- Auto-commit Plan (checkbox)
- Code Intelligence (checkbox)

**Models** (icon: `brain`):
- Brain Model (select/text input)
- Builder Model (select/text input)
- Max Parallel Agents (slider)
- Max Rounds (number input)
- Sub Max Rounds (number input)
- Max Golden Cycles (number input)
- Max Golden Stalls (number input)
- Handoff Threshold (slider)
- **Advanced** (collapsed subsection, shown only when "iddqd" easter egg active):
  - Override Base URL (text input)
  - Override API Key (password input)

**Account** (icon: `key` — will add to Icon):
- Login/Logout button + signed-in status
- API Key input
- Support link

**Agent** (icon: `construction-worker` — aliased as `robot`):
- YOLO Mode (checkbox)
- YOLO Blacklist (textarea)

**MCP** (icon: `package-process` — aliased as `server`):
- JSON editor for MCP server configs
- Test/status indicators

### Visual Design

- Same design tokens as the existing app (`--surface-*`, `--ink-*`, `--border-*`, `--accent`)
- Sidebar uses `--surface-2` background to visually separate from content area (`--surface-1`)
- Category items: 36px height, 8px border-radius, icon + label + optional badge
- Content area: 24px padding, settings in a single-column layout with consistent spacing
- Search bar: rounded pill at top of sidebar, matches the app's input style
- Resize handle: 4px wide, hover glow with accent color
- Panel slides in from right with a 200ms ease-out transition

## Risks

- **Extraction regression**: moving settings signals from App.tsx to child components could break reactive updates. Mitigation: pass signals as accessor/setter props, not values; test each category in isolation.
- **Save flow**: the current `saveConfig()` is a monolithic function in App.tsx. After extraction, save logic must remain intact. Mitigation: keep saveConfig in App.tsx and pass it as a callback.
- **Resize behavior on small screens**: below 1024px, panel could overflow. Mitigation: clamp min width to 480px, below which panel takes full viewport width.
- **Easter egg persistence**: the "iddqd" easter egg state must remain accessible to the Models sub-category. Mitigation: keep `easterEggActive` signal and keystroke handler in App.tsx, pass as prop.
- **SolidJS reactivity**: passing signal accessors as props preserves reactivity. Passing values (calling the accessor) at render time loses it. Mitigation: always pass accessor functions (e.g., `brainModel={() => string}`), never raw values. Verify with a quick smoke test that changing a setting in-state reflects immediately.

## Non-goals

- No settings page persistence (panel closes via Escape or clicking backdrop, same as current)
- No import/export settings
- No settings sync across devices
- No per-workspace settings editing (workspace overrides are read-only indicators, same as current)
- No change to the backend config storage (Rust `provider.rs`, `config.json`, `.claudinio.json`)
- No changes to theme or locale storage (they remain in localStorage)

## Low-Level Design

### Architecture Overview

Extract settings from `App.tsx` into a `SettingsPanel` container with 5 subsection components, keeping reactive signals in App.tsx and passing them as props. The new panel replaces the `<Show when={showConfig()}>` block currently in App.tsx (~lines 746-1380).

### File Changes

#### New Files

1. **`src/components/SettingsPanel.tsx`** — Container component
   - Props: all 31 settings signals as accessor/setter pairs, plus callbacks (saveConfig, doLogin, doLogout, pickPlanPath), plus easter egg signals, plus search state
   - Owns: sidebar navigation state (`activeCategory`), search query state, resize state
   - Exports: `SettingsPanel` named export
   - Structure:
     ```
     <div class="settings-panel-overlay">  <!-- backdrop, Escape key listener -->
       <SettingsPanelResizeHandle />       <!-- 4px drag handle -->
       <div class="settings-panel">        <!-- the panel itself, width from signal -->
         <SettingsSidebar>                  <!-- left 220px -->
           <SettingsSearchBar />
           <SettingsCategoryList />
         </SettingsSidebar>
         <SettingsContent>                  <!-- right, scrollable -->
           <Show when={activeCategory === 'general'}><SettingsGeneral /></Show>
           <Show when={activeCategory === 'models'}><SettingsModels /></Show>
           <Show when={activeCategory === 'account'}><SettingsAccount /></Show>
           <Show when={activeCategory === 'agent'}><SettingsAgent /></Show>
           <Show when={activeCategory === 'mcp'}><SettingsMcp /></Show>
         </SettingsContent>
       </div>
     </div>
     ```

2. **`src/components/settings/SettingsGeneral.tsx`** — General category
   - Props: language/theme signals, keepAwake, planSavePath, preferredIde, autoCommitPlan, codeIntelEnabled + their setters, `t()`, `pickPlanPath`, `availableIdes`, `availableModels`
   - Renders: Language select, ThemePicker, Keep Awake toggle, Plan Save Path input, Preferred IDE select, Auto-commit toggle, Code Intel toggle

3. **`src/components/settings/SettingsModels.tsx`** — Models category
   - Props: brainModel, builderModel, maxParallelAgents, maxRounds, subMaxRounds, maxGoldenCycles, maxGoldenStalls, handoffTokens + setters, workspaceConfigFields, easterEggActive, overrideBaseUrl/overrideApiKey + setters, availableModels
   - Renders: Brain/Builder model selects, Parallel agents slider, Max rounds inputs, Golden cycle inputs, Handoff slider, Advanced subsection (conditional on easterEgg)

4. **`src/components/settings/SettingsAccount.tsx`** — Account category
   - Props: accountLogin, hasApiKey, loggingIn, configApiKey, settingsApiKeyError + setters, doLogin, doLogout
   - Renders: Sign in/out UI, API key input, Support link

5. **`src/components/settings/SettingsAgent.tsx`** — Agent category
   - Props: yoloMode, yoloBlacklist + setters, workspaceConfigFields
   - Renders: YOLO Mode toggle, YOLO Blacklist textarea

6. **`src/components/settings/SettingsMcp.tsx`** — MCP category
   - Props: configMcpJson, mcpJsonError, mcpStatuses, mcpTesting + setters, `mcpMapToJsonText`, `parseMcpJson`, `mcpServerTemplate`, `listMcpServers`, `testMcpServer`, `activeWorkspace`
   - Renders: JSON editor textarea, test button, status list

#### Modified Files

7. **`src/App.tsx`** — Remove ~600 lines of settings JSX, replace with `<SettingsPanel>` component
   - Lines ~746-1380: Replace the entire `<Show when={showConfig()}>` block with `<SettingsPanel ...props />`
   - Keep: all signals, `openConfig`, `saveConfig`, `doLogin`, `doLogout`, `pickPlanPath`, easter egg handler
   - `saveConfig` stays in App.tsx; passed as callback prop to SettingsPanel
   - The `<button onClick={openConfig}>` (gear icon, line ~737) stays unchanged

8. **`src/components/Icon.tsx`** — Add 2 new icon paths
   - Add `sliders` icon (for General) — horizontal sliders SVG path (fill, 24×24 viewBox, standard lucide-style)
   - Add `key` icon (for Account) — key SVG path (fill, 24×24 viewBox)

9. **`src/lib/grill-me.ts`** — Potentially no changes needed, existing keys cover labels. If new i18n keys are needed (e.g., `settings.category.*` for category names), add them.

10. **`src/App.css`** — Add settings panel styles
    - `.settings-panel-overlay`: `fixed inset-0 z-50`, background `bg-black/40 backdrop-blur-[2px]`, flex justify-end (panel on right)
    - `.settings-panel`: width from CSS variable `--settings-panel-width`, `bg-surface-1`, flex row, `transition: width 0.1s` (during drag)
    - `.settings-panel-sidebar`: width `220px`, `bg-surface-2`, flex column, `border-r border-border-subtle`
    - `.settings-panel-content`: flex-1, overflow-y-auto, padding `1.5rem`
    - `.settings-panel-resize-handle`: 4px wide, `cursor-ew-resize`, hover/focus glow
    - Search bar: rounded pill input, full-width minus padding
    - Category items: `h-9`, `rounded-md`, `cursor-pointer`, active state with `bg-surface-0 text-accent`, default `text-ink-muted hover:text-ink`
    - Category badge: `rounded-full bg-accent/15 text-accent text-[10px] px-1.5`

### Data Flow

```
App.tsx (signals + saveConfig)
  │
  ├── SettingsPanel (container)
  │     ├── sidebar state: activeCategory (createSignal)
  │     ├── search state: searchQuery (createSignal)
  │     ├── resize state: panelWidth (createSignal, default 850)
  │     │
  │     ├── SettingsSidebar
  │     │   ├── search bar → sets searchQuery
  │     │   └── category list → sets activeCategory
  │     │
  │     └── SettingsContent
  │           ├── SettingsGeneral (receives signals as props)
  │           ├── SettingsModels (receives signals as props)
  │           ├── SettingsAccount (receives signals as props)
  │           ├── SettingsAgent (receives signals as props)
  │           └── SettingsMcp (receives signals as props)
  │
  └── Footer: Cancel + Save buttons → call saveConfig()
```

### Search Implementation

- `searchQuery` signal in SettingsPanel
- `filteredCategories` derived signal using `createMemo`:
  - Maps each category to its settings keys + labels
  - Checks if `searchQuery` appears in category name OR any setting label
  - Returns filtered array of category IDs with match metadata
- Sidebar renders only categories that pass the filter
- When search is active (`searchQuery().length > 0`), content area shows ALL settings from matching categories (cross-category view)
- When search is empty, shows only the active category

### Resize Implementation

- `panelWidth` signal (default 850)
- `onMouseDown` on resize handle: records `startX`, `startWidth`
- `onMouseMove` on document (during drag): `newWidth = startWidth + (startX - e.clientX)`, clamped to [480, viewportWidth * 0.9]
- `onMouseUp`: cleanup listeners
- CSS variable `--settings-panel-width` set via inline style on panel div

### Component Props Interfaces

Each sub-category component receives only the signals it needs — accessor functions (e.g., `brainModel: () => string`) and setter functions (e.g., `setBrainModel: (v: string) => void`). This keeps dependency explicit and testable.

```typescript
// Example: SettingsModels props
interface SettingsModelsProps {
  brainModel: Accessor<string>;
  setBrainModel: Setter<string>;
  builderModel: Accessor<string>;
  setBuilderModel: Setter<string>;
  maxParallelAgents: Accessor<number>;
  setMaxParallelAgents: Setter<number>;
  // ... other signals
  workspaceConfigFields: Accessor<Set<string>>;
  easterEggActive: Accessor<boolean>;
  // ...
}
```

### Easter Egg Handling

The keystroke handler stays in App.tsx (listens on `document` keydown). The `easterEggActive` signal is passed as a prop to SettingsModels, which uses `<Show when={easterEggActive()}>` to reveal the Advanced subsection. The `setEasterEggActive(false)` call on `openConfig` stays in App.tsx.

### Save Button Behavior

Save stays a footer button (not auto-save). The `saveConfig` function remains in App.tsx. SettingsPanel receives `onSave` prop. The footer renders "Cancel" (closes) and "Save" buttons.

### Icon Additions

Add to `PATHS` in `src/components/Icon.tsx`:

```typescript
// sliders — horizontal adjustment sliders
sliders: {
  viewBox: "0 0 24 24",
  path: "M4 21v-7m0-4V3m8 18v-9m0-4V3m8 18v-5m0-4V3M1 14h6m2-6h6m2 8h6",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 2,
  strokeLinecap: "round",
},
// key — classic key shape
key: {
  viewBox: "0 0 24 24",
  path: "M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 2,
  strokeLinecap: "round",
  strokeLinejoin: "round",
},
```

Existing icons mapped to categories:
- General: `sliders` (NEW)
- Models: `brain` (existing)
- Account: `key` (NEW)
- Agent: `construction-worker` (existing)
- MCP: `package-process` (existing)

### Tasks Summary

1. Add `sliders` and `key` icon paths to `Icon.tsx`
2. Create `src/components/settings/SettingsGeneral.tsx`
3. Create `src/components/settings/SettingsModels.tsx`
4. Create `src/components/settings/SettingsAccount.tsx`
5. Create `src/components/settings/SettingsAgent.tsx`
6. Create `src/components/settings/SettingsMcp.tsx`
7. Create `src/components/SettingsPanel.tsx` (container + search + resize)
8. Add settings panel CSS to `App.css`
9. Wire SettingsPanel into `App.tsx` (replace old modal, pass props)
10. Verify: build, open settings, navigate categories, search, resize, save


## Implementation Log — 2026-07-19 09:14
**Summary:** Settings panel redesign: VS Code-style sidebar + content panel, 5 category tabs, search filtering, resizable (480-90vw). All 35 test files/643 tests pass, production build succeeds.
**Changed files:** A	docs/plans/2026-07-19_settings-redesign-vscode-panel.md
**Commits:** 67fa003 docs(plan): settings-redesign-vscode-panel, 4bff3ff docs(plan): settings-redesign-vscode-panel
**Journal:** ## Build & Test Verification
- `pnpm test`: 35 test files, 643 tests — ALL passed, zero failures
- `pnpm run build` (includes `vitest run && vite build`): tests passed + Vite production build succeeded in 14.51s, zero errors
- Only pre-existing Vite info: `ipc.ts` has dual static/dynamic import — unrelated to our changes

## Key Architectural Decisions
- **Signals kept in App.tsx** — passed as accessor/setter props to SettingsPanel. This avoids reactivity regressions and keeps state ownership clear.
- **Resize handle on left edge** — panel docks to the right, so dragging left edge is natural
- **Footer inside flex-column panel** — simpler than fixed-position, no z-index or alignment issues
- **Search shows ALL matching categories** (cross-category) when query is active — prevents hidden results when the user searches from a different tab
- **Easter egg (`iddqd`)** — keystroke handler stays in App.tsx, passes `easterEggActive` boolean as prop through SettingsPanel → SettingsModels
- **Category sidebar labels hardcoded** — not i18n'd for simplicity; the search keys are translated, but sidebar titles use static English strings

## Files Changed
- `src/components/Icon.tsx` — added `sliders` and `key` stroke icons
- `src/components/SettingsPanel.tsx` — new VS Code-style container (sidebar nav, search, resize, footer)
- `src/components/settings/SettingsGeneral.tsx` — new (7 sections)
- `src/components/settings/SettingsModels.tsx` — new (6 sections + easter egg subsection)
- `src/components/settings/SettingsAccount.tsx` — new (signed-in/out states)
- `src/components/settings/SettingsAgent.tsx` — new (YOLO mode + blacklist)
- `src/components/settings/SettingsMcp.tsx` — new (JSON editor + server status)
- `src/App.css` — appended ~300 lines of settings panel CSS
- `src/App.tsx` — replaced ~626 lines of inline modal with `<SettingsPanel>`, cleaned up unused imports

## Gotchas & Lessons
- The `setLocale` signal in App.tsx is typed as `Accessor<string>`, but SettingsModels needs a setter. We wrapped it as `(v: string) => setLocale(v)` at the call site — clean adapter, no type gymnastics.
- Monaco editor lazy chunks added ~14 lines of output to build log but no errors.
- Pre-existing TS errors in ChatPanel.tsx and test files remain untouched — confirmed zero NEW errors from settings extraction.

**Task journal:**
- Add sliders and key icons to Icon.tsx: Added 'key' icon to PATHS (line 111) and STROKE_ICONS (line 272). Added 'sliders' icon to PATHS (line 26) and STROKE_ICONS (line 274). Both use standard 24×24 viewBox. No new TS errors.
- Create SettingsGeneral component: Created with all 7 sections. Zero TS errors.
- Create SettingsModels component: Created with 6 sections. Zero TS errors.
- Create SettingsAccount component: Created with signed-in/out states. Zero TS errors.
- Create SettingsAgent component: Created with YOLO Mode + blacklist. Zero TS errors.
- Create SettingsMcp component: Created with editor + status list. Zero TS errors.
- Create SettingsPanel container with sidebar, search, and resize: Created 307-line container. Zero TS errors.
- Add settings panel CSS to App.css: All CSS rules appended. No errors.
- Wire SettingsPanel into App.tsx (replace old modal): Replaced ~626 lines of inline JSX with SettingsPanel component. Added import. Wrapped setLocale with Setter adapter. Removed unused imports (SUPPORTED_LOCALES, FLAGS, LOCALE_LABELS, ThemePicker). Zero new TS errors from App.tsx.
- Verify: build, smoke-test all features: pnpm test: 35 test files, 643 tests, ALL passed — zero failures.; pnpm run build: tests re-ran (643 passed), then vite build completed successfully in 14.51s. No build errors. Only pre-existing Vite info warning about ipc.ts dual static/dynamic import.
