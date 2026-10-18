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

use crate::agent::events::AgentEvent;
use crate::agent::jev::{JevBackend, noul};
use crate::agent::persist::{SessionRecord, SessionStore, now_ms};
use crate::agent::provider::AgentConfig;
use crate::agent::session::{ModeCtl, ModeOrigin, PromptProfile, SessionMode};
use crate::agent::tools::ToolContext;
use std::sync::Arc;
use tauri::ipc::Channel;

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

    /// One line for the timeline when the router put the session in a mode.
    pub fn reason(&self) -> String {
        match (self.mode, self.p_decisions, self.p_design) {
            (SessionMode::Brain, Some(d), Some(g)) => format!(
                "Auto: this request has open decisions ({:.0}%) and needs a design first ({:.0}%)",
                d * 100.0,
                g * 100.0
            ),
            (SessionMode::Builder, _, _) => "Auto: this request can be built directly".into(),
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

/// Route the first prompt of a fresh session (`agent::route`): record what the
/// router would start it as, and — when the router is on and the user left the
/// choice to it — start it that way. Returns what an awaited verdict cost.
///
/// Called before the user's message becomes a turn, which is what makes
/// "fresh" checkable: no history, and no record that this session continues
/// another one or already has work lined up.
#[allow(clippy::too_many_arguments)]
pub(super) async fn route_first_prompt(
    config: &AgentConfig,
    profile: PromptProfile,
    fresh: bool,
    prompt: &str,
    store: &SessionStore,
    ctx: &ToolContext,
    mode_ctl: &Arc<ModeCtl>,
    event_tx: &Channel<AgentEvent>,
) -> f64 {
    use crate::agent::route::{self, RouteMode};
    // Read unconditionally: an Auto request is for this prompt and must not
    // survive to route a later one.
    let auto = mode_ctl.take_auto();
    let route_mode = config.jev.route;
    if profile != PromptProfile::Standard || route_mode == RouteMode::Off || !fresh {
        return 0.0;
    }
    // Brain always runs the Standard profile. On a window that small it has
    // nowhere to go but the refusal, and routing would trade a Builder session
    // that works (the Compact profile) for one that cannot start.
    if config.context_window_for(&config.brain_model) < crate::agent::surface::COMPACT_BELOW {
        return 0.0;
    }
    let records = crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
        .unwrap_or_default();
    // A successor session carries a plan; a session with tasks (golden goals
    // create them before the run) already has a loop that flips its mode.
    let has_work = records.iter().any(|r| {
        matches!(
            r,
            SessionRecord::LinkedFrom { .. } | SessionRecord::Tasks { .. }
        )
    });
    if has_work {
        return 0.0;
    }
    let chosen = mode_ctl.get();
    let (ran_as, _) = chosen;
    let backend = crate::agent::jev::backend(config);
    let record = move |v: &route::Verdict, shadow: bool| SessionRecord::ModeRoute {
        verdict: v.mode.as_str().into(),
        ran_as: ran_as.as_str().into(),
        source: v.source.into(),
        shadow,
        p_decisions: v.p_decisions,
        p_design: v.p_design,
        cost: v.cost,
        ts: now_ms(),
    };

    if route_mode == RouteMode::On && auto {
        let verdict = route::route(prompt, backend.as_ref()).await;
        // The verdict was awaited, and the mode toggle works meanwhile. A
        // choice the user made in that time is theirs: writing over it would
        // also hand the agent the right to leave a Brain the user turned on.
        let untouched = mode_ctl.get() == chosen;
        store.try_append(&record(&verdict, !untouched));
        // Auto was the last thing chosen, so the verdict is the mode, wherever
        // the control stood before: a user can try Brain, go back to Auto, and
        // be sent to Builder. And a Brain reached through Auto is the
        // harness's choice even when the user had clicked Brain earlier —
        // agent origin, like `enter_plan_mode`: the flow then runs interview →
        // plan → tasks → build without a manual flip, and the interview's
        // final confirmation is the user's approval point.
        let reorigin = verdict.mode == SessionMode::Brain && chosen.1 != ModeOrigin::Agent;
        if untouched && (verdict.mode != ran_as || reorigin) {
            mode_ctl.set(verdict.mode, ModeOrigin::Agent);
            store.try_append(&SessionRecord::Mode {
                mode: verdict.mode.as_str().into(),
                origin: ModeOrigin::Agent.as_str().into(),
                ts: now_ms(),
            });
            let _ = event_tx.send(AgentEvent::ModeChanged {
                mode: verdict.mode.as_str().into(),
                origin: ModeOrigin::Agent.as_str().into(),
                reason: Some(verdict.reason()),
            });
        }
        crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
        return verdict.cost;
    }

    // Shadow, or the user chose a mode themselves: the verdict is only worth
    // having next to that choice, and must not cost the first request a wait.
    // What the rules decide costs none, so it is written here, in order with
    // everything else this run writes.
    if route::rules(prompt).is_some() {
        let verdict = route::route(prompt, None).await;
        store.try_append(&record(&verdict, true));
        crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
        return 0.0;
    }
    let prompt = prompt.to_string();
    let path = store.path.clone();
    let cache = ctx.records_cache.clone();
    tokio::spawn(async move {
        let verdict = route::route(&prompt, backend.as_ref()).await;
        // Nobody answered: there is nothing to compare with the user's choice.
        // And a session deleted while Jev was thinking is not brought back as
        // a file holding one verdict.
        if verdict.source == route::SOURCE_UNAVAILABLE || !path.exists() {
            return;
        }
        SessionStore { path: path.clone() }.try_append(&record(&verdict, true));
        crate::agent::persist::invalidate_cache(&path, &cache);
    });
    0.0
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

#[cfg(test)]
mod route_tests {
    use super::*;
    use crate::agent::jev::test_support::spawn_stub;

    const FEATURE: &str = "I want users to be able to share a session with a teammate. \
                           Not sure yet whether it should be a link or an invite.";

    fn store(tag: &str) -> SessionStore {
        let path = std::env::temp_dir().join(format!(
            "claudinio_route_{tag}_{}_{}.jsonl",
            std::process::id(),
            now_ms()
        ));
        std::fs::write(&path, "").unwrap();
        SessionStore { path }
    }

    /// A Claudinio account whose plan endpoint is the stub: the one Jev backend
    /// whose address comes from the config rather than from a constant.
    fn config(route: RouteMode, jev_url: Option<&str>) -> AgentConfig {
        let mut cfg = AgentConfig::default();
        cfg.jev.route = route;
        if let Some(url) = jev_url {
            // Unique per test: a refused key is remembered process-wide.
            cfg.api_key = format!("sk-route-{}", now_ms());
            cfg.services_url = url.trim_end_matches("/v1/systemone").to_string();
        }
        cfg
    }

    fn jev_says(decisions: f64, design: f64) -> &'static str {
        Box::leak(
            serde_json::json!({
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

    async fn run(
        cfg: &AgentConfig,
        store: &SessionStore,
        mode_ctl: &Arc<ModeCtl>,
        fresh: bool,
    ) -> f64 {
        let ctx = crate::agent::tools::tests_support::ctx();
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        route_first_prompt(
            cfg,
            PromptProfile::Standard,
            fresh,
            FEATURE,
            store,
            &ctx,
            mode_ctl,
            &events,
        )
        .await
    }

    fn routes(store: &SessionStore) -> Vec<(String, String, String, bool)> {
        crate::agent::persist::load_records(&store.path)
            .unwrap()
            .into_iter()
            .filter_map(|r| match r {
                SessionRecord::ModeRoute {
                    verdict,
                    ran_as,
                    source,
                    shadow,
                    ..
                } => Some((verdict, ran_as, source, shadow)),
                _ => None,
            })
            .collect()
    }

    /// The shadow verdict is written by a background task.
    async fn wait_for_route(store: &SessionStore) -> Vec<(String, String, String, bool)> {
        for _ in 0..100 {
            let found = routes(store);
            if !found.is_empty() {
                return found;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        Vec::new()
    }

    fn builder() -> Arc<ModeCtl> {
        Arc::new(ModeCtl::new(SessionMode::Builder, ModeOrigin::Human))
    }

    #[tokio::test]
    async fn in_shadow_the_verdict_is_recorded_and_nothing_changes() {
        let (url, stub) = spawn_stub(200, jev_says(0.95, 0.9));
        let (store, mode) = (store("shadow"), builder());
        // Even with Auto requested: shadow never acts.
        mode.request_auto(true);
        let cost = run(&config(RouteMode::Shadow, Some(&url)), &store, &mode, true).await;
        assert_eq!(cost, 0.0, "nothing was awaited");
        assert_eq!(
            wait_for_route(&store).await,
            vec![("brain".into(), "builder".into(), "jev".into(), true)]
        );
        stub.join().unwrap();
        assert_eq!(mode.get(), (SessionMode::Builder, ModeOrigin::Human));
        std::fs::remove_file(&store.path).ok();
    }

    #[tokio::test]
    async fn on_auto_a_planning_request_starts_in_brain_as_the_agents_own_choice() {
        let (url, stub) = spawn_stub(200, jev_says(0.95, 0.9));
        let (store, mode) = (store("on"), builder());
        mode.request_auto(true);
        let cost = run(&config(RouteMode::On, Some(&url)), &store, &mode, true).await;
        stub.join().unwrap();
        assert!(cost > 0.0, "an awaited verdict is charged to the run");
        // Agent origin: it may exit Brain itself once the plan and tasks exist.
        assert_eq!(mode.get(), (SessionMode::Brain, ModeOrigin::Agent));
        assert_eq!(
            routes(&store),
            vec![("brain".into(), "builder".into(), "jev".into(), false)]
        );
        let records = crate::agent::persist::load_records(&store.path).unwrap();
        assert_eq!(
            crate::agent::persist::last_mode(&records),
            Some(("brain".into(), "agent".into())),
            "a reloaded session is in the mode the router chose"
        );
        assert!(!mode.take_auto(), "Auto is for one prompt");
        std::fs::remove_file(&store.path).ok();
    }

    // The verdict is awaited for up to two seconds, and the mode toggle works
    // in the meantime. What the user picks then is their choice — and a Brain
    // they turned on must stay one the agent cannot leave by itself.
    #[tokio::test]
    async fn a_mode_the_user_picks_while_auto_is_deciding_is_kept_as_theirs() {
        let (url, stub) = crate::agent::jev::test_support::spawn_stub_after(
            std::time::Duration::from_millis(400),
            200,
            jev_says(0.95, 0.9),
        );
        let (store, mode) = (store("race"), builder());
        mode.request_auto(true);
        let toggle = mode.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            toggle.set(SessionMode::Brain, ModeOrigin::Human);
        });
        run(&config(RouteMode::On, Some(&url)), &store, &mode, true).await;
        stub.join().unwrap();

        assert_eq!(mode.get(), (SessionMode::Brain, ModeOrigin::Human));
        // Recorded as a verdict that decided nothing.
        assert_eq!(
            routes(&store),
            vec![("brain".into(), "builder".into(), "jev".into(), true)]
        );
        let records = crate::agent::persist::load_records(&store.path).unwrap();
        assert_eq!(
            crate::agent::persist::last_mode(&records),
            None,
            "no mode record of ours"
        );
        std::fs::remove_file(&store.path).ok();
    }

    // Brain always runs the Standard profile. Where the Brain model's window
    // is too small for that, Auto would turn a Builder session that works —
    // on the Compact profile — into one that is refused on its first message.
    #[tokio::test]
    async fn auto_does_not_send_a_session_to_a_brain_that_cannot_run() {
        let (url, stub) = spawn_stub(200, jev_says(0.95, 0.9));
        let mut cfg = config(RouteMode::On, Some(&url));
        cfg.brain_model = "tiny/m".into();
        cfg.providers.insert(
            "tiny".into(),
            crate::agent::provider::ProviderEntry {
                api_key: "k".into(),
                base_url: "http://127.0.0.1:1".into(),
                protocol: "openai".into(),
                enabled_models: vec![],
                label: None,
                model_pricing: Default::default(),
                model_output_limits: Default::default(),
                custom: true,
                custom_models: vec!["m".into()],
                model_context_limits: Default::default(),
                context_window: Some(32_768),
            },
        );
        let (store, mode) = (store("tinybrain"), builder());
        mode.request_auto(true);
        let cost = run(&cfg, &store, &mode, true).await;
        // Nobody was asked: the stub is left waiting and gives up.
        drop(stub);

        assert_eq!(cost, 0.0);
        assert_eq!(mode.get(), (SessionMode::Builder, ModeOrigin::Human));
        assert!(routes(&store).is_empty());
        assert!(!mode.take_auto(), "Auto is still for one prompt only");
        std::fs::remove_file(&store.path).ok();
    }

    // What the rules decide needs no wait, so it is written in line — not by a
    // background task racing the run for the same file.
    #[tokio::test]
    async fn in_shadow_a_verdict_the_rules_reach_is_recorded_before_the_run_goes_on() {
        let (store, mode) = (store("inline"), builder());
        let ctx = crate::agent::tools::tests_support::ctx();
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        route_first_prompt(
            &config(RouteMode::Shadow, None),
            PromptProfile::Standard,
            true,
            "fix the typo",
            &store,
            &ctx,
            &mode,
            &events,
        )
        .await;
        // No waiting: it is there by the time the call returns.
        assert_eq!(
            routes(&store),
            vec![("builder".into(), "builder".into(), "rules".into(), true)]
        );
        std::fs::remove_file(&store.path).ok();
    }

    // The control can be moved to Brain and back to Auto before the first
    // message. Auto is then what was chosen: its verdict is the mode, in
    // either direction, and a Brain it confirms is its own.
    #[tokio::test]
    async fn auto_chosen_after_brain_was_tried_decides_like_auto_always_does() {
        let picked_brain = || Arc::new(ModeCtl::new(SessionMode::Brain, ModeOrigin::Human));

        let (url, stub) = spawn_stub(200, jev_says(0.2, 0.3));
        let (file, mode) = (store("back-to-auto"), picked_brain());
        mode.request_auto(true);
        run(&config(RouteMode::On, Some(&url)), &file, &mode, true).await;
        stub.join().unwrap();
        assert_eq!(mode.get(), (SessionMode::Builder, ModeOrigin::Agent));
        assert_eq!(
            routes(&file),
            vec![("builder".into(), "brain".into(), "jev".into(), false)]
        );
        let records = crate::agent::persist::load_records(&file.path).unwrap();
        assert_eq!(
            crate::agent::persist::last_mode(&records),
            Some(("builder".into(), "agent".into())),
            "a reloaded session is in the mode Auto chose"
        );
        std::fs::remove_file(&file.path).ok();

        // Auto agrees with the earlier click: still Brain, but now a Brain the
        // agent may leave by itself once the plan and tasks exist.
        let (url, stub) = spawn_stub(200, jev_says(0.95, 0.9));
        let (file, mode) = (store("auto-agrees"), picked_brain());
        mode.request_auto(true);
        run(&config(RouteMode::On, Some(&url)), &file, &mode, true).await;
        stub.join().unwrap();
        assert_eq!(mode.get(), (SessionMode::Brain, ModeOrigin::Agent));
        std::fs::remove_file(&file.path).ok();

        // Without Auto, a Brain the user picked is left exactly as it is.
        let (file, mode) = (store("stays-brain"), picked_brain());
        run(&config(RouteMode::On, None), &file, &mode, true).await;
        assert_eq!(mode.get(), (SessionMode::Brain, ModeOrigin::Human));
        std::fs::remove_file(&file.path).ok();
    }

    #[tokio::test]
    async fn on_auto_an_ordinary_request_stays_in_builder() {
        let (url, stub) = spawn_stub(200, jev_says(0.3, 0.9));
        let (store, mode) = (store("stay"), builder());
        mode.request_auto(true);
        run(&config(RouteMode::On, Some(&url)), &store, &mode, true).await;
        stub.join().unwrap();
        assert_eq!(mode.get(), (SessionMode::Builder, ModeOrigin::Human));
        assert_eq!(
            routes(&store),
            vec![("builder".into(), "builder".into(), "jev".into(), false)]
        );
        std::fs::remove_file(&store.path).ok();
    }

    #[tokio::test]
    async fn on_without_jev_auto_is_builder_and_says_nobody_decided() {
        let (store, mode) = (store("nojev"), builder());
        mode.request_auto(true);
        let cost = run(&config(RouteMode::On, None), &store, &mode, true).await;
        assert_eq!(cost, 0.0);
        assert_eq!(mode.get().0, SessionMode::Builder);
        assert_eq!(
            routes(&store),
            vec![(
                "builder".into(),
                "builder".into(),
                "unavailable".into(),
                false
            )]
        );
        std::fs::remove_file(&store.path).ok();
    }

    #[tokio::test]
    async fn a_mode_the_user_picked_is_never_overridden() {
        let (url, stub) = spawn_stub(200, jev_says(0.95, 0.9));
        let (store, mode) = (store("explicit"), builder());
        // Router on, but the user chose Builder by hand: no Auto request.
        run(&config(RouteMode::On, Some(&url)), &store, &mode, true).await;
        assert_eq!(
            wait_for_route(&store).await,
            vec![("brain".into(), "builder".into(), "jev".into(), true)],
            "recorded next to the user's choice, as in shadow"
        );
        stub.join().unwrap();
        assert_eq!(mode.get(), (SessionMode::Builder, ModeOrigin::Human));
        std::fs::remove_file(&store.path).ok();
    }

    #[tokio::test]
    async fn only_the_first_prompt_of_a_fresh_session_is_routed() {
        // The backend points nowhere: asking it would be the failure.
        let cfg = config(RouteMode::On, Some("http://127.0.0.1:9/v1/systemone"));

        // Not fresh: the session already has history.
        let (s1, mode) = (store("history"), builder());
        mode.request_auto(true);
        assert_eq!(run(&cfg, &s1, &mode, false).await, 0.0);
        assert!(
            !mode.take_auto(),
            "the request does not survive to a later prompt"
        );

        // A successor session: it carries a plan to execute.
        let s2 = store("successor");
        s2.try_append(&SessionRecord::LinkedFrom {
            prev_session_id: "p".into(),
            reason: "plan_execution".into(),
            golden_cycle: 0,
            golden_stalls: 0,
            golden_last_pending: vec![],
            ts: 1,
        });
        mode.request_auto(true);
        assert_eq!(run(&cfg, &s2, &mode, true).await, 0.0);

        // Golden goals created tasks before the run: their loop owns the mode.
        let s3 = store("goals");
        crate::agent::persist::append_tasks(&s3.path, &[]).unwrap();
        mode.request_auto(true);
        assert_eq!(run(&cfg, &s3, &mode, true).await, 0.0);

        for s in [&s1, &s2, &s3] {
            assert!(routes(s).is_empty());
            std::fs::remove_file(&s.path).ok();
        }
        assert_eq!(mode.get().0, SessionMode::Builder);
    }

    #[tokio::test]
    async fn off_asks_nothing_and_records_nothing() {
        let cfg = config(RouteMode::Off, Some("http://127.0.0.1:9/v1/systemone"));
        let (store, mode) = (store("off"), builder());
        mode.request_auto(true);
        assert_eq!(run(&cfg, &store, &mode, true).await, 0.0);
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        assert!(routes(&store).is_empty());
        std::fs::remove_file(&store.path).ok();
    }
}
