# confirm_plan_and_build — Agent-initiated Brain→Builder handoff

## Context

When a human enables Brain mode, the agent finishes planning and currently must say "the plan is ready, flip the toggle yourself." The user wants the agent to instead ask with `ask_user` ("Deseja implementar? 1) Sim 2) Não") and, if confirmed, call a tool that triggers the same Brain→Builder handoff that the "Continue with Builder" button does today.

This avoids a round-trip: the user confirms via the familiar ask_user question (with numeric options), and the transition fires within the same agent turn.

## Solution Design

### New tool: `confirm_plan_and_build`

- **Scope**: only available in **Brain** mode. Errors with "not in Brain mode" otherwise.
- **Origin-agnostic**: works regardless of whether Brain was entered by human or agent (unlike `exit_plan_mode` which rejects human origin).
- **Behavior**: identical to the existing `ManualBuilder` handoff path:
  1. Validates it's in Brain mode
  2. Persists a `Mode` record (`Builder`, `Human` origin)
  3. Auto-commits the latest plan file to git
  4. Creates a `HandoffSpec` with `reason: PlanExecution`
  5. The existing `run_to_completion` loop catches the `Handoff` outcome, calls `transition::link_session()` to create a linked successor session (fresh JSONL, empty history), rebuilds the ToolContext, and continues with the Builder kickoff message
- **Input schema**: empty (no parameters needed)

### Flow

```
mermaid
sequenceDiagram
    participant U as User
    participant A as Agent (Brain)
    participant S as Session Engine

    A->>U: ask_user("Plano pronto. Deseja executar?", ["1) Sim (Recommended)", "2) Não"])
    U->>A: "1) Sim"
    A->>S: confirm_plan_and_build()
    S->>S: Validate Brain mode ✓
    S->>S: Persist Mode(Builder, Human)
    S->>S: Auto-commit plan (git add + commit)
    S->>S: Set pending_handoff(PlanExecution)
    S->>A: Return "approved, handing off..."
    Note over A,S: Turn ends
    S->>S: run_to_completion loop catches Handoff
    S->>S: link_session() → new Builder session
    S->>S: Rebuild ToolContext, continue with plan inline
    S->>U: Builder session starts executing tasks
```

### Existing UI coexistence

The "Continue with Builder" button remains unchanged. Both paths produce the same linked-session result.

## Risks

- **Race condition**: if the user clicks "Continue with Builder" while the agent is about to call `confirm_plan_and_build`, both paths could create competing linked sessions. Mitigation: the Tauri command checks the session mode; if the agent's `confirm_plan_and_build` fires first, the old Brain session is gone and the button's `continue_with_builder` will fail gracefully with an appropriate error.
- **Thread safety**: the handler runs inside the agent's async session workflow, which already holds references to `AppState`, `TransitionMaps`, etc. — same risk profile as `exit_plan_mode`'s handoff path. No new concerns.

## Non-goals

- No change to `exit_plan_mode` or its origin check
- No change to the Human toggle or "Continue with Builder" button
- No change to the golden task loop or context handoff
- No new UI components (the tool call result renders in the existing tool call UI)
- No changes to the `tasks_set` LLD gate

## Low-Level Design

### Files to touch

| File | Change |
|------|--------|
| `core/src/agent/tools/mod.rs` | Add `confirm_plan_and_build_def()`; add `"confirm_plan_and_build"` to the intercepted tool names in `execute()` |
| `core/src/agent/session.rs` | Add `"confirm_plan_and_build"` to the tool routing in `run_workflow()`; add handler arm in `handle_mode_switch()`; add `confirm_plan_and_build_def()` to `api_tools()` under `SessionMode::Brain` |

### Detailed changes

#### 1. `core/src/agent/tools/mod.rs` — Tool definition

Add after `exit_plan_mode_def()` (~line 478):

```rust
/// Definition of the confirm_plan_and_build tool. Only offered in Brain mode.
/// The agent should call this after the user confirms via ask_user that they
/// want to execute the plan. Triggers a handoff to a new Builder session.
pub fn confirm_plan_and_build_def() -> ToolDef {
    ToolDef {
        name: "confirm_plan_and_build".into(),
        description: "Confirm the plan is ready and hand off to Builder mode for execution. Call this AFTER the user confirmed via ask_user that they want to build. Only works in Brain mode — creates a new Builder session with the plan inlined and tasks carried over.".into(),
        input_schema: serde_json::json!({"type": "object", "properties": {}, "required": []}),
    }
}
```

#### 2. `core/src/agent/session.rs` — Add to `api_tools()` (~line 836)

In the `SessionMode::Brain` arm, add `confirm_plan_and_build_def()` next to `exit_plan_mode_def()`:

```rust
SessionMode::Brain => {
    defs.push(write_plan_def());
    defs.push(exit_plan_mode_def());
    defs.push(confirm_plan_and_build_def());  // <-- new
}
```

Add the import at the top (either import `confirm_plan_and_build_def` or reference via `tools::confirm_plan_and_build_def()`).

#### 3. `core/src/agent/tools/mod.rs` — Intercept in `execute()` (~line 694)

Add `"confirm_plan_and_build"` to the intercepted list:

```rust
"enter_plan_mode" | "exit_plan_mode" | "confirm_plan_and_build" => {
    Err("mode switch tools are handled by the session orchestrator".into())
}
```

#### 4. `core/src/agent/session.rs` — Route in `run_workflow()` (~line 2268)

Change the condition from:
```rust
let block = if tool_name == "enter_plan_mode" || tool_name == "exit_plan_mode" {
```
to:
```rust
let block = if tool_name == "enter_plan_mode"
    || tool_name == "exit_plan_mode"
    || tool_name == "confirm_plan_and_build"
{
```

#### 5. `core/src/agent/session.rs` — Handler arm in `handle_mode_switch()` (~line 3077)

Add a third arm for `confirm_plan_and_build` after the `exit_plan_mode` arm:

```rust
"confirm_plan_and_build" => {
    if mode != SessionMode::Brain {
        (
            "Not in Brain mode. confirm_plan_and_build can only be called from Brain mode."
                .into(),
            Some("invalid".into()),
        )
    } else {
        // Persist mode record
        store.try_append(&SessionRecord::Mode {
            mode: SessionMode::Builder.as_str().into(),
            origin: ModeOrigin::Human.as_str().into(),
            ts: now_ms(),
        });
        crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);

        // Set handoff
        *pending_handoff = Some(HandoffSpec {
            reason: HandoffReason::PlanExecution,
            next_mode: SessionMode::Builder,
            next_origin: ModeOrigin::Human,
            first_message: "Plan approved. Read the plan and execute the tasks.".into(),
            golden_cycle: 0,
            golden_stalls: 0,
            golden_last_pending: vec![],
        });

        // Auto-commit the plan (same code as exit_plan_mode)
        let auto_commit = ctx.agent_config.as_ref()
            .map(|c| c.auto_commit_plan)
            .unwrap_or(true);
        if auto_commit {
            if let Some(root) = &ctx.workspace_root {
                let plan_save_path = ctx.plan_save_path.as_deref();
                if let Some(plan_path) = crate::agent::tools::write_plan::latest_plan_path(root, plan_save_path) {
                    let fname = plan_path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("plan");
                    let slug = if fname.len() > 11 && &fname[4..5] == "-" {
                        &fname[11..]
                    } else {
                        fname
                    };
                    let commit_msg = format!("docs(plan): {}", slug);
                    let _ = std::process::Command::new("git")
                        .arg("-C").arg(root)
                        .arg("add")
                        .arg(plan_path.to_string_lossy().as_ref())
                        .output();
                    let _ = std::process::Command::new("git")
                        .arg("-C").arg(root)
                        .arg("commit")
                        .arg("-m")
                        .arg(&commit_msg)
                        .arg("--no-verify")
                        .output();
                }
            }
        }

        (
            "Plan approved. Handing off to Builder mode in a new session. "
                .into(),
            None,
        )
    }
}
```

### Key design rationale

- **`next_origin: Human`**: mirrors the "Continue with Builder" button (the user explicitly confirmed via ask_user). This is a semantic distinction from `exit_plan_mode` which uses `Agent` (the agent decided autonomously).
- **`reason: PlanExecution`**: same as `exit_plan_mode`. The visual/logging differentiation comes from the origin field.
- **No change to `ModeOrigin` enum**: `confirm_plan_and_build` always uses `Human` origin, no new variants needed.
- **No change to `HandoffReason` enum**: `PlanExecution` fits semantically — the plan is being executed.

### Integration points

```
mermaid
flowchart LR
    A[User toggles Brain] --> B[Agent plans]
    B --> C[ask_user: implementar?]
    C -->|Sim| D[confirm_plan_and_build]
    C -->|Não| B
    D --> E[handle_mode_switch]
    E --> F[Mode record Builder/Human]
    E --> G[Auto-commit plan]
    E --> H[pending_handoff = HandoffSpec]
    H --> I[run_to_completion catches]
    I --> J[link_session]
    J --> K[New Builder session]
    
    L[User clicks<br/>Continue with Builder] --> M[continue_with_builder Tauri cmd]
    M --> J
    
    style D fill:#4a9,color:#fff
    style K fill:#48a,color:#fff
```

### Tasks summary

1. **Add tool definition**: `confirm_plan_and_build_def()` in `tools/mod.rs`
2. **Wire tool in execute() routing**: add intercept in `tools/mod.rs`
3. **Wire tool in run_workflow() routing**: add to condition in `session.rs`
4. **Add handler arm in handle_mode_switch()**: implement the Brain→Builder handoff
5. **Add to api_tools()**: expose in Brain mode
6. **Verify**: confirm tool appears in Brain mode, succeeds with handoff, fails in Builder mode, and the new Builder session receives the plan


## Implementation Log — 2026-07-21 11:19
**Summary:** Add confirm_plan_and_build tool for agent-initiated Brain→Builder handoff
**Changed files:** A	docs/plans/2026-07-21_confirm-plan-and-build.md
**Commits:** cf72e75 docs(plan): confirm-plan-and-build
**Journal:** Tool confirm_plan_and_build implemented. 5 source changes across 2 files, build compiles clean.

Key design decisions:
- next_origin: Human (matches "Continue with Builder" button semantics, user confirmed via ask_user)
- HandoffReason::PlanExecution (same as exit_plan_mode)
- Auto-commit logic duplicated from exit_plan_mode (cleaner than shared helper for this small block)
- Tool only available in Brain mode; errors clearly if called from Builder

Separated from exit_plan_mode because exit_plan_mode blocks when origin is Human (user-enabled Brain). confirm_plan_and_build is intentionally Human-origin — the user explicitly confirmed via ask_user — so it needs its own handler that skips that guard.

No UI changes needed: the tool call result renders in the existing tool call UI, and the "Continue with Builder" button remains unchanged as a parallel path.

**Task journal:**
- Add confirm_plan_and_build_def() in tools/mod.rs: Added confirm_plan_and_build_def() after exit_plan_mode_def(); ToolDef defined with empty input_schema, description about plan confirmation and Builder handoff
- Intercept confirm_plan_and_build in execute(): Pattern extended to match confirm_plan_and_build alongside enter_plan_mode and exit_plan_mode
- Route confirm_plan_and_build to handle_mode_switch in run_workflow(): Routing condition extended to include confirm_plan_and_build; Now routes to handle_mode_switch() correctly
- Add confirm_plan_and_build handler in handle_mode_switch(): Full handler arm added with Brain mode validation; Persists Mode(Builder, Human) record; Auto-commits plan via git same as exit_plan_mode; Sets pending_handoff with PlanExecution reason and Human origin; Returns 'Plan approved. Handing off to Builder mode...'
- Add to api_tools() in Brain mode: confirm_plan_and_build_def() added to Brain mode tool list; Tool becomes available only in Brain mode
- Verify: build passes: cargo build passes with 0 new errors (only pre-existing warnings)


## Implementation Log — 2026-07-21 11:31
**Summary:** Eager MCP pre-warm eliminates first-message delay in TUI
**Changed files:** M	cli/src/tui/app.rs
**Commits:** 701305d perf(cli/tui): eager MCP pre-warm to eliminate first-message delay on Enter
**Journal:** Root cause analysis: the first-message delay was a classic blocking-before-draw problem. `submit()` -> `app.commit()` only enqueues to a vector (stale draw), then `start_turn().await` blocks on `ensure_mcp_connected` which spawns actual stdio MCP processes on the first call. Only after `handle_event` returns does the main loop call `commit_and_draw()`.

Fix: one line added right after ChatCtx creation — eagerly call `ensure_mcp_connected` before the terminal even starts. The stdio spawn cost shifts to startup time (while the user sees the welcome banner), so the first real submit finds a hot connection cache and returns instantly, eliminating the visual delay between pressing Enter and seeing the message appear.

**Task journal:**
- Investigate first-message delay in TUI chat area: Root cause: `start_turn()` é awaited dentro de `submit()`, que é awaited dentro de `handle_event()`, e `commit_and_draw()` (que efetivamente renderiza a mensagem do usuário no terminal) só roda no loop principal DEPOIS que handle_event retorna.; Na primeira mensagem, `ensure_mcp_connected` dentro de `start_turn()` faz a conexão real (spawn de processos stdio de MCP servers), que é o principal gargalo.; `app.commit(user_lines)` só enfileira no vetor `to_commit` — não desenha nada. O desenho real acontece em `commit_and_draw()`, chamado no loop main após handle_event retornar.
- Add eager MCP pre-warm before terminal init: Added `chat.ws.ensure_mcp_connected(&chat.config).await;` right after ChatCtx creation, before terminal init. This pre-warms the MCP connection while the user is still seeing the welcome message, so by the time they type their first message, `start_turn()` finds the cache hot and returns instantly.; Compila sem erros em `claudinio-cli`.; Commit: 701305d perf(cli/tui): eager MCP pre-warm to eliminate first-message delay on Enter
