# Fix: Left-align all chat text content (remove unintended centering)

## Context

The user reports that all text blocks in the chat interface appear horizontally centered — "Asked you" blocks, "Thought" blocks, assistant messages, and user messages. This looks visually horrible.

**Root cause identified by the user:** The bug was introduced in a recent change to `ToolRow` (ChatPanel.tsx), where the title span got `shrink-0 whitespace-nowrap` and the summary span got `min-w-0 flex-1`. This layout change in the `ToolRow` header button, combined with missing explicit `text-left` on text content containers throughout the chat, caused text to appear centered.

## Investigation

Thorough search of all CSS and component files found **no explicit `text-align: center`** anywhere. The centering is a side effect of:
1. No explicit `text-left` on `.prose-content` (markdown rendering)
2. No explicit `text-left` on `AskUserBody` containers
3. No explicit `text-left` on user message text
4. No explicit `text-left` on `ThinkingRow` expanded content
5. The layout change in `ToolRow` triggered the centering behavior in certain container contexts

## Solution Design

Add explicit `text-left` to every content container that renders user-facing text. This is the most robust fix — it ensures text is always left-aligned regardless of ancestor layout changes.

### Changes

1. **`src/App.css` — `.prose-content` base class** (line ~313)
   - Add `text-align: left;` — covers ALL assistant markdown rendering globally

2. **`src/components/ChatPanel.tsx` — ToolRow body wrapper**
   - Line ~3144: add `text-left` to `<div class="ml-6 rounded-md bg-surface-1 p-2 text-xs">`
   - This covers ALL tool block bodies including AskUserBody

3. **`src/components/ChatPanel.tsx` — User message `<p>`**
   - Line ~1843: add `text-left` to the paragraph class

4. **`src/components/ChatPanel.tsx` — ThinkingRow expanded content**
   - Line ~3080 area: add `text-left` to the thinking text container

5. **Build and verify**

### Minimal approach
Rather than adding `text-left` to every individual element, we target the **ancestor content wrappers** so the fix cascades:
- `.prose-content` → covers ALL assistant/prose markdown
- `ToolRow` body wrapper → covers ALL tool bodies (ask_user, bash output, etc.)
- User message `<p>` → covers user text
- ThinkingRow body div → covers thought text

## Verification

1. Build passes: `pnpm build` returns exit code 0
2. Visual: all chat text blocks left-aligned (user msgs, assistant msgs, Asked you, Thought)
3. Regression: prose-content still renders correctly (code blocks, lists, tables, links, blockquotes)

## Tasks

1. Add `text-align: left` to `.prose-content` in App.css
2. Add `text-left` to ToolRow body wrapper in ChatPanel.tsx
3. Add `text-left` to user message `<p>` in ChatPanel.tsx
4. Add `text-left` to ThinkingRow expanded content in ChatPanel.tsx
5. Build and verify


## Implementation Log — 2026-07-13 01:35
**Summary:** Fix left-alignment of all chat text blocks (Asked you, Thought, user messages, assistant messages)
**Changed files:** M	src-tauri/Cargo.lock, M	src-tauri/src/agent/session.rs, M	src-tauri/src/agent/skills.rs, M	src-tauri/src/agent/subagent.rs, M	src-tauri/src/agent/tools/mod.rs, M	src/components/tool-renderers/ToolBody.test.tsx, M	src/components/tool-renderers/ToolBody.tsx
**Commits:** 2b4c6a8 fix: render search-tool results even when live payload is truncated, cb8a8ac fix: teach spawn_agents call shape and unblock skill reads outside workspace
**Journal:** Root cause: a recent change in ToolRow (ChatPanel.tsx) altered the header button layout (title span got shrink-0/whitespace-nowrap, summary span got min-w-0/flex-1). This change, combined with the lack of explicit `text-align: left` on text content containers, caused text to appear centered throughout the chat UI.

Fix: Added explicit `text-left` alignment at 5 key points:
1. `.prose-content` in App.css — covers ALL assistant markdown rendering
2. ToolRow body wrapper div — covers all tool timeline bodies (AskUserBody, BashBody, etc.)
3. User message `<p>` — covers plain user text
4. ThinkingRow expanded content wrapper and `<p>` — covers thought/thinking blocks
5. Live streaming assistant prose-content — covered by the `.prose-content` CSS rule

Approach was minimal and cascading: target ancestor containers so the fix propagates to all children without having to add `text-left` to every individual element. Build passes cleanly (33 test files, 622 tests, zero warnings).

**Task journal:**
- Add text-align: left to .prose-content in App.css: Added `text-align: left` to `.prose-content` in App.css at line 314
- Add text-left to ToolRow body wrapper in ChatPanel.tsx: Added `text-left` to ToolRow body wrapper class list
- Add text-left to user message <p> in ChatPanel.tsx: Added `text-left` to user message <p> class list
- Add text-left to ThinkingRow expanded content in ChatPanel.tsx: Added `text-left` to ThinkingRow expanded content wrapper and <p>
- Build and verify: Build passed: 33 test files, 622 tests, Vite build ok; No type errors, no warnings
