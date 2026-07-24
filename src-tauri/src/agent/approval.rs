//! Approval gates, resolved exactly once and attributed to whoever answered.
//!
//! The approval gate is the whole mitigation for prompt injection (SECURITY.md),
//! so the property that matters here is not "the user can answer" but "the
//! question is answered exactly once, and the transcript says by whom".
//!
//! The previous mechanism was a bare `HashMap<String, oneshot::Sender<bool>>`.
//! It already resolved once by accident — `remove` on the map made a second
//! answer find nothing — but it could not say *who* answered, and the loser of
//! a race got the string "not found or already handled", which is
//! indistinguishable from a stale request. Once a second answerer exists that
//! is not good enough: the losing UI has to close its own gate showing the
//! decision that actually won.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, oneshot};
use tokio::time::{Duration, Instant};

/// Who answered. `Peer` carries the pairing label rather than an opaque id so
/// the transcript reads as prose months later: "approved by iPhone 15".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "label", rename_all = "snake_case")]
pub enum Actor {
    /// Answered at the machine, through the desktop UI.
    Local,
    /// Answered by a paired remote peer.
    Peer(String),
    /// Nobody answered in time. A deny, never an approve — an approval gate
    /// that opens on its own is not a gate.
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub approved: bool,
    pub actor: Actor,
}

impl Decision {
    pub fn approved_by(actor: Actor) -> Self {
        Self {
            approved: true,
            actor,
        }
    }

    pub fn denied_by(actor: Actor) -> Self {
        Self {
            approved: false,
            actor,
        }
    }

    /// What the model is told when the gate closes against it.
    ///
    /// This string ends up in a tool result, so the model repeats it back to
    /// the user. Attributing it matters: reporting "rejected by user" for a
    /// request that simply timed out tells someone they refused something they
    /// never saw.
    pub fn rejection_message(&self, subject: &str) -> String {
        match &self.actor {
            Actor::Local => format!("{subject} rejected by user"),
            Actor::Peer(label) => format!("{subject} rejected by user from {label}"),
            Actor::Expired => {
                format!("{subject} denied: the approval request expired without an answer")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// Someone got there first. Carries the decision that won, so the loser can
    /// close its gate correctly instead of guessing.
    AlreadyResolved(Decision),
    /// No such gate: either it never existed or the session is gone.
    NotFound,
}

/// What a waiter gets back. A gate whose session dies mid-await must not hang
/// the agent loop, so that case is an explicit variant rather than a lost
/// oneshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitError {
    Abandoned,
}

struct Pending {
    tx: Option<oneshot::Sender<Decision>>,
    decided: Option<Decision>,
    expires_at: Option<Instant>,
}

/// The registry of open approval gates, keyed by `"{session_id}:{tool_use_id}"`.
#[derive(Clone, Default)]
pub struct ApprovalRegistry {
    pending: Arc<Mutex<HashMap<String, Pending>>>,
}

/// Handle held by the agent loop while it waits for an answer.
pub struct ApprovalHandle {
    key: String,
    rx: oneshot::Receiver<Decision>,
    expires_at: Option<Instant>,
    registry: ApprovalRegistry,
}

impl ApprovalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a gate. `ttl` of `None` means it waits forever, which is the
    /// current local-only behaviour; remote gates always carry one.
    pub async fn register(&self, key: &str, ttl: Option<Duration>) -> ApprovalHandle {
        let (tx, rx) = oneshot::channel();
        let expires_at = ttl.map(|d| Instant::now() + d);
        self.pending.lock().await.insert(
            key.to_string(),
            Pending {
                tx: Some(tx),
                decided: None,
                expires_at,
            },
        );
        ApprovalHandle {
            key: key.to_string(),
            rx,
            expires_at,
            registry: self.clone(),
        }
    }

    /// Answer a gate. First writer wins; everyone after learns who won.
    pub async fn resolve(
        &self,
        key: &str,
        approved: bool,
        actor: Actor,
    ) -> Result<Decision, ResolveError> {
        let mut pending = self.pending.lock().await;
        let entry = pending.get_mut(key).ok_or(ResolveError::NotFound)?;

        if let Some(decided) = &entry.decided {
            return Err(ResolveError::AlreadyResolved(decided.clone()));
        }

        // A gate past its deadline settles as a denial even if nobody has
        // observed the expiry yet, so a late answer cannot reopen it. Without
        // this, an answer that raced the deadline could approve a tool the user
        // was already told had been denied.
        if entry.expires_at.is_some_and(|at| Instant::now() >= at) {
            let expired = Decision::denied_by(Actor::Expired);
            entry.decided = Some(expired.clone());
            if let Some(tx) = entry.tx.take() {
                let _ = tx.send(expired.clone());
            }
            return Err(ResolveError::AlreadyResolved(expired));
        }

        let decision = if approved {
            Decision::approved_by(actor)
        } else {
            Decision::denied_by(actor)
        };
        entry.decided = Some(decision.clone());
        if let Some(tx) = entry.tx.take() {
            // The waiter may be gone — the agent loop can be torn down while a
            // gate is open. The decision is still recorded, so a reconnecting
            // UI reconciles correctly.
            let _ = tx.send(decision.clone());
        }
        Ok(decision)
    }

    /// Drop every gate belonging to a session. Waiters get `Abandoned`.
    ///
    /// This is also what bounds the map: resolved gates are deliberately kept
    /// so a replayed answer gets `AlreadyResolved` rather than `NotFound`, and
    /// the session ending is what clears them.
    pub async fn abandon_session(&self, session_id: &str) {
        let prefix = format!("{session_id}:");
        self.pending
            .lock()
            .await
            .retain(|k, _| !k.starts_with(&prefix));
    }
}

impl ApprovalHandle {
    /// Await the answer. Returns the winning decision, an expiry deny, or
    /// `Abandoned` if the gate was torn down.
    pub async fn wait(self) -> Result<Decision, WaitError> {
        let Some(deadline) = self.expires_at else {
            return self.rx.await.map_err(|_| WaitError::Abandoned);
        };

        match tokio::time::timeout_at(deadline, self.rx).await {
            Ok(Ok(decision)) => Ok(decision),
            Ok(Err(_)) => Err(WaitError::Abandoned),
            Err(_) => {
                // The deadline passed with nobody answering. Settling through
                // `resolve` rather than returning a local value keeps a single
                // source of truth: an answer that arrives a moment later must
                // lose to this expiry, which it can only do if the expiry is
                // recorded in the registry.
                match self
                    .registry
                    .resolve(&self.key, false, Actor::Expired)
                    .await
                {
                    Ok(decision) => Ok(decision),
                    Err(ResolveError::AlreadyResolved(decision)) => Ok(decision),
                    Err(ResolveError::NotFound) => Err(WaitError::Abandoned),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "session-1:tool-1";

    #[tokio::test]
    async fn a_resolved_gate_hands_the_decision_to_the_waiter() {
        let reg = ApprovalRegistry::new();
        let handle = reg.register(KEY, None).await;

        reg.resolve(KEY, true, Actor::Local).await.unwrap();

        assert_eq!(handle.wait().await, Ok(Decision::approved_by(Actor::Local)));
    }

    #[tokio::test]
    async fn a_denial_carries_its_actor_too() {
        let reg = ApprovalRegistry::new();
        let handle = reg.register(KEY, None).await;

        reg.resolve(KEY, false, Actor::Peer("iPhone".into()))
            .await
            .unwrap();

        assert_eq!(
            handle.wait().await,
            Ok(Decision::denied_by(Actor::Peer("iPhone".into())))
        );
    }

    /// The core of the design: local and remote can both answer, and the loser
    /// must be told what won rather than being told the gate does not exist.
    #[tokio::test]
    async fn the_second_answer_loses_and_learns_the_winner() {
        let reg = ApprovalRegistry::new();
        let handle = reg.register(KEY, None).await;

        let first = reg.resolve(KEY, true, Actor::Local).await;
        let second = reg.resolve(KEY, false, Actor::Peer("iPhone".into())).await;

        assert_eq!(first, Ok(Decision::approved_by(Actor::Local)));
        assert_eq!(
            second,
            Err(ResolveError::AlreadyResolved(Decision::approved_by(
                Actor::Local
            )))
        );
        // The waiter sees the winner, not the last writer.
        assert_eq!(handle.wait().await, Ok(Decision::approved_by(Actor::Local)));
    }

    /// A duplicated remote frame must not turn into a duplicated `rm`. Same
    /// actor, same decision, still exactly one resolution.
    #[tokio::test]
    async fn replaying_the_same_answer_does_not_resolve_twice() {
        let reg = ApprovalRegistry::new();
        let handle = reg.register(KEY, None).await;

        let peer = Actor::Peer("iPhone".into());
        assert!(reg.resolve(KEY, true, peer.clone()).await.is_ok());

        for _ in 0..3 {
            assert_eq!(
                reg.resolve(KEY, true, peer.clone()).await,
                Err(ResolveError::AlreadyResolved(Decision::approved_by(
                    peer.clone()
                )))
            );
        }

        assert_eq!(handle.wait().await, Ok(Decision::approved_by(peer)));
    }

    /// The sequential tests above show the rule; this one exercises it. Twenty
    /// answerers race the same gate and exactly one may win — "a duplicated
    /// approval is a duplicated `rm`" is the reason this module exists.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn exactly_one_answer_wins_under_concurrency() {
        for round in 0..50 {
            let reg = ApprovalRegistry::new();
            let handle = reg.register(KEY, None).await;

            let mut racers = Vec::new();
            for i in 0..20 {
                let reg = reg.clone();
                racers.push(tokio::spawn(async move {
                    reg.resolve(KEY, i % 2 == 0, Actor::Peer(format!("peer-{i}")))
                        .await
                }));
            }

            let mut winners = Vec::new();
            let mut losers = Vec::new();
            for racer in racers {
                match racer.await.unwrap() {
                    Ok(decision) => winners.push(decision),
                    Err(e) => losers.push(e),
                }
            }

            assert_eq!(winners.len(), 1, "round {round}: {} winners", winners.len());
            assert_eq!(losers.len(), 19);

            let winner = winners.pop().unwrap();
            // Every loser must name the same winner — a loser that reported a
            // different decision would close its gate showing the wrong answer.
            for loser in losers {
                assert_eq!(loser, ResolveError::AlreadyResolved(winner.clone()));
            }
            assert_eq!(handle.wait().await, Ok(winner));
        }
    }

    #[tokio::test]
    async fn answering_an_unknown_gate_is_not_found() {
        let reg = ApprovalRegistry::new();
        assert_eq!(
            reg.resolve("nope:nope", true, Actor::Local).await,
            Err(ResolveError::NotFound)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_unanswered_gate_expires_into_a_denial() {
        let reg = ApprovalRegistry::new();
        let handle = reg.register(KEY, Some(Duration::from_secs(60))).await;

        tokio::time::advance(Duration::from_secs(61)).await;

        // Expiry denies. An approval gate that opens on its own is not a gate.
        assert_eq!(handle.wait().await, Ok(Decision::denied_by(Actor::Expired)));
    }

    /// A late answer arriving after expiry must not reopen a closed gate — the
    /// tool has already been reported as denied.
    #[tokio::test(start_paused = true)]
    async fn an_answer_after_expiry_loses_to_the_expiry() {
        let reg = ApprovalRegistry::new();
        let handle = reg.register(KEY, Some(Duration::from_secs(60))).await;

        tokio::time::advance(Duration::from_secs(61)).await;
        assert_eq!(handle.wait().await, Ok(Decision::denied_by(Actor::Expired)));

        assert_eq!(
            reg.resolve(KEY, true, Actor::Peer("iPhone".into())).await,
            Err(ResolveError::AlreadyResolved(Decision::denied_by(
                Actor::Expired
            )))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_answer_just_before_expiry_still_wins() {
        let reg = ApprovalRegistry::new();
        let handle = reg.register(KEY, Some(Duration::from_secs(60))).await;

        tokio::time::advance(Duration::from_secs(59)).await;
        reg.resolve(KEY, true, Actor::Local).await.unwrap();

        assert_eq!(handle.wait().await, Ok(Decision::approved_by(Actor::Local)));
    }

    #[tokio::test]
    async fn abandoning_a_session_releases_its_waiters() {
        let reg = ApprovalRegistry::new();
        let mine = reg.register("session-1:tool-1", None).await;
        let other = reg.register("session-2:tool-1", None).await;

        reg.abandon_session("session-1").await;

        assert_eq!(mine.wait().await, Err(WaitError::Abandoned));
        // A sibling session is untouched.
        reg.resolve("session-2:tool-1", true, Actor::Local)
            .await
            .unwrap();
        assert_eq!(other.wait().await, Ok(Decision::approved_by(Actor::Local)));
    }

    /// Gates are per tool call, so a session with several open must keep them
    /// independent — answering one must not resolve the others.
    #[tokio::test]
    async fn gates_in_the_same_session_are_independent() {
        let reg = ApprovalRegistry::new();
        let first = reg.register("session-1:tool-1", None).await;
        let second = reg.register("session-1:tool-2", None).await;

        reg.resolve("session-1:tool-1", true, Actor::Local)
            .await
            .unwrap();

        assert_eq!(first.wait().await, Ok(Decision::approved_by(Actor::Local)));

        reg.resolve("session-1:tool-2", false, Actor::Local)
            .await
            .unwrap();
        assert_eq!(second.wait().await, Ok(Decision::denied_by(Actor::Local)));
    }

    /// An expiry must not be reported to the model as a human refusal — the
    /// model repeats that back, and telling someone they rejected something
    /// they never saw is worse than saying nothing.
    #[test]
    fn a_denial_explains_itself_differently_per_actor() {
        // The local wording is unchanged from before actors existed, so the
        // common case reads exactly as it always did.
        assert_eq!(
            Decision::denied_by(Actor::Local).rejection_message("Command"),
            "Command rejected by user"
        );
        assert_eq!(
            Decision::denied_by(Actor::Peer("iPhone".into())).rejection_message("Edit"),
            "Edit rejected by user from iPhone"
        );

        let expired = Decision::denied_by(Actor::Expired).rejection_message("Command");
        assert!(expired.contains("expired"), "{expired}");
        assert!(
            !expired.contains("rejected by user"),
            "an expiry must not be reported as a human refusal: {expired}"
        );
    }

    /// The actor survives a round trip through JSONL, which is what makes the
    /// transcript answer "who approved this?" months later.
    #[test]
    fn actor_round_trips_through_json() {
        for actor in [
            Actor::Local,
            Actor::Peer("Firefox on Windows laptop".into()),
            Actor::Expired,
        ] {
            let decision = Decision::approved_by(actor.clone());
            let json = serde_json::to_string(&decision).unwrap();
            assert_eq!(
                serde_json::from_str::<Decision>(&json).unwrap(),
                decision,
                "round trip failed for {actor:?} via {json}"
            );
        }
    }
}
