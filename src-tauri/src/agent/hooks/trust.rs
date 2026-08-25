//! Consent, stored.
//!
//! A hook is arbitrary code that a repository can ship. Cloning a project and
//! opening it must not be the same act as running whatever that project's
//! `.claude/settings.json` says to run, and `settings.local.json` is gitignored
//! in most repos precisely so that it can hold things nobody reviewed.
//!
//! So: hooks are discovered, listed and displayed by default, and spawned only
//! after the user has approved *that exact set* once. The approval is a hash
//! over the resolved commands, which means editing a command revokes it and
//! renaming a spinner label does not.
//!
//! The file lives under the user's home, never in the repository. A repo-local
//! approval could arrive already approved in a pull request, which would defeat
//! the entire gate.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustEntry {
    pub hash: String,
    pub approved_at: u64,
    /// What was approved, in the words the user was shown. Kept so the panel can
    /// say *what changed* rather than only *that something did*.
    #[serde(default)]
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustFile {
    pub version: u32,
    #[serde(default)]
    pub workspaces: BTreeMap<String, TrustEntry>,
}

impl Default for TrustFile {
    fn default() -> Self {
        Self {
            version: 1,
            workspaces: BTreeMap::new(),
        }
    }
}

/// Where a workspace stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrustStatus {
    /// Nothing to approve.
    NoHooks,
    /// Hooks exist and have never been approved here.
    Pending,
    /// Approved, and the set has not changed since.
    Trusted,
    /// Approved once, but a command changed. Treated exactly like `Pending` —
    /// named separately so the UI can say which it is.
    Changed,
}

impl TrustStatus {
    pub fn may_run(&self) -> bool {
        matches!(self, TrustStatus::Trusted)
    }
}

#[derive(Debug, Clone)]
pub struct TrustStore {
    path: PathBuf,
}

impl Default for TrustStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TrustStore {
    pub fn new() -> Self {
        Self {
            path: Self::default_path(),
        }
    }

    /// A store at an explicit path. Used by tests, which must not read or
    /// write the developer's real approvals.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(".claudinio")
            .join("hook-trust.json")
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read(&self) -> TrustFile {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn write(&self, file: &TrustFile) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, text).map_err(|e| e.to_string())
    }

    fn key(workspace: &str) -> String {
        workspace.trim_end_matches(['/', '\\']).to_string()
    }

    pub fn entry(&self, workspace: &str) -> Option<TrustEntry> {
        self.read().workspaces.get(&Self::key(workspace)).cloned()
    }

    /// The status of a workspace against the set that was just resolved.
    ///
    /// An empty fingerprint means no hooks were found, which is not a thing to
    /// approve — asking would train people to click through the prompt.
    pub fn status(&self, workspace: Option<&str>, fingerprint: &str) -> TrustStatus {
        if fingerprint.is_empty() {
            return TrustStatus::NoHooks;
        }
        let Some(ws) = workspace else {
            return TrustStatus::NoHooks;
        };
        match self.entry(ws) {
            Some(e) if e.hash == fingerprint => TrustStatus::Trusted,
            Some(_) => TrustStatus::Changed,
            None => TrustStatus::Pending,
        }
    }

    /// Record consent. The caller passes the fingerprint it displayed, so a set
    /// that changed between rendering and clicking cannot be approved blind.
    pub fn approve(
        &self,
        workspace: &str,
        fingerprint: &str,
        commands: Vec<String>,
        now_ms: u64,
    ) -> Result<(), String> {
        if fingerprint.is_empty() {
            return Err("there are no hooks to approve".into());
        }
        let mut file = self.read();
        file.workspaces.insert(
            Self::key(workspace),
            TrustEntry {
                hash: fingerprint.to_string(),
                approved_at: now_ms,
                commands,
            },
        );
        self.write(&file)
    }

    pub fn revoke(&self, workspace: &str) -> Result<(), String> {
        let mut file = self.read();
        file.workspaces.remove(&Self::key(workspace));
        self.write(&file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> TrustStore {
        let p = std::env::temp_dir()
            .join(format!("cc-hook-trust-{name}-{}", std::process::id()))
            .join("hook-trust.json");
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
        TrustStore::with_path(p)
    }

    #[test]
    fn an_unapproved_set_does_not_run() {
        let s = store("unapproved");
        assert_eq!(s.status(Some("/ws"), "sha256:a"), TrustStatus::Pending);
        assert!(!s.status(Some("/ws"), "sha256:a").may_run());
    }

    #[test]
    fn approving_stores_the_hash_and_lets_it_run() {
        let s = store("approve");
        s.approve("/ws", "sha256:a", vec!["run.sh".into()], 1)
            .unwrap();
        assert_eq!(s.status(Some("/ws"), "sha256:a"), TrustStatus::Trusted);
        assert!(s.status(Some("/ws"), "sha256:a").may_run());
        assert_eq!(s.entry("/ws").unwrap().commands, vec!["run.sh"]);
    }

    #[test]
    fn changing_a_command_revokes_the_approval() {
        let s = store("changed");
        s.approve("/ws", "sha256:a", vec![], 1).unwrap();
        assert_eq!(s.status(Some("/ws"), "sha256:b"), TrustStatus::Changed);
        assert!(!s.status(Some("/ws"), "sha256:b").may_run());
    }

    #[test]
    fn no_hooks_is_not_something_to_approve() {
        let s = store("empty");
        assert_eq!(s.status(Some("/ws"), ""), TrustStatus::NoHooks);
        assert_eq!(s.status(None, "sha256:a"), TrustStatus::NoHooks);
        assert!(s.approve("/ws", "", vec![], 1).is_err());
    }

    #[test]
    fn revoking_and_re_approving_round_trip() {
        let s = store("revoke");
        s.approve("/ws", "sha256:a", vec![], 1).unwrap();
        s.revoke("/ws").unwrap();
        assert_eq!(s.status(Some("/ws"), "sha256:a"), TrustStatus::Pending);
        s.approve("/ws", "sha256:a", vec![], 2).unwrap();
        assert_eq!(s.status(Some("/ws"), "sha256:a"), TrustStatus::Trusted);
    }

    #[test]
    fn the_trust_file_preserves_other_workspaces() {
        let s = store("multi");
        s.approve("/a", "sha256:a", vec![], 1).unwrap();
        s.approve("/b", "sha256:b", vec![], 2).unwrap();
        s.revoke("/a").unwrap();
        assert_eq!(s.status(Some("/b"), "sha256:b"), TrustStatus::Trusted);
        assert_eq!(s.read().workspaces.len(), 1);
    }

    #[test]
    fn a_trailing_separator_is_the_same_workspace() {
        let s = store("slash");
        s.approve("/ws/", "sha256:a", vec![], 1).unwrap();
        assert_eq!(s.status(Some("/ws"), "sha256:a"), TrustStatus::Trusted);
    }

    #[test]
    fn a_corrupt_trust_file_reads_as_no_approvals_rather_than_failing() {
        let s = store("corrupt");
        std::fs::create_dir_all(s.path().parent().unwrap()).unwrap();
        std::fs::write(s.path(), "{ not json").unwrap();
        assert_eq!(s.status(Some("/ws"), "sha256:a"), TrustStatus::Pending);
        // And it repairs itself on the next write.
        s.approve("/ws", "sha256:a", vec![], 1).unwrap();
        assert_eq!(s.status(Some("/ws"), "sha256:a"), TrustStatus::Trusted);
    }
}
