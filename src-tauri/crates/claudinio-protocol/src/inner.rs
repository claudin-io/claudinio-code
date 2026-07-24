//! The end-to-end messages: what travels inside the outer frame's payload,
//! encrypted, and which the relay has no types for.
//!
//! # On duplication
//!
//! `Actor` and `Decision` here look like the ones in the app's
//! `agent/approval.rs`, and that is deliberate rather than an oversight. The
//! agent owns the *concept* of who answered a gate; this crate owns its *wire
//! form*. Making `agent/` depend on the remote protocol to express a local
//! approval would be the agent knowing remote access exists, which is the one
//! thing the architecture test forbids.
//!
//! The conversion between the two lives in `remote/`, which is allowed to know
//! both, and is tested there. That is where drift would show, and it is a much
//! smaller surface to watch than a shared type reaching into the agent loop.

use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

/// Client-chosen id for a command, used for deduplication.
///
/// This is the single most important field in the protocol. A reconnect can
/// replay a command, and a replayed `ResolveApproval` that ran twice is a
/// duplicated `rm`.
pub type CmdId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PeerToDevice {
    /// Only workspaces on the policy allowlist come back.
    ListWorkspaces,
    ListSessions {
        workspace: String,
    },
    /// `from_seq: 0` means a full replay.
    Subscribe {
        session_id: String,
        from_seq: u64,
    },
    Unsubscribe {
        session_id: String,
    },
    SendMessage {
        cmd_id: CmdId,
        session_id: String,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<Attachment>,
    },
    /// Mid-reasoning guidance, matching the existing queue-and-inject behaviour.
    Steer {
        cmd_id: CmdId,
        session_id: String,
        text: String,
    },
    Interrupt {
        cmd_id: CmdId,
        session_id: String,
    },
    ResolveApproval {
        cmd_id: CmdId,
        session_id: String,
        tool_use_id: String,
        decision: Decision,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    SetMode {
        cmd_id: CmdId,
        session_id: String,
        mode: String,
    },
    /// Read-only. There is deliberately no `SetPolicy` — a peer that could widen
    /// its own policy would grant itself everything the moment it was
    /// compromised.
    GetPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
pub struct Attachment {
    pub name: String,
    pub media_type: String,
    /// Base64. Size is bounded by `MAX_FRAME` on the way out.
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeviceToPeer {
    /// A replay chunk read from the JSONL. `seq` is the highest included.
    Snapshot {
        session_id: String,
        records: Vec<serde_json::Value>,
        seq: u64,
    },
    Event {
        session_id: String,
        seq: u64,
        event: serde_json::Value,
    },
    ApprovalRequest {
        session_id: String,
        tool_use_id: String,
        tool: String,
        args: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
        /// Unix millis. An approval nobody answers must not hang forever.
        expires_at: u64,
    },
    /// Sent to the loser of a race so it closes its gate showing what won.
    ApprovalResolved {
        tool_use_id: String,
        decision: Decision,
        actor: Actor,
    },
    /// The peer fell behind. It must re-`Subscribe` from `from_seq`.
    Gap {
        session_id: String,
        from_seq: u64,
        to_seq: u64,
    },
    /// Always says which rule, never fails silently — a peer that cannot tell
    /// "denied" from "broken" will retry forever.
    PolicyDenied {
        cmd_id: CmdId,
        rule: String,
        human_reason: String,
    },
    /// The effective policy, so the UI can grey out what it cannot do rather
    /// than offering it and failing.
    Policy(Policy),
    Ack {
        cmd_id: CmdId,
    },
    Error {
        cmd_id: CmdId,
        code: String,
        message: String,
    },
}

/// Who answered an approval gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[serde(tag = "kind", content = "label", rename_all = "snake_case")]
pub enum Actor {
    Local,
    Peer(String),
    /// Nobody answered in time. Always a denial.
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
pub struct Decision {
    pub approved: bool,
}

/// What a peer is allowed to do, as the device sees it.
///
/// Sent to the peer for display only. The device enforces; the peer is told so
/// its UI can be honest about what is available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
pub struct Policy {
    pub send_message: bool,
    pub steer: bool,
    pub interrupt: bool,
    pub set_mode: bool,
    pub approve_edit: bool,
    pub approve_bash: BashApproval,
    pub read_attachment: bool,
    pub export_file: bool,
    /// Unix millis after which every grant lapses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum BashApproval {
    /// A remote peer may not approve shell at all.
    Never,
    /// Only commands on the read-only allowlist. The default.
    Allowlist,
    /// Any command. A deliberate, expiring grant — the highest-consequence
    /// setting in the system.
    Always,
}

impl Default for Policy {
    /// Deny by default, and two of these defaults carry the threat model.
    ///
    /// `read_attachment` is false because reads through the app's IPC surface
    /// are not workspace-scoped — attaching a file from `~/Downloads` is the
    /// point of the feature locally, where a native picker means the user chose
    /// it. Remotely there is no picker, so the same capability is an arbitrary
    /// read of the home directory.
    ///
    /// `export_file` is false because its whole security property is that the
    /// destination comes from a native dialog and never crosses IPC as an
    /// argument. There is no remote equivalent of standing at the machine.
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
            expires_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode and decode both ways, over a representative message from each
    /// direction. MessagePack is the transport, so JSON alone would not prove it.
    #[test]
    fn messages_round_trip_through_messagepack() {
        let from_peer = PeerToDevice::ResolveApproval {
            cmd_id: "cmd-1".into(),
            session_id: "s-1".into(),
            tool_use_id: "tool-1".into(),
            decision: Decision { approved: true },
            reason: Some("looks right".into()),
        };
        let bytes = rmp_serde::to_vec_named(&from_peer).unwrap();
        assert_eq!(
            rmp_serde::from_slice::<PeerToDevice>(&bytes).unwrap(),
            from_peer
        );

        let from_device = DeviceToPeer::ApprovalResolved {
            tool_use_id: "tool-1".into(),
            decision: Decision { approved: false },
            actor: Actor::Peer("iPhone".into()),
        };
        let bytes = rmp_serde::to_vec_named(&from_device).unwrap();
        assert_eq!(
            rmp_serde::from_slice::<DeviceToPeer>(&bytes).unwrap(),
            from_device
        );
    }

    /// The tag strings are the wire contract with the browser. Renaming a
    /// variant in Rust silently breaks a peer that has not been rebuilt, so the
    /// names are pinned here rather than left to whatever serde derives.
    #[test]
    fn wire_tags_are_pinned() {
        let cases: Vec<(PeerToDevice, &str)> = vec![
            (PeerToDevice::ListWorkspaces, "list_workspaces"),
            (PeerToDevice::GetPolicy, "get_policy"),
            (
                PeerToDevice::Subscribe {
                    session_id: "s".into(),
                    from_seq: 0,
                },
                "subscribe",
            ),
            (
                PeerToDevice::Interrupt {
                    cmd_id: "c".into(),
                    session_id: "s".into(),
                },
                "interrupt",
            ),
        ];
        for (message, tag) in cases {
            let json = serde_json::to_value(&message).unwrap();
            assert_eq!(json["kind"], tag, "wire tag drifted for {message:?}");
        }

        let gap = DeviceToPeer::Gap {
            session_id: "s".into(),
            from_seq: 1,
            to_seq: 9,
        };
        assert_eq!(serde_json::to_value(&gap).unwrap()["kind"], "gap");
    }

    /// The whole policy model is deny-by-default. A future field that defaults
    /// to permissive would be a silent capability grant, so the default is
    /// asserted field by field rather than by eye.
    #[test]
    fn the_default_policy_grants_nothing() {
        let policy = Policy::default();

        assert!(!policy.send_message);
        assert!(!policy.steer);
        assert!(!policy.interrupt);
        assert!(!policy.set_mode);
        assert!(!policy.approve_edit);
        assert_eq!(policy.approve_bash, BashApproval::Never);
        assert!(!policy.read_attachment);
        assert!(!policy.export_file);

        // Serialising and reading back must not widen anything either.
        let json = serde_json::to_string(&policy).unwrap();
        assert_eq!(serde_json::from_str::<Policy>(&json).unwrap(), policy);
    }

    /// `export_file` has no safe remote form at all — its security property is a
    /// native dialog the peer cannot have. If a future edit ever makes it
    /// default true, this fails.
    #[test]
    fn export_file_is_never_granted_by_default() {
        assert!(!Policy::default().export_file);
    }

    #[test]
    fn an_actor_round_trips_including_its_label() {
        for actor in [
            Actor::Local,
            Actor::Peer("Firefox on Windows laptop".into()),
            Actor::Expired,
        ] {
            let bytes = rmp_serde::to_vec_named(&actor).unwrap();
            assert_eq!(rmp_serde::from_slice::<Actor>(&bytes).unwrap(), actor);
        }
    }

    /// A message with an unknown tag must be a clean error, because the device
    /// answers it with `Error { code }` rather than dropping it.
    #[test]
    fn an_unknown_message_kind_is_an_error_not_a_panic() {
        let json = r#"{"kind":"from_a_newer_peer","what":"?"}"#;
        assert!(serde_json::from_str::<PeerToDevice>(json).is_err());
    }
}
