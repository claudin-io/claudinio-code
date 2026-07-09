# Auth Inline Card — Replace API Key Error with Authentication Button

## 1. Context / Problem Statement

**CONFIRMED by user:**
- When a user sends a message without authentication (no API key configured), the chat currently shows a raw error text: `"Failed to send: API key not configured. Use set_config first."` as a red chat bubble.
- This UX is unfriendly — it forces the user to discover the config modal, find the login button, sign in, then re-type their message.

**INFERRED from investigation:**
- The error originates in `src-tauri/src/commands/agent.rs:90-91` (Rust backend) — returns `Err("API key not configured. Use set_config first.")`.
- The frontend `ChatPanel.tsx:863-865` catches this in the `catch` block of `send()` and appends `t("chat.message.failedToSend", String(e))` as a user-role message.
- Same pattern at `ChatPanel.tsx:729` for compaction errors.
- The app already has OAuth login wired via `loginWithClaudinio()` in `ipc.ts` and `doLogin()` in `App.tsx`.
- `ChatPanel` currently does not import any auth functions and has no auth awareness.

## 2. Goal (Definition of Done)

Replace the "API key not configured" raw error text with an inline authentication card in the chat stream. The card shows a "Sign In" (OAuth) button. After successful login, the card removes itself and the original message is automatically re-sent.

Stretch: same treatment for `compact_session` auth error.

## 3. Key Findings (Prova Real)

| # | Finding | Method | Proof |
|---|---------|--------|-------|
| 1 | Error originates at `agent.rs:90-91` | `grep "API key not configured"` | `if cfg.api_key.is_empty() { return Err("API key not configured. Use set_config first.".into()); }` |
| 2 | Frontend catch at `ChatPanel.tsx:863` appends error as user message | `read_file ChatPanel.tsx:860-870` | `setMessages((prev) => [...prev, { role: "user", text: t("chat.message.failedToSend", String(e)) }]);` |
| 3 | `compactSession` has same guard at `agent.rs:555-556` | `grep compact_session agent.rs` | `if cfg.api_key.is_empty() { return Err("API key not configured.".into()); }` |
| 4 | `getConfig()` returns `hasApiKey: boolean` at `agent.rs:449` | `read_file agent.rs:441-460` | `"hasApiKey": !cfg.api_key.is_empty()` |
| 5 | `loginWithClaudinio()` already exported from `ipc.ts:320` | `grep loginWithClaudinio ipc.ts` | `export function loginWithClaudinio(): Promise<LoginResult>` |
| 6 | `ChatMessage` supports `role: "user" \| "assistant" \| "archived"` at `ChatPanel.tsx:68` | `file_outline ChatPanel.tsx` | `interface ChatMessage { role: "user" \| "assistant" \| "archived"; ... }` |

## 4. Authoritative Inputs

| Input | Source | Tagged use |
|-------|--------|------------|
| Error string `"API key not configured"` | `agent.rs:91` | String match for detecting auth error |
| `loginWithClaudinio()` from `ipc.ts:320` | per codebase | Auth button handler |
| `getConfig()` from `ipc.ts:307` | per codebase | Check auth state on mount / after login |
| `LoginResult { login, tier }` from `ipc.ts:316` | per codebase | Type returned by login |
| Locale files: `en-US.ts`, `pt-BR.ts` | per codebase | New i18n keys |
| `ChatMessage` interface at `ChatPanel.tsx:67` | per codebase | May need extension or we use special `text` content with special rendering |
| `t()` from `grill-me.ts` | per codebase | i18n function |

## 5. Changes (Steps)

### Step 1: Add new i18n keys `src/lib/locales/en-US.ts` + `src/lib/locales/pt-BR.ts`

**Target:** `src/lib/locales/en-US.ts` and `src/lib/locales/pt-BR.ts`
**Mutation:** Add new keys:
- `"chat.authCard.title"` → `"Sign in required"` / `"Autenticação necessária"`
- `"chat.authCard.description"` → `"Sign in to claudin.io to send messages."` / `"Faça login no claudin.io para enviar mensagens."`
- `"chat.authCard.signIn"` → `"Sign In"` / `"Entrar"`
- `"chat.authCard.signingIn"` → `"Signing in…"` / `"Entrando…"`
**Why:** Localized UX for the auth card.
**Constraints:** Follow existing pattern of key naming and placeholder usage.

### Step 2: Import auth functions in `src/components/ChatPanel.tsx`

**Target:** `src/components/ChatPanel.tsx`, import block (lines 2-33)
**Mutation:** Add imports for:
- `loginWithClaudinio` from `"../lib/ipc"`
- `getConfig` from `"../lib/ipc"`
**Why:** ChatPanel needs to be self-sufficient for auth awareness.
**Constraints:** Do not change existing imports, only add.

### Step 3: Add auth state signals to ChatPanel component

**Target:** `src/components/ChatPanel.tsx`, component body (after existing signals, ~line 410-420 area)
**Mutation:** Add:
```typescript
const [hasApiKey, setHasApiKey] = createSignal(false);
const [pendingMessage, setPendingMessage] = createSignal<string | null>(null);
```

Also call `getConfig()` on mount to initialize `hasApiKey` (add to existing `onMount` or create new one).

**Why:** Track auth state so we know when login succeeds, and store the blocked message for auto-resent.
**Constraints:** Follow SolidJS signal conventions used throughout the file.

### Step 4: Modify `send()` catch block to detect auth error and show card

**Target:** `src/components/ChatPanel.tsx`, `send()` function, catch block at line ~863-866
**Mutation:** Before appending error message, check if `String(e).includes("API key not configured")`:
- If YES: store the original message text in `pendingMessage()`, append a special auth-card message to messages array (using a marker in `text` or a new field) instead of the error text.
- If NO: keep existing behavior (append error text).
**Why:** Intercept auth errors and show the card instead.
**Constraints:** The auth card message should NOT trigger the error styling. Use `role: "user"` with a special marker prefix like `__auth_card__` that the rendering logic can detect.

### Step 5: Add auth card rendering in message list

**Target:** `src/components/ChatPanel.tsx`, message rendering section (~line 1280+, inside `<For each={messages()}>`)
**Mutation:** Before/within the existing user message rendering (line ~1280), add a `<Show>` block that checks if the message `text` starts with `__auth_card__`. If it does, render an auth card component instead of the normal user message bubble.
- Card: rounded container, light border, with text explaining auth is needed and a "Sign In" button.
- Card uses the OAuth flow only (no manual API key input).
- "Sign In" button calls `handleAuthSignIn()`.
- While signing in, button shows loading state (`signingIn` signal).
**Why:** Visual distinction — card feels like an action prompt, not an error.
**Constraints:** Match existing Tailwind design tokens (`bg-surface-1`, `border-border-subtle`, `text-ink-muted`, `text-accent`, etc.). Use `Icon` component for any icons.

### Step 6: Implement `handleAuthSignIn()` function

**Target:** `src/components/ChatPanel.tsx`, add new function in component body
**Mutation:** 
```typescript
const [authSigningIn, setAuthSigningIn] = createSignal(false);
const handleAuthSignIn = async () => {
  setAuthSigningIn(true);
  try {
    await loginWithClaudinio();
    // Remove auth card from messages
    setMessages((prev) => prev.filter((m) => !m.text.startsWith("__auth_card__")));
    // Re-send the pending message if any
    const pending = pendingMessage();
    if (pending) {
      setInput(pending);
      setPendingMessage(null);
      // Re-trigger send — set input then call send()
      await send();
    }
  } catch (e) {
    // Login failed — show error text in the card or flash error
    // Keep card visible
  } finally {
    setAuthSigningIn(false);
  }
};
```
**Why:** Core auth flow: login → remove card → re-send message.
**Constraints:** Avoid infinite loops if send() triggers auth error again (since we just logged in, this shouldn't happen, but guard defensively).

### Step 7: Handle compaction auth error similarly

**Target:** `src/components/ChatPanel.tsx`, compaction catch block at ~line 729
**Mutation:** Same check: if `String(e).includes("API key not configured")` → show auth card instead of raw error text.
**Why:** Consistent UX across both auth-guarded operations.
**Constraints:** For compaction, there's no "pending message" to resend — the card just offers login and then compaction would need to be retriggered manually (or we could store the compaction context). Simplest: show card, after login, the user can retry compaction. Since compaction is triggered automatically by the system when tokens exceed threshold, it will retry on next message send.

### Step 8: Clean up pendingMessage on successful auth poll

**Target:** `src/components/ChatPanel.tsx`
**Mutation:** After login, also call `getConfig()` to refresh `hasApiKey` signal. This way, future sends won't hit the auth guard.
**Why:** Keeps internal state consistent with reality.
**Constraints:** The `hasApiKey` signal is a backup — the real guard is in the Rust backend, so even if we mess up the signal, re-send will work if the key is actually set.

## 6. Verification Plan

### 6.1 Unit / Compile Check
- `pnpm run build` → no TypeScript errors
- `cargo build` → no Rust errors (backend unchanged)

### 6.2 Happy Path — Auth Card Shown, Login, Re-send
1. **Setup:** Clear API key (remove from `~/.config/claudinio-code/config.json` or use a fresh install)
2. **Action:** Open workspace, type "hello" in chat, press Enter
3. **Assert:** Chat shows auth card (not red error text) with "Sign In" button
4. **Action:** Click "Sign In", complete OAuth flow in browser
5. **Assert:** Auth card disappears, original "hello" message is sent automatically, agent responds

### 6.3 Error Path — Login Fails
1. **Action:** With auth card visible, cancel/close the OAuth browser window instead of completing
2. **Assert:** Card remains visible, "Sign In" button re-enabled (not stuck in loading state)

### 6.4 Auth Card Not Shown for Non-Auth Errors
1. **Action:** Trigger a different error (e.g., network error by disconnecting)
2. **Assert:** Normal "Failed to send: ..." error message appears (not auth card)

### 6.5 Compaction Auth Error
1. **Setup:** Clear API key, create a very long conversation to trigger auto-compaction
2. **Assert:** Compaction fails with auth card (not raw error text)

### 6.6 Regression — Normal Flow Unchanged
1. **Action:** With valid API key, send a message
2. **Assert:** Normal flow works exactly as before

### 6.7 i18n
1. **Action:** Switch language between en-US and pt-BR
2. **Assert:** Auth card text changes accordingly

## 7. Risks

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| Error string matching fragile if backend changes | Low | The string is hardcoded in Rust; unlikely to change without a code change noticed in PR |
| `send()` called from within `handleAuthSignIn` could trigger the same catch block | Low | After successful login, `config.api_key` is set in backend, so the guard won't fire |
| Compaction re-trigger after login not automatic | Medium | Acceptable — compaction retries on next message; we don't need to auto-compact |
| Multiple ChatPanels per workspace (one per open workspace) — auth card might appear in wrong panel | Low | Each panel has its own message list; `pendingMessage` is scoped to the panel where the error occurred |

---

## Tasks Summary

8 atomic tasks, all status=todo:
1. Add i18n keys for auth card
2. Import auth functions in ChatPanel
3. Add auth state signals
4. Modify send() catch to intercept auth errors
5. Add auth card rendering in message list
6. Implement handleAuthSignIn
7. Handle compaction auth error
8. Build verification
