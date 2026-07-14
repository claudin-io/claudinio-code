# Plan: Change Brain Mode Default Model to "claudius"

## Context

The user wants the default model for Brain (planning) mode to be `"claudius"` instead of `"claudinio"`. The Builder mode default should remain `"claudinio"`.

Currently, both `brain_model` and `builder_model` default to `"claudinio"` in the Rust backend and the TypeScript frontend.

## Solution Design

### What changes

| Layer | File | Change |
|-------|------|--------|
| Rust serde default | `src-tauri/src/agent/provider.rs` | `brain_model` gets its own `default_claudius()` serde fallback |
| Rust `Default` impl | `src-tauri/src/agent/provider.rs` | `brain_model` field changes from `"claudinio"` to `"claudius"` |
| Frontend UI signal | `src/App.tsx` | `configBrainModel` signal default changes from `"claudinio"` to `"claudius"` |

### What does NOT change

- Builder mode default stays `"claudinio"` everywhere — already correct, no changes.
- The `model_for_mode()` resolver in `provider.rs` (lines 201–204) stays the same — it already routes `"brain"` → `brain_model` and everything else → `builder_model`.
- The `availableModels` array in `App.tsx` already contains `"claudius"` — no change needed.
- No localization changes needed (labels are generic like "Brain Model" / "Modelo do Brain").

### User-facing impact

- Fresh installs (no existing `~/.config/claudinio-code/config.json`) will default Brain to `"claudius"`.
- Existing configs are NOT affected — the serde default only kicks in when the key is absent from the config file.
- The UI Settings panel will show `"claudius"` in the Brain Model dropdown on first open.

## Risks

| Risk | Mitigation |
|------|-----------|
| Existing users with `brain_model: "claudinio"` in their config will not see the change | This is desired — only new/empty configs get the new default |
| Frontend signal and backend default diverge | Both changed together in this plan |

## Non-goals

- Not changing Builder default (already `"claudinio"`)
- Not changing the model list or available models
- Not changing any runtime resolution logic
- Not changing any localization strings

## Low-Level Design

### Files and exact changes

#### 1. `src-tauri/src/agent/provider.rs` — Rust backend defaults

**File structure (relevant lines):**
- Line 48–49: serde attributes on `brain_model` and `builder_model` fields in the `AgentConfig` struct
- Lines 162–163: `default_claudinio()` function
- Lines 168–169: Default impl for `AgentConfig`
- Lines 201–204: `model_for_mode()` resolver

**Change A — Add `default_claudius()` function (after line 165, before `default_services_url()`):**

```rust
fn default_claudius() -> String {
    "claudius".into()
}
```

**Change B — Line 49: Change serde default for `brain_model`:**

Current: `#[serde(default = "default_claudinio")]`
New: `#[serde(default = "default_claudius")]`

**Change C — Line 176: Change `brain_model` in the `Default` impl:**

Current: `brain_model: "claudinio".into(),`
New: `brain_model: "claudius".into(),`

**NOT changed:**
- Line 51: `#[serde(default = "default_claudinio")]` on `builder_model` — stays as is
- Line 177: `builder_model: "claudinio".into(),` — stays as is

#### 2. `src/App.tsx` — Frontend signal default

**Line 101:**

Current: `const [configBrainModel, setConfigBrainModel] = createSignal("claudinio");`
New: `const [configBrainModel, setConfigBrainModel] = createSignal("claudius");`

**NOT changed:**
- Line 102: `configBuilderModel` signal — stays `"claudinio"`

### Data flow

1. `config.json` deserialization: if `brain_model` key is missing → serde `default = "default_claudius"` kicks in → `"claudius"`
2. Runtime model resolution: `model_for_mode("brain")` → reads `self.brain_model` → `"claudius"`
3. Frontend init: `createSignal("claudius")` → Settings panel shows `"claudius"` for Brain Model

### Existing patterns reused

- Same serde `default = "fn_name"` pattern already used on `builder_model` and `model` fields
- Same `default_*()` function convention already used for `default_claudinio()` and `default_services_url()`

## Tasks summary

| # | Task | File | Description |
|---|------|------|-------------|
| 1 | Add `default_claudius()` function | `provider.rs` | Insert after `default_claudinio()`, returns `"claudius".into()` |
| 2 | Update `brain_model` serde default | `provider.rs` | Change `#[serde(default = "default_claudinio")]` to `#[serde(default = "default_claudius")]` |
| 3 | Update `brain_model` in `Default` impl | `provider.rs` | Change `"claudinio"` to `"claudius"` |
| 4 | Update frontend signal default | `App.tsx` | Change `"claudinio"` to `"claudius"` in `configBrainModel` signal |
| 5 | Verify: build and test | All | `cargo build` passes; frontend compiles; default values confirmed |

## Verification Plan

1. **Build:** `cargo build` in `src-tauri/` succeeds without errors
2. **Frontend build:** `npm run build` (or equivalent) passes
3. **Grep confirmation:** No remaining `"claudinio"` default for `brain_model`/`configBrainModel` outside of the `builder_model` field
4. **Negative grep:** Confirm `builder_model` still defaults to `"claudinio"` — the builder default is untouched


## Implementation Log — 2026-07-14 16:33
**Summary:** Change Brain mode default model from "claudinio" to "claudius"
**Changed files:** M src-tauri/src/agent/provider.rs, M src/App.tsx, ?? docs/plans/2026-07-14_brain-model-default-to-claudius.md
**Commits:** _(git unavailable or none)_
**Journal:** The task was straightforward: change the default model for Brain mode from "claudinio" to "claudius", keeping Builder mode default as "claudinio". 

The changes were made in two files:
1. `src-tauri/src/agent/provider.rs` — added a `default_claudius()` function, updated the serde default attribute on `brain_model`, and changed the `Default` impl for `AgentConfig`. The `builder_model` field and its `default_claudinio()` serde default were left untouched.
2. `src/App.tsx` — changed the `configBrainModel` signal's initial value from "claudinio" to "claudius". The `configBuilderModel` signal stays "claudinio".

Key decisions:
- Created a separate `default_claudius()` function rather than reusing `default_claudinio()` to keep each default explicit and maintainable.
- No changes to the builder model defaults anywhere — only the brain model was touched.
- Existing users with `brain_model: "claudinio"` in their `~/.config/claudinio-code/config.json` won't see a change (correct behavior — the serde default only applies when the key is absent from the config file).
- The model was already in the `availableModels` list, so no changes needed there.

Builds: cargo build OK, npm run build (35 test files, 645 tests) OK. Grep confirmed no stale claudinio references remain for brain_model/configBrainModel in source files.

**Task journal:**
- Add default_claudius() function: Added `default_claudius()` function between `default_claudinio()` and `default_services_url()`
- Update brain_model serde default: Changed serde default on brain_model from default_claudinio to default_claudius; builder_model untouched
- Update brain_model in Default impl: Changed brain_model in Default impl to "claudius"; builder_model stays "claudinio"
- Update frontend signal default: Changed configBrainModel signal default to "claudius"; configBuilderModel untouched
- Verify: build, grep, confirm: cargo build OK; npm run build (vitest + vite) OK — 645 tests passed; grep confirmation: no source files have brain_model or configBrainModel defaulting to claudinio; builder_model still correctly set to claudinio in both provider.rs and App.tsx
