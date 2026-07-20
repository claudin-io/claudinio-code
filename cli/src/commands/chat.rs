//! `claudinio chat` — TUI interativa (ratatui) para brain/builder com handoff,
//! streaming, aprovação inline de ferramenta e toggle de modo. Multi-turno sobre
//! a mesma sessão do workspace, reaproveitando `core::run::run_to_completion`.

use crate::model;
use claudinio_core::agent::persist::{self, load_records, SessionStore};
use claudinio_core::agent::provider::{self, AgentConfig};
use claudinio_core::agent::session::{
    AgentEvent, AnswerMap, ApprovalMap, EventSink, EventTx, ModeCtl, ModeOrigin, SessionMode,
    SteeringCtl,
};
use claudinio_core::agent::tools::{ReadTracker, ToolContext};
use claudinio_core::agent::transition::{self, TransitionMaps};
use claudinio_core::run::{run_to_completion, RunArgs};
use claudinio_core::state::{SessionHandle, WorkspaceState};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

struct ChannelSink(mpsc::UnboundedSender<AgentEvent>);
impl EventSink for ChannelSink {
    fn send(&self, ev: AgentEvent) {
        let _ = self.0.send(ev);
    }
}

/// Estado visível da TUI.
struct App {
    lines: Vec<Line<'static>>,
    input: String,
    mode: SessionMode,
    model: String,
    effort: String,
    in_tok: u64,
    out_tok: u64,
    running: bool,
    thinking_len: usize,
    pending_approval: Option<(String, String)>, // (key, tool_name)
    quit: bool,
}

impl App {
    fn push(&mut self, line: Line<'static>) {
        self.lines.push(line);
    }

    fn apply(&mut self, ev: AgentEvent) {
        match ev {
            AgentEvent::Thinking(t) => {
                // Snapshot acumulado: só o delta interessa.
                let delta = t.get(self.thinking_len..).unwrap_or("").to_string();
                self.thinking_len = t.len();
                if delta.trim().is_empty() {
                    return;
                }
                let dim = Style::default().add_modifier(Modifier::DIM);
                self.push(Line::from(Span::styled(format!("  {}", delta.trim()), dim)));
            }
            AgentEvent::TextStep { text } => {
                self.thinking_len = 0;
                for l in text.lines() {
                    self.push(Line::from(l.to_string()));
                }
            }
            AgentEvent::ToolCall {
                session_id,
                tool_id,
                tool_name,
                permission,
                args,
                ..
            } => {
                self.thinking_len = 0;
                let cyan = Style::default().fg(Color::Cyan);
                self.push(Line::from(vec![
                    Span::styled(format!("▸ {tool_name} "), cyan),
                    Span::raw(tool_summary(&args)),
                ]));
                if permission == "requires_approval" {
                    self.pending_approval = Some((format!("{session_id}:{tool_id}"), tool_name));
                }
            }
            AgentEvent::ToolResult { output, error, .. } => {
                self.thinking_len = 0;
                let (txt, style) = match error {
                    Some(e) => (format!("  ✗ {}", first_line(&e)), Style::default().fg(Color::Red)),
                    None => (
                        format!("  {}", first_line(&output)),
                        Style::default().add_modifier(Modifier::DIM),
                    ),
                };
                if !txt.trim().is_empty() {
                    self.push(Line::from(Span::styled(txt, style)));
                }
            }
            AgentEvent::SessionLinked { .. } => {
                self.push(Line::from(Span::styled(
                    "⇄ handoff → nova sessão encadeada".to_string(),
                    Style::default().fg(Color::Yellow),
                )));
            }
            AgentEvent::ModeChanged { .. } => {}
            AgentEvent::SubagentStarted { name, .. } => {
                self.push(Line::from(Span::styled(
                    format!("  ⟳ subagente: {name}"),
                    Style::default().fg(Color::Magenta),
                )));
            }
            AgentEvent::SessionStats {
                input_tokens,
                output_tokens,
                ..
            } => {
                self.in_tok = input_tokens as u64;
                self.out_tok = output_tokens as u64;
            }
            AgentEvent::Done {
                input_tokens,
                output_tokens,
                ..
            } => {
                self.in_tok = input_tokens as u64;
                self.out_tok = output_tokens as u64;
                self.running = false;
                self.thinking_len = 0;
            }
            AgentEvent::Error(e) => {
                self.running = false;
                self.push(Line::from(Span::styled(
                    format!("Erro: {e}"),
                    Style::default().fg(Color::Red),
                )));
            }
            _ => {}
        }
    }
}

/// Contexto persistente entre turnos (montado uma vez).
struct ChatCtx {
    config: AgentConfig,
    ws: Arc<WorkspaceState>,
    maps: TransitionMaps,
    approvals: ApprovalMap,
    answers: AnswerMap,
    embedding_model: Arc<Mutex<Option<claudinio_core::code_intel::embeddings::SharedEmbedder>>>,
    agent_tx: mpsc::UnboundedSender<AgentEvent>,
}

pub async fn run(path: Option<String>) -> anyhow::Result<()> {
    let ws_root = model::resolve_workspace(path)?;
    let root = ws_root.to_string_lossy().to_string();

    let mut config = provider::load_config();
    if config.api_key.is_empty() {
        anyhow::bail!("API key não configurada. Rode `claudinio config set api_key <key>`.");
    }
    if let Some(ws_cfg) = provider::read_workspace_config(&root) {
        provider::merge_workspace_config(&mut config, &ws_cfg);
    }

    let db_path = model::index_db_path(&ws_root);
    if let Some(p) = db_path.parent() {
        std::fs::create_dir_all(p).ok();
    }
    let ws = Arc::new(
        WorkspaceState::open(ws_root.clone(), db_path.clone()).map_err(anyhow::Error::msg)?,
    );

    // Sessão nova para este chat, registrada como ativa no workspace.
    let id = uuid::Uuid::new_v4().to_string();
    let store = SessionStore::create(&id, Some(&root)).map_err(anyhow::Error::msg)?;
    *ws.active_session.lock().await = Some(SessionHandle {
        id: id.clone(),
        store_path: store.path.clone(),
    });

    let mode_ctl = Arc::new(ModeCtl::new(SessionMode::Brain, ModeOrigin::Human));
    let steering_map: Arc<Mutex<HashMap<String, Arc<SteeringCtl>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let modes_map: Arc<Mutex<HashMap<String, Arc<ModeCtl>>>> = Arc::new(Mutex::new(HashMap::new()));
    modes_map.lock().await.insert(id.clone(), mode_ctl.clone());
    let maps = TransitionMaps {
        steering: steering_map,
        modes: modes_map,
        records_cache: transition::new_records_cache(),
    };

    let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

    let model = config.model_for_mode("brain").to_string();
    let effort = config.thinking_effort.clone();
    let chat = ChatCtx {
        config,
        ws,
        maps,
        approvals: Arc::new(Mutex::new(HashMap::new())),
        answers: Arc::new(Mutex::new(HashMap::new())),
        embedding_model: Arc::new(Mutex::new(None)),
        agent_tx,
    };

    let mut app = App {
        lines: vec![Line::from(Span::styled(
            format!("claudinio chat — {root}  (Tab: modo · Ctrl+C: sair/interromper · Enter: enviar)"),
            Style::default().add_modifier(Modifier::DIM),
        ))],
        input: String::new(),
        mode: SessionMode::Brain,
        model,
        effort,
        in_tok: 0,
        out_tok: 0,
        running: false,
        thinking_len: 0,
        pending_approval: None,
        quit: false,
    };

    let mut terminal = ratatui::init();
    let res = event_loop(&mut terminal, &mut app, &chat, &mut agent_rx, mode_ctl).await;
    ratatui::restore();
    res
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    chat: &ChatCtx,
    agent_rx: &mut mpsc::UnboundedReceiver<AgentEvent>,
    mode_ctl: Arc<ModeCtl>,
) -> anyhow::Result<()> {
    loop {
        // Drena eventos do agente (não-bloqueante).
        while let Ok(ev) = agent_rx.try_recv() {
            app.apply(ev);
        }

        terminal.draw(|f| draw(f, app))?;

        if app.quit {
            return Ok(());
        }

        // Input com timeout curto para continuar drenando eventos.
        if event::poll(Duration::from_millis(30))? {
            if let Event::Key(k) = event::read()? {
                // Aprovação pendente: intercepta y/n.
                if let Some((key, _)) = app.pending_approval.clone() {
                    match k.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('s') => {
                            approve(chat, &key, true).await;
                            app.pending_approval = None;
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            approve(chat, &key, false).await;
                            app.pending_approval = None;
                        }
                        _ => {}
                    }
                    continue;
                }

                match (k.code, k.modifiers) {
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        if app.running {
                            // Interrompe o turno atual.
                            if let Some(h) = chat.ws.active_session.lock().await.as_ref() {
                                if let Some(s) = chat.maps.steering.lock().await.get(&h.id) {
                                    s.interrupt.store(true, Ordering::SeqCst);
                                }
                            }
                            app.push(Line::from(Span::styled(
                                "⏹ interrompendo…".to_string(),
                                Style::default().fg(Color::Yellow),
                            )));
                        } else {
                            app.quit = true;
                        }
                    }
                    (KeyCode::Tab, _) if !app.running => {
                        app.mode = match app.mode {
                            SessionMode::Brain => SessionMode::Builder,
                            SessionMode::Builder => SessionMode::Brain,
                        };
                        mode_ctl.set(app.mode, ModeOrigin::Human);
                        app.model = chat.config.model_for_mode(app.mode.as_str()).to_string();
                    }
                    (KeyCode::Enter, _) if !app.running => {
                        let msg = std::mem::take(&mut app.input);
                        if !msg.trim().is_empty() {
                            app.push(Line::from(Span::styled(
                                format!("❯ {msg}"),
                                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                            )));
                            app.running = true;
                            app.thinking_len = 0;
                            start_turn(chat, msg).await?;
                        }
                    }
                    (KeyCode::Backspace, _) => {
                        app.input.pop();
                    }
                    (KeyCode::Char(c), _) => {
                        app.input.push(c);
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn approve(chat: &ChatCtx, key: &str, ok: bool) {
    if let Some(s) = chat.approvals.lock().await.remove(key) {
        let _ = s.send(ok);
    }
}

/// Monta o `RunArgs` do turno a partir do estado persistente e spawna o driver.
async fn start_turn(chat: &ChatCtx, message: String) -> anyhow::Result<()> {
    let handle = chat
        .ws
        .active_session
        .lock()
        .await
        .clone()
        .ok_or_else(|| anyhow::anyhow!("sessão ativa ausente"))?;
    let store = SessionStore {
        path: handle.store_path.clone(),
    };
    let history = load_records(&handle.store_path)
        .map(|r| persist::history_from_records(&r))
        .unwrap_or_default();

    let mode_ctl = chat
        .maps
        .modes
        .lock()
        .await
        .get(&handle.id)
        .cloned()
        .unwrap_or_else(|| Arc::new(ModeCtl::new(SessionMode::Brain, ModeOrigin::Human)));

    // Steering fresco para este turno (o driver remove no fim).
    let steering = Arc::new(SteeringCtl::new());
    chat.maps
        .steering
        .lock()
        .await
        .insert(handle.id.clone(), steering.clone());

    let mcp = chat.ws.ensure_mcp_connected(&chat.config).await;
    let base_commit = claudinio_core::agent::tools::git_head(
        chat.ws.root.to_string_lossy().as_ref(),
    );

    let ctx = ToolContext {
        db_path: Some(chat.ws.index_db_path.to_string_lossy().to_string()),
        lsp_manager: Some(chat.ws.lsp_manager.clone()),
        workspace_root: Some(chat.ws.root.to_string_lossy().to_string()),
        embedding_model: chat.embedding_model.clone(),
        session_store_path: Some(handle.store_path.to_string_lossy().to_string()),
        read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
        interrupt: Some(steering.interrupt.clone()),
        agent_config: Some(chat.config.clone()),
        plan_save_path: chat.config.plan_save_path.clone(),
        base_commit,
        auto_approve_git: false,
        mcp: Some(mcp),
        mode_ctl: Some(mode_ctl.clone()),
        index_progress: Some(chat.ws.index_progress.clone()),
        records_cache: chat.maps.records_cache.clone(),
    };

    let chan: EventTx = Arc::new(ChannelSink(chat.agent_tx.clone()));
    let args = RunArgs {
        config: chat.config.clone(),
        ws: chat.ws.clone(),
        maps: chat.maps.clone(),
        approvals: chat.approvals.clone(),
        answers: chat.answers.clone(),
        chan,
        handle,
        store,
        ctx,
        mode_ctl,
        steering,
        history,
        message,
        attachment_blocks: Vec::new(),
    };
    tokio::spawn(run_to_completion(args));
    Ok(())
}

fn draw(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .split(f.area());

    // Transcript com auto-scroll para o fim.
    let height = chunks[0].height.saturating_sub(2) as usize;
    let scroll = app.lines.len().saturating_sub(height) as u16;
    let transcript = Paragraph::new(app.lines.clone())
        .block(Block::default().borders(Borders::ALL).title(" claudinio "))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    f.render_widget(transcript, chunks[0]);

    // Status bar.
    let status = if let Some((_, tool)) = &app.pending_approval {
        Line::from(Span::styled(
            format!(" aprovar `{tool}`?  [y/n] "),
            Style::default().bg(Color::Yellow).fg(Color::Black),
        ))
    } else {
        let state = if app.running { "⏳ executando" } else { "pronto" };
        Line::from(Span::styled(
            format!(
                " {}·{}·{} · {} in/{} out · {} ",
                app.mode.as_str(),
                app.model,
                app.effort,
                app.in_tok,
                app.out_tok,
                state
            ),
            Style::default().bg(Color::DarkGray).fg(Color::White),
        ))
    };
    f.render_widget(Paragraph::new(status), chunks[1]);

    // Input.
    let input = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" mensagem "));
    f.render_widget(input, chunks[2]);
}

fn first_line(s: &str) -> String {
    let l = s.lines().next().unwrap_or("").trim();
    if l.chars().count() > 160 {
        format!("{}…", l.chars().take(160).collect::<String>())
    } else {
        l.to_string()
    }
}

fn tool_summary(args: &serde_json::Value) -> String {
    for key in ["path", "command", "query", "file_path", "pattern"] {
        if let Some(v) = args.get(key).and_then(|v| v.as_str()) {
            return first_line(v);
        }
    }
    String::new()
}
