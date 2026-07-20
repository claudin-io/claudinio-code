//! Adaptadores que ligam os sinks abstratos do core aos canais IPC e eventos
//! do Tauri. O core (`claudinio-core`) não conhece Tauri; estes tipos são a
//! ponte usada apenas pelo app desktop.

use claudinio_core::agent::session::{AgentEvent, EventSink};
use claudinio_core::code_intel::indexer::{IndexProgress, ProgressSink};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter};

/// Encaminha cada `AgentEvent` do harness para o `ipc::Channel` que o frontend
/// passou na invocação — o stream que a UI consome.
pub struct ChannelSink(pub Channel<AgentEvent>);

impl EventSink for ChannelSink {
    fn send(&self, ev: AgentEvent) {
        let _ = self.0.send(ev);
    }
}

/// Reporta progresso de indexação. Emite o evento global `index-progress`
/// (indicador de status) e, opcionalmente, também no `ipc::Channel` da chamada
/// específica de `open_workspace` — preservando o duplo caminho original.
pub struct IndexProgressSink {
    pub app: AppHandle,
    pub channel: Option<Channel<IndexProgress>>,
}

impl ProgressSink for IndexProgressSink {
    fn emit(&self, p: IndexProgress) {
        let _ = self.app.emit("index-progress", p.clone());
        if let Some(ch) = &self.channel {
            let _ = ch.send(p);
        }
    }
}
