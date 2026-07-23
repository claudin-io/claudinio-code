# Security Policy

## Reporting a vulnerability

**Do not open a public issue for security problems.**

Report privately through [GitHub Security Advisories](https://github.com/claudin-io/claudinio-code/security/advisories/new),
or email **security@claudin.io**.

Please include:

- what you can do with the bug (impact), not just where it is
- the version (`Claudinio Code → About`, or the tag you built from)
- your OS and architecture
- reproduction steps, ideally with a minimal workspace

You will get an acknowledgement within 72 hours and a fix or a plan within 14
days. Please give us 90 days before public disclosure. We will credit you in
the release notes unless you prefer otherwise.

## Supported versions

Only the latest released version receives security fixes. There are no
long-term support branches while the project is pre-1.0.

## Threat model

Claudinio Code is a **local desktop application** that runs an AI agent with
access to your filesystem and shell. Understanding what is and is not a
security boundary matters when deciding what to report.

### What we defend

| Boundary | Guarantee |
|---|---|
| Workspace containment | File tools canonicalize paths and reject anything outside the opened workspace (`validate_path` in `src-tauri/src/agent/tools/mod.rs`), including `..` traversal when a path cannot be canonicalized. Documented read-only exception: `~/.agents`, `~/.claudinio` and `~/.claude` skill directories. Write tools never get that exception. |
| Shell approval | `bash` requires explicit user approval, except for a read-only allowlist. A denylist blocks known-destructive commands (`src-tauri/src/agent/permissions.rs`). |
| Edit approval | Every `edit_file` is shown as a diff and applied only after approval. |
| Credentials at rest | API keys live in `config.json` under the OS config directory (`~/.config/claudinio-code` on Linux and equivalents elsewhere) — never in the workspace, never in session transcripts. They are stored as plaintext protected by file permissions, not in an OS keychain; a keychain backend is on the roadmap. |
| Local listeners | The askpass bridge and OAuth callbacks bind to `127.0.0.1` on an ephemeral port and are torn down after use. |
| Update integrity | Release artifacts are signed; the updater verifies signatures against a public key baked into the app. |

A bug that breaks any row above is a vulnerability. Report it.

### What we explicitly do not defend

- **Prompt injection is not fully solved.** Content in your workspace (or
  fetched by `web_search`) can influence the agent. The approval gates on
  `bash` and `edit_file` are the mitigation. Do not open untrusted workspaces
  in permissive settings and expect containment.
- **Approved actions are the user's.** If you approve a command, the agent runs
  it with your privileges. That is the product working as designed.
- **Secrets shipped in the binary are not secret.** The HMAC request signing in
  `src-tauri/src/agent/app_sign.rs` is anti-abuse friction for
  `api.claudin.io`, not a security boundary — this is documented in the source.
  Extracting it is not a vulnerability; the real controls are server-side
  (revocable keys, budgets, rate limits).
- **A malicious model provider.** If you point the app at a hostile base URL,
  you have granted a hostile server the agent's tool surface.
- **Local attackers with your user account.** Anyone who can already run code as
  you can read the config directory.
