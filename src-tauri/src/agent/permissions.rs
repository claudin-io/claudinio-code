use serde::Serialize;

/// `PartialEq` so a resolved permission can be asserted on directly: the
/// decision this enum carries is a security decision, and a test that has to
/// go through a Tauri channel to check it is a test nobody writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PermissionLevel {
    Auto,
    RequiresApproval,
    Denied,
}

pub fn tool_permission(name: &str) -> PermissionLevel {
    match name {
        "edit_file" | "batch_edit" | "write_file" => PermissionLevel::RequiresApproval,
        "bash" => PermissionLevel::RequiresApproval,
        // Read-only against a page the user already let the agent open. They
        // reach no new origin, so re-prompting would be friction with no
        // corresponding risk.
        "browser_screenshot" | "browser_inspect" => PermissionLevel::Auto,
        // Resolved per action and per URL by `browser_permission`.
        "browser" => PermissionLevel::RequiresApproval,
        n if n.starts_with("mcp__") => PermissionLevel::RequiresApproval,
        _ => PermissionLevel::Auto,
    }
}

/// Hosts that are this machine or its local network.
///
/// A dev server is what the agent is normally pointed at, and asking for
/// approval on every reload of localhost is friction with no safety payoff.
fn is_local_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host == "localhost"
        || host == "127.0.0.1"
        || host == "0.0.0.0"
        || host == "::1"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.starts_with("192.168.")
        || host.starts_with("10.")
        || host.starts_with("127.")
}

/// Split `scheme://host[:port]/...` into its scheme and host.
fn scheme_and_host(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // Strip any userinfo, then the port. IPv6 literals keep their brackets
    // until `is_local_host` trims them.
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = match authority.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or("").to_string(),
        None => authority.split(':').next().unwrap_or("").to_string(),
    };
    if host.is_empty() {
        return None;
    }
    Some((scheme.to_ascii_lowercase(), host.to_ascii_lowercase()))
}

/// Permission for one `browser` action.
///
/// Navigation is the only action that reaches the network, so it is the only
/// one gated. Everything else operates on a page the user already approved, and
/// prompting again per step would make the tool unusable.
pub fn browser_permission(action: &str, url: Option<&str>) -> PermissionLevel {
    match action {
        "navigate" => match url.and_then(scheme_and_host) {
            // A browser that can open file:// is a file reader that bypasses
            // the workspace containment every other tool obeys; chrome:// and
            // devtools:// reach the browser's own internals; javascript: and
            // data: are the script execution this tool deliberately omits.
            Some((scheme, _)) if scheme != "http" && scheme != "https" => PermissionLevel::Denied,
            Some((_, host)) if is_local_host(&host) => PermissionLevel::Auto,
            Some(_) => PermissionLevel::RequiresApproval,
            None => PermissionLevel::Denied,
        },
        // Everything else acts on a page the user already approved opening.
        // Prompting per click would make the tool unusable for the case it
        // exists for — driving a local dev server — and the meaningful gate is
        // the one on which origin gets opened at all.
        "reload" | "back" | "url" | "close" | "click" | "type" | "press" | "scroll_to"
        | "focus" | "wait_for" => PermissionLevel::Auto,
        // Unknown actions fail closed.
        _ => PermissionLevel::RequiresApproval,
    }
}

const BASH_ALLOWLIST: &[&str] = &[
    "git status",
    "git diff",
    "git log",
    "git branch",
    "git remote",
    "ls",
    "cat ",
    "head ",
    "tail ",
    "wc ",
    "npm run dev",
    "pnpm dev",
    "pnpm run dev",
    "cargo build",
    "cargo check",
    "cargo test",
    // Verification commands. The quality harness runs these through its own
    // executor, but the agent also reaches for them ad hoc while debugging a
    // failure, and stopping to ask permission mid-investigation is friction
    // with no safety payoff: they read and report, they do not deploy.
    // `cargo fmt` and `cargo clippy` are deliberately absent: matching is by
    // prefix, so allowlisting them would also allowlist `cargo clippy --fix`
    // and let a session rewrite files without the approval an edit gets.
    "cargo nextest",
    "cargo llvm-cov",
    "npx vitest",
    "pnpm exec vitest",
    "pnpm vitest",
    "npx jest",
    "pnpm exec jest",
    "npm test",
    "pnpm test",
    "yarn test",
    "pytest",
    "go test",
    "node ",
    "python ",
    "pnpm run",
    "which ",
    "echo ",
    "pwd",
    "date",
];

const BASH_BLACKLIST: &[&str] = &[
    "rm -rf /",
    "rm -rf /*",
    "rm -rf ~",
    "sudo rm",
    "dd if=",
    ":(){",
    "mkfs.",
    "fdisk",
    "format ",
    "> /dev/",
    "chmod 777",
    "chown ",
    "mv /",
    "cp /",
];

/// The allowlisted commands that only look: they read files or repository
/// state and print it. A subset of `BASH_ALLOWLIST` (a test pins that), minus
/// everything there that runs code (`node `, `python `, `cargo test`,
/// `pnpm run`, …), prints whatever it is told (`echo `), or mutates with
/// arguments (`git branch -D`, `git remote add`).
const READ_ONLY_PREFIXES: &[&str] = &[
    "git status",
    "git diff",
    "git log",
    "ls",
    "cat ",
    "head ",
    "tail ",
    "wc ",
    "which ",
    "pwd",
];

/// Whether `command` only reads, with nothing that could make it do more.
///
/// For a caller that approves commands with nobody watching — the eval runner,
/// which executes model-chosen commands on the host. Stricter than the
/// allowlist on purpose: the allowlist matches by prefix as a convenience for a
/// person who sees the command, so `ls && curl … | sh` "starts with" `ls`.
/// Here any shell control character (chaining, pipes, substitution,
/// redirection, a second line) disqualifies the command outright, and so does
/// `--output`, the flag that makes `git diff`/`git log` write a file.
pub fn bash_is_read_only(command: &str) -> bool {
    let trimmed = command.trim();
    reads_by_shape(trimmed)
        && bash_permission(trimmed, false) == PermissionLevel::Auto
        && !bash_writes_files(trimmed)
}

/// One `READ_ONLY_PREFIXES` command with nothing that could chain, redirect
/// or substitute — the shape half of `bash_is_read_only`, without the
/// permission lookup, so `bash_permission` can use it too.
fn reads_by_shape(trimmed: &str) -> bool {
    if trimmed.is_empty()
        || trimmed.contains(['&', ';', '|', '`', '>', '<', '\n', '\r'])
        || trimmed.contains("$(")
        || trimmed.contains("--output")
    {
        return false;
    }
    READ_ONLY_PREFIXES.iter().any(|prefix| {
        trimmed == prefix.trim_end()
            || trimmed.starts_with(prefix)
                && (prefix.ends_with(' ') || trimmed[prefix.len()..].starts_with(' '))
    })
}

/// `bash_permission` for a session that edits files itself (Builder under the
/// Compact or Standard profile, `PromptProfile::edits_files`). Everywhere else
/// (Brain, the git operator) a command that writes a file is refused before
/// its permission is ever looked up, so the allowlist could be a list of
/// prefixes: `cat `, `echo `, `python ` read and report. Here `cat > src/main.rs`
/// reaches the tool, and a prefix that reads is no reason to let it write
/// unasked — an edit made through the shell is asked about exactly like one
/// made through `edit_file`.
pub fn bash_permission_writing(command: &str, auto_approve_git: bool) -> PermissionLevel {
    match bash_permission(command, auto_approve_git) {
        PermissionLevel::Auto if bash_writes_files(command) => PermissionLevel::RequiresApproval,
        other => other,
    }
}

/// Heuristic: does this bash command WRITE file contents? Used to keep Brain
/// and the git operator from editing files through bash, and to ask about
/// such a command in a session that edits (`bash_permission_writing`), the
/// way an `edit_file` is asked about. Deliberately targets content
/// editing — redirections, tee, in-place sed/perl, patch, and inline
/// interpreter scripts that write — not general filesystem ops (rm/mv/cp
/// stay governed by the normal approval flow).
pub fn bash_writes_files(command: &str) -> bool {
    // Strip redirections that don't produce file content before checking '>'.
    let stripped = command
        .replace("2>&1", "")
        .replace("&>/dev/null", "")
        .replace("&> /dev/null", "")
        .replace("2>/dev/null", "")
        .replace("2> /dev/null", "")
        .replace(">/dev/null", "")
        .replace("> /dev/null", "");
    if stripped.contains('>') {
        return true;
    }
    let lower = command.to_lowercase();
    if lower.starts_with("tee ") || lower.contains("| tee ") || lower.contains("|tee ") {
        return true;
    }
    for pat in ["sed -i", "gsed -i", "perl -i", "git apply", "patch "] {
        if lower.contains(pat) {
            return true;
        }
    }
    // Inline interpreter scripts that write files (python3 -c "...open(...,'w')...").
    let inline_script = ["python", "node", "ruby"].iter().any(|i| lower.contains(i))
        && [" -c ", " -e ", " <<"].iter().any(|f| lower.contains(f));
    if inline_script {
        for pat in [
            "open(",
            "write(",
            "json.dump",
            "writefile",
            "write_text",
            "file.write",
        ] {
            if lower.contains(pat) {
                return true;
            }
        }
    }
    false
}

/// The permission of a whole `bash` call — `bash_permission`, or
/// `bash_permission_writing` for a session that edits files itself, of its
/// `command`, and refused when the call names the project memory through its
/// `workdir`, its `stdin` or a link on disk under `root`
/// (`project_memory::shell_call_names_memory`) and is not a plain read. The
/// command text alone is not the call: `{"command": ">> MEMORY.md",
/// "workdir": ".claudinio"}` writes the memory with a command that never
/// names its folder (Phase 9 review).
pub fn bash_call_permission(
    input: &serde_json::Value,
    root: Option<&std::path::Path>,
    auto_approve_git: bool,
    writing: bool,
) -> PermissionLevel {
    let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
    if crate::agent::project_memory::shell_call_names_memory(root, input)
        && !reads_by_shape(command.trim())
    {
        return PermissionLevel::Denied;
    }
    if writing {
        bash_permission_writing(command, auto_approve_git)
    } else {
        bash_permission(command, auto_approve_git)
    }
}

/// `bash_permission` for Brain, which plans and changes nothing: a general
/// interpreter or a project script (`node `, `python `, `pnpm run`, …) is a way
/// to change anything, so it is not Auto there even though the allowlist has
/// it. Builds and test runners stay — they also run project code, so this is a
/// narrowing, not a proof that a command is read-only (`bash_is_read_only` is
/// that, for the eval).
pub fn bash_permission_brain(command: &str) -> PermissionLevel {
    match bash_permission(command, false) {
        PermissionLevel::Auto
            if segments(command.trim()).is_some_and(|segs| {
                segs.iter().any(|seg| {
                    BRAIN_RUNS_NOTHING_BUT
                        .iter()
                        .any(|p| starts_with_word(seg, p))
                })
            }) =>
        {
            PermissionLevel::RequiresApproval
        }
        other => other,
    }
}

/// Allowlisted prefixes that run whatever they are given: off in Brain.
const BRAIN_RUNS_NOTHING_BUT: &[&str] =
    &["node ", "python ", "pnpm run", "npm run dev", "pnpm dev"];

pub fn bash_permission(command: &str, auto_approve_git: bool) -> PermissionLevel {
    let trimmed = command.trim();

    for pattern in BASH_BLACKLIST {
        if trimmed.contains(pattern) {
            return PermissionLevel::Denied;
        }
    }
    // The project memory is written by `finalize_plan` alone (lean-harness
    // Phase 9). Denied rather than asked about, so YOLO and a hook's `allow`
    // — which relax a prompt, never a policy — cannot open it; only a plain
    // read of it passes.
    if crate::agent::project_memory::named_in(trimmed) && !reads_by_shape(trimmed) {
        return PermissionLevel::Denied;
    }

    // Every command of a chain or pipeline must be allowlisted on its own: a
    // prefix vouches for one command, and `ls; rm -rf src` starts with `ls`.
    // Substitution and `--output` make even one allowlisted command do more.
    match segments(trimmed) {
        Some(segs)
            if segs.iter().any(|s| !s.is_empty())
                && segs
                    .iter()
                    .all(|seg| seg.is_empty() || allowlisted(seg, auto_approve_git)) =>
        {
            PermissionLevel::Auto
        }
        _ => PermissionLevel::RequiresApproval,
    }
}

/// `command` split into the commands a shell runs — at `&&`, `||`, `;`, `|`,
/// `&` and line breaks outside quotes — each trimmed. `None` for anything that
/// runs a command the split cannot see (`$(…)`, backticks, `<(…)`), for
/// `--output` (which makes `git diff`/`git log` write a file) and for an
/// unbalanced quote. The redirections that only merge or discard streams
/// (`2>&1`, `>/dev/null`) are removed first, so they do not split.
fn segments(command: &str) -> Option<Vec<String>> {
    if command.contains('`')
        || command.contains("$(")
        || command.contains("<(")
        || command.contains(">(")
        || command.contains("--output")
    {
        return None;
    }
    let mut text = command.to_string();
    for harmless in [
        "&>/dev/null",
        "&> /dev/null",
        "2>&1",
        "1>&2",
        ">&2",
        "2>/dev/null",
        "2> /dev/null",
        ">/dev/null",
        "> /dev/null",
    ] {
        text = text.replace(harmless, " ");
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => {
                quote = None;
                cur.push(c);
            }
            (Some(_), '\\') => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                cur.push(c);
            }
            (None, '\\') => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            (None, '&' | '|' | ';' | '\n' | '\r') => {
                if matches!(chars.peek(), Some(&n) if n == c && c != ';') {
                    chars.next();
                }
                out.push(cur.trim().to_string());
                cur.clear();
            }
            (None, c) => cur.push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    out.push(cur.trim().to_string());
    Some(out)
}

/// `segment` starts with the word(s) of `prefix`: `ls` matches `ls -la` but
/// not `lsblk`.
fn starts_with_word(segment: &str, prefix: &str) -> bool {
    segment == prefix.trim_end()
        || segment.starts_with(prefix)
            && (prefix.ends_with(' ') || segment[prefix.len()..].starts_with(' '))
}

/// One command (no chaining) that runs unasked.
fn allowlisted(segment: &str, auto_approve_git: bool) -> bool {
    if auto_approve_git {
        let lower = segment.to_lowercase();
        if ["git add", "git commit", "git push"]
            .iter()
            .any(|p| starts_with_word(&lower, p))
        {
            return true;
        }
    }
    // `git branch NAME` makes a branch and `git remote add` changes the
    // repository; listing them is what the allowlist is for.
    if starts_with_word(segment, "git branch") {
        return lists_only(
            &segment["git branch".len()..],
            &[
                "-a",
                "-r",
                "-v",
                "-vv",
                "--all",
                "--remotes",
                "--list",
                "--show-current",
                "--verbose",
                "--merged",
                "--no-merged",
                "--no-color",
                "--color",
            ],
        );
    }
    if starts_with_word(segment, "git remote") {
        let rest = &segment["git remote".len()..];
        return lists_only(rest, &["-v", "--verbose"])
            || rest
                .split_whitespace()
                .next()
                .is_some_and(|w| w == "show" || w == "get-url");
    }
    BASH_ALLOWLIST.iter().any(|p| starts_with_word(segment, p))
}

/// Every word of `args` is one of the listing flags in `allowed`.
fn lists_only(args: &str, allowed: &[&str]) -> bool {
    args.split_whitespace().all(|w| allowed.contains(&w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_allowlist_ls() {
        assert!(matches!(
            bash_permission("ls", false),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("ls -la", false),
            PermissionLevel::Auto
        ));
    }

    #[test]
    fn bash_allowlist_git_status() {
        assert!(matches!(
            bash_permission("git status", false),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("git status --short", false),
            PermissionLevel::Auto
        ));
    }

    #[test]
    fn bash_allowlist_git_diff() {
        assert!(matches!(
            bash_permission("git diff", false),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("git diff --cached", false),
            PermissionLevel::Auto
        ));
    }

    #[test]
    fn bash_allowlist_pnpm_dev() {
        assert!(matches!(
            bash_permission("pnpm dev", false),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("pnpm run dev", false),
            PermissionLevel::Auto
        ));
    }

    #[test]
    fn bash_allowlist_pnpm_run() {
        assert!(matches!(
            bash_permission("pnpm run build", false),
            PermissionLevel::Auto
        ));
    }

    #[test]
    fn bash_allowlist_cargo_build() {
        assert!(matches!(
            bash_permission("cargo build", false),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("cargo build --release", false),
            PermissionLevel::Auto
        ));
    }

    #[test]
    fn bash_blacklist_rm_rf_root() {
        assert!(matches!(
            bash_permission("rm -rf /", false),
            PermissionLevel::Denied
        ));
        assert!(matches!(
            bash_permission("rm -rf /*", false),
            PermissionLevel::Denied
        ));
    }

    #[test]
    fn bash_blacklist_sudo_rm() {
        assert!(matches!(
            bash_permission("sudo rm -rf /tmp", false),
            PermissionLevel::Denied
        ));
    }

    #[test]
    fn bash_blacklist_chmod_777() {
        assert!(matches!(
            bash_permission("chmod 777 /etc", false),
            PermissionLevel::Denied
        ));
    }

    #[test]
    fn bash_neutral_requires_approval() {
        assert!(matches!(
            bash_permission("git push origin main", false),
            PermissionLevel::RequiresApproval
        ));
        assert!(matches!(
            bash_permission("npm install", false),
            PermissionLevel::RequiresApproval
        ));
        assert!(matches!(
            bash_permission("pnpm add some-package", false),
            PermissionLevel::RequiresApproval
        ));
    }

    #[test]
    fn bash_neutral_git_commit() {
        assert!(matches!(
            bash_permission("git commit -m 'fix'", false),
            PermissionLevel::RequiresApproval
        ));
    }

    #[test]
    fn bash_whitespace_trimmed_before_check() {
        assert!(matches!(
            bash_permission("  ls  ", false),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("  rm -rf /  ", false),
            PermissionLevel::Denied
        ));
    }

    // ── auto_approve_git tests ──

    #[test]
    fn auto_approve_git_add_no_flag() {
        // Without the flag, git add requires approval.
        assert!(matches!(
            bash_permission("git add .", false),
            PermissionLevel::RequiresApproval
        ));
    }

    #[test]
    fn auto_approve_git_add_with_flag() {
        assert!(matches!(
            bash_permission("git add .", true),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("git add src/main.rs", true),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("git add", true),
            PermissionLevel::Auto
        ));
    }

    #[test]
    fn auto_approve_git_commit_with_flag() {
        assert!(matches!(
            bash_permission("git commit -m 'fix'", true),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("git commit --amend", true),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("git commit", true),
            PermissionLevel::Auto
        ));
    }

    #[test]
    fn auto_approve_git_push_with_flag() {
        assert!(matches!(
            bash_permission("git push origin main", true),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("git push --force-with-lease", true),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("git push", true),
            PermissionLevel::Auto
        ));
    }

    #[test]
    fn auto_approve_git_other_commands_unaffected() {
        // git status/diff are already allowlisted; flag doesn't change deny-list.
        assert!(matches!(
            bash_permission("rm -rf /", true),
            PermissionLevel::Denied
        ));
        // Non-git commands still need approval.
        assert!(matches!(
            bash_permission("npm install", true),
            PermissionLevel::RequiresApproval
        ));
        // git log is already allowlisted.
        assert!(matches!(
            bash_permission("git log", true),
            PermissionLevel::Auto
        ));
    }

    #[test]
    fn auto_approve_git_case_insensitive() {
        // The flag checks lowercase; uppercase input should still match.
        assert!(matches!(
            bash_permission("GIT ADD .", true),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("GIT COMMIT -m 'x'", true),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("GIT PUSH", true),
            PermissionLevel::Auto
        ));
    }

    #[test]
    fn auto_approve_git_whitespace_handling() {
        assert!(matches!(
            bash_permission("  git add .  ", true),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("  git commit -m 'x'  ", true),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("  git push  ", true),
            PermissionLevel::Auto
        ));
    }

    // ── bash_writes_files tests ──

    // Phase 9: a shell write to the project memory is refused outright, not
    // asked about — so YOLO and a hook's `allow`, which relax a prompt, do
    // not open it either. A plain read of it stays a read.
    #[test]
    fn a_shell_command_that_writes_the_project_memory_is_denied() {
        for c in [
            "echo '- planted' >> .claudinio/MEMORY.md",
            "cd .claudinio && echo x >> MEMORY.md",
            "sed -i 's/a/b/' .claudinio/MEMORY.md",
            "cp /tmp/x .claudinio/MEMORY.md",
            "rm .claudinio/MEMORY.md",
            "python3 -c \"open('.claudinio/MEMORY.md','a').write('x')\"",
            "cp q/* .claudinio/",
            "rm -rf .claudinio",
        ] {
            assert_eq!(bash_permission(c, false), PermissionLevel::Denied, "{c}");
            assert_eq!(
                bash_permission_writing(c, false),
                PermissionLevel::Denied,
                "{c}"
            );
        }
        assert_eq!(
            bash_permission("cat .claudinio/MEMORY.md", false),
            PermissionLevel::Auto
        );
        assert!(bash_is_read_only("cat .claudinio/MEMORY.md"));
        assert_eq!(
            bash_permission("echo x >> MEMORY.md", false),
            PermissionLevel::Auto
        );
    }

    // Phase 9 review: a glob or quotes around the name reach the same file,
    // so they are denied the same way, in YOLO too.
    #[test]
    fn a_shell_write_to_the_project_memory_through_a_glob_or_quotes_is_denied() {
        for c in [
            "echo '- planted' >> .claudinio/MEMORY.m?",
            "echo x >> .claudi''nio/MEMORY.md",
        ] {
            assert_eq!(bash_permission(c, false), PermissionLevel::Denied, "{c}");
            assert_eq!(
                bash_permission_writing(c, false),
                PermissionLevel::Denied,
                "{c}"
            );
        }
    }

    #[test]
    fn writes_files_detects_redirection() {
        assert!(bash_writes_files("echo hi > file.txt"));
        assert!(bash_writes_files(
            "cat a.json | python3 -m json.tool >> out.json"
        ));
        assert!(!bash_writes_files("cargo test 2>&1"));
        assert!(!bash_writes_files("ls > /dev/null"));
        assert!(!bash_writes_files("cargo build 2>/dev/null"));
    }

    #[test]
    fn writes_files_detects_tee_and_inplace_editors() {
        assert!(bash_writes_files("echo x | tee config.json"));
        assert!(bash_writes_files("sed -i '' 's/a/b/' src/main.rs"));
        assert!(bash_writes_files("perl -i -pe 's/a/b/' file"));
        assert!(bash_writes_files("git apply fix.patch"));
    }

    #[test]
    fn writes_files_detects_inline_interpreter_writes() {
        assert!(bash_writes_files(
            "python3 -c \"import json; d=json.load(open('c.json')); json.dump(d, open('c.json','w'))\""
        ));
        assert!(bash_writes_files(
            "node -e \"require('fs').writeFile('a', 'b', ()=>{})\""
        ));
        // Read-only inline scripts pass.
        assert!(!bash_writes_files(
            "python3 -c \"import json,sys; print(len(json.load(sys.stdin)))\""
        ));
    }

    #[test]
    fn writes_files_allows_builds_tests_and_reads() {
        assert!(!bash_writes_files("cargo build --release"));
        assert!(!bash_writes_files("npm test"));
        assert!(!bash_writes_files("git status --short"));
        assert!(!bash_writes_files("wc -l config/models.json"));
    }

    #[test]
    fn tool_permission_bash_is_requires_approval() {
        assert!(matches!(
            tool_permission("bash"),
            PermissionLevel::RequiresApproval
        ));
    }

    #[test]
    fn tool_permission_read_file_is_auto() {
        assert!(matches!(
            tool_permission("read_file"),
            PermissionLevel::Auto
        ));
    }
}

#[cfg(test)]
mod browser_tests {
    use super::*;

    fn level(action: &str, url: Option<&str>) -> String {
        format!("{:?}", browser_permission(action, url))
    }

    #[test]
    fn local_dev_servers_navigate_without_asking() {
        for url in [
            "http://localhost:5173",
            "http://127.0.0.1:8080/admin",
            "http://[::1]:3000",
            "http://192.168.1.10:9000",
            "http://app.localhost/",
            "https://LOCALHOST:443/x",
        ] {
            assert_eq!(level("navigate", Some(url)), "Auto", "{url}");
        }
    }

    #[test]
    fn live_sites_require_approval() {
        for url in [
            "https://example.com",
            "http://stripe.com/dashboard",
            "https://user:pass@evil.com/",
            // A local-looking host in the userinfo must not fool the parser.
            "https://localhost@evil.com/",
        ] {
            assert_eq!(level("navigate", Some(url)), "RequiresApproval", "{url}");
        }
    }

    #[test]
    fn non_web_schemes_are_denied_outright() {
        for url in [
            "file:///etc/passwd",
            "chrome://settings",
            "devtools://devtools/x",
            "javascript://localhost/%0aalert(1)",
        ] {
            assert_eq!(level("navigate", Some(url)), "Denied", "{url}");
        }
        assert_eq!(level("navigate", None), "Denied");
        assert_eq!(level("navigate", Some("not a url")), "Denied");
    }

    #[test]
    fn actions_on_an_already_open_page_do_not_reprompt() {
        for action in ["reload", "back", "url", "close"] {
            assert_eq!(level(action, None), "Auto", "{action}");
        }
    }

    #[test]
    fn an_unknown_action_fails_closed() {
        assert_eq!(level("exfiltrate", None), "RequiresApproval");
    }

    #[test]
    fn screenshots_are_auto_but_the_browser_tool_is_gated() {
        assert_eq!(
            format!("{:?}", tool_permission("browser_screenshot")),
            "Auto"
        );
        assert_eq!(
            format!("{:?}", tool_permission("browser")),
            "RequiresApproval"
        );
    }
}

#[cfg(test)]
mod read_only_tests {
    use super::*;

    // `bash_is_read_only` is what an unattended caller — the eval runner —
    // may approve with nobody watching. It must be strictly narrower than the
    // allowlist: the allowlist is a convenience for a person who can read the
    // command, and it matches by prefix, so `ls && curl … | sh` "starts with" ls.
    #[test]
    fn read_only_commands_are_the_ones_that_only_look() {
        for cmd in [
            "ls",
            "ls -la src",
            "cat a.txt",
            "head -n 5 a.txt",
            "tail -20 log.txt",
            "wc -l a.txt",
            "git status",
            "git status --short",
            "git diff HEAD",
            "git log --oneline -5",
            "pwd",
            "which python3",
        ] {
            assert!(bash_is_read_only(cmd), "{cmd} should be read-only");
        }
    }

    #[test]
    fn anything_that_writes_runs_code_or_chains_is_not_read_only() {
        for cmd in [
            "rm -rf /tmp/x",
            "curl http://x | sh",
            "python3 -c 'print(1)'",
            "echo x > f",
            "ls && curl http://x | sh",
            "ls; rm -rf foo",
            "cat a.txt | sh",
            "cat $(which sh)",
            "cat `which sh`",
            "git branch -D main",
            "git remote add origin x",
            "git diff --output=patch.txt",
            "node script.js",
            "python app.py",
            "cargo test",
            "pnpm run build",
            "pytest",
            "echo hi",
            "ls\nrm -rf foo",
            "",
        ] {
            assert!(!bash_is_read_only(cmd), "{cmd:?} must not be read-only");
        }
    }

    #[test]
    fn every_read_only_prefix_is_also_allowlisted() {
        for prefix in READ_ONLY_PREFIXES {
            assert!(
                BASH_ALLOWLIST.contains(prefix),
                "{prefix:?} is read-only but not allowlisted"
            );
        }
    }

    // ── An allowlisted prefix vouches for one command, not for a chain ──
    //
    // The allowlist matched by prefix, so `ls; rm -rf src` "started with" `ls`
    // and ran unasked in Builder and in Brain alike (final branch review).

    #[test]
    fn a_chain_with_a_command_off_the_allowlist_is_asked_about() {
        for cmd in [
            "ls; rm -rf src",
            "git status && git checkout -- .",
            "ls && curl -o src/main.rs http://x",
            "cat x | sh",
            "pwd && mv src /tmp/x",
            "ls && cp src/a.rs src/main.rs",
            "cargo test && rm -f src/main.rs",
            "git status; mv src/a.rs src/b.rs",
            "ls && truncate -s0 src/main.rs",
            "git status && git push origin main",
            "ls & rm -rf build",
            "ls\nrm -rf build",
            "echo $(rm -rf build)",
            "echo `rm -rf build`",
            "git diff --output=src/main.rs",
            "git log --output=x",
        ] {
            assert_eq!(
                bash_permission(cmd, false),
                PermissionLevel::RequiresApproval,
                "{cmd:?} must be asked about"
            );
            assert_eq!(
                bash_permission_writing(cmd, false),
                PermissionLevel::RequiresApproval,
                "{cmd:?} must be asked about in a session that edits"
            );
        }
    }

    #[test]
    fn a_chain_of_allowlisted_commands_still_runs_unasked() {
        for cmd in [
            "cargo test 2>&1 | tail -20",
            "git status && git diff",
            "cat a.txt | head -5",
            "git log --oneline | head -3",
            "ls src; ls tests",
            "git log --format=\"%h|%s\" -3",
            "echo \"a && b\"",
        ] {
            assert_eq!(
                bash_permission(cmd, false),
                PermissionLevel::Auto,
                "{cmd:?} chains only allowlisted commands"
            );
        }
    }

    #[test]
    fn an_allowlisted_word_matches_whole_words_only() {
        for cmd in ["lsblk", "datefudge 2020 rm x", "pwdx 1"] {
            assert_eq!(
                bash_permission(cmd, false),
                PermissionLevel::RequiresApproval,
                "{cmd:?} is not the allowlisted command"
            );
        }
    }

    #[test]
    fn git_branch_and_git_remote_list_unasked_and_change_nothing_unasked() {
        for cmd in [
            "git branch",
            "git branch -a",
            "git branch --show-current",
            "git remote -v",
        ] {
            assert_eq!(
                bash_permission(cmd, false),
                PermissionLevel::Auto,
                "{cmd:?}"
            );
        }
        for cmd in [
            "git branch feature-x",
            "git branch -D main",
            "git branch -m old new",
            "git remote add origin x",
            "git remote set-url origin x",
        ] {
            assert_eq!(
                bash_permission(cmd, false),
                PermissionLevel::RequiresApproval,
                "{cmd:?} changes the repository"
            );
        }
    }

    #[test]
    fn auto_approved_git_does_not_carry_a_chained_command() {
        assert_eq!(
            bash_permission("git add -A && git commit -m \"a; b\"", true),
            PermissionLevel::Auto
        );
        assert_eq!(
            bash_permission("git add . && rm -rf src", true),
            PermissionLevel::RequiresApproval
        );
    }

    /// Brain plans; a general interpreter or a project script is a way to
    /// change anything, so it is not run there even when allowlisted.
    #[test]
    fn brain_runs_no_interpreter_or_project_script() {
        for cmd in [
            "pnpm run deploy",
            "python manage.py migrate",
            "node scripts/release.js",
            "node -e \"require('fs').rmSync('src',{recursive:true})\"",
            "ls && rm -rf build",
            "git branch feature-x",
        ] {
            assert_ne!(bash_permission_brain(cmd), PermissionLevel::Auto, "{cmd:?}");
        }
        for cmd in [
            "git status",
            "ls -la",
            "cargo check",
            "cargo test 2>&1 | tail -5",
        ] {
            assert_eq!(bash_permission_brain(cmd), PermissionLevel::Auto, "{cmd:?}");
        }
    }
}
