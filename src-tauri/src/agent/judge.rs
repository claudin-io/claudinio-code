//! The verdicts a run asks for about itself: is a terminal turn really finished
//! (`judge_terminal_turn_metered`, with Jev answering first when it can — `judge_with_jev`),
//! and is the tool loop stuck (`loop_verdict`). Every path fails safe: a judge
//! outage ends the turn or leaves the loop alone, it never wedges the run.

use crate::agent::provider::AgentConfig;
use serde_json::Value;

/// How a terminal `end_turn` (no tool call) should be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnVerdict {
    /// A complete, self-contained reply — end the run normally.
    Done,
    /// The model announced/implied an immediate next step (ask the user, spawn
    /// subagents, read a file, edit code…) but ended without taking it. The run
    /// must not go idle here — nudge the model to actually act and loop again.
    Continue,
}

/// Map the completion-judge model's reply to a verdict. Kept separate from the
/// HTTP call so the parsing is deterministic and unit-testable, and — crucially —
/// language-agnostic: the judge is instructed to answer with a fixed sentinel
/// token, never natural-language prose, so no per-language keyword list is ever
/// needed here (new UI languages need no changes).
///
/// Fails safe toward `Done`: an unrecognizable reply ends the run rather than
/// risking a spurious extra loop.
fn parse_turn_verdict(reply: &str) -> TurnVerdict {
    let norm = reply.trim().to_ascii_uppercase();
    // Accept the token anywhere in the reply so a chatty model ("CONTINUE — it
    // said it would ask a question") is still handled correctly. CONTINUE wins
    // ties: if the judge is unsure enough to emit both, keep working.
    // Anything that is not an explicit CONTINUE is treated as done: an
    // unparseable judge reply must let the turn end, never wedge the loop.
    if norm.contains("CONTINUE") {
        TurnVerdict::Continue
    } else {
        TurnVerdict::Done
    }
}

/// Ask the model itself whether a terminal turn is genuinely finished or merely
/// announced a next step it never took (the failure that stalled session
/// 912bb460: "Primeiro, preciso confirmar algo sobre o tempo:" with no tool
/// call, and the twin case where it said it would spawn subagents and didn't).
///
/// Uses the LLM instead of hardcoded phrases so it works in any language the UI
/// ever adds. Fails safe toward `Done` on any error — a judge outage must never
/// wedge the loop or fabricate an infinite continuation.
///
/// Returns the verdict with what became of the judge's request: `None` when
/// it failed (nothing was billed, no round happened), `Some(usage)` when it
/// was answered — `usage` itself `None` when the response carried none. The
/// judge is a request of the run, and an answered one is recorded as a round
/// whether or not the provider reported its usage.
pub(super) async fn judge_terminal_turn_metered(
    config: &AgentConfig,
    model: &str,
    assistant_text: &str,
) -> (TurnVerdict, Option<Option<crate::agent::provider::Usage>>) {
    match crate::agent::provider::classify_turn_completion_metered(config, model, assistant_text)
        .await
    {
        Ok((reply, usage)) => (parse_turn_verdict(&reply), Some(usage)),
        Err(_) => (TurnVerdict::Done, None),
    }
}

/// P(yes) on either completion question from which Jev's answer is "keep
/// working". Measured live (2026-10-05): sixteen real-shaped finals split
/// 0.03–0.05 (finished) against 0.61–0.99 (unfinished or asking), with nothing
/// in between — see `jev_harness_tests::PROBE`.
const JEV_CONTINUE_AT: f64 = 0.7;
/// Both answers at or below this mean "finished". Anything between the two
/// thresholds is handed to the LLM judge instead of guessed.
const JEV_DONE_AT_MOST: f64 = 0.3;

/// The two questions Jev answers about a terminal turn. Together they are
/// the LLM judge's rubric (`COMPLETION_JUDGE_SYSTEM`), split in two because Jev
/// reads each question literally.
fn completion_questions() -> Value {
    use crate::agent::jev::noul;
    serde_json::json!({
        "unfinished": noul(
            "This is the final message an AI coding agent wrote before ending its turn \
             without calling any tool. Did the agent announce, promise or begin an immediate \
             next action of its own (reading a file, running a command, editing code, spawning \
             subagents, investigating further) that it did not actually perform, or is the \
             message cut off mid-thought?",
            "It announced or started a next step and stopped before doing it, or it is cut off",
            "It does not announce any pending action of its own; the work it describes is \
             finished or it only reports results",
        ),
        "asks_user": noul(
            "This is the final message an AI coding agent wrote before ending its turn. Does \
             the message ask the user a question or request a decision or information from \
             the user?",
            "It asks the user something or waits for the user's decision/answer",
            "It asks the user nothing",
        ),
    })
}

/// Map Jev's two probabilities to a verdict, or `None` when the answer is not
/// clear enough to act on (the LLM judge decides those).
fn jev_turn_verdict(unfinished: Option<f64>, asks_user: Option<f64>) -> Option<TurnVerdict> {
    let (u, a) = (unfinished?, asks_user?);
    if u.max(a) >= JEV_CONTINUE_AT {
        Some(TurnVerdict::Continue)
    } else if u <= JEV_DONE_AT_MOST && a <= JEV_DONE_AT_MOST {
        Some(TurnVerdict::Done)
    } else {
        None
    }
}

/// Ask Jev whether a terminal turn is finished. Returns the verdict (None =
/// no clear answer, or Jev unavailable) and what the call cost.
pub(super) async fn judge_with_jev(
    backend: &crate::agent::jev::JevBackend,
    assistant_text: &str,
) -> (Option<TurnVerdict>, f64) {
    let state = format!("Agent's final message of the turn:\n\n{assistant_text}");
    match crate::agent::jev::decide(backend, &state, &completion_questions()).await {
        Some(d) => (
            jev_turn_verdict(d.noul("unfinished"), d.noul("asks_user")),
            d.cost,
        ),
        None => (None, 0.0),
    }
}

/// Run the tool-loop watch after a round: ask Jev when the gate is suspicious
/// (and Jev has not just cleared it), then decide the intervention. Returns the
/// action and what Jev cost.
pub(crate) async fn loop_verdict(
    watch: &mut crate::agent::loop_watch::LoopWatch,
    backend: Option<&crate::agent::jev::JevBackend>,
) -> (crate::agent::loop_watch::LoopAction, f64) {
    use crate::agent::loop_watch::{Gate, LoopAction};
    let gate = watch.gate();
    if gate == Gate::Clear || !watch.should_ask() {
        return (LoopAction::None, 0.0);
    }
    let mut cost = 0.0;
    let mut p_stuck = None;
    if let Some(b) = backend
        && let Some(d) =
            crate::agent::jev::decide(b, &watch.state(), &crate::agent::loop_watch::question())
                .await
    {
        cost = d.cost;
        p_stuck = d.noul("stuck");
    }
    let action = watch.act(gate, p_stuck);
    if action == LoopAction::None && p_stuck.is_some() {
        watch.mark_cleared();
    }
    (action, cost)
}

/// Cheap structural backstop for the judge: a terminal message ending in a
/// question mark or a trailing colon (like "the core design question:") is
/// extremely unlikely to be a genuine final reply — the harness must not
/// silently end the run, it must nudge the model to actually call ask_user
/// (the only channel for asking the user) or finish properly. Language-
/// agnostic by construction (pure punctuation), so it complements the LLM
/// judge without any hardcoded phrase list.
pub(super) fn should_nudge_terminal(text: &str) -> bool {
    text.trim_end().ends_with(':')
        || text.trim_end().ends_with('?')
        || text.trim_end().ends_with('…')
        || text.trim_end().ends_with("...")
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- completion-judge verdict parsing (harness must never go idle after
    // the model merely announces a next step without taking it) ---
    //
    // The decision is delegated to the Brain model at runtime (language-agnostic,
    // no hardcoded phrase lists), so the only pure logic to unit-test here is how
    // the judge's one-word reply maps to a verdict. The live HTTP path is covered
    // by `judge_backend_mock` below against a stubbed /v1/messages server.

    #[test]
    fn verdict_continue_when_reply_says_continue() {
        assert_eq!(parse_turn_verdict("CONTINUE"), TurnVerdict::Continue);
        assert_eq!(parse_turn_verdict(" continue "), TurnVerdict::Continue);
        // Chatty models: token embedded in prose still counts.
        assert_eq!(
            parse_turn_verdict("CONTINUE — it said it would ask a question"),
            TurnVerdict::Continue
        );
    }

    #[test]
    fn verdict_done_when_reply_says_done() {
        assert_eq!(parse_turn_verdict("DONE"), TurnVerdict::Done);
        assert_eq!(parse_turn_verdict("done.\n"), TurnVerdict::Done);
    }

    #[test]
    fn verdict_fails_safe_to_done_on_garbage() {
        // Unrecognizable / empty replies end the run rather than risk a spurious
        // extra loop.
        assert_eq!(parse_turn_verdict(""), TurnVerdict::Done);
        assert_eq!(parse_turn_verdict("¯\\_(ツ)_/¯"), TurnVerdict::Done);
    }

    #[test]
    fn verdict_continue_wins_when_both_tokens_present() {
        // If the judge hedges and emits both, keep working.
        assert_eq!(
            parse_turn_verdict("not DONE, you should CONTINUE"),
            TurnVerdict::Continue
        );
    }

    // --- structural backstop (language-agnostic punctuation check) ---
    // A terminal message ending in a question mark or a trailing colon is a
    // dangling question (the model can only ask via the ask_user tool), so the
    // harness must nudge instead of silently ending the run.

    #[test]
    fn structural_backstop_nudges_dangling_questions() {
        // The real stalled case: a promise to ask a design question, no tool.
        assert!(should_nudge_terminal(
            "Now for danger memory — the core design question:"
        ));
        assert!(should_nudge_terminal("Do you want me to continue?"));
    }

    #[test]
    fn structural_backstop_spares_real_final_replies() {
        assert!(!should_nudge_terminal("All done."));
        assert!(!should_nudge_terminal("Everything is ready."));
        // Empty text is not a dangling question either.
        assert!(!should_nudge_terminal(""));
    }
}

/// Backend/mock coverage for the completion judge: spin a throwaway local HTTP
/// server that answers `/v1/messages` with a canned Anthropic-shaped body, point
/// an AgentConfig at it, and assert the judge maps the reply to the right
/// verdict. No external network, no mock crate — just tokio (already a dep).
#[cfg(test)]
mod judge_backend_tests {
    use super::*;
    use crate::agent::provider::{AgentConfig, classify_turn_completion_metered};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Start a one-shot HTTP server that replies to a single request with the
    /// given `content` text wrapped in an Anthropic messages response. Returns
    /// the base URL to point the client at.
    async fn spawn_stub(content: &'static str) -> String {
        spawn_stub_with(serde_json::json!({
            "content": [ { "type": "text", "text": content } ]
        }))
        .await
    }

    /// `spawn_stub`, answering with `body` verbatim.
    async fn spawn_stub_with(body: serde_json::Value) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            // Serve every connection (a test may make several judge calls).
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => break,
                };
                // Drain the request headers enough to not RST the client; we
                // don't need the body for the stub.
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let body = body.to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });
        format!("http://{addr}")
    }

    fn cfg_for(base_url: String) -> AgentConfig {
        AgentConfig {
            base_url,
            api_key: "test-key".into(),
            ..AgentConfig::default()
        }
    }

    #[tokio::test]
    async fn judge_backend_mock_continue() {
        let base = spawn_stub("CONTINUE").await;
        let cfg = cfg_for(base);
        let (reply, _) =
            classify_turn_completion_metered(&cfg, "claudinio", "Primeiro vou confirmar:")
                .await
                .unwrap();
        assert_eq!(parse_turn_verdict(&reply), TurnVerdict::Continue);
        assert_eq!(
            judge_terminal_turn_metered(&cfg, "claudinio", "Primeiro vou confirmar:")
                .await
                .0,
            TurnVerdict::Continue
        );
    }

    /// The judge is a billed request like any other: its usage comes back
    /// with the verdict so the run can record it.
    #[tokio::test]
    async fn the_judge_reports_what_its_request_cost() {
        let base = spawn_stub_with(serde_json::json!({
            "content": [ { "type": "text", "text": "DONE" } ],
            "usage": { "input_tokens": 310, "output_tokens": 2, "cache_read_input_tokens": 0 }
        }))
        .await;
        let cfg = cfg_for(base);
        let (verdict, request) =
            judge_terminal_turn_metered(&cfg, "claudinio", "All done, tests pass.").await;
        assert_eq!(verdict, TurnVerdict::Done);
        let usage = request
            .expect("the request was answered")
            .expect("the response carried usage");
        assert_eq!((usage.input_tokens, usage.output_tokens), (310, 2));

        // An answer without usage is still an answered request.
        let base = spawn_stub_with(serde_json::json!({
            "content": [ { "type": "text", "text": "DONE" } ],
        }))
        .await;
        let (_, request) =
            judge_terminal_turn_metered(&cfg_for(base), "claudinio", "All done.").await;
        assert!(matches!(request, Some(None)), "{request:?}");

        // An unreachable judge still fails safe, and made no request.
        let (verdict, request) =
            judge_terminal_turn_metered(&cfg_for("http://127.0.0.1:1".into()), "claudinio", "x")
                .await;
        assert_eq!((verdict, request.is_none()), (TurnVerdict::Done, true));
    }

    #[tokio::test]
    async fn judge_backend_mock_done() {
        let base = spawn_stub("DONE").await;
        let cfg = cfg_for(base);
        let verdict =
            judge_terminal_turn_metered(&cfg, "claudinio", "Tudo pronto, testes passaram.")
                .await
                .0;
        assert_eq!(verdict, TurnVerdict::Done);
    }

    #[tokio::test]
    async fn judge_fails_safe_to_done_when_backend_unreachable() {
        // Nothing listening here — the request errors and the judge must NOT
        // wedge the loop: it falls back to Done.
        let cfg = cfg_for("http://127.0.0.1:1".into());
        let verdict =
            judge_terminal_turn_metered(&cfg, "claudinio", "Vou explorar com subagentes:")
                .await
                .0;
        assert_eq!(verdict, TurnVerdict::Done);
    }

    /// Extract the text of the LAST assistant Turn from a session JSONL — the
    /// dangling message the run ended on.
    fn last_assistant_text(jsonl: &str) -> Option<String> {
        jsonl
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| {
                v.get("kind").and_then(|k| k.as_str()) == Some("turn")
                    && v.get("role").and_then(|r| r.as_str()) == Some("assistant")
            })
            .filter_map(|v| {
                v.get("content").and_then(|c| c.as_array()).map(|blocks| {
                    blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join("")
                })
            })
            .rfind(|s| !s.trim().is_empty())
    }

    /// End-to-end reproduction of the stall: replay the real session that
    /// stopped (912bb460), take the exact dangling assistant message it ended
    /// on, and ask the LIVE Brain model whether the turn was finished. It must
    /// answer CONTINUE — proving the harness would have kept going instead of
    /// going idle.
    ///
    /// Ignored by default (needs network + a key). Run with:
    ///   CLAUDINIO_API_KEY=sk-… cargo test --lib judge_real_api_replays -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "hits the live claudin.io API; requires CLAUDINIO_API_KEY"]
    async fn judge_real_api_replays_stalled_session() {
        let Ok(api_key) = std::env::var("CLAUDINIO_API_KEY") else {
            eprintln!("skipping: CLAUDINIO_API_KEY not set");
            return;
        };
        let jsonl = std::fs::read_to_string(
            "/Users/victortavernari/claudinio_code/.claudinio/sessions/912bb460-7e9b-459a-968d-eb506e5e9ec9.jsonl",
        )
        .expect("session jsonl readable");
        let dangling = last_assistant_text(&jsonl).expect("a final assistant message");
        eprintln!("--- dangling final message ---\n{dangling}\n------------------------------");
        assert!(
            dangling.trim_end().ends_with(':'),
            "sanity: the replayed message is the mid-thought that stalled the run"
        );
        let cfg = AgentConfig {
            base_url: "https://api.claudin.io".into(),
            api_key,
            ..AgentConfig::default()
        };
        // Judge with the Brain model, exactly as the harness does.
        let brain = cfg.model().to_string();
        let (reply, _) = classify_turn_completion_metered(&cfg, &brain, &dangling)
            .await
            .expect("live judge call");
        eprintln!("--- live judge reply: {reply:?} ---");
        assert_eq!(
            parse_turn_verdict(&reply),
            TurnVerdict::Continue,
            "the live Brain model must recognise the dangling promise as unfinished"
        );
    }
}

#[cfg(test)]
mod jev_harness_tests {
    use super::*;
    use crate::agent::jev::test_support::{spawn_stub, stub_backend};
    use crate::agent::loop_watch::{LoopAction, LoopWatch};
    use serde_json::json;

    /// (P(unfinished), P(asks_user), expected) — the live probe of
    /// 2026-10-05, sixteen real-shaped finals in English and Portuguese.
    const PROBE: &[(f64, f64, TurnVerdict)] = &[
        (0.03, 0.03, TurnVerdict::Done), // "Done. I fixed the off-by-one…"
        (0.03, 0.03, TurnVerdict::Done), // "Pronto! Corrigi o bug…"
        (0.04, 0.03, TurnVerdict::Done), // root-cause explanation
        (0.03, 0.03, TurnVerdict::Done), // "## Summary …"
        (0.04, 0.03, TurnVerdict::Done), // "Feito. O build passa…"
        (0.03, 0.04, TurnVerdict::Done), // "I couldn't reproduce the crash…"
        (0.95, 0.61, TurnVerdict::Continue), // "Primeiro, preciso confirmar algo sobre o tempo:"
        (0.96, 0.03, TurnVerdict::Continue), // "Now I'll spawn three subagents…"
        (0.96, 0.04, TurnVerdict::Continue), // "Let me read the config loader next…"
        (0.94, 0.03, TurnVerdict::Continue), // "Vou agora executar os testes…"
        (0.04, 0.99, TurnVerdict::Continue), // "The key question is whether…?"
        (0.05, 0.99, TurnVerdict::Continue), // "I need to ask you: which database…"
        (0.97, 0.04, TurnVerdict::Continue), // cut off mid-sentence
        (0.95, 0.04, TurnVerdict::Continue), // "A seguir vou editar o router.ts…"
        (0.13, 0.99, TurnVerdict::Continue), // "Before I continue, could you tell me…"
        (0.12, 0.99, TurnVerdict::Continue), // "Done with the parser. Should I also…?"
    ];

    #[test]
    fn the_live_probe_maps_to_the_right_verdicts() {
        for &(u, a, want) in PROBE {
            assert_eq!(
                jev_turn_verdict(Some(u), Some(a)),
                Some(want),
                "u={u} a={a}"
            );
        }
    }

    #[test]
    fn the_grey_zone_and_missing_answers_defer_to_the_llm_judge() {
        assert_eq!(jev_turn_verdict(Some(0.5), Some(0.1)), None);
        assert_eq!(jev_turn_verdict(Some(0.1), Some(0.45)), None);
        assert_eq!(jev_turn_verdict(None, Some(0.99)), None);
    }

    #[tokio::test]
    async fn jev_judges_a_terminal_turn_and_reports_its_cost() {
        let (url, h) = spawn_stub(
            200,
            r#"{"answers":{"unfinished":{"type":"noul","noul":0.96},"asks_user":{"type":"noul","noul":0.03}},"usage":{"input_tokens":500,"cost":2.1e-5}}"#,
        );
        let (v, cost) = judge_with_jev(&stub_backend(&url), "Let me read the config next.").await;
        assert_eq!(v, Some(TurnVerdict::Continue));
        assert!((cost - 2.1e-5).abs() < 1e-12);
        let req = h.join().unwrap();
        assert!(req.contains("Let me read the config next."));
        assert!(req.contains(r#""unfinished""#) && req.contains(r#""asks_user""#));
    }

    #[tokio::test]
    async fn a_jev_outage_leaves_the_decision_to_the_llm_judge() {
        let (url, h) = spawn_stub(529, r#"{"error":"overloaded"}"#);
        let (v, cost) = judge_with_jev(&stub_backend(&url), "Done.").await;
        assert_eq!(v, None);
        assert_eq!(cost, 0.0);
        h.join().unwrap();
    }

    fn looping() -> LoopWatch {
        let mut w = LoopWatch::default();
        for _ in 0..3 {
            w.record(
                "bash",
                &json!({"command": "./gradlew assembleDebug"}),
                "FAILURE: 25.0.4.1",
            );
        }
        w
    }

    #[tokio::test]
    async fn a_suspect_loop_jev_calls_stuck_is_nudged() {
        let (url, h) = spawn_stub(
            200,
            r#"{"answers":{"stuck":{"type":"noul","noul":0.96}},"usage":{"input_tokens":300}}"#,
        );
        let mut w = looping();
        let (action, cost) = loop_verdict(&mut w, Some(&stub_backend(&url))).await;
        assert!(matches!(action, LoopAction::Nudge(_)), "{action:?}");
        assert!(cost > 0.0);
        assert!(h.join().unwrap().contains("gradlew"));
    }

    #[tokio::test]
    async fn a_suspect_loop_jev_clears_is_left_alone_and_not_reasked_at_once() {
        let (url, h) = spawn_stub(
            200,
            r#"{"answers":{"stuck":{"type":"noul","noul":0.1}},"usage":{"input_tokens":300}}"#,
        );
        let mut w = looping();
        let (action, _) = loop_verdict(&mut w, Some(&stub_backend(&url))).await;
        assert_eq!(action, LoopAction::None);
        h.join().unwrap();
        assert!(!w.should_ask());
    }

    #[tokio::test]
    async fn without_jev_only_the_certain_gate_acts() {
        let mut w = looping();
        assert_eq!(loop_verdict(&mut w, None).await.0, LoopAction::None);
        for _ in 0..2 {
            w.record(
                "bash",
                &json!({"command": "./gradlew assembleDebug"}),
                "FAILURE: 25.0.4.1",
            );
        }
        assert!(matches!(
            loop_verdict(&mut w, None).await.0,
            LoopAction::Nudge(_)
        ));
    }
}
