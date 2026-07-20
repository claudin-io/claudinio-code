//! `claudinio config` — lê/escreve a configuração global (mesma do app, em
//! `<config_dir>/claudinio-code/config.json`).

use claudinio_core::agent::provider::{self, AgentConfig};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Mostra a configuração atual (api_key mascarada).
    Show,
    /// Lê um campo: brain_model, builder_model, effort, base_url, api_key, services_url.
    Get { key: String },
    /// Define um campo.
    Set { key: String, value: String },
}

pub fn run(action: ConfigAction) -> anyhow::Result<()> {
    let mut cfg = provider::load_config();
    match action {
        ConfigAction::Show => {
            println!("brain_model   = {}", cfg.brain_model);
            println!("builder_model = {}", cfg.builder_model);
            println!("effort        = {}", cfg.thinking_effort);
            println!("base_url      = {}", cfg.base_url);
            println!("services_url  = {}", cfg.services_url);
            println!("api_key       = {}", mask(&cfg.api_key));
            if !cfg.providers.is_empty() {
                println!("providers     = {}", cfg.providers.keys().cloned().collect::<Vec<_>>().join(", "));
            }
        }
        ConfigAction::Get { key } => println!("{}", get_field(&cfg, &key)?),
        ConfigAction::Set { key, value } => {
            set_field(&mut cfg, &key, value)?;
            provider::save_config(&cfg);
            println!("ok");
        }
    }
    Ok(())
}

/// `claudinio models` — lista os modelos configurados para brain/builder.
pub async fn run_models() -> anyhow::Result<()> {
    let cfg = provider::load_config();
    println!("brain   → {}", cfg.brain_model);
    println!("builder → {}", cfg.builder_model);
    Ok(())
}

fn mask(s: &str) -> String {
    if s.is_empty() {
        "(vazio)".into()
    } else if s.len() <= 8 {
        "•".repeat(s.len())
    } else {
        format!("{}…{}", &s[..4], &s[s.len() - 4..])
    }
}

fn get_field(cfg: &AgentConfig, key: &str) -> anyhow::Result<String> {
    Ok(match key {
        "brain_model" | "brain" => cfg.brain_model.clone(),
        "builder_model" | "builder" => cfg.builder_model.clone(),
        "effort" | "thinking_effort" => cfg.thinking_effort.clone(),
        "base_url" => cfg.base_url.clone(),
        "api_key" => cfg.api_key.clone(),
        "services_url" => cfg.services_url.clone(),
        other => anyhow::bail!("campo desconhecido: {other}"),
    })
}

fn set_field(cfg: &mut AgentConfig, key: &str, value: String) -> anyhow::Result<()> {
    match key {
        "brain_model" | "brain" => cfg.brain_model = value,
        "builder_model" | "builder" => cfg.builder_model = value,
        "effort" | "thinking_effort" => cfg.thinking_effort = value,
        "base_url" => cfg.base_url = value,
        "api_key" => cfg.api_key = value,
        "services_url" => cfg.services_url = value,
        other => anyhow::bail!("campo desconhecido: {other}"),
    }
    Ok(())
}
