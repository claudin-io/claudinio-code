//! Turning remote access off.
//!
//! # Why this module exists
//!
//! `remote_enable` used to `tokio::spawn` a connection and keep no handle to it.
//! Nothing could stop it: not the settings panel, not the peer, not a revocation.
//! The only way out was quitting the app — which is a poor answer for a feature
//! whose whole promise is that you are somewhere else.
//!
//! Three different things need to stop a connection, and they all need the same
//! mechanism:
//!
//! - the user turning it off in the panel,
//! - the peer asking, via `EndRemote`,
//! - the device deciding, on a revocation or a lapsed grant.
//!
//! Each carries a `CloseReason`, because the peer must be told *why*. A peer that
//! only sees the socket drop cannot tell "you were turned off" from "the relay
//! hiccuped", and will sit reconnecting against a device that is never going to
//! answer.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use claudinio_protocol::inner::CloseReason;
use tokio::sync::watch;

/// The live connections, and the handle that stops each one.
///
/// Keyed by channel: one entry per connection, so stopping one leaves the others
/// alone. `stop_all` exists because the panel's switch is about remote access
/// rather than about a channel the user has never heard of.
#[derive(Default)]
pub struct Control {
    inner: Mutex<HashMap<String, Live>>,
}

/// One live connection.
struct Live {
    stop: watch::Sender<Option<CloseReason>>,
    /// Known only once the handshake completes, which is why it is not a
    /// constructor argument. Revocation needs it: §6.5 promises that revoking drops
    /// the session *immediately*, and without this the connection would keep going
    /// until the peer happened to reconnect.
    peer_key: Option<String>,
}

impl Control {
    pub fn new() -> Self {
        Self::default()
    }

    /// Announce a connection and take the receiver it should watch.
    ///
    /// A second registration for the same channel replaces the first and stops it:
    /// two tasks serving one channel would fight over the same Noise session, and
    /// the newer one is the one the user just asked for.
    pub fn register(&self, channel: &str) -> watch::Receiver<Option<CloseReason>> {
        let (stop, rx) = watch::channel(None);
        if let Ok(mut live) = self.inner.lock()
            && let Some(previous) = live.insert(
                channel.to_string(),
                Live {
                    stop,
                    peer_key: None,
                },
            )
        {
            let _ = previous.stop.send(Some(CloseReason::TurnedOffLocally));
        }
        rx
    }

    /// Note which peer a channel is serving, once the handshake says who it is.
    pub fn attach_peer(&self, channel: &str, peer_key: &str) {
        if let Ok(mut live) = self.inner.lock()
            && let Some(entry) = live.get_mut(channel)
        {
            entry.peer_key = Some(peer_key.to_string());
        }
    }

    /// Stop whatever this peer is connected on. `false` when it was not connected.
    ///
    /// This is what makes revocation mean what §6.5 says it means. Editing the
    /// pairing book alone only takes effect at the *next* handshake, so a peer that
    /// stayed connected would keep being served after being revoked — which is the
    /// one moment revocation has to work.
    pub fn stop_peer(&self, peer_key: &str, reason: CloseReason) -> bool {
        let Ok(mut live) = self.inner.lock() else {
            return false;
        };
        let channels: Vec<String> = live
            .iter()
            .filter(|(_, entry)| entry.peer_key.as_deref() == Some(peer_key))
            .map(|(channel, _)| channel.clone())
            .collect();

        let mut stopped = false;
        for channel in channels {
            if let Some(entry) = live.remove(&channel) {
                let _ = entry.stop.send(Some(reason));
                stopped = true;
            }
        }
        stopped
    }

    /// Stop everything, and say how many. This is what the panel's switch calls.
    pub fn stop_all(&self, reason: CloseReason) -> usize {
        let Ok(mut live) = self.inner.lock() else {
            return 0;
        };
        let stopped = live.len();
        for (_, entry) in live.drain() {
            let _ = entry.stop.send(Some(reason));
        }
        stopped
    }

    /// Drop a connection's entry without signalling it — for a task that has
    /// already finished on its own. Kept distinct from `stop` so a connection that
    /// ended by itself does not look like one the user turned off.
    pub fn forget(&self, channel: &str) {
        if let Ok(mut live) = self.inner.lock() {
            live.remove(channel);
        }
    }

    /// The live channels, so the panel can say "remote access is on" truthfully
    /// rather than from a flag someone forgot to clear.
    pub fn running(&self) -> Vec<String> {
        let Ok(live) = self.inner.lock() else {
            return Vec::new();
        };
        let mut channels: Vec<String> = live.keys().cloned().collect();
        channels.sort();
        channels
    }
}

/// The process-wide registry.
///
/// A global for the same reason `pairing::shared()` is one: threading it through
/// `AppState` would put a `#[cfg(feature = "remote")]` field in `state.rs` and leak
/// remote access into a module with no other reason to know it exists. Tests build
/// their own `Control`.
pub fn shared() -> &'static Arc<Control> {
    static SHARED: OnceLock<Arc<Control>> = OnceLock::new();
    SHARED.get_or_init(|| Arc::new(Control::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANNEL: &str = "ab";
    const OTHER: &str = "cd";

    #[test]
    fn a_registered_connection_is_running() {
        let control = Control::new();
        let _rx = control.register(CHANNEL);

        assert!(control.running().contains(&CHANNEL.to_string()));
        assert_eq!(control.running(), vec![CHANNEL.to_string()]);
    }

    const PEER: &str = "11";
    const OTHER_PEER: &str = "22";

    #[test]
    fn stopping_a_peer_delivers_the_reason() {
        let control = Control::new();
        let rx = control.register(CHANNEL);
        control.attach_peer(CHANNEL, PEER);

        assert!(control.stop_peer(PEER, CloseReason::Revoked));
        assert_eq!(*rx.borrow(), Some(CloseReason::Revoked));
    }

    #[test]
    fn a_stopped_connection_is_no_longer_running() {
        let control = Control::new();
        let _rx = control.register(CHANNEL);
        control.attach_peer(CHANNEL, PEER);
        control.stop_peer(PEER, CloseReason::Revoked);

        assert!(control.running().is_empty());
    }

    #[test]
    fn stopping_a_peer_that_is_not_connected_says_so() {
        let control = Control::new();
        assert!(!control.stop_peer(PEER, CloseReason::Revoked));
    }

    /// A peer whose handshake has not finished has no key attached yet. Revoking it
    /// must not stop somebody else's connection.
    #[test]
    fn a_connection_with_no_peer_yet_is_not_matched() {
        let control = Control::new();
        let rx = control.register(CHANNEL);

        assert!(!control.stop_peer(PEER, CloseReason::Revoked));
        assert_eq!(*rx.borrow(), None);
    }

    /// The panel's switch is about remote access, not about one channel.
    #[test]
    fn stop_all_stops_every_connection() {
        let control = Control::new();
        let one = control.register(CHANNEL);
        let two = control.register(OTHER);

        assert_eq!(control.stop_all(CloseReason::TurnedOffLocally), 2);
        assert_eq!(*one.borrow(), Some(CloseReason::TurnedOffLocally));
        assert_eq!(*two.borrow(), Some(CloseReason::TurnedOffLocally));
        assert!(control.running().is_empty());
    }

    /// Revoking one browser must not disconnect the others.
    #[test]
    fn revoking_one_peer_leaves_the_others_connected() {
        let control = Control::new();
        let one = control.register(CHANNEL);
        let two = control.register(OTHER);
        control.attach_peer(CHANNEL, PEER);
        control.attach_peer(OTHER, OTHER_PEER);

        control.stop_peer(PEER, CloseReason::Revoked);

        assert_eq!(*one.borrow(), Some(CloseReason::Revoked));
        assert_eq!(*two.borrow(), None);
        assert!(control.running().contains(&OTHER.to_string()));
    }

    /// One peer on two channels — it reconnected and the old entry lingered. Both go.
    #[test]
    fn a_peer_on_more_than_one_channel_is_fully_stopped() {
        let control = Control::new();
        let one = control.register(CHANNEL);
        let two = control.register(OTHER);
        control.attach_peer(CHANNEL, PEER);
        control.attach_peer(OTHER, PEER);

        assert!(control.stop_peer(PEER, CloseReason::Revoked));
        assert_eq!(*one.borrow(), Some(CloseReason::Revoked));
        assert_eq!(*two.borrow(), Some(CloseReason::Revoked));
        assert!(control.running().is_empty());
    }

    /// Two tasks on one channel would fight over the same Noise session. The newer
    /// one is what the user just asked for, so the older one goes.
    #[test]
    fn re_registering_a_channel_stops_the_previous_task() {
        let control = Control::new();
        let first = control.register(CHANNEL);
        let _second = control.register(CHANNEL);

        assert_eq!(*first.borrow(), Some(CloseReason::TurnedOffLocally));
        assert!(control.running().contains(&CHANNEL.to_string()));
        assert_eq!(control.running().len(), 1);
    }

    /// A connection that ended on its own must not read as one the user stopped.
    #[test]
    fn forgetting_does_not_signal() {
        let control = Control::new();
        let rx = control.register(CHANNEL);

        control.forget(CHANNEL);

        assert_eq!(*rx.borrow(), None);
        assert!(!control.running().contains(&CHANNEL.to_string()));
    }

    #[tokio::test]
    async fn a_watcher_wakes_when_it_is_stopped() {
        let control = Arc::new(Control::new());
        let mut rx = control.register(CHANNEL);

        let waiter = tokio::spawn(async move {
            rx.changed().await.ok();
            *rx.borrow()
        });

        control.stop_all(CloseReason::GrantExpired);

        assert_eq!(waiter.await.unwrap(), Some(CloseReason::GrantExpired));
    }

    /// The stop must be observable even if it lands before the serve loop next
    /// reaches its select — otherwise a connection told to stop while it was busy
    /// would keep serving until the next frame arrived.
    #[tokio::test]
    async fn a_stop_that_arrives_early_is_still_seen() {
        let control = Control::new();
        let mut rx = control.register(CHANNEL);
        control.stop_all(CloseReason::PeerAsked);

        // `changed()` returns immediately for a value sent before the wait.
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), Some(CloseReason::PeerAsked));
    }
}
