# Plan: "Continuar com Builder" Button After Brain Mode Completion

## 1. Context / Problem Statement

When the user manually toggles **Brain mode** (origin = `human`) and the agent finishes the planning phase (writes the plan, creates tasks, calls `write_plan` + `tasks_set`), the agent **cannot exit Brain mode on its own** — the backend enforces that only the human toggle can switch back to Builder. The agent's final message says something like "the plan and tasks are ready for you to flip the toggle."

The problem: the user then needs to locate the small Brain/Builder toggle in the chat input area and click Builder manually, which is not obvious or convenient. They want a prominent button that appears automatically when the plan is done.

**User request:** "Quando o modo brain acaba, poderiamos colocar um botão embaixo dizendo 'Continuar com Builder' e se apertar ele continua com o modo builder a implementação do plano."

**Confirmed via interview:**
- Button appears **below the last assistant message**, only when `mode === "brain"`, `modeOrigin === "human"`, and `status === "done"`.
- Clicking it: switches to Builder mode **AND** auto-sends a message like "Execute the plan" to start implementation.
- Button text: `"Continuar com Builder"` (PT) / `"Continue with Builder"` (EN).

## 2. Goal (Definition of Done)

After the agent in human-initiated Brain mode finishes its turn (status=`done`), a styled action button appears below the assistant's last message. Clicking it switches the mode to Builder and automatically dispatches a user message to trigger plan execution — the user does not need to touch the toggle or type anything.

## 3. Key Findings (Prova Real)

| # | Finding | Method | Proof |
|---|---|---|---|
| 1 | `modeOrigin` is NOT tracked as a signal in ChatPanel.tsx — only `mode` is | `grep` for `ModeOrigin` in ChatPanel.tsx | Only 1 match: literal `"human" as const` in `switchMode()` (line 478) |
| 2 | `ModeChangedData` already carries `origin: ModeOrigin` from the backend | `read_file ipc.ts:113-117` | `export interface ModeChangedData { mode: SessionMode; origin: ModeOrigin; reason?: string \| null; }` |
| 3 | `handle_mode_switch("exit_plan_mode")` in Rust (session.rs:2128-2152) rejects exit when origin != Agent with: *"The USER enabled Brain mode — only they can switch back to Builder."* | `read_file session.rs:2128-2170` | Hard block in backend; only human toggle can exit human-origin Brain |
| 4 | ChatPanel's `send()` function reads from `input()` textarea signal — we need a separate path for auto-send | `read_file ChatPanel.tsx:1137-1188` | `send()` uses `input().trim()` on line 1138 |
| 5 | The `sendMessage()` IPC call is a raw function call inside `send()` on line 1176 — we can call it directly with a programmatic message | `read_file ChatPanel.tsx:1176` | `const result = await sendMessage(props.workspace, text, [...], handleEvent, mode())` |
| 6 | Messages are rendered via `<For each={messages()}>` at line 1398; each message in `mb-6` div | `read_file ChatPanel.tsx:1398-1470` | Last assistant message is the last element in the For loop |
| 7 | The AgentEvent::ModeChanged is emitted by the backend when `set_session_mode` is called from the frontend | `read_file session.rs:2092-2100` | Emitted for both human and agent origin switches |

## 4. Authoritative Inputs

| Input | Value | Source |
|---|---|---|
| Button label (PT) | `"Continuar com Builder"` | Per user interview |
| Button label (EN) | `"Continue with Builder"` | Per user interview |
| Auto-send message (PT) | `"Executar o plano"` | Inferred (consistent design) |
| Auto-send message (EN) | `"Execute the plan"` | Inferred (consistent design) |
| Visibility condition | `mode === "brain" && modeOrigin === "human" && status === "done"` | Per user interview |
| Placement | Below the last assistant message bubble | Per user interview |

## 5. Changes (Steps)

### Step 1: Add `modeOrigin` signal tracking in ChatPanel.tsx

- **Target:** `/Users/victortavernari/claudinio_code/src/components/ChatPanel.tsx`
- **Mutation:** Add `const [modeOrigin, setModeOrigin] = createSignal<ModeOrigin>("human")` near the existing `mode` signal (line ~470). Update it in the `ModeChanged` event handler (line ~952-957) to also call `setModeOrigin(data.origin)`. On session reopen (line ~1279), read the last Mode record's origin and set it. Import `ModeOrigin` from `ipc.ts`.
- **Why:** Currently the frontend doesn't track who initiated the Brain mode, which is needed to decide whether to show the button.
- **Constraints:** Must default to `"human"` for safety. Must survive hot-reload of session.

### Step 2: Add `continueWithBuilder()` function

- **Target:** `/Users/victortavernari/claudinio_code/src/components/ChatPanel.tsx`
- **Mutation:** Create async function that:
  1. Calls `switchMode("builder")` — this updates the local `mode` signal, adds a timeline step, and calls `setSessionMode` backend.
  2. Adds a user message `{ role: "user", text: t("mode.continueMessage") }` to messages.
  3. Sets status to `"thinking"`, clears steps, scrolls to bottom.
  4. Calls `sendMessage(props.workspace, t("mode.continueMessage"), [], handleEvent, "builder")`.
  5. Uses the result to set `activeSessionId`.
- **Why:** We need a separate send path because the normal `send()` reads from the textarea input signal which is empty. This auto-dispatches the plan execution.
- **Constraints:** Must handle errors gracefully. Must set status correctly so the UI shows thinking state.

### Step 3: Render the button in the message list

- **Target:** `/Users/victortavernari/claudinio_code/src/components/ChatPanel.tsx`
- **Mutation:** After the `</For>` that renders messages (after line ~1470), add a `<Show when={showContinueWithBuilder()}>` block that renders a styled button. The signal `showContinueWithBuilder()` returns `mode() === "brain" && modeOrigin() === "human" && status() === "done"`.
- **Button styling:** Follow the app's design system. A pill-shaped button with accent color, icon + text. Something like:
  ```tsx
  <div class="mb-6 flex justify-center">
    <button
      onClick={continueWithBuilder}
      class="inline-flex items-center gap-2 rounded-full bg-accent px-5 py-2.5 text-sm font-semibold text-accent-ink shadow-lg shadow-accent/20 transition-all hover:bg-accent/90 hover:shadow-xl hover:shadow-accent/30 active:scale-[0.98]"
    >
      <Icon name="construction-worker" class="h-4 w-4" />
      {t("mode.continueWithBuilder")}
    </button>
  </div>
  ```
- **Why:** This is the core UX deliverable. Centered below messages so it's unmissable.
- **Constraints:** Must disappear once clicked (status becomes "thinking"), must not show during active conversation.

### Step 4: Add i18n keys

- **Target:** `/Users/victortavernari/claudinio_code/src/lib/locales/en-US.ts` and `pt-BR.ts`
- **Mutation:** Add to both files in the "Session mode" section:
  - `"mode.continueWithBuilder"`: `"Continue with Builder"` (EN) / `"Continuar com Builder"` (PT)
  - `"mode.continueMessage"`: `"Execute the plan"` (EN) / `"Executar o plano"` (PT)
- **Why:** All user-facing strings go through the i18n system in this codebase.
- **Constraints:** Follow existing key naming convention (`mode.*` prefix).

## 6. Verification Plan

### 6.1. Unit / build check
```bash
pnpm run build   # or pnpm run typecheck — ensure no TS errors
```

### 6.2. Visual verification (manual — UI)
1. Start the app, enter Brain mode (human toggle).
2. Send a planning request (e.g., "I want to add a dark mode toggle").
3. Wait for agent to finish (status = done).
4. **Assert:** A centered "Continue with Builder" button appears below the last assistant message.
5. Click the button.
6. **Assert:** Mode switches to Builder, a user message "Execute the plan" appears, and the agent starts executing.

### 6.3. Negative cases
- Button must NOT appear when status is "thinking" (agent still working).
- Button must NOT appear when mode is "builder".
- Button must NOT appear when modeOrigin is "agent" (agent entered Brain itself via enter_plan_mode).
- Button must disappear immediately after click (status becomes "thinking").

### 6.4. No regressions
- The existing Brain/Builder toggle in the input area continues working.
- The existing golden loop (agent-initiated mode flips) is unaffected.
- Session reopen correctly restores mode and modeOrigin.

## 7. Risks

| Risk | Mitigation |
|---|---|
| The auto-send message "Execute the plan" may not be enough context for the Builder agent | The agent reads the plan file from `.claudinio/plans/` via `tasks_get` first — confirmed in system prompt. Low risk. |
| User may want to edit the auto-send message | Out of scope for now; the message is hardcoded. Future enhancement could pre-fill the input instead of auto-sending. |
| If sendMessage fails, UI could get stuck | The existing error handling in `send()` catches errors and sets status to "error". We'll mirror that pattern. |

---

## Tasks Summary (for tasks_set)

1. **Add modeOrigin signal** — ChatPanel.tsx: create signal, update in ModeChanged handler, restore on session reopen
2. **Add continueWithBuilder() function** — ChatPanel.tsx: switchMode + auto-send
3. **Render the button** — ChatPanel.tsx: Show block after message loop
4. **Add i18n keys** — en-US.ts and pt-BR.ts: two new keys each
