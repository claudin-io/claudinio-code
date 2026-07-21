//! `claudinio auth` — autenticação com claudin.io e outros providers.
//! (login/logout via loopback OAuth chegam na sequência da Fase 4.)

use claudinio_core::agent::provider::{self, ProviderEntry};
use base64::Engine;
use sha2::{Digest, Sha256};
use claudinio_core::auth::{random_hex, wait_for_callback};
use serde_json::Value;
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
                println!("Abrindo browser para autorizar OpenRouter…");
                let key = openrouter_login_cli()
                    .await
                    .map_err(|e| anyhow::anyhow!("OpenRouter login falhou: {e}"))?;
                println!("✓ Chave OpenRouter obtida: sk-or-…{}", &key[key.len()-4..]);

                let mut cfg = provider::load_config();
                cfg.providers.insert(
                    "openrouter".to_string(),
                    ProviderEntry {
                        api_key: key,
                        base_url: "https://openrouter.ai/api/v1".to_string(),
                        protocol: "openai".into(),
                        enabled_models: Vec::new(),
                        label: Some("OpenRouter".into()),
                        model_pricing: std::collections::HashMap::new(),
                        model_output_limits: std::collections::HashMap::new(),
                    },
                );
                provider::save_config(&cfg);
                println!("✓ OpenRouter configurado e salvo. Use `claudinio chat` e `/model openrouter/...`.");
                Ok(())
            }
            other => anyhow::bail!("provider desconhecido: {other} (use claudinio ou openrouter)"),
        },
    }
}

async fn openrouter_login_cli() -> Result<String, String> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("falha ao iniciar listener local: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("falha ao ler porta: {e}"))?
        .port();

    let verifier = random_hex(32);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));

    let authorize_url = format!(
        "https://openrouter.ai/auth?callback_url=http%3A%2F%2F127.0.0.1%3A{port}%2Fcallback&code_challenge={challenge}&code_challenge_method=S256"
    );

    println!("Se o browser não abrir, acesse:\n  {authorize_url}");
    open::that(&authorize_url).map_err(|e| format!("falha ao abrir browser: {e}"))?;

    let code = wait_for_callback(listener, None).await?;

    let client = claudinio_core::http::default_client();
    let resp = client
        .post("https://openrouter.ai/api/v1/auth/keys")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "code": code,
            "code_verifier": verifier,
            "code_challenge_method": "S256",
        }))
        .send()
        .await
        .map_err(|e| format!("troca de chave OpenRouter falhou: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("OpenRouter key exchange falhou (HTTP {status}): {body}"));
    }

    let parsed: Value = resp
        .json()
        .await
        .map_err(|e| format!("resposta inválida do OpenRouter: {e}"))?;
    let key = parsed
        .get("key")
        .and_then(|k| k.as_str())
        .ok_or("resposta OpenRouter não contém 'key'")?
        .to_string();

    Ok(key)
}
