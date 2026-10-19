# Mid-point eval after Phases 8, 0, 1, 7, 2, 0f, 3, 4 and 4b (2026-10-18)

Two runs of the same 25 tasks, with the same model, prices, sandbox and eval policy as `../2026-10-18-deepseek-flash-before`. The binary was built at `72a8324`, before Phases 4f, 5, 3b, 6 and 9.

| | before a | before b | mid a | mid b |
|---|---|---|---|---|
| M1 cost per solved task | $0.0338 | $0.0305 | $0.0246 | $0.0179 |
| M2 median rounds per task (main + subagents) | 50 | 45 | 20 | 21 |
| M3 pass rate | 23/25 | 24/25 | 19/25 | 23/25 |
| total cost | $0.7767 | $0.7327 | $0.4677 | $0.4118 |
| input tokens | 28.5M | 30.6M | 15.9M | 14.5M |
| subagents spawned | 82 | 75 | 10 | 9 |
| median wall-clock per task | 164s | 89s | 51s | 44s |

## Reading it

- **Cost, rounds and tokens roughly halve.** The main session now edits directly (Phase 3), so ~80 subagents per run drop to ~10. The task ritual is gone (Phase 4), and the prefix is ~5k tokens instead of ~33k (Phase 2). On a task that passed in all four runs, the cost typically falls by 50–80%.
- **M3 falls in run a, and every failure has one cause: a new file that could not be created.**
  - Failures by run:
    - mid a: `docs/ARCHITECTURE.md`, `INCIDENT.md`, `src/lib/modeChoice.ts`, `src/components/settings/SettingsJev.tsx`, `src/lib/path.ts` and `validators.py`.
    - mid b: `INCIDENT.md` and `src/lib/modeChoice.ts`.
  - Why it fails: `edit_file` cannot create a file. Its read-before-edit gate needs a prior read of a path that does not exist yet, and the eval approves only read-only shell commands.
  - The baseline's code subagents sometimes worked around it, for example by editing a module so that importing it wrote the missing file. The direct-editing main session gives up sooner.
  - One agent tried `doc_create` with the title `../../INCIDENT` to get a file written. The title was slugified into `.claudinio/docs/`, as designed.
- Phase 3b fixes file creation. The after-run measures it.
- No run of the clean baseline or of this eval pruned its context: there is no `pruned` record in any session, and no `API error` either. So the DeepSeek thinking-mode 400 that 0f fixes was not exercised live here. It is covered by the regression test in `prune.rs` and by the replay recorded in the before README.
- Phase 2's deferred loading shows up as 15 `tools_loaded` and 12 `cache_break` records over the 50 runs. Phase 4b's document was linked in 11 sessions, mostly as an attempted way to write a file.
