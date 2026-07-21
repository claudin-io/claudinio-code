//! Estado da TUI, setup de sessão e o loop assíncrono. O loop faz `select!`
//! sobre: eventos do agente (mpsc), teclas (thread bloqueante de stdin → mpsc) e
//! um tick de spinner. Blocos finalizados vão para o scrollback via
//! `insert_before`; a região viva é redesenhada num `Viewport::Inline`.

use super::editor::Editor;
use super::overlays::{
    effort_items, help_items, rank_files, theme_items, Mention, Overlay, Select, SelectItem,
    SelectKind, Slash, SlashCmd,
};
use super::theme::{Theme, ThemeKind};
use super::transcript::{Status, SubLive, ToolCard};
use super::{event, render};

use crate::model;
use claudinio_core::agent::attachments;
use claudinio_core::agent::persist::{self, load_records, AttachmentMeta, SessionStore};
use claudinio_core::agent::provider::{self, AgentConfig};
use claudinio_core::agent::session::{
    AgentEvent, AnswerMap, ApprovalMap, EventSink, EventTx, ModeCtl, ModeOrigin, SessionMode,
    SteeringCtl, SteeringEntry, UserAnswer,
};
use claudinio_core::agent::tools::{ReadTracker, ToolContext};
use claudinio_core::agent::transition::{self, TransitionMaps};
use claudinio_core::run::{run_to_completion, RunArgs};
use claudinio_core::state::{SessionHandle, WorkspaceState};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use ratatui::{TerminalOptions, Viewport};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};

/// Como iniciar a sessão da TUI: nova, retomar a mais recente, ou uma específica
/// (id, prefixo de id, ou caminho pro `.jsonl`).
pub enum ResumeTarget {
    None,
    MostRecent,
    Specific(String),
}

/// Pergunta pendente do `ask_user`, respondida via editor (ou dígitos p/ opções).
pub struct PendingQuestion {
    pub key: String,
    pub items: Vec<QItem>,
    pub idx: usize,
    pub answers: Vec<UserAnswer>,
}

pub struct QItem {
    pub question: String,
    pub options: Vec<String>,
}

/// Estado visível + de controle da TUI.
pub struct App {
    pub theme_kind: ThemeKind,
    pub theme: Theme,
    pub mode: SessionMode,
    pub brain_model: String,
    pub builder_model: String,
    pub effort: String,
    pub cwd_label: String,

    pub in_tok: u64,
    pub out_tok: u64,
    pub cost: Option<f64>,
    pub is_sub: bool,
    pub context_tokens: u64,
    pub max_context_tokens: u64,

    pub running: bool,
    pub status: Status,
    pub spinner_tick: u64,
    pub retry_deadline: Option<Instant>,

    pub thinking: Option<String>,
    pub assistant: Option<String>,
    pub saw_assistant: bool,
    pub last_assistant: Option<String>,
    pub tools: Vec<ToolCard>,
    pub subagents: Vec<SubLive>,
    /// Tarefas atuais (painel fixo acima do input), populadas a partir dos args
    /// de `tasks_set`. Persistem no processo (inclusive no handoff Brain→Builder);
    /// só resetam no /new.
    pub tasks: Vec<claudinio_core::tasks::TaskItem>,
    pub question: Option<PendingQuestion>,

    pub editor: Editor,
    pub overlay: Option<Overlay>,
    /// Anexos pendentes (caminhos) que vão no próximo envio.
    pub attachments: Vec<String>,
    /// Arquivos do workspace (relativos), para o `@`-mention.
    pub file_list: Vec<String>,

    pub to_commit: Vec<Vec<Line<'static>>>,

    // controle de sessão (mutável em /new)
    pub mode_ctl: Arc<ModeCtl>,
    pub quit: bool,
}

impl App {
    pub fn cur_model(&self) -> String {
        match self.mode {
            SessionMode::Brain => self.brain_model.clone(),
            SessionMode::Builder => self.builder_model.clone(),
        }
    }

    pub fn commit(&mut self, lines: Vec<Line<'static>>) {
        if !lines.is_empty() {
            self.to_commit.push(lines);
        }
    }

    pub fn commit_notice(&mut self, text: impl Into<String>, color: ratatui::style::Color) {
        let s = text.into();
        self.commit(super::transcript::render_notice(&s, color));
    }

    /// Índice do card aguardando aprovação, se houver.
    pub fn awaiting_idx(&self) -> Option<usize> {
        self.tools
            .iter()
            .position(|c| c.state == super::transcript::ToolState::AwaitingApproval)
    }
}

/// Contexto persistente (imutável de fora: muta só via Mutex internos).
struct ChatCtx {
    config: AgentConfig,
    ws: Arc<WorkspaceState>,
    maps: TransitionMaps,
    approvals: ApprovalMap,
    answers: AnswerMap,
    embedding_model: Arc<Mutex<Option<claudinio_core::code_intel::embeddings::SharedEmbedder>>>,
    agent_tx: mpsc::UnboundedSender<AgentEvent>,
}

struct ChannelSink(mpsc::UnboundedSender<AgentEvent>);
impl EventSink for ChannelSink {
    fn send(&self, ev: AgentEvent) {
        let _ = self.0.send(ev);
    }
}

pub async fn run(path: Option<String>, resume: ResumeTarget) -> anyhow::Result<()> {
    let ws_root = model::resolve_workspace(path)?;
    let root = ws_root.to_string_lossy().to_string();

    let mut config = provider::load_config();
    if config.api_key.is_empty() {
        anyhow::bail!("API key not configured. Run `claudinio config set api_key <key>`.");
    }
    if let Some(ws_cfg) = provider::read_workspace_config(&root) {
        provider::merge_workspace_config(&mut config, &ws_cfg);
    }

    let db_path = model::index_db_path(&ws_root);
    if let Some(p) = db_path.parent() {
        std::fs::create_dir_all(p).ok();
    }
    let ws = Arc::new(WorkspaceState::open(ws_root.clone(), db_path.clone()).map_err(anyhow::Error::msg)?);

    // Sessão: nova, ou retomar uma existente (a mais recente, ou uma específica).
    let target_id: Option<String> = match &resume {
        ResumeTarget::None => None,
        ResumeTarget::MostRecent => persist::list_sessions(Some(&root))
            .ok()
            .and_then(|s| s.into_iter().next())
            .map(|s| s.session_id),
        ResumeTarget::Specific(v) => Some(resume_id_from_arg(v)),
    };
    let want_resume = !matches!(resume, ResumeTarget::None);
    let resolved = target_id.and_then(|tid| persist::resolve_chain(Some(&root), &tid).ok());
    let (id, store_path, initial_mode, resumed) = match resolved {
        Some((tip_id, tip_path, records)) => {
            let mode = records_mode(&records);
            (tip_id, tip_path, mode, Some(records))
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            let store = SessionStore::create(&id, Some(&root)).map_err(anyhow::Error::msg)?;
            (id, store.path, SessionMode::Brain, None)
        }
    };
    *ws.active_session.lock().await = Some(SessionHandle {
        id: id.clone(),
        store_path: store_path.clone(),
    });

    let mode_ctl = Arc::new(ModeCtl::new(initial_mode, ModeOrigin::Human));
    let steering_map: Arc<Mutex<HashMap<String, Arc<SteeringCtl>>>> = Arc::new(Mutex::new(HashMap::new()));
    let modes_map: Arc<Mutex<HashMap<String, Arc<ModeCtl>>>> = Arc::new(Mutex::new(HashMap::new()));
    modes_map.lock().await.insert(id.clone(), mode_ctl.clone());
    let maps = TransitionMaps {
        steering: steering_map,
        modes: modes_map,
        records_cache: transition::new_records_cache(),
    };

    let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

    let theme = Theme::dark();
    let is_sub = config.is_claudinio_account();
    let cwd_label = cwd_label(&ws_root);
    let chat = ChatCtx {
        config: config.clone(),
        ws,
        maps,
        approvals: Arc::new(Mutex::new(HashMap::new())),
        answers: Arc::new(Mutex::new(HashMap::new())),
        embedding_model: Arc::new(Mutex::new(None)),
        agent_tx,
    };

    // Pre-warm: connect MCP eagerly so the first turn doesn't block on stdio spawn.
    chat.ws.ensure_mcp_connected(&chat.config).await;

    // Lista de arquivos do workspace (respeitando .gitignore) para o @-mention.
    let file_list = claudinio_core::code_intel::list_files(&root, 5000);

    let mut app = App {
        theme_kind: ThemeKind::Dark,
        theme,
        mode: initial_mode,
        brain_model: config.brain_model.clone(),
        builder_model: config.builder_model.clone(),
        effort: config.thinking_effort.clone(),
        cwd_label,
        in_tok: 0,
        out_tok: 0,
        cost: None,
        is_sub,
        context_tokens: 0,
        max_context_tokens: 0,
        running: false,
        status: Status::Idle,
        spinner_tick: 0,
        retry_deadline: None,
        thinking: None,
        assistant: None,
        saw_assistant: false,
        last_assistant: None,
        tools: Vec::new(),
        subagents: Vec::new(),
        tasks: Vec::new(),
        question: None,
        editor: Editor::new(&theme),
        overlay: None,
        attachments: Vec::new(),
        file_list,
        to_commit: Vec::new(),
        mode_ctl,
        quit: false,
    };
    app.commit_notice(
        format!("claudinio chat — {root}   ·  Tab: mode · / commands · Ctrl+C: quit"),
        app.theme.dim,
    );

    // Sessão retomada: replay do histórico + restauração de stats/rodapé. O modo
    // já foi restaurado acima (initial_mode). O contexto do modelo é reconstruído
    // do JSONL no próximo turno — o replay aqui é só visual.
    if let Some(records) = &resumed {
        restore_session_stats(&mut app, records);
        for lines in replay_records(records, &app.theme) {
            app.commit(lines);
        }
        let dim = app.theme.dim;
        app.commit_notice(
            format!("── resumed: {} · {} turns ──", session_title(records), count_user_turns(records)),
            dim,
        );
    } else if want_resume {
        let warn = app.theme.warning;
        app.commit_notice("no session to resume — starting a new one", warn);
    }

    // Terminal inline (sem alt-screen: preserva scrollback). Altura DINÂMICA: só
    // o cromo (input+status+footer) quando ocioso — SEM buraco — e cresce pra
    // caber conteúdo/overlays (o loop redimensiona recriando o viewport).
    let init_vh = render::chrome_height(&app);
    let mut terminal = ratatui::try_init_with_options(TerminalOptions {
        viewport: Viewport::Inline(init_vh),
    })
    .map_err(|e| {
        anyhow::anyhow!("could not initialize the TUI (an interactive terminal is required): {e}")
    })?;
    let mut current_vh = init_vh;

    // Gate de stdin: redimensionar recria o viewport, que re-consulta a posição
    // do cursor (DSR `ESC[6n`). A thread leitora não pode estar lendo o stdin
    // nesse instante (senão rouba a resposta → "cursor position could not be
    // read"). O gate garante acesso exclusivo durante o resize.
    let stdin_gate = std::sync::Arc::new(std::sync::Mutex::new(()));

    // Thread leitora de stdin: poll curto segurando o gate, solta entre ciclos.
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Event>();
    let gate_r = stdin_gate.clone();
    std::thread::spawn(move || loop {
        let ev = {
            let _g = gate_r.lock().unwrap();
            match crossterm::event::poll(Duration::from_millis(5)) {
                Ok(true) => crossterm::event::read().ok(),
                Ok(false) => None,
                Err(_) => break,
            }
        };
        match ev {
            Some(ev) => {
                if input_tx.send(ev).is_err() {
                    break;
                }
            }
            None => std::thread::sleep(Duration::from_millis(3)),
        }
    });

    let mut tick = tokio::time::interval(Duration::from_millis(120));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let res = commit_and_draw(&mut terminal, &mut app, &mut current_vh, &stdin_gate);
    if let Err(e) = res {
        ratatui::restore();
        return Err(e.into());
    }

    loop {
        tokio::select! {
            biased;
            Some(ev) = agent_rx.recv() => {
                event::apply(&mut app, ev);
                while let Ok(ev) = agent_rx.try_recv() {
                    event::apply(&mut app, ev);
                }
                commit_and_draw(&mut terminal, &mut app, &mut current_vh, &stdin_gate)?;
            }
            Some(inp) = input_rx.recv() => {
                handle_event(&mut app, &chat, inp).await?;
                if app.quit { break; }
                commit_and_draw(&mut terminal, &mut app, &mut current_vh, &stdin_gate)?;
            }
            _ = tick.tick() => {
                if app.running {
                    app.spinner_tick = app.spinner_tick.wrapping_add(1);
                    refresh_retry(&mut app);
                    commit_and_draw(&mut terminal, &mut app, &mut current_vh, &stdin_gate)?;
                }
            }
        }
        if app.quit {
            break;
        }
    }

    let _ = terminal.clear();
    ratatui::restore();
    Ok(())
}

/// Recria o terminal com uma nova altura de viewport inline. `Terminal::drop` é
/// no-op aqui (não escondemos o cursor), então trocar é seguro.
fn reinit_terminal(vh: u16) -> std::io::Result<ratatui::DefaultTerminal> {
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    ratatui::Terminal::with_options(backend, TerminalOptions { viewport: Viewport::Inline(vh) })
}

/// Drena a fila de commits (→ scrollback), ajusta a altura do viewport pra caber
/// a região viva (cresce pra caber, encolhe pro cromo quando ocioso) e redesenha.
fn commit_and_draw(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    current_vh: &mut u16,
    gate: &std::sync::Mutex<()>,
) -> std::io::Result<()> {
    let width = terminal.size()?.width.max(1);
    for lines in std::mem::take(&mut app.to_commit) {
        let para = Paragraph::new(lines).wrap(Wrap { trim: false });
        let h = para.line_count(width).max(1) as u16;
        terminal.insert_before(h, |buf| para.render(buf.area, buf))?;
    }

    let full_rows = crossterm::terminal::size().map(|(_, h)| h).unwrap_or(24);
    let chrome = render::chrome_height(app);
    let want =
        render::desired_height(app, width).clamp(chrome, full_rows.saturating_sub(1).max(chrome));
    let target = if want > *current_vh {
        want
    } else if render::is_idle(app) {
        chrome
    } else {
        *current_vh
    };
    if target != *current_vh {
        // Resize = recriar o viewport (re-consulta o cursor via DSR). Segura o
        // gate pra a thread leitora não roubar a resposta do DSR.
        let _g = gate.lock().unwrap();
        terminal.clear()?;
        *terminal = reinit_terminal(target)?;
        *current_vh = target;
    }
    terminal.draw(|f| render::draw(f, app))?;
    Ok(())
}

fn refresh_retry(app: &mut App) {
    if let (Status::Retrying { attempt, max, .. }, Some(deadline)) = (&app.status, app.retry_deadline) {
        let secs = deadline.saturating_duration_since(Instant::now()).as_secs();
        app.status = Status::Retrying {
            attempt: *attempt,
            max: *max,
            secs,
        };
    }
}

async fn handle_event(app: &mut App, chat: &ChatCtx, ev: Event) -> anyhow::Result<()> {
    match ev {
        Event::Key(k) if k.kind == crossterm::event::KeyEventKind::Press => {
            handle_key(app, chat, k).await
        }
        Event::Resize(_, _) => Ok(()),
        _ => Ok(()),
    }
}

async fn handle_key(app: &mut App, chat: &ChatCtx, k: KeyEvent) -> anyhow::Result<()> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);

    // Ctrl+C: interrompe (se rodando) ou sai.
    if ctrl && matches!(k.code, KeyCode::Char('c')) {
        if app.running {
            interrupt(app, chat).await;
        } else {
            app.quit = true;
        }
        return Ok(());
    }

    // Aprovação de ferramenta pendente.
    if let Some(idx) = app.awaiting_idx() {
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('s') | KeyCode::Char('S') => {
                decide_approval(app, chat, idx, true).await;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                decide_approval(app, chat, idx, false).await;
            }
            _ => {}
        }
        return Ok(());
    }

    // Pergunta ativa (ask_user).
    if app.question.is_some() {
        return handle_question_key(app, chat, k).await;
    }

    // Overlay ativo.
    if app.overlay.is_some() {
        return handle_overlay_key(app, chat, k).await;
    }

    // Modo normal.
    match k.code {
        KeyCode::Tab if !app.running => {
            toggle_mode(app);
        }
        KeyCode::Enter => {
            if k.modifiers.contains(KeyModifiers::SHIFT) || k.modifiers.contains(KeyModifiers::ALT) {
                app.editor.insert_newline();
            } else {
                submit(app, chat).await?;
            }
        }
        KeyCode::Up if app.editor.is_single_line() => {
            app.editor.history_prev();
        }
        KeyCode::Down if app.editor.is_single_line() => {
            app.editor.history_next();
        }
        _ => {
            app.editor.input(k);
            refresh_overlays(app);
        }
    }
    Ok(())
}

/// Reabre/atualiza a paleta de slash OU o autocomplete de `@`-mention conforme
/// o texto do editor.
fn refresh_overlays(app: &mut App) {
    let text = app.editor.text();
    // Slash: "/comando" no início (uma palavra, sem "/" — não confundir com path).
    if let Some(rest) = text.strip_prefix('/') {
        if !rest.contains(' ') && !rest.contains('/') && app.editor.is_single_line() {
            app.overlay = Some(Overlay::Slash(Slash::build(rest)));
            return;
        }
    }
    // Mention: último "@" sem espaço depois → lista de arquivos filtrada.
    if let Some(q) = mention_query(&text) {
        let matches = rank_files(&q, &app.file_list, 20);
        app.overlay = Some(Overlay::Mention(Mention {
            query: q,
            matches,
            idx: 0,
        }));
        return;
    }
    if matches!(app.overlay, Some(Overlay::Slash(_)) | Some(Overlay::Mention(_))) {
        app.overlay = None;
    }
}

/// Extrai a query do `@`-mention em curso (após o último `@`, sem espaço).
/// Exige ao menos 1 caractere após o `@` (o `@` puro não abre o overlay).
fn mention_query(text: &str) -> Option<String> {
    let at = text.rfind('@')?;
    let after = &text[at + 1..];
    if after.is_empty() || after.chars().any(|c| c.is_whitespace()) {
        None
    } else {
        Some(after.to_string())
    }
}

/// Substitui o `@query` em curso pelo caminho selecionado (+ espaço).
fn insert_mention(app: &mut App, path: &str) {
    let text = app.editor.text();
    if let Some(at) = text.rfind('@') {
        app.editor.set_text(&format!("{}{} ", &text[..at], path));
    }
    app.overlay = None;
}

async fn handle_overlay_key(app: &mut App, chat: &ChatCtx, k: KeyEvent) -> anyhow::Result<()> {
    match k.code {
        KeyCode::Esc => {
            app.overlay = None;
        }
        KeyCode::Up => {
            if let Some(o) = &mut app.overlay {
                o.move_up();
            }
        }
        KeyCode::Down => {
            if let Some(o) = &mut app.overlay {
                o.move_down();
            }
        }
        KeyCode::Tab => match &app.overlay {
            Some(Overlay::Slash(s)) => {
                if let Some(cmd) = s.selected() {
                    app.editor.set_text(&format!("/{} ", cmd.name));
                    app.overlay = None;
                }
            }
            Some(Overlay::Mention(m)) => {
                if let Some(p) = m.selected().cloned() {
                    insert_mention(app, &p);
                }
            }
            _ => {}
        },
        KeyCode::Enter => {
            if let Some(Overlay::Mention(m)) = &app.overlay {
                match m.selected().cloned() {
                    Some(p) => insert_mention(app, &p),
                    None => app.overlay = None,
                }
                return Ok(());
            }
            let action = match &app.overlay {
                Some(Overlay::Slash(s)) => s.selected().map(OverlayAction::Slash),
                Some(Overlay::Select(s)) => s
                    .selected()
                    .map(|it| OverlayAction::Select(s.kind, it.value.clone())),
                _ => None,
            };
            app.overlay = None;
            if let Some(a) = action {
                apply_overlay_action(app, chat, a).await?;
            }
        }
        _ => {
            // Digitação filtra slash/mention; seletores ignoram.
            if matches!(app.overlay, Some(Overlay::Slash(_)) | Some(Overlay::Mention(_))) {
                app.editor.input(k);
                refresh_overlays(app);
            }
        }
    }
    Ok(())
}

enum OverlayAction {
    Slash(SlashCmd),
    Select(SelectKind, String),
}

async fn apply_overlay_action(app: &mut App, chat: &ChatCtx, action: OverlayAction) -> anyhow::Result<()> {
    match action {
        OverlayAction::Slash(cmd) => {
            app.editor.clear();
            run_command(app, chat, cmd.name, "").await
        }
        OverlayAction::Select(kind, value) => {
            match kind {
                SelectKind::Model => {
                    set_model(app, &value);
                    app.commit_notice(format!("modelo ({}) → {value}", app.mode.as_str()), app.theme.accent);
                }
                SelectKind::Effort => {
                    app.effort = value.clone();
                    app.commit_notice(format!("effort → {value}"), app.theme.accent);
                }
                SelectKind::Theme => {
                    set_theme(app, &value);
                }
                SelectKind::Sessions => {
                    resume_session(app, chat, &value).await?;
                }
                SelectKind::Help => {}
            }
            Ok(())
        }
    }
}

async fn handle_question_key(app: &mut App, chat: &ChatCtx, k: KeyEvent) -> anyhow::Result<()> {
    // Dígito escolhe opção; Enter usa o texto do editor; Shift/Alt+Enter = nova linha.
    if let KeyCode::Char(c) = k.code {
        if c.is_ascii_digit() && c != '0' {
            let pick = c.to_digit(10).unwrap() as usize - 1;
            let opt = app
                .question
                .as_ref()
                .and_then(|q| q.items.get(q.idx))
                .and_then(|it| it.options.get(pick))
                .cloned();
            if let Some(opt) = opt {
                answer_current(app, chat, opt).await;
                return Ok(());
            }
        }
    }
    match k.code {
        KeyCode::Enter if !k.modifiers.contains(KeyModifiers::SHIFT) && !k.modifiers.contains(KeyModifiers::ALT) => {
            let text = app.editor.text().trim().to_string();
            if !text.is_empty() {
                app.editor.clear();
                answer_current(app, chat, text).await;
            }
        }
        KeyCode::Enter => app.editor.insert_newline(),
        _ => app.editor.input(k),
    }
    Ok(())
}

/// Registra a resposta da pergunta atual; ao responder todas, envia via AnswerMap.
async fn answer_current(app: &mut App, chat: &ChatCtx, answer: String) {
    let theme = app.theme;
    let question_text = match app.question.as_ref().and_then(|q| q.items.get(q.idx)) {
        Some(i) => i.question.clone(),
        None => return,
    };
    if let Some(q) = app.question.as_mut() {
        q.answers.push(UserAnswer {
            question: question_text.clone(),
            answer: answer.clone(),
        });
        q.idx += 1;
    }
    let lines = super::transcript::render_question_answered(&question_text, &answer, &theme);
    app.commit(lines);

    let done = app.question.as_ref().map(|q| q.idx >= q.items.len()).unwrap_or(false);
    if done {
        if let Some(pending) = app.question.take() {
            if let Some(s) = chat.answers.lock().await.remove(&pending.key) {
                let _ = s.send(pending.answers);
            }
        }
    }
}

async fn submit(app: &mut App, chat: &ChatCtx) -> anyhow::Result<()> {
    let raw = app.editor.text().trim().to_string();

    // Comando de barra? (distinguir de um caminho colado tipo "/Users/...")
    if looks_like_command(&raw) {
        app.editor.clear();
        app.overlay = None;
        let after = &raw[1..];
        let (name, arg) = after.split_once(' ').unwrap_or((after, ""));
        return run_command(app, chat, name, arg.trim()).await;
    }

    // Anexos: auto-detectados no texto (arrastar/colar caminho) + os pendentes.
    let (cleaned, mut auto) = extract_attachments(&raw);
    let mut paths = std::mem::take(&mut app.attachments);
    paths.append(&mut auto);
    let text = cleaned;

    if text.is_empty() && paths.is_empty() {
        return Ok(());
    }
    if !raw.is_empty() {
        app.editor.push_history(raw);
    }
    app.editor.clear();

    let processed = attachments::process_attachments(&paths);
    let names: Vec<String> = processed.iter().map(|(_, m)| m.name.clone()).collect();

    if app.running {
        // Steering: enfileira no turno em andamento (com anexos).
        steer(app, chat, &text, processed).await;
        let note = if names.is_empty() {
            format!("↳ {text}")
        } else {
            format!("↳ {text}  📎 {}", names.join(", "))
        };
        app.commit_notice(note, app.theme.muted);
        return Ok(());
    }

    let theme = app.theme;
    let mut user_lines = super::transcript::render_user(&text, &theme);
    if !names.is_empty() {
        user_lines.push(attachment_pill_line(&names, &theme));
    }
    app.commit(user_lines);
    app.running = true;
    app.status = Status::Working;
    app.thinking = None;
    app.assistant = None;
    app.saw_assistant = false;
    let blocks: Vec<provider::ContentBlock> = processed.into_iter().map(|(b, _)| b).collect();
    start_turn(app, chat, text, blocks).await
}

async fn run_command(app: &mut App, chat: &ChatCtx, name: &str, arg: &str) -> anyhow::Result<()> {
    match name {
        "quit" | "q" | "exit" => app.quit = true,
        "mode" => toggle_mode(app),
        "theme" => {
            if arg.is_empty() {
                let cur = app.theme_kind.as_str().to_string();
                app.overlay = Some(Overlay::Select(Select::new(
                    SelectKind::Theme,
                    "theme",
                    theme_items(&cur),
                    0,
                )));
            } else {
                set_theme(app, arg);
            }
        }
        "effort" => {
            if arg.is_empty() {
                let items = effort_items(&app.effort);
                let sel = items.iter().position(|i| i.value == app.effort).unwrap_or(0);
                app.overlay = Some(Overlay::Select(Select::new(SelectKind::Effort, "effort (thinking)", items, sel)));
            } else {
                app.effort = arg.to_string();
                app.commit_notice(format!("effort → {arg}"), app.theme.accent);
            }
        }
        "model" => {
            if arg.is_empty() {
                let items = model_items(&chat.config, &app.cur_model());
                let sel = items.iter().position(|i| i.value == app.cur_model()).unwrap_or(0);
                app.overlay = Some(Overlay::Select(Select::new(SelectKind::Model, "model", items, sel)));
            } else {
                set_model(app, arg);
                app.commit_notice(format!("model ({}) → {arg}", app.mode.as_str()), app.theme.accent);
            }
        }
        "help" | "hotkeys" | "?" => {
            app.overlay = Some(Overlay::Select(Select::new(SelectKind::Help, "shortcuts", help_items(), 0)));
        }
        "new" => new_session(app, chat).await?,
        "sessions" | "resume" => {
            let root = chat.ws.root.to_string_lossy().to_string();
            match persist::list_sessions(Some(&root)) {
                Ok(sessions) if !sessions.is_empty() => {
                    let items: Vec<SelectItem> = sessions
                        .iter()
                        .map(|s| SelectItem {
                            label: if s.title.is_empty() { "(untitled)".into() } else { s.title.clone() },
                            desc: format!("{} · {} turns", rel_time(s.updated_at), s.turn_count),
                            value: s.session_id.clone(),
                        })
                        .collect();
                    app.overlay = Some(Overlay::Select(Select::new(
                        SelectKind::Sessions,
                        "sessions",
                        items,
                        0,
                    )));
                }
                Ok(_) => app.commit_notice("no saved sessions", app.theme.muted),
                Err(e) => app.commit_notice(format!("could not list sessions: {e}"), app.theme.warning),
            }
        }
        "copy" => {
            if let Some(text) = &app.last_assistant {
                copy_to_clipboard(text);
                app.commit_notice("copied to clipboard", app.theme.success);
            } else {
                app.commit_notice("nothing to copy yet", app.theme.muted);
            }
        }
        "provider" => {
            let parts: Vec<&str> = arg.split_whitespace().collect();
            let theme = app.theme;
            match parts.first().copied() {
                Some("list") => {
                    let cfg = provider::load_config();
                    if cfg.providers.is_empty() {
                        app.commit_notice("Nenhum provider conectado.", theme.muted);
                    } else {
                        for (id, p) in &cfg.providers {
                            let label = p.label.as_deref().unwrap_or(id);
                            app.commit_notice(
                                format!("  {label} ({id}) — {pr}", pr = p.protocol),
                                theme.accent,
                            );
                        }
                    }
                }
                Some("remove") => {
                    let id = parts.get(1).copied().unwrap_or("");
                    if id.is_empty() {
                        app.commit_notice("Use: /provider remove <id>", theme.warning);
                    } else {
                        let mut cfg = provider::load_config();
                        cfg.providers.remove(id);
                        let prefix = format!("{id}/");
                        if cfg.brain_model.starts_with(&prefix) {
                            cfg.brain_model = "claudius".into();
                        }
                        if cfg.builder_model.starts_with(&prefix) {
                            cfg.builder_model = "claudinio".into();
                        }
                        provider::save_config(&cfg);
                        app.commit_notice(format!("Provider '{id}' removido."), theme.success);
                    }
                }
                Some("add") => {
                    app.commit_notice(
                        "Use `claudinio provider add <id> --api-key ...` no terminal.",
                        theme.warning,
                    );
                }
                _ => {
                    app.commit_notice("/provider: add | list | remove", theme.muted);
                }
            }
        }
        "attach" => {
            if arg.is_empty() {
                app.commit_notice(
                    "usage: /attach <path>  (or drag the file into the terminal)",
                    app.theme.muted,
                );
            } else {
                let path = expand_tilde(&unescape_arg(arg));
                if attachments::is_attachable(&path) {
                    let name = attachments::describe(&path)
                        .map(|m| m.name)
                        .unwrap_or_else(|| path.clone());
                    app.attachments.push(path);
                    app.commit_notice(format!("📎 attached: {name}"), app.theme.accent);
                } else {
                    app.commit_notice(format!("file not found: {path}"), app.theme.warning);
                }
            }
        }
        other => {
            app.commit_notice(format!("unknown command: /{other}"), app.theme.warning);
        }
    }
    Ok(())
}

fn toggle_mode(app: &mut App) {
    app.mode = match app.mode {
        SessionMode::Brain => SessionMode::Builder,
        SessionMode::Builder => SessionMode::Brain,
    };
    app.mode_ctl.set(app.mode, ModeOrigin::Human);
}

fn set_theme(app: &mut App, value: &str) {
    app.theme_kind = if value == "light" { ThemeKind::Light } else { ThemeKind::Dark };
    app.theme = Theme::from_kind(app.theme_kind);
    app.editor.restyle(&app.theme);
    app.commit_notice(format!("tema → {}", app.theme_kind.as_str()), app.theme.accent);
}

fn set_model(app: &mut App, value: &str) {
    match app.mode {
        SessionMode::Brain => app.brain_model = value.to_string(),
        SessionMode::Builder => app.builder_model = value.to_string(),
    }
}

fn model_items(config: &AgentConfig, current: &str) -> Vec<SelectItem> {
    let mut ids: Vec<String> = vec![config.brain_model.clone(), config.builder_model.clone()];
    for (pid, entry) in &config.providers {
        if !entry.enabled_models.is_empty() {
            for m in &entry.enabled_models {
                ids.push(format!("{pid}/{m}"));
            }
        } else {
            for m in entry.model_pricing.keys() {
                ids.push(format!("{pid}/{m}"));
            }
        }
    }
    ids.retain(|s| !s.is_empty());
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .map(|id| SelectItem {
            desc: if id == current { "(current)".into() } else { String::new() },
            label: id.clone(),
            value: id,
        })
        .collect()
}

async fn interrupt(app: &mut App, chat: &ChatCtx) {
    if let Some(h) = chat.ws.active_session.lock().await.as_ref() {
        if let Some(s) = chat.maps.steering.lock().await.get(&h.id) {
            s.interrupt.store(true, Ordering::SeqCst);
        }
    }
    // Se havia aprovação pendente, rejeita para desbloquear.
    if let Some(idx) = app.awaiting_idx() {
        decide_approval(app, chat, idx, false).await;
    }
    app.commit_notice("⏹ interrupting…", app.theme.warning);
}

async fn decide_approval(app: &mut App, chat: &ChatCtx, idx: usize, ok: bool) {
    let key = app.tools.get(idx).and_then(|c| c.approval_key.clone());
    if let Some(key) = key {
        if let Some(s) = chat.approvals.lock().await.remove(&key) {
            let _ = s.send(ok);
        }
    }
    if ok {
        if let Some(c) = app.tools.get_mut(idx) {
            c.state = super::transcript::ToolState::Running;
            c.approval_key = None;
        }
    } else {
        // Rejeitado: finaliza o card como erro no scrollback.
        let theme = app.theme;
        if let Some(mut c) = app.tools.get(idx).cloned() {
            c.state = super::transcript::ToolState::Done;
            c.is_error = true;
            c.output = Some("rejected by user".into());
            let lines = super::transcript::render_tool_card(&c, &theme, 60);
            app.commit(lines);
        }
        if idx < app.tools.len() {
            app.tools.remove(idx);
        }
    }
}

async fn steer(
    app: &mut App,
    chat: &ChatCtx,
    text: &str,
    atts: Vec<(provider::ContentBlock, AttachmentMeta)>,
) {
    let _ = app;
    if let Some(h) = chat.ws.active_session.lock().await.as_ref() {
        if let Some(s) = chat.maps.steering.lock().await.get(&h.id) {
            s.push(SteeringEntry {
                text: text.to_string(),
                attachments: atts,
            });
        }
    }
}

async fn new_session(app: &mut App, chat: &ChatCtx) -> anyhow::Result<()> {
    let root = chat.ws.root.to_string_lossy().to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let store = SessionStore::create(&id, Some(&root)).map_err(anyhow::Error::msg)?;
    *chat.ws.active_session.lock().await = Some(SessionHandle {
        id: id.clone(),
        store_path: store.path.clone(),
    });
    let mode_ctl = Arc::new(ModeCtl::new(app.mode, ModeOrigin::Human));
    chat.maps.modes.lock().await.insert(id.clone(), mode_ctl.clone());
    app.mode_ctl = mode_ctl;
    app.in_tok = 0;
    app.out_tok = 0;
    app.cost = None;
    app.context_tokens = 0;
    app.thinking = None;
    app.assistant = None;
    app.tools.clear();
    app.subagents.clear();
    app.tasks.clear();
    app.question = None;
    app.attachments.clear();
    app.running = false;
    app.status = Status::Idle;
    app.commit_notice("── new session ──", app.theme.dim);
    Ok(())
}

/// Extrai o id de sessão do argumento de `-c`: se for um caminho/`.jsonl`, usa o
/// stem; senão, o valor cru (id ou prefixo, resolvido por `resolve_chain`).
fn resume_id_from_arg(v: &str) -> String {
    let p = std::path::Path::new(v);
    if p.extension().and_then(|e| e.to_str()) == Some("jsonl") || v.contains('/') || v.contains('\\') {
        p.file_stem().and_then(|s| s.to_str()).unwrap_or(v).to_string()
    } else {
        v.to_string()
    }
}

/// Modo salvo na sessão (último `Mode` record), com fallback pra Brain.
fn records_mode(records: &[persist::SessionRecord]) -> SessionMode {
    persist::last_mode(records)
        .and_then(|(m, _)| SessionMode::parse(&m))
        .unwrap_or(SessionMode::Brain)
}

/// Título da conversa: a primeira mensagem do usuário (truncada), como na lista.
fn session_title(records: &[persist::SessionRecord]) -> String {
    records
        .iter()
        .find_map(|r| match r {
            persist::SessionRecord::User { text, .. } => Some(text.chars().take(60).collect()),
            _ => None,
        })
        .unwrap_or_else(|| "session".into())
}

fn count_user_turns(records: &[persist::SessionRecord]) -> usize {
    records
        .iter()
        .filter(|r| matches!(r, persist::SessionRecord::User { .. }))
        .count()
}

/// Restaura os contadores do rodapé (tokens/custo/contexto) da sessão carregada.
fn restore_session_stats(app: &mut App, records: &[persist::SessionRecord]) {
    let (in_tok, out_tok, cost, ..) = persist::cumulative_stats(records);
    app.in_tok = in_tok;
    app.out_tok = out_tok;
    if cost.is_some() {
        app.cost = cost;
    }
    if let Some(ctx) = persist::last_context_tokens(records) {
        app.context_tokens = ctx;
    }
}

/// Resultado de ferramenta que representa um erro (mesmo prefixo que o core grava
/// em `session.rs`: `Error: …` / `Error applying: …`).
fn result_is_error(content: &str) -> bool {
    let t = content.trim_start();
    t.starts_with("Error: ") || t.starts_with("Error applying: ")
}

/// Reconstrói o transcript de uma sessão carregada (records → lotes de linhas pro
/// scrollback), reusando os renderizadores do caminho vivo. Edições ganham o diff
/// completo (recomputado de old/new, independe do disco); erros de ferramenta são
/// preservados; saídas bem-sucedidas ficam só no header (igual ao caminho vivo).
fn replay_records(records: &[persist::SessionRecord], theme: &Theme) -> Vec<Vec<Line<'static>>> {
    use super::transcript::{self, ToolState};

    // Pré-indexa os resultados de ferramenta por tool_use_id.
    let mut results: HashMap<String, String> = HashMap::new();
    for rec in records {
        if let persist::SessionRecord::Turn { message, .. } = rec {
            for block in &message.content {
                if let provider::ContentBlock::ToolResult { tool_use_id, content, .. } = block {
                    results.insert(tool_use_id.clone(), content.clone());
                }
            }
        }
    }

    let mut out: Vec<Vec<Line<'static>>> = Vec::new();
    for rec in records {
        match rec {
            persist::SessionRecord::User { text, .. } => {
                out.push(transcript::render_user(text, theme));
            }
            persist::SessionRecord::Turn { message, .. } if message.role == "assistant" => {
                for block in &message.content {
                    match block {
                        provider::ContentBlock::Text { text, .. } => {
                            if !text.trim().is_empty() {
                                out.push(transcript::render_assistant(text, theme));
                            }
                        }
                        provider::ContentBlock::ToolUse { id, name, input, .. } => {
                            let mut card =
                                ToolCard::new(id.clone(), name.clone(), transcript::tool_summary(input));
                            card.state = ToolState::Done;
                            // Diff determinístico pra edições (independe do disco).
                            if name == "edit_file" {
                                if let (Some(old), Some(new)) = (
                                    input.get("old_string").and_then(|v| v.as_str()),
                                    input.get("new_string").and_then(|v| v.as_str()),
                                ) {
                                    card.diff =
                                        Some(claudinio_core::agent::tools::diff_strings(old, new));
                                }
                            }
                            if let Some(res) = results.get(id) {
                                if result_is_error(res) {
                                    card.is_error = true;
                                    card.output = Some(res.clone());
                                }
                            }
                            out.push(transcript::render_tool_card(&card, theme, 0));
                        }
                        _ => {}
                    }
                }
            }
            persist::SessionRecord::LinkedFrom { reason, .. } => {
                out.push(transcript::render_notice(&format!("── linked ({reason}) ──"), theme.dim));
            }
            persist::SessionRecord::Handoff { .. } => {
                out.push(transcript::render_notice("── handoff ──", theme.dim));
            }
            persist::SessionRecord::Compacted { .. } => {
                out.push(transcript::render_notice("── context compacted ──", theme.dim));
            }
            _ => {}
        }
    }
    out
}

/// Reabre uma sessão salva na TUI em execução (via `/sessions`): resolve a cadeia,
/// aponta a sessão ativa pra ponta, restaura modo/stats, reseta o estado vivo e
/// faz replay do histórico. Modelado em `new_session`.
async fn resume_session(app: &mut App, chat: &ChatCtx, session_id: &str) -> anyhow::Result<()> {
    let root = chat.ws.root.to_string_lossy().to_string();
    let (tip_id, tip_path, records) = match persist::resolve_chain(Some(&root), session_id) {
        Ok(t) => t,
        Err(e) => {
            let warn = app.theme.warning;
            app.commit_notice(format!("could not open session: {e}"), warn);
            return Ok(());
        }
    };

    // Limpa a sessão atual se estiver vazia (evita órfãos "(empty session)").
    cleanup_active_if_empty(chat).await;

    *chat.ws.active_session.lock().await = Some(SessionHandle {
        id: tip_id.clone(),
        store_path: tip_path,
    });

    // Restaura o modo salvo.
    let mode = records_mode(&records);
    let mode_ctl = Arc::new(ModeCtl::new(mode, ModeOrigin::Human));
    chat.maps.modes.lock().await.insert(tip_id, mode_ctl.clone());
    app.mode_ctl = mode_ctl;
    app.mode = mode;

    // Reseta o estado vivo (como /new) e restaura os contadores do rodapé.
    app.in_tok = 0;
    app.out_tok = 0;
    app.cost = None;
    app.context_tokens = 0;
    app.max_context_tokens = 0;
    app.thinking = None;
    app.assistant = None;
    app.tools.clear();
    app.subagents.clear();
    app.tasks.clear();
    app.question = None;
    restore_session_stats(app, &records);

    // Replay do histórico + divisor.
    for lines in replay_records(&records, &app.theme) {
        app.commit(lines);
    }
    let dim = app.theme.dim;
    app.commit_notice(
        format!("── resumed: {} · {} turns ──", session_title(&records), count_user_turns(&records)),
        dim,
    );
    Ok(())
}

/// Se a sessão ativa não tem conteúdo real (só meta/mode), remove o arquivo pra
/// não deixar um órfão "(empty session)" na lista.
async fn cleanup_active_if_empty(chat: &ChatCtx) {
    let path = {
        let guard = chat.ws.active_session.lock().await;
        let Some(h) = guard.as_ref() else { return };
        let is_empty = load_records(&h.store_path)
            .map(|recs| {
                !recs.iter().any(|r| {
                    matches!(r, persist::SessionRecord::User { .. } | persist::SessionRecord::Turn { .. })
                })
            })
            .unwrap_or(false);
        if !is_empty {
            return;
        }
        h.store_path.clone()
    };
    let _ = std::fs::remove_file(&path);
    persist::invalidate_cache(&path, &chat.maps.records_cache);
}

/// Formata um epoch-ms como tempo relativo curto ("2h ago") pra lista de sessões.
fn rel_time(ts_ms: u64) -> String {
    let secs = persist::now_ms().saturating_sub(ts_ms) / 1000;
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// Monta o `RunArgs` do turno com os overrides do App e spawna o driver.
async fn start_turn(
    app: &mut App,
    chat: &ChatCtx,
    message: String,
    attachment_blocks: Vec<provider::ContentBlock>,
) -> anyhow::Result<()> {
    let handle = chat
        .ws
        .active_session
        .lock()
        .await
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no active session"))?;
    let store = SessionStore {
        path: handle.store_path.clone(),
    };
    let history = load_records(&handle.store_path)
        .map(|r| persist::history_from_records(&r))
        .unwrap_or_default();

    // Config com overrides de sessão (modelo por modo + effort).
    let mut config = chat.config.clone();
    config.brain_model = app.brain_model.clone();
    config.builder_model = app.builder_model.clone();
    config.thinking_effort = app.effort.clone();

    let steering = Arc::new(SteeringCtl::new());
    chat.maps
        .steering
        .lock()
        .await
        .insert(handle.id.clone(), steering.clone());

    let mcp = chat.ws.ensure_mcp_connected(&config).await;
    let base_commit = claudinio_core::agent::tools::git_head(chat.ws.root.to_string_lossy().as_ref());

    let ctx = ToolContext {
        db_path: Some(chat.ws.index_db_path.to_string_lossy().to_string()),
        lsp_manager: Some(chat.ws.lsp_manager.clone()),
        workspace_root: Some(chat.ws.root.to_string_lossy().to_string()),
        embedding_model: chat.embedding_model.clone(),
        session_store_path: Some(handle.store_path.to_string_lossy().to_string()),
        read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
        interrupt: Some(steering.interrupt.clone()),
        agent_config: Some(config.clone()),
        plan_save_path: config.plan_save_path.clone(),
        base_commit,
        auto_approve_git: false,
        mcp: Some(mcp),
        mode_ctl: Some(app.mode_ctl.clone()),
        index_progress: Some(chat.ws.index_progress.clone()),
        records_cache: chat.maps.records_cache.clone(),
    };

    let chan: EventTx = Arc::new(ChannelSink(chat.agent_tx.clone()));
    let args = RunArgs {
        config,
        ws: chat.ws.clone(),
        maps: chat.maps.clone(),
        approvals: chat.approvals.clone(),
        answers: chat.answers.clone(),
        chan,
        handle,
        store,
        ctx,
        mode_ctl: app.mode_ctl.clone(),
        steering,
        history,
        message,
        attachment_blocks,
    };
    tokio::spawn(run_to_completion(args));
    Ok(())
}

fn cwd_label(root: &std::path::Path) -> String {
    let root_s = root.to_string_lossy().to_string();
    let home = dirs::home_dir().map(|h| h.to_string_lossy().to_string());
    let base = match home {
        Some(h) if root_s.starts_with(&h) => format!("~{}", &root_s[h.len()..]),
        _ => root_s,
    };
    match git_branch(root) {
        Some(b) => format!("{base} ({b})"),
        None => base,
    }
}

fn git_branch(root: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let b = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if b.is_empty() {
        None
    } else {
        Some(b)
    }
}

/// Copia via OSC52 (sem dependência de clipboard nativo).
fn copy_to_clipboard(text: &str) {
    use std::io::Write as _;
    let b64 = base64_encode(text.as_bytes());
    let seq = format!("\x1b]52;c;{b64}\x07");
    let mut out = std::io::stdout();
    let _ = out.write_all(seq.as_bytes());
    let _ = out.flush();
}

fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Verdadeiro se `raw` é um comando de barra (e não um caminho colado tipo
/// `/Users/...`): a barra é seguida de UMA palavra sem `/`.
fn looks_like_command(raw: &str) -> bool {
    if let Some(after) = raw.strip_prefix('/') {
        let first = after.split_whitespace().next().unwrap_or("");
        return !first.is_empty() && !first.contains('/');
    }
    false
}

/// Remove aspas ao redor e desfaz escapes de shell (`\ `), para caminhos
/// arrastados/colados no argumento de `/attach`.
fn unescape_arg(s: &str) -> String {
    let s = strip_quotes(s.trim());
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&n) = chars.peek() {
                out.push(n);
                chars.next();
            } else {
                out.push('\\');
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn strip_quotes(s: &str) -> &str {
    s.strip_prefix('\'')
        .and_then(|x| x.strip_suffix('\''))
        .or_else(|| s.strip_prefix('"').and_then(|x| x.strip_suffix('"')))
        .unwrap_or(s)
}

/// Expande `~/` inicial para o diretório home.
fn expand_tilde(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    s.to_string()
}

/// Tokeniza respeitando escapes `\ ` (arrastar/colar caminhos com espaço no
/// terminal produz `Foo\ Bar.png`).
fn split_escaped(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&n) = chars.peek() {
                cur.push(n);
                chars.next();
            } else {
                cur.push('\\');
            }
            in_token = true;
        } else if c.is_whitespace() {
            if in_token {
                tokens.push(std::mem::take(&mut cur));
                in_token = false;
            }
        } else {
            cur.push(c);
            in_token = true;
        }
    }
    if in_token {
        tokens.push(cur);
    }
    tokens
}

/// Separa anexos (tokens que são caminhos de arquivos existentes) do texto.
/// Retorna (texto_limpo, caminhos_dos_anexos).
fn extract_attachments(msg: &str) -> (String, Vec<String>) {
    let mut kept: Vec<String> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    for tok in split_escaped(msg) {
        let looks_path = tok.starts_with('/')
            || tok.starts_with("~/")
            || tok.starts_with("./")
            || tok.starts_with("../");
        let candidate = expand_tilde(strip_quotes(&tok));
        if looks_path && attachments::is_attachable(&candidate) {
            paths.push(candidate);
        } else {
            kept.push(tok);
        }
    }
    (kept.join(" ").trim().to_string(), paths)
}

/// Linha de pílulas de anexo (`📎 nome  📎 nome`).
fn attachment_pill_line(names: &[String], theme: &Theme) -> Line<'static> {
    let text = names
        .iter()
        .map(|n| format!("📎 {n}"))
        .collect::<Vec<_>>()
        .join("   ");
    Line::from(Span::styled(format!("  {text}"), theme.dim_style()))
}

#[cfg(test)]
impl App {
    /// Constrói um App mínimo para testes de render (sem sessão/TTY).
    pub fn for_test() -> Self {
        let theme = Theme::dark();
        App {
            theme_kind: ThemeKind::Dark,
            theme,
            mode: SessionMode::Brain,
            brain_model: "claudius".into(),
            builder_model: "claudinio".into(),
            effort: "high".into(),
            cwd_label: "~/proj (main)".into(),
            in_tok: 0,
            out_tok: 0,
            cost: None,
            is_sub: false,
            context_tokens: 0,
            max_context_tokens: 0,
            running: false,
            status: Status::Idle,
            spinner_tick: 0,
            retry_deadline: None,
            thinking: None,
            assistant: None,
            saw_assistant: false,
            last_assistant: None,
            tools: Vec::new(),
            subagents: Vec::new(),
            tasks: Vec::new(),
            question: None,
            editor: Editor::new(&theme),
            overlay: None,
            attachments: Vec::new(),
            file_list: Vec::new(),
            to_commit: Vec::new(),
            mode_ctl: Arc::new(ModeCtl::new(SessionMode::Brain, ModeOrigin::Human)),
            quit: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claudinio_core::agent::session::EditProposalData;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Renderiza a região viva num TestBackend e devolve todo o texto da tela.
    fn screen(app: &App) -> String {
        let mut term = Terminal::new(TestBackend::new(80, 18)).unwrap();
        term.draw(|f| render::draw(f, app)).unwrap();
        term.backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn commits_text(app: &App) -> String {
        app.to_commit
            .iter()
            .flat_map(|lines| lines.iter())
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn idle_box_shows_mode_and_hint() {
        let app = App::for_test();
        let s = screen(&app);
        // A caixa (única com borda) tem o modo como título; a dica fica fora dela.
        assert!(s.contains("brain"), "faltou modo no título da caixa: {s:?}");
        assert!(s.contains("commands"), "faltou dica de comandos");
        assert!(s.contains("claudius·high"), "faltou footer com modelo·effort");
    }

    #[test]
    fn streaming_assistant_and_footer_render() {
        let mut app = App::for_test();
        app.running = true;
        event::apply(&mut app, AgentEvent::Thinking("planejando…".into()));
        // Enquanto pensa (antes de qualquer texto): indicador fixo, sem o texto cru.
        let s_think = screen(&app);
        assert!(s_think.contains("Thinking"), "faltou indicador fixo de thinking: {s_think:?}");
        assert!(!s_think.contains("planejando"), "texto cru de thinking não deve aparecer");
        event::apply(
            &mut app,
            AgentEvent::TextDelta {
                text: "resposta **forte**".into(),
            },
        );
        event::apply(
            &mut app,
            AgentEvent::SessionStats {
                input_tokens: 1200,
                output_tokens: 340,
                cumulative_cost: Some(0.014),
                cost_input: None,
                cost_output: None,
                cost_cache_read: None,
                context_tokens: 24_000,
                max_context_tokens: 200_000,
                compact_threshold: 0,
            },
        );
        let s = screen(&app);
        assert!(s.contains("working"), "faltou status de spinner: {s:?}");
        assert!(s.contains("12%/200k"), "faltou % de contexto no footer");
        assert!(s.contains("claudius"), "faltou modelo no footer");
        // O texto de "pensando" NUNCA vai pro scrollback (só o indicador fixo na status line).
        assert!(!commits_text(&app).contains("planejando"), "thinking não deve ser commitado");
        // O texto do assistente vai pro scrollback ao finalizar o passo (não é
        // desenhado na região viva, pra não redimensionar o viewport).
        event::apply(
            &mut app,
            AgentEvent::TextStep {
                text: "resposta **forte**".into(),
            },
        );
        let c = commits_text(&app);
        assert!(c.contains("resposta") && c.contains("forte"), "assistente não commitado: {c:?}");
    }

    #[test]
    fn tool_approval_shows_diff_and_sets_awaiting() {
        let mut app = App::for_test();
        app.running = true;
        let diff = "--- original\n+++ modified\n@@ -1,2 +1,2 @@\n contexto\n-antigo\n+novo\n".to_string();
        event::apply(
            &mut app,
            AgentEvent::ToolCall {
                session_id: "s1".into(),
                tool_id: "t1".into(),
                tool_name: "edit_file".into(),
                args: serde_json::json!({ "path": "src/lib.rs" }),
                permission: "requires_approval".into(),
                edit_proposal: Some(EditProposalData {
                    path: "src/lib.rs".into(),
                    old_string: "antigo".into(),
                    new_string: "novo".into(),
                    unified_diff: diff,
                }),
            },
        );
        assert_eq!(app.awaiting_idx(), Some(0));
        let s = screen(&app);
        assert!(s.contains("edit_file"), "faltou nome da ferramenta: {s:?}");
        assert!(s.contains("approve"), "faltou prompt de aprovação");
        assert!(s.contains("novo"), "faltou linha adicionada do diff");
        assert!(s.contains("+1"), "faltou contagem +add do diff");

        // Resultado finaliza o card → vai para o scrollback e sai da região viva.
        event::apply(
            &mut app,
            AgentEvent::ToolResult {
                tool_id: "t1".into(),
                tool_name: "edit_file".into(),
                output: "ok".into(),
                error: None,
            },
        );
        assert!(app.tools.is_empty(), "card deveria ter finalizado");
        assert!(commits_text(&app).contains("edit_file"), "card não commitado");
    }

    #[test]
    fn plain_tool_commits_header_without_output() {
        let mut app = App::for_test();
        app.running = true;
        // Ferramenta comum (sem edit_proposal): roda e devolve saída textual.
        event::apply(
            &mut app,
            AgentEvent::ToolCall {
                session_id: "s1".into(),
                tool_id: "t9".into(),
                tool_name: "semantic_search".into(),
                args: serde_json::json!({ "query": "release process" }),
                permission: "auto".into(),
                edit_proposal: None,
            },
        );
        event::apply(
            &mut app,
            AgentEvent::ToolResult {
                tool_id: "t9".into(),
                tool_name: "semantic_search".into(),
                output: "{\n  \"mode\": \"lexical-only\",\n  \"results\": []\n}".into(),
                error: None,
            },
        );
        assert!(app.tools.is_empty(), "card deveria ter finalizado");
        let c = commits_text(&app);
        // Só o header (nome + resumo da query) — a saída NÃO é despejada.
        assert!(c.contains("semantic_search"), "faltou header da ferramenta: {c:?}");
        assert!(c.contains("release process"), "faltou resumo da query no header");
        assert!(!c.contains("lexical-only"), "a saída da ferramenta não deve ir pro scrollback");
    }

    #[test]
    fn subagent_nested_calls_are_hidden() {
        let mut app = App::for_test();
        app.running = true;
        event::apply(
            &mut app,
            AgentEvent::SubagentStarted {
                subagent_id: "sub1".into(),
                parent_tool_id: "pt1".into(),
                name: "release-workflow-investigator".into(),
                goal: "Inspect the release workflow files".into(),
                mode: "builder".into(),
            },
        );
        // Chamada de ferramenta aninhada: NÃO deve virar linha no scrollback.
        event::apply(
            &mut app,
            AgentEvent::Subagent {
                subagent_id: "sub1".into(),
                event: Box::new(AgentEvent::ToolCall {
                    session_id: "s1".into(),
                    tool_id: "nt1".into(),
                    tool_name: "read_file".into(),
                    args: serde_json::json!({ "path": "/repo/.github/workflows/release.yml" }),
                    permission: "auto".into(),
                    edit_proposal: None,
                }),
            },
        );
        let c = commits_text(&app);
        assert!(c.contains("release-workflow-investigator"), "faltou início do subagente: {c:?}");
        assert!(!c.contains("read_file"), "chamada aninhada não deve aparecer no scrollback");
        assert!(!c.contains("release.yml"), "caminho da chamada aninhada não deve aparecer");
        // O subagente vivo é indicado na região viva ("está trabalhando").
        let s = screen(&app);
        assert!(s.contains("release-workflow-investigator"), "faltou indicador vivo do subagente: {s:?}");
    }

    #[test]
    fn ask_user_creates_question_and_answers_flow() {
        let mut app = App::for_test();
        app.running = true;
        event::apply(
            &mut app,
            AgentEvent::AskUser {
                session_id: "s1".into(),
                tool_id: "q1".into(),
                questions: serde_json::json!([
                    { "question": "Prosseguir?", "options": ["sim", "não"] }
                ]),
            },
        );
        assert!(app.question.is_some());
        let s = screen(&app);
        assert!(s.contains("Prosseguir?"), "faltou a pergunta: {s:?}");
        assert!(s.contains("1) sim"), "faltou opção numerada");
    }

    #[test]
    fn done_stops_running() {
        let mut app = App::for_test();
        app.running = true;
        app.saw_assistant = false;
        event::apply(
            &mut app,
            AgentEvent::Done {
                stop_reason: "end_turn".into(),
                text_output: "final".into(),
                input_tokens: 10,
                output_tokens: 20,
            },
        );
        assert!(!app.running);
        assert!(matches!(app.status, Status::Idle));
        assert!(commits_text(&app).contains("final"), "texto final não commitado");
    }

    #[test]
    fn command_vs_path_detection() {
        assert!(looks_like_command("/model"));
        assert!(looks_like_command("/attach /Users/x.png"));
        assert!(!looks_like_command("/Users/x/pic.png"), "caminho não é comando");
        assert!(!looks_like_command("oi mundo"));
        assert!(!looks_like_command("/"));
    }

    #[test]
    fn extract_attachments_handles_escaped_path() {
        let dir = std::env::temp_dir().join(format!("tui_att_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("Screen shot.png");
        std::fs::write(&f, b"x").unwrap();
        let escaped = f.to_string_lossy().replace(' ', "\\ ");
        let (cleaned, paths) = extract_attachments(&format!("descreva {escaped} por favor"));
        assert_eq!(paths.len(), 1, "deveria achar 1 anexo");
        assert!(paths[0].ends_with("Screen shot.png"));
        assert_eq!(cleaned, "descreva por favor");
        // Caminho inexistente fica no texto.
        let (c2, p2) = extract_attachments("veja /nao/existe.png aqui");
        assert!(p2.is_empty());
        assert_eq!(c2, "veja /nao/existe.png aqui");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pending_attachment_shows_pill() {
        let mut app = App::for_test();
        app.attachments.push("/tmp/foo/bar.png".into());
        let s = screen(&app);
        assert!(s.contains("attachments:"), "faltou rótulo de anexos: {s:?}");
        assert!(s.contains("bar.png"), "faltou nome do anexo");
    }

    #[test]
    fn mention_query_detects_at_token() {
        assert_eq!(mention_query("olha @Chat"), Some("Chat".into()));
        assert_eq!(mention_query("@"), None, "@ puro não abre o overlay");
        assert_eq!(mention_query("olha @Chat depois"), None);
        assert_eq!(mention_query("sem arroba"), None);
    }

    #[test]
    fn rank_files_prefers_basename() {
        let files = vec![
            "src/lib.rs".to_string(),
            "src/components/ChatPanel.tsx".to_string(),
            "docs/chat.md".to_string(),
        ];
        let r = rank_files("chat", &files, 10);
        assert_eq!(r[0], "docs/chat.md", "basename curto começando com a query vem 1º");
        assert!(r.contains(&"src/components/ChatPanel.tsx".to_string()));
        assert!(!r.contains(&"src/lib.rs".to_string()), "sem match não aparece");
    }

    #[test]
    fn mention_overlay_lists_files() {
        let mut app = App::for_test();
        app.file_list = vec!["src/main.rs".into(), "README.md".into()];
        app.overlay = Some(Overlay::Mention(Mention {
            query: String::new(),
            matches: rank_files("", &app.file_list, 20),
            idx: 0,
        }));
        let s = screen(&app);
        assert!(s.contains("files"), "faltou título arquivos: {s:?}");
        assert!(s.contains("main.rs"), "faltou arquivo listado");
    }

    /// Monta um evento `tasks_set` com a lista de tarefas nos args (como o core
    /// emite: a chamada carrega a lista completa de substituição).
    fn tasks_set_event(tasks: serde_json::Value) -> AgentEvent {
        AgentEvent::ToolCall {
            session_id: "s1".into(),
            tool_id: "tk1".into(),
            tool_name: "tasks_set".into(),
            args: serde_json::json!({ "tasks": tasks }),
            permission: "auto".into(),
            edit_proposal: None,
        }
    }

    #[test]
    fn tasks_set_fills_sticky_panel_and_suppresses_card() {
        let mut app = App::for_test();
        app.running = true;
        event::apply(
            &mut app,
            tasks_set_event(serde_json::json!([
                { "id": "task-0", "title": "Investigar exit_plan_mode", "description": "", "journal": [], "status": "done" },
                { "id": "task-1", "title": "Ligar handoff do builder", "description": "", "journal": [], "status": "doing" },
                { "id": "task-2", "title": "Ler run_to_completion", "description": "", "journal": [], "status": "todo" },
            ])),
        );
        // O painel fixo passa a refletir as tarefas…
        assert_eq!(app.tasks.len(), 3, "app.tasks deveria ter sido populado");
        // …e NÃO vira um card de ferramenta (nem vivo, nem no scrollback).
        assert!(app.tools.is_empty(), "tasks_set não deve empurrar um card");
        let s = screen(&app);
        assert!(s.contains("Tasks"), "faltou o header do painel: {s:?}");
        assert!(s.contains("Ligar handoff do builder"), "faltou a tarefa em andamento no painel");
        assert!(
            s.contains("✓ 1") && s.contains("● 1") && s.contains("○ 1"),
            "faltou a contagem por status: {s:?}"
        );
        assert!(!commits_text(&app).contains("tasks_set"), "o card de tasks_set não deveria commitar");
    }

    #[test]
    fn tasks_panel_orders_doing_before_done() {
        let mut app = App::for_test();
        event::apply(
            &mut app,
            tasks_set_event(serde_json::json!([
                { "id": "a", "title": "TAREFA_DONE", "description": "", "journal": [], "status": "done" },
                { "id": "b", "title": "TAREFA_DOING", "description": "", "journal": [], "status": "doing" },
            ])),
        );
        let s = screen(&app);
        let doing = s.find("TAREFA_DOING").expect("tarefa doing ausente");
        let done = s.find("TAREFA_DONE").expect("tarefa done ausente");
        assert!(doing < done, "em-andamento deveria vir antes de concluída no painel");
    }

    #[test]
    fn tasks_panel_caps_list_with_more_indicator() {
        let mut app = App::for_test();
        let many: Vec<_> = (0..9)
            .map(|i| {
                serde_json::json!({
                    "id": format!("t{i}"), "title": format!("tarefa {i}"),
                    "description": "", "journal": [], "status": "todo"
                })
            })
            .collect();
        event::apply(&mut app, tasks_set_event(serde_json::json!(many)));
        assert_eq!(app.tasks.len(), 9);
        // 9 tarefas, cap 6 → mostra 6 + "+3 more".
        let s = screen(&app);
        assert!(s.contains("+3 more"), "faltou o indicador de overflow: {s:?}");
    }

    #[test]
    fn golden_task_gets_marker() {
        let mut app = App::for_test();
        event::apply(
            &mut app,
            tasks_set_event(serde_json::json!([
                { "id": "golden-0", "title": "Meta dourada", "description": "", "journal": [], "status": "doing" },
            ])),
        );
        let s = screen(&app);
        assert!(s.contains("★"), "tarefa golden deveria ter marcador ★: {s:?}");
    }

    #[test]
    fn tasks_set_requiring_approval_keeps_card() {
        // Defensivo: se algum dia tasks_set exigir aprovação, o painel ainda
        // atualiza, mas o card (que carrega a aprovação) NÃO é suprimido.
        let mut app = App::for_test();
        app.running = true;
        event::apply(
            &mut app,
            AgentEvent::ToolCall {
                session_id: "s1".into(),
                tool_id: "tk9".into(),
                tool_name: "tasks_set".into(),
                args: serde_json::json!({ "tasks": [
                    { "id": "x", "title": "t", "description": "", "journal": [], "status": "todo" }
                ]}),
                permission: "requires_approval".into(),
                edit_proposal: None,
            },
        );
        assert_eq!(app.tasks.len(), 1, "o painel deveria atualizar mesmo exigindo aprovação");
        assert_eq!(app.tools.len(), 1, "o card de aprovação não deve ser suprimido");
    }

    #[test]
    fn malformed_tasks_set_falls_back_to_card() {
        let mut app = App::for_test();
        app.running = true;
        // Sem a chave "tasks" → não parseia; cai no fluxo normal (card genérico).
        event::apply(
            &mut app,
            AgentEvent::ToolCall {
                session_id: "s1".into(),
                tool_id: "tkbad".into(),
                tool_name: "tasks_set".into(),
                args: serde_json::json!({ "wrong": true }),
                permission: "auto".into(),
                edit_proposal: None,
            },
        );
        assert!(app.tasks.is_empty(), "args inválidos não devem popular o painel");
        assert_eq!(app.tools.len(), 1, "deveria cair no card genérico");
    }

    fn batches_text(batches: &[Vec<Line<'static>>]) -> String {
        batches
            .iter()
            .flat_map(|lines| lines.iter())
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn replay_reconstructs_transcript_with_edit_diff_and_error() {
        use claudinio_core::agent::provider::{ContentBlock, Message};
        let assistant = Message {
            role: "assistant".into(),
            content: vec![
                ContentBlock::text("on it"),
                ContentBlock::tool_use(
                    "t1",
                    "edit_file",
                    serde_json::json!({
                        "path": "src/x.rs",
                        "old_string": "let foo = 1;",
                        "new_string": "let bar = 2;",
                    }),
                ),
                ContentBlock::tool_use("t2", "bash", serde_json::json!({ "command": "make" })),
            ],
        };
        // Tool results live in the FOLLOWING user turn (t2 failed).
        let results = Message {
            role: "user".into(),
            content: vec![
                ContentBlock::tool_result("t1", "Edited src/x.rs"),
                ContentBlock::tool_result("t2", "Error: command failed"),
            ],
        };
        let records = vec![
            persist::SessionRecord::User { text: "change x and build".into(), ts: 1 },
            persist::SessionRecord::Turn { message: assistant, ts: 2 },
            persist::SessionRecord::Turn { message: results, ts: 3 },
        ];

        let text = batches_text(&replay_records(&records, &Theme::dark()));
        assert!(text.contains("change x and build"), "user turn replayed: {text}");
        assert!(text.contains("on it"), "assistant text replayed");
        assert!(text.contains("edit_file"), "edit tool header present");
        assert!(text.contains("bar = 2"), "full-fidelity edit diff shows new content: {text}");
        assert!(text.contains("bash"), "bash tool header present");
        assert!(text.contains("command failed"), "failed tool result surfaced as error: {text}");
    }
}
