//! Choosing the phase a session starts in: plan first, or build.
//!
//! Brain and Builder bundle four things — the write permission, the model, the
//! workflow prompt, and the context reset between plan and execution. Three of
//! those are hard boundaries and stay that way. The fourth is a judgement about
//! the request: does this need decisions and a design before any code? Until
//! now a person made that call by flipping a toggle, on roughly one new session
//! in three, and the model's own route to planning (`enter_plan_mode`) went
//! almost unused — 8 calls in 16,945 across the sessions measured.
//!
//! This module makes that one judgement, once, on the first prompt of a fresh
//! session. That is the only moment it is free: there is no history, so no
//! prefix cache to throw away by changing the prompt, the tools and the model.
//!
//! Rules decide what they can; Jev is asked only about meaning. And the two
//! ways of being wrong are not equally bad. Sending a planning request to
//! Builder is today's default — the model can still call `enter_plan_mode`.
//! Sending a quick edit to Brain costs the user an interview nobody asked for.
//! So Brain needs both questions answered yes with confidence, and everything
//! short of that — a rule, an unclear answer, no Jev, a timeout — is Builder.
//!
//! It is on by default, which means two things for a user who has never
//! opened Settings. A new session starts with Auto selected wherever there is
//! a Jev to ask; where there is none the control does not offer it, and the
//! session starts in Builder as it always did. And the first prompt of each
//! new session is sent to Jev — the setting's hint says so, and Off is one
//! click away.
//!
//! The thresholds below are still provisional: they have not been calibrated
//! on real first prompts (`live_route_calibration` does that, and Shadow
//! records what the router would have chosen next to what the user chose).
//! What keeps a wrong threshold cheap is the asymmetry above — the failure it
//! can produce by design is a session left in Builder.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

use crate::agent::jev::{JevBackend, noul};
use crate::agent::session::SessionMode;

/// How much say the router has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteMode {
    /// Never asked.
    Off,
    /// Asked in the background and recorded next to what the session actually
    /// ran as. Changes nothing, costs no latency.
    Shadow,
    /// A session left on Auto starts in the phase the router picks.
    #[default]
    On,
}

impl RouteMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            RouteMode::Off => "off",
            RouteMode::Shadow => "shadow",
            RouteMode::On => "on",
        }
    }

    pub fn parse(s: &str) -> Option<RouteMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => Some(RouteMode::Off),
            "shadow" => Some(RouteMode::Shadow),
            "on" => Some(RouteMode::On),
            _ => None,
        }
    }
}

// By hand, so that a value this build does not know is read as if the setting
// were absent — the default — rather than as an error. The config is loaded
// whole or not at all: a derived impl failing on `"route": "On"` — a hand
// edit, a value from a newer build — would discard the user's API key and
// providers along with it.
impl<'de> Deserialize<'de> for RouteMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(value
            .as_str()
            .and_then(RouteMode::parse)
            .unwrap_or_default())
    }
}

pub const SOURCE_RULES: &str = "rules";
pub const SOURCE_JEV: &str = "jev";
/// No backend, a timeout, or an answer missing a question: nobody decided.
pub const SOURCE_UNAVAILABLE: &str = "unavailable";

/// P(decisions are still open) from which a request may go to Brain.
/// PROVISIONAL — not yet calibrated against real prompts.
pub const PLAN_DECISIONS_AT: f64 = 0.75;
/// P(needs a written design) from which a request may go to Brain.
/// PROVISIONAL — not yet calibrated against real prompts.
pub const PLAN_DESIGN_AT: f64 = 0.75;
/// A prompt shorter than this goes to Builder without asking. Whatever it
/// wants, it has not said enough for anyone to judge that it needs a design:
/// the median first prompt of a session the user started in Brain was 164
/// characters, of one started in Builder 36.
pub const MIN_PLAN_PROMPT_CHARS: usize = 24;
/// More of the prompt than this adds cost and nothing a routing decision needs.
const MAX_PROMPT_CHARS: usize = 6_000;
/// The verdict is awaited before the first request when the router is on, so
/// it gets far less patience than Jev's own timeout.
const TIMEOUT: Duration = Duration::from_secs(2);

/// What the router concluded, and on what grounds.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub mode: SessionMode,
    pub source: &'static str,
    pub p_decisions: Option<f64>,
    pub p_design: Option<f64>,
    /// What Jev cost, in USD.
    pub cost: f64,
}

impl Verdict {
    fn builder(source: &'static str) -> Self {
        Verdict {
            mode: SessionMode::Builder,
            source,
            p_decisions: None,
            p_design: None,
            cost: 0.0,
        }
    }

    /// One line for the timeline when the router moved the session to Brain.
    pub fn reason(&self) -> String {
        match (self.p_decisions, self.p_design) {
            (Some(d), Some(g)) => format!(
                "Auto: this request has open decisions ({:.0}%) and needs a design first ({:.0}%)",
                d * 100.0,
                g * 100.0
            ),
            _ => "Auto".into(),
        }
    }
}

/// What can be decided without asking anyone. `None` means it is a judgement.
pub fn rules(prompt: &str) -> Option<SessionMode> {
    (prompt.trim().chars().count() < MIN_PLAN_PROMPT_CHARS).then_some(SessionMode::Builder)
}

/// The two questions. Both sides of each are spelled out: Jev reads a question
/// literally, and "needs planning" on its own would be answered yes for
/// anything longer than a sentence.
pub fn questions() -> Value {
    json!({
        "decisions": noul(
            "This is the first message a user sent to an AI coding agent. Carrying it out \
             well depends on product or design decisions that only the user can make — \
             scope, behaviour, user experience, trade-offs between approaches — and the \
             message does not settle them.",
            "Decisions only the user can make are still open, and the user should be \
             asked about them before any code is written",
            "The message is specific enough to act on as written, or it is a question, \
             a command to run, a bug to fix, or a small well-defined change",
        ),
        "design": noul(
            "This is the first message a user sent to an AI coding agent. It asks for a \
             substantial change — a new feature, a redesign, or a refactor that spans \
             several files or components — that should have a written design and a task \
             breakdown before implementation starts.",
            "It is a substantial change that should be designed and broken into tasks first",
            "It is a question, an explanation, a command, a bug fix, or a change small \
             enough to make directly",
        ),
    })
}

/// What Jev reads: the prompt and nothing else. No attachments, no workspace
/// content — Jev is injectable from its state, and the first prompt is the one
/// piece of it the user wrote themselves.
pub fn state(prompt: &str) -> String {
    let prompt = prompt.trim();
    let mut end = prompt.len().min(MAX_PROMPT_CHARS);
    while !prompt.is_char_boundary(end) {
        end -= 1;
    }
    format!("User's first message:\n\n{}", &prompt[..end])
}

/// Brain only when both answers are a confident yes.
pub fn decide(p_decisions: f64, p_design: f64) -> SessionMode {
    if p_decisions >= PLAN_DECISIONS_AT && p_design >= PLAN_DESIGN_AT {
        SessionMode::Brain
    } else {
        SessionMode::Builder
    }
}

/// Route one first prompt. Never fails: whatever goes wrong is Builder.
pub async fn route(prompt: &str, backend: Option<&JevBackend>) -> Verdict {
    if let Some(mode) = rules(prompt) {
        return Verdict {
            mode,
            ..Verdict::builder(SOURCE_RULES)
        };
    }
    let Some(backend) = backend else {
        return Verdict::builder(SOURCE_UNAVAILABLE);
    };
    let (state, questions) = (state(prompt), questions());
    let asked = crate::agent::jev::decide(backend, &state, &questions);
    let Ok(Some(answer)) = tokio::time::timeout(TIMEOUT, asked).await else {
        return Verdict::builder(SOURCE_UNAVAILABLE);
    };
    let (Some(p_decisions), Some(p_design)) = (answer.noul("decisions"), answer.noul("design"))
    else {
        return Verdict {
            cost: answer.cost,
            ..Verdict::builder(SOURCE_UNAVAILABLE)
        };
    };
    Verdict {
        mode: decide(p_decisions, p_design),
        source: SOURCE_JEV,
        p_decisions: Some(p_decisions),
        p_design: Some(p_design),
        cost: answer.cost,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::jev::test_support::{spawn_stub, stub_backend};

    const FEATURE: &str = "I want users to be able to share a session with a teammate. \
                           Not sure yet whether it should be a link or an invite.";

    fn answers(decisions: f64, design: f64) -> &'static str {
        Box::leak(
            json!({
                "answers": {
                    "decisions": {"type": "noul", "noul": decisions},
                    "design": {"type": "noul", "noul": design},
                },
                "usage": {"input_tokens": 200, "cost": 2e-6},
            })
            .to_string()
            .into_boxed_str(),
        )
    }

    #[test]
    fn a_route_setting_this_build_does_not_know_is_the_default_not_a_broken_config() {
        let load = |route: Value| -> crate::agent::provider::AgentConfig {
            serde_json::from_value(json!({
                "base_url": "https://api.claudin.io",
                "api_key": "sk-kept",
                "jev": { "route": route },
            }))
            .expect("the rest of the config still loads")
        };
        assert_eq!(load(json!("shadow")).jev.route, RouteMode::Shadow);
        assert_eq!(load(json!("OFF")).jev.route, RouteMode::Off);
        for unknown in [
            json!("always"),
            json!(true),
            json!(null),
            json!({"mode": "on"}),
        ] {
            let cfg = load(unknown);
            assert_eq!(cfg.jev.route, RouteMode::default());
            assert_eq!(cfg.api_key, "sk-kept");
        }
        // And what is written is what `parse` reads back.
        assert_eq!(
            serde_json::to_value(RouteMode::Shadow).unwrap(),
            json!("shadow")
        );
    }

    #[test]
    fn brain_needs_both_answers_to_be_a_confident_yes() {
        assert_eq!(decide(0.9, 0.9), SessionMode::Brain);
        assert_eq!(
            decide(PLAN_DECISIONS_AT, PLAN_DESIGN_AT),
            SessionMode::Brain
        );
        // A large but fully specified change: build it.
        assert_eq!(decide(0.2, 0.95), SessionMode::Builder);
        // An open question about a one-line change: ask in Builder, do not plan.
        assert_eq!(decide(0.95, 0.2), SessionMode::Builder);
        // Unsure on both is not a yes.
        assert_eq!(decide(0.6, 0.6), SessionMode::Builder);
    }

    #[test]
    fn a_prompt_too_short_to_judge_is_builder_by_rule() {
        assert_eq!(rules("continue"), Some(SessionMode::Builder));
        assert_eq!(rules("  fix the build  "), Some(SessionMode::Builder));
        assert_eq!(rules(""), Some(SessionMode::Builder));
        // Counted in characters, not bytes: accents do not make a prompt long.
        assert_eq!(rules("ação é função não"), Some(SessionMode::Builder));
        assert_eq!(rules(FEATURE), None);
    }

    #[tokio::test]
    async fn a_rule_is_applied_without_asking_jev() {
        // The backend points nowhere: a request would fail the test by timing out.
        let unreachable = stub_backend("http://127.0.0.1:9/unused");
        let v = route("run the tests", Some(&unreachable)).await;
        assert_eq!((v.mode, v.source), (SessionMode::Builder, SOURCE_RULES));
        assert_eq!(v.cost, 0.0);
    }

    #[tokio::test]
    async fn without_a_backend_the_answer_is_builder() {
        let v = route(FEATURE, None).await;
        assert_eq!(
            (v.mode, v.source),
            (SessionMode::Builder, SOURCE_UNAVAILABLE)
        );
    }

    #[tokio::test]
    async fn a_confident_yes_on_both_routes_to_brain_and_says_why() {
        let (url, stub) = spawn_stub(200, answers(0.91, 0.84));
        let v = route(FEATURE, Some(&stub_backend(&url))).await;
        let request = stub.join().unwrap();
        assert_eq!((v.mode, v.source), (SessionMode::Brain, SOURCE_JEV));
        assert_eq!((v.p_decisions, v.p_design), (Some(0.91), Some(0.84)));
        assert!(v.cost > 0.0);
        assert!(
            v.reason().contains("91%") && v.reason().contains("84%"),
            "{}",
            v.reason()
        );
        // Jev was shown the prompt, and only the prompt.
        assert!(
            request.contains("share a session with a teammate"),
            "{request}"
        );
    }

    #[tokio::test]
    async fn an_unsure_answer_stays_in_builder() {
        let (url, stub) = spawn_stub(200, answers(0.9, 0.5));
        let v = route(FEATURE, Some(&stub_backend(&url))).await;
        stub.join().unwrap();
        assert_eq!((v.mode, v.source), (SessionMode::Builder, SOURCE_JEV));
        // The probabilities are kept either way: they are what calibration reads.
        assert_eq!(v.p_design, Some(0.5));
    }

    #[tokio::test]
    async fn a_jev_error_or_a_half_answer_is_builder() {
        let (url, stub) = spawn_stub(529, r#"{"error":"overloaded"}"#);
        let v = route(FEATURE, Some(&stub_backend(&url))).await;
        stub.join().unwrap();
        assert_eq!(
            (v.mode, v.source),
            (SessionMode::Builder, SOURCE_UNAVAILABLE)
        );

        let half: &'static str =
            r#"{"answers":{"decisions":{"type":"noul","noul":0.99}},"usage":{"cost":1e-6}}"#;
        let (url, stub) = spawn_stub(200, half);
        let v = route(FEATURE, Some(&stub_backend(&url))).await;
        stub.join().unwrap();
        assert_eq!(
            (v.mode, v.source),
            (SessionMode::Builder, SOURCE_UNAVAILABLE)
        );
        assert!(v.cost > 0.0, "the half answer was still paid for");
    }

    #[test]
    fn the_state_is_the_prompt_and_is_clipped_on_a_character_boundary() {
        assert_eq!(state("  hello  "), "User's first message:\n\nhello");
        let long = "é".repeat(MAX_PROMPT_CHARS);
        let s = state(&long);
        assert!(s.len() <= MAX_PROMPT_CHARS + 32);
        assert!(s.ends_with('é'));
    }

    #[test]
    fn route_mode_round_trips_and_defaults_to_on() {
        for m in [RouteMode::Off, RouteMode::Shadow, RouteMode::On] {
            assert_eq!(RouteMode::parse(m.as_str()), Some(m));
        }
        assert_eq!(RouteMode::parse("always"), None);
        assert_eq!(RouteMode::default(), RouteMode::On);
    }

    /// Calibrate the thresholds on real first prompts, labelled by what the
    /// user actually chose. Needs a sessions directory and a Jev credential:
    ///
    ///   CLAUDINIO_REPLAY_SESSIONS=~/proj/.claudinio/sessions \
    ///   JEV_LIVE_OPENROUTER_KEY=sk-or-... \
    ///     cargo test --lib -- --ignored live_route_calibration --nocapture
    ///
    /// A session counts when it is not a successor and has a first prompt
    /// long enough to be a judgement; its label is the mode it was in when
    /// that prompt was sent. Prints the two score distributions per label and,
    /// for a grid of thresholds, how many Builder-labelled prompts would have
    /// gone to Brain (the expensive mistake) and how many Brain-labelled ones
    /// would have been caught.
    #[tokio::test]
    #[ignore]
    async fn live_route_calibration() {
        use crate::agent::persist::SessionRecord;
        let dir = std::env::var("CLAUDINIO_REPLAY_SESSIONS").expect("CLAUDINIO_REPLAY_SESSIONS");
        let key = std::env::var("JEV_LIVE_OPENROUTER_KEY").expect("JEV_LIVE_OPENROUTER_KEY");
        let backend = JevBackend {
            source: crate::agent::jev::JevSource::OpenRouter,
            url: crate::agent::jev::OPENROUTER_URL.into(),
            api_key: key,
            model: crate::agent::jev::OPENROUTER_MODEL.into(),
        };
        // (label is brain, p_decisions, p_design)
        let mut scored: Vec<(bool, f64, f64)> = Vec::new();
        let (mut by_rule, mut failed) = (0usize, 0usize);
        for entry in std::fs::read_dir(&dir).expect("readable sessions directory") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let records = crate::agent::persist::load_records(&path).unwrap_or_default();
            if records
                .iter()
                .any(|r| matches!(r, SessionRecord::LinkedFrom { .. }))
            {
                continue;
            }
            let Some((prompt, sent_at)) = records.iter().find_map(|r| match r {
                SessionRecord::User { text, ts } => Some((text.clone(), *ts)),
                _ => None,
            }) else {
                continue;
            };
            let brain = records
                .iter()
                .filter_map(|r| match r {
                    SessionRecord::Mode { mode, ts, .. } if *ts <= sent_at => Some(mode.as_str()),
                    _ => None,
                })
                .next_back()
                .and_then(SessionMode::parse)
                == Some(SessionMode::Brain);
            let v = route(&prompt, Some(&backend)).await;
            match (v.source, v.p_decisions, v.p_design) {
                (SOURCE_RULES, ..) => by_rule += 1,
                (_, Some(d), Some(g)) => scored.push((brain, d, g)),
                _ => failed += 1,
            }
        }
        let labelled = |brain: bool| scored.iter().filter(|s| s.0 == brain).count();
        println!(
            "judged {} (brain {}, builder {}) | builder by rule {by_rule} | no answer {failed}",
            scored.len(),
            labelled(true),
            labelled(false)
        );
        let quantiles = |pick: fn(&(bool, f64, f64)) -> f64, brain: bool| -> String {
            let mut xs: Vec<f64> = scored.iter().filter(|s| s.0 == brain).map(pick).collect();
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            if xs.is_empty() {
                return "-".into();
            }
            let at = |q: f64| xs[((xs.len() as f64 * q) as usize).min(xs.len() - 1)];
            format!("p10 {:.2} p50 {:.2} p90 {:.2}", at(0.1), at(0.5), at(0.9))
        };
        for (name, pick) in [
            ("decisions", (|s| s.1) as fn(&(bool, f64, f64)) -> f64),
            ("design", |s| s.2),
        ] {
            println!("{name:9} brain-labelled {}", quantiles(pick, true));
            println!("{name:9} builder-labelled {}", quantiles(pick, false));
        }
        println!("threshold (both) | builder→brain (wrong, costly) | brain caught");
        for t in [0.5, 0.6, 0.7, 0.75, 0.8, 0.85, 0.9] {
            let to_brain = |brain: bool| {
                scored
                    .iter()
                    .filter(|s| s.0 == brain && s.1 >= t && s.2 >= t)
                    .count()
            };
            println!(
                "  {t:.2}           | {:>3} of {:<3}                    | {:>3} of {}",
                to_brain(false),
                labelled(false),
                to_brain(true),
                labelled(true)
            );
        }
        assert!(
            !scored.is_empty(),
            "no session in {dir} had a prompt to judge"
        );
    }
}
