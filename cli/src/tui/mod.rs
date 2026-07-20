//! TUI de chat interativa (`claudinio chat`). Modelo de render inline +
//! scrollback (estilo `earendil-works/pi`): blocos finalizados vão para o
//! scrollback nativo via `Terminal::insert_before`; a região viva (bloco em
//! streaming, cards, footer, editor, overlays) é redesenhada num
//! `Viewport::Inline`. Consome o mesmo `AgentEvent`/`EventSink` do app Tauri —
//! nenhuma mudança no core.

pub mod app;
pub mod diff;
pub mod editor;
pub mod event;
pub mod footer;
pub mod markdown;
pub mod overlays;
pub mod render;
pub mod theme;
pub mod transcript;

pub use app::run;
