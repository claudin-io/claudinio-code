//! IPC surface for local inference: the runtime, the model library, and the
//! servers currently holding weights.
//!
//! Nothing here is reachable from a tool call. Installing a runtime or a model
//! spends the user's bandwidth and disk — tens of gigabytes in the model case —
//! which is a click, not something an agent may decide on its own.

use crate::llama::catalog::{self, InstallProgress, InstallProgressFn, LocalModel};
use crate::llama::hardware::{self, Fit, HardwareProfile};
use crate::llama::hf::{self, HfModelSummary, QuantOption};
use crate::llama::{LocalStatus, provision, supervisor};
use crate::state::AppState;
use serde::Serialize;
use std::sync::Arc;
use tauri::{Emitter, State};

const PROGRESS_EVENT: &str = "local-model-download-progress";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadProgress {
    pub key: String,
    pub file_index: usize,
    pub file_count: usize,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    /// Across every shard. A three-shard model whose bar resets twice reads as
    /// a stuck download, so the UI leads with this one.
    pub overall_done: u64,
    pub overall_total: u64,
    /// "download" | "verify" | "done"
    pub phase: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuantView {
    pub quant: String,
    pub total_bytes: u64,
    pub shards: usize,
    pub fit: Fit,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoQuants {
    pub repo: String,
    pub gated: bool,
    pub quants: Vec<QuantView>,
    pub recommended: Option<String>,
    pub context_length: Option<u32>,
    pub has_chat_template: bool,
    pub architecture: Option<String>,
    /// "gguf" or "mlx" — which engine can run what this repo publishes.
    pub format: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelView {
    #[serde(flatten)]
    pub model: LocalModel,
    pub running: bool,
    pub complete: bool,
    pub fit: Fit,
    /// What this model has cost on this machine, if it has been run.
    pub benchmark: Option<crate::llama::bench::ModelBenchmark>,
}

/// A model offered as a starting point.
///
/// Sourced from what the Hub is trending, filtered to what the preferred engine
/// can load; the built-in list is the offline fallback, and says so.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedModel {
    pub repo: String,
    pub display_name: String,
    pub downloads: u64,
    pub likes: u64,
    /// Only the built-in entries carry one — nobody has written a sentence
    /// about a repo that started trending this morning.
    pub blurb: Option<String>,
    /// True when this came from the built-in list because the Hub was
    /// unreachable, so the UI can say why it is showing what it is showing.
    pub offline: bool,
    /// Set only for MLX RAM-tier suggestions; lets the UI group entries by
    /// memory tier without duplicating hardware logic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram_tier: Option<SuggestedTier>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedTier {
    pub label: String,
    pub note: String,
    pub min_ram_gb: u32,
    /// True for the tier this machine's own RAM falls into.
    pub is_machine_tier: bool,
}

#[tauri::command]
pub async fn local_status(state: State<'_, AppState>) -> Result<LocalStatus, String> {
    let prefs = { state.config.lock().await.local.clone() };
    Ok(crate::llama::status(&prefs))
}

/// Live stats for every resident model, for the status bar.
#[tauri::command]
pub async fn local_runtime_stats() -> Result<Vec<supervisor::LocalModelStats>, String> {
    Ok(supervisor::stats().await)
}

/// Download the MLX sidecar. Separate from `local_install_server` because the
/// two runtimes are independent: a user can have either, both, or neither.
#[tauri::command]
pub async fn local_install_mlx(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<LocalStatus, String> {
    let emitter = app.clone();
    let on_progress: crate::download::ProgressFn = Arc::new(move |downloaded, total| {
        let _ = emitter.emit(
            "llama-server-install-progress",
            ModelDownloadProgress {
                key: "mlx".into(),
                file_index: 1,
                file_count: 1,
                downloaded_bytes: downloaded,
                total_bytes: total,
                overall_done: downloaded,
                overall_total: total,
                phase: "download",
            },
        );
    });
    provision::ensure_mlx_installed(Some(&on_progress)).await?;
    let prefs = { state.config.lock().await.local.clone() };
    Ok(crate::llama::status(&prefs))
}

#[tauri::command]
pub async fn local_uninstall_mlx(state: State<'_, AppState>) -> Result<LocalStatus, String> {
    supervisor::stop_all().await;
    provision::mlx_uninstall()?;
    let prefs = { state.config.lock().await.local.clone() };
    Ok(crate::llama::status(&prefs))
}

#[tauri::command]
pub fn local_hardware() -> HardwareProfile {
    hardware::detect()
}

/// Models to suggest: what the Hub is trending for this engine, falling back to
/// the built-in list when the Hub cannot be reached. MLX skips the Hub
/// entirely — it gets RAM-tier picks from the built-in tiers instead.
#[tauri::command]
pub async fn local_curated_models(
    state: State<'_, AppState>,
) -> Result<Vec<SuggestedModel>, String> {
    let engine = { state.config.lock().await.local.effective_engine() };

    // MLX models are whole repositories, so trending is less useful here than
    // a tiered short list that fits the machine's memory.
    if engine == crate::llama::Engine::Mlx {
        let hw = hardware::detect();
        let current_idx = crate::llama::mlx_tiers::current_tier_index(&hw);
        let tiers = crate::llama::mlx_tiers::for_machine(&hw);
        return Ok(tiers
            .iter()
            .enumerate()
            .flat_map(|(idx, tier)| {
                tier.models.iter().map(move |m| SuggestedModel {
                    repo: m.repo.to_string(),
                    display_name: m.display_name.to_string(),
                    downloads: 0,
                    likes: 0,
                    blurb: None,
                    offline: false,
                    ram_tier: Some(SuggestedTier {
                        label: tier.label.to_string(),
                        note: tier.note.to_string(),
                        min_ram_gb: tier.min_ram_gb,
                        is_machine_tier: idx == current_idx,
                    }),
                })
            })
            .collect());
    }

    if let Ok(trending) = hf::trending(engine, 12).await
        && !trending.is_empty()
    {
        return Ok(trending
            .into_iter()
            .map(|m| SuggestedModel {
                display_name: m.repo.rsplit('/').next().unwrap_or(&m.repo).to_string(),
                repo: m.repo,
                downloads: m.downloads,
                likes: m.likes,
                blurb: None,
                offline: false,
                ram_tier: None,
            })
            .collect());
    }

    let hw = hardware::detect();
    Ok(crate::llama::curated::for_hardware(&hw)
        .into_iter()
        .map(|m| SuggestedModel {
            repo: m.repo.to_string(),
            display_name: m.display_name.to_string(),
            downloads: 0,
            likes: 0,
            blurb: Some(m.blurb.to_string()),
            offline: true,
            ram_tier: None,
        })
        .collect())
}

/// Download and unpack the pinned `llama-server`.
#[tauri::command]
pub async fn local_install_server(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<LocalStatus, String> {
    let prefs = { state.config.lock().await.local.clone() };
    let emitter = app.clone();
    let on_progress: crate::download::ProgressFn = Arc::new(move |downloaded, total| {
        let _ = emitter.emit(
            "llama-server-install-progress",
            ModelDownloadProgress {
                key: "runtime".into(),
                file_index: 1,
                file_count: 1,
                downloaded_bytes: downloaded,
                total_bytes: total,
                overall_done: downloaded,
                overall_total: total,
                phase: "download",
            },
        );
    });
    provision::ensure_installed(prefs.backend, Some(&on_progress)).await?;
    provision::gc_stale_installs();
    Ok(crate::llama::status(&prefs))
}

/// Remove the managed runtime. Any running server is stopped first: on Windows
/// a mapped executable cannot be deleted.
#[tauri::command]
pub async fn local_uninstall_server(state: State<'_, AppState>) -> Result<LocalStatus, String> {
    supervisor::stop_all().await;
    provision::uninstall()?;
    let prefs = { state.config.lock().await.local.clone() };
    Ok(crate::llama::status(&prefs))
}

/// Search the Hub, or resolve a repository the user named outright.
///
/// Pasting the address bar is the natural gesture once you have found a model
/// on the Hub, and search does not match a full URL — so a pasted URL (or a
/// bare `owner/name`) is looked up directly instead. A named repo is never
/// filtered by engine: naming it is the whole point.
#[tauri::command]
pub async fn local_search_models(
    query: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<HfModelSummary>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    if let Some(repo) = hf::parse_repo_ref(query) {
        let detail = hf::repo_detail(&repo)
            .await
            .map_err(|e| format!("{repo}: {e}"))?;
        return Ok(vec![HfModelSummary {
            repo: detail.repo,
            downloads: 0,
            likes: 0,
            gated: detail.gated,
        }]);
    }

    // Only offer what the preferred engine can load: a GGUF listed while MLX
    // is selected is a download that ends in an unusable model.
    let engine = { state.config.lock().await.local.effective_engine() };
    hf::search(query, limit.unwrap_or(20).clamp(1, 50), engine).await
}

/// The quantizations a repo offers, sized, fit-checked and with one
/// pre-selected for this machine.
#[tauri::command]
pub async fn local_repo_quants(
    repo: String,
    state: State<'_, AppState>,
) -> Result<RepoQuants, String> {
    let detail = hf::repo_detail(repo.trim()).await?;
    let hw = hardware::detect();

    // An MLX repository publishes one quantization, named in the repo itself,
    // so there is nothing to pick — it is reported as a single option so the UI
    // can stay one shape for both engines.
    if hf::is_mlx_repo(&detail.files) {
        let files = hf::mlx_files(&detail.files);
        let total: u64 = files
            .iter()
            .map(|f| f.lfs.as_ref().map_or(f.size, |l| l.size))
            .sum();
        return Ok(RepoQuants {
            repo: detail.repo,
            gated: detail.gated,
            quants: vec![QuantView {
                quant: hf::mlx_quant(&repo),
                total_bytes: total,
                shards: files.len(),
                fit: hardware::fit_verdict(total, &hw),
            }],
            recommended: Some(hf::mlx_quant(&repo)),
            context_length: detail.gguf.as_ref().and_then(|g| g.context_length),
            has_chat_template: true,
            architecture: detail.gguf.and_then(|g| g.architecture),
            format: "mlx".into(),
        });
    }

    let options = hf::group_quants(&detail.files);
    if options.is_empty() {
        return Err(format!(
            "{repo} has no GGUF files we can verify — it may publish weights in another format"
        ));
    }

    let budget = hardware::usable_model_bytes(&hw);
    let ctx = {
        let cfg = state.config.lock().await;
        if cfg.local.ctx_size > 0 {
            cfg.local.ctx_size
        } else {
            detail
                .gguf
                .as_ref()
                .and_then(|g| g.context_length)
                .unwrap_or(8192)
        }
    };
    let recommended = hardware::recommend_quant(budget, ctx, &options).map(|o| o.quant.clone());

    Ok(RepoQuants {
        repo: detail.repo,
        gated: detail.gated,
        quants: options
            .iter()
            .map(|o| QuantView {
                quant: o.quant.clone(),
                total_bytes: o.total_bytes,
                shards: o.shards,
                fit: hardware::fit_verdict(o.total_bytes, &hw),
            })
            .collect(),
        recommended,
        context_length: detail.gguf.as_ref().and_then(|g| g.context_length),
        has_chat_template: detail
            .gguf
            .as_ref()
            .is_some_and(|g| g.chat_template.is_some()),
        architecture: detail.gguf.and_then(|g| g.architecture),
        format: "gguf".into(),
    })
}

/// Download one quantization of a repo.
///
/// The cancel path is a `Notify` rather than a flag threaded through
/// `download.rs`: dropping the download future aborts the byte stream, and that
/// is safe precisely because bytes land in a `.part` file that is only renamed
/// after the hash matches.
#[tauri::command]
pub async fn local_install_model(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    repo: String,
    quant: String,
) -> Result<LocalModel, String> {
    let detail = hf::repo_detail(repo.trim()).await?;
    if detail.gated {
        return Err(format!(
            "{repo} is gated — accept its licence on huggingface.co first"
        ));
    }
    let has_template = detail
        .gguf
        .as_ref()
        .is_some_and(|g| g.chat_template.is_some());
    let ctx = detail.gguf.as_ref().and_then(|g| g.context_length);
    let arch = detail.gguf.as_ref().and_then(|g| g.architecture.clone());

    let (spec, key) = if hf::is_mlx_repo(&detail.files) {
        let spec = catalog::mlx_spec(&detail.repo, &detail.files, ctx, true, arch);
        // An MLX model is a whole repository, so the repo name is what
        // identifies it — there is no single file to key on.
        let key = catalog::model_key(&detail.repo, "mlx");
        (spec, key)
    } else {
        let options = hf::group_quants(&detail.files);
        let option: &QuantOption = options
            .iter()
            .find(|o| o.quant.eq_ignore_ascii_case(&quant))
            .ok_or_else(|| format!("{repo} has no {quant} quantization"))?;
        let first = option
            .files
            .first()
            .ok_or_else(|| "this quantization has no files".to_string())?;
        let key = catalog::model_key(
            &detail.repo,
            first.path.rsplit('/').next().unwrap_or(&first.path),
        );
        (
            catalog::spec_from_quant(&detail.repo, ctx, has_template, arch, option),
            key,
        )
    };

    let cancel = Arc::new(tokio::sync::Notify::new());
    {
        let mut pending = state.local_downloads.lock().await;
        if pending.contains_key(&key) {
            return Err("this model is already downloading".into());
        }
        pending.insert(key.clone(), cancel.clone());
    }

    let emitter = app.clone();
    let progress_key = key.clone();
    let on_progress: InstallProgressFn = Arc::new(move |p: InstallProgress| {
        let _ = emitter.emit(
            PROGRESS_EVENT,
            ModelDownloadProgress {
                key: progress_key.clone(),
                file_index: p.file_index,
                file_count: p.file_count,
                downloaded_bytes: p.downloaded_bytes,
                total_bytes: p.total_bytes,
                overall_done: p.overall_done,
                overall_total: p.overall_total,
                phase: "download",
            },
        );
    });

    let outcome = tokio::select! {
        result = catalog::install(spec, Some(&on_progress)) => result,
        _ = cancel.notified() => {
            catalog::remove_partial(&key);
            Err("download cancelled".to_string())
        }
    };

    state.local_downloads.lock().await.remove(&key);

    let model = outcome?;
    let _ = app.emit(
        PROGRESS_EVENT,
        ModelDownloadProgress {
            key,
            file_index: model.files.len(),
            file_count: model.files.len(),
            downloaded_bytes: model.total_bytes,
            total_bytes: model.total_bytes,
            overall_done: model.total_bytes,
            overall_total: model.total_bytes,
            phase: "done",
        },
    );
    Ok(model)
}

#[tauri::command]
pub async fn local_cancel_install(state: State<'_, AppState>, key: String) -> Result<(), String> {
    let pending = state.local_downloads.lock().await;
    let notify = pending
        .get(&key)
        .ok_or_else(|| "that download is not running".to_string())?;
    notify.notify_waiters();
    Ok(())
}

#[tauri::command]
pub async fn local_list_models() -> Result<Vec<LocalModelView>, String> {
    let hw = hardware::detect();
    let benchmarks = crate::llama::bench::load();
    let mut out = Vec::new();
    for model in catalog::load()?.entries {
        out.push(LocalModelView {
            running: supervisor::is_running(&model.key).await,
            complete: catalog::is_complete(&model),
            fit: hardware::fit_verdict(model.total_bytes, &hw),
            benchmark: benchmarks.entries.get(&model.key).cloned(),
            model,
        });
    }
    out.sort_by(|a, b| a.model.display_name.cmp(&b.model.display_name));
    Ok(out)
}

#[tauri::command]
pub async fn local_remove_model(key: String) -> Result<(), String> {
    // Stop first: on Windows the GGUF is mapped by the running server and the
    // delete would fail with a sharing violation.
    supervisor::stop(&key).await;
    crate::llama::bench::forget(&key);
    catalog::remove(&key)
}

#[tauri::command]
pub async fn local_unload_model(key: String) -> Result<(), String> {
    supervisor::stop(&key).await;
    Ok(())
}

#[tauri::command]
pub async fn local_server_logs(key: String) -> Result<Vec<String>, String> {
    Ok(supervisor::stderr_tail(&key).await)
}

#[tauri::command]
pub async fn local_disk_usage() -> Result<u64, String> {
    catalog::disk_usage()
}

/// Start the model and put one real request through it.
///
/// The counterpart of `browser_test`: it turns "the model silently does
/// nothing" into a message, with llama-server's own stderr attached when the
/// start is what failed.
#[tauri::command]
pub async fn local_test_model(state: State<'_, AppState>, key: String) -> Result<String, String> {
    let config = { state.config.lock().await.clone() };
    let started = std::time::Instant::now();
    let rp = crate::agent::provider::resolve_provider_live(
        &config,
        &format!("{}/{key}", crate::llama::LOCAL_PROVIDER_ID),
    )
    .await?;
    let ready = started.elapsed();

    let reply = crate::agent::provider::openai::complete(
        &rp,
        "You are a health check. Reply with exactly: OK",
        "Reply with exactly: OK",
        16,
        crate::net_activity::NetSource::LlmOneShot,
    )
    .await?;

    Ok(format!(
        "Ready in {:.1}s at {} — model replied {:?}",
        ready.as_secs_f32(),
        rp.base_url,
        reply.trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The event payload is what the settings UI drives its two bars from;
    /// renaming a field silently breaks the progress display.
    #[test]
    fn progress_payload_is_camel_case() {
        let json = serde_json::to_string(&ModelDownloadProgress {
            key: "abc".into(),
            file_index: 1,
            file_count: 3,
            downloaded_bytes: 10,
            total_bytes: 20,
            overall_done: 10,
            overall_total: 60,
            phase: "download",
        })
        .unwrap();
        for field in [
            "\"fileIndex\"",
            "\"fileCount\"",
            "\"downloadedBytes\"",
            "\"overallDone\"",
            "\"overallTotal\"",
            "\"phase\"",
        ] {
            assert!(json.contains(field), "{field} missing from {json}");
        }
    }

    #[test]
    fn suggested_model_omits_ram_tier_when_absent_and_serializes_camel_case() {
        let plain = SuggestedModel {
            repo: "r/m".into(),
            display_name: "m".into(),
            downloads: 0,
            likes: 0,
            blurb: None,
            offline: false,
            ram_tier: None,
        };
        let json = serde_json::to_string(&plain).unwrap();
        assert!(!json.contains("ramTier"), "{json}");
        assert!(json.contains("\"displayName\":\"m\""), "{json}");

        let tiered = SuggestedModel {
            ram_tier: Some(SuggestedTier {
                label: "32GB".into(),
                note: "Strong coding + reasoning".into(),
                min_ram_gb: 32,
                is_machine_tier: true,
            }),
            ..plain
        };
        let json = serde_json::to_string(&tiered).unwrap();
        for field in ["\"ramTier\"", "\"minRamGb\"", "\"isMachineTier\""] {
            assert!(json.contains(field), "{field} missing from {json}");
        }
    }

    #[test]
    fn model_view_flattens_the_model_fields() {
        let view = LocalModelView {
            model: LocalModel {
                key: "k".into(),
                display_name: "m".into(),
                repo: "r/m".into(),
                quant: "Q4_K_M".into(),
                files: vec![],
                total_bytes: 1,
                context_length: None,
                has_chat_template: true,
                architecture: None,
                format: "gguf".into(),
                installed_at: "2026-08-19T00:00:00Z".into(),
            },
            running: false,
            complete: true,
            fit: Fit::Comfortable,
            benchmark: None,
        };
        let json = serde_json::to_string(&view).unwrap();
        assert!(json.contains("\"displayName\":\"m\""), "{json}");
        assert!(json.contains("\"running\":false"), "{json}");
        assert!(json.contains("\"fit\":\"comfortable\""), "{json}");
    }
}
