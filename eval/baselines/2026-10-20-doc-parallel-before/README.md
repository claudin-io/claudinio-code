# doc-parallel tasks before the single-timeline plan (2026-10-20)

Phase 0 of `docs/plans/2026-10-20_single-timeline-and-one-plan.md`: the two
`doc-parallel` tasks, run twice each on the binary built from `main` before
any phase of that plan (`/srv/claudinio-eval/bin/claudinio-eval-final`, built
at the commit equivalent to `0e4b34c`, the code of 0.8.0 `bac6206`). Same model,
prices, sandbox and eval policy as `../2026-10-19-deepseek-flash-after`:
`deepseek-flash` through `https://api.deepseek.com/anthropic`, prompt v2,
`--price-in 0.15 --price-out 0.6 --price-cache-read 0.003 --price-cache-write 0.15`,
`--task-timeout-s 1800`, a transient systemd unit with `/root` hidden.

That binary predates M4 and M6, so the two numbers were computed after the run
from the persisted sessions, with `scripts/session-stats.py --meters` (the same
rule the runner now applies): `meters.jsonl`, one line per run and task. The
results the binary wrote are `run-a.jsonl` and `run-b.jsonl`, unedited.

## The numbers

| | run a | run b |
|---|---|---|
| M1 cost per solved task | $0.0085 | $0.0219 |
| M2 median rounds per task | 15.5 | 35 |
| M3 pass rate | 2/2 | 2/2 |
| M4 tool-call error rate, main | 4.0% (2/50) | 1.8% (1/56) |
| M4 tool-call error rate, subagents | n/a (0/0) | 1.7% (1/59) |
| M4 errors by tool | `edit_file` 1, `finalize_plan` 1 | `edit_file` 1, `read_file` 1 |
| **M6 sections written by subagents** | **0% (0 of 7)** | **29% (2 of 7)** |
| subagents spawned | 0 | 4 (one `spawn_agents` call) |
| total cost | $0.0171 | $0.0437 |
| subagent share of tokens | 0% | 38.9% |
| wall-clock (both tasks) | 122 s | 167 s |

Per task (pass, rounds main + subagent, cost, sections by owner at the end):

| task | run a | run b |
|---|---|---|
| `doc-parallel-job-runner` (4 questions) | P, 13 + 0, $0.0102, main 4 | P, 20 + 35, $0.0367, sub 2 / main 2 |
| `doc-parallel-sync-tool` (3 questions) | P, 18 + 0, $0.0068, main 3 | P, 15 + 0, $0.0070, main 3 |

## What the sessions did

- **`doc_create` was called in all four sessions**, before the first section
  was researched, with exactly one section per question (4 and 3). The model
  routes a "write me a document" request to the living document today.
- **Delegation happened once in four.** Three sessions read every file in the
  main context and wrote every section themselves (M6 = 0). The fourth
  (run b, job-runner) gave all four sections to explore subagents in one
  `spawn_agents` call, which is the shape the plan wants; every subagent
  started and wrote its section (no `"section": ""` failure, F1, in this
  baseline).
- **Then main redid the subagents' work (F4).** In that same session, after
  the subagents finished, main re-read the five source files, `doc_read` all
  four sections, and rewrote s2 and s3 with `doc_write`, taking their
  ownership: at the end of the task 2 of 4 sections are the subagents', not 4.
  That one task cost $0.037 and 55 rounds, 3.6x the same task done without
  subagents in run a ($0.010, 13 rounds). Delegation as it runs today costs
  more and keeps less.
- **Planning surfaces stacked (2.1).** Both `doc-parallel-sync-tool` sessions
  kept a `tasks_set` checklist next to the document, and run a also called
  `finalize_plan`, which failed (`no plan file found`): three planning
  surfaces in one research session.
- **M4 errors.** Two are the model calling `edit_file` on the living
  document's file (refused: "write a section with doc_write"), one per run on
  the job-runner task — a failure the plan's §2.5 table does not list. The
  others are `finalize_plan` without a plan file (2.4) and a subagent
  `read_file` of `/tmp/probe.mjs`, outside the workspace.

## What this does not show

- **Two runs of two tasks.** M6 moved from 0% to 29% between runs only
  because one session chose to delegate; one session decides the number. The
  gate for later phases should read M6 per task and over several runs.
- **M3 cannot fall on these tasks for the reason the plan worries about.** Both
  oracles grade the document's content and one section per question, and every
  run passed with main writing all of it. The tasks measure *who* answers
  (M6) and at what cost (M1), not whether the answer is reachable.
- **Small fixtures.** Each question is one ~30-line file; on a fixture this
  size reading everything in the main context is cheap, which is why the
  no-delegation runs are the cheapest. The parallel shape pays off when the
  sections' reading does not fit comfortably in one context.
