# Baseline before the lean-harness phases (2026-10-18)

Two runs of the 25 tasks in `eval/tasks/`. The binary is `examples/claudinio-eval.rs` built at the commit after Phase 8 (a pure refactor) and Phase 0 (the eval itself), plus the workspace confinement of the eval policy (see "Contamination" below). The harness behaves as 0.7.2 does.

| | run a | run b |
|---|---|---|
| M1 cost per solved task | $0.0338 | $0.0305 |
| M2 median rounds per task (main + subagents) | 50 | 45 |
| M3 pass rate | 23/25 | 24/25 |
| total cost | $0.7767 | $0.7327 |
| cache-read share of input | 95.5% | 95.9% |
| subagent share of tokens | 49.9% | 50.8% |

The per-task numbers are in `table.md`, and the raw records in `run-a.jsonl` and `run-b.jsonl`.

## Settings

- Model: `deepseek-flash`, via `https://api.deepseek.com/anthropic` with the Anthropic protocol.
- Prices per Mtok: in $0.15, out $0.60, cache read $0.003, cache write $0.15.
- Limits: `--task-timeout-s 1800`, with each task's own `max_rounds`.
- Eval approval policy: `edit_file` is approved outside `.git/` and `.claudinio*`. bash is approved only when the command is read-only (`permissions::bash_is_read_only`) and every path stays inside the task workspace.
- Sandbox: `systemd-run`, as described in `eval/README.md`.

## What the runs showed

- **Every failure is one harness bug: `edit_file` cannot create a file.**
  - `read_file` on a missing path errors. The read-before-edit gate then refuses the edit, and `edit_file::preview` reads the file anyway.
  - It caused all three failures: doc-architecture-notes (a), py-extract-validation (a) and py-retry-backoff (b).
  - The passing py-extract-validation (b) got there by editing `handlers.py` so that importing it wrote `validators.py`. That took 117 rounds.
  - Phase 3b fixes it. The after-run's M3 therefore partly measures that fix.
- **The shed ladder breaks DeepSeek's thinking mode.**
  - Dropping a call removes its `tool_use` block from an earlier assistant message, and DeepSeek then answers HTTP 400 "content[].thinking in the thinking mode must be passed back to the API".
  - This was proven by replaying the history: unpruned gives 200, truncation only gives 200, one dropped call gives 400.
  - Fixed in Phase 0f: a dropped call keeps its `tool_use` block with an empty input.
- **Prompt caching:** DeepSeek's implicit prefix cache already reads ~96% of input from cache. Phase 1's `cache_control` markers therefore cannot move M1 on this provider. They matter on Anthropic-direct providers.

## Contamination (first attempt, discarded)

The first baseline (M3 23/25 in both runs) was thrown away. The policy approved read-only shell commands anywhere on disk, and agents read the reference solutions outside their workspace. The eval policy now refuses any read-only command that names a path outside the task workspace (`eval/policy.rs`, `stays_in_workspace`).
