//! `claudinio run` — executa um turno brain/builder one-shot, com streaming no
//! stdout. Reaproveita `core::run::run_to_completion` (o MESMO driver com
//! handoff do app), alimentando um sink de terminal.

use crate::model;
use claudinio_core::agent::persist::SessionStore;
use claudinio_core::agent::provider::{self, ContentBlock};
use claudinio_core::agent::session::{
    AgentEvent, AnswerMap, ApprovalMap, EventSink, EventTx, ModeCtl, ModeOrigin, SessionMode,
    SteeringCtl, UserAnswer,
};
use claudinio_core::agent::tools::{self, ReadTracker, ToolContext};
use claudinio_core::agent::transition::{self, TransitionMaps};
use claudinio_core::run::{run_to_completion, RunArgs};
use claudinio_core::state::{SessionHandle, WorkspaceState};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Sink que encaminha cada evento do agente para um canal mpsc, consumido pelo
/// renderer/aprovador do terminal.
struct ChannelSink(mpsc::UnboundedSender<AgentEvent>);

impl EventSink for ChannelSink {
    fn send(&self, ev: AgentEvent) {
        let _ = self.0.send(ev);
    }
}

pub async fn run(
    message: String,
    mode: String,
    path: Option<String>,
    yes: bool,
) -> anyhow::Result<()> {
    let ws_root = model::resolve_workspace(path)?;
    let root = ws_root.to_string_lossy().to_string();

    let mut config = provider::load_config();
    if config.api_key.is_empty() {
        anyhow::bail!(
            "API key não configurada. Rode `claudinio config set api_key <key>` (ou `auth login`)."
        );
    }
    if let Some(ws_cfg) = provider::read_workspace_config(&root) {
        provider::merge_workspace_config(&mut config, &ws_cfg);
    }

    let mode = SessionMode::parse(&mode)
        .ok_or_else(|| anyhow::anyhow!("modo inválido: use `brain` ou `builder`"))?;

    // Workspace + índice machine-local (abre/cria o DB).
    let db_path = model::index_db_path(&ws_root);
    if let Some(p) = db_path.parent() {
        std::fs::create_dir_all(p).ok();
    }
    let ws = Arc::new(
        WorkspaceState::open(ws_root.clone(), db_path.clone()).map_err(anyhow::Error::msg)?,
    );

    // Sessão nova persistida no JSONL do workspace.
    let id = uuid::Uuid::new_v4().to_string();
    let store = SessionStore::create(&id, Some(&root)).map_err(anyhow::Error::msg)?;
    let handle = SessionHandle {
        id: id.clone(),
        store_path: store.path.clone(),
    };

    let mode_ctl = Arc::new(ModeCtl::new(mode, ModeOrigin::Human));
    let steering = Arc::new(SteeringCtl::new());
    let approvals: ApprovalMap = Arc::new(Mutex::new(HashMap::new()));
    let answers: AnswerMap = Arc::new(Mutex::new(HashMap::new()));

    // Mapas de transição (para o handoff): registram esta sessão.
    let steering_map: Arc<Mutex<HashMap<String, Arc<SteeringCtl>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    steering_map.lock().await.insert(id.clone(), steering.clone());
    let modes_map: Arc<Mutex<HashMap<String, Arc<ModeCtl>>>> = Arc::new(Mutex::new(HashMap::new()));
    modes_map.lock().await.insert(id.clone(), mode_ctl.clone());
    let records_cache = transition::new_records_cache();
    let maps = TransitionMaps {
        steering: steering_map,
        modes: modes_map,
        records_cache: records_cache.clone(),
    };

    let mcp = ws.ensure_mcp_connected(&config).await;
    let base_commit = tools::git_head(&root);

    // Slot de embeddings vazio: a busca semântica do agente degrada para BM25.
    // Rode `claudinio index` antes para a perna vetorial.
    let embedding_model = Arc::new(Mutex::new(None));

    let ctx = ToolContext {
        db_path: Some(db_path.to_string_lossy().to_string()),
        lsp_manager: Some(ws.lsp_manager.clone()),
        workspace_root: Some(root.clone()),
        embedding_model,
        session_store_path: Some(store.path.to_string_lossy().to_string()),
        read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
        interrupt: Some(steering.interrupt.clone()),
        agent_config: Some(config.clone()),
        plan_save_path: config.plan_save_path.clone(),
        base_commit,
        auto_approve_git: yes,
        mcp: Some(mcp),
        mode_ctl: Some(mode_ctl.clone()),
        index_progress: Some(ws.index_progress.clone()),
        records_cache,
    };

    // Sink → mpsc → renderer/aprovador (task separada: a aprovação de ferramenta
    // é cross-task — o agente insere o oneshot, emite o evento e aguarda).
    let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
    let chan: EventTx = Arc::new(ChannelSink(tx));
    let approvals_c = approvals.clone();
    let answers_c = answers.clone();
    let renderer = tokio::spawn(async move {
        let mut state = RenderState::default();
        while let Some(ev) = rx.recv().await {
            render_event(ev, &approvals_c, &answers_c, yes, &mut state).await;
        }
    });

    let args = RunArgs {
        config,
        ws,
        maps,
        approvals,
        answers,
        chan,
        handle,
        store,
        ctx,
        mode_ctl,
        steering,
        history: Vec::new(),
        message,
        attachment_blocks: Vec::<ContentBlock>::new(),
    };

    // Roda até o fim seguindo handoffs. Ao retornar, `chan` (dentro de `args`) é
    // dropado → o renderer encerra.
    run_to_completion(args).await;
    let _ = renderer.await;
    Ok(())
}

/// Estado do renderer entre eventos: o `Thinking` chega como snapshot acumulado,
/// então guardamos o tamanho já impresso para streamar só o delta.
#[derive(Default)]
struct RenderState {
    thinking_len: usize,
    in_thinking: bool,
    saw_text: bool,
}

/// Fecha o bloco de "pensando" (newline) ao sair dele.
fn end_thinking(state: &mut RenderState) {
    if state.in_thinking {
        eprintln!("\x1b[0m");
        state.in_thinking = false;
        state.thinking_len = 0;
    }
}

async fn render_event(
    ev: AgentEvent,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    yes: bool,
    state: &mut RenderState,
) {
    match ev {
        AgentEvent::Thinking(t) => {
            let delta = t.get(state.thinking_len..).unwrap_or("");
            if !delta.is_empty() {
                if !state.in_thinking {
                    eprint!("\x1b[2m  ");
                    state.in_thinking = true;
                }
                use std::io::Write;
                eprint!("{delta}");
                let _ = std::io::stderr().flush();
            }
            state.thinking_len = t.len();
            return;
        }
        AgentEvent::TextStep { text } => {
            end_thinking(state);
            if !text.trim().is_empty() {
                state.saw_text = true;
                println!("\n{text}");
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
            end_thinking(state);
            println!("\x1b[36m▸ {tool_name}\x1b[0m {}", tool_summary(&args));
            if permission == "requires_approval" {
                let approve = if yes {
                    true
                } else {
                    prompt_yes_no(&format!("  aprovar `{tool_name}`?"))
                };
                let key = format!("{session_id}:{tool_id}");
                if let Some(s) = approvals.lock().await.remove(&key) {
                    let _ = s.send(approve);
                }
            }
        }
        AgentEvent::ToolResult { output, error, .. } => match error {
            Some(e) => println!("  \x1b[31m✗ {}\x1b[0m", first_line(&e)),
            None => {
                let o = first_line(&output);
                if !o.is_empty() {
                    println!("  \x1b[2m{o}\x1b[0m");
                }
            }
        },
        AgentEvent::AskUser {
            session_id,
            tool_id,
            questions,
        } => {
            let ans = answer_questions(&questions, yes);
            let key = format!("{session_id}:{tool_id}");
            if let Some(s) = answers.lock().await.remove(&key) {
                let _ = s.send(ans);
            }
        }
        AgentEvent::SessionLinked { .. } => {
            end_thinking(state);
            println!("\n\x1b[33m⇄ handoff → nova sessão encadeada\x1b[0m");
        }
        AgentEvent::SubagentStarted { name, .. } => {
            end_thinking(state);
            println!("  \x1b[35m⟳ subagente: {name}\x1b[0m");
        }
        AgentEvent::Done {
            stop_reason,
            text_output,
            input_tokens,
            output_tokens,
            ..
        } => {
            end_thinking(state);
            // Fallback: se o texto final não veio como TextStep (só como delta),
            // imprime o texto acumulado do Done.
            if !state.saw_text {
                let t = text_output.trim();
                if !t.is_empty() {
                    println!("\n{t}");
                }
            }
            eprintln!(
                "\x1b[2m— fim ({stop_reason}) · {input_tokens} in / {output_tokens} out\x1b[0m"
            );
        }
        AgentEvent::Error(e) => {
            end_thinking(state);
            eprintln!("\n\x1b[31mErro: {e}\x1b[0m");
        }
        _ => {}
    }
}

fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() > 200 {
        format!("{}…", line.chars().take(200).collect::<String>())
    } else {
        line.to_string()
    }
}

fn tool_summary(args: &Value) -> String {
    for key in ["path", "command", "query", "file_path", "pattern"] {
        if let Some(v) = args.get(key).and_then(|v| v.as_str()) {
            return first_line(v);
        }
    }
    String::new()
}

/// Sem `--yes`: pergunta y/n no terminal (bloqueante). Vazio/`n` = rejeita.
fn prompt_yes_no(prompt: &str) -> bool {
    use std::io::Write;
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes" | "s" | "sim")
}

/// Responde a um `AskUser`. Sem interação (ou `--yes`) manda respostas vazias
/// para o agente prosseguir; caso contrário lê uma linha por pergunta.
fn answer_questions(questions: &Value, yes: bool) -> Vec<UserAnswer> {
    let items = questions.as_array().cloned().unwrap_or_default();
    items
        .iter()
        .map(|q| {
            let question = q
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let answer = if yes {
                String::new()
            } else {
                use std::io::Write;
                print!("  {question}\n  > ");
                let _ = std::io::stdout().flush();
                let mut line = String::new();
                let _ = std::io::stdin().read_line(&mut line);
                line.trim().to_string()
            };
            UserAnswer { question, answer }
        })
        .collect()
}
