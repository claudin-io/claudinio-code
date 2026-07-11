# Git Changes Modal — Full Git Management UI

## 1. Context / Problem Statement

The current GitChangesModal is a read-only diff viewer with a single "Commit & Push" button that delegates everything to an LLM agent. Users want full git control directly from the UI: stage/unstage files and hunks, discard changes, edit files, and commit with awareness of what is staged vs unstaged.

## 2. Goal (Definition of Done)

A redesigned GitChangesModal with a two-panel layout (files + commit) that allows:
- **Stage/unstage** individual files or specific diff hunks
- **Discard** file changes with confirmation
- **Open file in FileEditorModal** from the git changes list
- **Commit** via the existing LLM agent flow, with the choice of "all changes" or "staged only"
- **Auto-resolve push conflicts** with `git pull --rebase` + retry

All new backend commands registered, all i18n keys added for both en-US and pt-BR.

## 3. Key Findings (Prova Real)

| Finding | Source | Method |
|---|---|---|
| `GitChangesModal` is 60vw, full list of files with expandable diffs, no stage/discard/edit buttons | `src/components/GitChangesModal.tsx` | `read_file` |
| Backend has only 3 git commands: `git_status`, `git_file_diff`, `git_branch` | `src-tauri/src/commands/git.rs` | `read_file` |
| `commit_and_push` creates agent session with `auto_approve_git: true` and sends instruction "Please commit and push all changes..." | `src-tauri/src/commands/agent.rs:942-965` | `read_file` |
| `CommitPushModal` takes `workspace`, `open`, `onClose` props — no `stagedOnly` param | `src/components/CommitPushModal.tsx` | `read_file` |
| `FileEditorModal` takes `filePath`, `rootPath`, `onClose` — reusable as-is | `src/components/FileEditorModal.tsx` | `read_file` |
| IPC bindings in `src/lib/ipc.ts` define `ChangedFile`, `GitStatus`, `gitStatus()`, `gitFileDiff()`, `gitBranch()`, `commitAndPush()` | `src/lib/ipc.ts` | `read_file` |
| 11 git i18n keys in each locale file (`pt-BR.ts:80-91`, `en-US.ts:80-91`) | `src/lib/locales/` | `grep` |
| ChatPanel wires GitChangesModal + CommitPushModal at lines 2333-2351 | `src/components/ChatPanel.tsx` | `read_file` |
| New git commands need registration in `src-tauri/src/lib.rs` lines 36-38 | `src-tauri/src/lib.rs` | `read_file` |
| Permission system in `permissions.rs` auto-approves `git add/commit/push` when `auto_approve_git=true` | `src-tauri/src/agent/permissions.rs` | `read_file` |

## 4. Authoritative Inputs

| Input | Value | Source |
|---|---|---|
| Hunk stage approach | Parse hunks in frontend, `git apply --cached` in backend | Per user decision |
| Discard approach | `git checkout -- <file>` for unstaged, `git reset HEAD <file>` for staged, with confirmation dialog | Per user decision |
| Commit flow | Keep LLM agent, add "staged only" vs "all" choice | Per user decision |
| Push fail strategy | `git pull --rebase` then retry push | Per user decision |
| Layout dimensions | 75-80vw, split: left 55% (files), right 45% (commit panel) | Per user decision |
| Commit panel location | Integrated into right side of GitChangesModal; button opens CommitPushModal | Per user decision |
| Modal height | 85vh (same as current CommitPushModal) | Existing pattern |
| FileEditorModal props | `filePath`, `rootPath`, `onClose` | Existing component API |
| `commitAndPush` signature | `(workspace: string, onEvent: (AgentEvent) => void) => Promise<{sessionId: string}>` | `src/lib/ipc.ts` |

## 5. Changes (Steps)

### Backend — New Git Commands

**Step 1: Add new Tauri commands to `src-tauri/src/commands/git.rs`**

Add these commands to the existing file (after `git_branch`):

1. `git_stage_file(workspace, path)` — runs `git add -- <path>`
2. `git_unstage_file(workspace, path)` — runs `git reset HEAD -- <path>`
3. `git_discard_file(workspace, path)` — runs `git checkout -- <path>` for unstaged + `git reset HEAD <path>` for staged; for untracked (`??`) files: deletes the file from disk via `std::fs::remove_file`
4. `git_stage_hunk(workspace, path, hunk_text)` — constructs a proper patch file (diff header + hunk text), writes to temp file, runs `git apply --cached <tempfile>`. The `hunk_text` is the raw text from `@@` line to end of hunk.
5. `git_unstage_hunk(workspace, path, hunk_text)` — same as stage_hunk but with `git apply --cached --reverse`
6. `git_stage_all(workspace)` — runs `git add -A`
7. `git_unstage_all(workspace)` — runs `git reset HEAD`

Also modify `ChangedFile` struct to include a `staged: bool` field. Update `git_status` to detect staged files via `git diff --cached --name-only` and mark them.

Modify `git_file_diff` to accept an optional `staged: bool` parameter (default `false`): when `true`, returns `git diff --cached -- <path>`.

**Step 2: Register new commands in `src-tauri/src/lib.rs`**

Add the 7 new command names to the `invoke_handler` list (after existing `git_branch`).

**Step 3: Modify `commit_and_push` in `src-tauri/src/commands/agent.rs`**

Add a `staged_only: bool` parameter. When `true`, change the agent instruction from "Please commit and push all changes" to "Please commit the currently staged changes and push. Do NOT run git add. If git push fails with 'non-fast-forward', run git pull --rebase and then git push again."

Also add the push-fail auto-resolution instruction to the default (all changes) flow.

**Step 4: Extend bash permissions in `src-tauri/src/agent/permissions.rs`**

Add `git pull --rebase` to the auto-approve list when `auto_approve_git=true` (already covered by the starts-with matching). Verify existing pattern covers `git pull`.

### Frontend — IPC Bindings

**Step 5: Add TypeScript bindings in `src/lib/ipc.ts`**

Add IPC functions:
```ts
gitStageFile(workspace: string, path: string): Promise<void>
gitUnstageFile(workspace: string, path: string): Promise<void>
gitDiscardFile(workspace: string, path: string): Promise<void>
gitStageHunk(workspace: string, path: string, hunkText: string): Promise<void>
gitUnstageHunk(workspace: string, path: string, hunkText: string): Promise<void>
gitStageAll(workspace: string): Promise<void>
gitUnstageAll(workspace: string): Promise<void>
```

Modify `gitFileDiff` to accept optional `staged` param:
```ts
gitFileDiff(workspace: string, path: string, staged?: boolean): Promise<string>
```

Modify `commitAndPush` to accept `stagedOnly`:
```ts
commitAndPush(workspace: string, onEvent: (AgentEvent) => void, stagedOnly?: boolean): Promise<{sessionId: string}>
```

### Frontend — GitChangesModal Redesign

**Step 6: Rewrite `src/components/GitChangesModal.tsx`**

Full redesign with two-panel layout:

**Left Panel (55% width) — File Changes:**
- Header: "Changes" with file count, "Stage All" / "Unstage All" buttons
- File list grouped by status (staged vs unstaged sections, or mixed)
- Each file row has:
  - Checkbox or status indicator (staged vs unstaged)
  - Expand/collapse chevron
  - File path (mono font, truncate)
  - +/- counts
  - Action buttons: Stage/Unstage (icon button), Discard (icon button), Edit (icon button)
- Discard shows a confirmation dialog before executing
- Edit opens `FileEditorModal` with the file
- When expanded, file shows its diff parsed into hunks
- Each hunk has a stage/unstage button
- Hunk diff rendered with the existing `renderDiff` syntax highlighting

**Right Panel (45% width) — Commit Panel:**
- Header: "Commit" 
- Summary of staged files (count, total +/-)
- List of staged file names (compact)
- Two buttons:
  - "Commit All" — opens CommitPushModal with `stagedOnly=false`
  - "Commit Staged" — opens CommitPushModal with `stagedOnly=true`, disabled if no staged files
- Refresh button at bottom

**Hunk parsing logic (new function in the component):**
```ts
function parseHunks(diff: string): Array<{header: string, lines: string[]}> 
```
Splits unified diff text by `@@` headers. Each hunk starts at a `@@` line and continues to the next `@@` or EOF.

**State management additions:**
- `stagedFiles: Set<string>` — tracks which files have staged changes
- `stagedHunks: Set<string>` — tracks which hunks (by `path:@@header` key) are staged

**Data flow:**
- On modal open: fetch git_status (now includes staged info via `staged: bool` on ChangedFile)
- On file expand: fetch both staged and unstaged diffs for the file
- On stage/unstage action: call backend, then refresh status

**Step 7: Modify `src/components/CommitPushModal.tsx`**

Add `stagedOnly` prop (boolean, default `false`). Pass it to the `commitAndPush` IPC call.

Add visual indicator in the modal header showing "Committing staged changes" vs "Committing all changes".

**Step 8: Update `src/components/ChatPanel.tsx` wiring**

Add state for `commitStagedOnly` signal. When CommitPushModal opens from "Commit Staged", set it to `true`. Pass it as prop to `CommitPushModal`.

### Localization

**Step 9: Add i18n keys to `src/lib/locales/en-US.ts` and `src/lib/locales/pt-BR.ts`**

New keys needed:
| Key | en-US | pt-BR |
|---|---|---|
| `git.stage` | Stage | Stage |
| `git.unstage` | Unstage | Unstage |
| `git.discard` | Discard | Descartar |
| `git.edit` | Edit | Editar |
| `git.discardConfirm` | Discard changes to {0}? This cannot be undone. | Descartar alterações em {0}? Isto não pode ser desfeito. |
| `git.stageAll` | Stage All | Stage Tudo |
| `git.unstageAll` | Unstage All | Unstage Tudo |
| `git.commitAll` | Commit All | Commit Tudo |
| `git.commitStaged` | Commit Staged | Commit Staged |
| `git.staged` | Staged | Staged |
| `git.unstaged` | Unstaged | Não staged |
| `git.stagedCount` | {0} staged | {0} arquivos staged |
| `git.commitAllDesc` | Stage all changes and commit | Stage de tudo e commit |
| `git.commitStagedDesc` | Commit only staged changes | Commit apenas do que está staged |
| `git.noStaged` | No staged changes | Nenhum arquivo staged |
| `git.pushFailRetry` | Push failed, pulling and retrying... | Push falhou, fazendo pull e tentando novamente... |

## 6. Risks

| Risk | Mitigation |
|---|---|
| Hunk patch construction may fail for edge cases (binary files, renames) | Graceful error handling — show error toast, suggest staging whole file instead |
| `git apply --cached --reverse` may fail if working tree doesn't match staged state | Catch errors, surface to user via toast |
| Discard for deleted files needs `git checkout -- <path>` not `rm` | Handle per-status logic in backend |
| `git pull --rebase` may have conflicts | The agent handles conflict resolution — if it fails, surface to user |

## 7. Tasks Summary

13 tasks total:
1. Backend: Add 7 new git commands + staged field to git.rs
2. Backend: Register commands in lib.rs
3. Backend: Modify commit_and_push for staged_only + pull-rebase
4. Backend: Verify git permissions cover pull --rebase
5. Frontend: Add IPC bindings for new commands
6. Frontend: Redesign GitChangesModal (left panel — file list with actions)
7. Frontend: Add hunk parsing and stage/unstage logic
8. Frontend: Build right panel (commit summary + buttons)
9. Frontend: Add FileEditorModal integration
10. Frontend: Modify CommitPushModal for stagedOnly prop
11. Frontend: Update ChatPanel wiring
12. i18n: Add new keys to en-US and pt-BR
13. Integration: End-to-end verification


## Implementation Log — 2026-07-11 14:57
**Summary:** Git Changes Modal: full git management with stage/unstage (file + hunk), discard, edit, staged-only commit, and auto pull-rebase on push fail
**Changed files:** M src-tauri/src/agent/permissions.rs, M src-tauri/src/commands/agent.rs, M src-tauri/src/commands/git.rs, M src-tauri/src/lib.rs, M src/components/ChatPanel.tsx, M src/components/CommitPushModal.tsx, M src/components/GitChangesModal.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-11_git-changes-full-management.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Key Findings & Decisions

1. **Backend git commands**: Added 7 new Tauri commands to `git.rs`. The hunk stage/unstage approach uses `git apply --cached` with a temp patch file — works well but the subagent initially didn't convert the `Result<String, String>` return to `Result<(), String>`. Fixed with `.map(|_| ())`.

2. **commit_and_push signature**: The subagent mistakenly added a `config` state param that doesn't exist in the project's crate. Fixed by removing it — the existing code already reads config from `state.config.lock().await`.

3. **Permissions**: Added `git pull` to the auto_approve_git section. The `git apply --cached` commands are not in the allowlist (they're used by the new hunk commands called directly, not by the agent). The agent only uses `git add/commit/push/pull` which are all covered.

4. **Two-panel layout**: GitChangesModal now uses 75vw width with 55/45 split (files vs commit panel). Staged files shown first, then unstaged. Each file row has stage/unstage/discard/edit buttons. FileEditorModal opens on edit click.

5. **Hunk UI**: `parseHunks()` splits diff at `@@` lines. Each hunk rendered with its own stage/unstage button. Tracked via `stagedHunks` Set with `path:@@header` keys.

6. **Build verification**: Both Rust (`cargo build`) and TypeScript (`pnpm build` = 395 tests + vite build) compile cleanly. All 22 permission tests pass including the new `auto_approve_git_pull_with_flag` test.

## Files Changed
- `src-tauri/src/commands/git.rs` — 7 new commands, staged field, modified git_status/git_file_diff
- `src-tauri/src/lib.rs` — registered 10 git commands
- `src-tauri/src/commands/agent.rs` — staged_only param on commit_and_push, conditional instruction
- `src-tauri/src/agent/permissions.rs` — git pull auto-approval + test
- `src/lib/ipc.ts` — 7 new IPC functions + modified gitFileDiff/commitAndPush
- `src/components/GitChangesModal.tsx` — complete rewrite: two-panel layout, file/hunk actions, commit panel
- `src/components/CommitPushModal.tsx` — stagedOnly prop + visual indicator
- `src/components/ChatPanel.tsx` — commitStagedOnly signal, updated wiring
- `src/lib/locales/en-US.ts` — 16 new git i18n keys
- `src/lib/locales/pt-BR.ts` — 16 new git i18n keys

**Task journal:**
- Backend: Add 7 git commands + staged field: Added 7 git commands to git.rs; Added staged: bool field to ChangedFile; Modified git_status to detect staged files; Modified git_file_diff to accept staged param; Registered all 10 git commands in lib.rs
- Backend: staged_only param + pull-rebase on push fail: Added staged_only param to commit_and_push; Conditional instruction: staged only vs all changes; Both paths include pull --rebase on push fail
- Backend: Verify git permissions cover new commands: Added git pull to auto_approve_git section; Added auto_approve_git_pull_with_flag test; 22 permissions tests pass
- Frontend: IPC bindings for 7 new git commands: Added 7 new git IPC functions; Modified gitFileDiff with staged param; Modified commitAndPush with stagedOnly param
- Frontend: Hunk parsing from unified diff: parseHunks function implemented inside GitChangesModal.tsx
- Frontend: Two-panel modal shell: Two-panel layout implemented: left 55% files, right 45% commit
- Frontend: File row actions (stage/unstage/discard/edit): FileRow with stage/unstage/discard/edit buttons; Staged/Unstaged sections with headers; Stage All / Unstage All buttons
- Frontend: Hunk-level stage/unstage buttons: Hunks parsed and rendered with stage/unstage buttons per hunk; stagedHunks Set tracks which hunks are staged
- Frontend: Commit panel with staged summary and buttons: Commit panel with staged file list, staged count, Commit Staged and Commit All buttons
- Frontend: CommitPushModal stagedOnly support: Added stagedOnly prop to CommitPushModal; Visual 'Staged Only' tag in header
- Frontend: ChatPanel wiring updates: ChatPanel wiring updated: commitStagedOnly signal, onCommitPush passes stagedOnly to CommitPushModal
- i18n: New git keys (en-US + pt-BR): Added 16 new i18n keys to en-US.ts; Added 16 new i18n keys to pt-BR.ts
- Verify: build + end-to-end git operations: cargo build: OK; pnpm build (395 tests + vite build): OK; cargo test permissions (22 tests): OK
