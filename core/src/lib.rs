//! claudinio-core — backend compartilhado entre o app Tauri e o CLI/TUI.
//!
//! Concentra o harness do agente (`agent`), a inteligência de código/indexação
//! e busca híbrida (`code_intel`), estado (`state`), auth/HTTP e subsistemas de
//! apoio. O app (src-tauri) e o CLI (cli) dependem deste crate e não duplicam
//! lógica: melhorias aqui fluem para os dois.
//!
//! NOTA (Fase 1): este crate ainda depende de `tauri` por conta do tipo
//! `Channel<AgentEvent>` e das emissões de `net_activity`/`askpass`. A Fase 2
//! introduz o `trait EventSink` e abstrai essas emissões para remover o `tauri`
//! daqui por completo.

pub mod agent;
pub mod askpass;
pub mod auth;
pub mod code_intel;
pub mod http;
pub mod lsp;
pub mod net_activity;
pub mod paths;
pub mod procutil;
pub mod run;
pub mod state;
pub mod tasks;
