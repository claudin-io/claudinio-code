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
        _ => PermissionLevel::Auto,
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
            || lower.starts_with("git pull ")
            || lower == "git pull"
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
        assert!(matches!(bash_permission("ls", false), PermissionLevel::Auto));
        assert!(matches!(bash_permission("ls -la", false), PermissionLevel::Auto));
    }

    #[test]
    fn bash_allowlist_git_status() {
        assert!(matches!(bash_permission("git status", false), PermissionLevel::Auto));
        assert!(matches!(bash_permission("git status --short", false), PermissionLevel::Auto));
    }

    #[test]
    fn bash_allowlist_git_diff() {
        assert!(matches!(bash_permission("git diff", false), PermissionLevel::Auto));
        assert!(matches!(bash_permission("git diff --cached", false), PermissionLevel::Auto));
    }

    #[test]
    fn bash_allowlist_pnpm_dev() {
        assert!(matches!(bash_permission("pnpm dev", false), PermissionLevel::Auto));
        assert!(matches!(bash_permission("pnpm run dev", false), PermissionLevel::Auto));
    }

    #[test]
    fn bash_allowlist_pnpm_run() {
        assert!(matches!(bash_permission("pnpm run build", false), PermissionLevel::Auto));
    }

    #[test]
    fn bash_allowlist_cargo_build() {
        assert!(matches!(bash_permission("cargo build", false), PermissionLevel::Auto));
        assert!(matches!(bash_permission("cargo build --release", false), PermissionLevel::Auto));
    }

    #[test]
    fn bash_blacklist_rm_rf_root() {
        assert!(matches!(bash_permission("rm -rf /", false), PermissionLevel::Denied));
        assert!(matches!(bash_permission("rm -rf /*", false), PermissionLevel::Denied));
    }

    #[test]
    fn bash_blacklist_sudo_rm() {
        assert!(matches!(bash_permission("sudo rm -rf /tmp", false), PermissionLevel::Denied));
    }

    #[test]
    fn bash_blacklist_chmod_777() {
        assert!(matches!(bash_permission("chmod 777 /etc", false), PermissionLevel::Denied));
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
        assert!(matches!(bash_permission("  ls  ", false), PermissionLevel::Auto));
        assert!(matches!(bash_permission("  rm -rf /  ", false), PermissionLevel::Denied));
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
    fn auto_approve_git_pull_with_flag() {
        assert!(matches!(
            bash_permission("git pull origin main", true),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("git pull --rebase", true),
            PermissionLevel::Auto
        ));
        assert!(matches!(
            bash_permission("git pull", true),
            PermissionLevel::Auto
        ));
        // Without the flag, git pull requires approval.
        assert!(matches!(
            bash_permission("git pull", false),
            PermissionLevel::RequiresApproval
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

    #[test]
    fn tool_permission_bash_is_requires_approval() {
        assert!(matches!(tool_permission("bash"), PermissionLevel::RequiresApproval));
    }

    #[test]
    fn tool_permission_read_file_is_auto() {
        assert!(matches!(tool_permission("read_file"), PermissionLevel::Auto));
    }
}
