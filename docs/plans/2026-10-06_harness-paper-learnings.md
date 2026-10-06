# Harness learnings from "An Empirical Study of Harness Design for Coding Agents"

**Status:** proposal, awaiting review. Nothing is implemented.
**Written against:** `main` at `39532d0` (0.5.0). Paths are relative to `src-tauri/src/` unless they start with `src/`.
**Sources:** the paper ([arXiv 2609.20804](https://arxiv.org/abs/2609.20804)), the code at that commit, and a measurement of the 509 session files in this workspace's `.claudinio/sessions` (§2).

## Context

The paper ran 176 harness configurations (4 open models, SWE-Bench Verified and Terminal-Bench 2.1) and isolated three levers: context management, a planning scaffold, and the tool surface. Its headline results:

| Lever | What the paper measured |
|---|---|
| Context management | At a 32k window, no management fails 78.7% of tasks by overflow; any managed tier fails 0%. The accuracy gain shrinks from 35.7 pp (32k) to 2.7 pp (128k). Elision followed by summarization is the cheapest tier. |
| Recall of elided content | Median zero calls. Elision with recall is 0.36 pp *worse* on average than elision alone. |
| Planning scaffold | +11.6 pp for the 30B model at higher cost; about 30% cheaper for the two strong models with a small accuracy loss (−0.4 to −2.0 pp). |
| Tool surface | The 30B model gains +15 pp from structured tools. Nemotron 550B is +3.6 pp and 53% cheaper with bash only. Mistral 128B loses 23.2 pp with bash only on SWE-Bench. |

The paper figures above come from an automated read of the arXiv HTML and should be checked against its tables before being quoted elsewhere.

Three things about the paper change how its results apply here:

1. **Its "planning" is our task system, not Brain mode.** Planning in the paper is a persistent task plan the agent maintains through an `update_plan` tool. That is `tasks_get`/`tasks_set` and the Task Panel, which are always on. The paper says nothing about a separate read-only design phase.
2. **Its cost ignores prompt caching.** Cost is tokens times list price. Eliding context at 60% of the window rewrites the prefix, which on a cached provider is a cold re-read. `docs/plans/2026-10-05_jev-no-harness.md` §6 already forbids rewriting history mid-session for this reason, and this plan keeps that rule.
3. **It never adapts the harness to the model.** Every ablation is a fixed setting. "Tools for small models, bash for large ones" is our reading of its per-model rows, and the Mistral row contradicts the simple version of it.

## 1. What the harness already does

| Paper lever | Claudinio Code today | Verdict |
|---|---|---|
| Elision before summarization (T4) | `prune.rs` (Jev decides, verbatim) → context handoff → `compact_history` | Same shape, with a better elision than the paper's rule |
| Recall | `output_trim.rs` writes the full output to a temp file and names the path | Enough. A recall tool is not worth building |
| Planning scaffold | Mandatory task system in every mode, plus the completion judge and golden loop | In place |
| Context reset between phases | Brain → Builder starts a new session seeded with the plan (`transition.rs`) | In place, and stronger than anything the paper tests |
| Tool surface per model | One catalog for every model (`session.rs::api_tools`) | Gap |
| Limits relative to the window | Fixed at 200k (`MAX_CONTEXT_TOKENS`), handoff floor 120k | Gap, and the largest one |
| Elision without Jev | None. No Jev backend means straight to handoff or summary; subagents get nothing | Gap |

## 2. Evidence from our own sessions

Measured on 509 session files created 2026-07-05 to 2026-10-06: 844 completed runs, 16,945 tool calls, 77.9M input and 12.1M output tokens. Limits of the data: one user, one workspace, main-session transcripts only. Subagent transcripts and thinking are not persisted, so neither can be broken down.

**"Carried mass"** below is characters multiplied by the number of later requests in the session that re-read them. It approximates where input tokens go, cached or not.

### 2.1 Where input goes

| Category | Share of carried mass |
|---|---|
| `read_file` results | 44.0% |
| `bash` results | 9.3% |
| `spawn_agents` reports | 7.8% |
| `file_outline` results | 6.0% |
| `tasks_set` arguments | 5.1% |
| `edit_file` arguments | 4.4% |
| `grep` results | 3.6% |
| All tool results together | 77.8% |
| All assistant and user prose | 2.9% |

- 7.5% of tool results are 8k chars or larger, and they hold 64.3% of all result characters.
- `file_outline` averages 21,930 chars per call against 5,467 for `read_file`. The system prompt tells the model to call it *before* reading, to save context.
- 10.2% of `read_file` calls repeat an earlier call with the same path and range; the superseded copies are 21.1% of all `read_file` characters. Counting any range of the same path, 57.7% of calls are re-reads.

### 2.2 Where visible output goes

| Category | Share of generated characters |
|---|---|
| `tasks_set` arguments | 24.3% |
| `edit_file` arguments | 18.5% |
| Assistant prose | 15.3% |
| `bash` arguments | 13.7% |
| `write_plan` arguments | 11.0% |

`tasks_set` is a full replacement, so every status change re-emits the whole list: 1,232 calls, median 1,851 chars each, median 5 per session and up to 43. It is the largest single thing the main session writes. Caveat: all visible main-session output is roughly 30% of billed output tokens at 3.5 chars per token; the rest is thinking and subagents.

### 2.3 Context pressure

| Peak context in a session | Sessions |
|---|---|
| under 64k | 234 (46.0%) |
| 64k to 120k | 99 (19.5%) |
| 120k or more | 70 (13.8%) |
| not recorded | 106 (20.8%) |

37 summarizing compactions and 7 context handoffs in three months. On 200k-window models, management is a cost question for about one session in seven, which matches the paper's large-window result.

A simulation of the simplest rule (replace tool results outside the first message and the last 6 with a 300-char head) on the 105 sessions that peaked at 96k or more frees a median 70% of history characters (10th percentile 51%). All 105 clear `prune::MIN_REDUCTION` (25%).

### 2.4 Tools the model actually uses

| Tool | Calls | Share |
|---|---|---|
| `bash` | 5,351 | 31.6% |
| `read_file` | 4,697 | 27.7% |
| `edit_file` | 1,734 | 10.2% |
| `grep` | 1,056 | 6.2% |
| `code_search` + `semantic_search` + `file_outline` | 645 | 3.8% |
| `find_references` + `symbol_lookup` + `go_to_definition` | 12 | 0.07% |

About 23% of `bash` calls are `grep`, `cat`, `ls`, `find`, `sed`, `head`, `tail` or `wc`, despite the prompt rule against bash search. The three LSP-style tools are paid for in every request prefix and were called 12 times.

### 2.5 Who decides to plan

| Fact | Value |
|---|---|
| Fresh sessions with a human first prompt | 457 |
| Started in Brain by the user | 148 (32%) |
| Started in Builder, later switched to Brain by the user | 5 |
| `enter_plan_mode` calls by the model | 8 |
| Median first prompt, Brain / Builder | 164 / 36 chars |
| Median assistant rounds, Brain / Builder | 40 / 6 |

The plan-or-build decision is made by the human, once, at session start. The model's own path to planning is effectively unused.

### 2.6 The fixed prefix

The context meter of a session's first status, minus the history it had at that point, estimates what every request carries before any conversation: system prompt, tool schemas, skills, specs, MCP tools.

| Sessions created in | Samples | Median prefix (tokens on the harness's meter) |
|---|---|---|
| July | 149 | 19,305 |
| August | 20 | 30,030 |
| October | 3 | 33,529 |

- The two cleanest October samples (a status written before any assistant turn) read 32,342 in Builder and 33,529 in Brain.
- By source size, the built-in system prompt and the 20 built-in tool schemas account for roughly 10k of that. The rest is skills, plugins and MCP tool schemas, which the session files do not break down.
- Consequence for small windows: in this setup the Standard profile's prefix is as large as a 32k window. A local 32k model cannot run it at all, whatever the context management does.
- The estimate is inflated by the last reply's output tokens when a run completed, and October has three samples. Phase 0 replaces it with an exact, itemized measurement.

### 2.7 An open question about caching

The session files record $84.42 of cost, of which $33.01 is labelled input and $1.06 cache read. The Jev plan cites the proxy measuring about 96% of executor input as cache read. Both cannot describe the same traffic unless cache reads are priced near zero or not reported to the client. This plan assumes the cache works as the proxy says, which is why it never rewrites history early. Phase 0 should confirm that assumption from the provider's usage fields, because if it is wrong, earlier elision becomes worth reconsidering.

## 3. Brain and Builder, or one mode?

**Recommendation: one entry point, two phases.** Keep Brain and Builder inside the engine, stop making the user choose between them, and let rules plus Jev pick the starting phase.

Brain/Builder is four things bundled together, and three of them should not be decided by a classifier:

| What the mode carries | Why it should stay a hard boundary |
|---|---|
| Write permission (no edit tool, bash write gate) | It is a guarantee enforced in code. Jev is injectable from its state and scores 67–68% on independent benchmarks; it should not be the thing that grants writes |
| Model (`brain_model` vs `builder_model`) | Changing model mid-session is a cold prefix on the new model |
| Context reset at plan → build | Execution starts from the plan, not from the exploration. This is the cheapest context management in the product |
| Workflow prompt (interview, design, tasks) | This is the only part that is a judgment about the request |

What changes is who makes the fourth decision:

- The mode control becomes **Auto | Brain | Builder**. An explicit Brain or Builder is obeyed as today.
- Auto decides **once, on the first prompt of a fresh session**, before the first request. There is no cache to lose at that point, which is the only place the Jev plan's §6 allows such a decision. §2.5 shows that is where the decision is made anyway.
- Deterministic rules run first: a successor session, a session with pending tasks, or a non-Standard profile goes to Builder. Jev is asked only about meaning, with two `noul` questions over the prompt text: does this need decisions only the user can make, and does it need a written design before code.
- The thresholds are asymmetric. A wrong Builder verdict is today's default, and the model can still call `enter_plan_mode`. A wrong Brain verdict costs the user an interview they did not want, so Brain needs high confidence.
- Fail-open: no Jev backend, timeout or error means Builder.

**Why not trust it immediately.** The Jev plan's own survey found per-prompt routing weak or negative in other harnesses. So this ships in shadow first, and before that it is calibrated offline: the 457 first prompts in §2.5 are already labelled by the user's own choice and can be replayed through Jev to pick thresholds.

## Solution Design

Six phases. Each is independently shippable and each has a gate in §Verification. Phases 1 to 3 are deterministic and involve no new Jev calls.

### Phase 0 — Model profile and measurement (no behaviour change)

- Know each model's real context window: Claudinio models from the existing constant, catalog providers from models.dev `limit.context` (already fetched, currently discarded), custom providers from the `/models` probe or a field in the form, local models from the served window.
- Record per run what the harness could not see until now: model id, context window, tool surface, per-tool call counts for subagents, and the prefix size itemized (system prompt, built-in tools, MCP tools, skills, specs).
- Record cached and uncached input tokens separately, to settle §2.7.
- Commit the session-statistics script used for §2 so every later gate is measured the same way.

### Phase 1 — Limits follow the real window

- Replace the fixed constants with a per-model `ContextBudget`.
- **Models with a 200k window or more keep today's numbers exactly** (handoff slider, 150k compaction, 200k ceiling).
- **Smaller windows scale by the paper's ratios** over the space the conversation can actually use: soft line at 60%, hard line at 85%, measured after subtracting the reply reserve and the fixed prefix.
- The per-result cap (24k chars), the trim budget (20k) and the verbatim tail (20k tokens) scale down with the window instead of being constants.
- Subagents use the same budget for their own model, and stop with a report instead of sending a request that cannot fit.
- The context meter and the overflow message show the real window.

### Phase 2 — Elision that does not depend on Jev

Three stages at the existing shed-weight line, never earlier:

| Stage | What it removes | Needs Jev |
|---|---|---|
| 0. Lossless | Superseded snapshots: older `tasks_get`/`tasks_set`, and a `read_file` result re-read later with the same path and range | No, always runs |
| 1. Judged | Today's `prune::plan`, on what is left | Yes |
| 2. By age | Oldest tool results cut to the 300-char head until the target is met | No, runs when stage 1 is unavailable or not accepted |

- All three produce the same `Decision` and the same `Pruned` record, so resume behaves as it does today.
- Stage 2 never drops a call, only its output: knowing a call was made is cheap and prevents redoing it.
- Subagents get stages 0 and 2 as well. Today a subagent without Jev has no context management at all.
- No recall tool. The paper measured it as unused, and the model can re-read a file.

### Phase 3 — Task updates as deltas

- Add `tasks_update`: one task id, a new status, journal lines to append.
- `tasks_set` stays for creating and restructuring the list. The prompts point status changes at `tasks_update`.
- Gates that live in `tasks_set` today (Brain LLD gate, quality gate, golden protection) apply to both.

### Phase 4 — Auto routing (§3)

- Offline calibration on the 457 labelled prompts, then shadow, then on.
- In shadow, the verdict is recorded next to what the user actually chose and changes nothing.
- When on, an Auto verdict of Brain enters with agent origin, so the flow runs interview → plan → tasks → build without a manual flip. The interview's final confirmation remains the user's approval point.

### Phase 5 — Tool surface per model

Chosen once at run start from the model profile and fixed for the session, never per turn.

| Surface | For | Composition |
|---|---|---|
| Full | Default for every model until measured otherwise | Today's catalog |
| Lean | Opt-in per model, for models fluent in shell | Drops `go_to_definition`, `find_references`, `symbol_lookup`, `grep`, `list_dir`; the prompt's tool section is rewritten to match |
| Compact | Windows under 64k (local models) | A new lean profile in the mould of `GitSync`: short prompt, structured file tools, task scaffold kept, no browser, no MCP, no Brain protocol |

- Lean is not a default for any model in this plan. The paper's one cross-family data point lost 23 pp with bash only, and our data covers only the main session. It becomes a default per model only after its gate.
- Compact exists because of §2.6: the Standard prefix alone is about the size of a 32k window. The paper shows small models need structured tools and the task scaffold, so Compact keeps both and cuts everything else. Its target is a prefix under 25% of the usable window.
- The prefix grew from about 19k to about 33k in three months, mostly outside the built-in catalog. Whether skills and MCP schemas need a budget in the Full surface too is decided after Phase 0 itemizes them.

### Phase 6 — Output formats (found while measuring, optional)

Not from the paper, same goal, each one small:

- `file_outline`: one line per symbol instead of pretty-printed JSON.
- `list_dir`, `grep`, `code_search`: compact line formats.
- An exact repeat of a `read_file` whose earlier result is still verbatim in history returns a short pointer to it.

## Decisions to confirm before implementation

| # | Decision | Recommendation |
|---|---|---|
| D1 | The handoff slider goes to 256k but the pre-flight guard refuses at 200k. A slider above 200k never triggers a handoff, and above 190k the compaction fallback (`slider + 10k`) is unreachable too | Clamp the soft line to the hard line minus 10k, and the hard line to 90% of the ceiling. Default settings are unaffected |
| D2 | In Compact, may the main session edit files directly instead of delegating to subagents? | Yes. Delegation costs a second prefix and a second context on a model that has neither to spare |
| D3 | When Auto picks Brain, does the plan execute without a manual flip? | Yes, with agent origin |
| D4 | Does Lean drop `grep` and `list_dir`, or only the three unused LSP tools? | Start with only the three unused tools, measure, then decide on the rest |
| D5 | Auto sends the first prompt text to Jev | Covered by the existing Jev setting; state it in the setting's description |
| D6 | Phase 6 in or out of this work | In, after Phase 3 |

## Risks

- **Windows reported wrong.** A catalog or probe value that is too large reintroduces overflow; too small wastes the window. Unknown always falls back to today's 200k, and the value is shown in the model picker.
- **Stage 2 removes something still needed.** It is the fallback, it keeps a 300-char head and the call itself, and the pinned first message and recent tail are untouched. The cost of a miss is a re-read.
- **Auto routes to Brain when the user wanted a quick edit.** Mitigated by the asymmetric threshold, the shadow gate, and the explicit Builder option one click away.
- **Compact changes product behaviour for local models.** It is a new profile selected by window size, so existing large-window local setups are unaffected.
- **Cache.** No phase adds a point where history is rewritten. Stage 0 and stage 2 run only where the harness already rewrites.
- **`tasks_update` drift.** If the model updates a task it has not loaded, the tool returns the current list in its error.

## Non-goals

- A recall tool for elided content.
- Elision at 60% of the window on cached providers.
- Bash-only as a default for any model.
- Changing model, tools or mode per turn.
- Using windows above 200k.
- Removing Brain or Builder from the engine.
- A benchmark runner. Gates use fixtures and session telemetry; a headless eval is a separate project.

## Low-Level Design

### Phase 0

- `agent/provider.rs`
  - `ProviderEntry`: add `model_context_limits: HashMap<String, u32>` with `#[serde(default)]`.
  - `ResolvedProvider`: add `context_window: Option<u32>`; filled in `resolve_provider` from the entry, and in `resolve_provider_live` from the `window` it already computes for local models.
  - New `AgentConfig::context_window_for(&self, model: &str) -> u64`: pure, no server start. Unknown returns `session::MAX_CONTEXT_TOKENS`; known values are capped at it.
- `agent/provider/catalog.rs::model_snapshots`: also return the `context` field that `trim_catalog` already keeps.
- `commands/providers.rs`: the three writers of a `ProviderEntry` (`openrouter_login`, `connect_provider`, `save_custom_provider`) store the new map. `parse_models_response` additionally reads a context size when the endpoint reports one. Entries saved before this change are backfilled from the on-disk catalog cache at config load, without a network call.
- `src/components/CustomProviderModal.tsx`: optional context-window field per model.
- `agent/persist.rs`: new `SessionRecord::RunConfig { model, context_window, prefix: { system, tools, mcp, skills, specs }, surface, ts }`, written once per run. `load_records` skips lines it cannot parse, so an older build reading a newer file drops the record instead of failing.
- `agent/session.rs`: the cost ledger already tracks `total_cache`; persist it on `Status` next to the input and output totals.
- `agent/subagent.rs`: `SubagentResult` gains per-tool call counts, carried on `SubagentDone` and persisted.
- `scripts/session-stats.py`: the §2 measurement.

### Phase 1

- New `agent/budget.rs`:

  ```rust
  pub struct ContextBudget {
      pub window: u64,            // what the model accepts, capped at MAX_CONTEXT_TOKENS
      pub soft: u64,              // prune, then handoff   (was effective_handoff_threshold)
      pub hard: u64,              // summarizing compaction (was COMPACT_THRESHOLD)
      pub ceiling: u64,           // refuse to send above   (was MAX_CONTEXT_TOKENS)
      pub tool_result_chars: usize, // was MAX_TOOL_RESULT_CHARS (24_000)
      pub trim_chars: usize,      // was output_trim::BUDGET_CHARS (20_000)
      pub tail_tokens: u64,       // was TAIL_MAX_TOKENS (20_000)
  }
  ```

  - `window >= MAX_CONTEXT_TOKENS`: `soft` = slider, `hard` = `max(150k, soft + 10k)`, `ceiling` = 200k, caps unchanged. A unit test pins 120k / 150k / 200k for the default config.
  - Otherwise, with `U = window − reply_reserve` and `S = U − prefix_tokens`: `soft = prefix + 0.60·S`, `hard = prefix + 0.85·S`, `ceiling = U`, `tool_result_chars = min(24_000, 0.25·S·3)`, `trim_chars = tool_result_chars·5/6`, `tail_tokens = min(20_000, 0.30·S)`.
  - `reply_reserve` is `ResolvedProvider::max_output_tokens` when known.
- `agent/session.rs`: every use of `MAX_CONTEXT_TOKENS`, `COMPACT_THRESHOLD` and `effective_handoff_threshold()` in the run loop reads the budget instead. Sites: `effective_compact_threshold`, `context_limit`, `try_verbatim_compaction`, `maybe_context_handoff`, both auto-compact blocks, both pre-flight guards, `compute_tail_turns`, the two `tool_result_block` call sites, and the `maxContextTokens`/`compactThreshold` fields of the status event. The budget is recomputed when the mode, and therefore the model, changes.
- `agent/output_trim.rs`: `BUDGET_CHARS` becomes a parameter.
- `agent/subagent.rs`: the prune trigger uses the budget for `builder_model`; above `ceiling` after pruning, return a `SubagentResult` with status `context_full`.
- `llama/mod.rs::effective_ctx` keeps taking the raw slider value, so the served window does not depend on the derived soft line.
- `commands/agent.rs` config getter: return the budget for the builder model instead of the two constants.
- `src/components/ChatPanel.tsx`: no logic change; the 256k/192k initial values are replaced by what the backend reports.

### Phase 2

- `agent/prune.rs`
  - `plan_lossless(history) -> Decision`: superseded `tasks_get`/`tasks_set` calls go to `drop_calls`; a `read_file` result with a later result for the same `(path, start_line, end_line)` goes to `truncate_results`.
  - `plan_by_age(history, target_chars) -> Outcome`: oldest unpinned results first, `Action::DropResult` only, stop at the target. `stage` is `"rules"`.
  - `is_pinned` and `PRESERVE_RECENT_MESSAGES` are reused unchanged.
- `agent/session.rs::try_verbatim_compaction`: apply `plan_lossless`, then `prune::plan` when a backend exists, then `plan_by_age` if the result is not accepted by `accept_prune`. One `Pruned` record per accepted outcome, with the stage in `stats`.
- `agent/subagent.rs`: the same ladder, in memory.

### Phase 3

- `agent/tools/tasks.rs`: `execute_update(id, status, journal_append)`, built on the same load, merge and gate functions as `execute_set` (`check_brain_lld_gate`, `check_quality_gate`, `merge_preserving_golden`).
- `agent/tools/mod.rs`: `tasks_update` definition and dispatch.
- `agent/session.rs`: `SYSTEM_PROMPT` §1 and the Builder block point status changes at `tasks_update`.
- `agent/subagent.rs::subagent_defs` removes only `spawn_agents` and `ask_user`, so subagents are offered `tasks_get` and `tasks_set` today. Confirm whether that is intended; if not, remove both from the subagent catalog here.
- `src/components/tool-renderers/`: a renderer for the new call.

### Phase 4

- New `agent/route.rs`
  - `rules(session_facts) -> Option<SessionMode>`.
  - `questions()` returning the two `noul`s via `jev::noul`; state is the first prompt text only, no attachments.
  - `decide(p_decisions, p_design) -> SessionMode` with thresholds as constants fixed by test, in the style of `prune::KEEP_RESULT_THRESHOLD`.
- `agent/persist.rs`: `SessionRecord::ModeRoute { verdict, p_decisions, p_design, source, shadow, ts }`, alongside `ContinuationJudge`.
- `agent/jev.rs`: `JevPrefs` gains `route: off | shadow | on`, default `shadow`.
- `commands/agent.rs`: before the first request of a fresh session whose mode control is Auto, run the router; when on and the verdict is Brain, `ModeCtl::set(Brain, Agent)`, a `Mode` record and a `ModeChanged` event carrying the reason.
- `src/components/ChatPanel.tsx`: three-state mode control. `src/lib/ipc.ts`: `SessionMode` gains `"auto"` for the control only; the engine's `SessionMode` enum is unchanged.
- `scripts/route-calibrate.py`: replays first prompts from `.claudinio/sessions` through the configured Jev backend and prints the score distributions per label.

### Phase 5

- `agent/session.rs`
  - `enum ToolSurface { Full, Lean }`, resolved at run start from a per-model setting; `api_tools` filters by it. `SYSTEM_PROMPT` §2 becomes a function of the surface.
  - `PromptProfile::Compact`, selected when `context_window_for(model) < 64_000`: its own prompt constant and tool list, modelled on the `GitSync` branch of `system_prompt` and `api_tools`. `edit_file` is offered in the main session for this profile only (D2).
- `agent/provider.rs`: `AgentConfig` gains `tool_surface: HashMap<String, String>` keyed by model id.
- `src/components/settings/SettingsModels.tsx`: the per-model surface control.

### Phase 6

- `agent/tools/mod.rs`: `file_outline`, `list_dir`, `grep`, `code_search` format their results as lines.
- `ReadTracker`: remember the `tool_use_id` of the last read per `(path, range)` and the file's mtime; `read_file` returns the pointer only when that id is not in any `Pruned` or behind a `Compacted` record and the mtime is unchanged.

## Tasks summary

1. P0: context window per model (provider entry, catalog snapshot, custom probe, local).
2. P0: `RunConfig` record, subagent tool counts, `scripts/session-stats.py`.
3. P1: `ContextBudget` and its unit tests, including the pinned 200k case.
4. P1: wire the budget through `session.rs`, `subagent.rs`, `output_trim.rs`, the config getter and the UI.
5. P2: `plan_lossless` and `plan_by_age` with tests; the three-stage ladder in the session and in subagents.
6. P3: `tasks_update`, prompts, renderer.
7. P4: `route.rs`, `ModeRoute`, calibration script, shadow wiring, three-state control.
8. P5: `ToolSurface` and the Lean setting; `PromptProfile::Compact`.
9. P6: line formats; duplicate-read pointer.

## Verification

Every phase passes the repo checks: `pnpm exec tsc --noEmit`, `pnpm exec vitest run`, `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --lib`. New behaviour is test-first against the existing HTTP stub (`spawn_stub` in `session.rs`).

| Phase | Gate |
|---|---|
| P1 | Default config yields exactly 120k / 150k / 200k. A stubbed 32k and a stubbed 64k model run a long scripted session with zero requests above the ceiling |
| P2 | On every stored session that peaked at 96k or more, stages 0 + 2 free at least 25%. No user or assistant text changes, by construction and by test. A resumed session equals the live one |
| P3 | Over the following two weeks of real use, `tasks_set` + `tasks_update` fall from 24% of generated characters to under 8% |
| P4 → shadow | Offline, on the 457 labelled prompts, the two score distributions separate; a threshold exists with at most 3% of Builder-labelled prompts routed to Brain |
| P4 → on | In shadow, agreement with the user's choice of at least 90% over 200 fresh sessions |
| P5 Lean as a default for a model | At least 50 runs per surface on that model with no drop in quality-gate pass rate or judge-clean completions, and lower cost per run |
| P5 Compact | A stubbed 32k model completes a multi-file scripted task; prefix under 25% of the usable window |
| P6 | Mean `file_outline` result under 5k chars on this repo; no change in how often an outline is followed by a full read |

## Implementation Log

Implemented on branch `feat/harness-paper-learnings`, on top of `39532d0` (0.5.0).

### What was built

| Commit | Phase | What |
|---|---|---|
| `769fbbd` | 0 + 1 | Context window per model (catalog snapshot, custom-provider probe or typed value, local engine). `ContextBudget` derives every line from it; a 200k or unknown window keeps exactly 120k / 150k / 200k. `run_config` and `subagent_run` records, `scripts/session-stats.py`. |
| `aa2bcee` | 2 | Shedding without Jev: superseded copies first (lossless), then Jev, then the oldest results by age. Subagents shed too and stop with `context_full` instead of failing at the provider. |
| `43f2776` | 3 | `tasks_update`: one task's status and journal, under the same gates as `tasks_set`. |
| `062c243` | 4 | Auto: rules, then two Jev questions, both at 0.75. Off / Shadow / On in Settings › Agent. `mode_route` record. |
| `33e4360` | 5 | `Lean` per model (drops the three LSP tools and every line that points to them). `Compact` profile under 64k in Builder: one prompt, ten tools, the session edits files itself. |
| `00bbe31` | 6 | List results as lines instead of JSON; a `read_file` whose result is already in the conversation enters history as a pointer. |
| `2162e5d` | — | README and ARCHITECTURE. |
| `dc60b25` | — | Fixes from two rounds of independent review (see the commit message for the full list). |
| `48de1b1` | — | Not part of this plan: `slugify` panicked on a `<goal>` with an accented letter at byte 40. Found by the review; its own commit so it can be taken or left. |
| `db4f427` | 4 | Auto on by default (see below). The Portuguese README gains the sections the English one got. |

### Decisions taken (D1–D6 were not answered; the recommendations were used)

- **D1** soft line clamped to 170k and hard to 180k when the slider is above them. Defaults unaffected.
- **D2** yes: a Compact session edits files itself. Both `edit_file` and a shell command that writes a file ask for approval.
- **D3** yes: Auto enters Brain with agent origin.
- **D4** Lean drops only `go_to_definition`, `find_references`, `symbol_lookup`.
- **D5** stated in the setting's hint.
- **D6** in.

### Where the implementation differs from this plan

- Phases 0 and 1 are one commit: the `run_config` record carries the budget.
- The window is a pure lookup (`AgentConfig::context_window_for`), not a field on `ResolvedProvider`.
- **Auto ships on, not in Shadow.** It was first implemented off by default: the thresholds are uncalibrated, and a default nobody chose sends every first prompt to Jev. The maintainer decided to ship it on (`db4f427`). The control offers Auto only where a Jev credential exists, so a user without one sees no change; with one, a new session starts on Auto and its first message goes to Jev. The shadow gate this plan put in front of "on" (below, "P4 → on") was therefore not applied.
- Calibration is an ignored Rust test (`live_route_calibration`), not `scripts/route-calibrate.py`.
- Compact also offers `run_quality` (ten tools, not nine): the finish-line gate tells the model to call it.
- The duplicate-read pointer is decided from the history itself (the last read of the same file and range must still be there and identical), not from `ReadTracker` and mtimes. It needs no invalidation and cannot point at something a prune removed.
- `semantic_search` got the line format too.
- Added beyond the plan, because the review showed the small-window path broke without them: the reply reserve and the request's `max_tokens` are one number (`budget::reply_tokens`); the results of one round are capped together on a scaled window; a prefix that cannot fit is refused before any handoff or summary is tried.
- Not done: showing a model's window in the model picker.

### Measured

- Shed ladder replayed on 24 stored sessions: 21 passed 60k tokens, all 21 freed at least 25% without Jev (5 by the lossless stage alone, 16 by age); median reduction 50%. No prose changed.
- List results re-encoded from the same sessions: 962k chars to about 282k. These tools are 13.9% of carried mass on the full 509 sessions.
- Duplicate reads replayed: 36 of 986 would have been pointers, 9.9% of the characters `read_file` returned.
- Prefix: Compact is about 3.6k tokens; Standard Builder is about 10k before skills and MCP.

### Not verified

- **The default (`ort`) feature build.** The sandbox could not download the ONNX runtime, so everything was built and tested with `--no-default-features --features embeddings-candle`: clippy clean, 1064 Rust tests, 898 frontend tests, `vite build`. Run `cargo clippy --all-targets -- -D warnings` and `cargo test` locally before merging.
- **Nothing was run against a real model.** Small-window behaviour is covered by a scripted model over HTTP, not by a 32k model doing real work. The Compact prompt in particular is untested on the models it is for.
- **Auto's thresholds are provisional, and Auto is on.** `live_route_calibration` was never run (it needs a Jev key). Both questions must reach 0.75 for Brain, so the expected way to be wrong is a planning request left in Builder; a quick edit sent to Brain is the failure to watch for.
- The settings UI and the Auto button were not looked at in a running app.

### Found by the review, already there before this branch, not changed

- A subagent's tool calls are not checked against its mode: an `explore` subagent that emits `edit_file` or `bash` gets it run, under the normal approval rules.
- The bash allowlist matches prefixes: `ls; rm -rf src` resolves to auto, in every mode.
- Code subagents can write files through an allowlisted prefix (`cat > file`) without approval. This branch closes that for the Compact main session only.
- A hook that answers `ask` on a bash command takes the approval path before the denylist is consulted, so a denylisted command the user then approves runs (by reading `run_tool`; not reproduced).
