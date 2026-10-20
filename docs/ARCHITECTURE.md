# Architecture

How Claudinio Code is put together, and which of its shapes are load-bearing.
Read this before a change that crosses a module boundary; `CONTRIBUTING.md`
covers the build and the checks.

## The layers

```
┌──────────────────────────────────────────────────────────────┐
│  src/  — SolidJS frontend                                    │
│  chat timeline · approvals · Monaco diffs · file tree        │
└──────────────────────────┬───────────────────────────────────┘
                           │  Tauri IPC (invoke + Channel events)
┌──────────────────────────┴───────────────────────────────────┐
│  src-tauri/src/                                              │
│                                                              │
│    commands/     the IPC surface — thin adapters             │
│         │                                                    │
│         ▼                                                    │
│    agent/        loop, providers, tools, subagents,          │
│                  permissions, skills, MCP, hooks, persistence│
│    code_intel/   tree-sitter → SQLite FTS5 + ONNX vectors    │
│    lsp/          language-server client                      │
│    workspace_path.rs, procutil.rs, http.rs, askpass.rs       │
└──────────────────────────────────────────────────────────────┘
```

**The dependency rule: `commands/` depends on the core; the core never depends
on `commands/`.** A `#[tauri::command]` should unwrap its arguments, call into
`agent/`, `code_intel/` or `lsp/`, and map the result — nothing more. Anything
with logic worth testing belongs below it, where it can be tested without a
Tauri runtime.

This is enforced, not just documented: `architecture_tests` in `lib.rs` fails
the build if `agent/`, `code_intel/` or `lsp/` import `crate::commands`. Three
things violated it historically and are worth knowing about, because they are
the shapes that tempt you back:

- a platform helper with no IPC in it (`procutil.rs`) filed under `commands/`
  because a command was its first caller;
- JSONL persistence for tasks living in `commands/tasks.rs` next to the commands
  that expose it (now in `agent/persist.rs`, which already owns the store);
- a global `INDEX_SEMAPHORE` in `commands/code_intel.rs` that the file watcher
  had to reach back for (now in `code_intel/mod.rs`).

## What one message does

```
send_message (commands/agent.rs)
   └─ agent/run_loop.rs :: run_workflow
        ├─ prompt::run_prefix          (profile for the model's window → prompt + tools)
        ├─ ContextBudget::for_model    (the lines this run is held to)
        ├─ handoff::*                  shed / handoff / compaction if the context is over a line
        └─ loop, per round:
             ├─ retry::stream_message_with_retry → provider::stream_message
             │                              (SSE → AgentEvent over a Channel)
             ├─ no tool calls? → judge::judge_terminal_turn, then done
             └─ tool calls  → tool_exec::run_tool: permissions → [approval gate]
                                → tools::execute → results appended, next round
                              (tool_batch::run_ahead: a contiguous run of read-only
                               calls runs concurrently, at most 4, results kept
                               in the model's order)
```

**Concurrency within a round.** A run of two or more consecutive calls that are
all `tools::PARALLEL_SAFE_TOOLS` (the file, index, LSP and web readers and
`tasks_get`), need no prompt and are watched by no `PreToolUse`/`PostToolUse`
hook is run ahead at once, at most four at a time, each body on tokio's
blocking pool so a synchronous read or `rg` never holds an async worker
(`agent/tool_batch.rs`). Any
other call (an approval, `edit_file`, `bash`, `spawn_agents`, `ask_user`, the
browser, the task and document mutators, `remember`, every MCP tool) ends the run and runs on
its own, in order, so a read after an edit sees the edit. Each result lands in
its call's slot and the loop then walks the round in the model's order as
before: the transcript, the loop watch, the repeated-read pointer and the hooks
see the same sequence they did when calls ran one by one, a kept result
included. A pause starts nothing new and waits for the calls already under way
(as it always waited for the one running call: a tool body cannot be stopped
mid-read), keeps their results and answers the calls it kept from starting as
not run, one result per call. Subagents use the same run-ahead.

The loop is split by responsibility, one file each under `agent/`: `prompt.rs`
(the prompt text and the tool list beside it), `run_loop.rs` (the loop above),
`tool_exec.rs` (one tool call, from permission to `tool_result`), `judge.rs`
(is a terminal turn finished, is the loop stuck), `handoff.rs` (what happens at
a context line), `retry.rs`, `nudges.rs` (every text the harness itself puts in
front of the model), `gate_text.rs` (what each gate says when it refuses) and
`events.rs` (`AgentEvent`). A refused request becomes
`API error: HTTP <status> — <the provider's message>`
(`provider/http_error.rs`: the error object's `message`, or the start of the
body, one line, the API key redacted); `retry.rs` classifies by the status
alone, so the message never changes whether a request is retried. `session.rs` keeps
the controls a session shares with the UI — steering, approvals, answers —
and re-exports the loop's public names, so `agent::session::run_workflow` still
resolves. `architecture_tests` fails the build when any file under `agent/`
passes 2,000 production lines: `session.rs` was 8,446 before the split.

Every step is persisted as a line of JSONL under `.claudinio/sessions/`. That
file is the source of truth, not memory: it is re-read on each message, so a
session survives a restart and the UI can rebuild the whole timeline from it
(`src/lib/chatRecords.ts` does that translation).

`AgentEvent`s stream to the frontend over a Tauri `Channel` while the run is in
flight. Approvals travel the other way through oneshot channels keyed by
`session_id:tool_use_id`.

### One timeline

A session is one timeline over one loop: it reads, asks (`ask_user`), edits
(`edit_file`, a diff to approve), runs `bash` and spawns subagents, and the
agent decides from the request whether the work needs a plan. There is no
session mode, no router choosing one and no plan-mode tools (single-timeline
plan, `docs/plans/2026-10-20_single-timeline-and-one-plan.md`). The handoff
machinery (`agent/transition.rs`) now serves only the token-threshold handoff
and a new top-level document's session (below). Records an older session
carries — `Mode`, `ModeRoute`, `GoldenCycle.mode`, `RunConfig.mode`,
`PlanFinalized` — still deserialize and render as history; nothing writes them.

**One model per session.** `AgentConfig::model()` serves the main session, its
subagents, the completion judge and the prompt enhancer. A config saved with
the old `brain_model`/`builder_model` pair migrates to its `builder_model`; a
workspace `.claudinio.json` sets `model` (an old `builder_model` there is still
read).

**One plan per session, sized to the request.** None for one step; a task list
(`tasks_set` with an id and a title per task, `tasks_update`) for multi-step
code work; a living document (`doc_create`, below) for research, a design, a
write-up or when asked. Never both, and the rule is in the tools, not only in
the prompt: `doc_create` replaces the list, keeping only the golden tasks
(`doc_session::drop_checklist`), and refuses more than `MAX_SECTIONS` (6)
sections — `doc_add` is uncapped; `tasks_set` is refused while the session
links a document (`gate_text::TASKS_WITH_DOCUMENT`).

**Golden goals.** `<goal>` tags in a message become one task per goal
(`tasks::create_golden_tasks`, id `golden-<slug>-1` — the `-1` suffix sessions
from before one timeline already carry, so their goals keep their ids), gated
by `check_quality_gate`: it closes only on a passing `run_quality` whose digest
still matches the worktree. A legacy `-0` "plan" half no longer blocks. A run
that ends its turn with a goal pending is sent back to it in the same session
(`nudges::golden_continue`, a `GoldenCycle` record), bounded by
`max_golden_cycles` and `max_golden_stalls`; at a cap the run stops with
`max_golden_cycles` or `golden_stalled` and names the goals left. A session with
golden goals never drops to the Compact profile, whose prompt has no golden
rules.

**The Standard prompt** (`agent/prompt/standard.rs`) is the prompt "at the right
altitude" (lean-harness Phase 5): a base of identity, the plan rule, how the
tools relate (and why indexed search beats shell search), when to ask and when
to stop, untrusted content, hook output, links and the language policy, about
1.4k tokens with the workspace line; a golden block when the session has goals;
and one `DOING THE WORK` block of about 340 tokens (budget 600). It describes no
gate the code enforces — the one-plan rule, the quality gate on closing a
golden goal, golden-task protection, read-before-edit, the write gates, the
spec guard, the finish-line check — beyond one line saying a refusal says what
to do next, and every refusal in `agent/gate_text.rs` does (`gate_text::tests`
pins each one's next step and that none names a deferred tool). The interview
is adaptive: only decisions that change the work, independent questions in one
`ask_user` call with the recommended answer first, none for a fully specified
request. The consensus checklist, UI sizing and asset rules, the confirming
question and the Mermaid catalog are the built-in `design` skill
(`agent/skills/design.md`), which writes the design into the living document;
it is listed in the catalog and read by reference. Because the prompt points at
it by name, `design` is reserved (`RESERVED_BUILTIN_SKILLS`): a project, user or
plugin skill of that name is ignored rather than allowed to shadow it, and
installing a remote skill under that name is refused before anything is
written. Capitals survive only where `standard::KEPT_IMPERATIVES` cites the
failure they prevent (at most three; today one, VERBATIM). The earlier *v1*
prompt and its mode blocks are gone. Compact and the git operator have one
prompt each.

### Context

A run never compares its context against a constant. `agent/budget.rs` turns
the model's window (catalog snapshot, a custom provider's probe or typed value,
the local engine's setting; 200k when unknown) into a `ContextBudget`: a soft
line where a chat session hands off, a hard line where it compacts, a ceiling
past which nothing is sent, and the caps a single tool result is held to. At
200k these are the numbers the app always used; below that they scale with the
window, after the reply reserve and the fixed prefix are taken out.

The reply reserve is the request's `max_tokens`, by construction
(`budget::reply_tokens` feeds both): a server that checks prompt plus
`max_tokens` against its context rejects a request that asks for more reply
than the budget set aside, whatever the prompt's size. On a scaled window the
results of one round are also capped together, not only one by one — the round
just read is the one thing no prune cuts, so it has to fit by itself. And a
prefix that does not fit the window at all is refused before anything is tried:
no handoff or summary makes a system prompt smaller.

Crossing a line sheds weight masking first (`agent/prune.rs`, lean-harness
Phase 6): copies a newer result supersedes (lossless); then the oldest tool
results cut to their head, the call kept (masking by age). Jev, when a backend
exists, no longer judges the whole history first: it is asked one question —
must this output stay verbatim — about each cut masking would make in the
eight newest rounds (`ARBITRATION_ROUNDS`), and what it keeps comes out of the
cut; older cuts stand unasked, and when Jev's version does not free enough the
cuts of masking alone are offered last, since a handoff or a summary loses
more than outputs that can be read again. Subagents run the same ladder as a
last resort (they may also drop the oldest calls outright). Text is never
touched, nor are the user's answers to `ask_user` or the current task list. A dropped call is stubbed, never
removed: its `tool_use` block stays with input `{}` and its result becomes a
one-line note, because DeepSeek's thinking mode re-attaches its reasoning by the
tool_use blocks of earlier assistant messages and refuses a history that lost
one (HTTP 400, first live eval, 2026-10-18). A stage is accepted only if
it frees enough to be worth the cache it invalidates — which is also why
nothing is pruned early: on a caching provider the history is cheap to resend
and expensive to rewrite.

When shedding is not enough a chat session hands off to a successor;
summarizing (`handoff::compact_history`) is the last resort, and what a manual
compaction runs. The summarizer
(`subagent::summary_goal`) writes a note for recall, not brevity — `## Goal`,
`## Decisions (and why)`, `## Files touched` (`path:lines`), `## What failed`
(the exact error text), `## Left to do`, `## Constraints the user stated`
(verbatim) — recall first, then trimmed, to about 1,500 words (it was a
600-word summary). The note, after any `PreCompact` hook output, is appended to
`.claudinio/notes/<session>.md` under a timestamp header (`agent/notes.rs`;
never through a link, and a failed write never fails the compaction), and the
compacted context ends with a line naming that file, so a successor can read
what an earlier compaction carried.

A tool result over its cap (`budget::RunLimits::tool_result_chars`) keeps its
head and its tail, about 70/30 and on line boundaries when one is close,
around one marker line: `[… N of T chars omitted; <how to get them> …]`
(`agent/result_cap.rs`). For `read_file` it names the file lines it left out
and the read that gets them (`lines 412-1880 of 1-2400; read_file with
start_line=412, end_line=1880`), counted from a ranged read's `start_line` and
past `read_file`'s own size warning; for anything else the whole output is first
written to a temp file, as `output_trim` does for `bash`, and the marker names
it (the file is the user's alone, 0600 in a 0700 folder — a folder that is a
link, another user's or writable by others gets no save and none of its files
is trusted — named by the hash of
its content so a repeated output is the same file and `loop_watch` still sees
the repeat, and saved outputs older than a week are swept on the first save of
the app's process (so an app left open for days does not sweep again until
it restarts); `read_file`, `grep` and the other read tools may open these files, and
only these, outside the workspace). `fit_round`, which holds one round's results to a share together, re-cuts
a capped result from what it kept and keeps its true size and hint; `prune`
reads the same marker (or the head-only note older sessions carry) to know a
capped read is not the whole file. Every cut lands on a character boundary,
and the lines a read cut left out are counted on bytes, so text in Portuguese,
CJK or emoji never splits a character (a slice inside one panicked the run
until Phase 6f). A marker counts only when its numbers add
up to the text around it.

An `edit_file` that leaves a file the tree-sitter grammars cover no longer
parsing, when it parsed before, answers with the edit applied and one warning
line naming the first error's line and its text (`agent/tools/syntax_check.rs`);
the edit is never reverted. `file_outline` is not capped: measured on this
repository's own sources (Phase 6), its mean output is 3,587 characters.

What is sent ahead of the conversation is chosen once per run
(`agent/surface.rs`): the profile the window calls for (`Standard`, or
`Compact` under 64k for a session without golden goals), then the tool surface
the user set for that model. Prompt, tool list and the write gates are derived
together and never change between turns of a session, because they are the
cached prefix —
with one exception, below: a deferred tool the model loads is appended to the
end of the list.
A chat session edits files itself under both chat profiles
(`PromptProfile::edits_files`; Standard since decision D1, lean-harness Phase
3): it is offered `edit_file`, which shows a diff and waits for approval, and a
shell command that writes is asked about like an edit. The git operator
refuses edits, approval or not, and also refuses a shell command
`permissions::bash_writes_files` sees writing (redirection, `tee`, in-place
`sed`/`perl`, `patch`, git's `--output`, inline scripts that call a write). What bounds the shell is one rule, whose whole
correctness argument is the comment at the top of the shell policy in
`agent/permissions.rs`: a command runs unasked (`bash_call_permission`) only
when, after the tokens
`2>&1`, `2>/dev/null`, `>/dev/null` and `1>/dev/null` are dropped, it has no
blank but space and tab (a form feed or NBSP splits a word for Rust and not
for sh), no backquote, `$`, `<`, `>`, `&`, `;`, backslash, brace,
parenthesis or glob — on Windows, where it runs under `cmd /c`, which
expands `%NAME%` before it finds `&` and `|`, no `%`, `^` or `!` either, and
the child runs with `NoDefaultCurrentDirectoryInExePath` so an allowlisted
name is not a `git.cmd` or `ls.bat` in the working folder — and
no `||`; every `|`-part closes its own quotes; the
first part starts with an allowlist entry (`BASH_ALLOWLIST`: git inspection,
reads, builds, test runners, `node`/`python`/`pnpm run`, `which`/`echo`/`pwd`;
`git branch`/`git remote` only bare or
listing, `date` only bare or `+FORMAT` and never under `cmd /c`, whose `DATE`
reads any argument as a new date) and is not an interpreter or a `date` (which reads a new date from stdin under `cmd /c`) fed the call's `stdin`; every later part is
one of `PIPE_FILTERS` (`head`, `tail`, `wc`, `grep`, `cut`, `tr`, `nl`); and
the write check sees nothing. With no backslash, `$` or backquote, quotes do
not escape, so each `|` the split sees is a pipe the shell sees, and no word
expands into another (only `~` becomes the home folder). Everything else asks
(YOLO or a hook's `allow` may approve it). The git operator (the
one session with `auto_approve_git`) also runs `git add`, `git commit` and
`git push` past the allowlist, still under rule 1 — so a commit message with
a parenthesis (`feat(ui): x`), `&`, `;`, `$`, `<`, `>` or a glob character
asks — and a push with a word starting `--r` or `--e` asks too
(`--receive-pack`, `--exec` and their abbreviations name a program git runs). The deny-list
(`on_the_deny_list`) and the memory guard run first and refuse outright. The
deny-list is a substring tripwire, refused in every session: a command is Denied
when its lowercased text, or its normalized text (every `'`, `"`, backquote,
backslash and `^` removed, blanks collapsed), contains a `BASH_BLACKLIST`
entry, redirects into `/dev/` other than exactly `null`, `stdout`, `stderr`,
`tty` or `fd/N`, or runs cmd's `format X:` — so a path (`/bin/rm -rf /`), a
wrapper (`time dd if=…`), a quote (`r''m`), a backslash (`d\d`) or a heredoc
is caught with no parsing; a spelling no entry names once those characters
are gone, such as bash's ANSI-C quote (`rm -rf $'\x2f'`), is not, as on the
original list. Its accepted false positives, as on the original list, are
ordinary text that contains an entry (`grep chown src`, `cp /tmp/x .`,
`git commit -m "fix: chown "`), plus text holding one of the entries the
original lacked (`git commit -m "departed from x"`). The rule replaced a chain
splitter, per-option `sort`/`uniq` readers, a stdin detector and runner
argument tables after three review rounds kept finding bypasses in them
(lean-harness plan, "Shell policy simplified"). The allowlisted
interpreters, runners and builds still run whatever their arguments and the
project choose, a program to run (`go test -exec`, `cargo --config`) or a
folder to clear (`pytest --basetemp`) included. Compact adds one rule of its
own: a call to any tool outside the profile's ten is refused rather than
run.

**Subagents** (`agent/subagent.rs`, `agent/delegation.rs`) are the exception
now, not the route every edit takes: the Standard prompt sends broad read-only
investigation to `explore` subagents and keeps `code` subagents for two or more
changes to disjoint files. A spawned explore subagent's system prompt asks for a
report of at most ~1,500 tokens in `## Findings` / `## Files` (`path:line`) /
`## Open questions`; the compaction summarizer, also explore mode, is not asked
for it (`SubagentSpec::reports_to_parent`). The parent's `tool_result` carries
at most 6,000 characters of any report — head and tail, cut on line boundaries,
around a marker counting what was omitted — while `SubagentDone` gives the
timeline all of it. Each code subagent's goal is prefixed with one `<brief>`
composed per `spawn_agents` call: the `doing` task, the session's document, and
the paths in the run's read tracker (shared with subagents, so earlier
subagents' reads count), relative, sorted, at most 40. The brief sits in the
first user message, never in a prefix.

**Deferred tools and skills by reference** (`agent/deferred.rs`,
`agent/skills.rs`). A Standard main session sends a core set of fourteen —
`read_file`, `list_dir`, `grep`, `edit_file`, `bash`, `ask_user`,
`tasks_set`, `tasks_update`, `run_quality`, `spawn_agents`, `skills`,
`doc_create`, `remember` and `load_tools` — and
names everything else in a `DEFERRED TOOLS` section of the system prompt, one
line each: the index and LSP searches, `web_search`, the browser tools,
`tasks_get`, and every MCP tool (its own description cut to 100
characters, at most 60 lines, the rest counted by server). `load_tools`
appends definitions to the end of the tool list in load order, and a call to a
deferred tool the model did not load loads it on the spot and runs; either way
the session file gets one `tools_loaded` record and one `cache_break` per round
that loaded anything (the prefix is rewritten once, however many joined), and every
later run starts from those records, so a resumed session sends the identical
array. The catalog never depends on what is loaded, so the system prompt stays
the same bytes and only the list grows (never a swap — that would rewrite the
cached prefix each time). Feature-gated tools leave the catalog with their
feature. Hooks select by the name of the tool called, through the alias table,
so a `WebSearch` matcher still selects `web_search` loaded or not. The skills
section is likewise one line per skill (`- name — description`, cut at 160
characters, scope priority then name, 25 lines and a count); a skill's
instructions come from the `skills` tool (`list` / `read`), which is also what
the catalog tells the model to do with a `<skill>name</skill>` mention.
Subagents defer nothing — a fresh context has no earlier load to build on —
and leave out only the three LSP tools. Compact and GitSync defer nothing
either. No Standard prompt line makes a deferred tool a first, last or
mandatory call, and no text the harness injects (`agent/nudges.rs`, the handoff
kickoffs in `agent/transition.rs`) names one — both pinned by tests, because the
model loads a tool to obey, and a load late in a session rewrites the cached
prefix in front of the whole conversation. That is why `remember`, called near a
session's end if at all, is core; `tasks_get` was core for the same
reason (the first call) until the task protocol stopped asking for it (below).
The default Standard prefix measures 4,813 tokens on the harness's meter
(12,337 before the lean harness), and 6,290 with ten skills and a twenty-tool
MCP server (17,223 before); `prompt.rs`'s `prefix_budget_tests` hold it to 5,000 and
10,000. In `run_config.prefix` the catalog counts as `system`, and `mcp` is
the definitions actually sent.

**Tasks as recitation, not RPC** (`agent/task_recital.rs`). The prompt used
to make `tasks_get` the first call of every run, a standalone `doing` update
before any work, a final update before the answer, and forbade batching — three
to five rounds per task spent on state the harness already holds. Now a
Standard-profile run (also when the window makes it Compact) ends
its user turn with a `<tasks>` block, one line per task (`id · status ·
title`, golden tasks marked), added beside `<hook-context>` and persisted in the
same `Turn`, so a reload replays it; the chat bubble is the `user` record and
never shows it. A successor session gets it after its kickoff, because
`link_session` copies the tasks before that run starts. After a summarizing
compaction `persist::history_from_records` restates the list as it stood at the
compaction point as the last block of the rebuilt context, when that context
ends on a user message (a compaction inside a run, or one between runs that kept
no tail; one that ends on the model's answer leaves it to the next run's turn,
and an identical last block is never repeated — a run whose turn merges into a
tailless summary drops the recital and appends its own, so the list is said once
and last). Successor kickoffs say that a list follows as a `<tasks>` block and
that no block means no list: the run appends one only when the session has
tasks. The prompts
say: a list only beyond a one-step change; `doing` in the same round as the
task's first tool call; batch status changes — `tasks_update` takes
`updates: [{id, status?, journal?}]` (a top-level task with an id beside it
joins the batch, first), checks every item (id, status) and then the quality
gate on the list as the whole batch leaves it, and applies nothing if any check
fails, naming the task and the reason. A task is an id and a title
(`tasks::fill_from_stored`): a known task resent without its title or status
keeps the stored ones, a new one without a title is refused. Because the block
carries no descriptions or journals, a `tasks_set` that resends an existing task
with an empty description or journal keeps the stored one
(`tasks::keep_stored_details`): a list rewritten from the block alone does not
erase the findings recorded since. "Existing" means the
same task, not only the same id: a stored task that is done and sent under
another title is a new task reusing the id (models number their lists `t1`,
`t2`, ...), and starts with what it was sent. A turn that ends
with a task still `doing` that this run set `doing` or wrote to (the loop keeps
the list it read at run start and compares) gets one `nudges::still_doing`
reminder per run, then finishes; a task an earlier run left `doing` is not
named (work is a status or journal change against the run-start snapshot;
a reworded title is not). `prompt.rs` pins the absence of mandated round-trips
(`no_prompt_mandates_a_task_round_trip`) across the Standard prompts, Compact,
the kickoffs and the nudges.

**The living document** (`agent/document.rs`, `agent/doc_session.rs`,
`agent/tools/doc.rs`; lean-harness Phase 4b). A session may own one Markdown
file under `.claudinio/docs/`, linked by a `document_linked` record (the latest
wins). `document.rs` is the format and nothing else: a placeholder is a line
whose trimmed content is `[topic]`; the harness tags each section with an HTML
comment it owns (`<!-- claudinio: id=s2 status=doing by=sub:x -->`, status
`todo | doing | done | human`); a written section is `## topic`, the comment,
its body and the end marker `<!-- claudinio: end -->` the harness writes after
it, so prose a person typed under a placeholder stays prose once that
placeholder is written (a file without the marker — older, or a person removed
it — reads the body up to the next section or level-1/2 heading, and the next
write adds it); a body cannot open or end a section (such lines are demoted or
escaped on write); everything else is carried
byte for byte, so `parse` then `render` is the file. Ids are never reused, not
even a deleted section's. `doc_session.rs` owns the file — only a relative, plain-component `.md` path
under `.claudinio/docs/` that does not resolve outside it through a link: the
session JSONL is workspace data, and any other linked path leaves the session
with no document rather than letting the harness rewrite, say, `Cargo.toml` at
a run start. `doc_create` (no approval; the name is the model's title) makes
the folder one component at a time and refuses a `.claudinio` or `docs` that is
a link or a file, treats any entry at a candidate name — a dangling link
included — as taken, and opens the file with `create_new`. One process-wide
lock around every read-modify-write (parallel subagents and the app's save
share the file), and a memory beside the session (`<session>.doc.json`: the
text last seen, the sections a person changed that no agent has read since,
and the changes not yet told to the model). It is not in the JSONL because
that file is replayed into every request. Every read first compares the file
with that memory: a section whose text changed is marked `human`, and an
agent's `doc_write` to it is refused with the current text until the agent has
read it (`doc_read`, or that refusal) — the human wins. The model carries the
outline, never the bodies: a run's user turn ends with a `<document path>`
block (`id · status · owner · topic`, beside `<tasks>`), every `doc_*` write
answers with it, and a person's edit found after a round joins that round's
persisted tool-results message as one `<doc-changed>` note (ids and topics)
— never a per-request injection, which would rewrite the cached prefix. A
summarizing compaction records the outline as the file stands then
(`Compacted.document`, `doc_session::outline_block`) and
`history_from_records` restates it after the `<tasks>` block, under the same
rules as the list.
`doc_create` is core in a Standard main session and answered by the loop; the
other three tools are deferred but never listed, loaded by a `doc_create` that
leaves the session with a document, in the same round (one cache break; a
failed one loads nothing), or seeded as loaded when a session starts with a
document (a successor's `tools_loaded` record). A first `doc_create` takes at
most `MAX_SECTIONS` (6) topics and replaces the session's task list, keeping
only its golden tasks (`drop_checklist`); `tasks_set` is refused from then on.
`doc_create` in a session that already has a document hands off (`HandoffReason::NewDocument`,
`HandoffSpec.document`): the successor owns the new file and starts its memory
at its first read (the old session's `<session>.doc.json` is left alone; its
first run — the one the handoff starts, and the first run of a session a
context handoff opened from it — may not hand off again, since the app
follows every handoff and the successor's first message repeats the request;
`doc_session::new_document_turn` walks `context_handoff` links back while each
session is in its first run),
and
of the old task list it keeps only the golden tasks, with the golden loop's
counters (`transition::tasks_for_successor`); any other successor carries the
predecessor's document and its memory. A subagent spawned with `section` gets that section claimed for it
(`doing`, owner `sub:<name>` with only letters, digits, `_` and `-` kept, so
a name cannot close the harness's comment; one held by another agent is not
given), `doc_write` for it alone and `doc_read` for any section, so it can
build on a finished sibling; every subagent context is
`DocAccess::Section` or `::None`, never the parent's `Main`. No subagent
outlives the run that spawned it, so a run's start frees any section still
`doing` under a `sub:` owner. A freed section the subagent never touched goes
back to the status and owner it had before the claim (kept in the memory's
`claimed`); one it wrote to without finishing goes to `todo`. `edit_file` refuses the linked document: an
agent's edit would read as the person's and lock the agent out of its own
section. The UI hears of
every write and every detected edit through `AgentEvent::DocumentUpdated`
and loads the panel with `get_document` (polled every 5 s; it finds the link
with `persist::linked_document_at`, which parses only the `document_linked`
lines, not the whole session); `save_document` merges a person's
save from the app with what agents wrote while the editor was open
(`document::merge_human_save`) — every app save of that file, however the
editor was opened on it (App decides with `isLinkedDocument` from the
document the panel reports). A model-written body that leaves a code fence
open is closed by the harness, or the fence would swallow the sections after
it. Both commands take a session id only as a
plain file name of an existing session (`doc_session::session_file`), and
write only the document that session links. Beside the document panel (its
sections, each with a status chip and an owner chip), the Task Panel shows a
session without a document its list as rows, a status chip and a title each,
and under a document only the golden goals, headed *Goals*. The *Documents*
button beside the chat lists the workspace's `.claudinio/docs/*.md`
(`list_documents`).

The prefix only stays cheap if the provider caches it. OpenAI-protocol
servers cache automatically, but on the Anthropic Messages API a prefix is
cached only up to an explicit `cache_control` breakpoint, and a request without
one is re-read cold every round. `agent/provider/cache.rs` marks the
serialized request of `provider::stream_message` (so the loop, the handoff
document and subagents alike): `system` goes out as one text block carrying the
marker, which covers the tool list too since tools precede it in the cache
order; and the last eligible block of the last message and of the most recent
earlier user message carry a moving pair, so each request reads what the
previous one wrote even when a round of parallel tool calls adds more blocks
than the API looks back over. Never more than three markers, never on a
thinking block or an empty text block, and `Message`/`ContentBlock` are left
alone — they are also the session format. `wants_cache_markers` decides who
gets them: providers configured with the Anthropic protocol only (the built-in
OpenRouter sign-in stores its provider with the OpenAI protocol, so it is not
marked), never a local engine or a loopback host,
and **never claudin.io** — its proxy runs a prompt manager that rewrites the
system prompt server-side and its upstreams cache implicitly (it measures ~96%
of executor input as cache reads), so changing the shape of `system` there
would be production risk for no gain. A marked request refused with a 400 is
retried once without markers, and the provider is remembered for the rest of
the process when the error names `cache_control` or the retry succeeds. What
goes into the prefix is deterministic for that reason: skills, plugins and
specs are scanned in path order, MCP servers are connected by name and their
tools enter the list by server then tool name, and nothing in it carries a
timestamp — `prompt.rs`'s `prefix_stability_tests` build it twice and compare
bytes. Each time the harness rewrites a history the provider had cached (an
accepted prune, a compaction, a context handoff) or grows the tool list (a
deferred-tool load) it appends a `cache_break` record, so the cost of a
rewrite is countable.

Each run writes a `run_config` record (model, window, lines, prefix by part,
profile), each subagent a `subagent_run`, and every provider request a `usage` record (round, model, uncached input,
cache-read, cache-creation and output tokens, the provider's cost when it
reports one, and which subagent asked; OpenAI's `prompt_tokens` includes its
`cached_tokens`, so the record subtracts them and the three input kinds add up
to all input on either protocol). A subagent's own turns and usage go to
`<session>.sub/<parent_tool_use_id>-<n>.jsonl` beside the session file, not
into it: the session file is the model's history and is replayed into the next
request. A compaction's summarizer is a subagent too and writes
`<session>.sub/compact-<n>.jsonl` (a fresh file per attempt). Jev's calls are
billed into the session total but leave no `usage` record. `scripts/session-stats.py` aggregates them (`--cost` for cost, rounds
and cache share per run, and cache breaks per session).

### Project memory

`agent/project_memory.rs` (lean-harness Phase 9, decision D6). Two files a
new session reads, one the harness writes:

- **`<workspace>/.claudinio/MEMORY.md`**, harness-owned: a `## Learned` list of
  `- YYYY-MM-DD — <line> (plan: <relative document path>)`, or without the
  pointer when the session has no document. The core `remember` tool is its
  only writer (single-timeline plan, decision S4): its `lines` (at most 5 of at
  most 200 characters, checked by `proposed_lines` before anything is written)
  are appended, pointing at the session's living document when it has one; no
  call, no entry — no journal is ever copied there. Append-only, capped at 4,000 characters by
  dropping the oldest entries whole (`appended`); never summarized or
  rewritten, the ACE paper's brevity bias and context collapse. Written by
  rename, never through a link at `.claudinio` or the file.
- **`AGENTS.md`, else `CLAUDE.md`**, at the workspace root: the user's own
  context, which the harness reads and never writes (the agent may edit it like
  any other file, through an approved `edit_file` or the shell), capped at 6,000 characters on a character
  boundary (snapped to a line break in the second half) with a note naming the
  absolute path and the `start_line` to `read_file` the rest.

`run_loop` reads the instructions file on the first run of every session (no
`user` record yet) and the memory only on the first run of a session that
continues none (no `linked_from` record either; `is_first_run`), under a chat
profile, and
puts them in the run's user turn after the user's words and before the
`<tasks>` recital, as `<project-instructions path="...">` then
`<project-memory>` — the same mechanism as `<hook-context>` and `<tasks>`:
persisted inside the turn, so a reload replays it, and never in the system
prompt, whose bytes must not move under the cache. A successor (a context handoff, a
new document's session) gets the instructions, which its handoff does not copy
— the handoff is told to point at files — and not the memory, whose content
its predecessor already had. The git operator gets neither. A
summarizing compaction does not restate the instructions; the turn that carried
them survives only in the kept tail.
`AgentConfig::project_memory` (default on; Settings › Agent › Project memory)
turns both off. A link out of the workspace is not followed (a cloned repo
cannot send another file to the provider), files are read at most 1 MiB, and
a forged closing tag is defused with a zero-width space. The timeline never
shows either block: the bubble is the raw `user` record.

## Trust boundaries

Three, and they are the parts to be careful with.

**1. What the model may touch.** `agent/tools/mod.rs::validate_path` gates every
file tool (the read tools go through `validate_read_path`, which also opens the
user's skill folders and the harness's own saved outputs, with links resolved
first — never for writing); `agent/permissions.rs` gates `bash` with an allowlist/denylist and an
approval prompt. `edit_file` always shows a diff first. It also creates files
(lean-harness Phase 3b): an empty `old_string` on a path that does not exist
proposes the whole file as a diff from nothing, and applying it makes the
missing folders and writes with `create_new`, so it never overwrites. The
workspace and spec guards run first, as for any edit. A relative path is
joined onto the workspace (`tools::in_workspace`) before it is checked, so the
file a tool opens is the one the guard approved — `std::fs` alone would open it
from the process's directory (`$HOME`, or `/` from Finder).
Read-before-edit (`ReadTracker`) has nothing to check on a missing path, and
records a created file as fully read. An `old_string` that occurs more than once
is refused with the count unless the call sets `replace_all`, and the count is
taken again on the file at approval; a `replace_all` proposal carries its count,
and the approval card says "Replaces all N occurrences".
The project memory (`.claudinio/MEMORY.md`) is written by `remember` alone,
whose lines are checked and capped and land in a list the harness appends to,
so the file's shape stays the harness's:
`validate_editable_path` refuses it at the path level (resolved like the
workspace guard, links followed, compared without case), which covers
`edit_file` and a subagent's apply. The path guard is the exact one; the shell check
below is defence in depth, a best-effort heuristic and not a boundary:
`permissions::bash_call_permission` *denies* —
rather than asks about — a `bash` call that names it and is not a plain read,
so neither YOLO nor a hook's `allow` opens it. The shell check
(`project_memory::mentions_memory`) is a raw scan of the text, in any case,
with no parsing and no recursion: `memory.md` anywhere, or `.claudinio` not
continued into another name by a letter, digit, `-` or `_` (directly or after
dots, so `.claudinio.json` is not it) — a brace list (`{.claudinio,}`,
`.claudinio{,}`) or a Windows backslash or trailing dot counts — unless `/`
follows with a path of plain names that ends cleanly and has no empty or
all-dots part (`.claudinio/docs/x.md`); `.claudinio/`, `.claudinio/M*`,
`{.claudinio/,}` and `.claudinio/docs/..` count — scanned as written and
again with quotes, backslashes and cmd's escape `^` dropped. It reads the whole call (`shell_call_names_memory`): the command, the
`stdin`, a `workdir` inside `.claudinio`, and, with the workspace known, the
names resolved on disk like the path guard does, so a link an earlier call
made (`ln -s .claudinio c`, then `>> c/M*`) is seen. A plain read —
`cat`/`head`/`tail`/`ls`/`wc`/`stat`/`file`/`grep` or `git
log`/`diff`/`show`/`status`/`blame`, no pipe, under the shell rule's first
clause, nothing the write check sees (`permissions::plain_memory_read`) — is
not refused. It cannot see a path the command builds at run time (a variable,
a script assembling the name), a glob or brace list that spells the folder's
own name (`.c*`, `.claudi{n,}io`), a name relative to a folder the command
moves into first (`cd .claudinio/docs && cp x ../M*`; `pushd`, `git -C`,
`find -execdir` alike), a copy or delete of a parent folder (`rm -rf .`), or a
hard link to the file; each of those asks
(`what_the_memory_guard_cannot_see_still_asks`). It errs the other way too: a
command that only names the folder in text (`echo '.claudinio/' >>
.gitignore`, `git commit -m "ignore .claudinio"`, `git add
.claudinio/docs/*.md`) is refused, a known cost of the raw scan. Nothing else stands there for certain: the allowlisted interpreters
(`node `, `python `) run a script unasked even outside YOLO, and YOLO waives
the approval of the rest; a file written by an MCP server is
outside it too. Note the memory is no more trusted than `AGENTS.md`, which the
agent may edit and which is also read at a session's start — the guard keeps
the harness's file the harness's, it is not a sandbox.

**2. What the webview may touch.** The chat renders untrusted content, so treat
the frontend as a place where attacker-influenced script could run. The
filesystem commands in `commands/fs.rs` therefore enforce workspace containment
themselves — they do not rely on the frontend asking nicely, and refuse a
relative path, which `std::fs` would open from the process's directory. Both guards call
`workspace_path.rs` so they cannot drift apart. Writes outside the workspace go
through `export_file`, which opens the save dialog *in Rust* so the destination
is never an IPC argument.

**3. What becomes DOM.** `src/lib/markdown.ts` is the only markdown→HTML path.
It sanitizes with an allowlist before anything reaches `innerHTML`, because
`marked` deliberately passes raw HTML through and Tauri always exposes
`window.__TAURI_INTERNALS__.invoke` to whatever runs in the page. The CSP in
`tauri.conf.json` (no `script-src 'unsafe-inline'`) is the second layer.

**4. What comes back from a web page.** The browser tools put third-party
content into the model's context, and a page can contain text written to
manipulate it. `browser/page.rs::wrap_untrusted` labels every such payload as
data and neutralizes a forged closing tag; the system prompt tells the model to
treat it as evidence, never instructions. That is mitigation, not a guarantee —
the load-bearing part is that the browser tools cannot execute anything, and
that `bash` and `edit_file` still require approval.

**5. What a hook may do.** Lifecycle hooks (`agent/hooks/`) run programs named by
a config file, and a config file can arrive in a `git clone`. Two rules bound
them. Nothing spawns until the user has approved that exact set of resolved
commands, hashed in `~/.claudinio/hook-trust.json` — never in the repository,
where an approval could arrive pre-approved in a pull request. And a hook's
`PreToolUse` verdict feeds `tool_exec.rs::effective_permission`, which treats an
`allow` exactly as it treats YOLO mode: it skips the approval prompt and still
runs the bash denylist and the browser scheme check. A hook may relax a prompt;
it may never relax a policy.

`SECURITY.md` states what each boundary guarantees and what is explicitly out of
scope.

## Browser

`browser/` drives a dedicated Chromium over the DevTools Protocol so the agent
can see and operate a running page. It knows nothing about the agent; the
adapter is `agent/tools/browser.rs`.

- `provision.rs` — downloads a pinned Chrome for Testing build, verified by
  sha256 against a table generated by `scripts/pin_chromium.py`. Never the
  user's own Chrome profile: it holds live session cookies, and an agent with
  those plus one hostile page is credential exfiltration. Even when configured
  to use an installed browser, a fresh process is launched against our own
  `--user-data-dir`.
- `launch.rs` — spawn and teardown. `--remote-debugging-port=0` plus the
  `DevToolsActivePort` file, and a profile directory hashed per workspace, so
  two open workspaces cannot collide (sharing one profile makes the second
  Chromium hand off its URL and exit, which presents as an unexplained launch
  timeout). A PID file lets the next run reap a browser orphaned by an app
  crash, and only when the live process's executable still matches ours.
- `cdp.rs` — one WebSocket, JSON-RPC demultiplexed by `id`, events fanned out on
  a broadcast channel. The frame size cap is raised well above tungstenite's
  16 MiB default, which a full-page screenshot exceeds in a single message.
- `page.rs` — the attached tab: navigation, interaction, and the event pump that
  fills the buffers. Clearing a field uses the element's own `select()` rather
  than a synthesized Cmd/Ctrl+A, because that shortcut is handled by the
  browser's UI layer and never reaches the renderer.
- `buffers.rs` — bounded console and network history with a read cursor, filled
  independently of tool calls so nothing logged during a page load is lost.
- `screenshot.rs` — the four capture modes. `Emulation.setDeviceMetricsOverride`
  pins `deviceScaleFactor: 1` so CSS pixels equal output pixels and the geometry
  is reproducible across machines; `DOM.getBoxModel` quads are viewport-relative
  and get shifted into document coordinates, which is what `clip` expects once
  `captureBeyondViewport` is on.

The download is never triggered by a tool call — it is ~190 MB of the user's
bandwidth, so the tool fails with an actionable message and the user starts it
from Settings.

## Local models

`llama/` runs models on the user's own machine. It knows nothing about the
agent: the agent reaches it through the ordinary provider path, because
`llama-server --jinja` speaks the same OpenAI-compatible
`/v1/chat/completions` that `agent/provider/openai.rs` already speaks to remote
providers. A local model is just the id `local/<key>`.

- `provision.rs` — downloads a pinned `llama-server` from the upstream GitHub
  release, verified by sha256 against a table generated by
  `scripts/pin_llama.py` (the release API publishes sizes but no checksums).
  The archive's single top-level directory is flattened away so the binary and
  its `libggml*` siblings end up in one directory, which is what `@loader_path`
  and `$ORIGIN` need. CUDA is deliberately not offered: it needs a second
  ~390 MB runtime archive unpacked alongside, and Vulkan already covers NVIDIA.
- `supervisor.rs` — one server process per loaded model, in a global registry
  rather than app state, because `provider::stream_message` reaches inference
  with `&AgentConfig` alone. Each server binds loopback on a probed port, gets a
  random per-process api-key, and runs with `--no-webui`. That key is a trust
  boundary, not hygiene: without it any local process — including a page the
  agent's own browser visits — could POST to the port for free inference and a
  prompt-injection oracle. Readiness is `/health` on a deadline derived from the
  file size, since a 20 GB model off a cold page cache genuinely takes minutes.
- `catalog.rs` / `hf.rs` — GGUF weights and where they come from. Unlike every
  other pinned artifact in the app, the checksum is not committed: the Hugging
  Face tree API returns each LFS object's `oid`, which *is* its sha256, so
  integrity is verified against upstream live. The catalog entry is written only
  after every shard verifies, which makes it the ready marker.
- `Engine` (in `mod.rs`) — llama.cpp or MLX. MLX is Apple Silicon only and is
  the default there, because it generates measurably faster on the same weights
  (277 → 353 tok/s on an M2 Max with Qwen3-0.6B 4-bit). Every read goes through
  `LocalPrefs::effective_engine`, which falls back to llama.cpp when the
  configured engine is not available: `config.json` syncs between machines, and
  "mlx" arriving on a Windows box should degrade rather than fail every request.
  The engine also decides the model format — GGUF is one file, an MLX model is a
  whole repository — so it selects which catalog applies.
- `hardware.rs` — RAM/VRAM detection behind the admission check. A model larger
  than memory does not fail with a dialog; it swaps the machine to a standstill,
  so both the download button and the server start refuse it.

The MLX engine is a sidecar of our own, `claudinio-mlx`, built on Apple's
`mlx-swift` and released from its own repository so the Metal build (which needs
`xcodebuild` and a separately downloaded Metal toolchain) never enters this
repo's release path. It is pinned by version and sha256 like every other
artifact: being ours says nothing about the network it arrives over. It exists
rather than Apple's Python `mlx_lm.server` for three reasons — no interpreter to
provision, an api-key that `mlx_lm.server` simply does not have, and weights
loaded from a directory this app has already verified. Tool-call parsing is
*not* reimplemented; it comes from `mlx-swift-lm`'s per-family parsers, which is
the part that fails silently when rewritten.

Neither runtime nor any model is ever fetched by a tool call — one is tens of
megabytes and the other tens of gigabytes of the user's bandwidth.

## Code intelligence

`code_intel/` builds and serves the index; nothing in it knows about the agent.

- `parser.rs` — tree-sitter across 77 grammars → symbols, signatures, doc
  comments, call relations.
- `db.rs` — SQLite. Symbols and an FTS5 table (`chunk_fts`) for BM25, plus
  embedding rows.
- `embeddings.rs` — `all-MiniLM-L6-v2` through ONNX Runtime, in-process. Code is
  never sent anywhere to be indexed. The model ships as a Tauri resource.
- Search is **hybrid**: BM25 and vector results fused by reciprocal rank fusion.
  Keyword matching finds an exact identifier; the vector side finds a thing by
  description. Neither alone was good enough.
- `watcher.rs` — debounced file watching keeps the index live, sharing
  `INDEX_SEMAPHORE` so a background reindex cannot pile onto a foreground one.

## Frontend

SolidJS, no store library — signals and props.

- `App.tsx` owns workspace-level state (open workspaces, config, modals).
- `components/ChatPanel.tsx` owns one conversation: the event stream, the
  timeline, the composer.
- `components/chat/TimelineRows.tsx` holds the leaf renderers — one component
  per row type. Presentational, props in, no session state.
- `lib/chatRecords.ts` is the pure `SessionRecord[] → ChatMessage[]` translation
  plus the small formatters. Pure so it is directly testable; it used to live
  inside ChatPanel and be *copied* into the test file, which meant the tests
  passed against code that did not ship.
- `lib/ipc.ts` is the single typed wrapper over `invoke`. Nothing else calls
  `invoke` directly.

The frontend makes no network requests of its own. Every provider call happens
in Rust, which is why the CSP can keep `connect-src` closed.

## The eval

`src-tauri/src/eval/` is a headless runner over the real agent loop, exposed as
the `claudinio-eval` Cargo example (not a `[[bin]]`, which the Tauri bundler
would pick up). It lives in the crate because it drives the private `agent`
module, and nothing in it is reachable from the IPC surface. A task (YAML under
`eval/tasks/`) is materialized into a fresh directory outside any repository,
given one `start` commit and a `.claudinio.json` whose quality `test_cmd` is the
task's check; then `run_workflow` runs one fresh session with a
`Channel::new` closure as the event sink, so no Tauri window is involved, and
the task's oracle decides pass or fail. Approvals and `ask_user` are answered
by `eval::policy`: edits yes (never inside `.git/` or to the runner's
`.claudinio.json`), read-only shell yes, every other shell command no (the
runner turns on `ToolContext::shell_gate`, which only ever adds a prompt, so
allowlisted commands reach the policy too). The policy prevents accidents; it
is not a sandbox, since the task's tests run model-written code, so `run`
belongs in a disposable container. Metrics are read back from the
session files the run wrote — the same `usage` records and `.sub/` transcripts
a user's sessions have. `validate` needs no model: each oracle must fail on the
start tree and pass on the reference. See `eval/README.md`.

## Things that look odd and are deliberate

- **English only, everywhere.** No translation layer. The system prompts are
  written and tuned in English and the agent asks users to write in English;
  a localized shell around that was worse than being direct. See the README.
- **`docs/plans/`** holds one design document per feature, written before the
  code. They are historical decision records, not living specs — see the README
  in that directory.
- **`src-tauri` is the Cargo workspace root.** There is no manifest at the
  repository root.
- **Portuguese test fixtures** in `agent/prompt.rs`, `agent/judge.rs` and the
  other modules split out of `session.rs` are verbatim captures of
  sessions that broke something. Translating them would destroy the regression.
- **`ContentBlock` is `#[serde(untagged)]` and its variant order matters.**
  Deserialization takes the first structural match, and the same enum is both
  the wire format and the session JSONL format, so reordering it silently
  changes how archived sessions reload. `ToolResultContent` is untagged for the
  same reason: it lets a `tool_result` carry either a bare string (what every
  tool wrote before images existed) or an array of blocks, with no migration.
- **List results are lines, not JSON.** `list_dir`, `grep`, the symbol searches
  and `file_outline` answer as `path:line kind signature` rows
  (`agent/tools/lines.rs`). A result is carried on every later request, and
  the JSON these replaced cost about three times as much for the same rows.
  `tool-renderers/lineResults.ts` parses the same formats back for the chat;
  the two files change together. Sessions recorded earlier still hold JSON,
  and the renderer reads both.
- **A `read_file` result in history can be a one-line pointer.** When the last
  read of the same file and range is still in the conversation and identical,
  the new result enters history as `[unchanged: …]` instead of a second copy.
  The live tool card shows the file; a reloaded session shows the note,
  because the note is what the model was given.
- **Images in session files live in `media/`, not in the JSONL.** A screenshot
  is ~200 KB of base64 and the session file is re-read on every message, so
  `persist.rs` swaps oversized payloads for a content-addressed reference on
  write and reads them back on load. A reference whose file is gone degrades to
  a text note rather than failing the request.
