# Fix: web_search tool never invoked, and gate it to claudinio-account sessions only

## Context
The user added a `/api/app/websearch` endpoint in `claudinio_litellm` and a matching `web_search` tool in `claudinio_code`, but has never observed the agent actually calling it. Investigation shows the tool is wired correctly end-to-end but is never mentioned in the system prompt, so the model has no cue to use it.

Separately, the user clarified a requirement while reviewing the fix: `web_search` is a claudinio-subscriber feature (it proxies through claudinio_litellm using the claudinio account's own API key), so it must only be exposed to the LLM when the session is authenticated via claudinio's own account — never when the user is on a third-party/BYOK override key. Today the tool is unconditionally included in the tool list regardless of key source.

## Solution Design
The fix has two independent parts: (A) gating `web_search` to claudinio-account sessions only, and (B) adding system-prompt guidance so the model knows when and how to use it.

For (A), we add an `is_claudinio_account()` helper on `AgentConfig` (checking that `override_api_key` is `None` and `api_key` is non-empty), then use `.retain()` in both the main session and subagent tool-list builders to exclude `web_search` when the session is not a claudinio account.

For (B), we add a bullet to the `# 2. CODE TOOLS` section of `SYSTEM_PROMPT` and equivalent one-liners to the Brain and Builder mode prompt blocks, all phrased as "if available" so the model gracefully handles the tool's absence in BYOK sessions.

### Why the agent never calls it
The tool is present in the LLM's tool list but the system prompt never mentions it. `SYSTEM_PROMPT` in `src-tauri/src/agent/session.rs:275-330` (section `# 2. CODE TOOLS`) and the Brain-mode block (`session.rs:427-438`, "Investigation: smart tools first") both spell out an explicit tool-priority hierarchy for investigation — `semantic_search` → `code_search`/`symbol_lookup` → `file_outline`/`read_file` → `grep`/bash — entirely scoped to the codebase, with no mention of reaching for external/current information.

### How claudinio-account vs BYOK is distinguished
`AgentConfig` (`src-tauri/src/agent/provider.rs:39-121`):
- `api_key: String` — claudinio's own key (set via `login_with_claudinio` or pasted).
- `override_api_key: Option<String>` — BYOK override, used only for `/v1/messages` inference calls. Doc comment confirms: *"Does NOT affect login, websearch, or list_models."*
- Existing pattern `effective_api_key = config.override_api_key.as_deref().unwrap_or(&config.api_key)` appears 3x in `provider.rs` (lines 545, 612, 700) but is only used for inference headers, not feature gating.

No existing helper answers "is this a claudinio account session" — need to add one.

The `web_search` tool *implementation* (`src-tauri/src/agent/tools/web_search.rs:34-54`) already only uses `config.api_key` (never `override_api_key`) and fails with "Not logged in" if it's empty — so execution is already implicitly claudinio-only. The gap is that the tool *definition* is still handed to the LLM in BYOK sessions, so the model can call it and get a hard failure instead of never seeing it as an option.

### Changes Summary
1. Add `is_claudinio_account()` helper to `AgentConfig` in provider.rs.
2. Gate `web_search` in main session tool list via `.retain()` in session.rs.
3. Thread `&AgentConfig` through subagent tool-list builders and apply same gate in subagent.rs.
4. Add system-prompt guidance to teach the model to use `web_search`.

## Risks
- **Low risk**: Adding a bullet to `SYSTEM_PROMPT` is a raw string literal (`r#"..."#`). The new bullet must be added inside the raw string, not after. No escaping issues expected.
- **Low risk**: The `retain` gate on `web_search` is a one-liner that mirrors the existing `retain` on `edit_file` (line 712). If the reference is wrong, the tool may appear or be hidden incorrectly — but this is trivially catchable in testing.
- **Low risk**: Threading `&AgentConfig` through `subagent_defs` and `api_tools` is a mechanical parameter plumbing. The only call site is already within `run_subagent` which HAS `&AgentConfig` in scope.

## Non-goals
- NOT adding web_search to the Builder mode prompt block's dedicated tool-hierarchy section (there isn't a separate one — just a one-liner at the bottom of that block).
- NOT changing the web_search tool implementation — execution already gates on `config.api_key` correctly.
- NOT adding runtime logging or telemetry for when web_search is gated out.

## Low-Level Design

This section describes the exact files, symbols, code changes, and data flow for implementing the Solution Design.

### Files & Symbols

| File | Symbol/Range | Change |
|---|---|---|
| `src-tauri/src/agent/provider.rs` | After line 121 (closing `}` of `AgentConfig` struct) | Add `impl AgentConfig` block with `is_claudinio_account()` |
| `src-tauri/src/agent/session.rs` | Line 286-289 (`# 2. CODE TOOLS` section in `SYSTEM_PROMPT` raw string) | Add web_search bullet after the last bullet |
| `src-tauri/src/agent/session.rs` | Lines 427-438 (Brain mode "Investigation" block) | Add one-liner after grep/bash line (~line 434) |
| `src-tauri/src/agent/session.rs` | Lines ~515-518 (Builder mode tool-hierarchy one-liner) | Add web_search mention |
| `src-tauri/src/agent/session.rs` | Line 706 (`let mut defs = tools::get_defs(maxp);` in `api_tools`) | Add `retain` gate |
| `src-tauri/src/agent/subagent.rs` | Line 72 (`pub fn subagent_defs(...)`) | Add `config: &AgentConfig` parameter |
| `src-tauri/src/agent/subagent.rs` | Line 77 (after `.filter(...)` line) | Add `retain` gate for web_search |
| `src-tauri/src/agent/subagent.rs` | Line 92 (`fn api_tools(mode, mcp_defs)`) | Add `config: &AgentConfig` parameter |
| `src-tauri/src/agent/subagent.rs` | Line 93 (call to `subagent_defs(...)`) | Pass `config` |
| `src-tauri/src/agent/subagent.rs` | Line 293 (call site in `run_subagent`) | Pass `config` to `api_tools` |

### Concrete Code Changes

**1. provider.rs — add helper** (after the `AgentConfig` struct closing `}` at line 121):

```rust
impl AgentConfig {
    /// True when the session uses claudinio's own API key (not a BYOK override)
    /// and that key is present. This gates subscriber-only features like web_search.
    pub fn is_claudinio_account(&self) -> bool {
        self.override_api_key.is_none() && !self.api_key.is_empty()
    }
}
```

**2. session.rs — SYSTEM_PROMPT CODE TOOLS bullet** (inside the raw string `r#"..."#`, after "Never use bash search tools..." at line ~289):

Add this line:
```
- For current/external information not in the codebase or your training data (docs, library versions, news, APIs), use `web_search` if available instead of guessing.
```

**3. session.rs — Brain Investigation block** (at line ~434, after the `grep` and bash search line):

Add:
```
* `web_search` (if available) for current/external information not in the codebase or your training data.
```

**4. session.rs — Builder mode tool hierarchy** (the last sentence of the Builder block, around line ~518):

From:
```
Investigate with the smart tools first - `semantic_search` for behavior questions, `code_search`/`symbol_lookup` for known names, `file_outline` before reading - and leave `grep`/bash searching as the last resort. Tell your subagents to do the same.
```
To:
```
Investigate with the smart tools first - `semantic_search` for behavior questions, `code_search`/`symbol_lookup` for known names, `file_outline` before reading - and leave `grep`/bash searching as the last resort. For current/external information not in the codebase or training data, use `web_search` if available. Tell your subagents to do the same.
```

**5. session.rs — api_tools gate** (after line 706, `let mut defs = tools::get_defs(maxp);`):

```rust
defs.retain(|t| t.name != "web_search" || config.is_claudinio_account());
```

**6. subagent.rs — subagent_defs signature change** (line 72):

From:
```rust
pub fn subagent_defs(mode: SubagentMode, mcp_defs: &[ToolDef], max_parallel: usize) -> Vec<ToolDef> {
```
To:
```rust
pub fn subagent_defs(mode: SubagentMode, mcp_defs: &[ToolDef], max_parallel: usize, config: &AgentConfig) -> Vec<ToolDef> {
```

**7. subagent.rs — subagent_defs body, add web_search gate** (after the filter at lines 75-77):

Add after the `spawn_agents`/`ask_user` filter line:
```rust
    tools.retain(|t| t.name != "web_search" || config.is_claudinio_account());
```

**8. subagent.rs — api_tools signature change** (line 92):

From:
```rust
fn api_tools(mode: SubagentMode, mcp_defs: &[ToolDef]) -> Vec<ToolDescription> {
```
To:
```rust
fn api_tools(mode: SubagentMode, mcp_defs: &[ToolDef], config: &AgentConfig) -> Vec<ToolDescription> {
```

And its body (line 93) from:
```rust
    subagent_defs(mode, mcp_defs, MAX_PARALLEL_AGENTS)
```
To:
```rust
    subagent_defs(mode, mcp_defs, MAX_PARALLEL_AGENTS, config)
```

**9. subagent.rs — call site update** (line 293, inside `run_subagent`):

From:
```rust
    let tools = api_tools(spec.mode, &mcp_defs);
```
To:
```rust
    let tools = api_tools(spec.mode, &mcp_defs, config);
```

### Data Flow
- `AgentConfig.is_claudinio_account()` → `true` when `override_api_key` is `None` AND `api_key` is non-empty (a `String`, not `Option`).
- This gates `web_search` at tool-definition time (before it reaches the LLM), so BYOK sessions never see the tool at all.
- Subagents inherit the parent session's `AgentConfig` via `run_subagent(config, ...)`, so the same gating applies uniformly.

### Integration Points
- `session.rs:api_tools` is called by `run_workflow` (main agent loop) — already has `&AgentConfig`, no signature change needed.
- `subagent.rs:api_tools` is called by `run_subagent` — already has `config: &AgentConfig` in scope at line 282; just needs to be passed down.
- No other callers exist (verified via codebase exploration).

## Tasks Summary

1. **provider-add-is-claudinio-account** — Add `is_claudinio_account()` method to `AgentConfig` in `src-tauri/src/agent/provider.rs`.
2. **session-prompt-websearch-bullet** — Add web_search guidance to SYSTEM_PROMPT CODE TOOLS section in `src-tauri/src/agent/session.rs`.
3. **session-brain-websearch-line** — Add web_search one-liner to Brain mode Investigation block in `src-tauri/src/agent/session.rs`.
4. **session-builder-websearch-line** — Add web_search mention to Builder mode tool-hierarchy line in `src-tauri/src/agent/session.rs`.
5. **session-gate-websearch** — Add `retain` gate for web_search in `api_tools` in `src-tauri/src/agent/session.rs`.
6. **subagent-thread-config** — Thread `&AgentConfig` through `subagent_defs` and `api_tools` in `src-tauri/src/agent/subagent.rs`, add web_search gate, update call site.
7. **cargo-check-verify** — Run `cargo check` in `src-tauri/` to verify compilation.


## Implementation Log — 2026-07-14 11:40
**Summary:** Gate web_search to claudinio-account sessions and add system-prompt guidance
**Changed files:** A  docs/plans/2026-07-14_gate-web-search-to-claudinio-account.md, M  src-tauri/src/agent/provider.rs, M  src-tauri/src/agent/session.rs, M  src-tauri/src/agent/subagent.rs
**Commits:** _(git unavailable or none)_
**Journal:** All 7 implementation tasks completed and verified with `cargo check` (zero errors, zero warnings).

Key decisions and learnings:
1. **`is_claudinio_account()` method**: Added to `AgentConfig` checking `override_api_key.is_none() && !api_key.is_empty()`. Note: `api_key` is a non-optional `String`, so the empty check is the only guard for unauthenticated sessions. `account_login` field also exists but wasn't used for this gate — the override_key pattern is the authoritative BYOK distinction.
2. **Prompt placement**: The web_search bullet was added to three places — the base `SYSTEM_PROMPT` (affects all modes), the Brain "Investigation" block, and the Builder tool-hierarchy one-liner. All phrased as "if available" so BYOK sessions don't get a misleading prompt.
3. **Subagent threading**: `subagent_defs` and `api_tools` in `subagent.rs` did NOT receive `&AgentConfig` before, but the call site (`run_subagent`) already had it. The mechanical plumbing was straightforward — no other callers to update.
4. **Gate placement**: The retain gate runs before the mode-specific match blocks in both session and subagent paths, so it applies uniformly to Brain, Builder, Explore, and Code modes.

**Task journal:**
- Add is_claudinio_account() helper to AgentConfig: Added impl AgentConfig block with is_claudinio_account() method after the struct definition (line 121). Checks: override_api_key.is_none() && !api_key.is_empty().
- Add web_search guidance to SYSTEM_PROMPT CODE TOOLS section: Added new bullet after 'Never use bash search tools...' in the SYSTEM_PROMPT raw string under '# 2. CODE TOOLS'.
- Add web_search one-liner to Brain mode Investigation block: Added 'web_search (if available) for current/external information...' bullet after the grep line in the Brain Investigation block.
- Add web_search mention to Builder mode tool-hierarchy line: Added 'For current/external information... use web_search if available' to the Builder mode tool-hierarchy sentence.
- Add retain gate for web_search in session api_tools: Added retain gate after tools::get_defs() in api_tools, before the match block.
- Thread &AgentConfig through subagent tool-list builders and add web_search gate: 1. Added 'config: &AgentConfig' to subagent_defs signature. 2. Added retain gate for web_search after the spawn_agents/ask_user filter. 3. Added config param to api_tools signature and passed it to subagent_defs. 4. Updated call site in run_subagent to pass config.
- Run cargo check to verify compilation: cargo check passed — zero errors, zero warnings.
