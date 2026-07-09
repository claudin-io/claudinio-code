# Onboarding Screen for First-Time Users

## 1. Context / Problem Statement

When a user opens Claudinio Code for the first time (or anytime without being logged in), they see the generic `EmptyState` component that only shows "Open a project folder". There is **no onboarding experience** explaining what the app does, its features, or how to get started. Sign-in is buried inside the settings modal. The user has no clear path to authenticate and understand value before using the product.

This plan creates a dedicated onboarding screen that appears **whenever no login is active** (i.e., `accountLogin` is `null` in config), replaces the current `EmptyState` in that scenario, and guides the user through a 3-step wizard ending with a mandatory sign-in.

The sign-in flow reuses the existing `loginWithClaudinio()` IPC call (browser-based OAuth via `claudin.io/app/authorize`).

## 2. Goal (Definition of Done)

- When no `accountLogin` is present in config, the fullscreen area shows the `OnboardingWizard` component (replacing `EmptyState`).
- The wizard has 3 steps: (1) Welcome, (2) Features, (3) Sign In.
- Step 3 has a "Sign In" button that calls `loginWithClaudinio()`. On success (`accountLogin` becomes non-null), the onboarding disappears and the regular `EmptyState` is shown (user can then open a project).
- Navigation between steps via dots/indicators and arrow buttons.
- Both `pt-BR` and `en-US` locales have all onboarding strings.
- The `OnboardingWizard` component has a test file with basic rendering tests.
- **No** "Skip" button — sign-in is mandatory per user decision.

## 3. Key Findings (Prova Real)

| Finding | Method | Proof |
|---------|--------|-------|
| No first-run detection exists in Rust backend | `backend-explorer` agent report | "No onboarding flag, welcome screen logic, or 'has user completed first setup' check anywhere" |
| Config stored at `~/.config/claudinio-code/config.json` | `agent/provider.rs` via `config_path()` | `load_config()` returns `AgentConfig::default()` when file missing — empty `api_key` and `account_login: None` |
| Login flow is browser OAuth → API key exchange | `commands/auth.rs` lines `login_with_claudinio()` | Opens `claudin.io/app/authorize`, listens on `127.0.0.1:<random_port>`, exchanges code for API key |
| Frontend tracks `accountLogin()` signal from `getConfig()` | `App.tsx` line: `setAccountLogin(cfg.accountLogin ?? null)` | Signal set in `openConfig()`, also passed from `loginWithClaudinio()` return |
| `EmptyState` shown when `activeWorkspace()` is falsy | `App.tsx` line: `<Show when={activeWorkspace()} fallback={<EmptyState .../>}>` | The EmptyState is the fallback when no workspace is open |
| `loginWithClaudinio()` returns `{ login, tier }` | `ipc.ts` `LoginResult` interface | `doLogin()` in `App.tsx` sets `accountLogin` and `accountTier` from result |
| Available icon set in `Icon.tsx` | Read `src/components/Icon.tsx` | `brain`, `thinking-face`, `terminal`, `layers`, `clock`, `check-circle`, `construction-worker`, `notebook-pen`, `search`, `package`, `goal`, etc. |
| App uses SolidJS + Tailwind CSS v4 | `package.json` dependencies | `solid-js: ^1.9.3`, `tailwindcss: ^4.3.2`, `@tailwindcss/vite: ^4.3.2` |
| Locale system uses `t()` function with `{0}` placeholders | `src/lib/grill-me.ts` | `t(key, ...args)` replaces `{0}`, `{1}`, etc. |
| Login button already exists in config modal | `App.tsx` — config modal section | `doLogin()` calls `loginWithClaudinio()` and sets `accountLogin`/`accountTier` |

## 4. Authoritative Inputs

| Input | Source | Notes |
|-------|--------|-------|
| Onboarding appears when `accountLogin === null` | User decision | NOT based on first-run flag — anytime no login |
| Fullscreen (replaces EmptyState area) | User decision | Same layout zone as current EmptyState |
| 3-step wizard (carousel) | User decision | Welcome → Features → Sign In |
| Features to highlight | User confirmed "Todas acima" | Chat agent, tool approvals, subagents, sessions, indexação, steering |
| Sign In uses existing `loginWithClaudinio()` | User decision | Browser OAuth flow |
| No skip button — sign in mandatory | User decision | User cannot access app without authenticating |
| Icon library: existing `Icon` component | Code (`Icon.tsx`) | Use `brain`, `thinking-face`, `layers`, `clock`, `check-circle`, `goal` |
| `package.json` | SolidJS + Tailwind v4 | No new dependencies needed |
| Locale files: `src/lib/locales/pt-BR.ts`, `en-US.ts` | Code | Add `onboarding.*` keys |

## 5. Changes (Steps)

### Step 1 — Create `OnboardingWizard` component

- **Target:** New file `src/components/OnboardingWizard.tsx`
- **Mutation:** Create a SolidJS component that renders a 3-step carousel:
  - **Step indicator:** Dots at the top/bottom showing active step (3 dots).
  - **Step 1 — Welcome:**
    - Logo/branding (`/reddit_icon_256.png` — same as header) or icon `brain` large.
    - Title: `onboarding.welcome.title`
    - Subtitle: `onboarding.welcome.subtitle` (explaining what Claudinio Code is: AI coding agent for developers)
    - A short tagline/description
  - **Step 2 — Features:**
    - 3-4 feature cards with icons from the existing Icon set:
      - Agent de IA: `thinking-face` — conversa, planeja, executa código
      - Ferramentas com aprovação: `check-circle` — comandos bash e edições com diff visual
      - Subagentes paralelos: `layers` — até 4 agentes rodando simultaneamente
      - Indexação inteligente: `search` — busca semântica com CodeBERT
    - Cards arranged in a 2×2 grid
  - **Step 3 — Sign In:**
    - Icon: `goal` (or similar)
    - Title: `onboarding.signIn.title`
    - Description: `onboarding.signIn.subtitle`
    - "Sign In" button calling `props.onSignIn()`
    - Loading state with spinner and text `onboarding.signIn.signingIn`
    - Error display if sign-in fails
  - **Navigation:**
    - Arrows (left/right) to navigate between steps.
    - Step dots are clickable (go directly to a step).
    - "Next" button on steps 1 and 2.
  - **Props:** `onSignIn: () => Promise<void>`, `signingIn: boolean`, `signInError: string | null`
- **Why:** This is the core onboarding UI component.
- **Constraints:** Use existing Tailwind v4 classes, existing `Icon` component, existing `t()` for i18n. Follow existing component patterns (SolidJS functional components with `Component<Props>` type). Use `createSignal` for local state (current step).

### Step 2 — Add `OnboardingWizard` test file

- **Target:** New file `src/components/OnboardingWizard.test.tsx`
- **Mutation:** Basic rendering tests:
  - Renders without crashing
  - Shows step 1 content by default
  - Can navigate to step 2 via "Next" button
  - Can navigate to step 3
  - Shows sign-in button on step 3
  - Clicking sign-in calls `onSignIn`
- **Why:** Maintain test coverage for new component.
- **Constraints:** Use `vitest` + `@solidjs/testing-library` + `@testing-library/jest-dom` (same as existing tests like `EmptyState.test.tsx`).

### Step 3 — Add locale strings to `pt-BR.ts` and `en-US.ts`

- **Target:** `src/lib/locales/pt-BR.ts`, `src/lib/locales/en-US.ts`
- **Mutation:** Add `onboarding.*` key block to both files:
  - `onboarding.welcome.title`
  - `onboarding.welcome.subtitle`
  - `onboarding.welcome.tagline`
  - `onboarding.features.title`
  - `onboarding.features.agent.title`
  - `onboarding.features.agent.desc`
  - `onboarding.features.approval.title`
  - `onboarding.features.approval.desc`
  - `onboarding.features.subagents.title`
  - `onboarding.features.subagents.desc`
  - `onboarding.features.indexing.title`
  - `onboarding.features.indexing.desc`
  - `onboarding.signIn.title`
  - `onboarding.signIn.subtitle`
  - `onboarding.signIn.button`
  - `onboarding.signIn.signingIn`
  - `onboarding.signIn.error`
  - `onboarding.next`
  - `onboarding.prev`
- **Why:** i18n support for onboarding text.
- **Constraints:** Follow existing pattern. Portuguese translations should be natural Brazilian Portuguese.

### Step 4 — Wire onboarding into `App.tsx`

- **Target:** `src/App.tsx`
- **Mutation:**
  1. Import `OnboardingWizard`.
  2. Track sign-in state at App level: add signals for `onboardingSigningIn` and `onboardingSignInError` (or reuse `loggingIn`).
  3. Create an `onboardingSignIn` handler that calls `loginWithClaudinio()`, sets `accountLogin`/`accountTier` on success, and handles errors.
  4. In the main content area (`<main>`), when `!activeWorkspace() && !accountLogin()`, show `<OnboardingWizard>` instead of `<EmptyState>`.
  5. When `!activeWorkspace() && accountLogin()`, show `<EmptyState>` as before (user can open a project).
  6. After successful sign-in from onboarding, `accountLogin` becomes non-null, so the conditional renders `EmptyState` automatically.
  7. The existing `doLogin()` in config modal should also work and update the same signals.
- **Why:** Integrates onboarding into the app flow.
- **Constraints:** Do NOT modify the config modal login flow. Both paths (onboarding sign-in and config modal sign-in) must work and update the same `accountLogin` signal.

### Step 5 — (No changes needed) Backend

- **Target:** `src-tauri/` — NO CHANGES
- **Why:** The existing `login_with_claudinio` command, `get_config`, and `set_config` already support everything needed. The onboarding is purely a frontend concern.

## 6. Verification Plan

### Dry-run / Review
- Review `OnboardingWizard.tsx` for correct SolidJS patterns, Tailwind v4 usage, i18n key usage.
- Review locale files for completeness and consistency.

### Apply
- `pnpm run test` — all existing tests + new `OnboardingWizard.test.tsx` pass.

### End-to-end
1. Launch app with no `config.json` (or `accountLogin: null`) → Onboarding Wizard displayed.
2. Navigate through 3 steps using arrow buttons and dots → all steps render correctly.
3. Click "Sign In" on step 3 → `loginWithClaudinio()` called → browser opens.
4. (Mocked) Sign-in completes → `accountLogin` set → Onboarding disappears → EmptyState shown with "Open folder" button.
5. Open settings modal → account shows "Signed in as ...".

### Visual proof
- Screenshot the onboarding wizard at all 3 steps (desktop 1280px width).
- Screenshot the transition to EmptyState after sign-in.
- Verify text is readable, cards are aligned, colors match the design system.

### Regression
- Existing config modal sign-in still works.
- Existing EmptyState still works when logged in but no workspace open.
- Workspace open/close flow unchanged.
- Language switcher affects onboarding text.

### Edge / No-op safety
- Rapid clicks on "Sign In" should not trigger multiple calls (button disabled while `signingIn === true`).
- If user is already logged in when app starts, onboarding never appears.
- Logout from config modal → back to onboarding if no workspace open.

## 7. Tasks Summary

1. Create `OnboardingWizard.tsx` component with 3-step carousel
2. Create `OnboardingWizard.test.tsx` with basic rendering tests
3. Add onboarding locale strings to `pt-BR.ts` and `en-US.ts`
4. Wire `OnboardingWizard` into `App.tsx` conditional rendering
5. Run full test suite + visual verification


## Implementation Log — 2026-07-09 12:05
**Summary:** Onboarding wizard screen with 3-step carousel (Welcome → Features → Sign In) shown when user is not logged in, replacing EmptyState until sign-in is completed.
**Changed files:** M src/App.tsx, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? .claudinio.json, ?? docs/plans/2026-07-09_onboarding-screen.md, ?? src/components/OnboardingWizard.test.tsx, ?? src/components/OnboardingWizard.tsx
**Commits:** _(git unavailable or none)_
**Journal:** All 5 tasks implemented and verified. 

Key decisions:
- Onboarding appears when accountLogin is null (not a first-run flag). Every time the user is logged out, they see onboarding.
- The component sits in the <main> area as a replacement fallback for EmptyState, using nested <Show> conditions.
- The sign-in handler (onboardingSignIn) is separate from the config modal's doLogin() — both coexist and update the same accountLogin signal.
- Locale keys follow the existing pattern: onboarding.* keys in both pt-BR and en-US.
- Feature card grid uses a 2×2 layout (grid grid-cols-2 gap-3) with icons from the existing Icon component.
- Step 3 has no "Next" button — sign-in button is the primary CTA, only Prev is available to go back.
- No changes to Rust backend — all frontend concerns.

Gotchas:
- The existing `loggingIn` signal in App.tsx is reused for the onboarding's signingIn prop — both paths (onboarding + config modal) trigger it via loginWithClaudinio().
- Images (/reddit_icon_256.png) work at build time but not in test environment — the OnboardingWizard test only checks textContent.
- Playwright was not available for visual verification (missing browser binaries), but the component was code-reviewed against design tokens.

**Task journal:**
- Create OnboardingWizard.tsx component: Created src/components/OnboardingWizard.tsx with 3-step carousel wizard component; Uses Icon component (brain, thinking-face, check-circle, layers, search, goal, external-link, chevron-left, chevron-right); Feature cards rendered via For loop from features array; Three step dots as clickable indicators; Nav buttons adapt per step (step 0: Next only, step 1: Prev+Next, step 2: Prev only)
- Create OnboardingWizard.test.tsx: Created src/components/OnboardingWizard.test.tsx with 8 tests; Tests cover: default render, next navigation, prev navigation, dots clicking, signIn callback, disabled state, error display; All 8 tests passing
- Add onboarding locale strings (pt-BR + en-US): Added 18 onboarding keys to pt-BR.ts: welcome (title, subtitle, tagline), features (title + 4 cards), signIn (title, subtitle, button, signingIn, error), next, prev; Added 18 matching keys to en-US.ts
- Wire OnboardingWizard into App.tsx: Added import of OnboardingWizard; Added onboardingSignInError signal; Added onboardingSignIn() handler that calls loginWithClaudinio(); Replaced <main> fallback: nested Show — OnboardingWizard when !accountLogin(), EmptyState when logged in
- Verify: run tests + visual check: pnpm run test: 288/288 passed (18 files); pnpm run build: successful, no errors; Chrome-based visual screenshots not possible due to missing playwright browsers, but all files verified via code review
