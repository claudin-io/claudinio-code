//! Driver de execução de sessão, compartilhado pelo app e pelo CLI.
//!
//! `drive` roda uma sessão até o fim seguindo handoffs: sempre que
//! `run_workflow` retorna `RunOutcome::Handoff`, uma sessão sucessora encadeada
//! é criada e o loop continua nela com histórico novo — no MESMO `EventTx`, de
//! modo que o frontend renderiza uma conversa contínua. É o ponto único que
//! liga brain→builder e o handoff por limite de contexto; nenhum frontend o
//! duplica.

use crate::agent::persist::{now_ms, SessionRecord, SessionStore};
use crate::agent::provider::{AgentConfig, ContentBlock, Message};
use crate::agent::session::{
    self, AgentEvent, AnswerMap, ApprovalMap, EventTx, ModeCtl, RunOutcome, SteeringCtl,
};
use crate::agent::transition::{self, TransitionMaps};
use crate::state::{SessionHandle, WorkspaceState};
use std::sync::Arc;

/// Tudo que o loop de execução precisa. Todos os tipos são do core, então tanto
/// o app Tauri quanto o CLI montam esta struct e chamam [`drive`].
pub struct RunArgs {
    pub config: AgentConfig,
    pub ws: Arc<WorkspaceState>,
    pub maps: TransitionMaps,
    pub approvals: ApprovalMap,
    pub answers: AnswerMap,
    pub chan: EventTx,
    pub handle: SessionHandle,
    pub store: SessionStore,
    pub ctx: crate::agent::tools::ToolContext,
    pub mode_ctl: Arc<ModeCtl>,
    pub steering: Arc<SteeringCtl>,
    pub history: Vec<Message>,
    pub message: String,
    pub attachment_blocks: Vec<ContentBlock>,
}

/// Spawna o loop de execução numa task tokio e retorna imediatamente (usado
/// pelo app). O stream de eventos sai por `args.chan` (o `EventTx`).
pub fn drive(args: RunArgs) {
    tokio::spawn(run_to_completion(args));
}

/// Roda o loop de execução até o fim na task atual (usado pelo CLI one-shot,
/// que precisa aguardar a conclusão). Segue handoffs como o [`drive`].
pub async fn run_to_completion(args: RunArgs) {
    let RunArgs {
        config: cfg,
        ws,
        maps,
        approvals: appr,
        answers: answ,
        chan,
        mut handle,
        mut store,
        mut ctx,
        mut mode_ctl,
        steering,
        mut history,
        mut message,
        mut attachment_blocks,
    } = args;

    loop {
            let msg = std::mem::take(&mut message);
            let atts = std::mem::take(&mut attachment_blocks);
            match session::run_workflow(
                &cfg, &mut history, msg, atts, &chan, &appr, &answ, &handle.id, &ctx, &store,
                &steering, &mode_ctl,
            )
            .await
            {
                Ok(RunOutcome::Completed) => break,
                Ok(RunOutcome::Handoff(spec)) => {
                    let mut spec = *spec;
                    spec.first_message = transition::resolve_first_message(
                        &spec,
                        ctx.workspace_root.as_deref(),
                        ctx.plan_save_path.as_deref(),
                    );
                    match transition::link_session(&maps, &ws, &handle, &spec, &chan).await {
                        Ok(new_handle) => {
                            let new_mode_ctl = maps
                                .modes
                                .lock()
                                .await
                                .get(&new_handle.id)
                                .cloned()
                                .unwrap_or_else(|| {
                                    Arc::new(ModeCtl::new(spec.next_mode, spec.next_origin))
                                });
                            ctx = transition::rebuild_tool_context(
                                &ctx,
                                &new_handle.store_path,
                                new_mode_ctl.clone(),
                                cfg.clone(),
                            );
                            mode_ctl = new_mode_ctl;
                            store = SessionStore {
                                path: new_handle.store_path.clone(),
                            };
                            history = Vec::new();
                            message = spec.first_message;
                            handle = new_handle;
                        }
                        Err(e) => {
                            store.try_append(&SessionRecord::Error {
                                message: e.clone(),
                                ts: now_ms(),
                            });
                            let _ = chan.send(AgentEvent::Error(e));
                            break;
                        }
                    }
                }
                Err(e) => {
                    store.try_append(&SessionRecord::Error {
                        message: e.clone(),
                        ts: now_ms(),
                    });
                    let _ = chan.send(AgentEvent::Error(e));
                    break;
                }
            }
        }
        // Run finished (success, error or panic-free return): drop the steering
        // entry so interrupt/steer report "session not running". `handle.id`
        // tracks the FINAL session of the chain — link_session moved the
        // steering entry along with each handoff.
    let mut map = maps.steering.lock().await;
    map.remove(&handle.id);
}
