//! Translation between the agent and the wire, and the place every peer command
//! is checked before it does anything.
//!
//! # Two sequence numbers, not one
//!
//! §5.4 of the plan says every device→peer message carries a `seq` derived from
//! the JSONL append index. That cannot be true of all of them, and the gap
//! matters.
//!
//! Live events — token deltas above all — are never persisted. They have no
//! append index, because there is nothing on disk to index. Numbering them with
//! a record index would produce a number that looks resumable and is not: asking
//! to resume from it would replay the wrong span, or nothing.
//!
//! So there are two counters and they mean different things:
//!
//! - `Event.seq` is a per-connection counter. Its only job is gap detection on
//!   the live stream: contiguous means nothing was dropped.
//! - `Snapshot.seq` is the JSONL index, and it is the only thing a peer may
//!   resume from. `Subscribe { from_seq }` is answered out of the transcript.
//!
//! Recovering from a live gap is therefore not "resume from the last event" but
//! "re-subscribe and replay records", which is what makes the relay stateless.

use claudinio_protocol::inner::{self as wire, BashApproval, DeviceToPeer, PeerToDevice, Policy};

use super::dedup::{CommandLog, Outcome, Verdict};
use crate::agent::approval;
use crate::agent::eventbus::Delivery;

/// The agent's `Actor` and the wire's are deliberately separate types — see
/// §0.2 of the plan. This is the seam where they meet, and the only place that
/// has to be kept in step.
pub fn actor_to_wire(actor: &approval::Actor) -> wire::Actor {
    match actor {
        approval::Actor::Local => wire::Actor::Local,
        approval::Actor::Peer(label) => wire::Actor::Peer(label.clone()),
        approval::Actor::Expired => wire::Actor::Expired,
    }
}

/// No inbound message carries an actor today, so nothing calls this yet. It is
/// kept because the mapping belongs in one place and is only trustworthy if it is
/// tested in both directions — a one-way conversion tested one way can be wrong
/// in a way no test would see.
#[allow(dead_code)]
pub fn actor_from_wire(actor: &wire::Actor) -> approval::Actor {
    match actor {
        wire::Actor::Local => approval::Actor::Local,
        wire::Actor::Peer(label) => approval::Actor::Peer(label.clone()),
        wire::Actor::Expired => approval::Actor::Expired,
    }
}

/// What answering a gate produced.
///
/// Losing the race is not a failure: the gate is closed, which is what the peer
/// wanted. This carries the winning decision so the bridge can send
/// `ApprovalResolved` and the remote UI can close its own gate showing what
/// actually won, per §5.3 — rather than showing an error for something that
/// worked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Resolved,
    AlreadyResolved(approval::Decision),
}

/// Everything the bridge is allowed to ask of the device.
///
/// A trait rather than a direct call into `commands/`, for two reasons. It keeps
/// the dependency arrow pointing the right way, and it makes the command path
/// testable without standing up an app — the tests below drive it with a fake
/// that counts calls, which is how "a replayed frame executes once" is asserted
/// on the execution and not on a log line.
pub trait DeviceActions {
    async fn resolve_approval(
        &self,
        session_id: &str,
        tool_use_id: &str,
        approved: bool,
        actor: approval::Actor,
    ) -> Result<ApprovalOutcome, String>;

    async fn send_message(&self, session_id: &str, text: &str) -> Result<(), String>;

    async fn steer(&self, session_id: &str, text: &str) -> Result<(), String>;

    async fn interrupt(&self, session_id: &str) -> Result<(), String>;
}

/// So a reconnect loop can give a fresh bridge the same actions on every attempt
/// without moving them, and without requiring them to be `Clone`.
impl<T: DeviceActions + Sync> DeviceActions for &T {
    async fn resolve_approval(
        &self,
        session_id: &str,
        tool_use_id: &str,
        approved: bool,
        actor: approval::Actor,
    ) -> Result<ApprovalOutcome, String> {
        (**self)
            .resolve_approval(session_id, tool_use_id, approved, actor)
            .await
    }

    async fn send_message(&self, session_id: &str, text: &str) -> Result<(), String> {
        (**self).send_message(session_id, text).await
    }

    async fn steer(&self, session_id: &str, text: &str) -> Result<(), String> {
        (**self).steer(session_id, text).await
    }

    async fn interrupt(&self, session_id: &str) -> Result<(), String> {
        (**self).interrupt(session_id).await
    }
}

pub struct Bridge<A: DeviceActions> {
    session_id: String,
    /// How this peer is named in the transcript and in `ApprovalResolved`.
    peer_label: String,
    /// Enforced here even though the editor for it is phase 3 work. The type
    /// defaults to granting nothing, so a bridge built before there is any way
    /// to widen it refuses everything — the enforcement point exists before the
    /// capability does, rather than after.
    policy: Policy,
    log: CommandLog,
    actions: A,
    event_seq: u64,
    /// The session's JSONL. `Subscribe` is answered out of this rather than from
    /// a buffer, which is what lets the relay hold no state at all.
    store_path: std::path::PathBuf,
}

impl<A: DeviceActions> Bridge<A> {
    pub fn new(
        session_id: impl Into<String>,
        peer_label: impl Into<String>,
        policy: Policy,
        log: CommandLog,
        actions: A,
        store_path: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            peer_label: peer_label.into(),
            policy,
            log,
            actions,
            event_seq: 0,
            store_path: store_path.into(),
        }
    }

    /// Answer `Subscribe` by replaying the transcript from `from_seq`.
    ///
    /// Chunked, because one session can be far larger than a frame. Each chunk
    /// carries the highest `seq` it contains, so a peer that is cut off partway
    /// resumes from the last chunk it actually received rather than starting over.
    fn snapshot(&self, from_seq: u64) -> Vec<DeviceToPeer> {
        let records = match crate::agent::persist::replay(&self.store_path, from_seq) {
            Ok(records) => records,
            Err(e) => {
                return vec![DeviceToPeer::Error {
                    cmd_id: String::new(),
                    code: "replay_failed".into(),
                    message: e,
                }];
            }
        };

        let mut chunks = Vec::new();
        let mut batch: Vec<serde_json::Value> = Vec::new();
        let mut batch_bytes = 0usize;
        let mut highest = from_seq.saturating_sub(1);

        for record in records {
            let value = match serde_json::to_value(&record.record) {
                Ok(value) => value,
                // One unserialisable record must not sink the whole replay.
                Err(_) => continue,
            };
            let size = value.to_string().len();

            // Flush before the batch would outgrow a frame, not after.
            if !batch.is_empty() && batch_bytes + size > SNAPSHOT_BUDGET {
                chunks.push(DeviceToPeer::Snapshot {
                    session_id: self.session_id.clone(),
                    records: std::mem::take(&mut batch),
                    seq: highest,
                });
                batch_bytes = 0;
            }

            highest = record.seq;
            batch_bytes += size;
            batch.push(value);
        }

        // Always send a final chunk, even when empty: a peer asking to resume
        // from the end must be told it is up to date rather than left waiting.
        chunks.push(DeviceToPeer::Snapshot {
            session_id: self.session_id.clone(),
            records: batch,
            seq: highest,
        });
        chunks
    }

    /// Turn something off the bus into something for the wire.
    ///
    /// `None` means there is nothing to send — a closed bus is the run ending,
    /// which the peer learns from the transcript rather than from a frame.
    pub fn translate(&mut self, delivery: Delivery) -> Option<DeviceToPeer> {
        match delivery {
            Delivery::Event(event) => {
                self.event_seq += 1;
                Some(DeviceToPeer::Event {
                    session_id: self.session_id.clone(),
                    seq: self.event_seq,
                    // The agent's event type is not part of the protocol: it is
                    // carried as its own serialisation so the wire does not have
                    // to grow a variant every time the agent gains one.
                    event: serde_json::to_value(&*event).unwrap_or(serde_json::Value::Null),
                })
            }
            Delivery::Gap { missed } => {
                // The peer is told the span it lost in the *live* numbering, and
                // its recovery is to re-subscribe and replay records — not to
                // ask for these numbers back, which no longer exist anywhere.
                let from = self.event_seq + 1;
                self.event_seq += missed;
                Some(DeviceToPeer::Gap {
                    session_id: self.session_id.clone(),
                    from_seq: from,
                    to_seq: self.event_seq,
                })
            }
            Delivery::Closed => None,
        }
    }

    /// Handle one command from the peer.
    ///
    /// Order matters and is deliberate: policy first, then deduplication, then
    /// the side effect. Checking policy first means a command the peer may not
    /// issue is never recorded as having been attempted, so a denied command
    /// stays retryable after the policy widens.
    pub async fn handle(&mut self, message: PeerToDevice) -> Vec<DeviceToPeer> {
        // Queries carry no cmd_id because they have no side effect, so they skip
        // deduplication entirely — replaying a read is harmless.
        let Some(cmd_id) = command_id(&message) else {
            return match message {
                PeerToDevice::Subscribe { from_seq, .. } => self.snapshot(from_seq),
                // The peer is told what it may do so its UI can grey out the
                // rest, rather than offering actions that will be refused.
                PeerToDevice::GetPolicy => vec![DeviceToPeer::Policy(self.policy.clone())],
                PeerToDevice::Unsubscribe { .. } => vec![],
                other => vec![DeviceToPeer::Error {
                    cmd_id: String::new(),
                    code: "unsupported".into(),
                    message: format!("{} is not implemented yet", tag_of(&other)),
                }],
            };
        };

        if let Some((rule, why)) = self.denied_by_policy(&message) {
            return vec![DeviceToPeer::PolicyDenied {
                cmd_id,
                rule: rule.into(),
                human_reason: why.into(),
            }];
        }

        match self.log.verdict(&cmd_id) {
            Verdict::Done(Outcome::Acked) => return vec![DeviceToPeer::Ack { cmd_id }],
            Verdict::Done(Outcome::Failed { code, message }) => {
                return vec![DeviceToPeer::Error {
                    cmd_id,
                    code,
                    message,
                }];
            }
            Verdict::Indeterminate => {
                // The device died between starting this command and finishing
                // it, so whether the side effect happened is unknowable. Refuse
                // and say so: a duplicated approval is a duplicated `rm`.
                return vec![DeviceToPeer::Error {
                    cmd_id,
                    code: "indeterminate".into(),
                    message: "this command was interrupted mid-execution and will not be \
                              retried automatically; issue it again with a new id if you \
                              still want it"
                        .into(),
                }];
            }
            Verdict::Fresh => {}
        }

        if let Err(e) = self.log.begin(&cmd_id) {
            // Unable to record the attempt means unable to promise it runs once.
            // Refuse rather than run something we cannot dedupe.
            return vec![DeviceToPeer::Error {
                cmd_id,
                code: "log_unavailable".into(),
                message: e,
            }];
        }

        let result = self.execute(&message).await;

        let outcome = match &result {
            Ok(_) => Outcome::Acked,
            Err(e) => Outcome::Failed {
                code: "failed".into(),
                message: e.clone(),
            },
        };
        let _ = self.log.finish(&cmd_id, outcome);

        match result {
            // Losing the race is a success with news attached: the gate is
            // closed, and the peer is told which answer won so its UI shows that
            // rather than an error for something that worked.
            Ok(Some((tool_use_id, decision))) => vec![
                DeviceToPeer::Ack { cmd_id },
                DeviceToPeer::ApprovalResolved {
                    tool_use_id,
                    decision: wire::Decision {
                        approved: decision.approved,
                    },
                    actor: actor_to_wire(&decision.actor),
                },
            ],
            Ok(None) => vec![DeviceToPeer::Ack { cmd_id }],
            Err(message) => vec![DeviceToPeer::Error {
                cmd_id,
                code: "failed".into(),
                message,
            }],
        }
    }

    /// `Some((tool_use_id, decision))` when a gate was already closed by someone
    /// else, so the caller can tell the peer who won.
    async fn execute(
        &self,
        message: &PeerToDevice,
    ) -> Result<Option<(String, approval::Decision)>, String> {
        match message {
            PeerToDevice::ResolveApproval {
                session_id,
                tool_use_id,
                decision,
                ..
            } => {
                let outcome = self
                    .actions
                    .resolve_approval(
                        session_id,
                        tool_use_id,
                        decision.approved,
                        approval::Actor::Peer(self.peer_label.clone()),
                    )
                    .await?;
                Ok(match outcome {
                    ApprovalOutcome::Resolved => None,
                    ApprovalOutcome::AlreadyResolved(winner) => Some((tool_use_id.clone(), winner)),
                })
            }
            PeerToDevice::SendMessage {
                session_id, text, ..
            } => self
                .actions
                .send_message(session_id, text)
                .await
                .map(|_| None),
            PeerToDevice::Steer {
                session_id, text, ..
            } => self.actions.steer(session_id, text).await.map(|_| None),
            PeerToDevice::Interrupt { session_id, .. } => {
                self.actions.interrupt(session_id).await.map(|_| None)
            }
            other => Err(format!("{} is not implemented yet", tag_of(other))),
        }
    }

    /// Which rule blocks this command, if any.
    ///
    /// Every denial names a rule. A peer that cannot tell "you may not" from
    /// "something broke" retries forever, and a UI that cannot say which rule
    /// leaves the user with nothing to change.
    fn denied_by_policy(&self, message: &PeerToDevice) -> Option<(&'static str, &'static str)> {
        match message {
            PeerToDevice::SendMessage { .. } if !self.policy.send_message => Some((
                "allow.send_message",
                "sending messages is not granted remotely",
            )),
            PeerToDevice::Steer { .. } if !self.policy.steer => {
                Some(("allow.steer", "steering is not granted remotely"))
            }
            PeerToDevice::Interrupt { .. } if !self.policy.interrupt => {
                Some(("allow.interrupt", "interrupting is not granted remotely"))
            }
            PeerToDevice::SetMode { .. } if !self.policy.set_mode => {
                Some(("allow.set_mode", "changing mode is not granted remotely"))
            }
            // Approving is split by what is being approved, because approving an
            // edit and approving a shell command are not the same risk. The
            // bridge cannot see which from the message alone, so the stricter of
            // the two gates it: with bash approval switched off entirely, no
            // remote approval passes.
            PeerToDevice::ResolveApproval { decision, .. }
                if decision.approved
                    && !self.policy.approve_edit
                    && self.policy.approve_bash == BashApproval::Never =>
            {
                Some((
                    "allow.approve_edit / allow.approve_bash",
                    "approving is not granted remotely",
                ))
            }
            _ => None,
        }
    }
}

/// How much JSON to put in one `Snapshot`.
///
/// Well under the 256 KiB frame ceiling: the frame also carries Noise overhead
/// and the MessagePack envelope, and a chunk that overflowed would be dropped
/// rather than split.
const SNAPSHOT_BUDGET: usize = 64 * 1024;

/// The dedup key, for commands that have one.
fn command_id(message: &PeerToDevice) -> Option<String> {
    match message {
        PeerToDevice::SendMessage { cmd_id, .. }
        | PeerToDevice::Steer { cmd_id, .. }
        | PeerToDevice::Interrupt { cmd_id, .. }
        | PeerToDevice::ResolveApproval { cmd_id, .. }
        | PeerToDevice::SetMode { cmd_id, .. } => Some(cmd_id.clone()),
        PeerToDevice::ListWorkspaces
        | PeerToDevice::ListSessions { .. }
        | PeerToDevice::Subscribe { .. }
        | PeerToDevice::Unsubscribe { .. }
        | PeerToDevice::GetPolicy => None,
    }
}

fn tag_of(message: &PeerToDevice) -> &'static str {
    match message {
        PeerToDevice::ListWorkspaces => "list_workspaces",
        PeerToDevice::ListSessions { .. } => "list_sessions",
        PeerToDevice::Subscribe { .. } => "subscribe",
        PeerToDevice::Unsubscribe { .. } => "unsubscribe",
        PeerToDevice::SendMessage { .. } => "send_message",
        PeerToDevice::Steer { .. } => "steer",
        PeerToDevice::Interrupt { .. } => "interrupt",
        PeerToDevice::ResolveApproval { .. } => "resolve_approval",
        PeerToDevice::SetMode { .. } => "set_mode",
        PeerToDevice::GetPolicy => "get_policy",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session::AgentEvent;
    use std::sync::Mutex;

    /// Counts calls, so "executed once" is asserted on the execution rather than
    /// on a log line that might be written twice or not at all.
    #[derive(Default)]
    struct Spy {
        approvals: Mutex<Vec<(String, bool, approval::Actor)>>,
        messages: Mutex<Vec<String>>,
        interrupts: Mutex<usize>,
        fail: bool,
        /// When set, every approval loses the race to this decision.
        already_resolved: Option<approval::Decision>,
    }

    impl DeviceActions for Spy {
        async fn resolve_approval(
            &self,
            _session_id: &str,
            tool_use_id: &str,
            approved: bool,
            actor: approval::Actor,
        ) -> Result<ApprovalOutcome, String> {
            if self.fail {
                return Err("the gate was already closed".into());
            }
            if let Some(winner) = &self.already_resolved {
                return Ok(ApprovalOutcome::AlreadyResolved(winner.clone()));
            }
            self.approvals
                .lock()
                .unwrap()
                .push((tool_use_id.to_string(), approved, actor));
            Ok(ApprovalOutcome::Resolved)
        }

        async fn send_message(&self, _session_id: &str, text: &str) -> Result<(), String> {
            self.messages.lock().unwrap().push(text.to_string());
            Ok(())
        }

        async fn steer(&self, _session_id: &str, text: &str) -> Result<(), String> {
            self.messages.lock().unwrap().push(format!("steer: {text}"));
            Ok(())
        }

        async fn interrupt(&self, _session_id: &str) -> Result<(), String> {
            *self.interrupts.lock().unwrap() += 1;
            Ok(())
        }
    }

    /// A policy that allows everything, for tests about something other than
    /// policy. Never a default anywhere outside tests.
    fn permissive() -> Policy {
        Policy {
            send_message: true,
            steer: true,
            interrupt: true,
            set_mode: true,
            approve_edit: true,
            approve_bash: BashApproval::Allowlist,
            read_attachment: false,
            export_file: false,
            expires_at: None,
        }
    }

    fn bridge(policy: Policy) -> (tempfile::TempDir, Bridge<Spy>) {
        bridge_with_transcript(policy, &[])
    }

    fn bridge_with_transcript(policy: Policy, lines: &[&str]) -> (tempfile::TempDir, Bridge<Spy>) {
        let dir = tempfile::tempdir().unwrap();
        let log = CommandLog::open(dir.path().join("commands.jsonl"));
        let store = dir.path().join("session.jsonl");
        std::fs::write(
            &store,
            if lines.is_empty() {
                String::new()
            } else {
                format!("{}\n", lines.join("\n"))
            },
        )
        .unwrap();
        (
            dir,
            Bridge::new("session-1", "iPhone", policy, log, Spy::default(), store),
        )
    }

    fn user_record(text: &str) -> String {
        serde_json::json!({ "kind": "user", "text": text, "ts": 1 }).to_string()
    }

    fn approve(cmd_id: &str) -> PeerToDevice {
        PeerToDevice::ResolveApproval {
            cmd_id: cmd_id.into(),
            session_id: "session-1".into(),
            tool_use_id: "tool-1".into(),
            decision: wire::Decision { approved: true },
            reason: None,
        }
    }

    // --- the seam between the two Actor types -------------------------------

    /// The two types are duplicated on purpose, so this is where drift would
    /// show. Every variant, both directions.
    #[test]
    fn actors_convert_both_ways_without_losing_anything() {
        let cases = [
            approval::Actor::Local,
            approval::Actor::Peer("Firefox on Windows laptop".into()),
            approval::Actor::Expired,
        ];

        for actor in cases {
            let round_tripped = actor_from_wire(&actor_to_wire(&actor));
            assert_eq!(round_tripped, actor, "lost information for {actor:?}");
        }

        // And from the wire side, so a new wire variant cannot be silently
        // mapped onto the wrong agent one.
        for actor in [
            wire::Actor::Local,
            wire::Actor::Peer("iPhone".into()),
            wire::Actor::Expired,
        ] {
            assert_eq!(actor_to_wire(&actor_from_wire(&actor)), actor);
        }
    }

    // --- dedup, the reason the module exists --------------------------------

    /// Asserted on the spy's call count, not on a log line: a replayed approval
    /// frame must resolve to exactly one execution.
    #[tokio::test]
    async fn a_replayed_approval_executes_once() {
        let (_d, mut bridge) = bridge(permissive());

        let first = bridge.handle(approve("cmd-1")).await;
        let second = bridge.handle(approve("cmd-1")).await;
        let third = bridge.handle(approve("cmd-1")).await;

        assert_eq!(bridge.actions.approvals.lock().unwrap().len(), 1);
        // And every reply is the same Ack, so the peer cannot tell the
        // difference and stops retrying.
        for reply in [first, second, third] {
            assert!(matches!(reply[0], DeviceToPeer::Ack { .. }), "{reply:?}");
        }
    }

    #[tokio::test]
    async fn the_peer_label_is_what_lands_in_the_approval() {
        let (_d, mut bridge) = bridge(permissive());

        bridge.handle(approve("cmd-1")).await;

        let approvals = bridge.actions.approvals.lock().unwrap();
        assert_eq!(
            approvals[0],
            (
                "tool-1".to_string(),
                true,
                approval::Actor::Peer("iPhone".into())
            )
        );
    }

    /// A failure is remembered as a failure, so a retry gets the same answer
    /// rather than a second attempt.
    #[tokio::test]
    async fn a_failed_command_replays_its_failure() {
        let dir = tempfile::tempdir().unwrap();
        let log = CommandLog::open(dir.path().join("commands.jsonl"));
        let spy = Spy {
            fail: true,
            ..Default::default()
        };
        let mut bridge = Bridge::new(
            "session-1",
            "iPhone",
            permissive(),
            log,
            spy,
            dir.path().join("session.jsonl"),
        );

        let first = bridge.handle(approve("cmd-1")).await;
        let second = bridge.handle(approve("cmd-1")).await;

        assert!(matches!(first[0], DeviceToPeer::Error { .. }));
        assert!(matches!(second[0], DeviceToPeer::Error { .. }));
        assert_eq!(bridge.actions.approvals.lock().unwrap().len(), 0);
    }

    /// A command interrupted mid-execution comes back refused, not retried. The
    /// side effect may or may not have happened, and guessing wrong runs an `rm`
    /// twice.
    #[tokio::test]
    async fn an_interrupted_command_is_refused_rather_than_retried() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("commands.jsonl");
        {
            // A device that started this command and died.
            let mut log = CommandLog::open(&path);
            log.begin("cmd-1").unwrap();
        }
        let mut bridge = Bridge::new(
            "session-1",
            "iPhone",
            permissive(),
            CommandLog::open(&path),
            Spy::default(),
            dir.path().join("session.jsonl"),
        );

        let reply = bridge.handle(approve("cmd-1")).await;

        match &reply[0] {
            DeviceToPeer::Error { code, .. } => assert_eq!(code, "indeterminate"),
            other => panic!("expected an indeterminate error, got {other:?}"),
        }
        assert_eq!(bridge.actions.approvals.lock().unwrap().len(), 0);
    }

    /// Losing the race to the local user is a success with news attached, not an
    /// error. The peer gets an Ack plus `ApprovalResolved` naming the winner, so
    /// its gate closes showing the answer that actually took effect — §5.3. An
    /// error here would make a working system look broken and invite a retry of
    /// something already decided.
    #[tokio::test]
    async fn losing_the_race_tells_the_peer_who_won() {
        let dir = tempfile::tempdir().unwrap();
        let log = CommandLog::open(dir.path().join("commands.jsonl"));
        let spy = Spy {
            already_resolved: Some(approval::Decision::denied_by(approval::Actor::Local)),
            ..Default::default()
        };
        let mut bridge = Bridge::new(
            "session-1",
            "iPhone",
            permissive(),
            log,
            spy,
            dir.path().join("session.jsonl"),
        );

        let reply = bridge.handle(approve("cmd-1")).await;

        assert!(matches!(reply[0], DeviceToPeer::Ack { .. }), "{reply:?}");
        match &reply[1] {
            DeviceToPeer::ApprovalResolved {
                tool_use_id,
                decision,
                actor,
            } => {
                assert_eq!(tool_use_id, "tool-1");
                assert!(!decision.approved, "the local rejection is what won");
                assert_eq!(actor, &wire::Actor::Local);
            }
            other => panic!("expected ApprovalResolved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn distinct_commands_each_run() {
        let (_d, mut bridge) = bridge(permissive());

        bridge.handle(approve("cmd-1")).await;
        bridge.handle(approve("cmd-2")).await;

        assert_eq!(bridge.actions.approvals.lock().unwrap().len(), 2);
    }

    // --- policy ------------------------------------------------------------

    /// The default policy grants nothing, so a bridge built before there is any
    /// way to widen it refuses everything. The enforcement point exists before
    /// the capability does.
    #[tokio::test]
    async fn the_default_policy_refuses_every_command() {
        let (_d, mut bridge) = bridge(Policy::default());

        for message in [
            approve("cmd-1"),
            PeerToDevice::SendMessage {
                cmd_id: "cmd-2".into(),
                session_id: "session-1".into(),
                text: "hello".into(),
                attachments: vec![],
            },
            PeerToDevice::Interrupt {
                cmd_id: "cmd-3".into(),
                session_id: "session-1".into(),
            },
            PeerToDevice::Steer {
                cmd_id: "cmd-4".into(),
                session_id: "session-1".into(),
                text: "stop".into(),
            },
        ] {
            let reply = bridge.handle(message).await;
            assert!(
                matches!(reply[0], DeviceToPeer::PolicyDenied { .. }),
                "expected a denial, got {reply:?}"
            );
        }

        assert_eq!(bridge.actions.approvals.lock().unwrap().len(), 0);
        assert!(bridge.actions.messages.lock().unwrap().is_empty());
        assert_eq!(*bridge.actions.interrupts.lock().unwrap(), 0);
    }

    /// Every denial says which rule. A peer that cannot tell "you may not" from
    /// "something broke" retries forever, and a UI that cannot name the rule
    /// leaves the user nothing to change.
    #[tokio::test]
    async fn a_denial_names_the_rule_and_explains_itself() {
        let (_d, mut bridge) = bridge(Policy::default());

        let reply = bridge
            .handle(PeerToDevice::Interrupt {
                cmd_id: "cmd-1".into(),
                session_id: "session-1".into(),
            })
            .await;

        match &reply[0] {
            DeviceToPeer::PolicyDenied {
                rule, human_reason, ..
            } => {
                assert_eq!(rule, "allow.interrupt");
                assert!(!human_reason.is_empty());
            }
            other => panic!("expected PolicyDenied, got {other:?}"),
        }
    }

    /// A denied command must stay retryable: policy is edited locally and can
    /// widen, and a denial that consumed the cmd_id would make the retry look
    /// like a duplicate forever.
    #[tokio::test]
    async fn a_denied_command_can_be_issued_again_once_policy_allows_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("commands.jsonl");

        let mut strict = Bridge::new(
            "session-1",
            "iPhone",
            Policy::default(),
            CommandLog::open(&path),
            Spy::default(),
            dir.path().join("session.jsonl"),
        );
        assert!(matches!(
            strict.handle(approve("cmd-1")).await[0],
            DeviceToPeer::PolicyDenied { .. }
        ));

        let mut relaxed = Bridge::new(
            "session-1",
            "iPhone",
            permissive(),
            CommandLog::open(&path),
            Spy::default(),
            dir.path().join("session.jsonl"),
        );
        assert!(matches!(
            relaxed.handle(approve("cmd-1")).await[0],
            DeviceToPeer::Ack { .. }
        ));
        assert_eq!(relaxed.actions.approvals.lock().unwrap().len(), 1);
    }

    /// A rejection is not an approval and is not gated by the approve rules: a
    /// peer may always say no. Denying that would leave a gate open with nobody
    /// able to close it.
    #[tokio::test]
    async fn a_peer_may_always_reject_even_under_the_default_policy() {
        let (_d, mut bridge) = bridge(Policy::default());

        let reply = bridge
            .handle(PeerToDevice::ResolveApproval {
                cmd_id: "cmd-1".into(),
                session_id: "session-1".into(),
                tool_use_id: "tool-1".into(),
                decision: wire::Decision { approved: false },
                reason: Some("no".into()),
            })
            .await;

        assert!(matches!(reply[0], DeviceToPeer::Ack { .. }), "{reply:?}");
        let approvals = bridge.actions.approvals.lock().unwrap();
        assert_eq!(approvals.len(), 1);
        assert!(!approvals[0].1, "it was recorded as a rejection");
    }

    // --- event translation --------------------------------------------------

    fn text(s: &str) -> Delivery {
        Delivery::Event(Box::new(AgentEvent::TextStep { text: s.into() }))
    }

    #[test]
    fn events_are_numbered_contiguously_from_one() {
        let (_d, mut bridge) = bridge(permissive());

        let seqs: Vec<u64> = ["a", "b", "c"]
            .iter()
            .filter_map(|s| match bridge.translate(text(s)) {
                Some(DeviceToPeer::Event { seq, .. }) => Some(seq),
                _ => None,
            })
            .collect();

        assert_eq!(seqs, vec![1, 2, 3]);
    }

    /// A gap consumes the numbers it lost, so the stream after it is still
    /// contiguous with what the peer will see next. Without that, every gap
    /// would look like a second gap forever.
    #[test]
    fn a_gap_reports_the_span_it_lost_and_numbering_continues_after_it() {
        let (_d, mut bridge) = bridge(permissive());

        bridge.translate(text("a"));

        match bridge.translate(Delivery::Gap { missed: 5 }) {
            Some(DeviceToPeer::Gap {
                from_seq, to_seq, ..
            }) => {
                assert_eq!((from_seq, to_seq), (2, 6));
            }
            other => panic!("expected a Gap, got {other:?}"),
        }

        match bridge.translate(text("after")) {
            Some(DeviceToPeer::Event { seq, .. }) => assert_eq!(seq, 7),
            other => panic!("expected an Event, got {other:?}"),
        }
    }

    #[test]
    fn a_closed_bus_produces_no_frame() {
        let (_d, mut bridge) = bridge(permissive());
        assert!(bridge.translate(Delivery::Closed).is_none());
    }

    /// The agent event rides as its own serialisation, so the wire does not grow
    /// a variant every time the agent gains one.
    #[test]
    fn an_event_carries_the_agents_own_serialisation() {
        let (_d, mut bridge) = bridge(permissive());

        match bridge.translate(text("hello")) {
            Some(DeviceToPeer::Event { event, .. }) => {
                assert_eq!(event["event"], "TextStep");
                assert_eq!(event["data"]["text"], "hello");
            }
            other => panic!("expected an Event, got {other:?}"),
        }
    }

    // --- subscribe and replay ----------------------------------------------

    fn snapshots(replies: &[DeviceToPeer]) -> Vec<(usize, u64)> {
        replies
            .iter()
            .map(|reply| match reply {
                DeviceToPeer::Snapshot { records, seq, .. } => (records.len(), *seq),
                other => panic!("expected a Snapshot, got {other:?}"),
            })
            .collect()
    }

    /// The resume path, and the reason the relay needs no storage: a peer asks
    /// for what it missed and the device reads it back off disk.
    #[tokio::test]
    async fn subscribing_from_zero_replays_the_whole_transcript() {
        let (_d, mut bridge) = bridge_with_transcript(
            permissive(),
            &[
                &user_record("one"),
                &user_record("two"),
                &user_record("three"),
            ],
        );

        let replies = bridge
            .handle(PeerToDevice::Subscribe {
                session_id: "session-1".into(),
                from_seq: 0,
            })
            .await;

        assert_eq!(snapshots(&replies), vec![(3, 3)]);
    }

    #[tokio::test]
    async fn subscribing_from_a_seq_replays_only_what_follows() {
        let (_d, mut bridge) = bridge_with_transcript(
            permissive(),
            &[
                &user_record("one"),
                &user_record("two"),
                &user_record("three"),
            ],
        );

        let replies = bridge
            .handle(PeerToDevice::Subscribe {
                session_id: "session-1".into(),
                from_seq: 3,
            })
            .await;

        assert_eq!(snapshots(&replies), vec![(1, 3)]);
    }

    /// A peer that has everything must be told so. Sending nothing would leave it
    /// waiting for a reply that never comes, which on a phone looks like a hang.
    #[tokio::test]
    async fn a_peer_that_is_up_to_date_still_gets_an_empty_snapshot() {
        let (_d, mut bridge) = bridge_with_transcript(permissive(), &[&user_record("one")]);

        let replies = bridge
            .handle(PeerToDevice::Subscribe {
                session_id: "session-1".into(),
                from_seq: 2,
            })
            .await;

        assert_eq!(snapshots(&replies), vec![(0, 1)]);
    }

    /// One session can be far larger than one frame, so a replay is chunked. Each
    /// chunk carries the highest seq it holds, so a peer cut off partway resumes
    /// from the last chunk it received instead of starting again.
    #[tokio::test]
    async fn a_large_transcript_is_split_across_frames() {
        let big = user_record(&"x".repeat(20 * 1024));
        let lines: Vec<&str> = (0..8).map(|_| big.as_str()).collect();
        let (_d, mut bridge) = bridge_with_transcript(permissive(), &lines);

        let replies = bridge
            .handle(PeerToDevice::Subscribe {
                session_id: "session-1".into(),
                from_seq: 0,
            })
            .await;

        assert!(replies.len() > 1, "expected chunking, got one frame");

        // Every record arrives exactly once, and the sequence numbers climb.
        let chunks = snapshots(&replies);
        assert_eq!(chunks.iter().map(|(n, _)| n).sum::<usize>(), 8);
        let seqs: Vec<u64> = chunks.iter().map(|(_, seq)| *seq).collect();
        assert!(seqs.windows(2).all(|w| w[0] <= w[1]), "{seqs:?}");
        assert_eq!(*seqs.last().unwrap(), 8);

        // And no chunk is big enough to be refused by the wire.
        for reply in &replies {
            let encoded = rmp_serde::to_vec_named(reply).unwrap();
            assert!(
                encoded.len() < claudinio_protocol::wire::MAX_FRAME,
                "a chunk of {} bytes would not fit a frame",
                encoded.len()
            );
        }
    }

    /// A read has no side effect, so replaying one is harmless and must not be
    /// deduplicated — otherwise a reconnecting peer could never re-subscribe.
    #[tokio::test]
    async fn subscribing_twice_replays_twice() {
        let (_d, mut bridge) = bridge_with_transcript(permissive(), &[&user_record("one")]);
        let subscribe = || PeerToDevice::Subscribe {
            session_id: "session-1".into(),
            from_seq: 0,
        };

        let first = bridge.handle(subscribe()).await;
        let second = bridge.handle(subscribe()).await;

        assert_eq!(snapshots(&first), snapshots(&second));
    }

    /// The peer is told what it may do, so its UI can grey out the rest instead of
    /// offering actions that will be refused.
    #[tokio::test]
    async fn asking_for_the_policy_returns_it() {
        let (_d, mut bridge) = bridge(Policy::default());

        let replies = bridge.handle(PeerToDevice::GetPolicy).await;

        match &replies[0] {
            DeviceToPeer::Policy(policy) => {
                assert!(!policy.send_message, "the default grants nothing");
                assert_eq!(policy.approve_bash, BashApproval::Never);
            }
            other => panic!("expected Policy, got {other:?}"),
        }
    }

    // --- unimplemented surface ---------------------------------------------

    /// A query phase 2 does not answer must say so rather than being dropped, or
    /// a peer waits forever for a reply that is never coming.
    #[tokio::test]
    async fn an_unimplemented_query_is_answered_with_an_error() {
        let (_d, mut bridge) = bridge(permissive());

        let reply = bridge.handle(PeerToDevice::ListWorkspaces).await;

        match &reply[0] {
            DeviceToPeer::Error { message, .. } => {
                assert!(message.contains("list_workspaces"), "{message}")
            }
            other => panic!("expected an Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unimplemented_command_reports_which_one() {
        let (_d, mut bridge) = bridge(permissive());

        let reply = bridge
            .handle(PeerToDevice::SetMode {
                cmd_id: "cmd-1".into(),
                session_id: "session-1".into(),
                mode: "builder".into(),
            })
            .await;

        match &reply[0] {
            DeviceToPeer::Error { message, .. } => {
                assert!(message.contains("set_mode"), "{message}")
            }
            other => panic!("expected an Error, got {other:?}"),
        }
    }
}
