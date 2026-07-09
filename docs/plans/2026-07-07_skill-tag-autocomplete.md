# Solution Design: `<skill>` Tag Autocomplete

## Context / Problem Statement

The user wants a 2-step autocomplete flow triggered by typing `<` in the chat input:

1. **Step 1 — Tag Type Selection:** Typing `<` opens a popover listing available tag types. Currently only "skill" is implemented, but the code should be prepared for future types like "agent", "prompt", etc.
2. **Step 2 — Skill Selection:** Selecting "skill" inserts `<skill>` into the text and immediately opens a second popover listing ALL available skills (fetched from the backend via `listSkills()`, which already returns `name` + `description`). Each item shows the skill name with a light description as a footnote.
3. **Completion:** Selecting a skill closes the tag with `</skill>` and positions the cursor between the opening and closing tags.

Fuzzy search should match against both skill name and description.

The existing `@` mention (file autocomplete via `FileMentionPopover`) is the reference pattern — same textarea detection approach, same caret coordinate mirror div, same keyboard navigation.

**CONFIRMED by user:**
- Full 2-step flow in this iteration
- Code prepared for future tag types (agent, prompt — disabled for now)
- All logic inside `ChatPanel.tsx`, reusing the `@`-mention pattern
- Popovers as separate components for cleanliness

## Goal (Definition of Done)

Typing `<` in the chat input opens a popover with "skill" as a selectable tag; selecting it inserts `<skill> ` and opens a skill picker showing all available skills with name + description; selecting a skill inserts `</skill>` and places the cursor between the tags. The flow is keyboard-navigable (Arrow keys, Enter, Escape) and supports fuzzy filtering.

## Key Findings (Prova Real)

1. **`SkillEntry` already has `name` + `description`** — `src-tauri/src/agent/skills.rs:27-36`. The `SkillEntry` struct serializes both fields. **Proof:** `pub name: String` (line 29), `pub description: String` (line 31).

2. **`listSkills()` IPC function already exists** — `src/lib/ipc.ts:472-474`. Returns `SkillsResponse` with `skills: SkillEntry[]` and `count: number`. **Proof:** `function listSkills(workspace: string): Promise<SkillsResponse>`.

3. **`FileMentionPopover` is the reference pattern** — `src/components/FileMentionPopover.tsx`. Uses Fuse.js for fuzzy matching, Portal rendering at computed caret coordinates, keyboard navigation (ArrowDown/Up, Enter, Escape), transparent backdrop for click-outside. **Proof:** lines 1-100, full component.

4. **Textarea trigger detection uses a mirror div** — `src/components/ChatPanel.tsx:getCaretCoordinates()` (lines ~515-560). Computes pixel position of a character index by rendering text into a hidden div with identical styles. **Proof:** `getCaretCoordinates` function.

5. **`listSkills()` is NOT called anywhere on the frontend yet** — the IPC function exists but there is no UI that consumes it. **Proof:** `grep` for `listSkills` in `src/` returns only the definition in `ipc.ts`.

6. **Fuse.js is already a dependency** — `package.json:18`, `"fuse.js": "^7.4.2"`. No new dependency needed.

## Authoritative Inputs

- **Backend skill list endpoint:** `listSkills(workspace: string) → SkillsResponse` (`src/lib/ipc.ts:472`)
- **SkillEntry shape:** `{ name: string, description: string, location: string, scope: string, body?: string }` (`src/lib/ipc.ts:449-453`)
- **Fuse.js API:** threshold 0.4, distance 100 (same as FileMentionPopover)
- **Textarea ref:** `inputRef: HTMLTextAreaElement` in ChatPanel
- **Caret coordinates helper:** `getCaretCoordinates(textarea, pos) → { top, left, height }` in ChatPanel
- **Popover positioning constants:** POPOVER_MAX_HEIGHT ≈ 260px, POPOVER_WIDTH ≈ 320px (for skill popover with description; tag popover is narrower ~220px), MARGIN = 8px

## Changes (Steps)

### Step 1: Create `TagMentionPopover` component (`src/components/TagMentionPopover.tsx`)

- **Target:** New file `src/components/TagMentionPopover.tsx`
- **Mutation:** Create a popover component similar to `FileMentionPopover` but for tag types.
  - Uses `Portal` for rendering at computed coordinates
  - Static tag list: `[{ id: 'skill', label: 'skill', icon: 'package', enabled: true }, { id: 'agent', label: 'agent', icon: 'robot', enabled: false }, { id: 'prompt', label: 'prompt', icon: 'message-square', enabled: false }]`
  - Disabled/future tags rendered with muted styling + "soon" badge
  - Fuse.js filtering on label as user types after `<`
  - Keyboard navigation: ArrowDown/Up, Enter to select, Escape to close
  - Transparent backdrop for click-outside dismiss
  - Props: `position: { top, left, height }`, `query: string`, `onSelect: (tagType: string) => void`, `onClose: () => void`
- **Why:** Step 1 of the 2-step flow; clean separation from ChatPanel
- **Constraints:** Follow exact pattern of `FileMentionPopover` (Portal, backdrop, keyboard events, Fuse.js). Use existing `Icon` component for icons. Width ~220px for compact tag list.

### Step 2: Create `SkillMentionPopover` component (`src/components/SkillMentionPopover.tsx`)

- **Target:** New file `src/components/SkillMentionPopover.tsx`
- **Mutation:** Create a popover component for skill selection.
  - Fetches skills on mount via `listSkills(props.workspace)` from `../lib/ipc`
  - Shows loading state while fetching; error state on failure
  - Each item: skill name (bold-ish, font-mono or text-ink) + description below as muted smaller text (text-ink-faint, text-[11px])
  - Fuse.js fuzzy search across BOTH `name` and `description` (use `keys: ['name', 'description']` in Fuse config)
  - Keyboard navigation: ArrowDown/Up, Enter to select, Escape to close
  - Transparent backdrop, Portal rendering, same coordinate math
  - Props: `workspace: string`, `position: { top, left, height }`, `query: string`, `onSelect: (skillName: string) => void`, `onClose: () => void`
- **Why:** Step 2 of the flow; shows full skill list with descriptions
- **Constraints:** Follow `FileMentionPopover` pattern. Import `listSkills` from `../lib/ipc`. Use `createResource` or manual `createSignal` + `onMount` for async fetch. Wider popover (~340-400px) to accommodate description text.

### Step 3: Integrate tag + skill flow into `ChatPanel.tsx`

- **Target:** `src/components/ChatPanel.tsx`
- **Mutation:**
  1. **State additions:** Add signals for `tagQuery`, `tagPosition`, `skillQuery`, `skillPosition`, and a `tagFlowStep` signal (`'tag' | 'skill' | null`).
  2. **Import new components:** `TagMentionPopover` and `SkillMentionPopover`.
  3. **onInput handler extension:** After the existing `@`-mention detection logic, add detection for `<` trigger:
     - Scan backwards from caret for `<`. If found before a space/newline, extract query after `<`, compute pixel position via `getCaretCoordinates`, set `tagQuery`, `tagPosition`, `tagFlowStep('tag')`.
     - Special handling: if the text around caret is inside `<skill>` already (i.e., after selecting tag type but before picking a skill), detect the query between `<skill>` and caret, set `skillQuery`, `skillPosition`, `tagFlowStep('skill')`.
  4. **`handleTagSelect(tagType: string)`:**
     - Replace `<query` with `<skill>` at the correct position in the input text.
     - Set `tagPosition` to null, `tagFlowStep('skill')`.
     - Compute new caret position (right after `<skill>`), set `skillQuery('')`, compute `skillPosition` based on new caret location.
  5. **`handleSkillSelect(skillName: string)`:**
     - Replace `<skill>` (or the query after it) with `<skill>skillName</skill>`.
     - Position cursor between `>` and `</skill>`.
     - Clear all popover state.
  6. **`handlePopoverClose`:** Clear all tag/skill popover state, reset `tagFlowStep(null)`.
  7. **Render popovers:** Add `<Show when={tagPosition() !== null}><TagMentionPopover .../></Show>` and `<Show when={skillPosition() !== null}><SkillMentionPopover .../></Show>` at the bottom of the JSX (next to the existing `FileMentionPopover`).
  8. **Enter key guard:** In `handleKeyDown`, also prevent sending when tag or skill popover is open (similar to `mentionPosition()` guard).
- **Why:** Core integration — wires the 2-step flow into the textarea interaction
- **Constraints:** Do NOT modify the `@`-mention logic. Add tag detection AFTER the `@` block in `onInput`. The `<` detection should NOT fire if `@` mention is active (check `mentionPosition()` first). The popovers must not interfere with each other — only one popover type is visible at a time.

### What does NOT change

- `src/components/FileMentionPopover.tsx` — no changes, the `@` mention remains unchanged
- `src/lib/ipc.ts` — `listSkills()` already exists; no backend changes needed
- `src-tauri/` — no Rust changes; the backend already serves all needed data
- `src/components/Icon.tsx` — existing icons are sufficient; if a new icon name is needed, add to the `IconName` type

## Verification Plan

1. **Type `<` → popover appears:** Open chat, type `<`, verify `TagMentionPopover` renders at the correct caret position with "skill", "agent" (dimmed), "prompt" (dimmed) options.
2. **Type `<s` → fuzzy filter:** Type `<s`, verify only "skill" is shown (fuzzy match).
3. **Arrow + Enter select tag:** Arrow down to "skill", press Enter, verify `<skill>` is inserted in textarea, `TagMentionPopover` closes, `SkillMentionPopover` opens.
4. **Skill popover loads:** Verify skills appear with name + description. Verify loading state briefly visible then replaced with skill list.
5. **Type after `<skill>` → fuzzy search:** Type "subt" in `<skill>subt`, verify fuzzy filter matches skills like "add-subtitles" (name match) and skills mentioning subtitles in description.
6. **Arrow + Enter select skill:** Arrow to a skill, Enter, verify `</skill>` is appended and cursor is between the tags (e.g., `<skill>add-subtitles|</skill>` with `|` being the cursor).
7. **Escape closing:** At any popover step, press Escape and verify popovers close, cursor remains.
8. **Click outside closing:** Click outside a popover, verify it closes.
9. **Enter send guard:** With a popover open, press Enter — verify it selects the highlighted item (or does nothing if no results), does NOT send the message.
10. **`@` mention still works:** Type `@`, verify `FileMentionPopover` still opens and works as before. The `<` detection should not interfere.
11. **Backend fetch failure:** Mock `listSkills` to reject, verify skill popover shows error state gracefully.

## Risks

- **Low:** Cursor position math for `<skill>` insertion edge cases (trailing spaces, multi-line). Mitigation: test with `getCaretCoordinates` and careful string slicing.
- **Low:** Race condition if user types fast while `listSkills()` is still loading. Mitigation: loading state prevents interaction during fetch.
- **Low:** Popover overflow at viewport edges. Mitigation: reuse email logic from `FileMentionPopover` for clamping left/right, and the existing top/below flip logic.

## Tasks Summary

1. Create `TagMentionPopover` component (tag type picker)
2. Create `SkillMentionPopover` component (skill list with descriptions)
3. Integrate both popovers + 2-step flow into `ChatPanel.tsx`
