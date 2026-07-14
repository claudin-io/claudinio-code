# Fix: Top Status Bar Shows "Working" Instead of "Thinking"

## Context

Both the **top agent status bar** (in `ChatPanel.tsx`, the `statusLabel()` function at line 1693) and the **bottom thinking bar** (the `ThinkingBar` component at line 3089) use the same i18n key: `chat.status.thinking` → `"Thinking"` / `"Pensando"`.

The user wants them to show different labels:
- Top bar: `"Working"` / `"Trabalhando"` (a new key)
- Bottom bar: `"Thinking"` / `"Pensando"` (unchanged, keeps existing key)

This makes semantic sense: the top bar reflects the agent's overall status (it is "Working"), while the bottom bar reflects live cognitive activity (it is "Thinking").

## Solution Design

1. Create a new i18n key `chat.status.working` in both locale files, with values `"Working"` (en-US) and `"Trabalhando"` (pt-BR).
2. Change the top status bar's `statusLabel()` switch so that when `status === "thinking"`, it returns `t("chat.status.working")` instead of `t("chat.status.thinking")`.
3. No change to `ThinkingBar` — it keeps using `t("chat.status.thinking")`.
4. No change to subagent components — they keep using `chat.subagent.running`.

## Risks

- Low risk. The change is purely cosmetic (label text) with no logic or flow impact.
- `chat.status.thinking` is used in three places (header status, ThinkingBar, and sidebar in App.tsx). We only change the header status; the sidebar and ThinkingBar are intentional about "Thinking" and should stay that way.

## Non-goals

- Do NOT change the ThinkingBar label (it should remain "Thinking").
- Do NOT change the sidebar workspace status in `App.tsx` (it should remain "Thinking").
- Do NOT change any subagent labels (`chat.subagent.running` → "Working").
- Do NOT change the status dot logic or any other status strings.

## Low-Level Design

This change touches three files across two concerns: adding i18n keys and updating a component reference.

### Files to Change

| File | Change |
|------|--------|
| `src/lib/locales/en-US.ts` | Add new key `chat.status.working` |
| `src/lib/locales/pt-BR.ts` | Add new key `chat.status.working` |
| `src/components/ChatPanel.tsx` | Change one line in `statusLabel()` |

### Step 1: Add i18n keys

**File:** `src/lib/locales/en-US.ts` (~line 122, in the `// ── ChatPanel - Status ──` block)

Insert AFTER `"chat.status.thinking": "Thinking",`:
```
"chat.status.working": "Working",
```

**File:** `src/lib/locales/pt-BR.ts` (~line 122, in the `// ── ChatPanel - Status ──` block)

Insert AFTER `"chat.status.thinking": "Pensando",`:
```
"chat.status.working": "Trabalhando",
```

### Step 2: Change statusLabel()

**File:** `src/components/ChatPanel.tsx`, line 1693

Current:
```tsx
case "thinking": return t("chat.status.thinking");
```

Change to:
```tsx
case "thinking": return t("chat.status.working");
```

The `statusDot()` function on line 1705 also has a `case "thinking"` branch — this is for the DOT COLOR only and must NOT be changed.

### Wiring / Seam Check

- The new key `chat.status.working` is defined in both locale files (`en-US.ts` and `pt-BR.ts`), and the `t()` function from `src/lib/grill-me.ts` resolves dot-notation keys against the locale dictionary — no registration step needed.
- `statusLabel()` uses `status()` which is a reactive `Status` signal (`"thinking"` is one of its values). When the session is thinking, the top bar will now display "Working" instead of "Thinking".
- `ThinkingBar` at line 3089 continues to use `t("chat.status.thinking")` — no wire change needed there.
- The sidebar workspace status in `App.tsx` also continues to use `t("chat.status.thinking")` — no wire change needed there.

### Verification Plan

1. TypeScript build passes: `npx tsc --noEmit` — no broken references.
2. en-US locale: grep confirms `chat.status.working` key exists with value `"Working"`.
3. pt-BR locale: grep confirms `chat.status.working` key exists with value `"Trabalhando"`.
4. Top bar: during agent thinking state, the top bar shows "Working" (or "Trabalhando" in pt-BR).
5. Bottom bar: during agent thinking state, the bottom ThinkingBar still shows "Thinking" (or "Pensando").
6. Regression: idle, done, error, awaiting_approval, awaiting_input status labels are unchanged.

## Tasks Summary

1. Add `chat.status.working` key to `src/lib/locales/en-US.ts`
2. Add `chat.status.working` key to `src/lib/locales/pt-BR.ts`
3. Change `statusLabel()` in `src/components/ChatPanel.tsx` to use `chat.status.working`

## Implementation Log — 2026-07-14 12:25
**Summary:** Top status bar now shows "Working"/"Trabalhando" via new i18n key; bottom bar keeps "Thinking"/"Pensando"
**Changed files:** M src/components/ChatPanel.tsx, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-14_top-bar-working-status.md
**Commits:** _(git unavailable or none)_
**Journal:** Chave `chat.status.working` criada em ambos os locales (en-US: "Working", pt-BR: "Trabalhando"). A função `statusLabel()` na barra superior foi alterada para usar a nova chave quando o status é "thinking". A barra inferior (ThinkingBar), a sidebar (App.tsx) e os subagentes permanecem inalterados — cada um mantendo suas respectivas chaves. O `statusDot()` também não foi alterado. O build TypeScript pré-existente tem erros não relacionados em `subagentTimeline.test.ts` (campo `cost` ausente).

**Task journal:**
- Add chat.status.working to en-US locale: Inserted `"chat.status.working": "Working"` after line 121
- Add chat.status.working to pt-BR locale: Inserted `"chat.status.working": "Trabalhando"` after line 121
- Change top status bar to use chat.status.working: Changed line 1693 to use `chat.status.working` instead of `chat.status.thinking`. Verified statusDot() is unchanged.
