# One timeline, one plan: removing Brain/Builder and making the task list and the living document one thing

**Status:** proposal, approved in direction by the maintainer (2026-10-20); nothing is implemented.
**Written against:** `main` at `bac6206` (0.8.0). Paths are relative to `src-tauri/src/` unless they start with `src/`, `eval/` or `docs/`.
**Companion to:** [`2026-10-18_lean-harness-audit-and-plan.md`](2026-10-18_lean-harness-audit-and-plan.md). That plan made each request lighter. This one makes the *product* lighter: fewer concepts for the person in the chair and for the model.

---

## 0. The one-paragraph version

Claudinio Code today asks a person to understand two modes (Brain, Builder, plus an Auto router that guesses between them), two models (one per mode), and two progress views (the Task Panel and the living document) — and it asks the model to choose between **three** planning surfaces (`tasks_set`, `doc_create`, `write_plan`) with no rule that tells them apart. The maintainer's first real use of the living document showed the cost: the panel hid the task list while the model kept writing to it; the document grew to 13 sections because the plan template (Solution Design, Risks, Rollout, Open Questions…) leaked into it; and a schema bug (`"section": ""`) made 3 of 4 `spawn_agents` calls fail whole, so 7 subagents never started and the main timeline did their work — then rewrote three of the sections the subagents had written. The plan below removes the mode split entirely (**one timeline**, one model per session, the agent decides whether a request needs a document), and makes **one plan per session**: nothing for a one-step change, a short checklist for multi-step code work, a short document (3–6 sections, independent questions filled in parallel) for research, design or a write-up — exclusive in code, shown in one panel. Golden goals, the quality gate and the finish-line check — the parts that make the harness trustworthy — are kept and stop depending on modes.

## 1. What is being optimised, and how we will know

The product goal is **simpler to use**. It is measured with the lean-harness eval (`eval/`, `examples/claudinio-eval.rs`) plus three new numbers:

| # | Metric | Gate |
|---|---|---|
| M1 | Billed cost per solved task (cache-aware), as in the lean-harness plan | not worse than the `bac6206` baseline |
| M3 | Pass rate on the eval | not worse (25/25 on the last baseline) |
| M4 | **Tool-call error rate** (results that are errors / all tool calls), main + subagents | down; reported per tool |
| M5 | **Planning surfaces used per session** (`tasks_set`, `doc_create`, `write_plan`) | ≤ 1 by construction after Phase 1 |
| M6 | **Share of document sections written by subagents**, on the `doc-*` eval subset | up; ≥ 50% on research-shaped tasks |

Every number above is taken before and after each phase; a phase that worsens M1 or M3 is reverted, as in the lean-harness rules.

## 2. What is wrong today (evidence)

**2.1 Three planning surfaces, no rule.** The v2 base prompt says "for anything beyond a one-step change, keep a task list" and, in the next section, "work with several distinct parts (research, a write-up, a multi-part feature): call `doc_create` first". A multi-part feature matches both. Brain mode adds a third, `write_plan`, with its own template (Context, Solution Design, Risks, Non-goals, Low-Level Design). Both base prompts advertise `tasks_*` and `doc_*`; `api_tools` offers `doc::session_defs()` in both modes (`agent/prompt.rs:385-387`); a run's user turn carries both a `<tasks>` and a `<document>` block (`agent/run_loop.rs:240-249`); compaction recites both.

**2.2 Decision D8 was implemented in the UI only.** The lean-harness plan chose "the document replaces the task list when present" (D8). The implementation log records it as *partial* (`write_plan`, the LLD gate and golden goals untouched). The result: `App.tsx:1309` hides `TasksPanel` whenever a document exists, while the model can still call `tasks_set`/`tasks_update`. A person cannot see a list the model is maintaining — **including golden goals**, which disappear from view as soon as a document exists.

**2.3 The document has no size, and its sections are not shaped for parallelism.** `doc_create` takes any number of sections and nothing says how many. The maintainer's session produced 13, verbatim: `["Aderência ao plano e objetivo", "Achados do código", "Solution Design", "Implementation Plan", "Execution Plan", "Execution Notes", "Decisions", "Assumptions", "Risks", "Testing", "Rollout", "Open Questions", "Next Steps"]` — the plan template, not a set of questions. Most are *views of the same decision* (Risks depends on the design, Next Steps on everything), so they cannot be filled in parallel however well delegation works. The same session also kept a task list next to the document (`tasks_set` with 3 tasks before `doc_create`, 9 tasks after), and called `finalize_plan` twice with a document open — all three planning surfaces in one session (2.1).

**2.4 The mode split is a concept people pay for and the model ignores.** From the code and the 2026-10-06 measurements:
- the model chose to plan on its own 8 times in 16,945 tool calls (`enter_plan_mode`); the eval baselines never called `enter_plan_mode`, `write_plan` or `exit_plan_mode`;
- `agent/route.rs` (1,008 lines, ~350 non-test) exists only to guess the mode, and sends every first prompt to Jev;
- the Brain → Builder switch throws the context away: Builder starts empty, reads the plan file and re-explores what Brain already read — the same "delegation tax" Phase 3 of the lean-harness plan removed for subagents (cost per solved task −82% when the main session started editing itself);
- `finalize_plan` was called 54, 39 and 7 times across the three eval baselines and **fails when there is no plan file** — pure error rounds (M4);
- the Compact profile is already one timeline with a task list and passes the eval.

**2.5 Tool failures around the document — measured.** Source: Langfuse traces of the maintainer's session (0.8.0, mimo-v2.6-flash then deepseek-flash; main `fb4cb4d615a93162` 188 requests + successor `fc2bd5c3a6992b1e` 161 requests, 2026-10-19 13:16 → 2026-10-20 10:28; four section subagents, 69 requests). Main context peaked at 186.6k tokens before a handoff.

| # | Failure | Count | Cause | Consequence |
|---|---|---|---|---|
| F1 | `spawn_agents` with `"section": ""` → `not started: this session has no document…` / `the document has no section ''` / `not started:  is given to another agent in this call` | **3 of 4 `spawn_agents` calls failed whole; 7 subagents never started** | `section` is optional in the schema, and the model fills an unused optional string with `""`; the harness treats `""` as a section id (so a plain explore agent needs a document, and every second `""` in a call collides with the first) | **This is why the main timeline did the work.** After the third failure (three plain explore agents at 14:41) the model never retried and investigated itself. |
| F2 | The error text itself: an empty id ("not started:  is given…") and "Two agents cannot hold one section." appended to every failure, whatever the cause | every F1 | message composition in the spawn path | the error does not teach the fix |
| F3 | Section subagents' `doc_read` of a *finished* sibling section → `you were given section s4 only; s3 belongs to another agent` | 4 | `owner_for` gates reads like writes | subagents cannot build on each other's findings; a wasted round each |
| F4 | Main `doc_write` over subagent-written s3, s4, s5 | 3 | main may write any section; the owner becomes `main` | the panel shows only s2 as a subagent's (the screenshot); the subagents' text was replaced by a rewrite in the main context — the very cost the document exists to avoid |
| F5 | `doc_read` ×8 in parallel on sections the outline already showed as unwritten | 8 rounds' worth of results | nothing says the outline is enough | wasted calls |
| F6 | `tasks_set` → `invalid args: missing field 'title'`; `tasks_update` → `pass either one task … or several as {"updates": [...]}, not both` | 2 | schema with required fields the model omits; two call shapes | error rounds (simplified away in Phase 5) |
| F7 | `read_file` → `end_line must be >= start_line` ×3, `start_line (260) exceeds file length (250)` ×1 | 4 | strict line arguments | error rounds where clamping would have answered |
| F8 | `unknown tool: trompt`, `unknown tool: ` (empty name) | 2 | model-side (mimo) | out of scope; noted |

Read-before-edit and new-file refusals (`edit_file` ×9) are the gates doing their job and are not counted as defects.

## 3. Design rules for this plan

1. **One concept per job.** One timeline. One model per session. One plan per session. One panel.
2. **Policy in code, not prose** (the lean-harness rule 5). Exclusivity between the checklist and the document is a tool refusal whose error message teaches, not a sentence the model may skip.
3. **Keep what makes the harness trustworthy:** golden goals, `run_quality`, the quality gate, the finish-line check, read-before-edit, the write gates, the completion judge.
4. **Old sessions still open.** Removed record kinds stay deserializable and render as history.
5. **Red test first, every phase** (the repo's TDD rule); each phase is one PR, eval before and after.

## 4. The design

### 4.1 One timeline

- `SessionMode` goes away. A session is a session: it reads, asks, edits, runs. What used to be "plan first" is the agent's judgement from the request — a request that needs decisions gets a short document and an `ask_user` with the decisions; a request that is clear gets done.
- **Removed:** the Brain/Builder/Auto control, `agent/route.rs` and the Jev route setting, `enter_plan_mode`, `exit_plan_mode`, `write_plan`, `finalize_plan`, the LLD gate, the Brain bash allowlist and edit gates, the Brain explore-streak guard, the `PlanExecution`/`ManualBuilder`/`GoldenFlip` handoffs, `continue_with_builder`, `set_session_mode`/`get_session_mode`/`check_plan_exists`, the Brain/Builder prompt blocks and their nudges.
- **One model per session.** `brain_model`/`builder_model` become `model`, migrated one-way where `provider.rs:727-760` already migrates the legacy `model` field: the new value is the old `builder_model` (what executed the work). Subagents and the completion judge use the session model unless the person sets a separate subagent model in Settings (a field that already exists implicitly today as `builder_model`). *(Decision S1 below.)*
- **Read-only work** is not a mode: the permission system already asks before edits and non-allowlisted bash; a person who wants "just look" says so. The Brain bash allowlist folds into the general allowlist where it is stricter.
- **The `plan` skill** survives as the `design` skill: the adaptive interview, the "(Recommended)" answer, verbatim user assets (session 44ec41c1), sizing, Mermaid — rewritten to write into the living document instead of `write_plan`.
- **`MEMORY.md`** loses its only writer (`finalize_plan`). It moves to a small deferred `remember(lines)` tool with the same caps (≤ 5 lines per call, `project_memory::append`). The implementation log `finalize_plan` appended to a plan file is dropped: commits and the document already hold it.

### 4.2 One plan per session

| The request | The plan | Where it shows |
|---|---|---|
| One step | none | panel: "single step" (exists since Phase 4) |
| Multi-step code work | **checklist**: `tasks_set` with `{id, title}` and `tasks_update` with `{id, status, note?}` | Plan panel, as rows |
| Research, design, a write-up, or the user asks for a document | **document**: `doc_create` with **3–6 sections**, each an independent question | Plan panel, as the outline |

- **Exclusive in code.** With a linked document, `tasks_set` is refused: *"this session's plan is its document — add a section with doc_add, or close one with doc_write"*. `doc_create` in a session whose checklist has open items is refused the other way unless the request is a new top-level topic (which already hands off to a new session). Golden goals are exempt: they are the user's, and stay as they are (4.3).
- **Short by default, long when asked.** `doc_create` refuses more than 6 sections with an error that says why ("a document is a handful of independent questions; add more with doc_add if the work or the user needs them"). `doc_add` stays uncapped — a person who asks for a long document gets one, a section at a time.
- **Shaped for parallelism.** The `doc_create` description and the LIVING DOCUMENT prompt section say: sections are *independent questions* (not Risks/Rollout/Next steps of one answer); right after `doc_create`, give every independent section to an explore subagent **in one `spawn_agents` call**; the main timeline writes only the synthesis section (the answer/decision) once the others are done. The `doc_create` result already suggests delegation; it will name the sections and the one call.
- **Leaner checklist.** `description` and `journal` leave the task schema (the journal's only reader was `finalize_plan`); `tasks_update` keeps an optional one-line `note`. Fewer fields, fewer schema errors (M4).
- **One panel.** `TasksPanel` and `DocumentPanel` merge into a `PlanPanel`: golden goals pinned on top (when present), then either the checklist rows or the document outline — same row shape, same status chips. The list never disappears.

### 4.3 Golden goals without modes

Golden goals are created from `<goal>` tags (`session::parse_goals`), stored as tasks, protected from the model, and gated by `check_quality_gate`. Today each goal is two tasks (`-0` "plan", closed by Brain; `-1` "execute", gated). Without modes:

- one task per goal (`golden-<slug>`), gated exactly like today's `-1`; legacy `-0` ids load and are treated as done;
- the end-of-turn "golden flip" (a handoff to the opposite mode while a goal is pending) becomes a **continue in the same session**: one nudge with the pending goals, same caps (`max_golden_cycles`, `max_golden_stalls`), same stall/stop records;
- golden goals coexist with either plan form and are always visible in the Plan panel.

### 4.4 The prompt

- v2 loses `BRAIN_BLOCK`/`BUILDER_BLOCK` and gains ~5 lines in the base prompt (the table in 4.2, as prose, plus "edit directly; delegate broad reading"). Target: Standard prefix *smaller* than today's.
- **v1 is removed** (decision S2). It was kept "for one release" (D7) and 0.8.0 has shipped; keeping it means maintaining Brain/Builder texts byte-pinned by fixtures for a mode that no longer exists.

## 5. The phases

Each phase is one PR, red tests first, eval (sandboxed, `eval/README.md`) before and after.

### Phase 0 — Baseline and the two new meters
- Eval on `bac6206`, v2, deepseek-flash, 2 × 25 → `eval/baselines/2026-10-20_pre-single-timeline/`.
- `scripts/session-stats.py` gains M4 (tool error rate per tool, main vs subagent) and M6 (sections by owner on sessions with a document). Tests in `scripts/test_session_stats.py`.
- Two eval tasks tagged `doc-parallel`: a research question with 3–4 independent sub-questions over this repo (graded on the content of the document and on M6).

### Phase 1 — The document's tool failures (F1–F5, F7)
*Smallest change, biggest effect on what the maintainer saw; ships on its own as 0.8.1.* One reproducing test per failure, seen red on an assertion, then the fix.
- F1: an empty or whitespace `section` means *no section* (`subagent.rs` spec parsing); the schema description says "omit unless the agent writes a document section".
- F2: each refusal states its own cause with the id quoted; the "two agents" sentence only on a real collision.
- F3: a section subagent may `doc_read` any section (and the outline); it still writes only its own.
- F4: main's `doc_write` over a section a subagent wrote keeps the attribution (`by=sub:<name>, edited=main`) and the panel shows both; the prompt says to synthesise in the synthesis section rather than rewrite delegated ones.
- F5: `doc_read` of an unwritten section answers from the outline without a round's worth of text, and the prompt line says the outline is enough to see status.
- F7: `read_file` clamps `start_line`/`end_line` to the file and swaps an inverted range, saying so in one line, instead of refusing.
- Gate: M4 down on the `doc-*` and `doc-parallel` subsets; M6 up.

### Phase 2 — One plan, exclusive in code; one panel
- `tasks_set` refused with a linked document (non-golden items); `doc_create` refused over open checklist items unless it hands off; prompt sentences updated.
- `PlanPanel` replaces the two panels; golden goals always visible.
- `doc_create` cap (6) and the parallel-shape wording in the description, prompt and result.
- Tests: `tools/tasks.rs` (refusal + golden exemption), `doc_session.rs` (cap, refusal), `src/components/PlanPanel.test.tsx` (checklist, outline, golden pinned, single step).

### Phase 3 — One timeline (backend)
- Remove `SessionMode`, `ModeCtl`, `route.rs`, plan tools, LLD gate, Brain gates and guards, mode handoffs and their commands; model config migration; subagents/judge on the session model.
- Golden goals per 4.3. `remember` tool. `design` skill.
- Persist: `Mode`, `ModeRoute`, `GoldenCycle.mode`, `PlanFinalized`, `RunConfig.mode` stay deserialize-only; `last_mode()` and `state.mode_for` go.
- v1 prompt and fixtures removed; v2 rewritten per 4.4; prefix byte-stability and size tests updated (the size gate must go *down*).
- Tests: the existing mode tests are deleted with the code they cover (said in the commit, not silently); new tests for migration, golden continue, legacy record loading, `remember`.

### Phase 4 — One timeline (frontend)
- Remove the mode control (`ChatPanel.tsx:2285-2322`), `lib/modeChoice.ts`, `lib/autoMode.ts`, "Read Plan"/"Continue with Builder", mode rows for new sessions (old ones still render), Settings: Brain/Builder pickers → one model picker, Jev route, "Auto-commit plan", plan save path.
- The "Plans" popover lists the workspace's documents (`.claudinio/docs/`) instead.
- `ipc.ts` types follow the backend; `normalizeSessionMode` stays lenient for old records.

### Phase 5 — Lean checklist schema; docs
- `tasks_set` `{id, title}`, `tasks_update` `{id, status, note?}`; old `Tasks` records with description/journal still load.
- README.md, README.pt-BR.md, ARCHITECTURE.md, the eval README; `hist-auto-mode-choice` marked historical.
- F6 disappears with the two-field schemas (one call shape for `tasks_update`: `updates: [...]`).
- Release 0.9.0.

## 6. Expected outcome, honestly bounded

- **Concepts a new user meets:** modes 3 → 0, model pickers 2 → 1, progress panels 2 → 1, planning tools for the model 3 → 2 (exclusive).
- **Code:** roughly −3k to −4k lines of non-test Rust/TS (route.rs, write_plan, finalize_plan, mode plumbing in run_loop/tool_exec/handoff/transition, v1 prompt, frontend mode control and settings), and more in tests.
- **M1/M3:** the eval already runs in Builder only, so the core loop is unchanged; the guess is *neutral to slightly better* M1 (no `finalize_plan` error rounds, smaller prefix) and unchanged M3. Document-shaped work should improve on M1 and M6; this is a guess until Phase 0's `doc-parallel` tasks measure it.
- **What we lose:** a cheaper-model/premium-model split by phase (S1), and a hard read-only mode. Both are judged worth less than the concepts they cost.

## 7. Decisions for the maintainer

| # | Question | Recommendation |
|---|---|---|
| S1 | The premium model for planning: keep a separate model for some work, or one model per session? | **One model per session**, chosen in the model picker. Simplest; the person picks claudius for a hard session. Revisit with data if eval shows planning quality drops. |
| S2 | Remove prompt v1 now? | **Yes.** It only exists to roll back the 0.8.0 prompt and is written around the modes being removed. |
| S3 | Document section cap | **6 at `doc_create`**, uncapped `doc_add`. |
| S4 | Keep `MEMORY.md` writes? | **Yes**, via a deferred `remember` tool. |

## Appendix A — Inventory used for this plan

Gathered 2026-10-20 at `bac6206`. Core mode state: `agent/session.rs:146-273`, `state.rs:161-249`. Router: `agent/route.rs` (all), `agent/jev.rs:55-163`, `commands/agent.rs:774-871`. Plan tools: `agent/tools/write_plan.rs` (439), `agent/tools/finalize_plan.rs` (645), `agent/tools/mod.rs:148-155, 687-760, 1025-1079`, `agent/tool_exec.rs:887-1030`, LLD gate `agent/tools/tasks.rs:292-320`. Prompt: `agent/prompt.rs:150-397`, `agent/prompt/v1.rs`, `agent/prompt/v2.rs`, `agent/deferred.rs`, `agent/surface.rs:232-257`, `agent/permissions.rs:189, 542`, `agent/gate_text.rs`, `agent/nudges.rs`, `skills.rs:298-318`. Run loop: `agent/run_loop.rs:260-319, 541-660, 973-975, 1048-1275, 1465-1660, 1846-1890`. Handoff/transition: `agent/handoff.rs:155-608`, `agent/transition.rs`. Config: `agent/provider.rs:127-811`, `commands/agent.rs:750-967, 1185-1503`, `commands/providers.rs:727-731`, `commands/enhance.rs`. Golden goals: `session.rs:46-58`, `tools/tasks.rs:334-525`, `run_loop.rs:1048-1135`, `persist.rs:522-585`. Persisted kinds: `persist.rs:52-411`. Frontend: `ChatPanel.tsx` (46 refs), `lib/modeChoice.ts`, `lib/autoMode.ts`, `lib/ipc.ts`, `lib/chatRecords.ts`, `components/chat/TimelineRows.tsx`, `App.tsx:1300-1325`, Settings (`SettingsModels`, `SettingsPanel`, `SettingsToolSurface`, `SettingsGeneral`, `SettingsJev`, `SettingsPromptVersion`, `SettingsLocalModels`).

## Implementation Log (2026-10-20)

Every phase was implemented red-first; each step is its own commit. Where a test that pinned the old behaviour went red, the commit says whether the code was wrong or the rule changed on purpose.

### Phase 0 — #99 (merged)
M4 (tool error rate) and M6 (sections written by subagents) in the eval runner and `scripts/session-stats.py --meters`; two `doc-parallel` tasks; baseline in `eval/baselines/2026-10-20-doc-parallel-before/`. On the old binary only 1 of 4 sessions delegated, and that one then rewrote half of what its subagents wrote ($0.037 vs $0.010 for the same task without subagents) — F4 measured.

### Phase 1 — #97 (merged)
F1 (blank `section` = no section), F2 (each refusal states its own cause), F3 (section agents read siblings; the old "reads only that section" test changed on purpose), F4 (main revising a finished subagent section keeps its author). **Not done: F7** — clamping `read_file` ranges would make `result_cap`'s line numbering lie; the error already says what to do. **F5** (reads of unwritten sections) became a prompt line only: the outline already shows status.

### Phase 2 — #98 (merged)
`doc_create` drops the non-golden checklist; `tasks_set` refused with a document (error names `doc_add`/`doc_write`); `doc_create` caps at 6 sections; v2's TASKS + LIVING DOCUMENT became one PLAN section; the task panel became rows and no longer disappears under a document (golden goals shown as "Goals").

### Phases 3 and 4 — #100
Router removed; one model (`model`, migrated from `builder_model`); `SessionMode`/`ModeCtl`, the four plan/mode tools, the LLD gate, Brain gates and guards, mode handoffs and commands, prompt v1 and its fixtures removed; the app's mode control, plan buttons and mode settings removed; "Documents" replaces "Plans". Default Standard prefix 4,995 → 4,824 tokens.

Deviations from §4:
- **Golden goals keep the `-1` id suffix** (`golden-<slug>-1`): one task per goal as planned, but with the id old sessions already carry, so `is_golden_execution` and stored goals need no migration. A legacy `-0` half no longer blocks.
- **`remember` is core, not deferred.** The memory guard's refusal points at it, and no harness text may order a deferred tool (a load is a prefix rewrite).
- **`set_workspace_config` is removed too** — its only job was `plan_save_path`.
- **`ModeRoute`, `Mode`, `GoldenCycle.mode`, `RunConfig.mode` and `PlanFinalized`** stay deserializable; a test loads a session with all of them.
- **The Brain bash allowlist is gone**, not folded in: the session's own allowlist already contains everything it listed, and the eval's unattended read-only list is now pinned as a subset of it. `pytest --basetemp=src`-style findings that Brain refused still run unasked in a session, as they did in Builder before this plan; unchanged here.

### Phase 5 — code
`tasks_set` requires only `id` and `title`; a known task resent without title/status keeps them; `tasks_update` lets a named top-level task join a batch (F6). Docs (README, README.pt-BR, ARCHITECTURE, eval README) follow in the same PR.

### Measurements (2026-10-20, Phases 1–4 at `59de40e` + #99's tasks; deepseek-flash, 2 runs, same sandbox and prices as before; raw output in `/srv/claudinio-eval/out/st/`)

25 baseline tasks (final a / b → st a / b): M1 $0.0060 / $0.0070 → **$0.0059 / $0.0063**; M2 15 / 14 → **11 / 12**; M3 25/25 / 25/25 → **25/25 / 25/25**; M4 main 2.6% / 2.1% → 1.7% / 2.0% (the drop is `finalize_plan`'s error rounds; without them the baseline was ~1.5%). No session called a removed tool; neither new refusal fired.

doc-parallel (before a / b → st a / b): M3 2/2 everywhere; **every session now delegates** (one `doc_create` with one section per question, one `spawn_agents` call, 7 subagents per run, all wrote their section), but **the main session still redoes their work**: in all 4 sessions it re-read 7–9 source files after the subagents finished, and in 3 it rewrote 2–3 delegated sections. M1 $0.0085 / $0.0219 → $0.0326 / $0.0248, M2 15.5 / 35 → 47 / 41. The runner's M6 reads 100% because F4 keeps a revised section's author; counted the old way (a section main rewrote is main's) it is 0/7, 2/7 → **2/7, 4/7**.

**Open:** the delegation now happens; the re-verification by the main session is the next cost to remove (§2.3's goal is not met on cost). F5 and F7 remain as noted above.
