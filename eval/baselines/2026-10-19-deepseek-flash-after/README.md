# After every lean-harness phase (2026-10-19)

Two runs of the same 25 tasks, with the same model, prices, sandbox and eval policy as `../2026-10-18-deepseek-flash-before` and `../2026-10-18-deepseek-flash-mid`. The binary was built at `31e39f9`, after Phases 8, 0, 1, 7, 2, 0f, 3, 4, 4b, 4f, 5, 3b, 6, 6f and 9 and before the final whole-branch review fixes. The prompt is v2, the default.

| | before a | before b | mid a | mid b | after a | after b |
|---|---|---|---|---|---|---|
| M1 cost per solved task | $0.0338 | $0.0305 | $0.0246 | $0.0179 | **$0.0060** | **$0.0055** |
| M2 median rounds per task | 50 | 45 | 20 | 21 | **12** | **12** |
| M3 pass rate | 23/25 | 24/25 | 19/25 | 23/25 | **25/25** | **25/25** |
| total cost | $0.777 | $0.733 | $0.468 | $0.412 | $0.151 | $0.139 |
| tokens sent (input + cache) | 28.5M | 30.6M | 15.9M | 14.5M | 5.5M | 5.7M |
| tool calls | 1,940 | 1,870 | 1,023 | 950 | 520 | 511 |
| subagents spawned | 82 | 75 | 10 | 9 | 0 | 0 |
| median wall-clock per task | 164s | 89s | 51s | 44s | 40s | 31s |
| total wall-clock | 138 min | 152 min | 79 min | 64 min | 41 min | 35 min |

Taking the mean of the two runs at each end:

- **M1:** cost per solved task fell from $0.032 to $0.0058 (−82%).
- **M2:** median rounds fell from 47.5 to 12 (−75%).
- **Tokens:** tokens sent fell by 81%.
- **M3:** the pass rate went from 47/50 to 50/50.

## What moved it

- **Mid → after: file creation (Phase 3b) took M3 to 25/25.**
  - Every earlier failure was a file the model could not create. Now `edit_file` with an empty `old_string` creates the file and any missing folders.
  - Tasks that used to need 50–200 rounds of workarounds now finish in 7–40. For example, `py-retry-backoff` went from 82–124 rounds to 7–9, and `hist-jev-settings` from 170–212 to 34–39.
- **Before → mid: the main session edits (Phase 3), there is no task ritual (Phase 4), and the prefix is ~5k tokens (Phase 2).**
  - These roughly halved cost and rounds.
  - Subagents fell from ~80 per run to ~10 at mid and 0 after. On these task sizes the model now does the work itself.
- **Prompt v2 (Phase 5), masking-first context (Phase 6) and project memory (Phase 9) are in the after binary.**
  - Their own gates need dedicated runs, which were not done: a v1-vs-v2 comparison, a long-task subset with Jev off, and a two-task memory pair.
  - No run here compacted or pruned, so the context phases were not exercised.

## What this does not show

- **Small tasks only.**
  - The tasks are small: 18 fixtures plus 7 history tasks from this repository.
  - No task here is long enough to compact, prune or hand off. The DeepSeek thinking-mode fix (0f) and the masking ladder (6) are covered by tests, not by this eval.
  - Terminal-Bench and SWE-bench were not run. `import-swebench` exists, but no instance set was imported.
- **One provider.**
  - Every run uses one model, `deepseek-flash`.
  - Its implicit prefix cache already serves ~95% of input from cache, so Phase 1's `cache_control` markers do not show here. They matter on Anthropic-direct providers.
- **No subagents or living document.**
  - With 0 subagents in the after runs, the code-subagent brief (Phase 3), parallel subagent reads (Phase 7) and section delegation (Phase 4b) were not exercised.
  - The living document is used when a task has several parts, and no task here asks for one.
- **Two runs per arm.** Run-to-run noise at this size is about ±15% on M1 (before a vs b: $0.0338 vs $0.0305).
