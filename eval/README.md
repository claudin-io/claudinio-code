# The eval

A fixed task set, run headless through the real agent loop and scored by each
task's own oracle. Every change to the harness that claims to make it cheaper,
faster or better is gated on the numbers this produces (lean-harness plan,
`docs/plans/2026-10-18_lean-harness-audit-and-plan.md`, §1 and Phase 0).

## The three numbers

| | Metric | How the runner computes it |
|---|---|---|
| **M1** | Billed cost per solved task, cache-aware | Total cost of every provider request in the run (main sessions, subagents, the completion judge) divided by the number of tasks whose oracle passed. The cost is what the provider reported; for a request with none, `--price-in/--price-out` (USD per million tokens) fill it in, cache reads at 0.1x and cache writes at 1.25x the input price unless given. When neither exists M1 prints `n/a` rather than a number that is too low. |
| **M2** | Rounds per task | Median over tasks of provider requests, main session and subagents together. Each round re-sends the whole context, so this is latency and money even when cached. |
| **M3** | Pass rate | Tasks whose oracle passed after the session, over tasks run. Nothing ships that lowers it. |

Also printed: the **cache-read share** (cache reads over all input processed:
uncached + cache reads + cache writes) and the **subagent share** of all
tokens. Token counts on their own are not a metric: a leaner context that costs
more turns can cost more money (Weinberger & Hozez, arXiv 2607.12161).

## Running it

The runner is a Cargo example inside the app crate (it drives the private
`agent` module, and a second `[[bin]]` would confuse the Tauri bundler). From
`src-tauri/`:

```bash
# No model: every task's oracle must fail on the start tree and pass on the
# reference solution. Exits non-zero if any task is invalid.
cargo run --example claudinio-eval -- validate ../eval/tasks
cargo run --example claudinio-eval -- validate ../eval/tasks --tasks js-lru-cache,py-pager-last-page

# One fresh Builder session per task against a provider, then the oracle.
export MY_KEY=...
cargo run --example claudinio-eval -- run ../eval/tasks \
  --model claude-sonnet-4-5 --base-url https://api.anthropic.com --api-key-env MY_KEY \
  --protocol anthropic --out results.jsonl --max-rounds 60

# SWE-bench instance records (JSONL) to task files. No dataset is downloaded.
cargo run --example claudinio-eval -- import-swebench instances.jsonl ../eval/swebench \
  --test-cmd-template 'python -m pytest {tests}'
```

`run` also takes `--context-window TOKENS` (the window the harness sizes its
limits and prompt profile from), `--task-timeout-s` (default 3600),
`--price-cache-read/--price-cache-write`, `--jev-api-key-env ENV` (Jev runs
only when this is given) and `--work-dir DIR`. Without `--work-dir` each command
makes a fresh temporary directory and keeps it, so a failed task can be
inspected; on a machine whose `/tmp` is in memory, point `--work-dir` at a disk.

`run` writes one JSON line per task as it goes — `id`, `pass`, `outcome`
(`completed`, `timeout`, `error: …`), `wall_s`, the oracle's exit code and
output tail, and the metrics: `rounds` (`main_rounds` + `subagent_rounds`),
`input_tokens`, `cache_read_tokens`, `cache_creation_tokens`, `output_tokens`,
`subagent_tokens`, `cost`, `cost_known`, `tool_calls` per tool, `subagents`,
`sessions` — then prints M1, M2, M3 and the shares. The sessions themselves are
ordinary session files under the task's workspace (`.claudinio/sessions/`,
subagent transcripts in `<id>.sub/`), so `scripts/session-stats.py --cost` reads
them too.

## What a run is

For each task the runner materializes the start tree in a fresh directory
**outside any git repository** and makes it a repository of its own with one
commit (`start`), so `git diff` and the quality gate behave as they do for a
user. A tree taken from git is exported with `git archive`, never cloned, so
the model cannot read the solution out of the history. It then writes a
`.claudinio.json` whose quality `test_cmd` is the task's `check` command,
builds a lexical code-intel index, and starts the normal session entry
(Builder, fresh session) with the task's prompt. The config is the app's, with
everything installed on the machine turned off: no hooks, no browser, no MCP
servers, no plugins, no claudin.io account; Jev only with `--jev-api-key-env`.
Skills in the home directory (`~/.claude/skills`, …) still load, as they would
for a user, and join the prompt's skill catalog; for numbers that compare
across machines, run the built binary under an empty home:
`HOME=$(mktemp -d) target/debug/examples/claudinio-eval run …`. After the session ends, the oracle runs in the same
directory: its hidden files are written, its patch applied and its reference
files copied in first, so the agent never saw them.

## The approval policy, and why

The runner executes the model's tool calls on the host, with nobody watching.
The person at the other end of every prompt is replaced by a deliberately
narrow policy (`src-tauri/src/eval/policy.rs`) whose job is to keep a *wrong*
model from doing damage by accident — an `rm -rf`, a `git push`, a stray
`curl | sh` — so it never auto-approves arbitrary shell.

**It is not a sandbox, and `run` must be executed in a disposable container or
VM.** `run_quality` and the oracle run pytest, `node --test` and vitest over
files the model wrote (a `conftest.py`, a test file, a vite config), so
model-authored code runs on the host in every task, by design: that is what
running a project's tests means. The policy closes the routes *through
approvals*; it does not contain a model that sets out to escape.

- **File edits are approved**, confined to the workspace (the temporary
  directory) by the path guard — except anything inside a `.git/` directory
  and the runner's own `.claudinio.json` / `.claudinio/`. `.git/config` can
  name a program git runs by itself (`core.fsmonitor` runs on every
  `git status`, which the policy approves), and `.claudinio.json` names the
  command `run_quality` runs; an approved edit to either would turn an approved
  read-only command into any command at all.
- **A shell command is approved only when it only looks**, as the permissions
  module classifies it (`ls`, `cat a.txt`, `git status`, `git diff`, `wc -l` …
  with no chaining, pipes, substitution or redirection).
- **Every other shell command is denied** (`rm -rf /tmp/x`, `curl … | sh`,
  `python3 -c …`, `echo x > f`, `node script.js`, `cargo test`), with a result
  telling the model to change files with `edit_file` and to run checks with
  `run_quality`, which runs the task's own `check` command from the workspace
  config (a file the model cannot edit). The runner sets a shell gate so that
  even commands the app's allowlist would run unasked reach this policy.
- **Anything else is refused** (MCP tools, browser navigation): neither is
  configured, and an unknown tool fails closed.
- **`ask_user`** is answered from the task's `answers` list, in order, and then
  with "No further input is available; proceed with your best judgement."

## The task format

One YAML file per task in a task-set directory; the file name is the id.
Unknown keys are refused, so a misspelled `oracle.fiels` cannot silently
produce a task whose hidden tests never run.

```yaml
id: py-pager-last-page
title: Fix the pager's last page
tags: [bugfix, python]
prompt: |                     # the user's words, the first message
  The last page of results is always missing. Fix it.
setup:                        # files, or a git tree (+ files laid over it)
  files: { pager.py: "...", test_pager.py: "..." }
  # git: { repo: ../.., commit: <sha> }   relative paths resolve from this file
  # symlinks: { node_modules: ../../node_modules }
check: python3 -m pytest -q   # what run_quality runs (default: oracle.command)
oracle:
  command: python3 -m pytest -q
  timeout_s: 120
  # files: { path: content }  hidden tests, written just before the command
  # patch: <unified diff>     applied with git apply before the command
  # reference_files: [path]   copied from reference.git_commit before the command
reference:                    # a known solution, used only by validate
  files: { pager.py: "..." }
  # git_commit: <sha> | patch: <unified diff>
answers: ["Use the existing helper."]
max_rounds: 40
```

A task that **links** a directory (`symlinks`) shares it with the repository it
points at, so its `check` and `oracle` may not run a package manager (`pnpm`,
`npm`, `npx`, `yarn`, `bun`): the parser refuses it. Measured on 2026-10-18:
`pnpm exec vitest` inside a materialized parent commit verified that commit's
dependencies, found them stale and reinstalled into the linked tree, rewiring
this repository's own `node_modules` into `/tmp`. Call tools by path
(`./node_modules/.bin/vitest`).

## The task set (`eval/tasks/`)

25 tasks, all valid under `validate` (about 50 seconds for the whole set).

- **18 self-contained fixtures** in Python (pytest) and JavaScript
  (`node --test`), covering the tags `bugfix`, `feature`, `refactor`,
  `multi-edit` (one change that must stay consistent across files),
  `independent-edits` (two unrelated changes in disjoint files), `long` (the
  cause is only found by reading many files), `search` (the right place is only
  findable by meaning), `document` (a write-up whose oracle checks a markdown
  file for given sections with content) and `brain` (a fully specified feature
  that should need no interview). Hidden tests sit in `oracle.files` where
  the agent never sees them, and every test file the agent *can* see is also
  in `oracle.files`, so the oracle rewrites it before it runs: the policy
  approves every edit in the workspace, and a run must not pass by changing
  the assertion instead of the code (pinned by
  `every_graded_file_the_agent_can_see_is_rewritten_by_the_oracle` in
  `src-tauri/src/eval/task.rs`).
- **7 history tasks** (`hist-*`) from this repository's own frontend: the start
  tree is the parent of a real commit, the oracle is the vitest file that
  commit added or changed (copied in from the commit only when the oracle
  runs), and the reference is the commit. They link the repository's
  `node_modules`, call `tsc` and `vitest` by path, and lay a small
  `vitest.eval.config.ts` over the tree that turns Vite's `fs.strict` off (the
  linked packages resolve to real paths outside the temporary tree, which Vite
  otherwise refuses to serve) and sets `passWithNoTests: false`; the oracle
  rewrites that config too, so a start-tree edit cannot make the graded run
  collect nothing and pass. The in-session check is `tsc` plus the full
  vitest suite, green on every start tree. They need the commits in the
  repository the task points at (`repo: ../..`), so they do not run from a
  shallow clone.

## What is not here

- **No baseline yet.** Phase 0 was built without a single live-model call;
  the first `run` on the default model records it, under `eval/baselines/`.
- **Terminal-Bench is not integrated.** Its harness drives agents from its own
  Docker and Python runner rather than being driven by one, so it does not fit
  a runner that owns the agent loop. SWE-bench fits: `import-swebench` turns
  its instance records into ordinary task files (setup = a clone of
  `https://github.com/<repo>` at `base_commit`; oracle = apply `test_patch`,
  then run `FAIL_TO_PASS` through the template; reference = the gold `patch`).
  Running them needs each instance's Python environment, which the runner does
  not build.
