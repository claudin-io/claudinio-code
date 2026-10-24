# Projects and prepared tasks: a task list per project, a runtime that lets tasks run unattended, and a preparation pipeline that pays its delegation tax in the open

**Status:** proposal. The maintainer's binding decisions of 2026-10-22 are inputs (§4.0), not open questions. Everything he left open, and every place this plan departs from his words, is in §7 with a recommendation; **D1, D10, D24 and the Phase 1a route were decided on 2026-10-24** (marked *decided* in §7). Nothing is implemented.
**Written against:** `main` at `47ba241` (0.9.0). Paths are relative to `src-tauri/src/` unless they start with `src/`, `eval/`, `scripts/`, `docs/` or `.github/`; `.rs` files under `eval/` belong to the Rust module `src-tauri/src/eval/`. Numbers marked *(computed)* were recomputed on 2026-10-22 from the raw eval output under `/srv/claudinio-eval/` at the eval's prices ($0.15 in, $0.60 out, $0.003 cache read per Mtok).
**Companion to:** [`2026-10-18_lean-harness-audit-and-plan.md`](2026-10-18_lean-harness-audit-and-plan.md), which made each request lighter (M1 −82%), and [`2026-10-20_single-timeline-and-one-plan.md`](2026-10-20_single-timeline-and-one-plan.md), which made the product one timeline with one plan. Both won largely by removing fresh-context hand-offs. The shape the maintainer now asks for has five of them. This plan builds it and makes each hand-off either move a meter or leave, except where his words fix it in place (verify in a new context, a new agent at ▶), which only he can change.

---

## 0. The one-paragraph version

Today the unit of work is an opened folder with one chat. The folder list lives only in localStorage (`src/App.tsx:33-60`), each folder has one conversation slot (`state.rs:57`) and keeps its sessions inside itself (`agent/persist.rs:647-657`). A run's events reach only the panel that started it, so every panel stays mounted (`src/lib/ipc.ts:489-504`, `src/App.tsx:1249-1256`). Approval waits ignore Stop (`agent/tool_exec.rs:344`), nothing bounds concurrent requests, and a subagent dies on its first 429 (`agent/subagent.rs:721`). The maintainer wants **projects** (a name, attached folders, an instruction, archive, an optional git worktree per task) that hold a **list of tasks**. A task starts from one prompt. When the person creates it, the system **prepares** it: a study split into small parallel questions, a plan with a low-level design, a verify in a new context that never saw the plan being written, a fix, and a report. A new agent then implements it when the person presses **▶**. Many tasks run at once (he named 1 to 20; the default cap is 4, adjustable to 20, D14), a **project chat** drives the list, a click on a task opens **its own chat**, and the right-hand panel goes away.

The order is in §5, and D24 decides it. First come meters, larger eval tasks and a small security fix handled under `SECURITY.md` (Phases 0–1a). Then the runtime: runs outlive their view, every model request passes one gate, and every wait is listed and stoppable (2–5). Then the product's shell: projects (6a–6b), a task list (7), ▶ with the task chat (8) and the project chat driving the list (9). At that point a task has no preparation yet: ▶ runs today's single timeline (Quick). **The recommended order (D24(b)) then builds the preparation he called "a parte principal" before anything else**: Light (13a–13c), ▶ on a prepared plan (14), Full with a parallel study (15), seeded reads (16), verify and fix (17). Only then do the right column go, the system prompt lose its absolute path, and projects span folders (10–12), followed by worktrees, auto-implement and the clean-up (18–21). Which depth a new task gets by default is his call (D1, and D25 for the interim); the plan recommends preparing by default, as he described, with its measured price on every card.

The delegation tax is met with mechanisms. Each one that can be switched off has a meter and an ablation flag:
- every stage and the implementer send one system prompt and tool list byte for byte, with permissions masked in code and the golden rules moved into the head (D23);
- per-task context rides a layered first user message, whose study and plan layers are frozen and byte-identical in every later stage, with absolute paths kept out of them;
- sibling sessions start only after the first one has answered (a hypothesis: §2.7 cannot say why one sibling was warm, so `--no-warm` decides);
- excerpts are copied from the file by the harness, never typed by the model;
- the report and the check are assembled by code;
- the implementer starts fresh, as he said; forking the planner's session is an eval arm only he can make the default (D9);
- a re-read meter and a pre-edit cost meter show whether the implementer redoes the study.

None of this makes a fresh context free. On today's small tasks preparation cannot pay for itself even in principle (§6): it is a feature with a price, not a saving.

## 1. What we measure, and the gates

The authority order of the lean-harness plan holds: **M1 decides**, and nothing ships if M3 drops (`docs/plans/2026-10-18_lean-harness-audit-and-plan.md:18-24, 108`). Every eval run uses deepseek-flash at the baseline prices.

| # | Meter | Definition | From |
|---|---|---|---|
| M1 | Cost per solved task, cache-aware | As today (`eval/metrics.rs:15-38, 91`), plus a split by stage through a new `stage` field on `SessionRecord::Usage` (`agent/persist.rs:382-400`). Triage's cost counts, Jev's included (§4.6), which today's Jev calls do not (`eval/README.md:12`) | split: P0 |
| M2 | Median rounds per task | As today, plus per stage | split: P0 |
| M3 | Oracle pass rate | As today | — |
| M4 | Tool-call error rate | As today. **Stage-mask refusals** (§4.5) are inside M4, broken out per stage: a refusal costs a round like any error | P0 |
| M6 | Sections written by subagents (`eval/metrics.rs:182-191`) | Retired in P21, when the document tools leave Standard | — |
| **M7** | **Re-read ratio** of a stage S that follows another (plan, verify, fix, implement) | Reads in S of files an earlier stage cited **that S does not edit**, over all of S's `read_file` calls. "Cited" means a study answer's `## Facts`, a plan's `## Changes` anchors and, in today's transcripts, an explore report's `## Files` (`agent/delegation.rs:21-26`). Reads of files S edits are excluded: read-before-edit (`agent/tools/mod.rs:1303-1334`) forces them, so counting them would put M7 near 1 by construction. They are reported apart as **M7e** (cited files S re-reads and edits), which seeded reads (P16) and the fork arm's tracker (P14) target | P0 |
| M8 | Stage cost share | Each stage's cost over the card's cost; the preparation share | P0 |
| M9 | Verify yield | Corrections raised and applied per card, uncited reads per verifier, and M3 of cards with and without corrections | P17 |
| **M10** | **Warm-start share** | For each harness fan-out of two or more sessions, siblings 2..N: cache reads over all input on their *first* request | P0 |
| M11 | Wall-clock per stage, per card, per group | `wall_s` exists (`eval/runner.rs:267`) | split: P0 |
| **M12** | **Pre-edit cost** | The implementer's cost on the requests strictly before the one that issues its first `edit_file`, over its whole cost, paired per task against the same task at Quick. This is the number a preparation can lower; E8 gives Quick's today | P0 |

M5 was never implemented, so the new meters skip that number.

**How results are compared** (all gates below):
- **Paired per task.** Each arm is compared with the baseline task by task, not by totals of different task mixes.
- **Noise.** M1 is "within noise" when the paired ratio of totals is ≤ 1.15 over two runs; the before-baseline measured ±15% (`eval/baselines/2026-10-19-deepseek-flash-after/README.md:48`), but mid-lean runs of one binary varied by 37% ($0.0246 against $0.0179, `:7`) and doc-parallel by 2.6× ($0.0085 against $0.0219, E3). A ratio between 1.15 and 1.3 earns a third run before it fails.
- **M3 holds** when no task that passed in both baseline runs fails in both arm runs. One flaky task cannot fail a phase.
- **A minimum sample before a number decides anything.** Auto's thresholds and the evidence offered for D9 and D26 need at least 12 large tasks × 3 runs. Below that, the numbers are published and the maintainer decides.

**The gates.**
- **G-Quick** (blocks the merge of every phase that changes what the model sees or how a task run is driven: 4, 8, 9, 11, 13b, 14–17, 21; the others are tests only). On the 25 small tasks, a task run at **Quick** keeps M3 and M1 within the rules above against the `47ba241` baseline. It protects the cost of Quick for whoever chooses Quick. It does **not** decide which depth a new task gets by default: D1 and D25 do, with the measured prices in front of the maintainer. A failure is fixed or the phase reverted, or the maintainer accepts that specific change with its numbers.
- **G-Prompt** (blocks the merge of any change to what the model sees: P9, P11, P14, P21).
  - M3 holds and M1 is within noise, on the 25 small tasks and the large subset.
  - The default Standard prefix stays ≤ 5,000 tokens and byte-stable (`agent/prompt.rs:1440`, `:813`).
  - Every new profile has its own byte-stable ≤ 5,000 test.
  - A failure is resolved by reverting that change or by a maintainer decision on that change.
- **G-Mechanism** (blocks the merge of each mechanism that can be switched off). The mechanism ships with an ablation flag and must move its own meter with M3 held. One that does not is removed, not kept in case.
  - Warm, then fan out: M10 ≥ 70% against `--no-warm` (P15). If `--no-warm` reaches the same M10, the cache warmed without T3 and T3 is removed.
  - The fourth breakpoint (T4): its M10 effect on an Anthropic-direct endpoint (D13). Until such a run exists it stays behind `--no-shared-head`, off.
  - Seeded reads: M7e goes down (P16).
- **Depth evidence** (reported, never a merge gate; P13–P17). Full against Light against Quick on the large subset: M1, M3, M7, M8, M12. Above the minimum sample, these set where *Auto* routes. They never remove a depth or a stage from the product: verify stays in every Full preparation whatever its M3 effect unless the maintainer decides otherwise (D26).
- **G-Group** (blocks the merge of P18). On the parallel track, every oracle passes after the merges, the group's wall-clock is ≤ 0.6× the serial run, and M1 is ≤ 1.2× serial.

**Where the warm-start meter is measured.** The eval reaches deepseek-flash through DeepSeek's Anthropic-protocol endpoint (`eval/README.md:271-274`). The app sends cache markers there (`agent/provider/cache.rs:154-174`), but DeepSeek's implicit prefix cache already serves ~95% of input, and "`cache_control` markers do not show here" (`eval/baselines/2026-10-19-deepseek-flash-after/README.md:44`). claudin.io gets no markers (`agent/provider/cache.rs:172`), and the app has never measured its upstreams' implicit cache. So M10 on either endpoint measures implicit caching only, which is what T3 targets. T4's breakpoint is measurable only on an Anthropic-direct endpoint, which makes it part of D13. The bar is 70% because the one warm sibling today read 3,456 of 3,986 tokens from cache, 87% *(computed)*, and 70% leaves room for the varying tail.

**Runtime invariants.** These are held by tests, not by the eval:
- **R1.** Every wait is listed with its owner: approval, question, askpass prompt, queue position, provider limit, rate-limit cooldown, budget pause.
- **R2.** Never more than N model requests are in flight, the completion judge's included.
- **R3.** Attaching to a run mid-way gives the same timeline as watching it from the start.
- **R4.** Archive and migration never move or delete anything inside a folder.
- **R5.** A folder without a worktree has at most one implementation writing to it, and an implementation writes only in folders it holds.

## 2. Evidence

### 2.1 The runtime assumes one folder, one chat and one visible panel
- **Folders.** The open and recent folders live in localStorage only (`src/App.tsx:33-60`). Every stored folder is re-indexed at startup (`:314-323`).
- **Runs reach one panel.** A run's events go only to the `Channel` that `sendMessage` creates per call (`src/lib/ipc.ts:489-504`; `commands/agent.rs:150-157`). `spawn_run_loop` sends to that channel and nowhere else (`commands/agent.rs:417`). Every `ChatPanel` therefore stays mounted "so streaming stays live" (`src/App.tsx:1249-1256`).
  - Each mounted panel runs two smooth-text loops while streaming, hidden or not (`src/components/ChatPanel.tsx:159, 180`).
  - Its popovers register document keydown and window resize listeners even while closed (`src/components/Popover.tsx:168, 229`).
  - The UI map counts about 160 keydown listeners and 40 timers at 20 mounted panels.
- **A dead view is invisible to its channel.** Tauri's `Channel::send` evaluates a script in the webview and returns `Ok` as long as the webview exists, whether or not the JS callback is still there (tauri 2.11.5, `ipc/channel.rs:134-180`). A failed send can therefore never tell the runtime that a view went away.
- **One slot per folder.** `send_message` continues `ws.active_session` or creates it (`commands/agent.rs:177-196`). `load_session` reads only `<folder>/.claudinio/sessions` and overwrites the slot (`:571-636`). A handoff swaps it unconditionally (`agent/transition.rs:151-159`). Only the UI stops a second run on the same session (`src/components/ChatPanel.tsx:1291-1300`).
- **Starting a run lives in the command layer.** The `ToolContext` assembly (`commands/agent.rs:246-265`), the hook context (`:359-387`), the transition maps (`:389-395`) and `spawn_run_loop` (`:417-498`) are private to `commands/agent.rs`, and core modules may not import `crate::commands` (`lib.rs:188-209`). A run is marked finished only after its loop returns, after `Done` has reached the UI (`commands/agent.rs:491-495`).
- **The panel holds the run state.** `ChatPanel.tsx` is 2,298 lines, and its event reducer is a closure at `:795`. No test mounts it: "too heavy to mount here" (`src/components/ChatPanel.test.ts:331-333`). Status is one value per folder (`src/lib/workspaceStatus.ts:13-21`), written by that folder's single panel (`ChatPanel.tsx:288`); with several runs per folder it would be last-writer-wins.
- **The window is narrow.** The minimum width is 900 px (`src-tauri/tauri.conf.json:18`). The left aside is 240 px (`src/App.tsx:954`) and the right one 256 px whenever the session has a checklist or a document (`:118-119, 1262`).

### 2.2 Nothing bounds concurrency, and failures are handled one request at a time
- **Fan-out is capped per call, not globally.** The default is 4 and the maximum 8 (`agent/subagent.rs:12-28`).
- **Subagents do not retry.** They call `provider::stream_message` (`agent/subagent.rs:721`), so a 429 ends them with "Provider error" (`:735-746`). The main loop retries 429 and 5xx (`agent/run_loop.rs:648`; `agent/retry.rs:28-35`) on fixed backoffs with no jitter (`agent/retry.rs:64-110`).
- **Model requests leave through five HTTP sends**, not one function. `stream_message` posts at `agent/provider.rs:1428` (or `agent/provider/openai.rs:398`); `one_shot`, used by the prompt enhancer (`commands/enhance.rs:138, 306`), posts at `agent/provider.rs:1256` (or through `openai::complete`); the completion judge, `classify_turn_completion_metered` (`agent/provider.rs:1136-1214`), runs on every terminal turn (`agent/judge.rs:56-61`), is billed in M1 (`eval/README.md:12`) and posts at `:1179-1181` or through `openai::complete_metered` (`agent/provider/openai.rs:708-760`, posting at `:747`); and `commands/local_llm.rs:712` calls `openai::complete` directly.
- **A plan budget is a hard stop.** claudin.io's "Budget exceeded for window" becomes `BUDGET_EXCEEDED_MARKER` (`agent/provider.rs:20-26, 1450-1451`), which is never retried (`agent/retry.rs:16-18`).
- **Local servers default to one slot** (`llama/mod.rs:254-256, 302-304`).
- **`agent/provider.rs` has 1,843 production lines**, 157 below the cap (`lib.rs:236-254`).

### 2.3 Waits nobody sees, and approvals that relax
- **Stop does not end a wait.** Five approval waits are a bare `approve_rx.await` (`agent/tool_exec.rs:344, 471, 600, 647, 769`), and so is `ask_user` (`:891-897`). `interrupt_session` only sets a flag (`commands/agent.rs:1151-1169`; `agent/session.rs:196-200`).
- **"Requires approval" is not a guarantee.** YOLO relaxes every `RequiresApproval` tool that is not in `config.yolo_blacklist` (`agent/tool_exec.rs:45`; the list is a setting, `src/App.tsx:150`). A PreToolUse hook's `Allow` relaxes them all (`agent/tool_exec.rs:74-79`). Only the bash and browser policy denials survive both (`:47-72`). A tool that only "requires approval", such as a chat tool that starts a task, would therefore run with no click. That breaks binding decision 1 unless it is exempted in code.
- **`auto_approve_git` also approves `git push`.** `AUTO_GIT` is `["git add", "git commit", "git push"]` (`agent/permissions.rs:203`), and `commit_and_push` sets it (`commands/agent.rs:1244`).
- **"Auto" bash is not read-only bash.** `bash_call_permission` returns Auto for the whole `BASH_ALLOWLIST` (`agent/permissions.rs:147-184`), which holds `cargo build`, `cargo test`, `npm run dev`, `pnpm dev`, `pnpm run`, `npm test`, `pytest`, `node ` and `python `: "the interpreters, which run whatever script they are given" (`:144-146`). The eval approves bash with nobody watching only under `bash_is_read_only` (`READ_ONLY_PREFIXES`, `:189-200`; `:525-528`) **and** `stays_in_workspace` (`eval/policy.rs:56-66, 82`), whose own comment assumes the first has already refused pipes (`:74-75`).
- **One Enter approves every card.** Each `ApprovalCard` adds its own document listener, guarded only by the panel being active (`src/components/chat/TimelineRows.tsx:1429-1440`).
- **Askpass prompts belong to no session.** A prompt carries no session (`askpass.rs:41-46`), and nothing could give it one: the bash child gets only `askpass::env_for_child()` (`agent/tools/bash.rs:143-148`), which names no session (`askpass.rs:61-81`); only hook processes get `CLAUDINIO_SESSION_ID` (`agent/hooks/runner.rs:177`). One global counter pauses every agent's bash timeout (`askpass.rs:30-32`; `agent/tools/bash.rs:196-204`). A second prompt replaces the first in the single UI signal (`src/App.tsx:271-279`). A prompt fails after 300 s (`askpass.rs:24-25`).
- **A checklist edit can vanish.** `set_tasks` with no active session returns `Ok(())` and drops the edit (`commands/tasks.rs:24-35`).

### 2.4 Storage lives inside the folder
- Sessions go to `<root>/.claudinio/sessions` (`agent/persist.rs:647-657`), documents to `.claudinio/docs` (`agent/doc_session.rs:44`), and compaction notes to `<workspace>/.claudinio/notes/` (`agent/notes.rs:1`). `remember` writes `<root>/.claudinio/MEMORY.md` (`agent/project_memory.rs:25`).
- Readers hardcode that directory: `doc_session::session_file` (`agent/doc_session.rs:794-807`), used by the document commands (`commands/document.rs:18`) and for the document carried over on a handoff (`agent/doc_session.rs:960-972`); `persist::chain_cumulative_cost` (`agent/persist.rs:1193-1230`), which seeds a successor's cost (`agent/transition.rs:121`); `persist::list_sessions` (`:1263`); and `load_session` (`commands/agent.rs:578`). `commit_and_push` creates its session there too (`commands/agent.rs:1201-1202`).
- A handoff successor is created in the workspace root, wherever its predecessor lives (`agent/transition.rs:77`). The eval reads `<ws>/.claudinio/sessions` (`eval/runner.rs:266`).
- A worktree removed after a merge would take its transcripts with it.
- **One finding of this audit is a security issue.** Under `SECURITY.md` it is fixed and released first (Phase 1a); its details are published here once that release is out.

### 2.5 One root, and the prefix names it
- **One root in the tools.** `ToolContext.workspace_root` is one string (`agent/tools/mod.rs:36`) and drives `validate_path` (`:143`). A relative bash `workdir` is resolved, but an absolute one is passed through unchecked (`:961-964`). The IPC guard already accepts any open root (`commands/fs.rs:188`).
- **The system prompt embeds the absolute root**: "Project workspace root: {root}" and "File tools take absolute paths inside this root" (`agent/prompt.rs:69-79`, the GitSync prompt at `:96-104`, and the subagent prompt at `agent/subagent.rs:544`). It is documented as "Byte-identical for every request in the same workspace" (`agent/prompt.rs:81-82`). A worktree per task is therefore a different prefix per task. The tool schemas say "Absolute path inside the workspace" (`agent/tools/mod.rs:316, 329, 352`) and "Absolute path to the file" (`:389, 400, 413`), with no path in them.
- **The system prompt also varies with golden goals.** `GOLDEN_PROMPT` (`agent/prompt/standard.rs:73-76`) is appended when the session has golden tasks at run start (`agent/prompt.rs:116-126`, the flag read once at `agent/run_loop.rs:249-251`), and below a 64k window a golden session stays on Standard while the others move to Compact (`agent/surface.rs:236-247`).
- **The Standard prompt sends every session to the plan panel.** "The user follows progress in the app's plan panel"; multi-step work "gets a task list (`tasks_set`)"; research "gets a living document (`doc_create`)"; "Subagents (`spawn_agents`) do broad or parallel reading" (`agent/prompt/standard.rs:36-46`). A read-only stage told this reaches for tools its mask refuses.
- **The prefix budget has 187 tokens left** on the harness's own meter, chars/3 (`agent/prompt.rs:1428`). Tool sizes were computed by serializing the definitions as a request does; the phase's prefix test is the authority.

| Item | Tokens |
|---|---|
| Default Standard prefix: system 7,033 chars + tools 7,409 chars, 14 tools, against a gate of 5,000 (`agent/prompt.rs:1440, 1526-1532`) | **4,813** |
| `tasks_set` definition (`agent/tools/mod.rs:592-615`) | ≈248 |
| `tasks_update` definition (`:616-640`) | ≈253 |
| `doc_create` definition (`agent/tools/doc.rs:15-28`) | ≈104 |
| The PLAN section's "Document:" line (`agent/prompt/standard.rs:41`) | ≈143 |

- **The core tool list is pinned exactly** (`agent/prompt.rs:1007-1047`), `doc_create` is pinned as core (`:1056-1078`), and so are the PLAN phrases (`:1316-1356`).

### 2.6 There is no worktree code, and the quality gate is folder-wide
- Nothing runs `git worktree`. `run_git` lives in the command layer (`commands/git.rs:25`), which core modules may not import (`lib.rs:188-209`).
- The quality evidence is a digest of the whole root (`quality/evidence.rs:25`), so two writers in one folder make each other's green run stale.
- Hook trust is keyed by path string (`agent/hooks/trust.rs:120-126`). A hook runs with `current_dir` and `CLAUDE_PROJECT_DIR` set to the root it was given (`agent/hooks/runner.rs:172-178`).
- CI runs on Ubuntu only (`.github/workflows/ci.yml:23, 45`), and clippy runs with `--all-targets -- -D warnings` (`:105`). Releases also ship Windows.

### 2.7 What the delegation tax cost, measured

| # | Fact | Source |
|---|---|---|
| E1 | Lean harness: M1 $0.032 → $0.0058 (−82%), M2 47.5 → 12, M3 47/50 → 50/50, subagents ~80 → 0 per run. **Confounded**: the README credits main-session edits, no task ritual, a ~5k prefix and file creation together | `eval/baselines/2026-10-19-deepseek-flash-after/README.md:17-31` |
| E2 | "Builder starts empty, reads the plan file and re-explores what Brain already read"; the median Brain session took 40 rounds against 6 in Builder | `docs/plans/2026-10-20_single-timeline-and-one-plan.md:38`; `docs/plans/2026-10-18_lean-harness-audit-and-plan.md:68` |
| E3 | doc-parallel after one timeline: every session delegates, but main "re-read 7–9 source files after the subagents finished" in 4 of 4 sessions, and rewrote 2–3 delegated sections in 3. M1 $0.0085 / $0.0219 → $0.0326 / $0.0248 | `docs/plans/2026-10-20_single-timeline-and-one-plan.md`, Measurements |
| E4 | Before that, the one delegating session cost $0.037 and 55 rounds, 3.6× the same task done without subagents ($0.010, 13 rounds) | `eval/baselines/2026-10-20-doc-parallel-before/README.md:49-57` |
| E5 | **13 of 14 parallel explore siblings started cold**, with 128 cached tokens out of ~4,000 input on the first request. The warm one read 3,456 of 3,986 (87%). It **started with its siblings** (all four `meta.created_at` within 16 ms in `doc-parallel-job-runner`); its first response returned ~1.0 s after the first sibling's. Why it was warm is unknown | *(computed)*, `/srv/claudinio-eval/work-st-{a,b}/doc-parallel-*/…/*.sub/*.jsonl` |
| E6 | One explore subagent: $0.0031–$0.0082 and 6–11 rounds. Output is 66–80% of each one's cost | *(computed)*, same transcripts |
| E7 | Over the single-timeline runs, output is **61–62% of all cost**, uncached input 29–30%, cache reads 8–9% | *(computed)*, `work-st-{a,b}` |
| E8 | Small tasks: median $0.0036 (range $0.0014–$0.042), median 11.5 rounds. **Strictly before the request that issues the first `edit_file`: 39% of rounds and 44% of cost** (medians). Up to and including that request: 48% and 59% | *(computed)*, 50 sessions in `work-st-{a,b}` |
| E9 | Model-driven fan-out is fragile: "3 of 4 `spawn_agents` calls failed whole" | `docs/plans/2026-10-20_single-timeline-and-one-plan.md:46` |
| E10 | Token cuts are not cost cuts: −38.4% tokens gave +6.8% billed cost | `docs/plans/2026-10-18_lean-harness-audit-and-plan.md:86` |
| E11 | "mask tools, don't remove them"; "parallel writers without shared context produce inconsistent work" | `docs/plans/2026-10-18_lean-harness-audit-and-plan.md:87, 93` |

What follows from these:
- **Preparation moves exploration; it does not remove it** (E8). The most an implementer can save is the cost it spends before its first edit, 44% of a small task (the request that issues the edit has to happen anyway).
- **A Full study is expensive on today's tasks.** Three or four explore-sized sessions cost $0.009–$0.033 (E6), which is 1.5–5.5× today's M1 per solved task ($0.0059 / $0.0063 on the 25-task subset).
- **Output is the expensive token** (E7). Whatever code can write instead of a model is a saving.
- **A warm start is not yet explained** (E5). Delaying siblings is a hypothesis to test, not a finding.

### 2.8 What exists to build on

| Piece | Where |
|---|---|
| A run headless, fed by `Channel::new(closure)`, with approvals answered by a policy | `eval/runner.rs:355-383`; `eval/policy.rs:56-66, 82` (`bash_is_read_only` and `stays_in_workspace`) |
| A detached session that leaves the folder's slot alone, with its own steering and profile | `commit_and_push`, `commands/agent.rs:1176-1294` |
| An agent the harness starts, not the model | `run_summary_agent`, `agent/subagent.rs:1232` |
| A scripted provider that records request bodies, for byte tests | `scripted_model::spawn_recording`, `agent/subagent.rs:1541, 1658` |
| Context sent once in the first user turn and persisted with it | `agent/run_loop.rs:188-229`; `agent/project_memory.rs:569-600` |
| Golden goals from `<goal>` tags, gated by `run_quality` evidence | `agent/session.rs:46`; `commands/agent.rs:299-321`; `agent/tools/tasks.rs:340, 507` |
| The checklist recital, so no round is spent fetching it | `agent/task_recital.rs:34` |
| A document format with owners per section, where the person's edits win, and a reader for it | `agent/document.rs:538-579`; `src/components/DocumentPanel.tsx` |
| The explore report contract (`## Findings` / `## Files` / `## Open questions`) and its 6,000-character cap | `agent/delegation.rs:21-26, 44` |
| A metered one-off call that returns its usage | `classify_turn_completion_metered`, `agent/provider.rs:1136`; `openai::complete_metered`, `agent/provider/openai.rs:721` |
| Atomic writes and plain-id checks | `agent/doc_session.rs:289-312, 790-807` |
| A native save dialog chosen in Rust | `commands/fs.rs:272-310` |
| One markdown path | `src/lib/markdown.ts`; `src/components/ProseContent.tsx` |

### 2.9 What the eval can and cannot show
- **It runs one task at a time** (`eval/mod.rs:262-282`).
- **Its tasks are small.** There are 27 (25, plus 2 doc-parallel). The after-baseline median is 12 rounds, and no task is long enough to compact.
- **It has one provider and one price set** (`eval/metrics.rs:15-38`), with no baseline on claudin.io or Anthropic-direct (`eval/README.md:267-274`).
- **It runs one root per task** (`eval/task.rs`, `eval/materialize.rs`), and its oracles see only the workspace.
- **So a fixed preparation cost dominates every current task.** A gain from preparation can only show on bigger tasks, which P1 adds.

## 3. Design rules

1. **The runtime owns runs, and views subscribe to them.** No run depends on a component staying mounted.
2. **Nothing waits unseen.** Every wait is in one inbox with its project, task and session, and Stop ends every wait.
3. **The person starts every write.** A write happens only in:
   - an implementation started by ▶, by an approved `task_play` card, or by auto-implement on a prepared task the person created or approved (P19);
   - a project-chat turn the person sent: one-step edits, lease-checked (D10);
   - a task-chat follow-up after ▶, which continues that implementation.
   Nothing relaxes the `task_play` card, or the `task_create` card while auto-implement is on: not YOLO, not a hook's `Allow`, not a repository file.
4. **One writer per folder, unless a worktree isolates it, and writes stay in the folders a run holds.** Preparation never writes and never asks: the mask *refuses*, and its bash is the eval's read-only rule, not the allowlist.
5. **Artefacts cross stage boundaries, transcripts do not.** VERIFY is always fresh. The implementer starts fresh, the literal "novo agente"; forking the planner's transcript is an eval arm that only the maintainer can make the default (D9).
6. **One prefix.** Every stage and the implementer send the same system prompt and tool list byte for byte: `run_prefix(Standard)`, with the golden rules of task runs carried in the head (D23). That holds within a task on any window, and across tasks on windows of 64k and more. Role, roots and artefacts ride a layered first user message, and permissions are masked in code (E11).
7. **The harness orchestrates and the model answers.**
   - Stages are started by code (E9).
   - Artefacts are parsed final messages.
   - Excerpts are copied from files.
   - The report and the check are assembled by code (E7).
8. **Depth scales with the request.** Quick is today's single timeline, whose cost is measured. Which depth is the default is the maintainer's (D1, D25).
9. **Keep what makes the harness trustworthy:** golden goals, `run_quality` and the quality gate, the completion judge, read-before-edit, the write gates, approvals.
10. **No limits of our own on what a model writes or how long it thinks** (D27). No stage or harness call sets `max_tokens` below the model's own limit; sizes in a stage's instruction are guidance, and their excess is reported, not refused.
11. **Every mechanism that can be switched off has a meter and an ablation flag** (T3, T4, T7, T8, the LLM verify). The structural ones (T1, T2, T5, T6, T9, the code-built check and report) are how the pipeline works; their ablation is the Quick arm.
12. **Old data loads, and nothing in a person's folder is moved or deleted.**
    - Removed record kinds stay deserializable.
    - New kinds join `src/lib/ipc.ts:544` in the same PR, pinned by `every_record_kind_is_in_the_frontend_union` (`agent/persist.rs:3275`).
13. **Red first, one PR per phase, and `main` shippable after each.**
    - A new module lands as a compiling stub, so its first test fails on an assertion.
    - When a pinning test goes red, the phase says whether the code was wrong or the behaviour changed on purpose.
    - The repository's guards hold: ≤ 2,000 production lines per `agent/` file (`lib.rs:236-254`), so new logic goes in new modules; no `crate::commands` in core; English only; markdown only through `renderMarkdown`; sensitive files flagged in the PR.
    - An eval flag lands in the phase that reads it, so no field is ever unread under `clippy -D warnings`.

## 4. The design

### 4.0 Binding decisions, and vocabulary

**Binding decisions** (the maintainer, 2026-10-22):
1. Execution is by an explicit ▶. Auto-implement is opt-in.
2. A task's study, plan and report live in the app (`~/.claudinio/projects/<project>/tasks/<task>/…`). A button exports the report as `.md` to a folder the person picks.
3. With worktrees on, an implementation ends as a **commit on `task/<slug>`** in its worktree, and stops there. The card offers merge, open PR and discard. Nothing merges automatically.
4. Without worktrees, implementations are **queued one per folder**. Preparation, which is read-only, always runs in parallel. With worktrees, implementations run in parallel too.
5. No hotfix for the right-panel bug.

**Vocabulary.**
- **Project.** A name, folders, an instruction, an archive flag, and switches for worktrees, auto-implement and the task permission policy.
- **Folder.** An attached directory. It keeps today's per-folder resources as a `WorkspaceState` (`state.rs:45-88`). A folder may belong to several projects (D29).
- **Task.** One card. It has a prompt, its preparation, its implementation and its chat.
- **Depth.** How much preparation a task gets: Quick (none), Light, Full or Auto.
- **Stage.** One of triage, scope, study, plan, check, verify, fix, report, implement.
- **Steps.** The agent's own checklist, today's `tasks_set` / `tasks_update` / `TaskItem` (D5).
- **Run.** One live execution of one session.
- **Lease.** The right to write in a folder, held by one implementation at a time when the folder has no worktree.
- **Inbox.** Everything currently waiting on the person.

### 4.1 Data model (`agent/project/`, a core module)

```rust
pub struct Project {
    pub id: String,                  // "p-" + short uuid; also the directory name ([A-Za-z0-9_-])
    pub name: String,
    pub folders: Vec<Folder>,        // >= 1; first = primary (bash cwd, relative paths)
    pub instruction: String,         // rides the first user turn (D20), never the system prompt
    pub archived: bool,              // hides the project; stops nothing; folders untouched (R4)
    pub worktrees: bool,             // a worktree per task per git folder it holds (P18)
    pub worktree_setup: Option<String>, // e.g. "pnpm install"; approved once, run by the harness (P18, D12)
    pub auto_implement: bool,        // binding 1: opt-in (P19)
    pub policy: TaskPolicy,          // Ask | WorktreeAutonomy (D8)
    pub default_depth: Depth,        // D1, D25
    pub legacy_sessions: Vec<PathBuf>, // <folder>/.claudinio/sessions, read-only (D4)
    pub created_at: u64, pub updated_at: u64, pub schema: u32,
}
pub struct Folder { pub path: PathBuf, pub name: String /* unique in the project */ }

pub enum Depth { Auto, Quick, Light, Full }
pub enum Stage { Triage, Scope, Study, Plan, Check, Verify, Fix, Report, Implement }
pub enum TaskState {
    Draft, Preparing { stage: Stage }, Ready, Queued { folders: Vec<String> }, Implementing,
    Review { commits: Vec<WorktreeCommit> },           // worktrees on: commit on task/<slug>
    Done { how: DoneHow },                             // Merged | PrOpened{url} | Uncommitted | Marked
    Discarded, Cancelled { reason: String },
    Stopped { stage: Stage, cause: StopCause },        // Person | Restart | Budget (4.4)
    Failed { stage: Stage, reason: String },           // e.g. "commit refused: <hook output>"
}
pub struct Task {
    pub id: String,                  // "t-0007", per project, never reused; "#7" in the UI and the chat
    pub title: String, pub prompt: String, // the person's words, verbatim
    pub goals: Vec<String>,          // <goal> tags (session::parse_goals, agent/session.rs:46)
    pub author: Author,              // Person | ProjectChat { session_id, approved: bool }
    pub depth: Depth, pub state: TaskState,
    pub holds: Vec<String>,          // folder names this task may write (its lease set, 4.3.6)
    pub sessions: TaskSessions,      // ids per stage, the task chat, the implementation chain
    pub worktrees: Vec<WorktreeRef>, // { folder, path, branch, base_branch, base_commit }
    pub questions: Vec<StageQuestion>, // ask_user calls recorded during preparation
    pub archived: bool, pub created_at: u64, pub updated_at: u64,
}
```

- **Order is priority.** It is one sequence, the list in `tasks/order.json`, whatever each task's state (D32 decides whether a project may hold several lists). A move rewrites that one file atomically (temp file plus rename, as `agent/doc_session.rs:289-312` does). Writes to `order.json` and the allocation of task ids go through one project-level actor, so a drag and a chat `task_create` never lose each other's update or share an id.
- **Cost is never stored.** It is derived from the stage-tagged `Usage` records of the task's sessions, so the card and the eval read the same numbers.
- **New record kinds:**
  - `Stage { task, stage, question?, status }`, the second line of every stage session;
  - `ForkedFrom { session_id, stage }`, for a forked implementer (the D9 arm);
  - `Verdict { ok, corrections }`.
- **One changed record:** `Usage` gains `stage: Option<String>`, with a serde default.
- **Every new field has `#[serde(default)]`.**

### 4.2 Storage on disk

```
~/.claudinio/projects/                     root injectable in tests (ProjectStore::with_root)
  index.json                               cache {id, name, archived, updated_at}, plus migrated_at; rebuilt if missing
  p-3f9c…/
    project.json                           atomic write
    chat/<sid>.jsonl, <sid>.sub/           project chat sessions
    tasks/order.json                       one writer: the project actor
    tasks/t-0007/
      task.json                            one writer: the task's actor (per-task lock)
      events.jsonl                         append-only transitions {from, to, cause, by, ts}
      study.md   study.lock                frozen study (document.rs format) + {base commits, blob digest per cited file}
      plan.md    plan.lock   plan.r1.md …  plan (document.rs format); its anchors' digests; old plans on re-plan
      addendum.md                          append-only: stale facts, the person's answers and decisions
      verify.json   report.md              verdicts; the report assembled by code
      sessions/<sid>.jsonl, <sid>.sub/     triage, every stage, the task chat, the implementation chain
<data_dir>/claudinio-code/worktrees/p-3f9c…/t-0007/<folder-name>/   git worktrees (D11); a short path on Windows (P18)
```

- **The app stores no sessions, task artefacts or notes in a folder.** Two kinds of write stay there by design. Git's own metadata: `git worktree add -b task/<slug>` writes `.git/worktrees/<name>` and `refs/heads/task/<slug>` into the folder's repository, and merge, discard and prune write there too (§4.9). And the person's project files: `remember` keeps writing `<folder>/.claudinio/MEMORY.md` (`agent/project_memory.rs:25`), and a Standard chat's `doc_create` keeps writing `.claudinio/docs/` until P21. The app still reads a folder's `.claudinio.json`, `MEMORY.md`, its legacy sessions and its docs. R4 covers archive and migration, which touch none of these.
- **Session files move to an injected directory.** `SessionStore::create_in(dir, id, workspace)` goes beside `create` (`agent/persist.rs:705-718`), and the compaction note follows the session's directory. Every reader that hardcodes `<root>/.claudinio/sessions` (§2.4) takes the session's directory instead: `doc_session::session_file` and the handoff's document carry-over, `chain_cumulative_cost`, `list_sessions`, `load_session`, and `commit_and_push`'s session. The eval points `create_in` at `<ws>/.claudinio/sessions`, so `metrics::collect` prices a pipeline run unchanged.
- **The webview never gets a writable path into this tree.** It reads artefacts through `get_task_artifact(project, task, kind)`, and Export uses the Rust save dialog (`commands/fs.rs:272-310`).

### 4.3 The runtime

**4.3.1 The run hub** (`agent/runs.rs`)
- **It owns every run, and it starts them.** Run start moves out of the command layer into `agent/runs.rs`: the `ToolContext` assembly (`commands/agent.rs:246-265`), the hook context (`:359-387`), the transition maps (`:389-395`) and the spawn loop that follows handoffs (`:417-498`). `send_message` and `commit_and_push` become thin adapters, and the core scheduler can start a queued run with no IPC call in flight. `start(sid, owner)` returns a `Channel::new(closure)` that copies each event to every subscriber. This is the eval's pattern (`eval/runner.rs:357-367`), so `run_workflow` keeps its signature (`agent/run_loop.rs:87-99`).
- **Its calls:**
  - `attach(sid, view_key, channel) -> sub_id` sends the tail first, then live events. It replaces any subscription that has the same view key, so a re-opened view never doubles events.
  - `detach(sub_id)`, which every view calls on cleanup. A failed send cannot detect a dead view (§2.1), so detaching is explicit.
  - `rename(old, new)` follows a handoff.
  - `list()` returns the running sessions.
  - `start` refuses a session that is already live. This server-side guard replaces the UI-only one. The hub clears a run's live flag **before** it emits `Done`, so a follow-up sent the moment `Done` arrives is accepted.
- **Status is computed once, in Rust:**
  - Needs you: a `ToolCall` that requires approval, an `AskUser`, or an askpass prompt.
  - Waiting: a rate-limit retry, a provider-limit permit, or a budget pause.
  - Idle: `Done`.
  - Failed: `Error`.
  Cards, the sidebar and keep-awake all read that value.
- **The tail and replay (R3).** The tail holds the current run's events, with consecutive deltas merged, up to 8 MB. Past that it is marked incomplete, and a view replays from the session file, which is complete for every finished round.

**4.3.2 The run store** (`src/lib/runReducer.ts`, `src/lib/runStore.ts`)
- **The reducer moves out of the component.** `ChatPanel.tsx:795` becomes a pure `reduce(state, event)`, with one store per session and one app-level `watchRuns` channel for statuses.
- **Only the visible chat is mounted.** Smooth text, scroll and the draft stay in the view, so a run nobody is looking at starts no timers and registers no document listeners.

**4.3.3 The provider gate** (`agent/provider_gate.rs`)
- **What it limits.** A semaphore of `max_concurrent_requests` permits (default 6, range 1–32; for a local provider, the server's slots) is held around **every HTTP send to a model**: `stream_message` (`agent/provider.rs:1428`, `agent/provider/openai.rs:398`), `one_shot` (`agent/provider.rs:1256`), the completion judge (`:1179`) and `openai::complete`/`complete_metered` (`agent/provider/openai.rs:747`, also reached from `commands/local_llm.rs:712`). They all call one `gated_send`. It is never held during tool execution. The logic lives in `provider_gate.rs` and `provider/http_error.rs`; `provider.rs` grows by at most 20 lines (157 lines of headroom, §2.2).
- **Classes, and nobody starves.** Each request carries its session's class: interactive (a chat the person is in), implementation, or preparation; the judge counts with its session. `min(2, limit − 1)` permits are reserved for interactive requests, none when the limit is 1. The rest are served round-robin across classes with aging, so preparation keeps moving while the maximum number of implementations stream (binding 4) and a limit of 1 still serves background runs.
- **Waiting is visible and stoppable.** A request waiting for a permit is a "Waiting: provider limit" item in the inbox, and the wait selects on the session's cancel `Notify` (§4.3.4), so Stop returns at once.
- **A 429 slows everyone.** It sets a shared cooldown: `Retry-After` when the provider sends one, otherwise the next backoff step, capped at 60 s. Backoffs get ±25% jitter.
- **A plan budget pauses everything, and fails nothing.** On `BUDGET_EXCEEDED_MARKER` (§2.2), the gate stops dispatching for every project. Live task runs move to `Stopped{stage, Budget}`, not Failed, and the cards and the inbox say "Paused: plan budget", with the reset time when the message names one. Resuming is a click, as after a restart (D22).
- **Subagents retry** through `stream_message_with_retry` (`agent/retry.rs:56`).

**4.3.4 The inbox** (`agent/inbox.rs`)
- **What it holds.** `PendingItem { id, session_id, owner, kind: Approval | Question | Askpass | Queued | ProviderLimit | BudgetPause, since }`.
- **Stop ends every wait.** `wait(item, &SteeringCtl)` selects on a `Notify` that is added next to the interrupt flag (`agent/session.rs:196-200`), so Stop resolves a wait as cancelled.
- **No timeout of the inbox's own.** A person may answer an approval tomorrow; the wait is made visible instead. Askpass keeps its 300 s (`askpass.rs:24-25`) unless D28 says otherwise: a credential prompt holds a live git or ssh child. The card then reads "Credential prompt expired: retry".
- **Askpass prompts belong to a session.** P5 makes `env_for_child` take the session id and add `CLAUDINIO_SESSION_ID` (`askpass.rs:61-81`; the bash child at `agent/tools/bash.rs:143-148`), the askpass script forwards it, and `AskpassRequest` (`askpass.rs:41-46`) carries it. Prompts then queue in the inbox and pause only their own session's bash timeout.

**4.3.5 Cancellation.**
- **`cancel_run(sid)`:**
  - raises the interrupt that the run's subagents share (`agent/subagent.rs:681`);
  - resolves the run's pending items as cancelled, a permit wait included;
  - drops its queue entries.
- **`cancel_task(reason)`** sets the task's cancellation token and cancels every live session of the task. The orchestrator checks the token before it starts any stage or any sibling, so a Stop between stages (after the study returns, during freeze, check or report, or while siblings wait for the first one) starts nothing more.
- **Archive stops nothing.** Running and queued work continues and stays visible in the inbox and on the project's row under "Archived". Auto-implement does not fire in an archived project. Unarchive restores the project unchanged.
- **Removing a folder from a project** is refused while a task holds that folder or has a worktree in it. Otherwise tasks that cite it keep their artefacts, and their ▶ says "folder X left the project: re-plan".

**4.3.6 Scheduler and leases** (`agent/project/scheduler.rs`)
- **Implementations.** `max_running_tasks` (default 4, range 1–20, D14) caps them. Each holds a writer **lease** on every folder in `task.holds`:
  - a prepared task holds the folders named in its plan's Changes;
  - a Quick task holds the folders its prompt @-mentions, or the primary folder when it names none;
  - the person can add a folder to a task from its menu.
- **Leases are app-wide.** They are keyed by canonical folder path, not by project, so a folder in two projects still has one writer (D29). Nested or overlapping paths count as one lease.
- **All of a task's leases are taken at once.** At ▶ the scheduler takes every lease in `task.holds` in canonical-path order, or none. A task that cannot take them all becomes `Queued { folders }`, first in first out in list order (binding 4). Nothing holds one lease while it waits for another, so two tasks that need the same two folders cannot deadlock.
- **Writes stay inside what a run holds.** The implementer's `ToolContext` refuses `edit_file`, `write_file`, `batch_edit` and a bash `workdir` outside the folders it holds or its worktrees, with the next step: "folder X is not in this task: Re-plan, or add it to the task". Adding a folder stops the run and queues it again with the larger set. A bash command's arguments are not parsed for writes; the workdir is checked, and in the Ask policy the person sees every non-allowlisted command.
- **With worktrees** the lease is per worktree, so implementations run in parallel.
- **Preparation** takes no lease and is bounded only by the gate.
- **A chat edit in a leased folder is refused.** The refusal names the next step: "folder X is being changed by #7: make this a task, or wait". The text lives in `gate_text`.
- **Merges into a folder are serialised** under that folder's lease (§4.9).

### 4.4 The task lifecycle

```
 create --> Draft --prepare (depth Light/Full/Auto)--> Preparing{[triage] -> scope -> [study] -> plan -> check -> [verify -> fix -> check] -> report}
              |                                          | done               | error               | stop / app quit / budget
              | > at Quick: a click, or an               v                    v                     v
              |   approved task_play card              Ready <-- re-plan -- Failed{stage}       Stopped{stage, cause}
              |                                          |  > (a click), approved task_play,            | resume (a click)
              |                                          |  or auto-implement (prepared tasks only)     |
              +------------------------------------------+--> all leases free? --no--> Queued{folders}  |
                                                         | yes                           | leases taken  |
                                                         v                               v               |
                                                     Implementing <---------------------------------------+
                                                       |              |                     | commit or setup refused
                                          worktrees on |              | worktrees off       v
                                                       v              v                   Failed{Implement}
                                      Review (commit on task/<slug>)  Done{Uncommitted}
                                        | merge      | open PR    | discard
                                        v            v            v
                                   Done{Merged}  Done{PrOpened}  Discarded
   cancel(reason): any non-terminal --> Cancelled (a worktree is kept; "Discard worktree" removes it)
   archive: a flag on the project; it stops nothing
```

| Event | From | To | Guard and effect |
|---|---|---|---|
| `create` | (none) | Draft | The prompt is not empty. The title is its first line until scope sets one. A new task starts preparing at once unless its depth is Quick |
| `prepare` | Draft, Ready, Failed, Stopped | Preparing{triage or scope} | No live implementation. A re-plan keeps the old plan as `plan.rN.md` and the study as it is |
| `stage_done` | Preparing{s} | Preparing{next}, or Ready | The depth picks the next stage (§4.6). An artefact that fails to parse is asked for once more, then Failed{s} |
| `edit_prompt` | Ready, or Failed or Stopped during preparation | Draft | The preparation no longer matches the prompt; Re-prepare is offered |
| `play` | Ready, or Draft at Quick | Implementing, or Queued | Refused while preparing, naming the stage. Leases and `max_running_tasks` apply |
| `implement_done` | Implementing | Review, Done{Uncommitted}, or Failed{Implement} | With worktrees the harness commits (§4.9). A refused commit is Failed{Implement, "commit refused: <hook output>"}, with Retry |
| `replay` | Review, Done, Failed | Implementing | A new run on the same implementation chain, in the same worktree |
| `stop` | Preparing, Queued, Implementing | Stopped{stage, Person or Budget} | Pending items resolve as cancelled |
| `discard_worktree` | Review, Cancelled, Failed, Stopped (with a worktree) | Discarded, or unchanged | After a confirmation: the worktree and its branch go (§4.9) |

**After an app restart, no money is spent without a click (D22).**

| State at quit | After restart | Resume |
|---|---|---|
| Preparing{s} | Stopped{s, Restart} | "Resume" re-runs stage s. Finished artefacts are kept |
| Queued | Stopped{Implement, Restart} | "Resume" queues it again. Nothing starts by itself |
| Implementing | Stopped{Implement, Restart} | "Resume" continues the implementation chain |
| A "needs you" item | resolved as cancelled | — |
| Review, Ready, Done, Draft | unchanged (nothing was running) | — |

Three rules hold across restarts:
- **Auto-implement fires only on a live transition from a preparation into Ready.** Never at Quick (there is nothing to have finished), never on a task restored as Ready, and only for a task the person created, or whose chat card the person approved (§4.11).
- **"Needs you" is a badge computed from the inbox, not a state.** That keeps the machine small.
- **A budget pause is a Stop with a cause.** Resume is a click.

`transition(&Task, Event) -> Result<Task, Refusal>` is pure and tested over the whole table.

### 4.5 One prefix, a layered head

**The prefix.** Every stage session (triage aside: scope, study, plan, verify, fix, the task chat before ▶) and the implementer is a main-loop session: `run_workflow_with_profile` with `PromptProfile::Standard`. That gives each of them retry, `Usage` records, persistence and the hub for free. Each one sends the same `run_prefix(Standard)` (`agent/prompt.rs:137`) byte for byte, under two rules:
- **The golden rules ride the head in task runs** (D23(b), recommended). `system_prompt` appends `GOLDEN_PROMPT` when the session has golden goals (§2.5), and a prepared implementer always has some (its acceptance criteria, T9), so the stages and the implementer would otherwise differ from the system block on. Every task session, a Quick implementer's included (from P14), therefore builds the system prompt without it, and the implementer's first user turn carries the same text (in L6 when it has a head). The golden gates themselves are unchanged. The profile choice uses the task's own goal flag for every session of the task, so a task's sessions share a profile on any window. The Standard chat is untouched.
- **The fork arm copies the planner's `tools_loaded` records** (`deferred::loaded_names`), so its tool list is the planner's (D9 arm).

A stage's permissions are a **mask in code**, `ToolContext.stage: Option<StageMask>`, checked in `tool_exec` before `effective_permission`:
- **Allowed:** reads and searches.
- **Bash** only if `bash_is_read_only(command)`, with no `stdin`, **and** the command stays inside one of the roots: the eval's rule (`eval/policy.rs:56-66`), with `stays_in_workspace` (`:82`) moved into `workspace_path.rs` and generalised to several roots. `bash_call_permission` is not used, because its Auto covers builds, test runners, dev servers and interpreters (§2.3). So a stage cannot run `cargo build`, `npm run dev`, `node x.js` or `python -c …`, compile into `target/`, start a server or fight an implementer over a lock in the same folder.
- **Refused:** every other tool, with a text that names the next step. `spawn_agents` is refused because the harness fans out.
- **`ask_user`** is recorded as a question for the person and answered "decide with your best judgement and list it under Assumptions".

A stage also runs with no hooks, with project memory off (the harness puts it into L1), and with no completion judge, because the harness parses the final message.

**The Standard prompt points elsewhere** (§2.5), so every stage's L5 opens with a fixed override, byte-pinned: "In this stage your final message is the answer. The plan panel, the checklist, the living document and subagents are not available here." Refusals are reported per stage in M4. **If they exceed 2% of stage tool calls in P13b's eval**, the fallback is named now: first, masking by deferral (the refused tools stay in the catalog, unloaded, so the model is not offered them); second, a stage profile with its own byte-stable ≤ 5,000 test, which is a deliberate exception to rule 6 and costs the shared cache between stages and the implementer.

**The head.** The harness builds the first user message of every task session as text blocks, each stable at its own level:

| Layer | Content | Stable per | Read from cache by |
|---|---|---|---|
| L1 project | `<project-instruction>`, then the `AGENTS.md` / `CLAUDE.md` and memory blocks (`agent/project_memory.rs:569-600`) | project | every session of the project |
| L2 task | `<task id>` with the prompt verbatim and its goals | task | every session of the task |
| L3 study | `<study hash>`: the frozen study, sorted by question id (Full only) | task, after freeze | plan, verify, fix, implement, task chat |
| L4 plan | `<plan hash>` | task, after check (or fix) | verify, fix, implement, task chat |
| W workspace | after P11: `<workspace>`, folder name → absolute path, primary marked. **The only layer with absolute paths**, so it sits after the frozen layers: a worktree implementer, whose paths differ, still shares L1–L4 | project's folder layout | every stage; not a worktree implementer |
| L5 stage | `<stage name>`: the override, then a constant instruction of about 300 tokens, byte-pinned | stage kind | all siblings of a fan-out |
| L6 item | the question, the check group, the corrections, or, for the implementer, the golden rules (D23) and "Implement the plan of #7." | session | — |
| append | `<addendum>`: stale facts, the person's decisions | — | written after the frozen layers, never between them |

**Paths in artefacts are folder-relative.** Study facts and plan anchors read `api/src/x.rs:3-9` in a multi-folder project and `src/x.rs:3-9` in a one-folder one, and the harness resolves them against W for each run. The frozen bytes therefore never name a machine path.

**The fourth cache breakpoint (T4).** On Anthropic-protocol providers, `apply_cache_breakpoints` (`agent/provider/cache.rs:50`) uses three of its four breakpoints today. A `shared_head` hint would put the fourth at the end of the deepest layer a session's siblings share: L5 for a fan-out, and L4 for the implementer, which holds only with D23(b). It is measurable only on an Anthropic-direct endpoint (§1), so it ships behind `--no-shared-head`, off, until D13 funds that run. On implicit-cache providers, identical leading bytes are enough.

**The root line until P11.** Until then, the system prompt carries the folder's absolute root and the W layer is empty. The root is shared by every stage of every task in that folder, so nothing above depends on P11 except a worktree implementer (P18).

### 4.6 The preparation pipeline, stage by stage

No stage and no harness call sets a `max_tokens` of its own: requests carry the model's own output limit (`max_output_for`, `agent/provider.rs:418`), as the maintainer's standing instruction asks (D27). Sizes below are guidance written into L5; their excess is reported on the card and in M1 per stage, never refused or resent. A stage stops when it answers, when the person stops it, or when its window fills.

| Stage | Depth | Input | Output (a parsed final message) | Sessions | Stops when |
|---|---|---|---|---|---|
| **Triage** | Auto | the prompt | Quick, Light or Full | no stage session: Jev `decide` (`agent/jev.rs:248`) when a backend is configured (`backend()`, `:88`), otherwise one **metered** provider call shaped like the completion judge (`agent/provider.rs:1136`), never `one_shot`, which returns no usage (`:1219-1225`). Either way a `Usage` record with stage `"triage"` (Jev's reported cost, `agent/jev.rs:240-245`) goes into a `triage` session file in the task's sessions, so M1 and the card count it | one answer; a failure falls back to the project's default depth |
| **Scope** | Light, Full | L1, L2, W, L5 | `## Size`, `## Facts` (`path:start-end — claim`). At Full: `## Questions` (guidance 2–6, each with where to look). At Light: the same session goes on to write the plan, with **no reset** | 1 | the output parses |
| **Study** | Full | L1, L2, the scope's facts, W, L5, one question | `## Answer`, `## Facts` (`path:start-end — claim`, folder-relative), `## Open questions`: the `delegation.rs:21-26` contract, with `## Files` replaced by `## Facts`, which carry ranges and claims. Guidance: about 1,500 tokens, not cut | 1 per question, ≤ `max_parallel_agents` at a time; **warm, then fan out** (T3, a hypothesis) | the output parses; a full context gives a partial answer, flagged on the card |
| **Freeze** | Full | the answers | `study.md`, rendered by question id whatever the finish order: each question is one `## <question>`, with its answer, facts and open questions at `###`. **The harness copies each cited range from the file.** A range that does not resolve goes to an "Unverified" list. `study.lock` records base commits and the blob digest of every cited file | code | — |
| **Plan** | Full (the scope writes it at Light) | L1–L3, W, L5 | `## Goal & acceptance` (each criterion a command or a named test), `## Design`, `## Changes` (per file: anchors `path:start-end`, whose excerpts the harness copies, and the change; or `(new)`), `## Steps`, `## Assumptions & risks`. Guidance: about 3,000 tokens and 12 steps; a longer plan is kept and flagged | 1, **one planner** (D18) | parses (one more ask on a parse failure) |
| **Check** | Light, Full | `plan.md` | every anchor resolves and its excerpt still matches; every path is inside a root; every edited file exists or is `(new)`; every `<goal>` maps to a step. A failure is sent back to the planner, at most twice | code | deterministic |
| **Verify** | Full | a **new session**: L1–L4, W, L5, a check group (V1–V6). **Never the planner's transcript** | `VERDICT: OK`, or `VERDICT: CORRECTIONS` followed by `- [sN] problem — path:start-end` (evidence required) | 1; 2 in parallel by check group when the plan touches ≥ 8 files (D18); warm first | parses (one more ask). Reads outside the study's citations are allowed and **counted** (M9), not refused: an independent check must be able to look where the study did not |
| **Fix** | Full, with corrections | L1–L4, W, L5, one correction | the replacement body of its step; new facts go to the addendum | **1 per correction**, as the maintainer described (D18); corrections on the same step run one after another, each owning that step while it runs | parses, then **Check again**. One verify → fix cycle (D18). Corrections still open go to "Open issues", and the task is Ready with a warning |
| **Report** | Light, Full | plan, verdict, questions, derived cost | `report.md`: what will be done, how, where (a table of files and anchors), acceptance, verification, questions for you, preparation cost and time | **code, no model** | — |

**The verification checks**, fixed in v1 (D18):
- **V1.** Every requirement in the prompt maps to a step.
- **V2.** Up to 10 anchors are spot-read.
- **V3.** Callers of every public symbol the plan changes are covered.
- **V4.** Every acceptance criterion is checkable by `run_quality` or by a named test.
- **V5.** Nothing contradicts the instruction or `MEMORY.md`.
- **V6.** Each step leaves the build passing where possible.

**Questions.** A stage's `ask_user` call becomes an entry under "Questions for you", together with the assumption the stage made. Answering it in the task chat appends the answer to the addendum and offers Re-plan. Re-plan re-runs plan → check (plus verify and fix at Full). The study is not redone.

**Small windows and local models.** On the Compact surface (`agent/surface.rs:152`) and on local engines with one slot, depth is capped at Light and nothing fans out.

### 4.7 The delegation tax: each mechanism, the evidence it answers, its meter and its ablation

| # | Mechanism | Answers | Enforced in | Meter | Ablation |
|---|---|---|---|---|---|
| T1 | The harness drives the stages; the model never calls `spawn_agents` to prepare | E9 | `agent/prepare/` | M4 per stage | structural |
| T2 | One prefix: every stage and the implementer send the same system prompt and tools byte for byte, the mask in code, the golden rules in the head (D23) | subagents run their own prompt (`agent/subagent.rs:534-556`); §2.5 | `ToolContext.stage`; byte tests in P13a and P14 | M10 | structural |
| T3 | Warm, then fan out: siblings start after the first sibling's first response has returned usage | E5 shows one warm sibling of ten, but all four started within 16 ms, so its warmth is unexplained. **An untested hypothesis**, decided only by the ablation | `agent/prepare/fanout.rs` | M10 ≥ 70% | `--no-warm` |
| T4 | The fourth breakpoint at the deepest shared layer | E5, E8 | `agent/provider/cache.rs` | M10 on an Anthropic-direct endpoint only (D13) | `--no-shared-head` (off until measured) |
| T5 | A frozen study and plan, byte-identical in plan, verify, fix, implement and the task chat; folder-relative paths; additions only in the addendum | E2, E3 (main rewrote delegated sections) | freeze, plus a byte test | M7, M12 | structural |
| T6 | Excerpts are copied from the file by the harness: zero output tokens, verbatim by construction | E7 (output is 61–62% of cost) | freeze, plan parse | M1 per stage | structural |
| T7 | The implementer starts fresh; forking the planner's session is an arm (D9 is the maintainer's) | E2 (the context was thrown away) | `agent/prepare/kickoff.rs` | M1, M12, M7, M3 | `--implement fresh\|fork` |
| T8 | Seeded reads: while a cited file's digest still matches the lock, its verified excerpt counts as read for `check_can_edit` (`agent/tools/mod.rs:1303-1334`, whose own rule is "may edit any text it has actually read verbatim", `:1317`). **A write-gate change, flagged** | E8 (44% of cost comes strictly before the first edit) | the `ReadTracker` (`:1269`) | M7e | `--seeded-reads off` |
| T9 | Steps seeded from the plan (the harness writes the `Tasks` record first, so no `tasks_set` round), and acceptance criteria as golden goals | ceremony rounds (E2) | kickoff | M2 of the implementer | structural |
| T10 | The report and the check are code; verify emits a verdict, not prose; fixers rewrite only their own step | E7 | `prepare/report.rs`, `check.rs` | M8, M9 | `--no-llm-verify` (an eval arm; the default keeps verify, D26) |
| T11 | Size guidance, not caps: L5 states the expected size of each artefact; nothing is cut, refused or resent for length (D27) | E7 | L5 text | M1 per stage, output tokens per artefact | — (nothing to switch off) |
| T12 | Depth scales: Quick, Light (scope and plan in one context, no reset), Full | §2.7: a Full study is 1.5–5.5× today's M1 | triage, plus the person's choice | M8, M12, G-Quick | `--depth` |

**What none of this removes:**
- **VERIFY re-reads by construction**, because the vision wants it fresh. Its M7 is reported, not gated.
- **Plans and fixes are output tokens.**
- **Every new session pays for its uncached tail.**
- **Each run starts its own read-before-edit tracker** (`commands/agent.rs:266`; a successor's at `agent/transition.rs:229`), filled only by its own reads (`agent/tools/mod.rs:736, 1055`). Carrying a transcript forward does not carry the tracker: only P14's rebuild (fork arm) and T8 (seeded reads) touch M7e.

**Warm starts save little at these prices.** A cold sibling's first request is about 4,000 input tokens, of which 3,832–3,925 were uncached; the warm one still had 530 uncached. The difference, about 3,320 tokens × $0.147/M, is **≈$0.0005 per sibling** *(computed)*. T3 and T4 matter more on Anthropic, where a cache write costs 1.25× input, and as contexts grow.

**No mechanism repays preparation on today's tasks.** The most an implementer can save is everything it spends strictly before its first edit: 44% (E8) of a median $0.0036 task, ≈$0.0016, or 0.44 × $0.0059 ≈ $0.0026 at M1. Light is projected at $0.004 or more (§6). T5, T7 and T8 lower the price of preparation; on these sizes they cannot pay it back. **The break-even is a task whose pre-edit cost at Quick exceeds the preparation's cost**, and that is the size Auto should route on (P15).

### 4.8 ▶: implementing a task

- **The session.** A new session in `tasks/<id>/sessions/`, on the Standard profile, unmasked, with hooks and approvals as today, and with writes confined to the folders it holds (§4.3.6). Its `ToolContext` is built by `agent/project/run_ctx.rs` on the hub's run start (§4.3.1). The precedent is `commit_and_push`, which does not touch the folder's slot (`commands/agent.rs:1176-1294`). Unlike that command, a task run **never sets `auto_approve_git`**, because `AUTO_GIT` includes `git push` (`agent/permissions.rs:203`).
- **Quick ▶** sends the person's prompt as the first message. That is today's single timeline.
- **▶ on a prepared task** starts **fresh** by default, the literal "novo agente que vai aplicar o plano inteiro": L1–L4, W, the implement stage text, and in L6 the golden rules and "Implement the plan of #7". The **fork** arm exists for the eval and for D9:
  - a new session that copies the planner's turns (the scope session at Light, the planner at Full) and its `tools_loaded` records, then `ForkedFrom`, then a user turn: "The verified plan supersedes your draft where they differ", followed by L4;
  - its read-before-edit tracker is rebuilt from the copied transcript's `read_file` results whose file digest is unchanged since the read. Those reads were real, so the rule "may edit any text it has actually read verbatim" still holds, but the tracker's input changes: **flagged as a write-gate change** (P14);
  - with worktrees, one more turn remaps the paths: "folder X is now at <worktree path>; the main folder is not a root of this session". A tool call on an old absolute path is refused by `validate_path` and counted in M4.
  - Either way, `<goal>` per acceptance criterion goes through the existing goal path (`commands/agent.rs:299-321`), and the Steps `s1..sN` are pre-seeded so the recital shows them (`agent/task_recital.rs:34`).
- **Staleness.** At ▶, cited files whose blob digest no longer matches the lock (another task merged, or the person edited them) are listed in the addendum, and their excerpts are not seeded. If more than half are stale, ▶ asks "The plan may be stale. Re-prepare?" At Full, it also offers "Re-verify?".
- **The finish line is unchanged:** completion judge, quality gate, golden loop.
- **When the run ends:**
  - With worktrees, the harness commits each worktree that has changes (`git add -A -- . ':!.claudinio'` and `git commit -m "<title> (#7)"`, inside the worktree only), with the repository's own commit hooks (D16), and the task moves to Review. A refused commit (a hook, a commit-msg rule, no `user.name`) leaves the task Failed{Implement} with the hook's output and a Retry.
  - Without worktrees, it moves to Done{Uncommitted} with Changes (`GitChangesModal`) and Commit & push (D17).
- **Follow-ups** in the task chat continue the implementation chain: the context is all there and warm. Read-before-edit still re-reads, because each run starts its own tracker (§4.7).

### 4.9 Worktrees

- **When they are made.** Only when `project.worktrees` is on, and only for git folders. A non-git folder falls back to the lease and the queue, and the card says so. They are created at ▶, never at preparation, one in each git folder the task holds (§4.3.6). Preparation is read-only and **reads the live tree** of the main folder; staleness is detected at ▶ by digest (§4.8).
- **How they are made.** `git -C <folder> worktree add -b task/<slug> <path> <base>`, run through a core git helper. `run_git` moves out of `commands/git.rs:25`, so `agent/` can call it without importing the command layer. The helper sets `GIT_TERMINAL_PROMPT=0`. The slug comes from `doc_session::slug` (`agent/doc_session.rs:374`), with `-2`, `-3` on collision. `WorktreeRef` records the base branch as well as the base commit.
- **Setup.** A project's optional **worktree setup command** (for example `pnpm install`) is approved once when the person turns worktrees on, then run by the harness when each worktree is created, with its output on the card. A failure is Failed{Implement, "setup failed"} with Retry. Without it, the implementer would have to run the install itself, which asks for approval in every parallel task.
- **A worktree run is headless**, in the eval's pattern (`eval/runner.rs:298-345`). It has no LSP, browser or watcher. The folder is its **config root**: `.claudinio.json`, `MEMORY.md`, hook discovery and hook trust come from there, and nothing is copied in. But **the worktree is where things run**:
  - hooks run with `current_dir` and `CLAUDE_PROJECT_DIR` set to the worktree, not the folder (today both are the root the hook context receives, `agent/hooks/runner.rs:172-178`), so a PostToolUse `cargo fmt` formats the task's copy;
  - stdio MCP servers were started with the folder as their working directory, so under WorktreeAutonomy an MCP tool call in a worktree run asks;
  - `remember` writes the config root's `MEMORY.md` through an explicit path that asks, and the harness commit excludes `.claudinio/`.
- **Search in a worktree.** The folder's index is reused, with paths translated, but it can be wrong in both directions:
  - files the task changed (`git diff --name-only <base>` in the worktree);
  - files the folder changed since the base (merges, or the person's edits).
  For those paths, index hits are dropped and the files are searched live. This is pinned by a test (P18), so a stale hit cannot mislead the implementer.
- **The quality gate.** The digest and `base_commit` are the worktree's own (`quality/evidence.rs:25`), so parallel tasks no longer make each other's evidence stale.
- **Dependencies.** `node_modules`, `.venv` and `target/` are **never symlinked writable** into a worktree, because one task's `npm install` would change the main folder and every sibling. The default is an install per worktree, through the setup command. Read-only sharing is D12. For a repository like this one (a `node_modules` and a Tauri `target/`), a worktree is several GB and minutes of compile; P18 measures one and derives the default `max_running_tasks` with worktrees from it.
- **Merge.** Done in the folder: `git merge --no-ff task/<slug>`, serialised per folder under its lease. The button names the real target ("Merge into main"). It is refused while the folder has uncommitted changes or a detached HEAD, and it asks when HEAD is not on the base branch the task started from. On a conflict the harness runs `git merge --abort`, and the card stays in Review with "Conflicts: resolve in the task chat"; that chat runs in the worktree and rebases.
- **Open PR.** `git push -u origin task/<slug>`, with its askpass attributed to the task's session. Then `gh pr create --fill --head task/<slug>` when `gh` is on PATH, otherwise the compare URL opens. The person's click is the approval for that push.
- **Discard.** From Review, Cancelled, Failed or Stopped, after a confirmation: `git worktree remove --force` and `git branch -D`. After a merge the worktree and branch go; after a PR only the worktree goes. Cancel keeps a worktree and offers "Discard worktree".
- **Prune** never deletes work. On startup and when a project opens, the app runs `git worktree prune`, which drops git's admin entries for directories that are already gone. A worktree directory whose task is missing or unreadable is **never removed automatically**: it is listed as "Orphaned worktree", with whether it has uncommitted changes or unmerged commits, and a Discard button.
- **Windows.** Worktree paths are kept short (`<data_dir>\cc\wt\<short id>\…`), worktrees are created with `core.longpaths=true`, and `git worktree remove` is retried, with a "locked, retry" state when a file is held open. CI runs on Ubuntu only (§2.6), so P18 adds a Windows job for the worktree module's tests.

### 4.10 Multi-root and the prompt
- **The tool context.** `ToolContext.workspace_root` becomes `roots: WorkRoots { work: Vec<PathBuf> /* primary first */, config: Vec<PathBuf> }`.
  - `validate_path` accepts any work root.
  - Relative paths join the primary root, or a folder-prefixed path (`api/src/x.rs`) its folder.
  - `grep` with no path searches every root.
  - An **absolute bash `workdir` is checked against the roots**, closing `agent/tools/mod.rs:961-964` (sensitive area).
- **Search.** One index per folder; `code_search` and `semantic_search` merge results by score.
- **Quality evidence** is required for every folder the session changed.
- **The prompt (P11, on its own).** "Project workspace root: …", with its sentence "File tools take absolute paths inside this root", leaves every system prompt (`agent/prompt.rs:69-79, 96-104`; `agent/subagent.rs:544`). The first user turn carries the W layer instead (`<workspace>`, folder name → absolute path, primary marked): after L4 in a task session, after L1 in a chat. It is persisted with the turn like project memory (`agent/run_loop.rs:188-229`). The tool schemas (`agent/tools/mod.rs:316, 329, 352, 389, 400, 413`) name no path and stay as they are, so the tool characters and the core-tool pins do not move. The prefix becomes byte-identical across folders and worktrees, and slightly smaller.
- **A task writes only in the folders it holds** (D19), with one worktree per git folder it holds when worktrees are on.

### 4.11 The project chat and the task chats

**The project chat.**
- **Where it lives.** Sessions are stored in `projects/<id>/chat/`, with New and History in its header.
- **Until P9** it is today's Standard chat, stored in the project home.
- **From P9** it has its own profile, `PromptProfile::ProjectChat`. It is Standard **minus** `tasks_set` (≈248 tokens), `tasks_update` (≈253) and `doc_create` (≈104), **plus** four card tools:
  - `task_create {title, prompt, after?}`; the depth comes from the project's default, as for a task the person types;
  - `task_update {number, title?, prompt?, before?, cancel_reason?, archive?}`; a prompt edit on a prepared task sends it back to Draft;
  - `task_get {number}`, which returns the report;
  - `task_play {number}`.
  The new tools are estimated at 400–550 tokens; the profile's own byte-stable ≤ 5,000 test measures them. Its PLAN section says "a one-step change: do it; anything more is a task; the list is the plan". Each run's user turn ends with a `<project-tasks>` block (`#N · state · title`), built like `agent/task_recital.rs:34`, so no round is spent listing tasks. Whether finished tasks' reports join it as project context is D30.
- **Left out on purpose:** stopping a task, preparing or re-planning it, changing its depth, and merge, open PR or discard. Merge, PR and discard stay clicks on the card under binding 3; a preparation spends money and is the person's click or the creation itself; re-planning belongs to the task chat. A `task_stop` would be harmless and is the first candidate to add.
- **`task_play` always asks**, and so do `task_create` and a prompt-editing `task_update` while auto-implement is on, because then a created or edited task can end in an implementation with no other click. `effective_permission` exempts them from the YOLO and hook-`Allow` relaxation (`agent/tool_exec.rs:45, 74-79`). The `task_play` card reads "▶ Implement #7 *title*", so the person's click *is* the ▶ (binding 1); the `task_create` card reads "Add #14, and implement it once prepared". A chat-created task the person did not approve never auto-implements.
- **Edits are lease-checked.** The chat may make one-step edits (D10), but not in a folder a task is implementing in.
- **Goal tags.** A `<goal>` typed in this chat is carried into the task the chat creates, where it becomes a golden goal of the implementer (D6). The profile has no checklist to close one with.
- **No collision.** No profile carries both `tasks_*` and `task_*` (D5).
- **The hand-off it creates.** A person who types work into the chat gets: the chat explores, writes `task_create`, and after ▶ a fresh implementer explores again. P9 measures that path against Quick ▶ (§5), and if it costs more than noise, the chat's findings ride into the task (a seeded addendum) or the implementer forks the chat session, each gated.

**The task chat**, one per task. **A click on a task opens its chat**, with the preparation rows and the report summary above the conversation.
- **Before ▶**, it is a read-only session (Standard with the stage mask) whose head is L1–L4 and W. The person asks about the plan. The composer offers **Re-plan with this note**: the note goes to the addendum's `<decisions>`, and plan and check run again (verify and fix too at Full). The study is kept. That it cannot write before ▶ follows from bindings 1 and 4 (nothing executes before ▶, and preparation is read-only); it narrows "toda tarefa pode abrir um chat" to a chat that reads and re-plans until ▶.
- **After ▶**, it is the implementation chain. The composer steers a running implementation and continues a finished one.
- **Stage rows** sit above the conversation: scope, one row per study question, plan, check, verify, fix. Each shows rounds, cost and status, and opens its own timeline.

### 4.12 Concurrency and approvals

| Run | Writes | Parallel? | Asks the person? |
|---|---|---|---|
| Preparation (every stage) | nothing (the mask); bash only under `bash_is_read_only` inside the roots | always (binding 4), bounded by the gate | never: refuses, or records a question |
| Implementation, no worktree | the folders it holds, under their leases | one per folder; others Queued | as today, through the inbox; the person's own YOLO applies as it does in a chat today (minus `yolo_blacklist`) |
| Implementation, worktree, policy **Ask** (default) | its worktrees | yes | as above |
| Implementation, worktree, policy **WorktreeAutonomy** (D8) | its worktrees | yes | edits and read-only bash inside the worktree are approved; push, network, MCP and anything outside the worktree ask; the harness commits; nothing merges |
| Project chat | one-step edits in an unleased folder | — | as today; **`task_play` always**, and `task_create` while auto-implement is on |
| Any task run | — | — | permission settings come only from the person's own config and the project, never from a file inside a folder; `auto_approve_git` is never set |

The card always says which limit it is waiting on: "Queued: folder busy", "Waiting: rate limit", "Waiting: provider limit" or "Paused: plan budget". Keyboard approval acts only on the focused card.

### 4.13 The UI

Every new screen follows the same rules:
- English literals only.
- Markdown only through `renderMarkdown` and `ProseContent`.
- Long titles wrap, and a click opens the whole task. No detail lives only in a `title=` attribute; that is the reported bug's class, fixed by construction (binding 5).
- The chat keeps at least 400 px at the 900 px minimum (`src-tauri/tauri.conf.json:18`), checked by a pure width-budget function test. Until P10 removes the right aside, the task list is a **Tasks** tab beside the project chat in the centre column (P7); from P10 it has its own column, and the project column collapses (D15).
- Icons: `archive-drawer` (`src/components/Icon.tsx:254-257`, today's Documents button, `src/components/ChatPanel.tsx:1592`) is reused for Archive; new ones are `play`, `trash` and `grip`.
- Study and Plan tabs use **DocumentReader**, today's `DocumentPanel` section reader (section rows, owner chips that open the writer's timeline, a section's lines opened in the file) extracted in P13c, because `study.md` and `plan.md` are the same `document.rs` format. "Legacy documents" uses it too.

**The project list, and New project:**
```
+--------------------------------------------------------------------------------------------+
| Claudinio Code 0.10                                                  [Inbox 2]  [Settings] |
+--------------------------+-----------------------------------------------------------------+
| PROJECTS             [+] |  +-- New project --------------------------------------------+  |
| * claudinio-code   3 ! 1 |  | Name         [ claudinio                                ] |  |
|   2 folders, 3 running   |  | Folders      /root/claudinio-code          (primary)  [x] |  |
| o claudinio-litellm      |  |              /root/claudinio-litellm                  [x] |  |
| o enemjuntos   preparing |  |              [+ Add folder]                               |  |
| o popbot                 |  | Instruction  [ Desktop app and proxy. TDD always: red   ] |  |
|                          |  |              [ test first. Never edit .env.     ~40 tok ] |  |
|  (right-click: Edit,     |  | [x] A git worktree per task (ends as a commit on          |  |
|   Archive)               |  |     task/<name>; you merge, open a PR or discard)         |  |
|                          |  |     Setup command [ pnpm install                     ]    |  |
|                          |  | [ ] Implement as soon as a task is prepared               |  |
|                          |  | Preparation      (o) Auto  ( ) Quick  ( ) Light  ( ) Full |  |
|                          |  | Task permissions (o) Ask me                               |  |
|                          |  |                  ( ) Inside the task's worktree, don't ask|  |
|                          |  |                                      [Cancel]  [Create]   |  |
| ARCHIVED (2)          >  |  +-----------------------------------------------------------+  |
+--------------------------+-----------------------------------------------------------------+
```
A folder may sit in several projects (D29): here `/root/claudinio-litellm` is in both. The Preparation default follows D1 and D25.

**A project's task list, with the project chat** (from P10; before it, the list is a tab of the centre column):
```
+--------------------------------------------------------------------------------------------+
| claudinio-code . 2 folders                                           [Inbox 2]  [Settings] |
+----------------+-----------------------------------+---------------------------------------+
| PROJECTS    [+]| [ Describe a task...       Add ]  | Project chat          [New] [History] |
| * claudinio-   | Show: (o) Open  ( ) Needs you (1) |---------------------------------------|
|   code   3 ! 1 |       ( ) Done (12)  ( ) Archived | you  Split the askpass work into two  |
| o litellm      |-----------------------------------|      tasks and prepare both.          |
| o enemjuntos   | = #10 Add OAuth scopes    RUNNING | [task_create] #14 Askpass per session |
|                |     step 3/5 . task/add-oauth-... | [task_create] #15 Queue askpass       |
|                | = #11 Rate-limit the API  ! YOU   |                   prompts             |
|                |     approve: cargo add governor   | Both are preparing. #15 touches       |
|                | = #9  Fix login redirect  READY > | askpass.rs; I listed it as a risk     |
|                | = #14 Askpass per session  STUDY  | in its prompt.                        |
|                | = #15 Queue askpass prompts PLAN  |                                       |
|                | = #12 Dark-mode toggle    QUEUED  |                                       |
|                |     folder busy (behind #10)      |                                       |
|                | = #8  Cache headers +42 -7 REVIEW |                                       |
|                |     [Merge] [Open PR] [Discard]   | +----------------------------------+  |
| ARCHIVED (2) > | (= drag: one priority order)      | | Ask, or describe tasks...      > |  |
+----------------+-----------------------------------+---------------------------------------+
```
One list, in priority order, whatever each task's state: the state is a badge on the row, "Needs you" is a filter and a badge, and Done and Archived are filters, not sections. Creating a task takes only its prompt; its depth comes from the project's default and can be changed from the task's menu.

**A task, as a click opens it (its chat, prepared and Ready):**
```
+-- #9 Fix login redirect ---------------------------------- READY . Full . claudinio-code --+
| [> Implement]  [Re-plan]  [Export .md]  [...]          prep $0.021 . 2m14s . 38 requests   |
| Chat | Study 4 | Plan 5 steps | Verify 1 fixed | Report | Changes                          |
|--------------------------------------------------------------------------------------------|
| PREPARATION                                                                    [collapse]  |
|  Scope     5 rounds   $0.002   4 questions, size: medium                       [timeline]  |
|  Study q1  How does login keep `next`?         9 rounds  $0.005  done          [timeline]  |
|  Study q2  Which routes redirect after auth?   7 rounds  $0.004  done          [timeline]  |
|  Study q3  Where is the OAuth callback?        6 rounds  $0.003  done          [timeline]  |
|  Study q4  Which tests cover redirects?        8 rounds  $0.004  done          [timeline]  |
|  Plan      10 rounds  $0.003   5 steps, 9 anchors                              [timeline]  |
|  Check     9 of 9 anchors resolve                                                          |
|  Verify    fresh session  6 rounds  $0.002   1 correction (V3: callers)        [timeline]  |
|  Fix       s3 rewritten  3 rounds  $0.001   check passed                                   |
|  Report    assembled by the app                                                            |
| REPORT  What   The redirect after login drops `next` when ...                  [full >]    |
|         Where  web/src/auth.ts:41-58    keep `next` through the callback                   |
|                web/src/routes.ts:12-20  pass it to the guard                               |
| QUESTIONS FOR YOU (1)                                                                      |
|  Keep `next` across the OAuth round trip?   assumed: yes                                   |
|--------------------------------------------------------------------------------------------|
| you  Why not keep `next` in the session cookie?                                            |
| The plan keeps it in the OAuth state parameter because ... (web/src/auth.ts:44-47)         |
| [ Ask about the plan, or answer a question...    ] [Re-plan with this note]  [Send]        |
+--------------------------------------------------------------------------------------------+
 The same header once the run has finished, with worktrees on:
| #9 Fix login redirect   REVIEW . 3f9c2a1 on task/fix-login-redirect . 3 files +84 -12      |
| [Merge into main]  [Open PR]  [Review diff]  [Discard]                total $0.039 . 6m    |
```

**A task's chat while it implements:**
```
+-- #10 Add OAuth scopes -------------------------- IMPLEMENTING . round 14 . $0.034 [Stop] -+
| Chat* | Study | Plan | Report | Changes                     worktree task/add-oauth-scopes |
| Steps  [x] s1 Scope enum  [x] s2 Parse header  [>] s3 Wire callback  [ ] s4 Tests          |
|--------------------------------------------------------------------------------------------|
| Preparation  scope . 4 questions . plan . verified (1 fix)            $0.019   [details v] |
| > Implement the plan of #10.  (a new session; the plan in its head)                        |
|   read_file    api/oauth.rs:30-60                                                          |
|   edit_file    api/oauth.rs                                                   +12 -3       |
|   run_quality  tests: pass                                                                 |
|   +-- Approval -------------------------------------------------------------------+        |
|   | bash: cargo add oauth2                                  [Approve]  [Reject]   |        |
|   +-------------------------------------------------------------------------------+        |
| [ Steer the agent...                                                        ] [Send]       |
+--------------------------------------------------------------------------------------------+
```

The Inbox opens from the header. It lists each wait as `project . task . what`, with Approve/Reject, Answer, Retry, Resume or Open.

### 4.14 What is removed, and how old data still loads

| Removed | Phase | Old data |
|---|---|---|
| localStorage folder lists as the source of truth (`src/App.tsx:33-60`) | 6b (keys deleted in P20) | each open or recent folder becomes a one-folder project, once (D3). The keys are **kept, read-only**, until P20, so a different answer to D3 or D4 can re-run the migration from them |
| One mounted `ChatPanel` per folder (`src/App.tsx:1249-1256`) | 3 | — |
| The right `<aside>`, `DocumentPanel` as a panel, `TasksPanel`, `taskColumn*`, `taskCounts`, `conversations` (`src/App.tsx:102-182, 1259-1290`); document publishing in `documentStore` (`isLinkedDocument` stays); the Documents popover (`src/components/ChatPanel.tsx:215-216, 1452-1467, 1586-1594, 1646-1675`) | 10 | `DocumentPanel`'s reader survives as `DocumentReader` (P13c). `.claudinio/docs/*.md` stay in the folder and are listed read-only under the project's "Legacy documents", opened in that reader (P10) |
| `WorkspaceState.active_session` (`state.rs:57`), and the task and document commands keyed by workspace | 20 | — |
| The document tools in Standard (`agent/tools/doc.rs:15-113`), the PLAN section's document lines, and the `NewDocument` handoff | 21 | `DocumentLinked`, `Tasks` with journals, `SubagentRun`, `Mode`, `ModeRoute` and `PlanFinalized` stay deserializable; unknown kinds are skipped (`agent/persist.rs:8-10`) |

**Migration** (D3, D4) runs on the first launch of P6b:
- `migrate_legacy_workspaces(open ∪ recent)` matches folders by canonical path, so a second run finds the same projects and changes nothing. `index.json` records `migrated_at`.
- `legacy_sessions` points at `<folder>/.claudinio/sessions`. Those chats open read-only under "Earlier chats". "Continue here" starts a project-chat session that links to the old file, and loading it walks the chain across both directories; old sessions are never made the active one.
- **Nothing in any folder is moved, copied or deleted (R4).**

## 5. The phases

Every phase is one PR whose first commit holds the red tests, seen failing on their assertions. `main` stays shippable after each phase. Commands stay thin adapters.

**Order (D24).** Phase numbers are identifiers. Under D24(b), the recommendation, they land as **0, 1, 1a, 2–5, 6a, 6b, 7–9, 13a–17, 10–12, 18–21**: the preparation he called "a parte principal" comes before the right column goes and before projects span folders. Under D24(a) they land in numeric order. Every dependency below holds in both orders: P13a–P17 need only one folder, the task view of P7 and the ▶ of P8; P13c extracts the document reader whether or not P10 has run; P18 needs P11 and P12 in both.

**Which phases run which gate.** G-Quick: 4, 8, 9, 11, 13b, 14–17, 21. G-Prompt: 9, 11, 14, 21. G-Mechanism: 15 (T3, T4), 16. G-Group: 18. Every other phase is tests only.

| After phase | The app looks like |
|---|---|
| 0–1 | unchanged (eval only) |
| 1a | unchanged; a security fix (release 0.9.1, details under `SECURITY.md`) |
| 2 | unchanged; runs survive their view (invisible) |
| 3 | the same UI, lighter with several folders open; correct status per run |
| 4 | fewer failed subagents; "Waiting: rate limit"; a plan budget pauses instead of failing |
| 5 | an **Inbox**; Stop always stops; askpass says who asked; Enter approves one card |
| 6a | unchanged (storage moves; old sessions still open) |
| 6b | **Projects** in the sidebar (name, one folder, instruction, archive); old chats as read-only history |
| 7 | each project has a **task list**, a Tasks tab beside the chat: add from one prompt, edit, reorder, cancel with a reason, archive |
| 8 | **▶** runs a task in **its own chat**, at Quick (no preparation yet); same-folder tasks queue; Needs you on cards; nothing auto-resumes after a restart |
| 9 | the **project chat drives the list**: "add tasks for X, Y, Z"; ▶ from the chat is a card to click. *The task-list shell is complete; the preparation he described is not in it yet* |
| 10 | **no right column**; the task list gets its own column; the Steps strip in each chat; legacy documents listed |
| 11 | unchanged (the system prompt no longer names a path) |
| 12 | **projects span folders**: "Add folder" |
| 13a | unchanged (the stage mask and the head exist, unused) |
| 13b–13c | **Prepare (Light)**: a report you can read, question, re-plan and export; ▶ implements from it |
| 14 | ▶ on a prepared task starts with its Steps listed and its acceptance as golden goals |
| 15 | **Full** preparation: parallel study questions, a row each; Auto picks the depth |
| 16 | prepared tasks implement with fewer re-reads (if the arm wins) |
| 17 | **Verify and fix**: "Verified, 1 correction fixed". *The preparation he described is complete* |
| 18 | **Worktrees**: tasks implement side by side and end as a commit, with Merge / Open PR / Discard |
| 19 | **Auto-implement**, and the "inside the worktree, don't ask" policy |
| 20 | History per project; opening an old chat never redirects a running one |
| 21 | unchanged; a lighter default prefix; current documentation |

### Phase 0: Meters (eval only)
- **What changes.**
  - `Usage.stage`, with a serde default.
  - `eval/metrics.rs` gains the per-stage split, M7 and M7e, M8, M10 and M12.
  - `--price-model model=in,out,read,write`, because `Prices` is a single set today.
  - No other flag yet: each one lands in the phase that reads it (rule 13), so `clippy --all-targets -D warnings` never sees an unread field (`eval/runner.rs:43-56` derives only `Debug, Clone`).
  - `scripts/session-stats.py --meters` computes the same numbers.
- **Files:** `agent/persist.rs:382-400`; `eval/metrics.rs:15-38, 49-89, 91-200`; `eval/mod.rs:221-256`; `eval/runner.rs:44-73`; `scripts/session-stats.py`, `scripts/test_session_stats.py`; `eval/README.md`.
- **Red first.**
  - `eval::metrics::tests::usage_is_split_by_stage`: `by_stage["study"].cost == 0.001`. The stub returns an empty map.
  - `…::a_reread_is_a_cited_file_read_again_and_not_edited`: a study answer's `## Facts` cites `a.py:3-5` and `b.py:9-12`; the next session reads both and edits `b.py`. Asserts `rereads == 1` and `m7e == 1`. The stub returns 0.
  - `…::an_explore_reports_files_section_counts_as_cited`: the same with a `## Files` section (`agent/delegation.rs:21-26`), for today's transcripts.
  - `…::first_request_cache_share_counts_siblings_after_the_first`: four transcripts replay one E5 fan-out. Asserts that the two cold siblings after the first give < 0.05 and the warm one 0.867. The stub returns `None`.
  - `…::pre_edit_cost_stops_before_the_request_that_edits`: three requests, the third issues `edit_file`; asserts M12 is the first two's share. The stub returns 0.
  - `eval::tests::a_model_price_overrides_the_run_prices`: fails because the flag is unknown.
  - `persist::tests::a_usage_record_without_stage_loads`: a pin, green on purpose.
- **Gate: a meter that cannot reproduce a known number is wrong.** Re-collecting `/srv/claudinio-eval/work-st-{a,b}` must give:
  - M1 $0.0059 / $0.0063 on the 25-task subset, and $0.0079 / $0.0076 on all 27 *(computed from `out/st/st-{a,b}.jsonl`)*;
  - M10 showing 1 warm sibling of 10 (siblings 2..N of E5's 4 fan-outs), at 3,456 / 3,986 for that one;
  - M12's medians: 44% strictly before the first edit, against 59% including that request (E8);
  - the E3 hand count (7–9 source files re-read after the subagents, in 4 of 4 sessions), with "cited" taken as "read by a subagent". The stricter `## Files` count is reported beside it.

### Phase 1: Bigger tasks, a parallel track, baselines (eval only; can run beside Phase 2)
- **What changes.**
  - **At least 6 `large-*` tasks now, growing to 12** before any depth evidence decides anything (§1). Each spans ≥ 2 areas and ≥ 4 files, comes from this repo's history and is graded by vitest like the `hist-*` tasks. For example, `0d82ceb` "offer to bootstrap a suite" touches 6 source and test files across settings, chat and lib.
  - **`eval/groups/*.yaml`**: two groups of 4 independent tasks on one start tree, plus `run --group <file> --parallel N`.
  - **Baselines at `47ba241`** in `eval/baselines/2026-10-2x-before-projects/`: the 25 + 2 tasks × 2, the large subset × 3, each group serial × 1, and the large set's own noise.
- **Files:** `eval/tasks/large-*.yaml`, `eval/groups/` (new); `eval/task.rs:41-61`; `eval/runner.rs:44-57`; `eval/mod.rs:262-282`; `eval/README.md`.
- **Red first.**
  - `validate` on each new task fails on its start tree and passes on its reference. That is the existing contract.
  - `eval::task::tests::a_group_refuses_tasks_with_different_start_trees`.
  - `eval::runner::tests::parallel_runs_overlap`: a stub provider sleeps 1 s per request, and two tasks with `--parallel 2` must finish in < 1.8 s. Today they take ≈ 2 s, serial (`eval/mod.rs:262-282`).
- **Gate:** every task is valid, and the baselines are recorded.

### Phase 1a: A security fix (sensitive)
- **What changes.** A fix to a security issue this audit found. Under `SECURITY.md` it is handled privately: red tests first, released alone as 0.9.1 before Phase 2, and described here once the release is out.
- **Gate:** tests only.

### Phase 2: The run hub
- **What changes.**
  - `agent/runs.rs` (§4.3.1), with **run start moved into core**: `ToolContext` assembly, the hook context, the transition maps and the spawn loop that follows handoffs. `send_message`, `spawn_run_loop`'s callers and `commit_and_push` register their runs through it; the caller's `Channel` is the first subscriber, so the frontend is unchanged.
  - New commands `attach_run`, `detach_run`, `list_runs` and `watch_runs`.
  - `link_session` swaps the folder's slot only if that slot held the old session.
- **Files:** `agent/runs.rs`, `commands/runs.rs` (new); `state.rs:146-216`; `commands/agent.rs:150-351` (`:246-265` moves), `:359-395` (moves), `:417-498` (moves), `:1176-1294`; `agent/transition.rs:140-159`; `lib.rs:39` (handler list); `src/lib/ipc.ts`.
- **Red first.**
  - `runs::tests::a_late_subscriber_gets_the_missed_events_then_live`: asserts `[e1, e2, e3]`. The stub's `attach` sends nothing.
  - `runs::tests::a_detached_subscriber_receives_nothing`.
  - `runs::tests::re_attaching_a_view_does_not_duplicate_events`: two `attach` calls with one view key, then one event; asserts it arrives once.
  - `runs::tests::status_follows_the_event_stream`.
  - `runs::tests::starting_a_run_on_a_live_session_is_refused`: today a second `send_message` reuses the live session (`commands/agent.rs:177-196`).
  - `runs::tests::a_send_right_after_done_is_accepted`: today the steering entry is removed after `Done` reaches the UI (`commands/agent.rs:491-495`), so a hub that cleared "live" there would refuse it.
  - `transition::tests::a_handoff_moves_the_live_run_to_the_successor`.
  - `transition::tests::a_handoff_leaves_a_folder_slot_it_does_not_hold`: asserts `ws.active_session.id == "chat"`. It fails today because the swap is unconditional (`:151-159`). **The code is wrong** once two sessions share a folder; that cannot happen before P8.
- **Pins:** `lib.rs::core_modules_do_not_depend_on_the_command_layer` stays green with run start in `agent/`.
- **Gate:** tests only, since the model sees no change. **Ships as:** invisible.

### Phase 3: The run store, one mounted chat view
- **What changes:** §4.3.2. The status per folder is derived from run statuses, and keep-awake reads it.
- **Files:** `src/lib/runReducer.ts`, `src/lib/runStore.ts` (new); `src/components/ChatPanel.tsx:127-288, 795-1132, 1291-1365`; `src/App.tsx:198-203, 1249-1256`; `src/lib/workspaceStatus.ts:5-28`.
- **Red first.**
  - `runReducer.test.ts`, "replaying a recorded stream equals the live timeline", over fixtures captured from a real session. The stub returns its initial state.
  - `runReducer.test.ts`, "attach mid-run, then live, equals live from the start" (R3).
  - `runStore.test.ts`, "a session with no view still updates its status": expects `awaiting_approval`. The stub keeps `idle`.
  - `runStore.test.ts`, "twenty sessions ingest a streaming fixture and the store registers no timers and no document listeners, while the one mounted view registers its own". Against the stub it fails on the status it never computes. The timer half is a guard: green on the stub, red the day a change adds a timer or listener to the store.
  - A source pin in `ChatPanel.test.ts`, "the panel no longer defines `const handleEvent = (event: AgentEvent)`". It is red today at `:795`.
- **Pins:** `ChatPanel.test.ts:337-353` stays green.
- **Gate:** the unit invariants above, plus a manual measurement written into the PR: listener and timer counts in the devtools at 1 and at 20 running sessions. CI has no browser harness, so the number is recorded, not gated.

### Phase 4: The provider gate (G-Quick)
- **What changes:** §4.3.3, the budget pause included (dispatch stops and runs are told; the task half lands in P8).
- **Files:** `agent/provider_gate.rs` (new); `agent/provider.rs:1136-1214` (judge), `:1219-1260` (`one_shot`), `:1311-1460` (`stream_message`), **net ≤ +20 lines**; `agent/provider/openai.rs:379-400, 708-760`; `commands/local_llm.rs:712`; `agent/subagent.rs:716-746`; `agent/retry.rs:56-110`; `agent/provider/http_error.rs` (carries `Retry-After`); `agent/events.rs:96-97` (`Retrying` gains `rate_limited`); `commands/agent.rs` (clamp in `set_config`); `src/components/settings/SettingsModels.tsx:85-112`; `llama/mod.rs:254-256`.
- **Red first.**
  - `provider_gate::tests::never_more_than_the_limit_in_flight`: 20 tasks against a limit of 3 see at most 3 at once.
  - `…::the_completion_judge_takes_a_permit_and_honours_the_cooldown`: today it posts directly (`agent/provider.rs:1179-1181`).
  - `…::a_429_makes_every_new_request_wait_out_the_cooldown`, on a paused clock.
  - `…::interactive_requests_keep_their_reserved_permits`.
  - `…::a_limit_of_one_still_serves_background_runs`.
  - `…::preparation_progresses_while_max_implementations_run`: four implementation sessions loop on a limit of 6; a preparation request is served within one round-robin turn.
  - `…::stop_while_waiting_for_a_permit_returns_at_once`.
  - `…::a_budget_exceeded_error_pauses_dispatch_and_fails_no_other_run`.
  - `subagent::tests::a_subagent_rides_out_a_429`: the stub returns 429, then OK. Today the agent fails with "Provider error" (`agent/subagent.rs:735-746`).
  - `retry::tests::two_sessions_do_not_retry_in_lockstep`.
- **Gate:** G-Quick on the 25 tasks × 2. The runner is serial, so the numbers should not move, and this run confirms it.

### Phase 5: The inbox
- **What changes:** §4.3.4–4.3.5, and keyboard approval on the focused card only.
- **Files:** `agent/inbox.rs` (new); `agent/session.rs:196-200`; `agent/tool_exec.rs:325-347, 448-471, 579-614, 624-647, 746-769, 885-897`; `commands/agent.rs:646-700, 1151-1169`, plus `cancel_run`; `askpass.rs:24-81` and its helper script; `agent/tools/bash.rs:143-148, 196-204`; `src/components/Inbox.tsx` (new); `src/App.tsx:271-279`; `src/components/AskpassModal.tsx`; `src/components/chat/TimelineRows.tsx:1429-1440`. **Flag in the PR:** the approval path and askpass.
- **Red first.**
  - `inbox::tests::stop_while_waiting_for_approval_ends_the_wait`: `timeout(1s, wait)` returns `Cancelled`. Today it never returns (`agent/tool_exec.rs:344`).
  - `tool_exec::tests::a_stopped_approval_returns_a_cancelled_result`.
  - `inbox::tests::every_pending_item_names_its_session_and_owner`.
  - `askpass::tests::an_askpass_prompt_names_the_session_that_ran_the_command`: `env_for_child("s-1")` contains `CLAUDINIO_SESSION_ID=s-1`, and the request carries it. Today the function takes no session (`askpass.rs:61-81`).
  - `askpass::tests::two_sessions_can_each_have_a_prompt_pending`: fails on the global counter (`askpass.rs:30-32`).
  - `ApprovalCard.test.tsx`, "Enter approves only the focused card". Today both cards approve, which is **a code bug**.
  - `AskpassModal.test.tsx`, "a second prompt waits behind the first". Today it replaces the first (`src/App.tsx:274-276`), also **a code bug**.
- **Pins:** the 300 s askpass timeout stays (D28); its test, if D28 keeps it, is `askpass::tests::an_expired_prompt_fails_the_command_and_says_retry`.
- **Gate:** tests only.

### Phase 6a: Projects and the storage home (backend)
- **What changes.**
  - §4.1 (the project half), §4.2, and the migration in §4.14, which implements D3(a) and D4(a) and keeps the legacy keys.
  - `create_in`, and every reader of `<root>/.claudinio/sessions` (§2.4) takes the session's directory.
  - The instruction becomes L1 (D20): sent in the first user turn of each new session, with no cap of the app's own; a session that continues after the instruction changed gets one `<project-instruction-updated>` block in its next user turn.
- **Files:** `agent/project/{mod,store,migrate}.rs`, `commands/projects.rs` (new); `agent/persist.rs:647-718` (`create_in`), `:1193-1230` (`chain_cumulative_cost`), `:1263` (`list_sessions`); `agent/doc_session.rs:790-807, 960-972`; `commands/document.rs:18`; `agent/notes.rs`; `agent/transition.rs:58-78, 121`; `agent/project_memory.rs:569-600`; `commands/agent.rs:552-643, 1197-1202`; `lib.rs`.
- **Red first.**
  - `project::store::tests::a_project_round_trips_through_its_file`.
  - `…::archiving_a_project_never_touches_its_folders`: the folder tree is snapshotted before and after.
  - `…::archive_stops_nothing_and_unarchive_restores_the_project`.
  - `…::migration_is_idempotent_and_matches_folders_by_path`.
  - `…::a_folder_may_belong_to_two_projects` (D29(a)).
  - `transition::tests::a_successor_is_created_beside_its_predecessor`: today it is always `<ws>/.claudinio/sessions` (`:77`).
  - `doc_session::tests::a_project_home_session_resolves_its_document`: today `session_file` looks only under the folder (`agent/doc_session.rs:794-807`).
  - `persist::tests::chain_cost_survives_a_handoff_in_the_project_home`: today `chain_cumulative_cost` walks the folder's directory and returns `None` (`agent/persist.rs:1197`).
  - `persist::tests::a_session_continued_from_a_legacy_chat_loads_its_whole_chain_across_directories`: today `load_session` walks one directory (`commands/agent.rs:578-626`).
  - `project_memory::tests::a_project_instruction_rides_in_the_first_user_turn_only`.
  - `project_memory::tests::an_edited_instruction_reaches_a_continued_session_once`.
  - `prompt::tests::an_instruction_never_changes_the_run_prefix`.
- **Gate:** the existing byte tests prove the prefix is unchanged. No eval.

### Phase 6b: Projects in the sidebar (UI)
- **What changes.** The project list, the dialog (one folder per project for now; "Add folder" arrives in P12), Archive and unarchive, Edit, "Earlier chats". Opening a project opens its folders; other projects' folders open lazily, instead of re-indexing every stored folder at startup (`src/App.tsx:314-323`).
- **Files:** `src/App.tsx:33-60, 96-121, 314-323, 618-705, 954-1237`; `src/components/EmptyState.tsx`; `src/components/ProjectList.tsx`, `ProjectDialog.tsx`, `src/lib/projects.ts` (new); `commands/code_intel.rs`.
- **Red first.**
  - `projects.test.ts`, "open and recent folders become one-folder projects once, and the old keys stay readable".
  - `ProjectList.test.tsx`, "archived projects are hidden behind Archived (n), and unarchive brings one back".
  - `ProjectDialog.test.tsx`, "the instruction shows its size in tokens".
- **Pins:** `src/App.test.ts:41-153` (the localStorage helpers) stays green: the keys are read, not deleted, until P20. **Nothing deliberate changes here.**
- **Gate:** tests only.

### Phase 7: The task list
- **What changes.**
  - The task half of §4.1, the project actor for `order.json` and ids.
  - The pure state machine of §4.4.
  - Cancel with a reason, archive, the folder-removal rule (§4.3.5).
  - The task list as a **Tasks** tab beside the project chat, and a task-view shell whose landing tab is Chat. There is no ▶ yet.
- **Files:** `agent/project/{task,task_store,actor}.rs`, `commands/project_tasks.rs` (new); `src/lib/taskStore.ts`, `src/lib/layout.ts`, `src/components/TaskList.tsx`, `TaskView.tsx` (new); `src/components/Icon.tsx:4-258`; `lib.rs`.
- **Red first.**
  - `task::tests::every_transition_in_the_table`: table-driven; the stub returns `Err`.
  - `…::play_is_refused_while_preparing_and_names_the_stage`.
  - `…::every_non_terminal_state_can_be_stopped_or_cancelled`: exhaustive over the enum.
  - `…::editing_a_ready_tasks_prompt_sends_it_back_to_draft`.
  - `task_store::tests::reordering_rewrites_only_order_json`.
  - `task_store::tests::concurrent_creates_get_distinct_ids_and_both_appear_in_order`.
  - `project::store::tests::removing_a_folder_a_task_holds_is_refused`.
  - `TaskList.test.tsx`, "the order is one sequence, whatever the states": a Ready task dragged above a Running one stays there, and no state heading splits the list.
  - `TaskList.test.tsx`, "a long title wraps and a click opens the task's chat". This is the reported class of failure: `TasksPanel.test.tsx:147-153` pinned the detail into a tooltip.
  - `TaskList.test.tsx`, "creating a task asks only for its prompt".
  - `layout.test.ts`, "at 900 px with the right aside open, the chat keeps at least 400 px": the stub returns a width budget that sums past 900.
- **Gate:** tests only.

### Phase 8: ▶ (Quick), the queue, the task chat, restart (G-Quick)
- **What changes.**
  - §4.3.6 and §4.8 (the Quick path), with app-wide leases taken all at once.
  - **Sessions addressed by id**, the core that P20 used to hold: `load_session_records(project, sid)` with no side effect, `send_to_session(project, sid, msg)`, and compaction by session id. A task's transcript lives in `tasks/<id>/sessions/`, so without these the task chat could neither open its past nor take a follow-up without seizing the folder's slot.
  - The restart table of §4.4, and the budget pause's task half (`Stopped{stage, Budget}`).
  - The task view's Chat tab, attached through the run store.
  - Keep-awake follows the hub, and `SessionEnd` fires for hub runs on quit.
  - The eval flags `--mode task --depth quick`.
- **Files:** `agent/project/{scheduler,run_ctx}.rs` (new); `agent/runs.rs`; `agent/gate_text.rs` (`FOLDER_BUSY`, `NOT_HELD`); `agent/tool_exec.rs` (the lease check); `commands/agent.rs:571-636, 1011-1106`; `commands/project_tasks.rs` (play, stop, resume); `commands/hooks.rs:262-288`; `src/lib/ipc.ts`; `src/components/TaskView.tsx`; `src/App.tsx:198-203`; `eval/runner.rs`.
- **Red first.**
  - `scheduler::tests::two_implementations_in_one_folder_run_one_after_the_other`.
  - `scheduler::tests::a_preparation_never_waits_for_a_lease`.
  - `scheduler::tests::a_folder_in_two_projects_has_one_lease`.
  - `task::tests::a_restart_stops_every_live_state_and_starts_nothing`: covers Preparing, Queued and Implementing. Review and Ready are unchanged.
  - `task::tests::a_budget_pause_stops_every_task_and_fails_none`.
  - `tool_exec::tests::a_chat_edit_in_a_folder_a_task_is_implementing_in_is_refused_with_the_next_step`. Today the edit is allowed.
  - `run_ctx::tests::a_task_run_never_touches_the_folder_chat_slot`.
  - `run_ctx::tests::a_task_run_never_auto_approves_git`: `AUTO_GIT` includes push (`agent/permissions.rs:203`).
  - `runs::tests::opening_a_task_transcript_leaves_the_project_chat_slot_alone`: today `load_session` overwrites the slot (`commands/agent.rs:628-632`).
  - `runs::tests::a_follow_up_in_a_task_chat_continues_that_task_session`: today `send_message` continues only the slot (`:177-196`).
  - `TaskView.test.tsx`, "Play shows Queued behind #7 when the folder is busy", and "after a restart, Resume starts the run only on click".
- **Gate:** the task wrapper must cost nothing. G-Quick through a card (`--mode task --depth quick`), 25 + 2 tasks × 2. The default prefix is unchanged.

### Phase 9: The project chat drives the list (G-Prompt)
- **What changes:** §4.11, the project-chat half, and an eval mode for it.
- **Files:** `agent/session.rs:152-177` (`ProjectChat`, with `is_chat` at `:166`); `agent/prompt/project.rs` (new); `agent/prompt.rs:137-178, 1434-1442`; `agent/tools/project_tasks.rs` (new); `agent/tool_exec.rs:29-86` (the card exemptions); `agent/task_recital.rs`; `agent/surface.rs`; `agent/deferred.rs`; `src/components/tool-renderers/toolPresentation.ts`; `eval/runner.rs` (a ProjectChat mode with an injected `ProjectStore` root; the task list exported as `ws/.oracle/tasks.json` before the oracle runs) and `eval/task.rs`; `eval/tasks/chat-*.yaml` (new, 4–6 tasks such as "add tasks for X, Y and Z", "move the second above the first", "cancel Y with this reason").
- **Red first.**
  - `prompt::project_chat_tests::the_project_chat_prefix_is_byte_stable_and_at_most_5000_tokens`: fails because the profile does not exist.
  - `…::the_project_chat_offers_task_tools_and_no_checklist_or_document_tools`: an exact list.
  - `tool_exec::tests::task_play_asks_even_with_yolo_and_a_hook_allow`: `effective_permission` must return `RequiresApproval` for `task_play`, both with `yolo_mode = true` and with `PreToolVerdict::Allow`. It fails today because `:74-79` relaxes both.
  - `project_tasks::tests::task_create_records_the_chat_as_author`.
  - `project_tasks::tests::a_goal_tag_in_the_chat_becomes_a_goal_of_the_task_it_creates`.
  - `task_recital::tests::the_project_chat_turn_ends_with_the_task_list`.
  - `eval::runner::tests::a_chat_task_oracle_sees_the_task_list_the_chat_wrote`.
- **Pins:** none. The Standard profile is byte-identical.
- **Gate (G-Prompt).**
  - The chat tasks: M3 is all of them, and M4 for the card tools is ≤ 5%.
  - The ProjectChat prefix, measured by its test, is ≤ 5,000. It drops ≈605 tokens of checklist and document tools and adds the four card tools, estimated at 400–550.
  - **The chat-to-task hand-off, reported under G-Quick's rules.** The 25 + large tasks typed into the project chat, the `task_play` card approved by policy: M1 of chat plus implementer, and the implementer's M7 and M12, against Quick ▶ on the same tasks. If the hand-off costs more than noise, a follow-up PR carries the chat's findings into the task (a seeded addendum) or forks the chat session for the implementer, each with its own gate.
- **Ships as:** release 0.10.0 under D24(a); under D24(b) the release waits for P17 (D21).

### Phase 10: The right column goes (UI only)
- **What changes:**
  - Removed: the aside, `TasksPanel`, `taskColumn*`, the Documents popover, and `DocumentPanel` as a panel. Its reader stays as `DocumentReader` (extracted in P13c, or here if P10 lands first).
  - The task list gets its own column (D15), and the width budget is three columns.
  - A **Steps strip** in each chat header shows whichever plan the session has: checklist rows, or a document outline, because a Standard session can still create one until P21.
  - Golden-goal dismissal moves to the strip. `TasksPanel.tsx:125-137` was its only UI.
  - **Legacy documents:** a read-only list under the project of each folder's `.claudinio/docs/*.md`, opened in `DocumentReader`.
- **Files:** `src/App.tsx:102-182, 1259-1290`; `src/components/ChatPanel.tsx:215-216, 1452-1467, 1586-1594, 1646-1675`; `src/components/TasksPanel.tsx`, `DocumentPanel.tsx` and their tests; `src/lib/documentStore.ts`; `src/lib/layout.ts`; `src/components/StepsStrip.tsx`, `LegacyDocuments.tsx` (new); `src/components/DocumentReader.tsx` (new here if P13c has not run).
- **Red first.**
  - `StepsStrip.test.tsx`, "a step's full title and journal show on click".
  - `StepsStrip.test.tsx`, "a golden goal can be dismissed".
  - `StepsStrip.test.tsx`, "a document outline shows when the session has one".
  - `LegacyDocuments.test.tsx`, "a folder's documents are listed read-only and open in the reader".
  - `layout.test.ts`, "three columns at 900 px leave the chat at least 400 px with the project column collapsed".
- **Pins that go red are deliberate changes** (the vision of 2026-10-22: the right column goes):
  - `src/App.test.ts:155-183` is deleted with the code it tests.
  - `TasksPanel.test.tsx` is deleted with its component. `DocumentPanel.test.tsx` keeps its reader cases, moved to `DocumentReader.test.tsx`; its panel and publishing cases are deleted.
  - `documentStore.test.ts` loses its publishing cases.
  - `ChatPanel.test.ts:350-353` is inverted: the panel must *not* contain `listDocuments`.

### Phase 11: A prefix with no path in it (G-Prompt)
- **What changes:** the root line, with "File tools take absolute paths inside this root", moves from the three system prompts into the W layer (§4.5, §4.10). The tool schemas are not touched (`agent/tools/mod.rs:316, 329, 352, 389, 400, 413`), so the tool characters and the core-tool pins hold.
- **Files:** `agent/prompt.rs:66-127`; `agent/subagent.rs:534-556`; `agent/run_loop.rs:188-229`.
- **Red first.**
  - `prompt::tests::the_run_prefix_is_the_same_bytes_for_two_workspace_roots`: today the prefixes differ at "Project workspace root: /a" (`agent/prompt.rs:72`).
  - `subagent::tests::the_subagent_prompt_names_no_absolute_path`: fails at `:544`.
  - `run_loop::tests::the_first_turn_names_the_workspace_once_and_replays_identically`.
  - `prepare::head::tests::the_workspace_layer_sits_after_the_plan_layer`: today there is no W layer.
- **Pins:** `agent/run_loop.rs:1929` asserts `system.contains("Project workspace root: /ws.")`. It is replaced by a first-turn assertion. **A deliberate change:** the path moved, and the eval gate below judges it. The tool-description pins stay green.
- **Gate (G-Prompt).** 25 + large tasks × 2: M3 holds, M1 within noise. Main M4 ≤ 2.5%, watching `read_file` path errors. Default prefix ≤ 4,813 (it drops by the root line).

### Phase 12: Multi-folder projects (sensitive)
- **What changes:** §4.10. "Add folder" in the dialog, one file tree per folder, @-mentions prefixed by folder, the write confinement of §4.3.6 across folders, and multi-root eval tasks.
- **Files:** `agent/tools/mod.rs:32-98, 143-238, 955-967`; `workspace_path.rs:12-70` (**sensitive**); `agent/permissions.rs` (the memory guard per root); `agent/tools/quality.rs`; `quality/evidence.rs:25`; `agent/project/scheduler.rs`; `commands/fs.rs:188` (**sensitive**); `src/lib/fileIndex.ts:4`; `src/lib/ipc.ts:22-31` (`pickFolder` with several folders); `ProjectDialog.tsx`; `eval/task.rs` (several `setup` roots), `eval/materialize.rs`, `eval/runner.rs`, `eval/tasks/large-two-folder-*.yaml` (new).
- **Red first.**
  - `tools::tests::a_path_in_the_second_folder_is_accepted`: today `validate_path` knows one root (`:143-149`).
  - `…::a_path_outside_every_folder_is_refused`.
  - `…::an_absolute_bash_workdir_outside_the_folders_is_refused`: today it is unchecked (`:961-964`).
  - `run_ctx::tests::a_prepared_task_cannot_write_a_folder_outside_its_lease`: the refusal names "Re-plan, or add it to the task".
  - `scheduler::tests::two_tasks_needing_two_folders_never_deadlock`: A holds `api` and wants `web`, B the reverse; both finish.
  - `scheduler::tests::a_quick_task_holds_the_folders_its_prompt_mentions`.
  - `quality::tests::evidence_is_required_for_every_folder_the_session_changed`.
  - `eval::materialize::tests::a_task_with_two_roots_materializes_both_and_the_ctx_sees_both`.
- **Pins:** `prompt::tests::the_prefix_is_identical_for_one_and_three_folders`, green from P11 on.
- **Gate:** the two-folder large fixture passes with Quick. M3 on the other sets holds (one run).
- **Ships as:** release 0.11.0 under D24(b).

### Phase 13a: Preparation I, the mask and the head (sensitive)
- **What changes.** The stage mask (§4.5) with the eval's read-only bash rule, the head builder (L1, L2, L4, L5, L6, and the slot W fills once P11 lands), the L5 override, and the golden rules of task runs moved into the head (D23). Nothing calls them yet.
- **Files:** `agent/stage.rs`, `agent/prepare/{mod,head}.rs` (new); `agent/tool_exec.rs:29-86`; `workspace_path.rs` (receives `stays_in_workspace` from `eval/policy.rs:82`, generalised to several roots; **sensitive**); `eval/policy.rs:56-66` (calls it there); `agent/prompt.rs:116-178` (`run_prefix` takes where the golden rules go); `agent/gate_text.rs`.
- **Red first.**
  - `stage::tests::a_read_only_stage_refuses_edit_file_without_an_approval_request`: the result starts "Error: preparation is read-only" and no `requires_approval` event is sent. Today the edit asks.
  - `stage::tests::a_read_only_stage_refuses_cargo_build_npm_run_dev_and_interpreter_scripts_without_asking`: `cargo build`, `cargo test`, `npm run dev`, `node x.js` and `python -c 'print(1)'` each come back refused, with no approval request. Under `bash_call_permission` each would be Auto (`agent/permissions.rs:147-184`).
  - `stage::tests::a_read_only_stage_runs_git_log_and_refuses_a_redirect_without_asking`.
  - `stage::tests::ask_user_in_a_stage_is_recorded_and_answered_with_best_judgement`.
  - `prepare::tests::a_stage_session_sends_the_standard_system_prompt_and_tools_byte_for_byte`: uses `scripted_model::spawn_recording` (`agent/subagent.rs:1658`) and compares against `run_prefix(Standard)` for the same root. **This is T2's pin from day one.**
  - `prepare::head::tests::layers_are_project_task_plan_stage_item_byte_for_byte`.
  - `prepare::head::tests::every_stage_text_opens_with_the_override`.
  - `prompt::tests::a_task_session_with_goals_sends_the_system_prompt_of_one_without`: today `GOLDEN_PROMPT` is appended (`agent/prompt.rs:121-126`).
- **Pins:** `prompt::tests` on `GOLDEN_PROMPT` for chats (`agent/prompt.rs:1581`) stay green: a chat is unchanged.
- **Gate:** tests only; the default prefix is unchanged.

### Phase 13b: Preparation I, Light (G-Quick; eval-reported)
- **What changes.** Scope and plan in one session, the check, the report, Questions, Re-plan, Export, the task cancellation token, and the fresh kickoff for ▶ on a prepared task. The project's default depth follows D25. The eval flags `--mode pipeline --depth light`.
- **Files:** `agent/prepare/{scope,plan,check,report}.rs` (new); `agent/persist.rs` (`Stage`); `src/lib/ipc.ts:544`; `commands/project_tasks.rs` (prepare, re-plan, `get_task_artifact`, export through the pattern at `commands/fs.rs:298`); `eval/runner.rs:307-430`.
- **Red first.**
  - `prepare::plan::tests::excerpts_are_copied_from_the_file_not_the_message`: when the model's claimed text differs from the file, the file's lines are kept and the claim is flagged.
  - `prepare::plan::tests::a_long_plan_is_kept_and_flagged_never_resent`: a 14-step plan reaches Check with a "longer than guided" flag and no second request.
  - `prepare::tests::a_stage_request_sets_no_max_tokens_of_its_own`: the recorded body carries the model's own output limit (D27).
  - `prepare::check::tests::an_anchor_past_the_end_of_the_file_is_sent_back_to_the_planner`.
  - `prepare::report::tests::the_report_costs_no_request`: the stub sees 0 requests.
  - `prepare::tests::stop_between_scope_and_check_starts_nothing_more`.
- **Pins:** `every_record_kind_is_in_the_frontend_union` goes red until `ipc.ts` gains `stage`. That is the guard working.
- **Gate.**
  - G-Quick holds: Quick is unchanged.
  - Stage-mask refusals are ≤ 2% of stage tool calls, reported per stage; above that, the fallback of §4.5 is the next PR.
  - The Light arm is published on 25 + large: M1, M3, M7, M8 and M12 of the implementer, paired against Quick.

### Phase 13c: Preparation I, the task view
- **What changes.** The task's chat with preparation rows above the conversation (§4.13), the Report tab, Export, Questions answered in the chat, and `DocumentReader` extracted from `DocumentPanel` for the Plan tab. Preparation rows are rebuilt on reload: `src/lib/chatRecords.ts` has no `subagent_run` case today, so such rows are never rebuilt.
- **Files:** `src/components/TaskView.tsx`; `src/components/DocumentReader.tsx` (new, from `src/components/DocumentPanel.tsx`); `src/lib/chatRecords.ts`.
- **Red first.**
  - `TaskView.test.tsx`, "renders the report through renderMarkdown and exports it".
  - `TaskView.test.tsx`, "the preparation rows rebuild from the session files after a reload".
  - `DocumentReader.test.tsx`, "an owner chip opens that stage's timeline", moved from `DocumentPanel.test.tsx` and pointed at the extracted component.
- **Gate:** tests only.

### Phase 14: ▶ on a prepared plan: fresh, and the fork arm (G-Prompt)
- **What changes:** T7 and T9. The fresh kickoff stays the default; the fork arm, the `ForkedFrom` record with `persist::fork_session`, the fork's copy of `tools_loaded`, and its read-before-edit tracker rebuilt from real reads (**a write-gate input change, flagged**); Steps seeded from the plan; acceptance criteria as golden goals with their rules in L6 (D23); `--implement fresh|fork`.
- **Files:** `agent/prepare/kickoff.rs` (new); `agent/persist.rs`; `agent/tools/mod.rs:1269-1334` (the tracker rebuild); `src/lib/ipc.ts:544`; `commands/agent.rs:299-321` (the goal path, reused); `agent/task_recital.rs:34`; `agent/deferred.rs` (`loaded_names`); `eval/runner.rs`.
- **Red first.**
  - `kickoff::tests::the_implementer_with_seeded_goals_sends_the_stages_prefix_byte_for_byte`: a planner request and a golden-goal implementer request, recorded by `spawn_recording`, share the system prompt and tools byte for byte. Without D23(b) they differ at `GOLDEN_PROMPT`.
  - `kickoff::tests::fork_copies_the_planner_turns_and_loaded_tools_then_the_supersede_turn`: the first request body holds the planner's messages, the same tool list, then the plan turn.
  - `kickoff::tests::a_forked_implementer_may_edit_what_the_planner_read_while_unchanged`, and `…::but_not_after_the_file_changed`: today the tracker starts empty (`agent/transition.rs:229`, `commands/agent.rs:266`).
  - `kickoff::tests::acceptance_criteria_become_golden_goals`.
  - `kickoff::tests::the_steps_are_seeded_without_a_tasks_set_round`.
  - `kickoff::tests::the_plan_block_is_the_same_bytes_the_check_passed`.
  - `persist::tests::a_forked_session_loads_without_its_parent_file`.
- **Gate.**
  - G-Prompt and G-Quick: the golden rules moved from the system prompt into the head for every task run, Quick ones included.
  - Arms: Quick (A), Light + fresh (B1) and Light + fork (B2), on 25 + large, paired, × 3 on the large subset.
  - **The numbers go to the maintainer for D9; the default stays fresh until he decides.** They are: M1, M3, the implementer's M12 against Quick's, M7 and M7e.
- **Ships as:** release 0.11.0 under D24(a); part of 0.10.0 under D24(b).

### Phase 15: Preparation II, Full (G-Mechanism for T3 and T4)
- **What changes.**
  - Scope questions.
  - Parallel study.
  - Freeze, with `study.lock`, rendering each question as one `## <question>` with its sections at `###`.
  - Warm, then fan out, behind `--no-warm`.
  - The `shared_head` hint behind `--no-shared-head`, off by default (§4.5).
  - Triage for Auto, as a metered call recorded with stage `"triage"`; its routing rule is the break-even size of §4.7, set from the P13–P17 numbers once the sample allows (§1).
  - The Study tab, on `DocumentReader`.
  - **The doc-parallel tasks graded in pipeline mode, on the study alone.** At Full, a doc-parallel task stops after the report, and the runner exports `study.md` to `ws/.claudinio/docs/<id>.md` before the oracle runs, so the oracles are unchanged. The oracle passes if any document under `.claudinio/docs/` passes (`eval/tasks/doc-parallel-job-runner.yaml:233-243`), so the implementer must not write one of its own: in this mode there is no implementer.
  - The eval flags `--depth full|auto`, `--no-warm`, `--no-shared-head`.
- **Files:** `agent/prepare/{study,freeze,fanout,triage}.rs` (new); `agent/provider/cache.rs:50-75`; `agent/surface.rs:232-247` (Compact caps depth at Light); `eval/runner.rs`; `src/components/TaskView.tsx` (one row per question, reusing the `SubagentRow` shape, `src/components/chat/TimelineRows.tsx:921`).
- **Red first.**
  - `fanout::tests::siblings_start_only_after_the_first_returns_usage`: checks the recorded request order. The stub starts them all at once.
  - `fanout::tests::sibling_requests_are_identical_up_to_their_question`.
  - `fanout::tests::stop_while_siblings_wait_starts_none_of_them`.
  - `freeze::tests::the_study_renders_the_same_bytes_from_any_finish_order`.
  - `freeze::tests::each_question_is_one_second_level_section`: the doc-parallel oracle's parser splits on `## ` (`eval/tasks/doc-parallel-job-runner.yaml:202`).
  - `freeze::tests::a_frozen_study_is_never_rewritten_and_additions_go_to_the_addendum`.
  - `freeze::tests::an_unresolvable_citation_is_listed_as_unverified`.
  - `triage::tests::a_failed_triage_falls_back_to_the_project_default`.
  - `triage::tests::triage_cost_is_a_usage_record_with_its_stage`.
  - `cache::tests::the_shared_head_takes_the_fourth_breakpoint`: today there are three markers.
- **Gate.**
  - G-Quick.
  - **M10 ≥ 70% against `--no-warm`** on the eval endpoint, and on claudin.io if D13 funds it. If `--no-warm` reaches the same share, T3 is removed.
  - T4 stays off until an Anthropic-direct run (D13) shows its M10 effect.
  - Depth evidence: Full against Light against Quick on large, and doc-parallel at Full against Quick only (Light has no study to grade): M1, M3, M7, M8, M12. Reported; Auto's thresholds are set only above the minimum sample.

### Phase 16: Seeded reads (G-Mechanism for T8)
- **What changes:** T8, plus the stale-citation addendum at ▶ (§4.8), and `--seeded-reads`.
- **Files:** `agent/tools/mod.rs:1269-1334` (**a write-gate change, flagged**); `agent/prepare/kickoff.rs`; `eval/runner.rs`.
- **Red first.**
  - `tools::tests::a_verified_excerpt_counts_as_read_while_the_file_is_unchanged`: today `check_can_edit` refuses it as not read (`:1303-1316`).
  - `…::a_verified_excerpt_of_a_changed_file_does_not_count`.
  - `kickoff::tests::stale_citations_go_to_the_addendum_and_frozen_bytes_stay`.
  - `kickoff::tests::play_asks_to_re_prepare_when_most_citations_are_stale`.
- **Gate:** G-Quick, and arms with and without seeding, at Light and Full, measured after P14 so the fork's tracker rebuild is known. The feature ships enabled only if M7e drops and M3 holds. Otherwise it is removed, and §6 records the result.

### Phase 17: Verify and fix
- **What changes:** the Full-depth verify and fix (§4.6), the `Verdict` record, the Verify tab, and `--no-llm-verify` as an eval arm.
- **Files:** `agent/prepare/{verify,fix}.rs` (new); `agent/persist.rs`; `src/lib/ipc.ts:544`; `src/components/TaskView.tsx`; `eval/runner.rs`.
- **Red first.**
  - `verify::tests::the_verifier_never_sees_the_planner_transcript`: no planner turn text and no planner tool result appears in any verifier request body recorded by `spawn_recording`.
  - `…::a_correction_without_evidence_is_refused`.
  - `…::uncited_reads_by_a_verifier_are_counted_not_refused`.
  - `…::a_malformed_verdict_is_asked_for_once_then_fails_the_stage`.
  - `fix::tests::each_correction_gets_its_own_fixer`.
  - `fix::tests::two_corrections_on_one_step_run_one_after_the_other`.
  - `fix::tests::a_fixer_writes_only_its_step`.
  - `prepare::tests::check_runs_again_after_fix_and_verify_never_loops`.
- **Gate:** G-Quick, and depth evidence, reported. Full with and without the LLM verifier, on large: M3 and M9. **Verify stays in every Full preparation whatever the result** until the maintainer answers D26. The check stays either way, because it is free.
- **Ships as:** release 0.10.0 under D24(b): projects, the task list, ▶, both chats and the whole preparation.

### Phase 18: Worktrees (G-Group)
- **What changes:** §4.9.
- **Files:** `gitcmd.rs` and `agent/worktree.rs` (new; `commands/git.rs:25` uses the helper); `agent/project/scheduler.rs`; index search for changed paths; `commands/project_tasks.rs` (merge, open PR, discard, discard worktree, orphans); `commands/fs.rs:188` (**sensitive**: worktree roots registered); `agent/hooks/runner.rs:172-178` and `agent/hooks/trust.rs:120-126` (config root apart from the cwd); `agent/project_memory.rs:25` (`remember` from a worktree); `quality/evidence.rs`; startup prune in `lib.rs`; `src/components/TaskView.tsx` (`GitChangesModal` on the worktree); `ProjectDialog.tsx` (setup command); `.github/workflows/ci.yml` (a Windows job for the worktree tests).
- **Red first** (real git in temporary directories):
  - `worktree::tests::play_creates_the_worktree_outside_the_repo_on_task_branch`.
  - `…::the_setup_command_runs_once_before_the_implementer_starts`.
  - `…::finishing_commits_the_task_changes_and_leaves_the_folder_clean`.
  - `…::the_harness_commit_never_includes_dot_claudinio`.
  - `…::a_rejected_commit_leaves_the_task_failed_with_the_hook_output`.
  - `…::merge_refuses_a_folder_with_uncommitted_changes`.
  - `…::merge_asks_when_the_folder_is_not_on_the_base_branch`.
  - `…::two_merges_into_one_folder_run_one_after_the_other`.
  - `…::a_conflicting_merge_is_aborted_and_reported`.
  - `…::discard_removes_the_worktree_and_its_branch`.
  - `…::a_cancelled_task_can_discard_its_worktree`.
  - `…::prune_never_deletes_a_dirty_or_unmerged_orphan`: a worktree whose `task.json` is gone, with a dirty file, survives startup and is listed as orphaned.
  - `hooks::tests::a_hook_in_a_worktree_run_runs_in_the_worktree`: today `current_dir` is the root given (`agent/hooks/runner.rs:178`).
  - `worktree_index::tests::a_hit_in_a_file_the_task_changed_comes_from_the_worktree_not_the_index`, and the same for a file the folder changed since the base.
  - `quality::tests::two_worktrees_of_one_folder_keep_their_own_evidence`.
  - `commands::fs::tests::a_registered_worktree_is_inside_the_guard_and_an_unregistered_one_is_not`.
  - `kickoff::tests::the_worktree_implementer_shares_l1_to_l4_bytes_with_the_stages`.
  - `kickoff::tests::a_forked_worktree_implementer_gets_the_remap_turn_and_the_main_folder_is_not_a_root`.
  - `scheduler::tests::twenty_tasks_on_a_stub_provider_all_finish_and_nothing_waits_unlisted`: a soak test for R1, R2 and R5. **Twenty is tested only against a stub.**
- **Gate.**
  - G-Group on both groups.
  - Reported beside it: both groups together, 8 tasks at `--parallel 8`, on the real provider.
  - Reported beside it: one worktree task on this repository (disk, time to the first `run_quality`), from which the default `max_running_tasks` with worktrees is derived.
- **Ships as:** release 0.12.0.

### Phase 19: Auto-implement and the task permission policy
- **What changes:** `project.auto_implement`, and `TaskPolicy` (§4.12, D8). Auto-implement fires only from a preparation into Ready, never at Quick, and only for a task the person created or whose chat card the person approved; while it is on, `task_create` and a prompt-editing `task_update` always ask.
- **Files:** `agent/task_policy.rs` (new); `agent/tool_exec.rs:29-86` (the policy is consulted; the card exemptions stay); `agent/provider/workspace_config.rs` (a task run's view takes permission settings only from the person and the project); `agent/permissions.rs` (**sensitive**); `agent/project/scheduler.rs`; `agent/tools/project_tasks.rs`; `ProjectDialog.tsx`.
- **Red first.**
  - `task_policy::tests::worktree_autonomy_approves_an_edit_inside_the_worktree_only`.
  - `…::worktree_autonomy_asks_before_a_push`.
  - `…::worktree_autonomy_asks_before_an_mcp_tool`.
  - `…::a_task_run_takes_permission_settings_only_from_the_person_and_the_project`: **a deliberate policy change**.
  - `task::tests::auto_implement_plays_on_a_live_transition_from_a_preparation_only`.
  - `task::tests::auto_implement_never_fires_for_a_quick_task`.
  - `task::tests::auto_implement_never_fires_on_a_task_restored_after_restart`.
  - `task::tests::auto_implement_never_fires_in_an_archived_project`.
  - `project_tasks::tests::a_chat_created_task_never_starts_without_a_click`: with auto-implement on, `task_create` returns `RequiresApproval` even under YOLO and a hook `Allow`, and an unapproved one never plays.
- **Gate:** tests only, because the model sees no change.

### Phase 20: The folder slot goes
- **What changes:** `active_session` is removed, with every caller still keyed by workspace (the session-addressed core landed in P8). Steps and document commands take a session id. The localStorage folder keys are deleted, once D3 and D4 are confirmed.
- **Files:** `state.rs:57`; `commands/agent.rs:150-351, 503-643, 979-1106`; `commands/tasks.rs:1-65`; `commands/document.rs`; `agent/transition.rs:151-159`; `src/lib/ipc.ts:489-594`; `src/App.tsx:33-60`.
- **Red first.**
  - `steps::tests::a_person_edit_with_no_session_is_an_error`: today it returns `Ok(())` and drops the edit (`commands/tasks.rs:33-34`). **A code bug.**
  - `persist::tests::loading_a_session_chain_writes_nothing`.
  - `transition::tests::a_handoff_announces_its_successor_without_touching_workspace_state`.
- **Pins:** `src/lib/ipc.test.ts:426-469` (the shape of `sendMessage`). **A deliberate change:** a session id replaces the folder slot. `src/App.test.ts:41-153` is deleted with the keys.

### Phase 21: The living-document tools leave Standard; docs (G-Prompt)
- **What changes.**
  - `doc_create` and the deferred doc tools leave Standard, together with the PLAN section's document lines and the `NewDocument` handoff. `agent/document.rs` stays, as the format of the study and the plan.
  - M6 is retired.
  - The docs are updated: README, README.pt-BR, `docs/ARCHITECTURE.md`, `eval/README.md`, and CONTRIBUTING's list of sensitive areas, which gains the `tool_exec` approval path, `agent/task_policy.rs` and `agent/worktree.rs`.
- **Files:** `agent/tools/doc.rs:15-113`; `agent/prompt/standard.rs:38-41`; `agent/prompt.rs:1007-1078, 1316-1356`; `agent/deferred.rs`; `agent/handoff.rs`, `agent/transition.rs`, `agent/doc_session.rs` (the `NewDocument` handoff); `eval/metrics.rs:182-191`; the docs above.
- **Red first.**
  - `prompt::tests::the_core_tool_list_has_no_document_tools`, which flips the exact list at `agent/prompt.rs:1007-1047`.
  - `persist::tests::a_session_with_every_removed_kind_still_loads`: a guard, green.
- **Pins, all deliberate:**
  - `doc_create_is_core_and_the_document_tools_wait_unlisted_until_loaded` (`agent/prompt.rs:1056-1078`), which asserts that `doc_create` is sent, is deleted.
  - The PLAN phrases at `agent/prompt.rs:1316-1356` that live in the Document line: `` `doc_create` ``, "3-6 sections", "independent question", "one `spawn_agents` call" (`:1344-1345`), "edits win", "new session", `` `<document>` ``, `` `section` ``.
  - The doc-tool tests in `agent/deferred.rs`.
- **Gate (G-Prompt).** The default prefix drops by about 247 tokens (`doc_create` ≈104 plus the Document line ≈143), measured by the test. M3 holds and M1 is within noise.

## 6. Expected outcome, honestly bounded

- **Product.**
  - Opened folders become projects.
  - One chat per folder becomes a project chat plus one chat per task, opened by a click on the task.
  - The right panel becomes Steps in the chat header and a report inside each task.
  - One run per folder becomes N tasks: preparation always in parallel, implementation in parallel with worktrees.
  - The task-list shell is usable after P9. The preparation he described (study fan-out, plan, fresh verify, fix, report) exists after P17, and under D24(b) it is in the first release that has projects.
- **Runtime.** Runs continue with no view attached. Every wait is listed and stoppable. A 429 slows everyone down instead of killing a subagent, and a spent plan budget pauses every task instead of failing them. Two writers never share a folder without a worktree, and a task never writes outside what it holds. These are invariants with tests (R1–R5), not hopes.
- **Cost of Quick:** equal to today within noise, $0.0059–$0.0063 per solved task on deepseek-flash (25-task subset). G-Quick holds this for whoever picks Quick. What a new task costs **by default** depends on D1 and D25.
- **Cost of preparation on today's small tasks.** These are projections built from measured per-session costs (E6), not measurements.
  - **Light:** +$0.004–$0.010 per card before implementation. One session costs about what an explore agent does ($0.003–$0.008), plus a plan of about 3,000 output tokens (≈$0.0018). That is 1–3× the median small task's whole cost, and **it cannot be recovered on these sizes**: an implementer that redid nothing before its first edit would save 44% (E8), ≈$0.0016 of a median task or ≈$0.0026 at M1. On today's tasks preparation is a pure cost of at least $0.002–$0.008 per task. It is a feature (a reviewed plan before code) with a price shown on every card.
  - **Full:** +$0.02–$0.05 per card: scope, 3–4 study sessions, plan, verify and 1–3 fixers. That is 5–15× the median small task.
  - **The break-even** is a task whose pre-edit cost at Quick exceeds the preparation's cost. That size, measured on the large set, is what Auto should route on.
- **On large tasks we claim nothing.** The hypothesis is that a study and a verified plan cut implementer rounds and raise M3. The eval has never contained such a task. P1 adds the first six, growing to twelve, which is still a small sample.
- **What the mechanisms can and cannot do.**
  - Warm starts save about $0.0005 per sibling at these prices, more on Anthropic, if T3 works at all (E5 does not show why one sibling was warm).
  - Harness-copied excerpts and the code-built report take output tokens, the expensive kind (E7), off every stage.
  - The frozen layers plus the fork's tracker or seeded reads are the only levers on the implementer's re-reading. They lower preparation's price; they cannot repay it on today's sizes.
  - VERIFY re-reads by design. Plans are output.
  - If T5, T7 and T8 do not move M7, M7e and M12, prepared tasks cost a timeline plus a preparation, and D1 rests on product value alone.
- **Plan budgets.** On claudin.io, preparation spends the person's plan windows: twenty Full cards at 5–15× a small task each can empty an hourly window. The gate then pauses every task with a message instead of failing them, but the person still waits for the window to reset.
- **Latency.** Light adds about one session, under a minute on these tasks. Full adds scope, a parallel study, plan, verify and fix: a few minutes (a guess; M11 measures it). Preparation runs in the background, and parallel tasks raise throughput, not one task's speed.
- **Resources.** A worktree run has no index, LSP, browser or watcher of its own. Its cost is the disk for one checkout plus its dependency install (D12): for this repository, several GB and minutes of compile per worktree, which P18 measures and which bounds the default number of parallel worktree tasks more than RAM does.
- **Code (a guess).** About +6–7.5k production lines of Rust across ~22 new modules and +3–4k of TypeScript, against −1.5k removed (the panels, the aside, the document store, the active-session plumbing, the document tools in Standard).
- **What we lose.**
  - The always-visible checklist on the right; it is in the chat header now.
  - Permission settings read from a folder's own files, for task runs.
  - Continuing old chats in place (D4).
  - The single chat that edits anything at any time: the project chat edits only folders no task is implementing in.
  - Before ▶, a task's chat cannot write (bindings 1 and 4).

## 7. Decisions for the maintainer

Rows marked *(departs from your words)* recommend something other than the literal reading of 2026-10-22; each says why, and his answer wins.

| # | Decision | Options | Recommendation | Why |
|---|---|---|---|---|
| D1 | Does a tiny task go through every stage? ***decided 2026-10-24: (b) Auto, with a triage*** | (a) every task, every stage; (b) **Auto**: a triage picks Quick, Light or Full, and the person can override per task and per project; (c) the person always picks | **(b)**, with Auto's routing set at the measured break-even size (§4.7) once the sample allows (§1) | Your premise for splitting the study was "primeiro ser mais rápido e para segundo ser mais eficiente". E4–E6 put that in doubt on today's tasks: one delegating session cost 3.6× the same task without subagents, a sibling started cold 13 times out of 14, and on small tasks preparation cannot pay itself back (§6). Full is projected at 5–15× a small task. Auto keeps preparation for the tasks where it can pay; the numbers are in front of you, not decided for you |
| D2 | Per-stage models *(undecided; S1 chose one model per session)* | (a) one model per project for every stage; (b) a "preparation model" for study and verify; (c) a model per stage | **(a) now**; (b) as an eval arm after P17 with `--price-model`, shipped only if M3 holds and M1 drops ≥ 20% on two runs | The cached prefix is per model: a different study model cannot share the planner's and implementer's cache (T2). Only deepseek-flash has been measured |
| D3 | Existing folders *(undecided)* | (a) each open or recent folder becomes a one-folder project, once; (b) start empty and offer an import; (c) one project holding every folder | **(a)**. P6b implements it but keeps the old keys until P20, so (b) or (c) can still be chosen | Nothing is lost or moved (R4); with D29(a), a folder can later join a bigger project without leaving its own |
| D4 | Existing chats *(undecided)* | (a) read-only history under the migrated project, where "Continue here" starts a project-chat session that links to the old file; (b) continuable in place; (c) copied into the app home | **(a)** | (b) keeps the folder slot this plan removes; (c) duplicates files that then drift |
| D5 | The checklist against "tasks" *(undecided)* | (a) keep `tasks_set`/`tasks_update`/`<tasks>` as the model sees them, call them **Steps** in the UI, name the card tools `task_*`, and never put both in one profile; (b) rename to `steps_*`; (c) remove the checklist | **(a)** | No byte-pinned prompt or schema moves (`agent/prompt.rs:1007-1047`), the `TodoWrite` hook aliases keep working (`agent/hooks/matcher.rs:39-43`), and no eval is needed. The collision is gone where it matters: inside one prompt |
| D6 | Golden goals *(undecided)* | (a) keep them: `<goal>` in a task prompt and the plan's acceptance criteria become the implementer's golden goals, and a `<goal>` in the project chat goes into the task it creates; (b) retire them; (c) only from the plan | **(a)**, with their rules carried in the head of task runs (D23) | They are the mechanical finish line (`agent/tools/tasks.rs:340`) that holds a fresh implementer to the plan; the model still cannot mint them |
| D7 | The living-document tools *(undecided)* | (a) gone from the project chat in P9 and from Standard in P21, once Full preparation grades doc-parallel, with `document.rs` kept as the study/plan format and its reader kept as `DocumentReader`; (b) keep them in every chat; (c) remove them now | **(a)** | The study *is* the doc-parallel shape, and you suggested reusing the reader ("pode até reaproveitar a interface"). Removing the tools now would break the eval's oracle; P21 frees ≈247 prefix tokens |
| D8 | Permissions for background and auto-implemented tasks *(undecided)* | (a) **Ask**: preparation refuses instead of asking, and implementations ask through the inbox, with the person's own YOLO applying as it does in a chat; (b) **WorktreeAutonomy**, only with worktrees: edits and read-only bash inside the worktree approved, everything else asks, the harness commits, nothing merges; (c) auto-implement inherits the person's YOLO and nothing else asks | **(a) by default; (b) offered with worktrees; (c) is what (a) already does for a person who turned YOLO on.** In every case task runs take permission settings only from the person and the project, `auto_approve_git` is never set, and the `task_play` card (and `task_create` while auto-implement is on) always asks | You framed auto-implement "para um usuário que é mais YOLO mode": that person keeps their YOLO in task runs, minus the cards that start work. Under (b) the blast radius is a branch nothing merges by itself (binding 3). `AUTO_GIT` includes push (`agent/permissions.rs:203`) |
| D9 | The implementer: fresh or fork *(departs from your words if (b))* | (a) **fresh**, the literal "novo agente que vai aplicar o plano inteiro"; (b) fork: a new session carrying the planner's transcript, its loaded tools and its real reads | **(a) stays the default.** P14 publishes both arms paired (M1, M3, M12, M7, M7e), and you decide whether (b) replaces it | Fork attacks the Brain → Builder re-exploration (E2), but it departs from "novo agente", so a metric does not get to make it the default. VERIFY stays fresh either way |
| D10 | What the project chat may do ***decided 2026-10-24: (a) one-step edits plus the task tools*** | (a) Standard minus the checklist and document tools, plus the card tools: one-step edits allowed (lease-checked), implementation only through `task_play`'s card; (b) read-only plus the card tools, so every write goes through a task; (c) full Standard plus the card tools; (d) the literal reading: when asked, the chat implements the task itself, in its own context; (e) like (a), but an explicit request in the person's latest message ("implement #7") counts as the ▶, with no card | **(a)** | You said the chat "pode até implementar a tarefa ... se assim for pedido". (a) keeps that through a one-click card that is the ▶ (binding 1) and keeps the task's own chat as the record of its implementation. (d) puts implementation transcripts in the project chat and skips the task's lease. (e) saves a click but lets a model's reading of a sentence start work. (b) makes every typo a card. (c) puts checklist and card tools in one prompt |
| D11 | Where worktrees live | (a) `<data_dir>/claudinio-code/worktrees/…`, short on Windows; (b) `~/.claudinio/worktrees`; (c) next to the repository | **(a)** | Outside every repository and beside the app's other bulky data (`dirs::data_dir`, as `llama/provision.rs:108` uses). Worktree runs have no watcher, so the location matters for disk and tidiness only. (c) clutters the person's folders |
| D12 | Dependencies in a worktree | (a) install per worktree, by an approved setup command; (b) share read-only: symlink, and refuse package-manager installs in that worktree; (c) share writable | **(a)**; (b) as a per-project opt-in if install time or disk hurts; **never (c)** | (c) lets one task's `npm install` change the main folder and every sibling. (b) can break tools that write caches into `node_modules`. The setup command spares every parallel task an install approval |
| D13 | Pay for eval runs on claudin.io and on one Anthropic-direct provider | yes / no | **Yes, at P11, P14, P15 and P17** | Every number so far is one model through one endpoint (`eval/README.md:267-274`). T4 can only be measured on Anthropic-direct, and warm starts and cold costs differ most there |
| D14 | Concurrency defaults and priority | requests 6 (1–32); agents per stage 4; running implementations 4 (1–20); classes interactive, implementation, preparation | **As stated**, adjustable in Settings; `min(2, limit − 1)` permits reserved for interactive requests, the rest round-robin with aging, so no class starves; local engines use the server's slots | You asked for "1, 2, 10, 3, 20 agentes rodando em paralelo": 20 is the maximum. The default is 4 because 20 tasks × 4 agents is 80 streams against one provider's limits and the person's plan budget, and because a worktree task on a repository like this one costs GBs and minutes of compile (P18 measures it). Twenty is tested only against a stub (P18) |
| D15 | Where the task list lives | (a) its own column between projects and the open task or chat; (b) under the project in the sidebar | **(a)** from P10, with the project column collapsible; a tab beside the chat until then | The list is the product's centre; with the right aside gone it fits 900 px with the chat at ≥ 400 px (P10's width test) |
| D16 | Who commits, and how merges happen | (a) the harness commits at the end of the run, **with the repository's commit hooks**; a refused commit fails the task with the hook's output and a Retry; `merge --no-ff` in the folder, refused when dirty, asked when not on the base branch; a conflict aborts and is resolved in the task chat; (b) the same with `--no-verify`; (c) squash | **(a)** | Deterministic, with no model rounds and no git approvals; keeps the task's commits; a repository's hooks are its own rules, and a failure is visible rather than skipped |
| D17 | How an implementation without worktrees ends | (a) the changes stay uncommitted, with Changes and Commit & push offered; (b) a commit on the current branch | **(a)** | Binding 3 covers worktrees only; a commit on the person's own branch is theirs to make |
| D18 | Planning, verification and fix shape *(departs from your words)* | the planners: one, 2–4 in parallel by area, or 2–4 **sequential** planning stages; the verifiers: split by check group always, or only for large plans; the checks: fixed V1–V6 or per project; the cycles: one or a loop; the fixers: one per correction or one per step | **A deliberate first step:** one planner; 2 verifiers by check group when the plan touches ≥ 8 files, else 1; fixed V1–V6; one verify → fix cycle; **one fixer per correction, as you said**, with fixers on the same step run in sequence | You said "o plano você pode quebrar em duas, três, quatro etapas" and "a verificação é igual". "Etapas" may mean parallel planners or successive planning stages. Parallel writers without shared context are inconsistent (E11), and a loop is unbounded cost, so one planner is measured first. Eval arms for 2–4 planners (either reading) and always-split verifiers follow P17, and you choose with the numbers |
| D19 | Where a task may write in a multi-folder project | (a) only the folders it holds: its plan's Changes, or the folders a Quick prompt names, or the primary; one worktree per git folder it holds; adding a folder re-queues it; (b) exactly one folder per task, reading all of them; (c) any project folder | **(a)** | The vision creates worktrees "from the folders". (c) lets an implementer write a folder with no lease and no worktree whenever a plan is incomplete, which breaks bindings 3 and 4 |
| D20 | Where the project instruction goes *(departs from your words)* | (a) the first user turn of each new session, with no cap; an edit reaches a continued session once, as a `<project-instruction-updated>` block in its next turn; (b) the system prompt, as "uma espécie de sistema do projeto"; (c) (a) with a cap, e.g. 6,000 characters | **(a)** | It still acts as the project's system: it opens every session's first message. In the system prompt, every edit would cold-start every task in the project, and the prefix has 187 tokens of room. No cap, by your standing instruction; the dialog shows its size |
| D21 | Release cadence (follows D24) | release after each visible phase, or in milestones | Under D24(b): **0.9.1 after P1a, 0.10.0 after P17, 0.11.0 after P12, 0.12.0 after P18**. Under D24(a): 0.10.0 after P9, 0.11.0 after P14, 0.12.0 after P18. Patches between | Merges and releases are pre-authorized once CI is green. Under either order, projects hold **one folder** until P12, and under (a) 0.10.0 has no preparation at all |
| D22 | After an app restart | (a) Stopped with a one-click Resume and no automatic spend; (b) resume automatically | **(a)** | No money is spent without the person; auto-implement fires only on live transitions |
| D23 | Where the golden rules ride in task runs | (a) send `GOLDEN_PROMPT` in every session of a task, stages included; (b) build every task session's system prompt without it and put the same text in the implementer's L6; (c) do not turn acceptance criteria into golden goals | **(b)** | A prepared implementer always has golden goals, so without this its system prompt differs from every stage's (§2.5), the fork arm's copied transcript is billed uncached, and T4's L4 breakpoint misses. (a) tells read-only stages about goals they cannot close and splits the project's cache in two. (c) loses the finish line D6 keeps |
| D24 | Sequencing and what each release contains ***decided 2026-10-24: (b) preparation right after the shell*** | (a) numeric order: the shell, then the right column and multi-folder (10–12), then preparation (13–17), then worktrees; (b) **preparation first**: 0–9, then 13–17, then 10–12, then 18–21; (c) a thin end-to-end pipeline (scope, study fan-out, plan, fresh verify, fix, report at one depth) in one phase right after P9, deepened afterwards | **(b)** | You called preparation "a parte principal". Under (a), 0.10.0 ships task cards with no preparation and 0.11.0 ships Light, which is not the pipeline you described. (b) puts the whole pipeline in the first release that has projects, with each stage still behind its own red tests and gate; (c) is faster to a demo but merges five stages in one PR, against the one-PR-per-phase rule. The cost of (b): multi-folder projects and the three-column layout arrive later |
| D25 | The default depth for a new task before D1 is answered | (a) Quick (today's path); (b) **prepare by default, as you described**: Light from P13b, Full from P17; (c) Auto as soon as P15 lands | **(b)**, with Quick one menu choice away and the price on every card | You said that when the person says create, "ele vai para a primeira parte, que é o planejamento". Making every task unprepared until you rule would bring back the "chat manual" you called outdated. The price is real and shown (§6) |
| D26 | Does verify stay in Full if it does not raise M3? | (a) **yes, always**: a new context checks every Full preparation; (b) Auto may skip it, with a per-task "Verify" toggle; (c) off by default | **(a)** until you decide otherwise; P17 reports M3 and M9 with and without it | Verify in "uma nova sessão, um novo contexto" is part of the preparation you specified. A 6-to-12-task eval should not remove it from the product |
| D27 | Output and round caps on preparation | (a) **no caps**: no `max_tokens` of our own on any stage or triage call, sizes as guidance in L5, rounds and lengths reported; (b) soft caps: a long artefact is flagged, never cut or resent; (c) hard caps: plan ≤ 3,000 tokens and 12 steps with a resend, round caps per stage, ≤ 3 uncited reads per verifier | **(a)** | Your standing instruction: "não é pra colocar limites nas coisas, deixa a llm pensar à vontade", and ask before adding one. A hard cap on a plan that must carry the LLD you asked for would cut it on exactly the large tasks preparation is for. On a reasoning model, a small `max_tokens` is eaten by reasoning and returns empty content, a failure this project has already met. A cap on uncited reads would tie the "independent" verifier to the study's citations |
| D28 | The askpass timeout | (a) keep 300 s (`askpass.rs:24-25`); the card says "credential prompt expired: retry"; (b) remove it for inbox-listed prompts; (c) a longer timeout | **(a)** | A prompt holds a live git or ssh child; task runs never push on their own (Open PR is a click), so an unattended prompt is rare. Tested either way (P5) |
| D29 | May a folder belong to several projects? | (a) yes, with leases keyed per folder across projects; (b) no, exclusive | **(a)** | Nothing in your words forbids it, the mocks need it, and after the migration every folder is already in a one-folder project, so (b) would refuse every bigger project. Leases are app-wide (§4.3.6), so one folder still has one writer |
| D30 | Do finished tasks become project context? | (a) no: the chat calls `task_get` when it needs one; (b) the project chat's `<project-tasks>` block lists each Done task with the one-line "what changed" of its report; (c) reports indexed for search by stages and the chat | **(b) for the project chat; (a) for preparation** | You said a task "vai usar o que está no projeto, o contexto do projeto". The chat benefits from knowing what was done; a study should read the code, which is the truth, while old reports go stale. (b) costs about 15–25 tokens per Done task in each chat turn, outside the cached prefix; which Done tasks to list (all, or recent) is part of the choice |
| D31 | Withheld: decided with the Phase 1a security fix, under `SECURITY.md` | — | — | — |
| D32 | One list or several per project ("várias listinhas de tarefas") | (a) one ordered list per project; (b) several named lists per project; (c) each task is itself a small list (its Steps) | **(a) now**; (b) later if wanted: `order.json` becomes `lists/<name>.json` | "Várias listinhas" may mean several lists per project or the many small lists of steps. One list keeps one priority order, which is what ▶ and the queue read |

## Appendix A: Inventory of files touched

Checked on 2026-10-22 at `47ba241`. "(new)" marks files this plan creates.

**Rust, existing**
- `state.rs`: `:45-88` (`WorkspaceState`), `:57` (`active_session`; P2 makes the swap conditional, P20 removes it), `:146-216` (`AppState`: hub P2, gate P4, inbox P5, projects P6a).
- `lib.rs`: `:39` (handler list; most phases); `:188-209` and `:236-254` are the layering and size guards, which constrain every phase and are not changed.
- `commands/agent.rs`:
  - `send_message` `:150-351`: P2 (run start moves, `:246-265`), P20.
  - goals `:299-321`: reused in P14.
  - `build_hook_ctx`, `transition_maps` `:359-395`: moved into core in P2.
  - `spawn_run_loop` `:417-498` (`:491-495`, live state cleared before `Done`): P2.
  - `new_session`, `list_sessions`, `load_session` `:503-643` (`:571-636`, `:578`): P6a, P8, P20.
  - approvals `:646-700`: P5.
  - steering and `compact_session` `:979-1106`: P8, P20.
  - `interrupt_session` `:1151-1169`: P5.
  - `commit_and_push` `:1176-1294` (`:1201-1202`, `:1244`): P2, P6a, and the template for P8.
- `commands/tasks.rs:1-65`: P20. `commands/document.rs` (`:18`): P6a, P20. `commands/git.rs:25`: P18. `commands/fs.rs:188` (**sensitive**): P12, P18; `:272-310`: P13b. `commands/hooks.rs:262-288`: P8. `commands/code_intel.rs`: P6b. `commands/local_llm.rs:712`: P4. `commands/enhance.rs:138, 306`: gated through `one_shot` in P4, unchanged.
- `agent/run_loop.rs`:
  - `:87-99`: signature unchanged.
  - `:188-229`: P6a (instruction), P11 (W layer), P9 (recital, profile).
  - `:249-251`: the golden flag, read by P13a.
  - `:648`: P4.
  - `:1929`: the pin replaced in P11.
- `agent/tool_exec.rs`:
  - `:29-86`: P8 (lease), P9 (card exemptions at `:45, 74-79`), P13a (stage mask), P19 (policy).
  - `:325-347, 448-471, 579-614, 624-647, 746-769, 885-897`: P5.
- `agent/subagent.rs`:
  - `:12-28`: read by P4.
  - `:534-556` (`:544`): P11.
  - `:681`: shared interrupt, reused in P5.
  - `:716-746`: P4.
  - `:1232`: the template for the harness-driven stages.
  - `:1541, 1658`: `scripted_model`, used by the byte tests.
- `agent/retry.rs:16-18, 28-35, 56-110`: P4.
- `agent/provider.rs` (1,843 production lines; every phase adds at most 20): `:20-26, 1450-1451` (budget marker): P4; `:418` (`max_output_for`): read by P13b and P15; `:1136-1214` (judge), `:1219-1260` (`one_shot`), `:1311-1460` (`stream_message`): P4.
- `agent/provider/openai.rs:379-400, 708-760`: P4. `agent/provider/cache.rs:50-75, 154-174`: P15. `agent/provider/http_error.rs`: P4. `agent/events.rs:96-97`: P4.
- `agent/persist.rs`: `:8-10` (unknown kinds skipped); `:382-400` (`Usage.stage`): P0; `:647-718` (`create_in`, `fork_session`): P6a, P14; `:1193-1230` (`chain_cumulative_cost`), `:1263` (`list_sessions`): P6a; `:3275` (union guard): P13b, P14, P17.
- `agent/prompt.rs`:
  - `:66-127`: P11; `:116-126` (golden rules): P13a.
  - `:137-178` (`run_prefix`): P9, P13a.
  - `:813`: byte stability, kept.
  - `:1007-1047`: core list; P21 flips it. `:1056-1078`: deleted in P21.
  - `:1316-1356` (`:1344-1345`): PLAN pins; P21.
  - `:1428-1442, 1526-1532`: budgets; P9 adds ProjectChat's.
  - `:1581`: the chat's golden-prompt test, kept green in P13a.
- `agent/prompt/standard.rs:36-46, 73-76`: read (§2.5); `:38-41`: P21. `agent/prompt/project.rs` (new): P9.
- `agent/session.rs`: `:46` (`parse_goals`, reused); `:152-177` (`ProjectChat`): P9; `:196-200` (`SteeringCtl`): P5.
- `agent/tools/mod.rs`:
  - `:32-98, 143-238, 955-967`: P12.
  - `:316, 329, 352, 389, 400, 413`: tool schemas, unchanged (P11).
  - `:592-640`: checklist definitions, unchanged (D5).
  - `:736, 1055`: where reads are recorded, read by P14.
  - `:1269-1334`: P14 (tracker rebuild), P16.
- `agent/tools/doc.rs:15-113`: P21. `agent/tools/tasks.rs:340, 507`: reused in P14. `agent/tools/bash.rs:143-148, 196-204`: P5. `agent/tools/quality.rs`: P12.
- `agent/transition.rs`: `:58-78, 121`: P6a; `:140-159`: P2, P20; `:229`: read by P14.
- `agent/project_memory.rs:25`: P18; `:569-600`: P6a. `agent/task_recital.rs:34`: P9, P14. `agent/gate_text.rs`: P8, P13a. `agent/surface.rs:152, 232-247`: P9, P13a, P15. `agent/deferred.rs`: P9, P14, P21. `agent/doc_session.rs`: `:289-312` and `:374` (reused); `:790-807, 960-972`: P6a. `agent/document.rs:538-579` (reused). `agent/delegation.rs:21-26, 44` (reused, and read by P0's M7). `agent/notes.rs`: P6a. `agent/judge.rs:56-61`: gated through P4. `agent/hooks/runner.rs:172-178`, `agent/hooks/trust.rs:120-126`: P18. `agent/hooks/matcher.rs:39-43`: unchanged (D5). `agent/jev.rs:39, 88, 240-248`: P15 triage.
- `agent/permissions.rs:147-200, 525-528` (**sensitive**): read by P13a; `:203`: read by P8, P19. `workspace_path.rs:12-70` (**sensitive**): P12, P13a. `askpass.rs:24-81` and its helper script: P5. `quality/evidence.rs:25`: P12, P18. `llama/mod.rs:254-256, 302-304`: P4. `llama/provision.rs:108`: read (D11).
- `eval/metrics.rs:15-38, 49-89, 91-200`: P0. `eval/runner.rs:43-73, 266, 298-345, 355-383, 307-430`: P0, P1, P8, P9, P12, P13b–P17. `eval/mod.rs:221-256, 262-282`: P0, P1. `eval/task.rs:41-61`: P1, P9, P12. `eval/materialize.rs`: P12. `eval/policy.rs:56-66, 82`: P13a (moved to `workspace_path.rs`, called from here).

**Rust, new:** `agent/provider/workspace_config.rs` (P19); `agent/runs.rs`, `commands/runs.rs` (P2); `agent/provider_gate.rs` (P4); `agent/inbox.rs` (P5); `agent/project/{mod,store,migrate}.rs`, `commands/projects.rs` (P6a); `agent/project/{task,task_store,actor}.rs`, `commands/project_tasks.rs` (P7); `agent/project/{scheduler,run_ctx}.rs` (P8); `agent/tools/project_tasks.rs`, `agent/prompt/project.rs` (P9); `agent/stage.rs`, `agent/prepare/{mod,head}.rs` (P13a); `agent/prepare/{scope,plan,check,report}.rs` (P13b); `agent/prepare/kickoff.rs` (P14); `agent/prepare/{study,freeze,fanout,triage}.rs` (P15); `agent/prepare/{verify,fix}.rs` (P17); `gitcmd.rs`, `agent/worktree.rs` (P18); `agent/task_policy.rs` (P19).

**Frontend, existing**
- `src/App.tsx`:
  - `:33-60`: P6b (read), P20 (deleted); `:96-121, 314-323, 618-705, 954-1237`: P6b.
  - `:102-182, 1259-1290`: P10.
  - `:150`: the YOLO blacklist setting, read (§2.3).
  - `:198-203`: P3, P8.
  - `:271-279`: P5.
  - `:1249-1256`: P3.
- `src/App.test.ts`: `:41-153`: green in P6b, deleted in P20; `:155-183`: P10.
- `src/components/ChatPanel.tsx`:
  - `:127-288, 795-1132, 1291-1365`: P3.
  - `:215-216, 1452-1467, 1586-1594, 1646-1675`: P10.
- `src/components/ChatPanel.test.ts`: `:331-353` (P3 adds a pin; P10 inverts the Documents pin).
- `src/components/DocumentPanel.tsx` and its test: the reader extracted in P13c, the panel removed in P10.
- `src/lib/ipc.ts`: `:22-31` (P12); `:489-504` (P2, P8, P20); `:544` (P13b, P14, P17).
- `src/lib/ipc.test.ts:426-469`: P20.
- `src/lib/workspaceStatus.ts:5-28`: P3.
- `src/lib/chatRecords.ts`: P13c. `src/lib/fileIndex.ts:4`: P12. `src/lib/documentStore.ts` and its test: P10.
- `src/components/chat/TimelineRows.tsx`: `:921` (`SubagentRow` shape, reused in P15); `:1429-1440` (P5).
- `src/components/TasksPanel.tsx` with its tests: removed in P10 (`TasksPanel.tsx:22-25, 108, 125-137`; `TasksPanel.test.tsx:147-153`).
- `src/components/AskpassModal.tsx`: P5. `src/components/EmptyState.tsx`: P6b. `src/components/Icon.tsx:4-258` (`:254-257` reused): P7. `src/components/settings/SettingsModels.tsx:85-112`: P4. `src/components/tool-renderers/toolPresentation.ts`: P9. `src/components/GitChangesModal.tsx`: P18. `src/components/ProseContent.tsx`, `src/lib/markdown.ts`: reused.

**Frontend, new:** `src/lib/runReducer.ts`, `runStore.ts` (P3); `src/components/Inbox.tsx` (P5); `src/lib/projects.ts`, `src/components/{ProjectList,ProjectDialog}.tsx` (P6b); `src/lib/taskStore.ts`, `src/lib/layout.ts`, `src/components/{TaskList,TaskView}.tsx` (P7); `src/components/StepsStrip.tsx`, `LegacyDocuments.tsx` (P10); `src/components/DocumentReader.tsx` (P13c). Each comes with its tests.

**Eval, scripts, CI, docs:** `eval/tasks/large-*.yaml`, `eval/groups/`, `eval/tasks/chat-*.yaml`, `eval/tasks/large-two-folder-*.yaml`, `eval/baselines/2026-10-2x-*` (new); `eval/tasks/doc-parallel-job-runner.yaml:202, 233-243` (read: the oracle's parser and its any-document rule); `eval/README.md`; `scripts/session-stats.py`, `scripts/test_session_stats.py` (P0); `.github/workflows/ci.yml` (`:23, 45, 105` read; a Windows job in P18); `README.md`, `README.pt-BR.md`, `docs/ARCHITECTURE.md`, `CONTRIBUTING.md` (P21). Outside the repository and read only: tauri 2.11.5 `ipc/channel.rs:134-180` (§2.1). This plan gets an Implementation Log, appended as each phase lands.
