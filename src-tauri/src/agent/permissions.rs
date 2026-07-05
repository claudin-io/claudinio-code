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

pub fn bash_permission(command: &str) -> PermissionLevel {
    let trimmed = command.trim();

    for pattern in BASH_BLACKLIST {
        if trimmed.contains(pattern) {
            return PermissionLevel::Denied;
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
        assert!(matches!(bash_permission("ls"), PermissionLevel::Auto));
        assert!(matches!(bash_permission("ls -la"), PermissionLevel::Auto));
    }

    #[test]
    fn bash_allowlist_git_status() {
        assert!(matches!(bash_permission("git status"), PermissionLevel::Auto));
        assert!(matches!(bash_permission("git status --short"), PermissionLevel::Auto));
    }

    #[test]
    fn bash_allowlist_git_diff() {
        assert!(matches!(bash_permission("git diff"), PermissionLevel::Auto));
        assert!(matches!(bash_permission("git diff --cached"), PermissionLevel::Auto));
    }

    #[test]
    fn bash_allowlist_pnpm_dev() {
        assert!(matches!(bash_permission("pnpm dev"), PermissionLevel::Auto));
        assert!(matches!(bash_permission("pnpm run dev"), PermissionLevel::Auto));
    }

    #[test]
    fn bash_allowlist_pnpm_run() {
        assert!(matches!(bash_permission("pnpm run build"), PermissionLevel::Auto));
    }

    #[test]
    fn bash_allowlist_cargo_build() {
        assert!(matches!(bash_permission("cargo build"), PermissionLevel::Auto));
        assert!(matches!(bash_permission("cargo build --release"), PermissionLevel::Auto));
    }

    #[test]
    fn bash_blacklist_rm_rf_root() {
        assert!(matches!(bash_permission("rm -rf /"), PermissionLevel::Denied));
        assert!(matches!(bash_permission("rm -rf /*"), PermissionLevel::Denied));
    }

    #[test]
    fn bash_blacklist_sudo_rm() {
        assert!(matches!(bash_permission("sudo rm -rf /tmp"), PermissionLevel::Denied));
    }

    #[test]
    fn bash_blacklist_chmod_777() {
        assert!(matches!(bash_permission("chmod 777 /etc"), PermissionLevel::Denied));
    }

    #[test]
    fn bash_neutral_requires_approval() {
        assert!(matches!(
            bash_permission("git push origin main"),
            PermissionLevel::RequiresApproval
        ));
        assert!(matches!(
            bash_permission("npm install"),
            PermissionLevel::RequiresApproval
        ));
        assert!(matches!(
            bash_permission("pnpm add some-package"),
            PermissionLevel::RequiresApproval
        ));
    }

    #[test]
    fn bash_neutral_git_commit() {
        assert!(matches!(
            bash_permission("git commit -m 'fix'"),
            PermissionLevel::RequiresApproval
        ));
    }

    #[test]
    fn bash_whitespace_trimmed_before_check() {
        assert!(matches!(bash_permission("  ls  "), PermissionLevel::Auto));
        assert!(matches!(bash_permission("  rm -rf /  "), PermissionLevel::Denied));
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
