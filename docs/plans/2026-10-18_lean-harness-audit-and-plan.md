# A lighter harness that still wins: audit of 0.7.2 and the plan

**Status:** proposal, awaiting review. Nothing is implemented.
**Written against:** `main` at `f3f6026` (0.7.2). Paths are relative to `src-tauri/src/` unless they start with `src/` or `scripts/`.
**Sources:** the code at that commit; the session measurements in [`2026-10-06_harness-paper-learnings.md`](2026-10-06_harness-paper-learnings.md) §2 (509 sessions, 16,945 tool calls — still the only telemetry we have); and the papers and engineering posts listed in §9. Where a number comes from a third-party summary rather than the primary text it is marked *(secondary)*.
**Companion to:** the 2026-10-06 plan. That one asked "what does the harness-design paper say we are missing?" and shipped as 0.6.0. This one asks the broader question the maintainer put: *how do we make Claudinio Code lighter and simpler to use, and still the harness that wins?* It does not repeat what 0.6.0 built; it builds on it.

---

## 0. The one-paragraph version

The harness is already strong where most harnesses are weak: context is shed verbatim before it is summarized, limits follow the model's window, verification is mechanical, every write is gated, and everything is persisted and inspectable. Its weight is somewhere else — **in what every request carries and in how many requests a task takes**. Today a Builder session starts with a ~10k-token prefix that reaches ~33k once skills and MCP schemas join; nothing in that prefix is marked for the provider's cache, so on Anthropic-protocol providers it is re-read cold every turn; the main session is forbidden to edit files and must brief a fresh subagent (a second prefix, a second exploration, a re-typed spec) for every change; and the task protocol costs three to five round-trips of pure ceremony per task. The literature of the last eighteen months says the same five things from five directions: **measure cost per solved task, not tokens** (PointFive 2026); **the cache is the margin** (Manus; our own proxy numbers); **one agent with one context beats a committee unless the work is read-only and parallel** (Cognition; Anthropic's own 15× figure); **load context and tools just in time** (Anthropic's tool search, −85% tool tokens); and **put policy in code, not in prose** (the eleven-harness study; this repo's `claudius` incident on the LiteLLM side). The plan below is ten phases, each shippable and gated on a headless eval we do not yet have — which is why Phase 0 is the eval. The target is a Standard prefix under 8k tokens, half the round-trips per task, the same or better pass rate on the eval, and one place (the loop) a new contributor can read in an afternoon.

---

## 1. What is being optimised, and how we will know

Three numbers, in this order of authority:

| # | Metric | Why it is the one that counts | Where it comes from |
|---|---|---|---|
| M1 | **Billed cost per successfully completed task**, cache-aware | Weinberger & Hozez (arXiv 2607.12161) cut Claude Code's delivered tool-output tokens by 38.4% and the bill went *up* 6.8%; token count correlated with cost at r = 0.15. Cache traffic dominates input cost and a leaner context changed the trajectory (more retrieval, more turns). Token counts are not the metric. | Phase 0 eval runner + `usage` fields (`input`, `cache_read`, `output`) per request |
| M2 | **Rounds (requests) per task** and wall-clock to first edit | Each round re-sends the whole context. Even cached, a 150k-token read is latency and money; uncached it is the whole bill. This is also what "simpler to use" feels like from the chair. | `session-stats.py`, already recorded |
| M3 | **Pass rate on the eval** (quality gate green + task accepted) | The constraint. Nothing below ships if M3 drops. | Phase 0 |

Everything in §5 states its expected effect on M1–M3 and the gate it must pass. Where the effect is a guess it says so.

---

## 2. Audit: what the harness carries today

### 2.1 Keep — the parts that are already right

| What | Where | Why it stays |
|---|---|---|
| Verbatim shedding before summarizing; three stages, cheapest first; never early on a cached provider | `prune.rs` (`plan_lossless` → Jev → `plan_by_age`), `session.rs::try_verbatim_compaction` | Exactly the "observation masking first" result of the JetBrains study (arXiv 2508.21433): masking halves cost at equal or better solve rate. Anthropic's post calls tool-result clearing "the lightest-touch form of compaction". |
| Limits derived from the model's window; Compact profile under 64k | `budget.rs`, `surface.rs` | The harness-design paper's strongest effect (78.7% overflow failures at 32k unmanaged → 0%). Done in 0.6.0. |
| Context reset at the plan → build boundary | `transition.rs` | Cheapest context management in the product: execution starts from the plan, not from the exploration. |
| Mechanical verification: `run_quality`, the digest that invalidates evidence after an edit, golden goals that cannot be closed on a claim | `quality/`, `tools/tasks.rs::check_quality_gate` | The one thing that makes a harness "unbeatable" rather than fluent. Every top Terminal-Bench/SWE-bench entry verifies. |
| Output trimming that keeps the tail of a build log and writes the full output to a file | `output_trim.rs` | The SWE-agent ACI lesson (cap and summarize tool output) plus Manus's "restorable compression". |
| Approval gates, path guard, bash denylist, hooks that may relax a prompt but never a policy | `permissions.rs`, `hooks/`, `tools/mod.rs::validate_path` | Policy in code. Keep. |
| Session JSONL as the source of truth; `run_config` with the prefix itemized | `persist.rs` | This is what makes the rest measurable. |
| Loop breaker, completion judge, duplicate-read pointer | `loop_watch.rs`, `session.rs::judge_terminal_turn`, 0.6.0 P6 | Cheap, bounded, visible. |

### 2.2 Costs — ranked by what they do to M1/M2

**C1. Nothing is marked for the provider's cache.** `provider.rs::RequestBody.system` is `Option<String>`; there is no `cache_control` anywhere in `provider.rs` or `provider/openai.rs`. On the Anthropic Messages API and on OpenRouter's Anthropic routes a prefix is only cached where the request puts a breakpoint, so a direct-Anthropic user pays the full input price for the whole context, every round. The 2026-10-06 plan already flagged the symptom without the cause (§2.7: $33.01 of input against $1.06 of cache read across 509 sessions, when the proxy says ~96% of executor input is cache reads). The claudin.io path is served by upstreams with implicit prefix caching, so claudin.io customers are less exposed — but the app is sold as working with "any Anthropic-compatible API", and on those it is 5–10× more expensive than it needs to be. *Effect on M1: the largest single lever in this document, and it has zero quality risk.*

**C2. The prefix: ~10k built-in, ~33k in practice.** From the source (`session.rs` 466–870, `tools/mod.rs` 225–660):

| Part | Size | Note |
|---|---|---|
| `SYSTEM_PROMPT` + workspace line | ~6.3k chars | Of which the "LINKS (Markdown)" section with its examples is ~1.2k chars |
| `GOLDEN_PROMPT` | ~1.5k chars | Appended in both modes even when no golden task exists |
| Builder block / Brain block | ~6k / ~11k chars | Imperative protocol text (see C5) |
| 20 built-in tool schemas (Builder) | ~32k chars of source ≈ 7–8k tokens serialized | `ask_user` 4.1k chars, `browser` 2.9k, `spawn_agents` 2.0k, `browser_screenshot` 1.9k, `tasks_set` 1.9k, `browser_inspect` 1.7k, `semantic_search` 1.6k |
| Two built-in skills with **full body inlined** | 6.6k chars | `find-skills` and `plugin-crafter` (`skills.rs` 170–308, `build_skills_system_prompt_section`) — needed in perhaps one session in a hundred, paid in all of them |
| User skills catalog | name + description + path per skill | Unbounded; grows with every skill installed |
| MCP tool schemas | unbounded | Anthropic measured a five-server setup at ~55k tokens of definitions |
| Three LSP tools | ~2.6k chars | Called 12 times in 16,945 (0.07%); still on by default, Lean is opt-in |

The measured prefix went 19k → 30k → 33k tokens from July to October (previous plan §2.6). Anthropic's tool-search post is the reference point: deferring definitions until needed took a 55k-token tool prefix to ~8.7k (−85%) and *raised* accuracy (Opus 4: 49% → 74% on their MCP eval), because fewer irrelevant schemas is also less distraction — the Chroma "context rot" result in miniature.

**C3. The delegation tax.** `session.rs::api_tools` removes `edit_file` from every Standard main session ("Builder delegates ALL file modifications to code-mode subagents"), and the Builder prompt makes it a rule. Every change therefore costs: a second full prefix (the subagent's own ~8k tokens of tools + prompt), a second exploration of the same files by a model that has none of the parent's context, the parent *writing* the entire technical spec into the goal verbatim (output tokens, the expensive kind — the prompt literally demands "repeat every concrete value from the plan/task VERBATIM"), and a report coming back (7.8% of carried mass before counting the subagent's own transcript, which is **not persisted** and so has never been measured). Cognition's "Don't build multi-agents" is about exactly this failure: subagents that cannot see each other's decisions produce inconsistent work, and the fix is one agent that owns the context. Anthropic's own multi-agent research post reports the cost side: ~15× the tokens of a single chat — worth it for breadth-first *research*, not for applying a diff. The harness-design paper notes the same for small windows, and the Compact profile already proves the alternative works in this codebase: a Compact session edits files itself. *Effect on M1/M2: large; probably the second lever after C1. Unmeasured because subagent transcripts are not kept — Phase 0 fixes that first.*

**C4. Task ceremony as remote procedure calls.** The Standard prompt requires `tasks_get` as the first call of every run (state the harness already holds and could hand over for free), a `tasks_update` to `doing` before touching anything, one to `done` after, and a final update before the last message — "Never batch updates". That is three to five extra rounds per task, each round a full re-read of the context. Before `tasks_update` existed, `tasks_set` alone was 24.3% of everything the main session generated. The Task Panel is a real differentiator — the user sees progress without reading a transcript — and Manus's "recitation" argument (rewrite the todo so the goal stays near the end of the context) is a reason to keep a task list. Neither requires a round-trip per state change.

**C5. Prompt altitude.** The Brain block is ~11k chars of protocol: a *mandatory* interview, one question per `ask_user` call, a five-dimension consensus checklist, a mandatory second `write_plan` for the LLD, three mandatory deliverables "regardless of who enabled this mode", a Mermaid section. The Builder block has eight numbered steps. Across the three blocks: 8 MUST, 3 MANDATORY, 2 NEVER, 1 STRICT. Anthropic's context-engineering post calls this the low-altitude failure mode ("brittle, hardcoded if-else logic") and recommends starting minimal on the best model and adding only what fixes an observed failure. Two facts from our own sessions say the prose is not even working: 23% of `bash` calls are `grep/cat/ls/find/sed` despite the rule against bash search, and the median Brain session takes 40 assistant rounds against 6 in Builder. The user asked for "simpler to use"; the mandatory interview is where a user feels the harness most. Meanwhile the rules that *matter* are already enforced in code (`check_brain_lld_gate`, `check_quality_gate`, the write gates, the finish-line check) — which is the eleven-harness study's observed trend: behavioural policy moving from prompt prose into configuration and code (Codex replaced prompt rules with feature flags; Mistral Vibe dropped a "never commit" rule from its prompt). Where a gate exists, the paragraph that describes it is a tax.

**C6. Tool calls run one at a time.** `session.rs` 3406 iterates `tool_uses` sequentially. A round of four `read_file` calls is four serial file reads — and when one needs approval, the rest wait behind it. No token cost; pure latency, and latency is most of "feels heavy".

**C7. Tool output shape.** `read_file` has a 2 MB input cap and a 24k-char history cap (`budget::TOOL_RESULT_CHARS`) that cuts the *head* — right for a file, wrong for anything whose answer is at the end (the trim ladder catches `bash`, not other tools). `file_outline` averaged 21.9k chars per call while the prompt told the model to call it *before* reading, to save context; 0.6.0 P6 moved list results to lines, which should be re-measured.

**C8. Weight for the maintainer.** 62.7k lines of Rust, 35k of TypeScript. `session.rs` is 8,446 lines and owns the prompt text, the loop, the judge, the handoff, the prefix, the retry policy and the event enum; `run_workflow_with_profile` alone spans lines 2204–3944. `tauri::ipc::Channel` appears 22 times in it, which is why there is no headless runner and therefore no eval (§2.3). Jev is consulted at five points (route, completion, loop, trim, prune) through a paid external API with hand-measured thresholds pinned to `jev-1.13.0` — all fail-open, all defensible, but five more things that a reader has to hold.

### 2.3 The gap that blocks everything else: there is no eval

Every gate in the previous plan is phrased as "over the following two weeks of real use". That is one user, one workspace, and it cannot tell a regression from a hard week. The harness-design paper's whole method is *hold the loop fixed, vary one thing, measure on a fixed task set*; Terminal-Bench 2.1 shows the same model moving from 76.4% to 83.4% on harness alone *(secondary)*. We cannot claim "unbeatable" — or safely remove a single prompt line — without a task set we run on every change.

---

## 3. What the literature says, and what each finding means here

| Source | Finding | What it means for Claudinio Code |
|---|---|---|
| Weinberger & Hozez, *Token Reduction Is Not Cost Reduction* (arXiv 2607.12161, 2026) | −38.4% tool-output tokens → +6.8% billed cost; cache creation/reads dominate input cost; compression changed trajectories and reduced patch success on SWE-bench Go | M1 is cost per solved task with cache accounted for. Never ship a context cut on a token count. Phase 0 before anything. |
| Manus, *Context Engineering for AI Agents* (2025) | KV-cache hit rate is the production metric (10× price gap, ~100:1 input:output); keep the prefix byte-stable and append-only; **mask tools, don't remove them** mid-task; the file system is the unlimited, restorable context; recite the todo so the goal stays recent; keep errors in; vary serialization to avoid few-shot ruts | C1 (cache markers), C2 (prefix stability tests), C4 (recitation without RPC), and a rule for Phase 2: deferred tools are *appended*, never swapped. |
| Anthropic, *Effective context engineering for AI agents* (2025) | Right-altitude prompts; minimal set of high-signal tokens; just-in-time retrieval with lightweight references; tool-result clearing as the lightest compaction; structured note-taking outside the window; sub-agents return a condensed 1–2k-token summary | C5 rewrite; skills by reference (C2); notes file at handoff (Phase 6); subagents as read-only researchers (Phase 3). |
| Anthropic, *Advanced tool use* — Tool Search Tool, Programmatic Tool Calling (Nov 2025) | Deferred tool definitions: 55k → 8.7k tokens (−85%), accuracy 49% → 74% (Opus 4) on MCP evals; programmatic calling −37% on research tasks *(secondary)* | Phase 2: a `load_tools` surface for MCP, browser, LSP, plan tools; core eight always present. |
| Anthropic, *Writing effective tools for agents* (2025) | Fewer, non-overlapping tools; token-efficient responses; namespacing; description wording measurably moves eval results | Phase 2 description diet; Phase 6 output formats. |
| Anthropic, *Building effective agents* (Dec 2024) | Start with the simplest thing that works; add orchestration only when it measurably helps | The spirit of Phase 3 and Phase 8. |
| Anthropic, *How we built our multi-agent research system* (2025) | Parallel subagents: +90.2% on their research eval at ~15× the tokens of a chat; worth it for breadth-first, parallelizable *reading* | Subagents stay for explore; delegation for edits goes (C3). |
| Cognition, *Don't Build Multi-Agents* (2025) | Share the full trace; every action carries implicit decisions; parallel writers without shared context produce inconsistent work; one agent plus compression | C3. The Builder main session edits. |
| Lindenbauer et al., *The Complexity Trap* (arXiv 2508.21433, JetBrains, NeurIPS 2025 DL4Code) | Observation masking ≈ half the cost of LLM summarization at equal or slightly better solve rate (Qwen3-Coder 480B: 53.8 → 54.8%) | Already our stage 2; make it the universal first stage before Jev, including subagents (Phase 6). |
| Fan et al., *An Empirical Study of Harness Design for Coding Agents* (arXiv 2609.20804) | Context management is decisive at 32k, marginal at 128k+; recall tools unused; structured tools +15 pp for a 30B model, bash-only +3.6 pp and −53% cost for a 550B model, −23 pp for another | Already applied in 0.6.0; the "per model, by measurement" rule for Lean stays. |
| *Harness Engineering: … A Source-Code Study of Eleven Systems* (arXiv 2609.00006) | ~4M lines across 11 harnesses; no agent framework, no vector RAG over code; loop sophistication does not predict benchmark results (Mini-SWE-Agent ≈ OpenHands); most code is safety/UX/transport; built-in tool counts from 1 (Mini-SWE-Agent) to 43 (Claude Code) and 109+ (OpenClaw); Claude Code restricts workers to a 16-tool whitelist; policy is moving from prompt prose to configuration; a 90-line minimum-viable harness implements 10 of its 18 recommendations | Phase 8 (split the loop, shrink what the model-facing surface depends on); Phase 5 (policy in code); a reminder that our embeddings index is a differentiator the field has *not* adopted — keep it, but as a tool the model reaches for, not a mandate in the prompt. |
| Yang et al., *SWE-agent: Agent-Computer Interfaces* (arXiv 2405.15793) | The interface is a lever of its own: windowed file viewer, capped search results, linting after edits recovers from bad edits | Phase 6: syntax check on `edit_file` when a tree-sitter grammar exists (we already ship 77 of them); capped and shaped outputs. |
| mini-SWE-agent / SWE-bench Bash-Only | A ~100-line bash-only loop scores in the same band as large scaffolds on strong models | The ceiling on what the *harness* adds is the model; everything we add must pay for itself on M1. |
| Zhang et al., *Agentic Context Engineering* (arXiv 2510.04618, ICLR 2026) | Contexts as evolving playbooks: append deltas, curate, never rewrite wholesale; names the failure modes *brevity bias* and *context collapse* | Phase 6 compaction prompt (recall first, structured, keep errors); Phase 9 project memory as an append-only, size-capped file. |
| Chroma, *Context Rot* (2025); Liu et al., *Lost in the Middle* (2023); NoLiMa (ICML 2025) | Performance degrades with input length even on simple tasks; distractors compound; position matters | A smaller prefix is a quality lever, not only a cost one — it supports Phase 2's accuracy claim. |
| OpenAI, *Harness engineering: leveraging Codex in an agent-first world* (Feb 2026) | AGENTS.md as a table of contents into `docs/`, not an encyclopedia; golden rules and linters as scaffolding; repository context is the harness | Phase 9: `.claudinio/MEMORY.md` ≤ 1k tokens, pointers not prose; `finalize_plan` journals feed it. |
| Terminal-Bench 2.0/2.1 (ICLR 2026; leaderboards) | Same model, different harness: 76.4% → 83.4% *(secondary)*; scores measure harness + model | Phase 0's external track: run a TB 2.1 / SWE-bench Verified subset headless so the harness is comparable to the field. |

---

## 4. Design rules this plan holds itself to

1. **M1 decides.** A change that lowers tokens and raises cost per solved task is reverted.
2. **The prefix is append-only and byte-stable within a session.** Deferred tools and skills are appended, never swapped (Manus). A test asserts two consecutive requests share an identical prefix.
3. **One agent owns the task context.** Subagents read; the main session writes. Parallel writers only when the user asks for them or the changes are provably disjoint.
4. **Policy lives in code; the prompt explains, it does not enforce.** Every "MUST" in a prompt either has a gate in code (then the MUST goes) or an observed failure it fixes (then it stays, with the failure cited in a comment).
5. **Just-in-time over pre-loaded.** Skills by reference, tools by catalog, specs by index, memory by pointer.
6. **Tool output is shaped for a model**: compact, ordered, the answer never cut off by a head truncation.
7. **Every phase ships alone and is gated on the eval.** No phase depends on a later one.
8. **Nothing is adapted to a model without measuring on that model** (the Lean lesson from 0.6.0).

---

## 5. The plan

Each phase: what changes, where, the expected effect on M1–M3 (honest guesses marked), the gate, the risk. Phases 1–5 are the token work in order of leverage; 6–9 are quality and simplicity and can run in parallel with them.

### Phase 0 — The eval, and seeing the subagents (prerequisite)

*What.* A headless runner and a fixed task set, so every later phase has a number.

- **Headless runner.** Abstract the event sink: `session.rs` and `subagent.rs` take `Channel<AgentEvent>` directly (22 and 7 sites). Introduce `trait EventSink: Send + Sync { fn send(&self, AgentEvent) }`, implemented by the Tauri channel and by a collector. `ApprovalMap`/`AnswerMap` get an auto-approve policy for the runner (approve edits and allowlisted bash, deny anything in the denylist, answer `ask_user` from a fixture or fail the task). Exposed as `cargo run --bin claudinio-eval -- <taskset> --model <id>`.
- **Task set, two tracks.**
  - *Internal (ours):* 25–40 tasks drawn from this repository's own history — a commit whose diff is the reference, the parent as the starting tree, the repo's tests as the oracle. The plan documents in `docs/plans/` already describe many of them in the user's words. Graded by `run_quality` green + a reviewer rubric on the diff.
  - *External (the field's):* a fixed subset of SWE-bench Verified (50 instances) and Terminal-Bench 2.1 (20 tasks), run through the same binary. Not to chase a leaderboard — to notice when a change costs us against the only yardstick others use.
- **Per-request accounting.** Persist `input_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens` (Anthropic) / `cached_tokens` (OpenAI) and provider-reported cost per request, not only per run. `session-stats.py` gains `--cost`: M1, M2, cache hit share, and the *subagent* share — `SubagentDone` carries tokens already; persist the subagent transcript to its own JSONL (`.claudinio/sessions/<id>.sub/<n>.jsonl`) so §2.1's 44%/9%/7.8% can be redone with the whole picture.
- **Prefix stability test.** Build the prefix twice for the same config and assert byte equality; run it in CI.

*Files.* `agent/session.rs` (sink trait, approvals policy), `agent/subagent.rs`, `agent/persist.rs` (per-request usage record, subagent store), new `src-tauri/src/bin/claudinio-eval.rs`, `scripts/session-stats.py`, new `eval/tasks/*.toml`.
*Effect.* None on the product. Everything on the plan.
*Gate.* The runner completes the internal set on the default claudin.io model with a recorded baseline for M1/M2/M3 and cache share; the baseline is committed under `eval/baselines/`.
*Risk.* Approval auto-policy must never leak into the app: it lives in the eval binary, behind a type the app cannot construct.

### Phase 1 — Cache discipline (C1)

*What.*
- Anthropic protocol: send `system` as content blocks and set `cache_control: {"type": "ephemeral"}` on (a) the last tool definition, (b) the last system block, and (c) the last user turn of the *previous* round — the moving breakpoint — so the whole history up to the current round is read from cache. Four breakpoints are the API limit; three is enough.
- OpenAI protocol: caching is automatic on most servers, but verify with the per-request accounting from Phase 0 that `cached_tokens` is non-zero on claudin.io, OpenRouter and a local llama.cpp with `--cache-prompt`; where it is zero, find out why before touching anything else.
- Make the prefix provably stable: `{max_parallel}` substitution is fine; audit the skills catalog order (sort by name), the spec index, the hook `SessionStart` context (goes into the first *user* turn, never the system prompt — check), and anything carrying a timestamp.
- Compaction and handoff already avoid rewriting history early; add a counter to `run_config` of *cache-breaking events* per session (prune, compact, handoff, tool list change) so Phase 2's deferred loading can be held to a budget.

*Files.* `agent/provider.rs` (`RequestBody.system: SystemContent`, `ToolDescription.cache_control`, `Message` block on the breakpoint), `agent/provider/openai.rs` (verification only), `agent/session.rs::run_prefix`, `agent/skills.rs`.
*Effect.* M1 on direct Anthropic / OpenRouter-Anthropic: input cost −60% to −80% for any session longer than three rounds (cache read is 10% of the input price; a 50-round session re-reads its prefix 50 times). M2 unchanged. M3 unchanged — this changes nothing the model sees. On claudin.io: confirms or refutes the §2.7 discrepancy.
*Gate.* Eval on an Anthropic-direct model: cache-read share of input ≥ 80% after round 3; M1 down; M3 equal. Prefix-stability test green.
*Risk.* A provider that rejects `cache_control` (some OpenAI-compatible shims behind an Anthropic façade). Detect the 400 once, drop the field for that provider, remember it in the provider entry.

### Phase 2 — The prefix diet (C2): target ≤ 8k tokens Standard, ≤ 10k with skills and MCP

*What — in order of tokens saved per hour of work.*

1. **Skills by reference, always.** Stop inlining the two built-in bodies (−6.6k chars on every request). The catalog becomes one line per skill — `name — description` — capped at 25 entries ranked by scope (project > user > plugin > built-in); the rest are reachable through a `skills` tool (`list`, `read <name>`) that the catalog line names. This is Anthropic's progressive disclosure and what Claude Code does with its own skills.
2. **Deferred tools.** Core catalog, always present and byte-stable: `read_file`, `list_dir`, `grep`, `edit_file`, `bash`, `ask_user`, `tasks_update`, `run_quality`, `spawn_agents` (nine). Everything else — `code_search`, `semantic_search`, `file_outline`, the three LSP tools, `web_search`, the three browser tools, `tasks_get`/`tasks_set`, `write_plan`/`finalize_plan`, `enter`/`exit_plan_mode`, and **every MCP tool** — is listed in the prompt as a one-line catalog (`name: what it is for`) and materialized by `load_tools(names)`. Loaded definitions are *appended* to the tool array and stay for the session (one cache write per load, never a swap — Manus's rule, and Anthropic's tool-search semantics). Anthropic measured −85% tool tokens and higher accuracy on exactly this shape. A hook matcher such as `Edit|Write` resolves through the alias table whether or not the tool is loaded.
3. **Description diet.** Rewrite each built-in description to ≤ 400 chars with one example at most; move the `ask_user` option-shape rules into the schema (`oneOf` already carries them) and the "never add Other" rule into the tool's *error message*, where it costs nothing until it is needed. `ask_user` 4.1k → ~0.8k; `browser*` 6.5k → ~2.5k; `spawn_agents` 2.0k → ~0.9k; `tasks_set` 1.9k → ~0.8k.
4. **Prompt sections that are not instructions.** The LINKS section (1.2k chars of examples) becomes two lines; `GOLDEN_PROMPT` is appended only when a golden task exists; the Brain block loses what Phase 5 moves into code.
5. **Lean by default for the LSP trio** (12 calls in 16,945), now that they are deferred anyway — a model that wants them loads them.

*Files.* `agent/skills.rs::build_skills_system_prompt_section`, new `agent/tools/skills_tool.rs`, `agent/tools/mod.rs` (`get_defs` split into `core_defs`/`deferred_defs`, `load_tools` def and dispatch), `agent/session.rs::api_tools` and the per-round tool array (append on load; persist a `ToolsLoaded` record so a resumed session rebuilds the same array), `agent/surface.rs` (Compact unchanged: it already is the core set), `agent/hooks/mod.rs` alias table, `src/components/tool-renderers/` (a row for `load_tools`).
*Effect.* Prefix: ~33k → ~8–10k tokens in the measured setup (−70%); built-in alone ~10k → ~5k. M1 down on every provider including cached ones (cache reads are 10%, not 0%, and the first request of every session is a cold write). M3: expected flat to up — fewer distractors (Chroma; Anthropic's 49% → 74%). M2: possibly +1 round per session for the first `load_tools`.
*Gate.* `run_config.prefix` total ≤ 10k tokens on the eval setup with 10 skills and one MCP server; M3 within noise of the Phase 1 baseline; M1 down; cache-breaking events ≤ 2 per session median.
*Risk.* A model that never calls `load_tools` and so never searches semantically. Mitigation: the catalog line for `semantic_search` says when to reach for it; the eval has tasks that need it; and `code_search`/`semantic_search` can be promoted back to core *per model* through the existing surface setting if a model needs them.

### Phase 3 — The main session edits; subagents read (C3)

*What.*
- `api_tools` stops removing `edit_file` from Standard Builder. The Builder prompt loses step 4 ("You CANNOT edit files yourself") and gains a paragraph on *when* to delegate: a read-only investigation across many files (explore), or two or more changes that touch disjoint files and the user asked for speed (code). The write-gate for bash (`effective_permission`) stays exactly as it is.
- Explore subagents keep their fresh context and return a *condensed* report: the subagent prompt asks for ≤ 1,500 tokens, structured as `findings / files:lines / open questions`, and the harness truncates to the budget (Anthropic's 1–2k figure). The parent sees the report, not the transcript.
- Code subagents remain available and are offered a *shared brief* the harness composes — the current task, the plan path, the files the parent already read in this run (paths only) — so two writers at least start from the same facts (Cognition's "share the trace", in the cheapest form).
- Compact already works this way; nothing changes there.

*Files.* `agent/session.rs::api_tools`, the Builder block, `agent/subagent.rs` (`SUBAGENT_SYSTEM_PROMPT`, report cap, brief composition), `agent/surface.rs` docs.
*Effect.* Per edit: −1 prefix (~8k tokens after Phase 2, ~10k before), −1 exploration, −1 verbatim spec in output tokens, −1 report. On the §2 data the visible part alone (spec re-emission + reports) is >10% of generated characters and ~8% of carried mass, before the never-measured subagent side. **Guess: M1 −25% to −40% on multi-step Builder tasks.** M2 down (fewer rounds of "brief, wait, read report"). M3: the Cognition argument predicts *up* on tasks whose edits must agree with each other.
*Gate.* Eval internal set: M1 down ≥ 25%, M3 not down. A tagged eval subset of "two independent changes" confirms code subagents still pay off there, or the prompt stops recommending them.
*Risk.* A long Builder session accumulates more context in the main window than before (it now reads the files it edits). The shed ladder exists for exactly this; Phase 6 makes it stronger. The approval UX for edits in the main session already exists (Compact).

### Phase 4 — Tasks as recitation, not RPC (C4)

*What.*
- **No `tasks_get` round.** At the start of each run the harness injects the current task list (compact, one line per task: `id · status · title`) into the first user turn of the run as a `<tasks>` block — it already injects hook context there. The first message of a successor session carries it too. `tasks_get` becomes a deferred tool for the rare "show me the journal of task 3".
- **Batched updates.** `tasks_update` accepts `updates: [{id, status?, journal?}]`; "never batch" leaves the prompt. The gates (`check_quality_gate`, golden protection, LLD gate) run per item exactly as now.
- **Harness-side `doing`.** When the model's first tool call of a round names a path that a `todo` task's description mentions… *no.* Rejected: inference of intent from paths is the kind of cleverness that produces wrong panels. Instead: the prompt asks for `doing` *in the same round* as the first tool call for that task (tool calls in one round are free of extra requests), and the harness renders a task as *in progress* when a `tasks_update` to `doing` arrives in a round with other calls — which is what already happens, minus the standalone round.
- **Small tasks need no list.** The Standard prompt adopts Compact's line: "for anything beyond a one-step change". The Task Panel shows "single step" rather than an empty list.
- **Recitation.** On a handoff or a compaction, the task list is restated at the tail of the new context (it already survives, as the one thing no prune cuts — make it the *last* thing, where attention is highest; Manus's point).

*Files.* `agent/session.rs` (run start injection, prompt §1 and Builder steps 1/3/5), `agent/tools/tasks.rs::execute_update` (array form, same gates), `agent/tools/mod.rs` (schema), `agent/transition.rs`, `src/components/TasksPanel.tsx` (single-step state), tool renderer.
*Effect.* M2: −2 to −4 rounds per task (the `tasks_get` round, the standalone `doing` round, the final-update round). M1 follows M2 (each avoided round is a full context read). M3 unchanged; the Panel still shows the same states.
*Gate.* Eval: median rounds per internal task down ≥ 25%; Task Panel parity tests in `src/components/TasksPanel.test.tsx` extended for batched updates and the single-step state.
*Risk.* A model that forgets to update at all once it is not forced per step. The finish-line check already refuses to end with a task `doing`; keep it, and the completion judge's rubric gains "tasks left `todo` that the message claims done".

### Phase 4b — The living document: one `.md` is the session

*The idea (the maintainer's, 2026-10-18).* A session owns one Markdown file. The agent writes its scaffold first — a title and one placeholder per step of the work — and then fills each placeholder on demand, in order or in parallel. The user can open the file at any time and edit it or add to it, and that edit is now part of the plan. The UI shows, per section, who is working on it. A new top-level topic is a new file, and therefore a new session.

```md
# Pudim de leite

[origem do pudim de leite]
[ingredientes originais]
[Modo de preparo clássico]
[Como servir o pudim]
[Como armazenar o pudim]
```

*Why it belongs in this plan.* It is four of the sources above in one artefact. The file system is the context (Manus) — the work lives outside the window and is restorable by reading it. The outline at the tail of every request is the recitation that keeps the goal in attention. It is structured note-taking (Anthropic) that survives compaction and handoff by construction. And it is the shared trace Cognition asks for: subagents do not *report back through the parent* — they write their section into the document, and the parent reads headings and status, not bodies. The human editing the plan in place is also the cheapest interview there is: an edit replaces an `ask_user` round-trip.

*What.*
- **Format.** Plain Markdown that any editor can open. A placeholder is a line of the form `[topic]`; the harness treats each as one section/task. Ownership and status live in an HTML comment the harness maintains on the section heading, invisible when rendered: `<!-- claudinio: id=s3 status=doing by=sub:2 since=... -->`. Nothing else is special syntax, so a human never has to learn one.
- **Session ↔ document, 1:1.** `.claudinio/docs/<slug>.md`, path recorded on the session's `Meta`. The existing `write_plan` / plan file becomes this document (one mechanism, not two). A request that opens a new top-level topic is a new document and a new linked session — `transition.rs` already knows how to hand off; the trigger is "the user or the agent starts a new document", and the old one is closed with its outline intact.
- **Placeholders are the task list.** The harness parses the document; each placeholder is a task (`todo`); a section being written is `doing` by the agent or subagent that claimed it; a filled section is `done`. The Task Panel becomes a *view* of the document, so Phase 4's round-trips disappear for this kind of work: claiming a section is one `doc_write(section)` call that also does the work. Golden goals are sections the user wrote; the quality gate still decides when an *execution* section may close.
- **Writes are section-scoped.** The agent writes through `doc_write(id, body)` (a core tool) and never through `edit_file` on this path. Subagents in `explore` mode may write *only* the section they were given — their first write capability, and a safe one: one section, one file, inside `.claudinio/`. Two agents cannot claim the same section.
- **Human edits are first-class.** The file watcher already exists for indexing; it watches this file too. A human edit to a section marks it `human-edited`; if an agent was mid-write on that section its write is rejected and it re-reads (the human wins). A human adding a new `[placeholder]` adds a task. A human deleting a section removes the task. The harness tells the agent what changed as a short `<doc-changed>` note in the next user turn (sections and ids, not bodies).
- **What the model carries.** The outline — heading, status, owner, one line per section — at the tail of every request (recitation), not the bodies. Bodies are read when needed (`doc_read(id)`), and the duplicate-read pointer from 0.6.0 applies. Compaction and handoff keep the outline and the path; the summary no longer has to carry what the document already holds.
- **UI.** A document panel beside the chat: sections as rows with the status chip and the owner (`main`, `sub:2 — origem`), live, clicking a row opens the section in the existing Monaco viewer for editing. The subagent timeline already exists; the chip links to it. The Task Panel stays for sessions without a document.

*Files.* `agent/persist.rs` (`Meta.document`, section ids), new `agent/document.rs` (parse, claim, write, watch diff), `agent/tools/mod.rs` (`doc_write`, `doc_read` in the core set; `write_plan` folded in), `agent/subagent.rs` (section grant), `agent/session.rs` (outline injection at the tail; `<doc-changed>` note), `agent/transition.rs` (new-document handoff), `code_intel/mod.rs` watcher hook, `src/components/DocumentPanel.tsx`, `TasksPanel.tsx` as a view.
*Effect.* M2 down on research-shaped and multi-section work: no `tasks_set`/`tasks_update` rounds, no subagent report round, fewer `ask_user` rounds. M1 follows, plus the parent never carries section bodies it did not ask for. M3 up where the user's edits would otherwise have arrived as a correction three turns late. The cost is re-reads after human edits, bounded by outline-only notifications. **Guess: on a five-section research task with two subagents, the parent's input falls by the size of two reports and four task calls per section.** The eval gets a tagged subset of such tasks.
*Gate.* Round-trip tests on the format (parse → write → parse is identity; a human edit in a foreign editor survives); a resumed session rebuilds the same outline; eval subset: M2 down ≥ 30%, M3 not down; the panel's status matches the file within one watcher tick.
*Risk.* Two writers on one file: section-scoped writes and the claim table remove the common case; a human and an agent editing the *same* section at once is resolved in the human's favour and reported. A document that grows large: bodies are never in the prompt, so the window is unaffected; the file is the user's, so size is their call.
*Relation to other work.* The LiteLLM side carries a `conversation_branches` experiment (sub-task as a branch, merged back as one structured summary, Jev-judged, `CLAUDIN_BRANCHES=off` today). The document is the client-side, human-visible version of the same intuition: the section is the branch, the filled section is the merge, and a person can see and edit it.

### Phase 5 — The prompt at the right altitude (C5)

*What.* One rewrite, test-driven against the eval, in three moves.

1. **Delete every sentence that describes a gate the code already enforces**, and leave one line pointing at the gate's *error message* as the teacher: the LLD gate (`tasks_set` rejects until `## Low-Level Design` exists), the quality gate, the golden protection, the read-before-edit check, the write gates, the finish-line check. Each gate's error message is reviewed to say what to do next, because that message is now the instruction.
2. **The Brain interview becomes adaptive.** Replace "keep interviewing until consensus, one question per call, five dimensions" with: *ask about decisions whose answer changes the plan; bundle independent questions in one `ask_user` call; a request that is fully specified needs no interview; confirm the design in one question before writing it.* The checklist moves to a `plan` skill the model can read when the request is large (just-in-time), and `write_plan`'s own error message carries the required sections. Expected: median Brain rounds 40 → ≤ 15 with the same plan acceptance in the eval rubric.
3. **The base prompt is identity, environment, judgement.** ≤ 1.5k tokens: who the agent is; the workspace facts; how the tools relate (indexed search exists and is cheap — *why* it is better, not a MUST; the 23% bash-search figure says the MUST was ignored anyway); when to ask and when to stop; untrusted content; hook output; language. Mode blocks ≤ 600 tokens each. Two or three *canonical examples* replace the rule lists where a rule keeps failing in the eval (Anthropic's advice: examples over edge-case rules).

*Files.* `agent/session.rs` (`SYSTEM_PROMPT`, `GOLDEN_PROMPT`, the two mode blocks; `build_spec_prompt_section`), new built-in `plan` skill (by reference, Phase 2), gate error messages in `tools/tasks.rs`, `tools/finalize_plan.rs`, `tools/mod.rs`.
*Effect.* Prefix −3k to −4k tokens (on top of Phase 2). M2 in Brain down sharply. M3: the risk phase — see gate.
*Gate.* The eval, twice: once with the old prompt, once with the new, same model, same tasks; M3 not down, Brain rounds down, user-rubric "asked only what mattered" up. Every removed MUST has either a gate test or an eval task that fails without it (rule 4).
*Risk.* Regressions that only a human notices (tone, over-asking, under-asking). Mitigation: the eval rubric includes a question-count budget per task, and the rewrite ships behind the existing profile mechanism (`PromptProfile::Standard2`) for one release so a user can flip back.

### Phase 6 — Context and output quality (keeps M3 up while 1–5 take weight out)

- **Masking first, everywhere.** Run stage 0 (lossless) and stage 2 (by age, keep the call, cut the output to a head) *before* paying for Jev, and ask Jev only to arbitrate what stage 2 would cut from the last N rounds. Subagents get the same. (JetBrains: masking alone is enough most of the time; Jev then costs less and decides less.)
- **Compaction prompt, recall first.** The 600-word summary becomes a structured note — `goal · decisions and why · files touched (paths:lines) · what failed and the exact error · what is left · constraints the user stated verbatim` — tuned the way Anthropic describes (maximize recall, then trim), and it **keeps errors in** (Manus). It is also written to `.claudinio/notes/<session>.md`, so the successor and the user can read it: the file system as memory, and a `PreCompact` hook's natural landing place.
- **Tail-aware caps.** `tool_result_block` keeps head *and* tail for any output (not only `bash`), with the omitted-lines marker and the full output in a temp file, generalizing `output_trim`.
- **Edit guardrail.** After `edit_file` is applied, if a tree-sitter grammar covers the file, parse it; on a new syntax error, return the error with the line in the tool result (SWE-agent's linting result: recovery from bad edits). No LSP needed; the grammars are already in the binary.
- **Re-measure `file_outline`** after 0.6.0 P6; if still > 5k chars mean, cap by depth.

*Files.* `agent/prune.rs`, `agent/session.rs::try_verbatim_compaction`, `agent/subagent.rs::run_summary_agent` prompt, `agent/persist.rs` (notes path), `agent/tools/edit.rs` (post-apply parse), `agent/output_trim.rs`.
*Effect.* M3 up on long tasks; M1 slightly down (fewer Jev calls, fewer re-reads after a lossy summary).
*Gate.* Eval long-task subset (≥ 40 rounds): M3 not down with Jev disabled; compaction recall rubric (does the successor re-read files the note should have carried?) improves.

### Phase 7 — Parallel tool execution (C6)

Tool calls in one round that are auto-approved and read-only (`read_file`, `list_dir`, `grep`, the search tools, `browser_inspect`, MCP tools the server marks read-only) run concurrently with a bound of 4; anything that needs approval, writes, or is `bash` runs after them, in order. Results are appended in the model's original order so the transcript is unchanged.
*Files.* `agent/session.rs` around line 3406, `agent/tools/mod.rs` (a `is_parallel_safe` flag on the def).
*Effect.* Wall-clock per round down; no token change. *Gate.* Ordering tests; eval wall-clock down on read-heavy tasks.

### Phase 8 — Lighter for the maintainer (C8)

- Split `session.rs` by responsibility, no behaviour change: `prompt.rs` (the text and `system_prompt`), `loop.rs` (`run_workflow_with_profile`), `judge.rs` (completion + loop verdicts), `handoff.rs`, `events.rs` (`AgentEvent`). Target: no file over 2,000 lines; `loop.rs` readable top to bottom.
- `Jev` behind one trait, `Judge`, with the rule-based implementation as the default and the TypeSafe/OpenRouter/claudin.io backends as the alternative — so a reader meets one concept, and a build without Jev is a configuration, not five `if let Some(backend)` sites.
- `GitSync` becomes a skill plus the core tool set, not a `PromptProfile`; the profile enum goes back to `Standard | Compact`.
- `architecture_tests` gains the file-size and layering assertions.
*Effect.* None on M1–M3 by construction (tests pin behaviour). The payoff is every later phase being cheaper. *Gate.* `cargo test --lib` identical before and after; `architecture_tests` green; line counts.

### Phase 9 — Project memory that improves the next session (ACE-lite)

- `.claudinio/MEMORY.md` per workspace: ≤ 1k tokens, loaded into the first user turn of a session (not the system prompt — it changes), formatted as pointers: *decision → where it is written down*, *gotcha → the file that explains it*. The OpenAI "AGENTS.md as a table of contents" shape.
- Written by the harness, not the model: `finalize_plan`'s journal already asks for "key decisions, gotchas, what was learned"; the harness appends the journal's one-line entries with the plan path. **Append-only with a size cap**, oldest entries demoted to the plan files they point to — the ACE paper's answer to brevity bias (never summarize the memory) and context collapse (never rewrite it).
- The model may *propose* a memory line through `finalize_plan`; it may not edit the file (path guard), so the memory cannot be prompt-injected by a page the agent read.
*Effect.* M2 down on repeat work in the same repo (less re-discovery); M3 up where the gotcha would have bitten. *Gate.* Eval pairs (task A then task B in the same repo): B's rounds and re-reads down with memory on.

---

## 6. Expected outcome, honestly bounded

| Measure | Today (measured) | After Phases 1–5 (estimate) | Basis |
|---|---|---|---|
| Standard prefix, built-in only | ~10k tokens | ~5k | §2.2 C2 arithmetic |
| Standard prefix, measured setup (skills + MCP) | ~33k tokens | ≤ 10k | Anthropic −85% on deferred tools; skills by reference |
| Cache-read share of input, Anthropic-direct | ~0 (no markers) | ≥ 80% after round 3 | Anthropic prompt-caching semantics |
| Rounds per Builder task | median 6 (whole sessions) | −30% to −50% | Phase 3 + 4 round accounting; more on document-shaped work (4b) |
| Brain rounds per session | median 40 | ≤ 15 | Phase 5 |
| **Cost per solved task (M1)** | baseline in Phase 0 | **−40% to −60%** on Anthropic-direct; **−25% to −40%** on claudin.io | sum of the above; the wide range is the point of Phase 0 |
| Pass rate (M3) | baseline in Phase 0 | not down; up on multi-edit and long tasks | Cognition, Chroma, JetBrains |

The numbers in the third column are *estimates built from the sources cited*; none of them is a promise, and Phase 0 exists so that each later phase replaces its row with a measurement.

---

## 7. Order, and why

```
Phase 0 ──► Phase 1 ──► Phase 2 ──► Phase 3 ──► Phase 4 ──► Phase 4b ──► Phase 5
 (eval)     (cache)     (prefix)   (edits)     (tasks)     (document)   (prompt)
                │
                ├──► Phase 6 (context quality)   — any time after 0
                ├──► Phase 7 (parallel tools)    — any time after 0
                ├──► Phase 8 (split the loop)    — best *before* 3–5, it makes them cheaper
                └──► Phase 9 (memory)            — after 5
```

Phase 1 before 2 because 1 is pure gain with no model-visible change, and because 2's "deferred tools cost one cache write per load" is only true once there is a cache. Phase 2 before 3 because a main session that edits carries more context, and should carry a smaller prefix first. Phase 4b after 4 because it reuses 4's task-list-as-view and 3's section grant for subagents; it could also absorb 4 entirely if the eval says the document is the better task list. Phase 8 is placed early on purpose: splitting `session.rs` is the single change that makes 3, 4 and 5 safe to do in small commits.

Each phase is one PR series on its own branch, merged when the eval gate is green, in the commit discipline this repo already keeps (test first; a prompt line removed carries the eval task or gate test that justifies it).

---

## 8. Decisions for the maintainer

| # | Decision | Recommendation |
|---|---|---|
| D1 | Does the Standard Builder session get `edit_file` back (Phase 3)? | **Yes.** Keep code subagents as an option with stated criteria. The Compact profile already proves the path. |
| D2 | Deferred tools: is one cache miss per `load_tools` acceptable? | **Yes**, bounded by the cache-breaking counter (≤ 2 per session median). The alternative — carrying ~20k tokens of schemas on every round — is the thing we are removing. |
| D3 | External eval track (SWE-bench Verified subset + TB 2.1) — in Phase 0 or later? | **In Phase 0**, small (50 + 20). It is the only way "unbeatable" becomes a sentence with a number in it. |
| D4 | Keep Jev? | **Yes, behind one trait** (Phase 8), with masking-first so it decides less. It is cheap and fail-open; the cost was conceptual, not monetary. |
| D5 | The Brain interview: adaptive (Phase 5) or keep mandatory one-question-at-a-time? | **Adaptive.** Median 40 rounds is the harness being heavy in the user's hands; the eval rubric guards against under-asking. |
| D6 | Where does the project memory live — `.claudinio/MEMORY.md` or `AGENTS.md`? | `.claudinio/MEMORY.md`, harness-owned; *read* `AGENTS.md`/`CLAUDE.md` if present as user-owned context (many repos already have one). |
| D7 | Ship Phase 5 behind a profile flag for one release? | **Yes.** |
| D8 | Phase 4b: does the living document *replace* the plan file and become the only task list when present, or coexist with `tasks_*`? | **Replace, when a document exists.** One mechanism the user can see and edit beats two the model has to keep in step. Sessions without a document keep `tasks_*`. |
| D9 | Phase 4b: may `explore` subagents write their own section? | **Yes** — one section, one file under `.claudinio/`, granted by the parent. It is what removes the report round. |

---

## 9. Sources

Primary texts, in the order they appear above. Figures marked *(secondary)* in the body were read from summaries and should be checked against the primary before being quoted elsewhere.

- Weinberger, Hozez — *Token Reduction Is Not Cost Reduction* — arXiv 2607.12161 (2026). https://arxiv.org/abs/2607.12161
- Manus — *Context Engineering for AI Agents: Lessons from Building Manus* (2025). https://manus.im/blog/Context-Engineering-for-AI-Agents-Lessons-from-Building-Manus
- Anthropic — *Effective context engineering for AI agents* (2025). https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- Anthropic — *Advanced tool use* (Tool Search Tool, Programmatic Tool Calling, Tool Use Examples) (Nov 2025). https://www.anthropic.com/engineering/advanced-tool-use
- Anthropic — *Writing effective tools for agents* (2025). https://www.anthropic.com/engineering/writing-tools-for-agents
- Anthropic — *Building effective agents* (Dec 2024). https://www.anthropic.com/engineering/building-effective-agents
- Anthropic — *How we built our multi-agent research system* (2025). https://www.anthropic.com/engineering/multi-agent-research-system
- Anthropic — *Prompt caching* (API docs; breakpoints, 10% read price, 4-breakpoint limit). https://docs.claude.com/en/docs/build-with-claude/prompt-caching
- Cognition — *Don't Build Multi-Agents* (2025). https://cognition.ai/blog/dont-build-multi-agents
- Lindenbauer et al. — *The Complexity Trap: Simple Observation Masking Is as Efficient as LLM Summarization for Agent Context Management* — arXiv 2508.21433 (NeurIPS 2025 DL4Code). https://arxiv.org/abs/2508.21433 · https://blog.jetbrains.com/research/2025/12/efficient-context-management/
- Fan et al. — *An Empirical Study of Harness Design for Coding Agents* — arXiv 2609.20804 (2026). https://arxiv.org/abs/2609.20804
- *Harness Engineering: Anatomy, Architecture, and Evolution of Coding Agents — A Source-Code Study of Eleven Systems* — arXiv 2609.00006 (2026). https://arxiv.org/abs/2609.00006
- Yang et al. — *SWE-agent: Agent-Computer Interfaces Enable Automated Software Engineering* — arXiv 2405.15793 (2024). https://arxiv.org/abs/2405.15793
- SWE-bench *Bash Only* leaderboard (mini-SWE-agent). https://www.swebench.com/bash-only.html
- Zhang et al. — *Agentic Context Engineering: Evolving Contexts for Self-Improving Language Models* — arXiv 2510.04618 (ICLR 2026). https://arxiv.org/abs/2510.04618
- Chroma — *Context Rot: How Increasing Input Tokens Impacts LLM Performance* (2025). https://research.trychroma.com/context-rot
- Liu et al. — *Lost in the Middle* (2023). https://arxiv.org/abs/2307.03172
- OpenAI — *Harness engineering: leveraging Codex in an agent-first world* (Feb 2026). https://openai.com/index/harness-engineering/
- Merrill et al. — *Terminal-Bench: Benchmarking Agents on Hard, Realistic Tasks in Command Line Interfaces* (ICLR 2026). https://arxiv.org/abs/2601.11868
- This repository — `docs/plans/2026-10-06_harness-paper-learnings.md` §2 (session measurements), `docs/ARCHITECTURE.md`, `scripts/session-stats.py`.

---

## Appendix A — Measurements taken for this audit (2026-10-18, `f3f6026`)

| What | Value | How |
|---|---|---|
| Rust / TypeScript source lines | 62,702 / 34,944 | `wc -l` over `src-tauri/src`, `src` |
| Largest files | `session.rs` 8,446 · `provider.rs` 2,789 · `persist.rs` 2,782 · `prune.rs` 2,609 · `tools/mod.rs` 2,170 · `ChatPanel.tsx` 2,453 | same |
| `SYSTEM_PROMPT` + golden + mode blocks | ~6.3k + ~1.5k + ~6k (Builder) / ~11k (Brain) chars | `sed -n` over `session.rs` 466–870 |
| Built-in tool definitions, source | 31,834 chars across 24 defs; top: `ask_user` 4,063, `browser` 2,899, `spawn_agents` 1,974, `browser_screenshot` 1,905, `tasks_set` 1,876, `browser_inspect` 1,664, `semantic_search` 1,629 | split of `tools/mod.rs` 225–660 on `ToolDef {` |
| Built-in skills inlined in every prompt | `find-skills` 1,370 + `plugin-crafter` 5,278 chars | `skills.rs` 170–308 |
| Imperatives in the three prompt blocks | 8 MUST · 3 MANDATORY · 2 NEVER · 1 STRICT | grep |
| `cache_control` occurrences in providers | 0 | grep over `provider.rs`, `provider/openai.rs` |
| `tauri::ipc::Channel` coupling in the core | 22 sites in `session.rs`, 7 in `subagent.rs` | grep |
| Tool execution within a round | sequential (`for (ti, tool_use) in tool_uses.iter().enumerate()`) | `session.rs` 3406 |
| Standard Builder main session offers `edit_file` | no (`defs.retain(|t| t.name != "edit_file")`) | `session.rs::api_tools` |

## Appendix B — What this plan deliberately does not do

- Go bash-only for any model by default (the Mistral row of 2609.20804; rule 8).
- Remove the Task Panel, Brain mode, the quality gate, the golden loop or the approval gates. They are the product; the plan makes them cheaper to carry, not absent.
- Adopt an agent framework or a vector store for retrieval (0 of 11 production harnesses do; our FTS5 + ONNX index is already the better version of that idea).
- Rewrite history early on a cached provider (unchanged from the 2026-10-05/06 plans).
- Trust any estimate in §6 over a Phase 0 measurement.

---

## Implementation Log (lean harness, 2026-10-18)

### Phase 8 — `session.rs` split by responsibility (no behaviour change)

**Commits.** `7436fb2` prompt + events · `c181ade` judge + retry + handoff · `7dba5d8` tool_exec · `15e300e` run_loop + first-prompt routing into `route.rs` · `9d646fa` the 2,000-line architecture test · and the docs commit that adds this entry. Each commit compiles, is clippy/fmt clean and passes `cargo test --lib` on its own (1022 passed / 8 ignored until the last code commit, 1024 / 8 after it — the two new architecture tests).

**What was built.** `agent/session.rs` went from 8,446 lines (5,326 production) to 746 (464 production in the old counting; 498 by the test's rule, which also counts module docs and imports). The code moved verbatim; `git diff` is moves plus imports, module docs, and `pub(super)` on items another file now needs (visible inside `agent/` only). A line-multiset comparison against the original file shows nothing else changed except the items listed under deviations.

| File | Production lines | Holds |
|---|---|---|
| `run_loop.rs` | 1,855 | `run_workflow`, `run_workflow_with_profile`, golden/quality caps, `run_finish_line_checks`; `finish_line_tests`, `surface_tests`, `context_line_tests` |
| `tool_exec.rs` | 1,417 | `effective_permission`, `run_tool`, `execute_and_report`, approval gate, `deny_tool`, `ask_user`, mode switches, `fit_round`/`tool_result_block`/`rich_result_block`, `truncate` |
| `handoff.rs` | 557 | `HandoffReason`/`HandoffSpec`/`RunOutcome`, `compact_history`, `compute_tail_turns`, `shed_line`, `accept_prune`, `shed_options`, `try_verbatim_compaction`, `maybe_context_handoff` |
| `prompt.rs` | ~460 | `SYSTEM_PROMPT`, `GOLDEN_PROMPT`, `GIT_SYNC_PROMPT`, mode blocks, `system_prompt`, `build_spec_prompt_section`, `run_prefix`, `api_tools` |
| `events.rs` | ~280 | `AgentEvent`, `QualityLayerView`, `EditProposalData` |
| `judge.rs` | ~165 | `TurnVerdict`, `parse_turn_verdict`, `judge_terminal_turn`, Jev completion questions/thresholds, `judge_with_jev`, `loop_verdict`, `should_nudge_terminal` |
| `retry.rs` | ~110 | `is_retryable_error`, `stream_message_with_retry` |
| `session.rs` | 498 | `SessionMode`, `PromptProfile`, `ModeOrigin`, `ModeCtl`, `SteeringCtl`, approval/answer maps, token estimation, `push_turn`/`push_user_blocks`/`inject_steering`, `write_status`, pricing; re-exports the names other modules reach through `agent::session` |

`architecture_tests::no_agent_file_carries_more_than_two_thousand_production_lines` fails the build when any `.rs` under `src/agent/` (recursive) passes 2,000 lines outside its `#[cfg(test)]` modules. It was written first and seen red ("src/agent/session.rs (5350)"). "From the first `#[cfg(test)]` to end of file" does not hold — `tools/mod.rs` opens `tests_support` at line 90 — so each gated `mod … {` is skipped up to its column-0 `}` (rustfmt's shape); `production_lines_leave_out_only_the_test_modules` pins that rule.

**Deviations from the plan and the brief, and why.**

- **Two more files than suggested: `tool_exec.rs` and `retry.rs`.** The brief put tool execution in `run_loop.rs`, but the loop alone is ~1,770 lines and tool dispatch ~1,400: together they would be ~3,200, over the 2,000-line rule this phase adds. Retries (`stream_message_with_retry`) are used by both the loop and the handoff, so they got their own small file instead of making `handoff.rs` depend on `run_loop.rs`.
- **`route_first_prompt` moved to `route.rs`, not `run_loop.rs`.** It is the session side of the router `route.rs` already holds ("this one judgement, once, on the first prompt of a fresh session"), and in `run_loop.rs` it would push that file to ~1,970 lines, leaving no room for the next phase's changes. Its `route_tests` moved with it. The plan named the file `loop.rs`; it is `run_loop.rs` as the brief suggested.
- **Facade, not call-site updates.** `session.rs` re-exports `AgentEvent`, `HandoffReason`, `HandoffSpec`, `RunOutcome`, `compact_history`, `run_workflow(_with_profile)` (pub) and `accept_prune`, `shed_options`, `loop_verdict`, `run_tool`, `deny_tool`, `fit_round`, `MAX_STOP_HOOK_BLOCKS`, `tool_response_for_hook` (pub(crate)), so `commands/`, `subagent.rs`, `transition.rs`, `state.rs` and the tests compile unchanged.
- **The source-scanning test** (`a_message_in_any_language_is_never_refused`, which read `include_str!("session.rs")`) now scans every file holding a piece of the old `session.rs` (session, run_loop, tool_exec, handoff, judge, prompt, events, retry, route). Checked that it still bites: planting the refusal string in `run_loop.rs` makes it fail naming that file.
- **`dummy_ctx` is copied, not shared.** The permission tests (now in `tool_exec.rs`) and the `push_user_blocks` tests (still in `session.rs`) both use it. Copying the 25-line helper verbatim kept the tests a pure move, and `tools/mod.rs` already has the same shape (`tests_support::ctx` and its own `test_ctx`).
- **Test imports.** A moved test module that reached a name through `use super::*` from the old file now imports it explicitly (e.g. `surface_tests` imports `tools`, `api_tools`, `system_prompt`, `shed_line`). The redundant `use crate::agent::route::RouteMode;` in `route_tests` was dropped, since that module now lives in `route.rs`. No assertion changed.
- **Not done, by decision: the Jev `Judge` trait.** Every call site is already a fail-open `Option<&JevBackend>` (`judge_with_jev`, `loop_verdict`, `prune::shed`, `route::route`); a trait would add a concept without removing one.
- **Not done, by decision: GitSync as a skill.** GitSync's tool restriction (`bash` + `ask_user` only, `api_tools`) is a policy enforced in code. Turning it into a skill would move that policy into prose, against design rule 4 ("policy lives in code; the prompt explains, it does not enforce"). `PromptProfile` keeps its three variants.
- **Flake seen once, unrelated:** `hooks::runner::tests::hooks_run_in_parallel_and_keep_their_order` (wall-clock) failed once on this host, which also serves production traffic; it passed on three isolated reruns and on the next full run.
