# Hooks

Claudinio Code runs programs you choose at fixed points in a session — before a
tool call, on every prompt, when the context is about to be compacted, when the
run is about to end. It implements **Claude Code's hook protocol**, field for
field, so a `hooks` block written for that harness works here without being
edited. Codex reimplemented the same wire format (its engine is literally named
`ClaudeHooksEngine`) and Gemini CLI reads the same `hookSpecificOutput`, so one
config covers all of them.

The protocol is deliberately not improved. Where Claude Code is specific we
match it; where it is silent we accept and ignore rather than reject.

## Where hooks are declared

Six places. **Every source contributes; none overrides another** — a guard in
your home settings and a guard shipped by the repository both run.

| Source | Path |
|---|---|
| User settings | `~/.claude/settings.json` → `hooks` |
| App settings | Claudinio's own `config.json` → `hooks` |
| Plugins | a plugin's `hooks` manifest field, or its `hooks/hooks.json` |
| Workspace config | `<project>/.claudinio.json` → `hooks` |
| Project settings | `<project>/.claude/settings.json` → `hooks` |
| Local settings | `<project>/.claude/settings.local.json` → `hooks` |

The same command declared twice runs once; the settings panel says where else it
came from. Plugins are found the way Claude Code finds them — a `hooks/hooks.json`
inside the package, whether or not the manifest mentions it — because the
plugins worth supporting are ones you did not write and should not have to edit.

## Nothing runs until you approve it

A hook is arbitrary code a repository can ship, and `.claude/settings.local.json`
is gitignored in most projects precisely so it can hold things nobody reviewed.
So Claudinio discovers, lists and displays hooks immediately, and **spawns
nothing** until you have approved that exact set once, in Settings → Hooks or
from the banner in the thread. The approval is a SHA-256 over the resolved
commands: editing a command revokes it, renaming a spinner label does not. It is
stored in `~/.claudinio/hook-trust.json`, never in the repository — a repo-local
approval file could arrive pre-approved in a pull request.

Approving mid-run takes effect on the very next event; no restart.

## The nine events

| Event | Fires | Can |
|---|---|---|
| `SessionStart` | a run begins (`startup`, `resume`, `clear`, `compact`) | add context |
| `UserPromptSubmit` | before your text becomes a turn | add context, refuse the prompt |
| `PreToolUse` | before a tool call is dispatched | `allow`, `ask`, `deny` |
| `PostToolUse` | after a tool returns | feed the model a correction |
| `Notification` | the agent is waiting on you | — |
| `Stop` | the run is about to end | refuse, and the run continues |
| `SubagentStop` | a subagent is about to finish | refuse, and it continues |
| `PreCompact` | context is about to be compacted or handed off | add context |
| `SessionEnd` | the conversation is cleared, you log out, or the app quits | — |

Hooks fire for subagents too. A guard that only watched the main loop would be a
guard the model can walk around by delegating.

## Configuration

```jsonc
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [
          { "type": "command", "command": "$CLAUDE_PROJECT_DIR/.claude/guard.sh", "timeout": 10 }
        ]
      }
    ]
  }
}
```

`args` (argv, no shell) and `statusMessage` (the spinner label) are supported
alongside `command` and `timeout`. `timeout` is in seconds and defaults to 60.

### Matchers and tool names

Claude Code calls its tools `Bash`, `Edit`, `Read`, `Task`. Claudinio calls them
`bash`, `edit_file`, `read_file`, `spawn_agents`. A matcher may use either:

| Matcher | Selects |
|---|---|
| `Bash` | `bash` |
| `Edit`, `Write`, `MultiEdit`, `NotebookEdit` | `edit_file` |
| `Read` | `read_file` |
| `Glob`, `LS` | `list_dir` |
| `Grep` | `grep` |
| `Task` | `spawn_agents` |
| `WebSearch` | `web_search` |
| `TodoWrite` / `TodoRead` | `tasks_set` / `tasks_get` |
| `AskUserQuestion` | `ask_user` |
| `ExitPlanMode` | `exit_plan_mode` |
| `mcp__server__tool` | itself — Claudinio uses the same prefix |

Tools with no Claude Code counterpart (`browser`, `run_quality`, `code_search`,
`semantic_search`, `symbol_lookup`, `file_outline`, `go_to_definition`,
`find_references`, `write_plan`, `finalize_plan`, `enter_plan_mode`) are matched
by their own names.

Matching is a case-sensitive, **unanchored** regex, as in Claude Code. Absent,
empty and `*` all mean everything. The sharp edge is inherited rather than
introduced: `Write` also selects `TodoWrite`, so an edit guard also guards the
task list. Anchor it (`^Write$`) to opt out. `PreCompact` matchers are
`manual|auto`; `SessionStart` matchers are `startup|resume|clear|compact`.

The Settings panel lists, for every hook, exactly which tools its matcher will
hit — because the characteristic hook bug is a config that installs cleanly,
runs on every prompt and matches nothing.

## What a hook reads on stdin

```json
{
  "session_id": "…",
  "transcript_path": "/project/.claudinio/sessions/<id>.jsonl",
  "transcript_format": "claudinio-jsonl/1",
  "cwd": "/project",
  "hook_event_name": "UserPromptSubmit",
  "prompt": "what is the port"
}
```

Per event, in addition: `tool_name` + `tool_input` (`PreToolUse`), plus
`tool_response` (`PostToolUse`), `prompt` (`UserPromptSubmit`), `message`
(`Notification`), `stop_hook_active` (`Stop`, `SubagentStop`), `trigger` +
`custom_instructions` (`PreCompact`), `source` (`SessionStart`), `reason`
(`SessionEnd`).

`tool_name` carries the Claude Code name where one exists (`Edit`, `Bash`,
`Task`), so a published hook's `case "$tool_name"` works. The native name always
rides alongside as `claudinio_tool_name`, and `tool_input` gains `file_path`
next to `path` for the same reason. Nothing standard is renamed or removed.

**One real difference.** `transcript_path` points at a complete transcript in
Claudinio's own `SessionRecord` JSONL format, not Claude Code's. A hook that only
locates the file is served correctly; one that parses transcripts can read
`transcript_format` and bail rather than misread. Shipping a translator was
rejected: it would be a second serializer of an undocumented format, written on
every prompt, for a field almost nothing reads.

## Environment

| Variable | Value |
|---|---|
| `CLAUDE_PROJECT_DIR` / `CLAUDINIO_PROJECT_DIR` | absolute workspace root |
| `CLAUDE_PLUGIN_ROOT` / `CLAUDINIO_PLUGIN_ROOT` | plugin package root (plugin hooks only) |
| `CLAUDINIO_HOOK_EVENT` | e.g. `UserPromptSubmit` |
| `CLAUDINIO_SESSION_ID` | the session id |
| `PATH` | your login shell's PATH |

`${CLAUDE_PLUGIN_ROOT}` and `${CLAUDE_PROJECT_DIR}` are expanded in `command` and
in every entry of `args`, before the trust hash is computed, so what you approve
is what runs.

The credential-prompt bridge is deliberately **not** passed to hooks. A hook is
not `git`, and letting config-declared code raise the credential dialog is a
phishing surface with no compensating use.

## What a hook says back

**Exit code 0** — success. stdout starting with `{` is parsed as JSON; anything
else is plain text, which becomes context for `UserPromptSubmit` and
`SessionStart` and is transcript-only everywhere else.

**Exit code 2** — blocking. stderr is the message. It blocks for `PreToolUse`
(the tool never runs), `PostToolUse` (the model is told), `UserPromptSubmit` (the
prompt is refused), `Stop` and `SubagentStop` (the run continues). For the four
observational events it is a warning, not a refusal.

**Any other exit code** — a problem you see and the agent ignores. A missing
binary, a crash, a timeout and unreadable output all land here. This is what
keeps a hook whose program is not installed from becoming an error on every
prompt.

### JSON

```json
{
  "continue": false,
  "stopReason": "…",
  "suppressOutput": true,
  "systemMessage": "…",
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow" | "deny" | "ask",
    "permissionDecisionReason": "…",
    "additionalContext": "…"
  }
}
```

The older `{"decision": "approve" | "block", "reason": "…"}` spelling is
accepted. **Unknown fields are ignored, never rejected** — a parser that refuses
what it does not recognise is a parser that refuses every future field.

Hooks in a batch run in parallel; `deny` beats `ask` beats `allow`, and
`continue: false` outranks all of them.

### What a hook cannot do

**Hooks may relax a prompt. They may never relax a policy.** A `PreToolUse`
`allow` skips the approval dialog exactly as YOLO mode does — and, exactly as
YOLO mode does, it still hits the bash deny-list and the browser scheme check.
An `allow` on `sudo rm -rf /` is still denied. Brain mode stays read-only. An
`ask` can only tighten: it turns an automatic tool into a prompt and can never
un-deny anything.

A `Stop` hook that refuses is told `stop_hook_active: true` on its next call. If
it refuses anyway, it gets three tries and then the run ends.

## Seeing what happened

Every run is a row in the timeline — the `statusMessage` while it runs, then its
status, exit code and duration — and a line in the session JSONL, including runs
that were skipped for lack of approval. Context a hook injected is its own row,
so "the brain added five facts to your prompt" is visible rather than an
unexplained bulge in your message.

Settings → **Hooks** lists every hook with its source, its resolved command,
what its matcher will hit, and a **Run now** button that executes it against a
synthetic payload and prints the exit code.
