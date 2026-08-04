# Quality Harness — mechanical verification of agent-written code

## Context

Claudinio Code has a harness for *coding* — tasks, golden goals, Brain/Builder
flips, the never-stall judge, anti-stall guards. It governs how work flows. It
has never had a harness for *quality*, and the gap was load-bearing:

`GOLDEN_PROMPT` told the model, in so many words, to "run the checks — build,
tests, coverage, whatever the goal requires" before marking a golden goal done.
Nothing checked that it did. A goal could be closed on the model's word alone,
which means the strongest guarantee the product offered was an assertion by the
thing being guaranteed.

The framing this work adopts: **a harness is what earns the developer the right
not to read the generated code.** That right cannot rest on the generator's
self-report. It rests on layers of independent checks, each catching a class of
defect the others miss:

| Layer | Question it answers | What is lost without it |
|---|---|---|
| Unit tests | Does this logic work? | Breaks on ordinary inputs and edge cases |
| Coverage | Was the code written actually exercised? | Ghost code and untested paths ship |
| Mutation | Do the tests catch real breakage, or are they decoration? | The model breaks the logic and the suite stays green |
| Gherkin / spec | Does it do what the business asked? | The worst failure: building the wrong thing perfectly |
| Metrics | Is the codebase improving or rotting? | Debt accrues silently at machine speed |

Phases 1–4 (tests, diff coverage, mutation, Gherkin) are implemented. Phase 5 is
the roadmap below.

## Principle: enforcement, not trust

Every rule here is deterministic Rust. No layer asks a model whether the code is
good.

Concretely: a `QualityReport` is only usable as evidence while its **digest**
still matches the worktree. The digest is a SHA-256 over `git HEAD`, the full
diff against it, and the identity of every untracked file. So the obvious
defeat — run the tests, then edit the code, then close the goal — does not work:
the edit invalidates the evidence and the gate asks for a fresh run.

The design deliberately rejected a "Verifier" *mode* (a third `SessionMode`, or
an LLM subagent that judges the work). A second model saying "trust me" adds
cost, not confidence. It would also have meant refactoring the Brain↔Builder
flip, which assumes exactly two modes.

The correction loop needed no invention either: because the gate blocks the
`done` transition, `end_turn` falls into the existing golden-pending path and
the GoldenFlip sends the model back to work. One loop, not two.

## Architecture

```
src-tauri/src/quality/          — no dependency on the command/IPC layer
  mod.rs        Layer, LayerStatus, LayerResult, GateVerdict, QualityReport,
                evaluate_gate(), run_layers()
  config.rs     QualityConfig — the "quality" object of .claudinio.json
  profile.rs    ProjectProfile / StackProfile — detection + user overrides
  runner.rs     long-command executor, run_tests(), run_coverage()
  evidence.rs   workspace_digest() — freshness
  diff.rs       changed_lines() since base_commit
  parsers/
    tests.rs    cargo test / vitest --reporter=json / jest --json → TestSummary
    coverage.rs lcov → diff coverage
```

Every parser is a pure function over text. That is the point: the trustworthy
part of the harness is the part with no I/O and no model in it, so it can be
tested exhaustively.

### Why not the bash tool

`agent/tools/bash.rs` truncates at 100 KiB and kills at 30 s — right for an
agent poking around, wrong for an instrumented coverage build. `quality/runner.rs`
streams full output to a log file **outside the workspace**, hands the model
only a bounded tail, and takes a per-layer timeout (tests 10 min, coverage
15 min, both configurable).

Command strings come from detection or from `.claudinio.json` — **never** from
the model's tool arguments. That is what makes `run_quality` safe to
auto-approve.

### Project detection

There was no notion of project type in the app before this (the LSP manager's
two-server lookup was the closest thing). Detection checks the workspace root,
then one level down, skipping `node_modules`/`target`/`dist`/etc.

That shallow scan is what this repo's own shape requires: a pnpm/vitest frontend
at the root with the Cargo crate in `src-tauri/`. Both stacks are detected and
both run. A test asserts this against the real checkout rather than a fixture,
since a fixture can drift from reality.

| Stack | Tests | Coverage |
|---|---|---|
| Rust | `cargo test` (summary line + exit code) | `cargo llvm-cov --lcov` |
| vitest | `vitest run --reporter=json` | `vitest run --coverage --coverage.reporter=lcovonly` |
| jest | `jest --json` | `jest --coverage --coverageReporters=lcovonly` |

Mutation is driven natively for Rust only: `cargo mutants -o {artifact_dir}
{in_diff}`. Other stacks need `quality.mutation_cmd`. That is deliberate —
Stryker's flags for redirecting its report and scoping to changed files were not
verified end to end here, and shipping an unverified command means the user
waits minutes to learn nothing. The standard mutation-testing-elements JSON that
Stryker emits is already parsed, so wiring it up later is a command, not a
rewrite.

lcov is the common denominator every tool can emit, so the scorer stays
stack-agnostic. The package manager comes from the lockfile (`pnpm exec`,
`bunx`, `yarn`, else `npx`).

### Three statuses, not two

`Unavailable` is distinct from `Fail`. A missing `cargo-llvm-cov`, a spawn
error, a timeout — these mean *we learned nothing*. Reporting them as failures
would send the model off to fix tests that never ran; reporting them as passes
would be a lie. They are reported honestly and excluded from the verdict, and
the UI marks them `–` rather than `✓`.

### Mutation: the layer that judges the tests

Coverage proves a line ran. It cannot prove anything checked the result — and
`assert!(result.is_ok())` is exactly the shape a model reaches for. Mutation
closes that hole by breaking the code on purpose and demanding the suite notice.

Verified on a fixture with a deliberately weak test: **100% line coverage, and 3
of 5 mutants survived.** That gap is the entire argument for the layer.

Two design points matter more than the plumbing:

*Cost.* Mutation reruns the suite once per mutant, so it is minutes where tests
are seconds. Three things keep it affordable: it is scoped to the diff
(`--in-diff`), it is excluded from `run_quality`'s default layers so an agent
cannot trigger it mid-task, and it is skipped outright when the suite is red —
mutants on a failing baseline teach nothing, and cargo-mutants would refuse
anyway (exit 4).

*Trust.* cargo-mutants documents `outcomes.json` as "subject to change", so
nothing parses it. The stable surface is the four outcome text files plus the
documented exit codes, and the two are cross-checked: exit 2 means survivors
exist, so a report listing none means the format moved under us — reported as
unavailable, never as a pass.

Exit codes carry the distinction the whole harness is built on: 0/2/3 are real
results, while 4 (red baseline), 5/6 (bad diff) and 70 (internal error) mean we
never found out.

### Gherkin: the only check whose subject is not the code

Tests, coverage and mutation all ask "is this code sound?". They cannot ask "is
this the thing we were asked to build?", and that is the failure the others let
through: an implementation that is correct, tested, well-covered, and solves the
wrong problem.

Specs get two properties nothing else in the harness has.

**They are human-owned.** `edit_file` refuses any path under the spec directory,
on both write paths — the proposal tool and the code subagent's direct apply.
A spec the implementer may rewrite is not a specification, it is a comment: any
disagreement between code and requirement would be settled by editing the
requirement. The refusal tells the agent to ask the user instead.

**They drive planning.** The scenario index is injected into the system prompt
beside the skills block, so the planner sees the scenarios it has to satisfy
before it designs anything. A specification nobody reads while designing is
decoration.

The Gherkin parser is a deliberate subset written here rather than pulled in as
a crate: the harness never *executes* scenarios — a real BDD runner does — so it
needs names, tags and steps for indexing, not full fidelity. It never fails
either; a malformed feature yields whatever could be read, because a typo in a
spec must not silently remove it from the index.

**No LLM judge.** The original plan had a fallback: when no BDD runner exists,
ask a subagent to map scenarios onto tests and report a soft warning. That was
dropped. It would have been the one place in this harness where a model's
opinion is dressed up as evidence, and a spec you cannot execute is exactly
where that lie is most expensive. Instead the layer reports `Unavailable` with
the scenario count and how to make it real — the same honesty every other layer
already applies to missing tooling.

cucumber-js is detected automatically. cucumber-rs needs nothing: it runs as an
ordinary test target, so `cargo test` already executes it and a separate command
would run every scenario twice.

### Diff coverage, not project coverage

A project-wide percentage barely moves when an agent adds forty lines, so it is
useless as a gate. The share of *changed executable* lines that no test runs is
exactly the "code the model wrote and nothing checks" signal. It also keeps the
gate fair: nobody pays off a legacy repo's coverage debt to land a two-line fix.

Changed lines that are not executable (blank, comment, declaration) are excluded
from the denominator, not counted as uncovered. Files absent from the coverage
report — a changed `README.md` — contribute nothing. A change touching no
executable line scores 100%.

## The gates

**1. `tasks_set` (`agent/tools/tasks.rs::check_quality_gate`)** — the primary
block, modelled on the existing `check_brain_lld_gate`. Closing an execution
goal is refused unless the session holds a passing run whose digest still
matches. The rejection message always names the goal and the call to make next.

Only the execution half (`golden-<slug>-1`) is gated. The planning half is
closed by Brain, which has written no code and could not make a suite green if
it wanted to. Goals *already* done are not re-gated — `tasks_set` is a full
replace, so re-gating settled goals would make the task list unwritable after
any later edit.

**2. The harness at the finish line (`agent/session.rs`)** — the model can also
simply stop talking. Before a run is allowed to finish, the loop checks the
evidence itself and, if it is missing, stale or red, **runs the checks itself** —
the same pattern as `auto_finalize`. Red sends the model back with the parsed
failures, up to `MAX_QUALITY_RETRIES` (3), then stops honestly with
`stop_reason = "quality_failed"`, mirroring `golden_stalled`.

A red report whose digest still matches is reused rather than re-run: nothing
changed since it was produced, so a second run would spend minutes reaching the
same conclusion. That is the common case while the model is being sent back
over a failing gate.

### What triggers the finish-line check

Keying enforcement only off `<goal>` tags made the harness invisible to anyone
who did not know the tag exists — which is most people, and the opposite of
earning the right not to read the code. `enforce_on` widens it:

| `enforce_on` | Verified at the finish line |
|---|---|
| `"code_change"` (default) | Any run that touched a file a test could execute |
| `"goals"` | Only runs with a tagged `<goal>` |

Verifying by default is the whole point: a harness that acts only when the user
remembers a tag protects nobody. `"goals"` is the escape hatch for a repo whose
suite is too slow to sit through on every change.

Either way the check is **once per run**, at the end — never per task, or a
session with ten tasks would mean ten test runs. A read-only session, or one
that only edited prose, costs nothing.

"Touched source" comes from a *denylist* of documentation and asset extensions,
not an allowlist of source ones. An unfamiliar extension therefore counts as
code and gets verified: over-checking an unknown file type costs minutes,
under-checking it costs the guarantee. Lockfiles and build config are *not*
excluded — a dependency bump can break a suite as surely as an edited function.

The `tasks_set` gate is unaffected by this setting: it always and only guards
the execution half of a tagged goal.

It fails open when the harness itself cannot run (no recognizable project):
blocking a finish on *our* inability to check would strand the user.

### Evidence lives in the session JSONL

`SessionRecord::QualityRun`, alongside the task list, golden state and
`PlanFinalized` — the same source of truth, no second database, no migrations.

Per-session is the correct scope: a linked successor starts with no evidence,
and that is right, because a handoff happens precisely when the work was *not*
finished.

One subtlety worth stating: the digest **excludes `.claudinio/`**. The session
JSONL lives there and grows on every turn — including the turn that records the
quality run — so counting it would make every report stale the instant it was
written. It is gitignored in this repo but not necessarily in the user's, so the
exclusion is explicit rather than left to gitignore.

## Configuration

`<workspace_root>/.claudinio.json`:

```json
{
  "quality": {
    "enabled": true,
    "enforce_on": "code_change",
    "enforced_layers": ["tests"],
    "diff_coverage_threshold": 80.0,
    "mutation_score_threshold": 60.0,
    "test_cmd": null,
    "coverage_cmd": null,
    "mutation_cmd": null,
    "features_dir": "features",
    "gherkin_cmd": null,
    "test_timeout_secs": 600,
    "coverage_timeout_secs": 900,
    "mutation_timeout_secs": 1800
  }
}
```

Defaults enforce **tests only**. Coverage needs tooling the user may not have
installed, and a harness that blocks on day one gets switched off. Empty
`enforced_layers` is observation mode: report, never block.

Malformed JSON falls back to the defaults *with the gate on*. A stray comma must
never silently disable enforcement.

The **Quality tab** in Settings edits this same file — it is a view onto
`.claudinio.json`, never a second source of truth. It saves on change rather
than on the panel's Save button, because that button writes the *global* config:
one button appearing to cover two different files is worse than two obvious
behaviours. The tab also lists the stacks detection found and the commands it
would run, so "did it understand my project?" is answerable without running
anything.

The panel cannot write a file the harness fails to read: unknown layer names are
dropped, the threshold is clamped to 0–100, and a zero timeout (which would kill
every run) is floored at one second.

`test_cmd` replaces detection entirely rather than running alongside it — if the
user said how to test the project, also running our guess would double the wall
clock and could contradict them.

## A correctness fix the mutation work surfaced

Scoping used to return an empty change set both when git said nothing changed
*and* when there was no git at all. The two are not the same: the second means
we cannot tell, and treating it as "nothing to check" let a non-git project pass
coverage and mutation forever without either layer ever running. `changed_lines`
now returns `Option`, and an unknown scope reports as unavailable.

## Roadmap

**Phase 4 — Gherkin / BDD.** `features/**/*.feature` as human-owned ground
truth: `edit_file` rejects writes there, and Brain receives a scenario index in
the dynamic prompt block so plans reference the scenarios they cover. Validation
is hybrid — a real BDD runner is a hard gate; where none exists, a `Verify`
subagent maps scenario→test as a **soft** warning. Model judgement is never sold
as mechanical evidence.

**Phase 5 — Metrics and trend.** Cyclomatic complexity via the tree-sitter
grammars `code_intel` already carries, duplication, and non-regression on
touched files. This is the phase that needs cross-session history, and so the
phase that should introduce `quality.db` (a separate database with real
migrations — never `index.db`, which is dropped and rebuilt on schema bumps).

Also deferred: a manual "run checks" button with a live progress channel, and a
history/trend panel.

## Verification

- Parsers, gate evaluation, config, detection, digest, diff parsing: unit tests
  against fixtures.
- Runner: real subprocesses (pass, fail, timeout, missing report).
- Gate: closing a goal with no evidence / stale evidence / red evidence /
  fresh green evidence; planning half ungated; ordinary tasks ungated;
  observation mode ungated; a rejected call leaves the task list untouched.
- End-to-end: a real git repository, a commit, a change, both layers run, the
  verdict names the blocking layer, and the digest is shown to pin the report to
  that exact state.
- Detection is asserted against this repository's own layout.
- Both real commands (`vitest --reporter=json`, `vitest --coverage
  --coverage.reporter=lcovonly`) were run against this repo to confirm the
  generated flags and artifact paths are correct.
