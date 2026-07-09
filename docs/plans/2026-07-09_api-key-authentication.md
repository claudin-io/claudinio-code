# API Key Authentication — Solution Design

## 1. Context / Problem Statement

Claudinio Code currently requires OAuth sign-in via claudin.io to access the app. Organizations on claudin.io that use only API keys (no user accounts) are completely locked out — they cannot get past the onboarding screen because the app gate checks `accountLogin` (which only gets set via OAuth), and there is no way to enter an API key during onboarding.

**Confirmed by user:**
- Organizations use API keys only; they have no claudin.io user account
- The API key option must be available in both onboarding (Step 2) and settings
- Validation should happen when the key is submitted (ping the API by listing models)
- On failure, show the error inline and keep the user on the same screen

**Inferred (from codebase):**
- `get_config` already returns `hasApiKey: bool` — the frontend just ignores it
- `set_config` already accepts `api_key` — no backend changes needed to persist
- There's no initial `getConfig()` call on app startup — accountLogin starts null, which means even previously-authenticated users may see onboarding again on restart (this is a pre-existing gap)

## 2. Goal (Definition of Done)

1. Users can authenticate by entering an API key during **onboarding** (Step 2) via a "Use API Key instead" link → paste field → validation → transition to main UI
2. Users can authenticate by entering an API key in **settings** (prominently, not buried in "Advanced")
3. The app gate checks **either** `accountLogin` (OAuth) **or** `hasApiKey` (manual key) to decide whether to show onboarding vs. main UI
4. API-key-only users see "Signed In" in settings with a Sign Out button (no account details)
5. On app restart, previously authenticated API-key users skip onboarding

## 3. Key Findings (Prova Real)

| Finding | Method | Proof |
|---|---|---|
| `get_config` returns `hasApiKey` but frontend ignores it | `src-tauri/src/commands/agent.rs:497-525` — `"hasApiKey": !cfg.api_key.is_empty()` in JSON response | Code inspection |
| `set_config` accepts optional `api_key` | `src-tauri/src/commands/agent.rs:439-459` — `SetConfigArgs.api_key: Option<String>` | Code inspection |
| Onboarding gate checks only `accountLogin()` | `src/App.tsx:654` — `<Show when={accountLogin()} fallback={<OnboardingWizard ...>}>` | Code inspection |
| Settings auth section checks only `accountLogin()` | `src/App.tsx:371-399` — `<Show when={accountLogin()} fallback={<button>Sign In</button>}>` | Code inspection |
| No initial `getConfig()` call on app startup | `src/App.tsx:99-141` — two `onMount` blocks, neither loads config | Code inspection |
| `list_models` swallows errors and returns fallback | `src-tauri/src/commands/agent.rs:517-543` — any failure returns `["claudinio", "claudius"]` | Code inspection |
| `OnboardingWizard` only has OAuth sign-in | `src/components/OnboardingWizard.tsx` — Step 2 has only `onSignIn` button | Code inspection |
| `logout_claudinio` clears api_key + account fields | `src-tauri/src/commands/auth.rs:226-234` | Code inspection |

## 4. Authoritative Inputs

| Input | Value | Source |
|---|---|---|
| Onboarding API key UX | Small "Use API Key instead" link → expands a paste field | User |
| Account display for API-key-only | "Signed In" label, no personal info | User |
| Auth gate logic | `accountLogin \|\| hasApiKey` → skip onboarding | User |
| Validation method | Ping API by listing models (validate before transition) | User |
| Error display | Inline below the API key field with actual error message | User |
| Settings for API-key-only | "Signed In" + Sign Out only, both hidden when unauthenticated | User |
| API key settings prominence | Together with sign-in button; both hidden when authenticated | User |

## 5. Changes (Steps)

### Step A: New Tauri command — `validate_api_key`

**Target:** `src-tauri/src/commands/auth.rs` (or `agent.rs`)

**Mutation:** Add a new `#[tauri::command]` that:
- Accepts an `api_key: String` parameter (we already have the base_url from AppState)
- Calls `GET {base_url}/v1/models` with `x-api-key: {key}` header
- On non-success status: returns `Err("Authentication failed (HTTP {status}): {body}")`
- On success: parses models from response JSON and returns them
- On network error: returns `Err("Network error: {e}")`  

**Register:** Add to `lib.rs` `generate_handler![]`

**Why:** The existing `list_models` command returns fallback on ANY error, so it can't distinguish "bad key" from "network down". This gives precise validation feedback.

### Step B: Frontend IPC wrapper — `validateApiKey`

**Target:** `src/lib/ipc.ts`

**Mutation:** Add:
```ts
export function validateApiKey(apiKey: string): Promise<string[]> {
  return invoke<string[]>("validate_api_key", { apiKey });
}
```

### Step C: New signals in App.tsx

**Target:** `src/App.tsx` (signals section, ~line 75)

**Mutation:** Add signals:
```ts
const [hasApiKey, setHasApiKey] = createSignal(false);
const [apiKeyValidating, setApiKeyValidating] = createSignal(false);
const [onboardingApiKeyError, setOnboardingApiKeyError] = createSignal<string | null>(null);
```

### Step D: Load initial auth state on app startup

**Target:** `src/App.tsx` (onMount section, ~line 127)

**Mutation:** Add a new `onMount` that calls `getConfig()` and initializes `accountLogin`, `accountTier`, and `hasApiKey` from the persisted config:
```ts
onMount(async () => {
  try {
    const cfg = await getConfig();
    if (cfg) {
      setAccountLogin(cfg.accountLogin ?? null);
      setAccountTier(cfg.accountTier ?? null);
      setHasApiKey(cfg.hasApiKey ?? false);
    }
  } catch {}
});
```

**Why:** Without this, `hasApiKey` is always false on startup and API-key-only users see onboarding every time.

### Step E: Update auth gate condition

**Target:** `src/App.tsx` (line ~654)

**Mutation:** Change from:
```tsx
<Show when={accountLogin()} fallback={...}>
```
To:
```tsx
<Show when={accountLogin() || hasApiKey()} fallback={...}>
```

### Step F: Onboarding — API key handler + pass new props

**Target:** `src/App.tsx` (near `onboardingSignIn`)

**Mutation:** Add `onboardApiKeySubmit` handler:
```ts
const onboardApiKeySubmit = async (key: string) => {
  setOnboardingApiKeyError(null);
  setApiKeyValidating(true);
  try {
    await setConfig({ apiKey: key });
    // Validate
    await validateApiKey(key);
    setHasApiKey(true);
  } catch (e) {
    setOnboardingApiKeyError(String(e));
  } finally {
    setApiKeyValidating(false);
  }
};
```

Also update the `<OnboardingWizard>` props to pass the new callbacks.

### Step G: OnboardingWizard — API key UI

**Target:** `src/components/OnboardingWizard.tsx`

**Mutation:**
- Add props: `onApiKeySubmit: (key: string) => Promise<void>`, `apiKeyValidating: boolean`, `apiKeyError: string | null`
- Add local state: `showApiKeyField` (toggled by link)
- Add local state: `apiKeyInput` (the key value)
- **Step 2 changes:**
  - After the OAuth sign-in button, add: `<button>Use API Key instead</button>` (a small link-styled button)
  - When `showApiKeyField` is true, replace the OAuth button area with:
    - API key input field (password type, placeholder "sk-...")
    - "Continue" button (disabled while `apiKeyValidating`, shows spinner when validating)
    - Error message below on failure
    - Small link "← Back to sign in" to toggle back to OAuth

### Step H: Settings modal — rework auth section

**Target:** `src/App.tsx` (settings modal, lines ~370-414)

**Mutation:** Replace the current account section:

```tsx
{/* Account / Auth */}
<label class="mb-1 block text-xs text-ink-muted">{t("app.config.account")}</label>

<Show when={accountLogin() || hasApiKey()}
  fallback={
    // NOT authenticated — show both options
    <div class="space-y-2 mb-4">
      <button onClick={doLogin} disabled={loggingIn()} ...>
        {loggingIn() ? t("app.config.signingIn") : t("app.config.signIn")}
      </button>
      {/* API Key field — always visible when not authenticated (no "Advanced" toggle) */}
      <label class="block text-xs text-ink-muted">{t("app.config.apiKey")}</label>
      <input type="password" value={configApiKey()} .../>
    </div>
  }>
  {/* Authenticated */}
  <div class="mb-4 flex items-center justify-between rounded-md border ...">
    <span>{accountLogin() ? t("app.config.signedInAs", accountLogin()!) : t("app.config.signedIn")}</span>
    <Show when={accountTier()}> — {accountTier()}</Show>
    <button onClick={doLogout}>Sign Out</button>
  </div>
</Show>
```

**Also:** Remove the `showAdvancedAuth` toggle entirely — the API key field is no longer hidden behind "Advanced".

### Step I: Update logout to clear hasApiKey

**Target:** `src/App.tsx` (doLogout ~line 218)

**Mutation:** Add `setHasApiKey(false)` after clearing accountLogin/accountTier.

### Step J: i18n strings

**Target:** `src/lib/locales/en-US.ts` and `src/lib/locales/pt-BR.ts`

**Mutation:** Add new keys under `onboarding.signIn`:
```ts
"onboarding.signIn.apiKeyLink": "Use API Key instead",
"onboarding.signIn.apiKeyBack": "← Back to sign in",
"onboarding.signIn.apiKeyPlaceholder": "Paste your API key",
"onboarding.signIn.apiKeyContinue": "Continue",
"onboarding.signIn.apiKeyValidating": "Validating…",
```

And one under `app.config`:
```ts
"app.config.signedIn": "Signed In",
```

### Step K: Validate API key in settings too

**Target:** `src/App.tsx` (saveConfig ~line 175-200)

**Mutation:** When `configApiKey()` is non-empty and being saved from the settings modal (user is not already authenticated), also validate the key before closing the modal. If validation fails, show the error inline in settings (or via alert).

### Step L: Register new command in Tauri

**Target:** `src-tauri/src/lib.rs`

**Mutation:** Add `commands::auth::validate_api_key` to the `generate_handler![]` macro.

## 6. Verification Plan

### A. Dry-run: review the diff before applying
- Read each modified file and verify changes match the plan.

### B. Build check
```bash
cd src-tauri && cargo check 2>&1
```
Expected: no compilation errors.

### C. Frontend type check
```bash
pnpm exec tsc --noEmit 2>&1
```
Expected: no type errors.

### D. Happy path — API key on onboarding
1. Start app fresh (no config.json or empty api_key)
2. Navigate to onboarding Step 2
3. Click "Use API Key instead"
4. Paste a valid API key
5. Click "Continue"
6. Verify: "Validating..." appears, then onboarding disappears, main UI shows
7. Restart app → verify onboarding is SKIPPED

### E. Error path — invalid API key on onboarding
1. Start fresh, navigate to Step 2
2. Click "Use API Key instead"
3. Paste an invalid key
4. Click "Continue"
5. Verify: error appears inline below the field, onboarding remains visible

### F. Happy path — API key in settings
1. Open settings when unauthenticated
2. Verify: both OAuth button AND API key field are visible (no "Advanced" toggle)
3. Paste valid key, click Save
4. Verify: settings closes, account shows "Signed In"

### G. Settings — authenticated view (API-key-only)
1. Open settings when authenticated via API key
2. Verify: shows "Signed In" + Sign Out button, no account details

### H. Settings — authenticated view (OAuth)
1. Open settings when authenticated via OAuth
2. Verify: shows email + tier + Sign Out (unchanged behavior)

### I. Logout
1. Click Sign Out from settings
2. Verify: onboarding reappears (no key, no account)

### J. Regression — OAuth login
1. Start fresh, navigate to Step 2
2. Use the OAuth "Sign in with claudin.io" button
3. Verify: same flow as before, no regressions

## 7. Risks

| Risk | Mitigation |
|---|---|
| `validate_api_key` exposes the key in a network call before the user finishes configuring | Acceptable — this is user-initiated validation, same pattern as every SaaS API key setup |
| `getConfig()` on startup adds latency | It's a local file read (~ms), fire-and-forget so it doesn't block UI |
| `hasApiKey` could get out of sync with `accountLogin` | Both are always set from the same `getConfig()` response on startup and settings open; logout clears both |
| Pre-existing: app lacks initial config load → users may briefly see onboarding flash | This plan fixes that gap by adding the startup `getConfig()` call |

## 8. Tasks Summary

| # | Task | Files |
|---|---|---|
| A | Add `validate_api_key` Rust command | `src-tauri/src/commands/auth.rs`, `src-tauri/src/lib.rs` |
| B | Add `validateApiKey` IPC wrapper | `src/lib/ipc.ts` |
| C | Add new signals to App.tsx | `src/App.tsx` |
| D | Load initial auth state on startup | `src/App.tsx` |
| E | Update auth gate condition | `src/App.tsx` |
| F | Add onboarding API key handler | `src/App.tsx` |
| G | Update OnboardingWizard with API key UI | `src/components/OnboardingWizard.tsx` |
| H | Rework settings auth section | `src/App.tsx` |
| I | Update logout to clear hasApiKey | `src/App.tsx` |
| J | Add i18n strings | `src/lib/locales/en-US.ts`, `src/lib/locales/pt-BR.ts` |
| K | Validate API key in settings save | `src/App.tsx` |
| L | Register new command | `src-tauri/src/lib.rs` |


## Implementation Log — 2026-07-09 15:10
**Summary:** Implement API key authentication support for organization users in onboarding and settings
**Changed files:** M src-tauri/src/commands/auth.rs, M src-tauri/src/lib.rs, M src/App.tsx, M src/components/OnboardingWizard.test.tsx, M src/components/OnboardingWizard.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? .claudinio.json, ?? docs/plans/2026-07-09_2026-07-09-fix-onboarding-bugs.md, ?? docs/plans/2026-07-09_api-key-authentication.md
**Commits:** _(git unavailable or none)_
**Journal:** Implementation went smoothly. Key decisions/learnings:

1. **validate_api_key** command was necessary because the existing `list_models` command silently swallows all errors and returns a fallback list — making it impossible to distinguish "bad key" from "network down". The new command uses the same `/v1/models` endpoint but preserves error details.

2. **startup auth state load** was a critical addition — without it, API-key-only users would see onboarding every time they restart the app, because `hasApiKey` was never loaded from the persisted config.

3. **The `showAdvancedAuth` toggle** was removed entirely since the API key field is now always visible alongside the OAuth button when not authenticated — simplifying the UX.

4. **OnboardingWizard test** needed the new required props added to all test cases to keep type-check passing. The remaining TS errors are pre-existing issues (vitest globals not recognized by tsc, test file issues unrelated to this work).

5. **Rust backend** compiled cleanly with no issues — the new command integrates well with the existing AppState/config pattern.

**Task journal:**
- A: Add validate_api_key Rust command: Added validate_api_key to auth.rs calling /v1/models with provided key, returning distinct errors for network vs auth failure
- B: Add validateApiKey IPC wrapper: Added validateApiKey() IPC wrapper
- C: Add new signals to App.tsx: Signals added, validateApiKey imported
- D: Load initial auth state on startup: Added onMount after workspace restore — fires getConfig() async and populates auth state
- E: Update auth gate condition: Updated the Show condition that gates onboarding vs main UI
- F: Add onboarding API key handler: Added onboardApiKeySubmit: saves key via setConfig, validates via validateApiKey, sets hasApiKey on success
- G: Update OnboardingWizard with API key UI: Added showApiKeyField state, 'Use API Key instead' link, password input, Continue button with spinner, error display, back link
- H: Rework settings auth section: Removed showAdvancedAuth toggle. When not authenticated: shows OAuth button + API key input side by side. When authenticated: shows Signed In label + Sign Out
- I: Update logout to clear hasApiKey: setHasApiKey(false) added alongside accountLogin/accountTier clears
- J: Add i18n strings: Added 7 keys: apiKeyLink, apiKeyBack, apiKeyPlaceholder, apiKeyContinue, apiKeyValidating, signedIn + translations
- K: Validate API key in settings save: saveConfig now calls validateApiKey before saving if a new key is provided by an unauthenticated user. Error shown inline in settings modal
- L: Register new Tauri command: Added commands::auth::validate_api_key to generate_handler![] in lib.rs
