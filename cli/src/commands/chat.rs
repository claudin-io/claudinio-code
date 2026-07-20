//! `claudinio chat` — TUI interativa. A implementação vive em `crate::tui`
//! (render inline + scrollback estilo `earendil-works/pi`, paridade de
//! experiência com o app desktop). Este módulo só delega para preservar o
//! ponto de despacho `commands::chat::run` usado pelo `main`.

pub async fn run(path: Option<String>) -> anyhow::Result<()> {
    crate::tui::run(path).await
}
