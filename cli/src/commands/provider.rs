use claudinio_core::agent::provider::{self, ProviderEntry};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum ProviderAction {
    /// Add a manual provider (DeepSeek, Anthropic, etc.) via API key.
    Add {
        /// Provider id (e.g. "deepseek", "my-custom")
        id: String,
        /// API key (omit for interactive prompt without echo)
        #[arg(long)]
        api_key: Option<String>,
        /// API base URL (omit for interactive prompt)
        #[arg(long)]
        base_url: Option<String>,
        /// Wire protocol: "openai" (default) or "anthropic"
        #[arg(long, default_value = "openai")]
        protocol: String,
        /// Display label (defaults to provider id if omitted)
        #[arg(long)]
        label: Option<String>,
    },
    /// List connected providers.
    List,
    /// Remove a connected provider; model slots fall back to Claudinio defaults.
    Remove { id: String },
}

pub async fn run(action: ProviderAction) -> anyhow::Result<()> {
    match action {
        ProviderAction::List => run_list().await,
        ProviderAction::Remove { id } => run_remove(&id).await,
        ProviderAction::Add { id, api_key, base_url, protocol, label } => {
            run_add(&id, api_key, base_url, &protocol, label).await
        }
    }
}

async fn run_list() -> anyhow::Result<()> {
    let cfg = provider::load_config();
    if cfg.providers.is_empty() {
        println!("Nenhum provider conectado.");
        return Ok(());
    }
    println!("Providers conectados:");
    for (id, p) in &cfg.providers {
        let label = p.label.as_deref().unwrap_or(id);
        let models = p.model_pricing.len();
        println!("  {label} ({id}) — {} · {models} modelos", p.protocol);
    }
    Ok(())
}

async fn run_remove(id: &str) -> anyhow::Result<()> {
    let mut cfg = provider::load_config();
    if !cfg.providers.contains_key(id) {
        anyhow::bail!("Provider '{id}' não encontrado.");
    }
    cfg.providers.remove(id);
    let prefix = format!("{id}/");
    if cfg.brain_model.starts_with(&prefix) {
        cfg.brain_model = "claudius".into();
    }
    if cfg.builder_model.starts_with(&prefix) {
        cfg.builder_model = "claudinio".into();
    }
    provider::save_config(&cfg);
    println!("Provider '{id}' removido.");
    Ok(())
}

async fn run_add(
    id: &str,
    api_key_opt: Option<String>,
    base_url_opt: Option<String>,
    protocol: &str,
    label_opt: Option<String>,
) -> anyhow::Result<()> {
    // Resolve API key: flag ou prompt
    let api_key = match api_key_opt {
        Some(k) => k,
        None => {
            eprint!("API key para '{id}': ");
            let mut buf = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
            buf.trim().to_string()
        }
    };
    if api_key.trim().is_empty() {
        anyhow::bail!("API key é obrigatória.");
    }

    // Resolve base URL: flag ou prompt
    let base_url = match base_url_opt {
        Some(u) => u,
        None => {
            eprint!("Base URL (ex: https://api.deepseek.com): ");
            let mut buf = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
            buf.trim().to_string()
        }
    };
    if base_url.trim().is_empty() {
        anyhow::bail!("Base URL é obrigatória.");
    }

    // Validar protocolo
    match protocol {
        "openai" | "anthropic" => {}
        other => anyhow::bail!("Protocolo inválido: {other} (use openai ou anthropic)"),
    }

    let label = label_opt.filter(|l| !l.is_empty()).unwrap_or_else(|| id.to_string());

    let mut cfg = provider::load_config();

    // Se já existe, confirmar overwrite
    if cfg.providers.contains_key(id) {
        eprint!("Provider '{id}' já existe. Sobrescrever? [s/N]: ");
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
        if !buf.trim().eq_ignore_ascii_case("s") {
            println!("Abortado.");
            return Ok(());
        }
    }

    cfg.providers.insert(
        id.to_string(),
        ProviderEntry {
            api_key: api_key.trim().to_string(),
            base_url: base_url.trim().to_string(),
            protocol: protocol.to_string(),
            enabled_models: Vec::new(),
            label: Some(label),
            model_pricing: std::collections::HashMap::new(),
            model_output_limits: std::collections::HashMap::new(),
        },
    );
    provider::save_config(&cfg);
    println!("✓ Provider '{id}' adicionado. Use `claudinio chat` e modelos com prefixo '{id}/'.");
    Ok(())
}
