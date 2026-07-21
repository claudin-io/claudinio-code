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
    let ws = Arc::new(WorkspaceState::open(ws_root.clone(), db_path.clone()).map_err(anyhow::Error::msg)?);

    // Sessão nova.
    let id = uuid::Uuid::new_v4().to_string();
    let store = SessionStore::create(&id, Some(&root)).map_err(anyhow::Error::msg)?;
    *ws.active_session.lock().await = Some(SessionHandle {
        id: id.clone(),
        store_path: store.path.clone(),
    });

    let mode_ctl = Arc::new(ModeCtl::new(SessionMode::Brain, ModeOrigin::Human));
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

    // Lista de arquivos do workspace (respeitando .gitignore) para o @-mention.
    let file_list = claudinio_core::code_intel::list_files(&root, 5000);

    let mut app = App {
        theme_kind: ThemeKind::Dark,
        theme,
        mode: SessionMode::Brain,
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
        format!("claudinio chat — {root}   ·  Tab: modo · / comandos · Ctrl+C: sair"),
        app.theme.dim,
    );

    // Terminal inline (sem alt-screen: preserva scrollback). Altura DINÂMICA: só
    // o cromo (input+status+footer) quando ocioso — SEM buraco — e cresce pra
    // caber conteúdo/overlays (o loop redimensiona recriando o viewport).
    let init_vh = render::chrome_height(&app);
    let mut terminal = ratatui::try_init_with_options(TerminalOptions {
        viewport: Viewport::Inline(init_vh),
    })
    .map_err(|e| {
        anyhow::anyhow!("não foi possível inicializar a TUI (é preciso um terminal interativo): {e}")
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
                    "tema",
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
                app.overlay = Some(Overlay::Select(Select::new(SelectKind::Model, "modelo", items, sel)));
            } else {
                set_model(app, arg);
                app.commit_notice(format!("modelo ({}) → {arg}", app.mode.as_str()), app.theme.accent);
            }
        }
        "help" | "hotkeys" | "?" => {
            app.overlay = Some(Overlay::Select(Select::new(SelectKind::Help, "atalhos", help_items(), 0)));
        }
        "new" => new_session(app, chat).await?,
        "copy" => {
            if let Some(text) = &app.last_assistant {
                copy_to_clipboard(text);
                app.commit_notice("copiado para o clipboard", app.theme.success);
            } else {
                app.commit_notice("nada para copiar ainda", app.theme.muted);
            }
        }
        "attach" | "anexar" => {
            if arg.is_empty() {
                app.commit_notice(
                    "uso: /attach <caminho>  (ou arraste o arquivo para o terminal)",
                    app.theme.muted,
                );
            } else {
                let path = expand_tilde(&unescape_arg(arg));
                if attachments::is_attachable(&path) {
                    let name = attachments::describe(&path)
                        .map(|m| m.name)
                        .unwrap_or_else(|| path.clone());
                    app.attachments.push(path);
                    app.commit_notice(format!("📎 anexado: {name}"), app.theme.accent);
                } else {
                    app.commit_notice(format!("arquivo não encontrado: {path}"), app.theme.warning);
                }
            }
        }
        other => {
            app.commit_notice(format!("comando desconhecido: /{other}"), app.theme.warning);
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
            desc: if id == current { "(atual)".into() } else { String::new() },
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
    app.commit_notice("⏹ interrompendo…", app.theme.warning);
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
            c.output = Some("rejeitado pelo usuário".into());
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
    app.question = None;
    app.commit_notice("── nova sessão ──", app.theme.dim);
    Ok(())
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
        .ok_or_else(|| anyhow::anyhow!("sessão ativa ausente"))?;
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
        assert!(s.contains("comandos"), "faltou dica de comandos");
        assert!(s.contains("claudius·high"), "faltou footer com modelo·effort");
    }

    #[test]
    fn streaming_assistant_and_footer_render() {
        let mut app = App::for_test();
        app.running = true;
        event::apply(&mut app, AgentEvent::Thinking("planejando…".into()));
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
        assert!(s.contains("trabalhando"), "faltou status de spinner: {s:?}");
        assert!(s.contains("12%/200k"), "faltou % de contexto no footer");
        assert!(s.contains("claudius"), "faltou modelo no footer");
        // O bloco de "pensando" foi commitado ao scrollback quando o texto começou.
        assert!(commits_text(&app).contains("planejando"), "thinking não commitado");
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
        assert!(s.contains("aprovar"), "faltou prompt de aprovação");
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
        assert!(s.contains("anexos:"), "faltou rótulo de anexos: {s:?}");
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
        assert!(s.contains("arquivos"), "faltou título arquivos: {s:?}");
        assert!(s.contains("main.rs"), "faltou arquivo listado");
    }
}
