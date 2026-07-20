//! `claudinio auth` — autenticação com claudin.io e outros providers.
//! (login/logout via loopback OAuth chegam na sequência da Fase 4.)

use claudinio_core::agent::provider;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum AuthAction {
    /// Faz login (claudinio ou openrouter) via browser.
    Login {
        #[arg(long, default_value = "claudinio")]
        provider: String,
    },
    /// Remove a credencial salva.
    Logout,
    /// Mostra o estado de autenticação atual.
    Status,
}

pub async fn run(action: AuthAction) -> anyhow::Result<()> {
    match action {
        AuthAction::Status => {
            let cfg = provider::load_config();
            if cfg.api_key.is_empty() {
                println!("Não autenticado. Rode `claudinio auth login`.");
            } else {
                println!("Autenticado em {}", cfg.base_url);
            }
            if !cfg.providers.is_empty() {
                println!(
                    "Providers conectados: {}",
                    cfg.providers.keys().cloned().collect::<Vec<_>>().join(", ")
                );
            }
            Ok(())
        }
        AuthAction::Logout => {
            let mut cfg = provider::load_config();
            cfg.api_key.clear();
            provider::save_config(&cfg);
            println!("Credencial claudin.io removida.");
            Ok(())
        }
        AuthAction::Login { provider: which } => match which.as_str() {
            "claudinio" | "claudin.io" => {
                println!("Abrindo o browser para autorizar…");
                let result = claudinio_core::auth::login_claudinio(|url| {
                    // Imprime a URL como fallback caso o browser não abra.
                    println!("Se o browser não abrir, acesse:\n  {url}");
                    open::that(url).map_err(|e| format!("falha ao abrir browser: {e}"))
                })
                .await
                .map_err(anyhow::Error::msg)?;
                let tier = result.tier.map(|t| format!(" ({t})")).unwrap_or_default();
                println!("✓ Autenticado como {}{tier}", result.login);
                Ok(())
            }
            "openrouter" => {
                anyhow::bail!("login openrouter pelo CLI ainda não implementado; use o app por ora.")
            }
            other => anyhow::bail!("provider desconhecido: {other} (use claudinio ou openrouter)"),
        },
    }
}
