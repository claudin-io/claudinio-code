//! `AgentEvent`: what a run streams to the frontend over a Tauri `Channel`
//! while it is in flight, and the view structs those events carry. The session
//! file (`persist::SessionRecord`) is the record; these are the live view.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum AgentEvent {
    #[serde(rename = "TextStep")]
    TextStep { text: String },
    /// Live, accumulated snapshot of the assistant text currently streaming.
    /// Superseded by the next `TextStep`/`Done` for the same block; never persisted.
    #[serde(rename = "TextDelta")]
    TextDelta { text: String },
    /// The model's turn ended on this text with no tool call: an answer.
    ///
    /// Usually `Done` follows and carries the same text. But the run can go on
    /// past an answer — the judge nudges, the gate comes back red, a Stop hook
    /// objects, the user steers — and the next round's `TextDelta` then
    /// overwrote it on screen, so the answer the user had just read vanished
    /// (it was never a `TextStep`: those are text *alongside* tool calls). The
    /// client keeps this one, and when more model activity follows it closes
    /// the message there instead of folding everything into the last answer.
    #[serde(rename = "FinalText")]
    FinalText { text: String },
    #[serde(rename = "Thinking")]
    Thinking(String),
    #[serde(rename = "ToolCall")]
    ToolCall {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "toolId")]
        tool_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        args: Value,
        permission: String,
        #[serde(rename = "editProposal")]
        edit_proposal: Option<EditProposalData>,
    },
    #[serde(rename = "ToolResult")]
    ToolResult {
        #[serde(rename = "toolId")]
        tool_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        output: String,
        error: Option<String>,
    },
    /// Images a tool produced (browser screenshots, images returned by an MCP
    /// server), already compressed and base64-encoded.
    ///
    /// Deliberately a follow-up event rather than a field on `ToolResult`:
    /// `ToolResult` is constructed in ~30 places that have no images to give,
    /// and the UI already keys tool rows by `toolId`, so attaching on arrival
    /// costs nothing. Emitted immediately after the matching `ToolResult`.
    #[serde(rename = "ToolResultImages")]
    ToolResultImages {
        #[serde(rename = "toolId")]
        tool_id: String,
        images: Vec<crate::imageutil::ImageAttachment>,
    },
    #[serde(rename = "AskUser")]
    AskUser {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "toolId")]
        tool_id: String,
        questions: Value,
    },
    #[serde(rename = "Done")]
    Done {
        #[serde(rename = "stopReason")]
        stop_reason: String,
        #[serde(rename = "textOutput")]
        text_output: String,
        #[serde(rename = "inputTokens")]
        input_tokens: u32,
        #[serde(rename = "outputTokens")]
        output_tokens: u32,
    },
    #[serde(rename = "SteeringInjected")]
    SteeringInjected {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<crate::agent::persist::AttachmentMeta>>,
    },
    #[serde(rename = "Error")]
    Error(String),
    /// Transient provider failure being retried with backoff — lets the UI
    /// show "reconnecting" instead of looking dead during waits of up to 5min
    /// (claudin.io failover: one server dies, the backup takes ~2min to pick
    /// up). Never persisted.
    #[serde(rename = "Retrying")]
    Retrying {
        attempt: u32,
        #[serde(rename = "maxAttempts")]
        max_attempts: u32,
        #[serde(rename = "delayMs")]
        delay_ms: u64,
        error: String,
    },
    #[serde(rename = "SubagentStarted")]
    SubagentStarted {
        #[serde(rename = "subagentId")]
        subagent_id: String,
        #[serde(rename = "parentToolId")]
        parent_tool_id: String,
        name: String,
        goal: String,
        mode: String,
    },
    #[serde(rename = "SubagentDone")]
    SubagentDone {
        #[serde(rename = "subagentId")]
        subagent_id: String,
        status: String,
        rounds: u32,
        #[serde(rename = "inputTokens")]
        input_tokens: u32,
        #[serde(rename = "outputTokens")]
        output_tokens: u32,
        #[serde(rename = "report")]
        report: String,
        #[serde(rename = "cost")]
        cost: f64,
    },
    #[serde(rename = "Subagent")]
    Subagent {
        #[serde(rename = "subagentId")]
        subagent_id: String,
        event: Box<AgentEvent>,
    },
    #[serde(rename = "ModeChanged")]
    ModeChanged {
        mode: String,
        origin: String,
        reason: Option<String>,
    },
    /// The run tried to finish with golden tasks still pending: a new golden
    /// cycle starts in `mode`. `pending` lists the unfinished golden task ids.
    #[serde(rename = "GoldenLoop")]
    GoldenLoop {
        cycle: u32,
        #[serde(rename = "maxCycles")]
        max_cycles: u32,
        pending: Vec<String>,
        mode: String,
    },
    /// A quality-harness run finished: the project's own tests / coverage ran
    /// and were scored against the gate. Emitted whether the harness or the
    /// agent triggered it, so the user always sees the verification, not just
    /// the model's claim about it.
    #[serde(rename = "QualityVerdict")]
    QualityVerdict {
        pass: bool,
        summary: String,
        /// One entry per layer/stack: "tests", "pass", "3 passed, 0 failed".
        layers: Vec<QualityLayerView>,
        /// "tool" when the agent asked, "harness" when the loop enforced it.
        trigger: String,
    },
    /// The session was linked to a new successor via handoff. The old session
    /// ends here; the UI stitches the successor's events into the same thread.
    #[serde(rename = "SessionLinked")]
    SessionLinked {
        #[serde(rename = "prevSessionId")]
        prev_session_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        reason: String,
        mode: String,
        #[serde(rename = "firstMessage")]
        first_message: String,
    },
    #[serde(rename = "SessionStats")]
    SessionStats {
        #[serde(rename = "inputTokens")]
        input_tokens: u32,
        #[serde(rename = "outputTokens")]
        output_tokens: u32,
        #[serde(rename = "cumulativeCost")]
        cumulative_cost: Option<f64>,
        #[serde(rename = "costInput")]
        cost_input: Option<f64>,
        #[serde(rename = "costOutput")]
        cost_output: Option<f64>,
        #[serde(rename = "costCacheRead")]
        cost_cache_read: Option<f64>,
        #[serde(rename = "contextTokens")]
        context_tokens: u64,
        #[serde(rename = "maxContextTokens")]
        max_context_tokens: u64,
        #[serde(rename = "compactThreshold")]
        compact_threshold: u64,
    },
    /// A lifecycle hook started. Carries `statusMessage` because that is the
    /// spinner label the config author wrote, and a finished-only event would
    /// throw it away at exactly the moment it is useful.
    #[serde(rename = "HookStarted")]
    HookStarted {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "hookId")]
        hook_id: String,
        event: String,
        command: String,
        source: String,
        #[serde(rename = "statusMessage")]
        status_message: Option<String>,
        /// The tool call a `PreToolUse`/`PostToolUse` hook fired around, so the
        /// timeline can put it on that call's line instead of on one of its
        /// own. `HookFinished` does not repeat it: it updates the same row.
        #[serde(rename = "toolId", default, skip_serializing_if = "Option::is_none")]
        tool_id: Option<String>,
    },
    #[serde(rename = "HookFinished")]
    HookFinished {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "hookId")]
        hook_id: String,
        event: String,
        status: String,
        #[serde(rename = "exitCode")]
        exit_code: Option<i32>,
        #[serde(rename = "durationMs")]
        duration_ms: u64,
        output: String,
        error: Option<String>,
        decision: Option<String>,
        #[serde(rename = "systemMessage")]
        system_message: Option<String>,
    },
    /// This workspace declares hooks that have never been approved. Nothing ran.
    #[serde(rename = "HooksAwaitingApproval")]
    HooksAwaitingApproval {
        workspace: String,
        hash: String,
        count: usize,
        commands: Vec<String>,
    },
}

/// One row of the quality panel: which layer, on which stack, and how it went.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityLayerView {
    pub layer: String,
    pub stack: String,
    pub status: String,
    pub summary: String,
}

impl QualityLayerView {
    pub fn from_report(report: &crate::quality::QualityReport) -> Vec<Self> {
        report
            .layers
            .iter()
            .map(|l| QualityLayerView {
                layer: l.layer.as_str().to_string(),
                stack: l.stack.clone(),
                status: l.status.as_str().to_string(),
                summary: l.summary.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditProposalData {
    pub path: String,
    pub old_string: String,
    pub new_string: String,
    pub unified_diff: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session::{COMPACT_THRESHOLD, MAX_CONTEXT_TOKENS};

    use serde_json::json;

    #[test]
    fn agent_event_round_trip_text_step() {
        let ev = AgentEvent::TextStep {
            text: "hello".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::TextStep { text } if text == "hello"));
    }

    #[test]
    fn agent_event_round_trip_text_delta() {
        let ev = AgentEvent::TextDelta {
            text: "partial".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["event"], "TextDelta");
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::TextDelta { text } if text == "partial"));
    }

    #[test]
    fn agent_event_round_trip_final_text() {
        // The client matches on the tag to tell an answer from a snapshot.
        let ev = AgentEvent::FinalText {
            text: "answer".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["event"], "FinalText");
        assert_eq!(json["data"]["text"], "answer");
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::FinalText { text } if text == "answer"));
    }

    #[test]
    fn agent_event_round_trip_thinking() {
        let ev = AgentEvent::Thinking("thinking text".into());
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::Thinking(t) if t == "thinking text"));
    }

    #[test]
    fn agent_event_round_trip_tool_call() {
        let ev = AgentEvent::ToolCall {
            session_id: "s1".into(),
            tool_id: "t1".into(),
            tool_name: "read_file".into(),
            args: json!({"path": "/foo"}),
            permission: "auto".into(),
            edit_proposal: None,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::ToolCall {
                session_id,
                tool_id,
                tool_name,
                ..
            } => {
                assert_eq!(session_id, "s1");
                assert_eq!(tool_id, "t1");
                assert_eq!(tool_name, "read_file");
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn agent_event_round_trip_tool_result() {
        let ev = AgentEvent::ToolResult {
            tool_id: "t1".into(),
            tool_name: "read_file".into(),
            output: "content".into(),
            error: None,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::ToolResult { tool_id, .. } if tool_id == "t1"));
    }

    #[test]
    fn agent_event_round_trip_done() {
        let ev = AgentEvent::Done {
            stop_reason: "end_turn".into(),
            text_output: "done".into(),
            input_tokens: 10,
            output_tokens: 20,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::Done {
                stop_reason,
                input_tokens,
                output_tokens,
                ..
            } => {
                assert_eq!(stop_reason, "end_turn");
                assert_eq!(input_tokens, 10);
                assert_eq!(output_tokens, 20);
            }
            _ => panic!("expected Done"),
        }
    }

    #[test]
    fn agent_event_round_trip_subagent_started() {
        let ev = AgentEvent::SubagentStarted {
            subagent_id: "sa1".into(),
            parent_tool_id: "pt1".into(),
            name: "explorer".into(),
            goal: "find stuff".into(),
            mode: "explore".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::SubagentStarted {
                subagent_id, name, ..
            } => {
                assert_eq!(subagent_id, "sa1");
                assert_eq!(name, "explorer");
            }
            _ => panic!("expected SubagentStarted"),
        }
    }

    #[test]
    fn agent_event_round_trip_subagent_done() {
        let ev = AgentEvent::SubagentDone {
            subagent_id: "sa1".into(),
            status: "completed".into(),
            rounds: 5,
            input_tokens: 100,
            output_tokens: 50,
            report: String::new(),
            cost: 0.0,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::SubagentDone {
                subagent_id,
                status,
                rounds,
                ..
            } => {
                assert_eq!(subagent_id, "sa1");
                assert_eq!(status, "completed");
                assert_eq!(rounds, 5);
            }
            _ => panic!("expected SubagentDone"),
        }
    }

    #[test]
    fn agent_event_round_trip_subagent_wrapped() {
        let inner = AgentEvent::Thinking("inner thought".into());
        let ev = AgentEvent::Subagent {
            subagent_id: "sa1".into(),
            event: Box::new(inner),
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::Subagent { subagent_id, event } => {
                assert_eq!(subagent_id, "sa1");
                assert!(matches!(*event, AgentEvent::Thinking(t) if t == "inner thought"));
            }
            _ => panic!("expected Subagent"),
        }
    }

    #[test]
    fn agent_event_round_trip_error() {
        let ev = AgentEvent::Error("something broke".into());
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::Error(e) if e == "something broke"));
    }

    #[test]
    fn agent_event_round_trip_steering_injected() {
        let ev = AgentEvent::SteeringInjected {
            text: "steer".into(),
            attachments: None,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::SteeringInjected { text, .. } if text == "steer"));

        // Round-trip with attachments
        let ev2 = AgentEvent::SteeringInjected {
            text: "steer2".into(),
            attachments: Some(vec![crate::agent::persist::AttachmentMeta {
                name: "photo.png".into(),
                media_type: "image/png".into(),
                size: 1024,
            }]),
        };
        let json2 = serde_json::to_value(&ev2).unwrap();
        let back2: AgentEvent = serde_json::from_value(json2).unwrap();
        match back2 {
            AgentEvent::SteeringInjected { text, attachments } => {
                assert_eq!(text, "steer2");
                let atts = attachments.unwrap();
                assert_eq!(atts.len(), 1);
                assert_eq!(atts[0].name, "photo.png");
                assert_eq!(atts[0].media_type, "image/png");
                assert_eq!(atts[0].size, 1024);
            }
            _ => panic!("expected SteeringInjected"),
        }
    }

    #[test]
    fn agent_event_round_trip_ask_user() {
        let ev = AgentEvent::AskUser {
            session_id: "s1".into(),
            tool_id: "t1".into(),
            questions: json!([{"question": "q?", "options": ["a", "b"]}]),
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::AskUser {
                session_id,
                tool_id,
                ..
            } => {
                assert_eq!(session_id, "s1");
                assert_eq!(tool_id, "t1");
            }
            _ => panic!("expected AskUser"),
        }
    }

    #[test]
    fn agent_event_round_trip_session_stats() {
        let ev = AgentEvent::SessionStats {
            input_tokens: 500,
            output_tokens: 200,
            cumulative_cost: Some(0.003),
            cost_input: Some(0.001),
            cost_output: Some(0.0015),
            cost_cache_read: Some(0.0005),
            context_tokens: 42_000,
            max_context_tokens: MAX_CONTEXT_TOKENS,
            compact_threshold: COMPACT_THRESHOLD,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["data"]["contextTokens"], 42_000);
        assert_eq!(json["data"]["maxContextTokens"], 200_000);
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::SessionStats {
                input_tokens,
                output_tokens,
                cumulative_cost,
                context_tokens,
                ..
            } => {
                assert_eq!(input_tokens, 500);
                assert_eq!(output_tokens, 200);
                assert_eq!(cumulative_cost, Some(0.003));
                assert_eq!(context_tokens, 42_000);
            }
            _ => panic!("expected SessionStats"),
        }
    }

    #[test]
    fn session_stats_without_cost() {
        let ev = AgentEvent::SessionStats {
            input_tokens: 100,
            output_tokens: 50,
            cumulative_cost: None,
            cost_input: None,
            cost_output: None,
            cost_cache_read: None,
            context_tokens: 0,
            max_context_tokens: MAX_CONTEXT_TOKENS,
            compact_threshold: COMPACT_THRESHOLD,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::SessionStats {
                cumulative_cost, ..
            } => {
                assert_eq!(cumulative_cost, None);
            }
            _ => panic!("expected SessionStats"),
        }
    }
}
