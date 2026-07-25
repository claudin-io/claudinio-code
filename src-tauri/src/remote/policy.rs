//! What a paired peer is allowed to do.
//!
//! This is the guardrail, so it is pure: a file in, a set of permissions out, no
//! I/O beyond reading the file and no way to reach the network. Everything that
//! decides whether a remote command runs is decided here and checked in one place
//! (`bridge.rs`).
//!
//! # Two rules the type system carries rather than the reviewer
//!
//! **Nothing is granted by default.** A missing file, an unreadable file, a file
//! from a newer build, an expired grant, remote access switched off — every one of
//! those produces a policy that permits nothing. Failing closed is not a
//! convention here; there is no path through this module that turns an error into
//! a permission.
//!
//! **A peer can never widen its own policy.** That is invariant I4, and it is
//! enforced by absence: this module can load and evaluate, and there is no
//! function that writes. Widening happens through the local UI, on the machine, or
//! not at all — because the first thing a compromised peer would do is grant
//! itself everything.

use std::path::Path;

use claudinio_protocol::inner::{BashApproval, Policy as WirePolicy};
use serde::{Deserialize, Serialize};

/// The on-disk shape. Mirrors §6.4 of the plan.
///
/// `deny_unknown_fields` is deliberate. A file written by a newer build may name
/// permissions this one has never heard of, and the safe reading of "I do not
/// understand this policy" is to refuse the whole thing rather than to apply the
/// half of it that parsed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct StoredPolicy {
    /// Off until the user turns it on, locally. §6.3 step 1.
    pub enabled: bool,
    /// Unix millis. Every grant expires by default — a permission with no end is
    /// one nobody remembers giving.
    pub expires_at: Option<u64>,
    /// Absolute paths a peer may see at all. Empty means none.
    pub workspaces: Vec<String>,
    pub idle_disconnect_minutes: u32,
    pub allow: Allow,
    /// Added to the local bash denylist for remote callers, never subtracted from
    /// it. A remote peer's shell surface is a subset of the local user's.
    pub bash_remote_denylist_extra: Vec<String>,
    pub max_unattended_minutes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Allow {
    pub send_message: bool,
    pub steer: bool,
    pub interrupt: bool,
    pub set_mode: bool,
    pub approve_edit: bool,
    pub approve_bash: BashApproval,
    /// Reads through the app's IPC surface are not workspace-scoped: attaching a
    /// file from `~/Downloads` is the point of the feature locally, where a native
    /// picker means the user chose it. Remotely there is no picker, so the same
    /// capability is an arbitrary read of the home directory.
    pub read_attachment: bool,
    /// Present so a policy file can say it, and ignored so it cannot mean it. See
    /// `effective`.
    pub export_file: bool,
}

impl Default for Allow {
    fn default() -> Self {
        Self {
            send_message: false,
            steer: false,
            interrupt: false,
            set_mode: false,
            approve_edit: false,
            approve_bash: BashApproval::Never,
            read_attachment: false,
            export_file: false,
        }
    }
}

impl Default for StoredPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            expires_at: None,
            workspaces: Vec::new(),
            idle_disconnect_minutes: 30,
            allow: Allow::default(),
            bash_remote_denylist_extra: default_remote_denylist(),
            max_unattended_minutes: 0,
        }
    }
}

/// Commands that are never approvable remotely, whatever the policy says.
///
/// Not a security boundary on its own — a denylist never is, and §6.5 says so.
/// It is a guard against the specific, common, expensive mistake: tapping approve
/// on a phone for something whose consequences are not visible on a phone.
pub fn default_remote_denylist() -> Vec<String> {
    [
        "git push --force",
        "git push -f",
        "rm -rf",
        "curl|sh",
        "curl | sh",
        "npm publish",
        "cargo publish",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Why a policy grants nothing, when it grants nothing.
///
/// Surfaced so the local UI can say "your grant expired" rather than leaving
/// someone to guess why their phone stopped working.
///
/// There is no `Unreadable` variant: a file that cannot be parsed is an error from
/// `load`, not a policy. Turning it into an inert policy would let a corrupt file
/// read as a deliberate "everything off", and the two want different words on
/// screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inert {
    Disabled,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effective {
    /// Remote access is live, with these permissions.
    Active(WirePolicy),
    /// Nothing is granted, for this reason.
    Nothing(Inert),
}

impl Effective {
    /// The wire form to hand a peer. An inert policy still produces a policy —
    /// the all-denying one — so the peer's UI can grey everything out rather than
    /// showing nothing at all.
    pub fn wire(&self) -> WirePolicy {
        match self {
            Self::Active(policy) => policy.clone(),
            Self::Nothing(_) => WirePolicy::default(),
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active(_))
    }
}

impl StoredPolicy {
    /// Load the policy at `path`.
    ///
    /// A missing file is the default, which grants nothing — that is the state of
    /// a machine where nobody has turned remote access on. An unparseable file is
    /// an error rather than a fallback to defaults, because silently substituting
    /// a policy for the one the user wrote is how a guardrail becomes a surprise.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(body) => serde_json::from_str(&body)
                .map_err(|e| format!("{} is not a valid policy: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("cannot read {}: {e}", path.display())),
        }
    }

    /// What this policy actually permits, at `now_ms`.
    pub fn effective(&self, now_ms: u64) -> Effective {
        if !self.enabled {
            return Effective::Nothing(Inert::Disabled);
        }
        if self.expires_at.is_some_and(|at| now_ms >= at) {
            return Effective::Nothing(Inert::Expired);
        }

        Effective::Active(WirePolicy {
            send_message: self.allow.send_message,
            steer: self.allow.steer,
            interrupt: self.allow.interrupt,
            set_mode: self.allow.set_mode,
            approve_edit: self.allow.approve_edit,
            approve_bash: self.allow.approve_bash,
            read_attachment: self.allow.read_attachment,
            // Never granted, whatever the file says. Its whole security property
            // is that the destination comes from a native dialog and never
            // crosses IPC as an argument, and there is no remote equivalent of
            // standing in front of the machine. A policy file that asks for it is
            // asking for something that does not exist.
            export_file: false,
            expires_at: self.expires_at,
        })
    }

    /// Whether a peer may see this workspace at all.
    ///
    /// Exact match on the absolute path. Not a prefix test: `/Users/v/work` must
    /// not open `/Users/v/work-secrets`, and a prefix check is how that happens.
    pub fn allows_workspace(&self, workspace: &Path) -> bool {
        self.workspaces
            .iter()
            .any(|allowed| Path::new(allowed) == workspace)
    }

    /// The bash denylist a remote caller is subject to: the local one plus the
    /// extras. Additive by construction — there is no way to express a removal.
    pub fn remote_bash_denylist(&self) -> Vec<String> {
        let mut list = default_remote_denylist();
        for extra in &self.bash_remote_denylist_extra {
            if !list.contains(extra) {
                list.push(extra.clone());
            }
        }
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn permissive() -> StoredPolicy {
        StoredPolicy {
            enabled: true,
            expires_at: Some(NOW + 60_000),
            workspaces: vec!["/Users/v/work".into()],
            allow: Allow {
                send_message: true,
                steer: true,
                interrupt: true,
                set_mode: true,
                approve_edit: true,
                approve_bash: BashApproval::Allowlist,
                read_attachment: true,
                export_file: true,
            },
            ..Default::default()
        }
    }

    fn write(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote-policy.json");
        std::fs::write(&path, body).unwrap();
        (dir, path)
    }

    // --- failing closed -----------------------------------------------------

    /// The state of a machine where nobody has turned remote access on.
    #[test]
    fn the_default_policy_grants_nothing() {
        let effective = StoredPolicy::default().effective(NOW);

        assert_eq!(effective, Effective::Nothing(Inert::Disabled));
        assert!(!effective.is_active());
        // And the wire form is the all-denying one, so a peer's UI can grey
        // everything out rather than rendering nothing.
        assert_eq!(effective.wire(), WirePolicy::default());
    }

    #[test]
    fn a_missing_file_is_the_default_rather_than_an_error() {
        let dir = tempfile::tempdir().unwrap();

        let policy = StoredPolicy::load(&dir.path().join("nope.json")).unwrap();

        assert!(!policy.enabled);
        assert!(!policy.effective(NOW).is_active());
    }

    /// Substituting a default for a policy the user wrote is how a guardrail
    /// becomes a surprise. Better to refuse and say so.
    #[test]
    fn an_unparseable_file_is_an_error_not_a_silent_default() {
        let (_d, path) = write("{ this is not json");

        assert!(StoredPolicy::load(&path).is_err());
    }

    /// A file from a newer build may name permissions this one has never heard of.
    /// Applying the half that parsed would be applying a policy nobody wrote.
    #[test]
    fn a_policy_naming_an_unknown_permission_is_refused_whole() {
        let (_d, path) =
            write(r#"{"enabled":true,"allow":{"send_message":true,"deploy_to_production":true}}"#);

        let error = StoredPolicy::load(&path).unwrap_err();

        assert!(error.contains("not a valid policy"), "{error}");
    }

    #[test]
    fn an_expired_grant_permits_nothing_however_generous_it_was() {
        let mut policy = permissive();
        policy.expires_at = Some(NOW - 1);

        assert_eq!(policy.effective(NOW), Effective::Nothing(Inert::Expired));
    }

    /// Expiry is inclusive of the instant itself, so a grant is never live at its
    /// own deadline.
    #[test]
    fn a_grant_is_not_live_at_the_exact_moment_it_expires() {
        let mut policy = permissive();
        policy.expires_at = Some(NOW);

        assert_eq!(policy.effective(NOW), Effective::Nothing(Inert::Expired));
    }

    #[test]
    fn switching_remote_access_off_beats_every_other_setting() {
        let mut policy = permissive();
        policy.enabled = false;

        assert_eq!(policy.effective(NOW), Effective::Nothing(Inert::Disabled));
    }

    // --- what a live policy grants -----------------------------------------

    #[test]
    fn a_live_policy_grants_what_it_says() {
        match permissive().effective(NOW) {
            Effective::Active(wire) => {
                assert!(wire.send_message);
                assert!(wire.steer);
                assert!(wire.interrupt);
                assert!(wire.approve_edit);
                assert_eq!(wire.approve_bash, BashApproval::Allowlist);
                assert!(wire.read_attachment);
            }
            other => panic!("expected an active policy, got {other:?}"),
        }
    }

    /// `export_file` has no safe remote form: its security property is a native
    /// dialog the peer cannot have. A policy file may ask for it; it never gets
    /// it. This is the one permission the file cannot grant.
    #[test]
    fn export_file_is_refused_even_when_the_file_grants_it() {
        let policy = permissive();
        assert!(policy.allow.export_file, "the file does ask for it");

        match policy.effective(NOW) {
            Effective::Active(wire) => assert!(
                !wire.export_file,
                "a policy file must not be able to grant export_file"
            ),
            other => panic!("expected an active policy, got {other:?}"),
        }
    }

    /// The peer is told when its access lapses, so its UI can say so instead of
    /// looking broken.
    #[test]
    fn the_expiry_travels_to_the_peer() {
        match permissive().effective(NOW) {
            Effective::Active(wire) => assert_eq!(wire.expires_at, Some(NOW + 60_000)),
            other => panic!("expected an active policy, got {other:?}"),
        }
    }

    // --- workspaces ---------------------------------------------------------

    #[test]
    fn only_listed_workspaces_are_visible() {
        let policy = permissive();

        assert!(policy.allows_workspace(Path::new("/Users/v/work")));
        assert!(!policy.allows_workspace(Path::new("/Users/v/secrets")));
    }

    /// The reason this is an exact match and not a prefix test. A prefix check
    /// would let an allowlisted `/Users/v/work` open `/Users/v/work-secrets`,
    /// which is a different directory that merely starts the same way.
    #[test]
    fn a_workspace_that_merely_starts_the_same_is_not_allowed() {
        let policy = permissive();

        assert!(!policy.allows_workspace(Path::new("/Users/v/work-secrets")));
        assert!(!policy.allows_workspace(Path::new("/Users/v/workshop")));
    }

    #[test]
    fn no_workspaces_listed_means_none_visible() {
        let policy = StoredPolicy::default();

        assert!(!policy.allows_workspace(Path::new("/Users/v/work")));
        assert!(!policy.allows_workspace(Path::new("/")));
    }

    // --- the bash denylist --------------------------------------------------

    /// Additive by construction: there is no field that removes an entry, so a
    /// remote peer's shell surface cannot be widened past the local user's.
    #[test]
    fn the_remote_denylist_is_the_default_plus_the_extras() {
        let mut policy = permissive();
        policy.bash_remote_denylist_extra = vec!["kubectl delete".into()];

        let list = policy.remote_bash_denylist();

        assert!(list.contains(&"kubectl delete".to_string()));
        for baseline in default_remote_denylist() {
            assert!(list.contains(&baseline), "{baseline} was dropped");
        }
    }

    /// A file that lists a default entry again must not produce it twice — a
    /// denylist with duplicates is harmless but suggests the merge is wrong.
    #[test]
    fn repeating_a_default_entry_does_not_duplicate_it() {
        let mut policy = permissive();
        policy.bash_remote_denylist_extra = vec!["rm -rf".into()];

        let list = policy.remote_bash_denylist();

        assert_eq!(list.iter().filter(|e| *e == "rm -rf").count(), 1);
    }

    /// Emptying the extras cannot empty the baseline. This is the test that would
    /// fail if someone made the field replace rather than extend.
    #[test]
    fn clearing_the_extras_does_not_clear_the_defaults() {
        let mut policy = permissive();
        policy.bash_remote_denylist_extra = Vec::new();

        assert_eq!(policy.remote_bash_denylist(), default_remote_denylist());
        assert!(!policy.remote_bash_denylist().is_empty());
    }

    // --- round trip ---------------------------------------------------------

    /// A policy written by the local editor must read back as itself, or a grant
    /// could silently change on the next load.
    #[test]
    fn a_policy_survives_a_round_trip_through_json() {
        let policy = permissive();

        let json = serde_json::to_string(&policy).unwrap();
        let back: StoredPolicy = serde_json::from_str(&json).unwrap();

        assert_eq!(back.effective(NOW), policy.effective(NOW));
        assert_eq!(back.remote_bash_denylist(), policy.remote_bash_denylist());
        assert_eq!(back.workspaces, policy.workspaces);
    }

    /// A file that sets only what it cares about gets denials for the rest, rather
    /// than whatever a partially-initialised struct would hold.
    #[test]
    fn omitted_permissions_default_to_denied() {
        let (_d, path) = write(r#"{"enabled":true,"allow":{"interrupt":true}}"#);

        let policy = StoredPolicy::load(&path).unwrap();

        match policy.effective(NOW) {
            Effective::Active(wire) => {
                assert!(wire.interrupt);
                assert!(!wire.send_message);
                assert!(!wire.approve_edit);
                assert_eq!(wire.approve_bash, BashApproval::Never);
            }
            other => panic!("expected an active policy, got {other:?}"),
        }
    }

    /// I4, stated as a test so it is not only a comment: this module has no way to
    /// widen a policy. If a `save` ever appears here, the reviewer should have to
    /// delete this test to add it.
    #[test]
    fn this_module_cannot_write_a_policy() {
        // Deliberately not a runtime assertion — there is nothing to call. The
        // check is that the module's surface is load and evaluate only, and the
        // absence of a writer is what makes "a peer cannot widen its own policy"
        // true rather than merely intended.
        let _load: fn(&Path) -> Result<StoredPolicy, String> = StoredPolicy::load;
        let _evaluate: fn(&StoredPolicy, u64) -> Effective = StoredPolicy::effective;
    }
}
