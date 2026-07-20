//! Reducer: `AgentEvent` → mutações no `App`. Blocos finalizados são
//! renderizados aqui (tema atual) e enfileirados para o scrollback; a região
//! viva guarda só o que está em progresso. Consome TODOS os eventos que a TUI
//! antiga descartava (TextDelta, AskUser, Retrying, subagentes, GoldenLoop,
//! SteeringInjected, SessionStats, diff da aprovação).

use super::app::{App, PendingQuestion, QItem};
use super::transcript::{self, Status, SubLive, ToolCard, ToolState};
use claudinio_core::agent::session::AgentEvent;
use serde_json::Value;
use std::time::{Duration, Instant};

pub fn apply(app: &mut App, ev: AgentEvent) {
    let theme = app.theme;
    match ev {
        AgentEvent::Thinking(t) => {
            clear_retry(app);
            app.thinking = Some(t);
        }
        AgentEvent::TextDelta { text } => {
            clear_retry(app);
            commit_thinking(app);
            app.assistant = Some(text);
        }
        AgentEvent::TextStep { text } => {
            commit_thinking(app);
            if !text.trim().is_empty() {
                let lines = transcript::render_assistant(&text, &theme);
                app.commit(lines);
                app.last_assistant = Some(text);
                app.saw_assistant = true;
            }
            app.assistant = None;
        }
        AgentEvent::ToolCall {
            session_id,
            tool_id,
            tool_name,
            args,
            permission,
            edit_proposal,
        } => {
            commit_thinking(app);
            commit_assistant(app);
            let mut card = ToolCard::new(tool_id.clone(), tool_name, transcript::tool_summary(&args));
            if let Some(ep) = edit_proposal {
                card.diff = Some(ep.unified_diff);
            }
            if permission == "requires_approval" {
                card.state = ToolState::AwaitingApproval;
                card.approval_key = Some(format!("{session_id}:{tool_id}"));
            } else {
                card.state = ToolState::Running;
            }
            app.tools.push(card);
        }
        AgentEvent::ToolResult {
            tool_id,
            output,
            error,
            ..
        } => {
            if let Some(pos) = app.tools.iter().position(|c| c.tool_id == tool_id) {
                let mut card = app.tools.remove(pos);
                card.state = ToolState::Done;
                match error {
                    Some(e) => {
                        card.is_error = true;
                        card.output = Some(e);
                    }
                    None => {
                        if card.diff.is_none() {
                            card.output = Some(output);
                        }
                    }
                }
                let lines = transcript::render_tool_card(&card, &theme, 200);
                app.commit(lines);
            }
        }
        AgentEvent::AskUser {
            session_id,
            tool_id,
            questions,
        } => {
            commit_thinking(app);
            commit_assistant(app);
            app.question = Some(PendingQuestion {
                key: format!("{session_id}:{tool_id}"),
                items: parse_questions(&questions),
                idx: 0,
                answers: Vec::new(),
            });
            app.status = Status::Idle;
        }
        AgentEvent::Done {
            input_tokens,
            output_tokens,
            text_output,
            ..
        } => {
            commit_thinking(app);
            commit_assistant(app);
            // Fallback: texto que só veio como delta / nenhum TextStep.
            if !app.saw_assistant && !text_output.trim().is_empty() {
                let lines = transcript::render_assistant(&text_output, &theme);
                app.commit(lines);
                app.last_assistant = Some(text_output);
            }
            app.in_tok = input_tokens as u64;
            app.out_tok = output_tokens as u64;
            app.running = false;
            app.status = Status::Idle;
            app.retry_deadline = None;
        }
        AgentEvent::SessionStats {
            input_tokens,
            output_tokens,
            cumulative_cost,
            context_tokens,
            max_context_tokens,
            ..
        } => {
            app.in_tok = input_tokens as u64;
            app.out_tok = output_tokens as u64;
            if cumulative_cost.is_some() {
                app.cost = cumulative_cost;
            }
            app.context_tokens = context_tokens;
            app.max_context_tokens = max_context_tokens;
        }
        AgentEvent::Retrying {
            attempt,
            max_attempts,
            delay_ms,
            ..
        } => {
            app.status = Status::Retrying {
                attempt,
                max: max_attempts,
                secs: delay_ms / 1000,
            };
            app.retry_deadline = Some(Instant::now() + Duration::from_millis(delay_ms));
        }
        AgentEvent::SubagentStarted {
            subagent_id,
            name,
            goal,
            ..
        } => {
            app.subagents.push(SubLive {
                id: subagent_id,
                name: name.clone(),
            });
            app.commit_notice(
                format!("⟳ subagente: {name} — {}", transcript::truncate_line(&goal, 100)),
                theme.subagent,
            );
        }
        AgentEvent::Subagent { event, .. } => {
            // Detalhe aninhado: só um traço dim para chamadas de ferramenta.
            if let AgentEvent::ToolCall { tool_name, args, .. } = *event {
                app.commit_notice(
                    format!("  ⟳ ▸ {tool_name} {}", transcript::tool_summary(&args)),
                    theme.dim,
                );
            }
        }
        AgentEvent::SubagentDone {
            subagent_id,
            status,
            rounds,
            input_tokens,
            output_tokens,
            cost,
            ..
        } => {
            let name = app
                .subagents
                .iter()
                .position(|s| s.id == subagent_id)
                .map(|pos| app.subagents.remove(pos).name)
                .unwrap_or_else(|| "subagente".into());
            let lines = transcript::render_subagent_done(
                &name, &status, rounds, input_tokens, output_tokens, cost, &theme,
            );
            app.commit(lines);
        }
        AgentEvent::GoldenLoop {
            cycle, max_cycles, ..
        } => {
            app.commit_notice(format!("◇ golden — ciclo {cycle}/{max_cycles}"), theme.warning);
        }
        AgentEvent::SessionLinked { reason, .. } => {
            app.commit_notice(format!("⇄ handoff ({reason}) → nova sessão encadeada"), theme.warning);
        }
        AgentEvent::SteeringInjected { text, .. } => {
            app.commit_notice(format!("↳ steering: {}", transcript::truncate_line(&text, 120)), theme.muted);
        }
        AgentEvent::ModeChanged { mode, origin, .. } => {
            if let Some(m) = claudinio_core::agent::session::SessionMode::parse(&mode) {
                app.mode = m;
            }
            if origin == "agent" {
                app.commit_notice(format!("⇄ modo → {mode} (agente)"), theme.muted);
            }
        }
        AgentEvent::Error(e) => {
            app.running = false;
            app.status = Status::Idle;
            app.commit_notice(format!("erro: {e}"), theme.error);
        }
    }
}

fn commit_thinking(app: &mut App) {
    let theme = app.theme;
    if let Some(t) = app.thinking.take() {
        if !t.trim().is_empty() {
            let lines = transcript::render_thinking(&t, &theme);
            app.commit(lines);
        }
    }
}

fn commit_assistant(app: &mut App) {
    let theme = app.theme;
    if let Some(a) = app.assistant.take() {
        if !a.trim().is_empty() {
            let lines = transcript::render_assistant(&a, &theme);
            app.commit(lines);
            app.last_assistant = Some(a);
            app.saw_assistant = true;
        }
    }
}

fn clear_retry(app: &mut App) {
    if matches!(app.status, Status::Retrying { .. }) {
        app.status = if app.running { Status::Working } else { Status::Idle };
    }
    app.retry_deadline = None;
}

fn parse_questions(v: &Value) -> Vec<QItem> {
    v.as_array()
        .map(|arr| {
            arr.iter()
                .map(|q| {
                    let question = q
                        .get("question")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    let options = q
                        .get("options")
                        .and_then(|o| o.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|opt| {
                                    opt.as_str()
                                        .map(|s| s.to_string())
                                        .or_else(|| opt.get("label").and_then(|l| l.as_str()).map(|s| s.to_string()))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    QItem { question, options }
                })
                .collect()
        })
        .unwrap_or_default()
}
