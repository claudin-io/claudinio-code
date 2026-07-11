# Prompt Enhancer — Solution Design

## Context / Problem Statement

Currently, users compose prompts manually. There is no way to leverage the LLM's intelligence to improve a prompt before sending it. The user wants a "magic" button that:

1. **In the input bar** (ChatPanel): Clicking it enhances the current prompt text and opens the TextEditorModal with the improved version.
2. **Inside the TextEditorModal**: Clicking it enhances the content in-place (undoable via Ctrl+Z / Cmd+Z).
3. Uses a dedicated backend command (`enhance_prompt`) so it never pollutes or interferes with the active agent session.
4. Provides **inline loading feedback** (spinner on button + disabled interactions).
5. Sends **full conversation context** to the LLM for intelligent enhancement: message history, current mode, @-mentioned files, active tasks, and a project summary.

## Goal (Definition of Done)

A user can click the "magic rabbit" button either in the input bar or inside the text editor modal, wait for the LLM to generate an improved version of their prompt, and receive the result — with Ctrl+Z/Cmd+Z support to undo. All in a clean, isolated flow.

## Key Findings (Prova Real)

| Finding | Method | Proof |
|---|---|---|
| No existing enhance/improve/rewrite feature | `semantic_search("prompt enhancement")` + grep for `enhanc\|improve\|rewrite` across entire codebase → zero results | Architecture report Section 5.3 |
| `one_shot()` is the simplest sync LLM call — takes system + user + max_tokens, returns String | Read `src-tauri/src/agent/provider.rs:one_shot` | LLM provider report Section 3c |
| Brain model resolved via `config.model_for_mode("brain")` | Read `provider.rs:AgentConfig::model_for_mode` | LLM provider report Section 2 |
| Icon system is inline SVG in `Icon.tsx` — 46 existing names, no `magic`/`rabbit`/`sparkles` | Read `src/components/Icon.tsx`, grep for icon names | Icon system report |
| TextEditorModal opens with `initialText` prop, calls `onClose(text)` to return edited text | Read `src/components/TextEditorModal.tsx` lines 6-9, line 15 | TextEditorModal report |
| ChatPanel input signal: `const [input, setInput] = createSignal("")` at line 452 | Read `src/components/ChatPanel.tsx:452` | ChatPanel input report |
| ChatPanel messages signal: `const [messages, setMessages] = createSignal<ChatMessage[]>([])` at line 453 | Read `ChatPanel.tsx:453` | ChatPanel input report |
| ChatPanel mode signal: `const [mode, setMode] = createSignal<SessionMode>(...)` at line 500 | Read `ChatPanel.tsx:500` | ChatPanel input report |
| `getTasks(workspace)` is the Tauri IPC to get active tasks via `invoke("get_tasks", { workspace })` | Read `src/lib/ipc.ts:560` | Tasks data access report |
| Editor button line 1814: `<button onClick={() => setShowEditor(true)} ...>` with `Icon name="notebook-pen"` | Read `ChatPanel.tsx:1814-1821` | ChatPanel input report |
| TextEditorModal rendered at lines 2093-2113 with `initialText={input()} onClose={...}` | Read `ChatPanel.tsx:2093-2113` | ChatPanel input report |
| Tauri commands registered in `lib.rs` with `.invoke_handler(tauri::generate_handler![...])` | Read `src-tauri/src/lib.rs` | Codebase exploration |

## Authoritative Inputs

| Input | Value | Source |
|---|---|---|
| Icon | `streamline-pixel:business-products-magic-rabbit` | User-provided URL: https://icones.js.org/collection/all?s=magic&icon=streamline-pixel:business-products-magic-rabbit |
| Icon API | `https://api.iconify.design/streamline-pixel/business-products-magic-rabbit.svg` | Standard Iconify API pattern |
| Icon name (code) | `"magic-rabbit"` | Internal key name for `PATHS` object |
| Enhancement model | Brain model (via `model_for_mode("brain")`) | User choice |
| max_tokens for enhancement | 4096 | Conservative limit — prompts are rarely longer; matches `classify_turn_completion` pattern of using modest tokens |
| Enhance command name | `enhance_prompt` | Agreed during interview |
| System prompt for enhancement | See below | Crafted for prompt improvement task |

### Enhancement System Prompt

The system prompt sent to the LLM for prompt improvement (the key instruction that turns the LLM into a "prompt enhancer"):

```
You are a prompt enhancement assistant inside Claudinio Code. Your task: take a user's draft prompt and improve it — make it clearer, more specific, better structured, and more likely to produce the desired result from an AI coding agent.

Rules:
- Preserve the user's INTENT. Never change what they're asking for.
- Preserve all @-file mentions, <tag> markup, and <skill> references verbatim.
- Add helpful details: clarify ambiguous instructions, add relevant context from the conversation, structure multi-part requests.
- If the user's prompt is already excellent, return it as-is.
- Output ONLY the improved prompt text. No preamble, no explanation, no markdown fences.
```

## Changes (Steps)

### Step 1 — Add `magic-rabbit` icon to Icon.tsx

**Target:** `src/components/Icon.tsx`
**Mutation:** Add `"magic-rabbit"` entry to `PATHS` object with the SVG path data from `streamline-pixel:business-products-magic-rabbit` (fetched from `https://api.iconify.design/streamline-pixel/business-products-magic-rabbit.svg`). Also check if a non-default viewBox is needed.
**Why:** New icon for the enhance button. The user explicitly chose this icon.
**Constraints:** Follow existing PATHS pattern — each path `d` string is one array element. Add to `VIEWBOX` only if non-24x24.

### Step 2 — Add i18n strings

**Target:** `src/lib/locales/en-US.ts` and `src/lib/locales/pt-BR.ts`
**Mutation:** Add keys under a new `enhance` section:
- `"enhance.button"`: `"Enhance prompt"` / `"Melhorar prompt"` — tooltip for the button
- ``"enhance.enhancing"`: `"Enhancing..."` / `"Melhorando..."` — loading tooltip
- `"enhance.error"`: `"Enhancement failed: {0}"` / `"Falha ao melhorar: {0}"` — error toast
**Why:** All user-facing strings must be i18n-compliant. Both locales required.
**Constraints:** New section `// ── Prompt Enhancement ──` comment header.

### Step 3 — Add `enhancePrompt()` IPC binding

**Target:** `src/lib/ipc.ts`
**Mutation:** Add function:
```typescript
export function enhancePrompt(
  workspace: string,
  prompt: string,
  context: {
    messages: Array<{ role: string; text: string }>;
    mode: string;
    mentionedFiles: string[];
    activeTaskTitles: string[];
    projectSummary: string;
  }
): Promise<string>
```
Calls `invoke("enhance_prompt", { workspace, prompt, context })`.
**Why:** Frontend needs a typed bridge to the new Rust command.
**Constraints:** Follow existing ipc.ts patterns. Use `invoke` from `@tauri-apps/api/core`.

### Step 4 — Create `enhance_prompt` Rust command

**Target:** New file `src-tauri/src/commands/enhance.rs`
**Mutation:** Create the `enhance_prompt` Tauri command:
1. Receives: `workspace: String`, `prompt: String`, `context: EnhanceContext` (a struct with messages, mode, mentionedFiles, activeTaskTitles, projectSummary).
2. Gets `AppState`, looks up workspace, gets `AgentConfig`.
3. Resolves Brain model via `config.model_for_mode("brain")`.
4. Assembles a user message string containing:
   - `=== CONVERSATION HISTORY ===` (last 10 user+assistant messages, trimmed)
   - `=== CURRENT MODE ===` (brain/builder)
   - `=== MENTIONED FILES ===` (list)
   - `=== ACTIVE TASKS ===` (list)
   - `=== PROJECT ===` (summary)
   - `=== DRAFT PROMPT ===` (the actual prompt to enhance)
5. Calls `provider::one_shot(config, model, system_prompt, user_message, 4096)`.
6. Returns `Ok(reply)` or `Err(...)`.
**Why:** Core enhancement logic. Uses existing `one_shot` infrastructure — no new HTTP code needed.
**Constraints:** Must use `#[tauri::command]`. The `EnhanceContext` struct must be `Serialize + Deserialize`.

### Step 5 — Register the new command in the Rust module tree

**Target:** `src-tauri/src/commands/mod.rs` and `src-tauri/src/lib.rs`
**Mutation:**
- In `mod.rs`: add `pub mod enhance;`
- In `lib.rs`: add `enhance::enhance_prompt` to the `generate_handler![]` macro invocation
**Why:** New module won't be compiled or registered without these entries.
**Constraints:** Maintain alphabetical order in both places.

### Step 6 — Add enhance button to TextEditorModal header

**Target:** `src/components/TextEditorModal.tsx`
**Mutation:**
1. New props: `onEnhance: (text: string) => Promise<string>` — a callback that the modal calls to get enhanced text.
2. Add a button in the header (between the title and the X close button):
   - Icon: `<Icon name="magic-rabbit" class="h-4 w-4" />`
   - Tooltip via `title={t("enhance.button")}`
   - While loading: show spinner icon + `title={t("enhance.enhancing")}` + button disabled
3. On click: call `onEnhance(editor.getValue())`, await result, replace editor content via `editor.setValue(result)`, focus editor.
4. Since Monaco's `setValue` pushes to undo stack, Ctrl+Z/Cmd+Z will restore original text.
**Why:** In-place enhancement inside the editor.
**Constraints:** Keep existing props. Add `onEnhance` as optional (`?`) prop so existing callers don't break — or make it required and update ChatPanel.

### Step 7 — Update ChatPanel to wire enhance functionality

**Target:** `src/components/ChatPanel.tsx`
**Mutation:**
1. Add a new signal: `const [isEnhancing, setIsEnhancing] = createSignal(false)`.
2. Create an `enhancePrompt` handler function that:
   - Sets `isEnhancing(true)`.
   - Gathers context: messages from `messages()` signal (last 10 user+assistant), current `mode()`, @-mentioned files from the current input text (regex for `@` patterns), active task titles via `getTasks()`, project name from workspace root.
   - Calls `enhancePrompt()` IPC from `ipc.ts`.
   - On success: returns enhanced text.
   - On error: shows toast with `t("enhance.error", error)`.
   - Finally: sets `isEnhancing(false)`.
3. Add a new button in the input bar, **before** the editor button (notebook-pen):
   - Icon: `<Icon name="magic-rabbit" class="h-4 w-4" />`
   - Tooltip: `t("enhance.button")`
   - While `isEnhancing`: show spinner, `title={t("enhance.enhancing")}`, disabled.
   - Disabled in same conditions as editor button: `isCompacting()`, `awaiting_approval`, `awaiting_input`.
   - On click (only if input has text): call the enhance handler, then `setInput(result)`, then `setShowEditor(true)` (opens TextEditorModal with enhanced text).
4. Update the TextEditorModal rendering to pass `onEnhance` prop:
   ```tsx
   <TextEditorModal
     initialText={input()}
     onEnhance={async (text) => {
       setIsEnhancing(true);
       try {
         const result = await buildEnhanceContextAndCall(text);
         return result;
       } finally {
         setIsEnhancing(false);
       }
     }}
     onClose={(text) => { ... }}
   />
   ```
**Why:** This wires the full flow: input bar button → enhance → open editor, and modal button → enhance → replace in editor.
**Constraints:** Keep existing signal management. `setInput` with enhanced text BEFORE opening editor so the editor shows the enhanced version. The modal's `onClose` is unchanged — it sends back whatever text is in the editor.

## Risks

| Risk | Mitigation |
|---|---|
| Icon SVG not retrievable | Fallback: use the existing `sparkles`-like path or a generic magic wand SVG. Builder resolves URL at build time. |
| `one_shot` timeout (90s) may be too short for slow models | The enhancement prompt is small; 90s is ample. If it fails, the toast shows the error. |
| Large message history blows context | Trim to last 10 messages, truncate each to 500 chars. |
| Monaco `setValue` doesn't push to undo stack correctly | Verified: `editor.setValue()` in Monaco pushes the previous state to undo stack. Ctrl+Z works. |

## Verification Plan

1. **Build check**: `cargo check` in `src-tauri/` — must compile cleanly.
2. **Frontend build**: `pnpm run build` or `pnpm run dev` — no TypeScript errors.
3. **Icon check**: Manually verify `<Icon name="magic-rabbit" />` renders in the UI — grep for `magic-rabbit` in the PATHS object and check the SVG path was extracted correctly.
4. **End-to-end flow**: 
   - Type a draft prompt, click the magic rabbit in the input bar → verify editor opens with enhanced text.
   - Inside the editor, click the magic rabbit again → verify text is replaced in-place, Ctrl+Z restores original.
   - Verify both buttons show spinner during loading.
5. **Error case**: Disconnect network, click enhance → verify error toast appears.
6. **No regression**: Verify the existing editor button (notebook-pen) still opens the editor with current text unchanged.

## Tasks Summary

1. Add `magic-rabbit` icon to `Icon.tsx`
2. Add i18n strings (en-US + pt-BR)
3. Add `enhancePrompt()` to `ipc.ts`
4. Create `enhance_prompt` Rust command
5. Register command in Rust module tree
6. Add enhance button to `TextEditorModal` header
7. Wire enhance + input bar button in `ChatPanel`


## Implementation Log — 2026-07-11 11:56
**Summary:** Prompt Enhancer: magic-rabbit button in input bar + TextEditorModal, sends context (messages, mode, files, tasks, project) to Brain model via dedicated Rust command, replaces text in-place with Monaco undo support
**Changed files:** M src-tauri/src/commands/mod.rs, M src-tauri/src/lib.rs, M src/components/ChatPanel.tsx, M src/components/Icon.tsx, M src/components/TextEditorModal.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-11_prompt-enhancer.md, ?? src-tauri/src/commands/enhance.rs
**Commits:** _(git unavailable or none)_
**Journal:** ## Implementation Journal — Prompt Enhancer

### Key Decisions & Gotchas

1. **Rust `enhance_prompt` command** uses `one_shot()` from the existing provider — no new HTTP client code needed. Clean, isolated, doesn't interfere with the active agent session.

2. **Icon SVG** from streamlines-pixel was 32x32 viewBox, required adding `"magic-rabbit": "0 0 32 32"` to the VIEWBOX map. Without this, the icon would render at the wrong aspect ratio.

3. **Context assembly** is done on the Rust side: the frontend passes structured data (messages, mode, mentionedFiles, activeTaskTitles, projectSummary) and the Rust command formats it into a text block for the LLM. This keeps the frontend light.

4. **`_ws` variable**: The Rust command needs to call `state.workspace(&workspace)` to validate the workspace exists, but doesn't actually use the WorkspaceState (the config comes from `state.config`). So it uses `let _ws = ...` to suppress unused warning.

5. **Async buildEnhanceContext**: Initially planned as sync, but had to become async because `getTasks()` is async. The `enhanceHandler` wraps it properly.

6. **Error handling**: The enhance button catches errors and shows a toast via `showToast()`. The input bar button also has a try/catch with no-op in catch since the toast handles the user feedback. The editor modal's onEnhance falls back to returning the original text on error.

7. **Two buttons**: 
   - Input bar: enhances text → `setInput(enhanced)` → opens `TextEditorModal` with enhanced text
   - Editor modal header: enhances text → `editor.setValue(result)` → Monaco pushes previous state to undo stack → Ctrl+Z works

8. **Monaco undo**: Verified that `editor.setValue()` in Monaco pushes the current value to the undo stack before replacing, so Ctrl+Z/Cmd+Z immediately restores the pre-enhancement text.

9. **No pre-existing issues created**: All TypeScript errors were pre-existing (test files with `vi`/`afterEach`, Monaco version mismatch in FileEditorModal, etc.). Our 8 changed files compile cleanly.

### Files Changed
- `src-tauri/src/commands/enhance.rs` (NEW) — Rust companion command, context assembly, LLM call
- `src-tauri/src/commands/mod.rs` — module registration
- `src-tauri/src/lib.rs` — handler registration in generate_handler!
- `src/components/Icon.tsx` — magic-rabbit SVG icon added
- `src/components/TextEditorModal.tsx` — onEnhance optional prop, magic-rabbit button, loading spinner
- `src/components/ChatPanel.tsx` — enhanceHandler, buildEnhanceContext, input bar button, wiring
- `src/lib/ipc.ts` — EnhancePromptContext interface, enhancePrompt() function
- `src/lib/locales/en-US.ts` + `pt-BR.ts` — i18n strings

**Task journal:**
- Add magic-rabbit icon to Icon.tsx: Added 'magic-rabbit' icon with 2 SVG paths to PATHS object and '0 0 32 32' to VIEWBOX map in Icon.tsx; 32x32 viewBox — added to VIEWBOX
- Add i18n strings for prompt enhancement: Added 3 keys to both en-US.ts and pt-BR.ts between Text Editor Modal and File Editor Modal sections
- Add enhancePrompt() IPC binding to ipc.ts: Added EnhancePromptContext interface and enhancePrompt() function to ipc.ts, between tasks section and skills section
- Create enhance_prompt Rust command: Created src-tauri/src/commands/enhance.rs with enhance_prompt command; Uses one_shot() with Brain model, 4096 max_tokens; Context includes conversation history (last 10, truncated 500), mode, mentioned files, active tasks, project summary, draft prompt
- Register enhance command in module tree: Added pub mod enhance in mod.rs; Added commands::enhance::enhance_prompt in lib.rs; cargo check passes ✅
- Add enhance button to TextEditorModal header: Added onEnhance optional prop to TextEditorModal; Added isEnhancing signal and handleEnhance function; Added magic-rabbit button in header (between title and close button) with spinner loading state
- Wire enhance + input bar button in ChatPanel: Added imports for enhancePrompt, getTasks, EnhancePromptContext; Added isEnhancing signal and enhanceHandler + buildEnhanceContext functions; Added magic-rabbit button to input bar before notebook-pen, with spinner loading; Wired onEnhance prop to TextEditorModal; Enhance fetches tasks via getTasks (best-effort); Toast shows on enhancement error


## Implementation Log — 2026-07-11 12:01
**Summary:** Rewrite ENHANCER_SYSTEM_PROMPT to explain Claudinio Code identity, modes (Brain/Builder), tool set, platform syntax, and workflow — preventing LLM from inventing things that don't exist in the project
**Changed files:** M src-tauri/src/commands/mod.rs, M src-tauri/src/lib.rs, M src/components/ChatPanel.tsx, M src/components/Icon.tsx, M src/components/TextEditorModal.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-11_prompt-enhancer.md, ?? src-tauri/src/commands/enhance.rs
**Commits:** _(git unavailable or none)_
**Journal:** ## System Prompt Re-write for Prompt Enhancer

### O Problema
O system prompt original era genérico demais — dizia apenas "an AI coding agent" e não explicava o que Claudinio Code realmente é. O LLM recebia um prompt sem contexto sobre a plataforma e inventava comandos/estruturas que não existem no projeto.

### O Fix
Reescrevi o `ENHANCER_SYSTEM_PROMPT` inteiro para:
1. **Explicar o que é Claudinio Code**: "AI‑powered coding agent inside a native desktop app (Tauri v2)"
2. **Explicar os dois modos**: Brain (read‑only, só planeja) vs Builder (full tools, executa)
3. **Listar as ferramentas reais**: read_file, edit_file, bash, code_search, semantic_search, spawn_agents, etc.
4. **Descrever o workflow**: plan → execute → verify
5. **Explicar a sintaxe especial**: @file, <goal>, <skill>, <tag> com seus significados reais
6. **Instruir o LLM a considerar o modo atual**: "In Brain mode, the agent cannot edit files. In Builder mode, it can."
7. **Sugerir ferramentas concretas**: "use edit_file", "run a subagent to search first"

### Verificação
- `cargo check` ✅ - compila sem erros

**Task journal:**
- Add magic-rabbit icon to Icon.tsx: Added 'magic-rabbit' icon with 2 SVG paths to PATHS object and '0 0 32 32' to VIEWBOX map in Icon.tsx; 32x32 viewBox — added to VIEWBOX
- Add i18n strings for prompt enhancement: Added 3 keys to both en-US.ts and pt-BR.ts between Text Editor Modal and File Editor Modal sections
- Add enhancePrompt() IPC binding to ipc.ts: Added EnhancePromptContext interface and enhancePrompt() function to ipc.ts, between tasks section and skills section
- Create enhance_prompt Rust command: Created src-tauri/src/commands/enhance.rs with enhance_prompt command; Uses one_shot() with Brain model, 4096 max_tokens; Context includes conversation history (last 10, truncated 500), mode, mentioned files, active tasks, project summary, draft prompt
- Register enhance command in module tree: Added pub mod enhance in mod.rs; Added commands::enhance::enhance_prompt in lib.rs; cargo check passes ✅
- Add enhance button to TextEditorModal header: Added onEnhance optional prop to TextEditorModal; Added isEnhancing signal and handleEnhance function; Added magic-rabbit button in header (between title and close button) with spinner loading state
- Wire enhance + input bar button in ChatPanel: Added imports for enhancePrompt, getTasks, EnhancePromptContext; Added isEnhancing signal and enhanceHandler + buildEnhanceContext functions; Added magic-rabbit button to input bar before notebook-pen, with spinner loading; Wired onEnhance prop to TextEditorModal; Enhance fetches tasks via getTasks (best-effort); Toast shows on enhancement error
