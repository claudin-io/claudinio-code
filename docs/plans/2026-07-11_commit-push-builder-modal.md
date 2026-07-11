# Commit & Push — Builder Modal

## Context / Problem Statement

Currently, when the user clicks "Commit & Push" in the GitChangesModal, the flow:
1. Closes the modal
2. Creates a new chat session via `newSession`
3. Sends the message "Please commit and push all changes" as a user message in the chat
4. The agent processes it through the normal chat flow — requiring manual approval for `git commit` and `git push` tool calls

This forces the commit/push operation through the main chat, polluting the conversation and requiring manual approval steps. The user wants:
- A dedicated modal that shows the builder working on commit & push in real-time
- No interaction with the main chat
- A cancel button to interrupt if desired
- Auto-approval for git commands (the "Commit & Push" button click IS the authorization)

## Goal (Definition of Done)

When the user clicks "Commit & Push":
1. A dedicated modal opens showing the builder's progress in real-time (timeline with thinking, tool calls, results)
2. The builder auto-generates a commit message based on the project's git log patterns
3. All git commands (add, commit, push) are auto-approved — no manual approval needed
4. A cancel button interrupts the process immediately, leaving whatever was already done in git state
5. The main chat is unaffected — no messages are added to the chat history

## Key Findings (Prova Real)

| Finding | Method | Proof |
|---------|--------|-------|
| `handleCommitPush` is in `ChatPanel.tsx` lines 2110-2128 | code-search + read_file | Closes modal → newSession → sendMessage |
| `sendMessage` creates Channel<AgentEvent> and streams events | `src/lib/ipc.ts` | Channel-based streaming via Tauri invoke |
| `interruptSession` sets `AtomicBool` shared by parent + all subagents | `session.rs:233`, `commands/agent.rs:857` | One flag stops everything |
| SubagentModal pattern exists at ChatPanel.tsx lines 2544-2629 | File read | Full timeline viewer with expandable steps |
| Existing SubagentModal has NO cancel button | SubagentModal code (lines 2544-2629) | Only X button, ESC, and backdrop to close |
| Git permissions: status/diff/log/branch/remote = auto; commit/push = requires approval | `permissions.rs:19-25, 151-153` | Security allowlist |
| Agent events include: TextStep, Thinking, ToolCall, ToolResult, Done, Error | `ipc.ts` AgentEvent union type | Discriminated union with 14 variants |
| `TimelineSteps` component already renders steps with expand/collapse | `ChatPanel.tsx` | Reusable for the modal |
| The builder already generates commit messages from git log | Existing agent behavior | Agent uses bash to run `git log` then crafts message |
| Tauri `Channel<T>` is the streaming bridge Rust→Frontend | `ipc.ts` multiple uses | Event-driven push model |

## Authoritative Inputs

| Input | Source | Value |
|-------|--------|-------|
| Auto-approve git commands | User confirmed | The "Commit & Push" click IS the authorization |
| Modal display level | User confirmed | Full timeline like SubagentModal (thinking, tool calls, results, status) |
| Cancel behavior | User confirmed | Interrupt immediately, leave git state as-is |
| Commit message | User confirmed | Builder runs `git log` to detect pattern, auto-generates message |
| "Commit & Push" localization key | `en-US.ts:84` | `"git.commitPush": "Commit & Push"` |
| Auto-commit message key | `en-US.ts` | `"git.autoCommitMessage": "Please commit and push all changes"` |

## Changes (Steps)

### Step 1: Backend — New `commit_and_push` Tauri command

**Target:** `src-tauri/src/commands/agent.rs`

**Mutation:** Add a new `#[tauri::command]` function `commit_and_push` that:
- Takes `workspace: String` and `event_channel: Channel<AgentEvent>`
- Creates a new session for the workspace (same as `send_message` but without persisting to chat history, or uses a special session type)
- Builds a `ToolContext` with `auto_approve_git: true` flag
- Sends the instruction "Please commit and push all changes. First run `git log` to understand the commit message pattern, then stage, commit with an appropriate message, and push." to the agent
- Spawns `run_workflow` in `tokio::spawn`
- Returns `{ session_id: String }`

**Why:** Dedicated entry point that doesn't touch the chat session.

**Constraints:** Reuse existing `run_workflow` infrastructure. Only add what's different.

### Step 2: Backend — Auto-approve git commands in permission system

**Target:** `src-tauri/src/agent/permissions.rs` and `src-tauri/src/agent/session.rs` (or `src-tauri/src/agent/tools/mod.rs`)

**Mutation:** Add a mechanism so that when `auto_approve_git` is set in the ToolContext:
- `git add`, `git commit`, `git push` commands are treated as `Auto` instead of `RequiresApproval`
- All other commands retain their normal permission level

**Why:** The user's "Commit & Push" click is the authorization.

**Constraints:** Thread the flag safely through ToolContext (already Clone). Don't relax permissions globally.

### Step 3: Backend — Register the new command

**Target:** `src-tauri/src/lib.rs`

**Mutation:** Register `commit_and_push` in the Tauri command handlers.

**Why:** Make the command available to the frontend.

### Step 4: Frontend — IPC binding for `commit_and_push`

**Target:** `src/lib/ipc.ts`

**Mutation:** Add:
```typescript
export function commitAndPush(
  workspace: string,
  onEvent: (event: AgentEvent) => void,
): Promise<{ sessionId: string }>
```
Same pattern as `sendMessage` — creates a Channel<AgentEvent>, sets onmessage, invokes `"commit_and_push"`.

**Why:** Frontend needs to call the new backend command and receive streaming events.

### Step 5: Frontend — `CommitPushModal` component

**Target:** `src/components/CommitPushModal.tsx` (new file)

**Mutation:** Create a modal component that:
- Props: `workspace: string`, `open: boolean`, `onClose: () => void`, `onComplete: () => void`
- On mount, calls `commitAndPush(workspace, handleEvent)` and stores the `sessionId`
- Renders a timeline using `TimelineSteps` (reuse from ChatPanel)
- Shows a "Cancel" button that calls `interruptSession(sessionId)`
- Shows status badge (running/completed/failed/interrupted) matching SubagentModal styling
- Closes on completion or cancel
- Handles ESC to cancel/interrupt
- On cancel: calls interrupt, shows "interrupted" status briefly, then closes

**Why:** The core new UI component for this feature.

**Constraints:** Reuse existing `TimelineSteps`, `SubagentModal` styling patterns, and `SubagentTimelineState` structure. Extract `TimelineSteps` into its own file if needed for clean reuse.

### Step 6: Frontend — Modify `handleCommitPush` in ChatPanel

**Target:** `src/components/ChatPanel.tsx`

**Mutation:** Change the `onCommitPush` callback (lines ~2110-2128) to:
- Set `showCommitPushModal(true)` instead of creating a new session and sending a chat message
- The GitChangesModal closes (current behavior stays)
- `CommitPushModal` opens instead

Add `showCommitPushModal` signal and render `<CommitPushModal>` (similar to how `<SubagentModal>` is rendered with `<Show when={...}>`).

**Why:** Divert the flow from chat to the dedicated modal.

### Step 7: Frontend — Localization strings

**Target:** `src/lib/locales/en-US.ts` and `src/lib/locales/pt-BR.ts`

**Mutation:** Add any new localization keys needed:
- `commitPush.modalTitle`: "Commit & Push"
- `commitPush.cancel`: "Cancel"
- `commitPush.completed`: "Completed"
- `commitPush.failed`: "Failed"
- `commitPush.interrupted`: "Interrupted"
- `commitPush.committing`: "Committing changes..."

**Why:** i18n support for the new modal.

## Verification Plan

### Dry-run / Build check
1. `cargo check` in `src-tauri/` — Rust compiles without errors
2. `pnpm build` or `npx tsc --noEmit` in `src/` — TypeScript compiles without errors

### Apply
3. `pnpm tauri dev` — app launches successfully

### End-to-end
4. Make a change to any file in a git workspace
5. Click "Commit & Push" in the GitChangesModal
6. **Verify:** Modal opens showing the builder timeline
7. **Verify:** Builder runs `git log`, stages files, creates a commit, pushes
8. **Verify:** No messages appear in the main chat
9. **Verify:** Modal shows "Completed" and closes (or stays with success state)

### Cancel
10. Make another change, click "Commit & Push"
11. Click "Cancel" during the process
12. **Verify:** Process stops immediately
13. **Verify:** Modal shows "Interrupted"
14. **Verify:** git state is preserved (if commit was done, it stays; if not, files remain unstaged)

### Regression
15. Normal chat flow — send a message, verify agent still works normally
16. Git permissions unchanged for normal chat — `git commit` still requires approval in normal chat

### Edge cases
17. Click "Commit & Push" with no changes — button should be disabled (existing behavior)
18. Click "Commit & Push" without a remote — push fails gracefully, commit still succeeds
19. Close modal via X/ESC during process — same as cancel

## Tasks Summary

1. Backend: Add `auto_approve_git` flag to ToolContext and permission system
2. Backend: Implement `commit_and_push` Tauri command
3. Backend: Register `commit_and_push` in lib.rs
4. Frontend: Add `commitAndPush` IPC binding
5. Frontend: Extract `TimelineSteps` into reusable component (if currently inline in ChatPanel)
6. Frontend: Create `CommitPushModal` component
7. Frontend: Modify `handleCommitPush` in ChatPanel to use modal
8. Frontend: Add localization strings
9. Integration testing and polish


## Implementation Log — 2026-07-11 11:03
**Summary:** Commit & Push now opens a dedicated builder modal instead of using the main chat, with auto-approval for git commands and a cancel button.
**Changed files:** M src-tauri/src/agent/permissions.rs, M src-tauri/src/agent/session.rs, M src-tauri/src/agent/tools/bash.rs, M src-tauri/src/agent/tools/finalize_plan.rs, M src-tauri/src/agent/tools/mod.rs, M src-tauri/src/commands/agent.rs, M src-tauri/src/lib.rs, M src/components/ChatPanel.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-11_commit-push-builder-modal.md, ?? src/components/CommitPushModal.tsx
**Commits:** _(git unavailable or none)_
**Journal:** Key decisions and learnings:

1. **auto_approve_git**: Added as a field on `ToolContext`, wired down to `bash_permission()` in permissions.rs. Only activates for `git add`, `git commit`, `git push` — perfectly targeted. Normal chat sessions always pass `false`, so their security is unaffected.

2. **commit_and_push command**: Creates a FRESH session (new UUID + SessionStore) without touching `ws.active_session`. This is the key architectural decision: the main chat history is NEVER polluted. The session JSONL file is written to disk for the agent's history but never loaded by the UI.

3. **Steering registration**: The command explicitly registers the SteeringCtl in `state.steering` so `interrupt_session()` can find it. Cleanup on completion removes the entry. This was critical — without it, the Cancel button would show "session not running".

4. **CommitPushModal**: Uses its own lightweight timeline state manager rather than the heavy SubagentTimeline machinery from ChatPanel. This keeps the component self-contained and avoids extraction of TimelineSteps which would have been a large refactor.

5. **Approval flow**: The YOLO-mode override path in `session.rs::run_tool()` was the model for implementating the auto-approve path. The bash tool at the "RequiresApproval" level re-checks `bash_permission` — and now passes `ctx.auto_approve_git`, so git commands get auto-approved inline.

6. **Error handling**: Removed unused `cancelDone`/`setCancelDone` signals that caused a TypeScript error. The modal's auto-close (1500ms after completion) gives the user time to see the final status before closing.

Files changed: 12 (10 modified, 2 new: CommitPushModal.tsx + plan file)

**Task journal:**
- Backend: auto_approve_git flag in permission system: Added auto_approve_git: bool to ToolContext in tools/mod.rs; Modified bash_permission in permissions.rs with new auto_approve_git flag param; Wired it through session.rs at all 3 bash_permission call sites; All 21 permission tests pass (including 7 new tests for auto_approve_git)
- Backend: commit_and_push Tauri command: Added commit_and_push Tauri command (creates FRESH session — NOT attached to ws.active_session); Uses auto_approve_git: true, registers steering controller so interrupt_session works; Spawns tokio::spawn calling run_workflow, cleans up steering map on completion; Returns SessionStarted { session_id }
- Backend: register commit_and_push command: Added commands::agent::commit_and_push to generate_handler! macro
- Frontend: commitAndPush IPC binding: Added commitAndPush function after sendMessage (line 265) using Tauri Channel pattern
- Frontend: CommitPushModal component: Created CommitPushModal.tsx with local timeline state; Handles: Thinking, ToolCall, ToolResult, TextStep, Done, Error events; Cancel button calls interruptSession, shows interrupted status, closes after 800ms; Auto-closes 1500ms after completion/failure; ESC and backdrop click both trigger cancellation; Uses t() for all user-facing strings with i18n keys
- Frontend: modify handleCommitPush in ChatPanel: Added showCommitPushModal signal; Replaced the onCommitPush callback (8 lines) with simple setShowCommitPushModal(true); Removed all old session creation + sendMessage code from the callback; Added CommitPushModal rendering right after GitChangesModal
- Frontend: localization strings: Added English keys to en-US.ts; Added Portuguese keys to pt-BR.ts; Both inserted after git section, before ChatPanel - Status
- Integration testing & verification: cargo check: passes (0 warnings); cargo test agent::permissions: all 21 tests pass; npx tsc --noEmit: only pre-existing test errors, no errors from our files; commit_and_push command registered and compiling; commitAndPush IPC binding in place; CommitPushModal created and compiling; handleCommitPush rewired to use modal; i18n keys added for both en-US and pt-BR
