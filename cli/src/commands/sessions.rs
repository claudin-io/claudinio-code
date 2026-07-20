//! `claudinio sessions` — lista sessões salvas (JSONL append-only).

use claudinio_core::agent::persist;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum SessionsAction {
    /// Lista as sessões de um workspace (ou globais, se sem --path).
    List {
        #[arg(long)]
        path: Option<String>,
    },
}

pub fn run(action: SessionsAction) -> anyhow::Result<()> {
    match action {
        SessionsAction::List { path } => {
            let sessions = persist::list_sessions(path.as_deref()).map_err(anyhow::Error::msg)?;
            if sessions.is_empty() {
                println!("Nenhuma sessão.");
                return Ok(());
            }
            for s in &sessions {
                // SessionSummary é Serialize; imprime em JSON compacto — robusto
                // a mudanças de campo sem acoplar o CLI ao formato interno.
                println!("{}", serde_json::to_string(s).unwrap_or_default());
            }
            Ok(())
        }
    }
}
