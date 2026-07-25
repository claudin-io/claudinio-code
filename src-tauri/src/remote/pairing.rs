//! Pairings: the named, listed, individually revocable objects a device serves.
//!
//! # Revocation is the part that has to work offline
//!
//! §6.5 promises that revoking a pairing "drops the Noise session immediately and
//! blacklists the peer key on-device, independent of relay availability". That
//! sentence is the whole design of this module.
//!
//! Revocation that needs a server round trip is revocation that fails exactly when
//! someone needs it — a stolen laptop, a phone left in a taxi, a relay that is
//! down. So the list of who may connect lives on the device, is matched on the
//! peer's static key, and is consulted at handshake time. Nothing is asked of
//! anybody.
//!
//! The consequence worth stating: this list is authoritative even against the
//! dashboard. If the two ever disagree, the device wins, because the device is the
//! thing being protected.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A peer that has completed pairing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pairing {
    /// The peer's X25519 static public key, hex. This is the identity — a label
    /// can be edited, a key cannot, and the key is what arrives at handshake time.
    pub peer_key: String,
    /// What the user called it: "Safari on iPhone". Shown in the transcript, in
    /// `ApprovalResolved`, and in the revocation list, so it has to read as prose.
    pub label: String,
    /// Unix millis.
    pub paired_at: u64,
    /// When this pairing stops being accepted. Every grant expires by default;
    /// `None` is a deliberate choice the local UI has to make explicit.
    pub expires_at: Option<u64>,
}

/// Why a peer was turned away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No pairing for this key. The ordinary case for a stranger.
    Unknown,
    /// Paired once, revoked since. Kept distinct from `Unknown` because the local
    /// UI should be able to say "you revoked this device" rather than "unknown
    /// device", which reads like a bug.
    Revoked,
    Expired,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Book {
    #[serde(default)]
    pairings: Vec<Pairing>,
    /// Keys that were paired and are not any more.
    ///
    /// Kept rather than merely removed, for two reasons. It lets the device tell a
    /// revoked peer apart from a stranger, and it means re-adding a revoked key has
    /// to be a deliberate act rather than a side effect of a peer reconnecting.
    #[serde(default)]
    revoked: Vec<String>,
}

/// The device's pairings, persisted next to its key.
pub struct Pairings {
    path: PathBuf,
    book: Book,
}

impl Pairings {
    /// Load the book at `path`. A missing file is an empty book: a device that has
    /// never paired accepts nobody.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        let book = match std::fs::read_to_string(&path) {
            Ok(body) => serde_json::from_str(&body)
                .map_err(|e| format!("{} is not a valid pairing list: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Book::default(),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        Ok(Self { path, book })
    }

    /// May this key connect, and as whom?
    ///
    /// Revocation is checked before the pairing list, so a key that appears in both
    /// is refused. That ordering is the safe one and the test below pins it.
    pub fn admit(&self, peer_key: &str, now_ms: u64) -> Result<&Pairing, Refusal> {
        if self.book.revoked.iter().any(|k| k == peer_key) {
            return Err(Refusal::Revoked);
        }
        let pairing = self
            .book
            .pairings
            .iter()
            .find(|p| p.peer_key == peer_key)
            .ok_or(Refusal::Unknown)?;

        if pairing.expires_at.is_some_and(|at| now_ms >= at) {
            return Err(Refusal::Expired);
        }
        Ok(pairing)
    }

    /// Record a completed pairing.
    ///
    /// Refuses a revoked key. Re-admitting one has to go through `unrevoke`, so it
    /// cannot happen as a side effect of a peer simply trying again.
    pub fn add(&mut self, pairing: Pairing) -> Result<(), String> {
        if self.book.revoked.contains(&pairing.peer_key) {
            return Err("that key was revoked; un-revoke it first".into());
        }
        self.book
            .pairings
            .retain(|p| p.peer_key != pairing.peer_key);
        self.book.pairings.push(pairing);
        self.save()
    }

    /// Revoke a pairing. Takes effect on the next handshake and, through the
    /// caller, on any session currently open.
    pub fn revoke(&mut self, peer_key: &str) -> Result<(), String> {
        self.book.pairings.retain(|p| p.peer_key != peer_key);
        if !self.book.revoked.iter().any(|k| k == peer_key) {
            self.book.revoked.push(peer_key.to_string());
        }
        self.save()
    }

    /// Deliberately un-revoke, so a key can be paired again.
    pub fn unrevoke(&mut self, peer_key: &str) -> Result<(), String> {
        self.book.revoked.retain(|k| k != peer_key);
        self.save()
    }

    pub fn rename(&mut self, peer_key: &str, label: &str) -> Result<(), String> {
        match self
            .book
            .pairings
            .iter_mut()
            .find(|p| p.peer_key == peer_key)
        {
            Some(pairing) => {
                pairing.label = label.to_string();
                self.save()
            }
            None => Err("no such pairing".into()),
        }
    }

    pub fn list(&self) -> &[Pairing] {
        &self.book.pairings
    }

    pub fn revoked(&self) -> &[String] {
        &self.book.revoked
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create pairing dir: {e}"))?;
        }
        let body = serde_json::to_string_pretty(&self.book)
            .map_err(|e| format!("serialize pairings: {e}"))?;

        // Written through a temporary and renamed, so a crash mid-write cannot
        // leave a truncated list. A pairing file that fails to parse means the
        // device accepts nobody, which is safe but is also everyone locked out of
        // their own machine.
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, &body).map_err(|e| format!("write pairings: {e}"))?;
        std::fs::rename(&temp, &self.path).map_err(|e| format!("replace pairings: {e}"))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

/// How long a pairing window stays open. §6.3 says 120 s, and the reason is human
/// rather than cryptographic: long enough to read a code across a desk, short
/// enough that a screen left unlocked is not an open door.
pub const TOKEN_TTL_MS: u64 = 120_000;

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn key(byte: &str) -> String {
        byte.repeat(32)
    }

    fn pairing(peer_key: &str) -> Pairing {
        Pairing {
            peer_key: peer_key.to_string(),
            label: "Safari on iPhone".into(),
            paired_at: NOW,
            expires_at: None,
        }
    }

    fn book() -> (tempfile::TempDir, Pairings) {
        let dir = tempfile::tempdir().unwrap();
        let pairings = Pairings::load(dir.path().join("pairings.json")).unwrap();
        (dir, pairings)
    }

    // --- admission ----------------------------------------------------------

    #[test]
    fn a_device_that_has_never_paired_accepts_nobody() {
        let (_d, pairings) = book();

        assert_eq!(pairings.admit(&key("aa"), NOW), Err(Refusal::Unknown));
    }

    #[test]
    fn a_paired_key_is_admitted_with_its_label() {
        let (_d, mut pairings) = book();
        pairings.add(pairing(&key("aa"))).unwrap();

        let admitted = pairings.admit(&key("aa"), NOW).unwrap();

        assert_eq!(admitted.label, "Safari on iPhone");
    }

    #[test]
    fn an_unpaired_key_is_refused_even_when_others_are_paired() {
        let (_d, mut pairings) = book();
        pairings.add(pairing(&key("aa"))).unwrap();

        assert_eq!(pairings.admit(&key("bb"), NOW), Err(Refusal::Unknown));
    }

    // --- revocation, which is the point of the module -----------------------

    /// The promise in §6.5: revocation works with the relay unreachable, because
    /// nothing is asked of anybody. This test needs no network by construction.
    #[test]
    fn a_revoked_key_is_refused_with_nothing_asked_of_anyone() {
        let (_d, mut pairings) = book();
        pairings.add(pairing(&key("aa"))).unwrap();

        pairings.revoke(&key("aa")).unwrap();

        assert_eq!(pairings.admit(&key("aa"), NOW), Err(Refusal::Revoked));
    }

    /// A revoked peer is not a stranger, and the difference matters to the person
    /// reading the screen: "you revoked this device" is an answer, "unknown device"
    /// reads like a bug.
    #[test]
    fn a_revoked_peer_is_distinguishable_from_a_stranger() {
        let (_d, mut pairings) = book();
        pairings.add(pairing(&key("aa"))).unwrap();
        pairings.revoke(&key("aa")).unwrap();

        assert_eq!(pairings.admit(&key("aa"), NOW), Err(Refusal::Revoked));
        assert_eq!(pairings.admit(&key("bb"), NOW), Err(Refusal::Unknown));
    }

    /// Re-pairing a revoked key must be deliberate. If `add` silently un-revoked,
    /// a peer that simply kept trying would let itself back in.
    #[test]
    fn a_revoked_key_cannot_be_re_added_by_accident() {
        let (_d, mut pairings) = book();
        pairings.add(pairing(&key("aa"))).unwrap();
        pairings.revoke(&key("aa")).unwrap();

        assert!(pairings.add(pairing(&key("aa"))).is_err());
        assert_eq!(pairings.admit(&key("aa"), NOW), Err(Refusal::Revoked));
    }

    #[test]
    fn un_revoking_is_explicit_and_then_pairing_works_again() {
        let (_d, mut pairings) = book();
        pairings.add(pairing(&key("aa"))).unwrap();
        pairings.revoke(&key("aa")).unwrap();

        pairings.unrevoke(&key("aa")).unwrap();
        pairings.add(pairing(&key("aa"))).unwrap();

        assert!(pairings.admit(&key("aa"), NOW).is_ok());
    }

    /// Revocation is checked first, so a key in both lists is refused. The other
    /// ordering would make a stale pairing entry override an explicit revocation.
    #[test]
    fn revocation_wins_over_a_pairing_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pairings.json");
        // Hand-written to put the key in both lists, which `add` and `revoke`
        // would never produce together.
        std::fs::write(
            &path,
            serde_json::json!({
                "pairings": [{
                    "peer_key": key("aa"),
                    "label": "stale",
                    "paired_at": NOW,
                    "expires_at": null
                }],
                "revoked": [key("aa")]
            })
            .to_string(),
        )
        .unwrap();

        let pairings = Pairings::load(&path).unwrap();

        assert_eq!(pairings.admit(&key("aa"), NOW), Err(Refusal::Revoked));
    }

    #[test]
    fn revoking_one_peer_leaves_the_others_alone() {
        let (_d, mut pairings) = book();
        pairings.add(pairing(&key("aa"))).unwrap();
        pairings.add(pairing(&key("bb"))).unwrap();

        pairings.revoke(&key("aa")).unwrap();

        assert_eq!(pairings.admit(&key("aa"), NOW), Err(Refusal::Revoked));
        assert!(pairings.admit(&key("bb"), NOW).is_ok());
    }

    // --- expiry -------------------------------------------------------------

    #[test]
    fn an_expired_pairing_is_refused() {
        let (_d, mut pairings) = book();
        let mut expiring = pairing(&key("aa"));
        expiring.expires_at = Some(NOW);
        pairings.add(expiring).unwrap();

        assert_eq!(pairings.admit(&key("aa"), NOW), Err(Refusal::Expired));
        assert!(pairings.admit(&key("aa"), NOW - 1).is_ok());
    }

    // --- persistence --------------------------------------------------------

    /// Revocation that did not survive a restart would be revocation that lasts
    /// until the app is closed, which is worse than none because it looks like it
    /// worked.
    #[test]
    fn revocation_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pairings.json");
        {
            let mut pairings = Pairings::load(&path).unwrap();
            pairings.add(pairing(&key("aa"))).unwrap();
            pairings.add(pairing(&key("bb"))).unwrap();
            pairings.revoke(&key("aa")).unwrap();
        }

        let reopened = Pairings::load(&path).unwrap();

        assert_eq!(reopened.admit(&key("aa"), NOW), Err(Refusal::Revoked));
        assert!(reopened.admit(&key("bb"), NOW).is_ok());
    }

    #[test]
    fn a_corrupt_pairing_file_is_an_error_rather_than_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pairings.json");
        std::fs::write(&path, "{ not json").unwrap();

        assert!(Pairings::load(&path).is_err());
    }

    #[test]
    fn adding_the_same_key_twice_replaces_rather_than_duplicates() {
        let (_d, mut pairings) = book();
        pairings.add(pairing(&key("aa"))).unwrap();

        let mut renamed = pairing(&key("aa"));
        renamed.label = "Firefox on Windows laptop".into();
        pairings.add(renamed).unwrap();

        assert_eq!(pairings.list().len(), 1);
        assert_eq!(
            pairings.admit(&key("aa"), NOW).unwrap().label,
            "Firefox on Windows laptop"
        );
    }

    #[test]
    fn a_pairing_can_be_renamed() {
        let (_d, mut pairings) = book();
        pairings.add(pairing(&key("aa"))).unwrap();

        pairings.rename(&key("aa"), "the old iPad").unwrap();

        assert_eq!(
            pairings.admit(&key("aa"), NOW).unwrap().label,
            "the old iPad"
        );
        assert!(pairings.rename(&key("bb"), "nope").is_err());
    }
}
