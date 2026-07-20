//! Claudinio Code — CLI. Reaproveita o backend `claudinio-core` (o mesmo do app
//! Tauri): indexação, busca híbrida, brain/builder, handoff, auth. Sem webview,
//! sem JS. Ver docs/plans/cli-tui.md.

mod model;
mod commands;
mod tui;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "claudinio",
    version,
    about = "Claudinio Code — agente de código no terminal (brain/builder, busca semântica)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Lê/escreve configuração (modelos, chave, effort, base_url).
    Config {
        #[command(subcommand)]
        action: commands::config::ConfigAction,
    },
    /// Lista os modelos disponíveis para brain/builder.
    Models,
    /// Indexa um workspace (símbolos + embeddings) para busca semântica.
    Index {
        /// Diretório do workspace (padrão: diretório atual).
        path: Option<String>,
    },
    /// Busca híbrida (BM25 + vetorial) no índice do workspace.
    Search {
        /// Texto da consulta.
        query: String,
        /// Diretório do workspace (padrão: diretório atual).
        #[arg(long)]
        path: Option<String>,
        /// Número máximo de resultados.
        #[arg(long, default_value_t = 10)]
        limit: i64,
    },
    /// Executa um turno brain/builder (one-shot, streaming no stdout).
    Run {
        /// Mensagem para o agente.
        message: String,
        /// Modo: brain (planeja) ou builder (executa).
        #[arg(short, long, default_value = "brain")]
        mode: String,
        /// Diretório do workspace (padrão: diretório atual).
        #[arg(long)]
        path: Option<String>,
        /// Auto-aprova todas as chamadas de ferramenta.
        #[arg(long)]
        yes: bool,
    },
    /// TUI de chat interativa (brain/builder com handoff).
    Chat {
        /// Diretório do workspace (padrão: diretório atual).
        #[arg(long)]
        path: Option<String>,
    },
    /// Autenticação com claudin.io e outros providers.
    Auth {
        #[command(subcommand)]
        action: commands::auth::AuthAction,
    },
    /// Lista/inspeciona sessões salvas.
    Sessions {
        #[command(subcommand)]
        action: commands::sessions::SessionsAction,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Chat { path: None }) {
        Command::Config { action } => commands::config::run(action),
        Command::Models => commands::config::run_models().await,
        Command::Index { path } => commands::index::run(path).await,
        Command::Search { query, path, limit } => commands::search::run(query, path, limit).await,
        Command::Run { message, mode, path, yes } => commands::run::run(message, mode, path, yes).await,
        Command::Chat { path } => commands::chat::run(path).await,
        Command::Auth { action } => commands::auth::run(action).await,
        Command::Sessions { action } => commands::sessions::run(action),
    }
}
