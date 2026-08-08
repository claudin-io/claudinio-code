use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
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
    // and let the main session rewrite files outside the subagent path.
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

/// Heuristic: does this bash command WRITE file contents? Used to keep the
/// main Builder session from editing files through bash (all modifications
/// must go through code-mode subagents). Deliberately targets content
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

pub fn bash_permission(command: &str, auto_approve_git: bool) -> PermissionLevel {
    let trimmed = command.trim();

    for pattern in BASH_BLACKLIST {
        if trimmed.contains(pattern) {
            return PermissionLevel::Denied;
        }
    }

    // When auto_approve_git is set, auto-approve git add / git commit / git push.
    if auto_approve_git {
        let lower = trimmed.to_lowercase();
        if lower.starts_with("git add ")
            || lower == "git add"
            || lower.starts_with("git commit ")
            || lower == "git commit"
            || lower.starts_with("git push ")
            || lower == "git push"
        {
            return PermissionLevel::Auto;
        }
    }

    for pattern in BASH_ALLOWLIST {
        if trimmed.starts_with(pattern) || trimmed.starts_with(&format!("{pattern} ")) {
            return PermissionLevel::Auto;
        }
    }

    PermissionLevel::RequiresApproval
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
