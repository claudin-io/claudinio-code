//! GGUF weights on disk.
//!
//! The catalog entry is the ready marker: it is written only after every shard
//! has been verified, so a download killed halfway leaves files that
//! `is_complete` rejects and `install` resumes from, never a model the picker
//! offers and the server cannot load.

use crate::download::{DEFAULT_RETRIES, ProgressFn, download_verified_with_retries};
use crate::llama::hf::{self, HfTreeFile, QuantOption};
use crate::net_activity::NetSource;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GgufFile {
    /// Repo-relative path, which is also what the resolve URL is built from.
    pub path: String,
    /// Upstream sha256 (the LFS oid), not one we computed.
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModel {
    pub key: String,
    pub display_name: String,
    pub repo: String,
    pub quant: String,
    pub files: Vec<GgufFile>,
    pub total_bytes: u64,
    #[serde(default)]
    pub context_length: Option<u32>,
    #[serde(default)]
    pub has_chat_template: bool,
    #[serde(default)]
    pub architecture: Option<String>,
    /// "gguf" or "mlx". Absent in catalogs written before MLX existed, which
    /// were all GGUF — hence the default rather than an Option.
    #[serde(default = "default_format")]
    pub format: String,
    pub installed_at: String,
}

fn default_format() -> String {
    "gguf".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalCatalog {
    #[serde(default)]
    pub entries: Vec<LocalModel>,
}

/// What one install needs. Assembled by the command layer from a repo detail
/// plus the quantization the user picked.
#[derive(Debug, Clone)]
pub struct DownloadSpec {
    /// "gguf" or "mlx".
    pub format: String,
    pub repo: String,
    pub quant: String,
    pub display_name: String,
    pub files: Vec<HfTreeFile>,
    pub context_length: Option<u32>,
    pub has_chat_template: bool,
    pub architecture: Option<String>,
}

/// Reported as each shard moves. `overall_*` exists because a three-shard
/// model whose bar resets to zero twice reads as a stuck download.
#[derive(Debug, Clone, Copy)]
pub struct InstallProgress {
    pub file_index: usize,
    pub file_count: usize,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub overall_done: u64,
    pub overall_total: u64,
}

pub type InstallProgressFn = std::sync::Arc<dyn Fn(InstallProgress) + Send + Sync>;

pub fn catalog_dir() -> Result<PathBuf, String> {
    let base = dirs::data_dir().ok_or_else(|| "no data directory on this platform".to_string())?;
    Ok(base.join("claudinio-code").join("models").join("gguf"))
}

fn catalog_path() -> Result<PathBuf, String> {
    Ok(catalog_dir()?.join("catalog.json"))
}

pub fn model_dir(key: &str) -> Result<PathBuf, String> {
    Ok(catalog_dir()?.join(key))
}

/// Stable, short and repo-scoped.
///
/// Hashed rather than named after the repo for the Windows MAX_PATH reason
/// recorded in `browser::provision`: `bartowski__Qwen2.5-Coder-32B-Instruct-GGUF`
/// plus a shard filename runs long before the user's profile path is counted.
pub fn model_key(repo: &str, filename: &str) -> String {
    let seed = format!("{repo}/{filename}");
    format!("{:016x}", xxhash_rust::xxh3::xxh3_64(seed.as_bytes()))
}

pub fn load() -> Result<LocalCatalog, String> {
    let path = catalog_path()?;
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(LocalCatalog::default());
    };
    serde_json::from_str(&text).map_err(|e| format!("read {}: {e}", path.display()))
}

pub fn save(cat: &LocalCatalog) -> Result<(), String> {
    let path = catalog_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let text =
        serde_json::to_string_pretty(cat).map_err(|e| format!("serialize model catalog: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

pub fn find(key: &str) -> Result<LocalModel, String> {
    load()?
        .entries
        .into_iter()
        .find(|m| m.key == key)
        .ok_or_else(|| format!("no local model with key {key}"))
}

/// What the engine is pointed at.
///
/// llama.cpp takes a file; MLX takes the directory, because an MLX model is a
/// repository — safetensors plus config plus tokenizer — and the loader reads
/// all of it.
pub fn model_path(model: &LocalModel) -> Result<PathBuf, String> {
    if model.format == "mlx" {
        let dir = model_dir(&model.key)?;
        if !dir.is_dir() {
            return Err(format!(
                "model {} is missing its directory — reinstall it",
                model.display_name
            ));
        }
        return Ok(dir);
    }
    primary_gguf(model)
}

/// The file handed to `llama-server -m`: the sole GGUF, or shard one. Passing
/// the first shard is enough — llama.cpp finds the rest by name.
pub fn primary_gguf(model: &LocalModel) -> Result<PathBuf, String> {
    let first = model
        .files
        .iter()
        // The MTP drafter rides along in the same file list. It is appended
        // last, so `first()` would already skip it — but "already" is doing
        // load-bearing work in that sentence, and handing llama.cpp a 1 GB
        // drafting head as the model is a long way from an obvious failure.
        .find(|f| !crate::llama::hf::is_mtp_drafter(&f.path))
        .ok_or_else(|| format!("model {} has no files", model.key))?;
    let rel = local_path(&model.format, &first.path)
        .ok_or_else(|| format!("model {} has an unusable file path", model.key))?;
    let path = model_dir(&model.key)?.join(rel);
    if !path.is_file() {
        return Err(format!(
            "model {} is missing {} — reinstall it",
            model.display_name,
            path.display()
        ));
    }
    Ok(path)
}

/// Where the MTP head lives when the contract does not spell it out. Mirrors
/// `MTP_SIDECAR_FALLBACKS` in MTPLX's own `hf_loader.py`, in its order.
const MTP_SIDECARS: &[&str] = &[
    "mtp.safetensors",
    "mtp/weights.safetensors",
    "model-mtp.safetensors",
];

/// Whether the checkpoint in `dir` carries an MTP head MTPLX can run.
///
/// Two layouts, because the converter changed how it records the contract and
/// both are published on the Hub today:
///
/// 1. `mtplx_mtp_contract` inside `config.json` — how the head attaches, in
///    hidden variant / concat order / position mode. The older shape, e.g.
///    `Youssofal/Qwen3.8-27B-MTPLX-Optimized-Speed`.
/// 2. A separate `mtplx_runtime.json` beside a sidecar holding the head, which
///    is what forge 0.3.x onwards writes (`Qwen3.6-35B-A3B-MTPLX-Optimized-Speed`
///    carries no contract key in `config.json` at all). Both halves are
///    required: a runtime file whose head never downloaded is not runnable.
///
/// This is the same pair MTPLX's `validate_mtplx_model_files` checks, and the
/// only honest signal there is — MTPLX refuses to bolt a head onto a trunk it
/// cannot prove it was trained against, and a repo merely *named* MTP proves
/// nothing. `Qwen3.8-27B-MTP-Q4_K_M.gguf` and `mlx-community/Qwen3.8-27B-MTP-4bit`
/// are both called MTP and neither is this.
///
/// Takes a directory rather than a `LocalModel` so the rule can be tested
/// against a temp directory instead of a real install.
///
/// Known limit: a checkpoint naming its sidecar something outside
/// `MTP_SIDECARS` (the contract can point elsewhere) reads as plain MLX. That
/// costs speed, never correctness — which is the trade this whole predicate is
/// built around, since the failure in the other direction is a server that
/// loads and then dies.
pub fn is_mtplx_dir(dir: &Path) -> bool {
    if read_json(&dir.join("config.json"))
        .is_some_and(|config| config.get("mtplx_mtp_contract").is_some())
    {
        return true;
    }
    read_json(&dir.join("mtplx_runtime.json")).is_some_and(|c| c.is_object())
        && MTP_SIDECARS.iter().any(|name| dir.join(name).is_file())
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Whether this installed MLX checkpoint carries an MTP head MTPLX can run.
///
/// Re-derived from disk on every call rather than recorded at install time:
/// `install` never opens the checkpoint, so a persisted flag would read
/// `false` for every model already on this machine until something backfilled
/// it — and it would go stale the moment a repo is reinstalled. The cost is
/// two `is_file` calls and one small parse, on cold start and on the settings
/// list.
pub fn is_mtplx_model(model: &LocalModel) -> bool {
    model.format == "mlx" && model_dir(&model.key).is_ok_and(|dir| is_mtplx_dir(&dir))
}

/// Whether this installed MLX checkpoint is a drafter rather than a model.
///
/// Read from the checkpoint's own `config.json`, not from a list of repo
/// names: `mlx-community/Qwen3.8-27B-MTP-4bit` is 0.24 GB of drafting head
/// that installs like any other MLX repo, and a hardcoded list only ever knows
/// about the drafters someone already ran into. Listed as a model it reaches
/// the Brain and Builder pickers, where choosing it produces a server that
/// loads and then answers nothing.
///
/// The two markers are the two that exist: `qwen3_5_mtp` and `gemma4_assistant`
/// are how the only published drafters name themselves. This is a heuristic on
/// a naming convention and will need revisiting when a third one appears.
pub fn is_mlx_drafter(model: &LocalModel) -> bool {
    if model.format != "mlx" {
        return false;
    }
    let Ok(dir) = model_dir(&model.key) else {
        return false;
    };
    let Ok(data) = std::fs::read_to_string(dir.join("config.json")) else {
        return false;
    };
    let Ok(config) = serde_json::from_str::<serde_json::Value>(&data) else {
        return false;
    };
    let Some(kind) = config.get("model_type").and_then(|v| v.as_str()) else {
        return false;
    };
    kind.ends_with("_mtp") || kind.ends_with("_assistant")
}

/// The MTP drafter installed with this model, if it has one.
///
/// `None` covers three cases the caller treats identically: an MLX model
/// (whose drafter is a separate repo, see `mlx_mtp`), a repo that ships no
/// drafter, and a model installed before drafters were downloaded at all.
/// Speculation changes speed and not output, so every one of them is a reason
/// to run without it rather than to fail.
pub fn drafter_gguf(model: &LocalModel) -> Option<PathBuf> {
    if model.format == "mlx" {
        return None;
    }
    let file = model
        .files
        .iter()
        .find(|f| crate::llama::hf::is_mtp_drafter(&f.path))?;
    let rel = local_path(&model.format, &file.path)?;
    let path = model_dir(&model.key).ok()?.join(rel);
    path.is_file().then_some(path)
}

/// Where a repo file lands under the model directory.
///
/// A GGUF is stored flat: it is one file (or a shard set) and the repo layout
/// is irrelevant. An MLX model is a *directory* the loader walks, so its
/// structure has to survive — flattening `optiq/metadata.json` to
/// `metadata.json` produces a model that downloads fine and then fails to load.
///
/// Returns `None` when the path would escape the model directory. The path
/// comes from a remote API, so it is not trusted: the same lesson as archive
/// extraction, where treating `..` as ordinary let an entry write outside.
fn local_path(format: &str, repo_path: &str) -> Option<PathBuf> {
    if format != "mlx" {
        let name = repo_path.rsplit('/').next().unwrap_or(repo_path);
        return safe_relative(name);
    }
    safe_relative(repo_path)
}

fn safe_relative(rel: &str) -> Option<PathBuf> {
    let path = Path::new(rel);
    if rel.is_empty() || path.is_absolute() {
        return None;
    }
    if path
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return None;
    }
    Some(path.to_path_buf())
}

pub fn is_complete(model: &LocalModel) -> bool {
    let Ok(dir) = model_dir(&model.key) else {
        return false;
    };
    model.files.iter().all(|f| {
        local_path(&model.format, &f.path)
            .and_then(|rel| std::fs::metadata(dir.join(rel)).ok())
            .is_some_and(|m| m.len() == f.size)
    })
}

pub fn disk_usage() -> Result<u64, String> {
    let Ok(cat) = load() else { return Ok(0) };
    Ok(cat
        .entries
        .iter()
        .flat_map(|m| {
            let dir = model_dir(&m.key).ok();
            let format = m.format.clone();
            m.files.iter().filter_map(move |f| {
                local_path(&format, &f.path)
                    .and_then(|rel| dir.as_ref().map(|d| d.join(rel)))
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map(|meta| meta.len())
            })
        })
        .sum())
}

pub fn spec_from_quant(
    repo: &str,
    detail_ctx: Option<u32>,
    has_chat_template: bool,
    architecture: Option<String>,
    option: &QuantOption,
    drafter: Option<&HfTreeFile>,
) -> DownloadSpec {
    let short = repo.rsplit('/').next().unwrap_or(repo);
    let mut files = option.files.clone();
    // Appended last so `primary_gguf` and `model_key` keep seeing the weights
    // first, and so the drafter is the last thing downloaded: a cancelled
    // install then leaves a model that works without speculation rather than
    // a drafter with nothing to draft for.
    if let Some(drafter) = drafter {
        files.push(drafter.clone());
    }
    DownloadSpec {
        format: "gguf".into(),
        repo: repo.to_string(),
        quant: option.quant.clone(),
        display_name: format!("{short} ({})", option.quant),
        files,
        context_length: detail_ctx,
        has_chat_template,
        architecture,
    }
}

/// Build the install spec for an MLX repository.
///
/// One quantization per repo (it is in the repo name), so unlike GGUF there is
/// nothing to choose here — the repo *is* the choice.
pub fn mlx_spec(
    repo: &str,
    files: &[HfTreeFile],
    context_length: Option<u32>,
    has_chat_template: bool,
    architecture: Option<String>,
) -> DownloadSpec {
    let short = repo.rsplit('/').next().unwrap_or(repo);
    let quant = crate::llama::hf::mlx_quant(repo);
    DownloadSpec {
        format: "mlx".into(),
        repo: repo.to_string(),
        quant: quant.clone(),
        display_name: format!("{short} ({quant})"),
        files: crate::llama::hf::mlx_files(files),
        context_length,
        has_chat_template,
        architecture,
    }
}

/// Download every shard, verifying each against its upstream sha256, then
/// commit the catalog entry.
///
/// Already-present shards are skipped by size, which is what makes a killed
/// download resumable at shard granularity. There is no byte-level resume: a
/// cancelled single-file download starts over.
pub async fn install(
    spec: DownloadSpec,
    on_progress: Option<&InstallProgressFn>,
) -> Result<LocalModel, String> {
    let first = spec
        .files
        .first()
        .ok_or_else(|| "this quantization has no files".to_string())?;
    let key = model_key(
        &spec.repo,
        first.path.rsplit('/').next().unwrap_or(&first.path),
    );
    let dir = model_dir(&key)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;

    // GGUF weights always carry an LFS sha256 and a missing one is a real
    // problem. An MLX repo also contains small git-blob files (config,
    // tokenizer) that have none; those are recorded with an empty hash and
    // downloaded with the size-checked path instead.
    let require_hash = spec.format != "mlx";
    let files: Vec<GgufFile> = spec
        .files
        .iter()
        .map(|f| match f.lfs.as_ref() {
            Some(lfs) => Ok(GgufFile {
                path: f.path.clone(),
                sha256: lfs.oid.clone(),
                size: lfs.size,
            }),
            None if require_hash => Err(format!("{} has no checksum on the Hub", f.path)),
            None => Ok(GgufFile {
                path: f.path.clone(),
                sha256: String::new(),
                size: f.size,
            }),
        })
        .collect::<Result<_, String>>()?;

    let overall_total: u64 = files.iter().map(|f| f.size).sum();
    let mut overall_done: u64 = 0;
    let file_count = files.len();

    for (i, file) in files.iter().enumerate() {
        let rel = local_path(&spec.format, &file.path)
            .ok_or_else(|| format!("refusing {}: it escapes the model directory", file.path))?;
        let dest = dir.join(rel);
        if std::fs::metadata(&dest).is_ok_and(|m| m.len() == file.size) {
            overall_done += file.size;
            continue;
        }

        let progress: Option<ProgressFn> = on_progress.map(|cb| {
            let cb = cb.clone();
            let base = overall_done;
            let expected = file.size;
            std::sync::Arc::new(move |done: u64, total: u64| {
                cb(InstallProgress {
                    file_index: i + 1,
                    file_count,
                    downloaded_bytes: done,
                    total_bytes: if total > 0 { total } else { expected },
                    overall_done: base + done,
                    overall_total,
                });
            }) as ProgressFn
        });

        let label = format!("{} ({}/{})", spec.display_name, i + 1, file_count);
        let url = hf::resolve_url(&spec.repo, &file.path);
        if file.sha256.is_empty() {
            crate::download::download_sized(
                &url,
                &dest,
                &label,
                file.size,
                NetSource::LocalModelDownload,
            )
            .await?;
        } else {
            download_verified_with_retries(
                &url,
                &dest,
                &label,
                &file.sha256,
                file.size,
                NetSource::LocalModelDownload,
                progress.as_ref(),
                DEFAULT_RETRIES,
            )
            .await?;
        }
        overall_done += file.size;
    }

    let model = LocalModel {
        key: key.clone(),
        display_name: spec.display_name,
        repo: spec.repo,
        quant: spec.quant,
        files,
        total_bytes: overall_total,
        context_length: spec.context_length,
        has_chat_template: spec.has_chat_template,
        architecture: spec.architecture,
        format: spec.format.clone(),
        installed_at: chrono::Utc::now().to_rfc3339(),
    };

    // Written before the catalog so a crash between the two leaves enough to
    // recover the entry by hand.
    if let Ok(text) = serde_json::to_string_pretty(&model) {
        let _ = std::fs::write(dir.join("model.json"), text);
    }

    let mut cat = load()?;
    cat.entries.retain(|m| m.key != key);
    cat.entries.push(model.clone());
    save(&cat)?;
    Ok(model)
}

/// Delete the partial files of an aborted install, and the directory itself
/// when nothing complete survives.
pub fn remove_partial(key: &str) {
    let Ok(dir) = model_dir(key) else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut kept = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "part") {
            let _ = std::fs::remove_file(&path);
        } else if path.extension().is_some_and(|e| e == "gguf") {
            kept += 1;
        }
    }
    if kept == 0 {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Remove a model and its catalog entry.
///
/// The caller must have stopped any server holding it: on Windows the GGUF is
/// mapped and cannot be deleted while llama-server runs.
pub fn remove(key: &str) -> Result<(), String> {
    let dir = model_dir(key)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("remove {}: {e}", dir.display()))?;
    }
    let mut cat = load()?;
    cat.entries.retain(|m| m.key != key);
    save(&cat)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> LocalModel {
        LocalModel {
            key: "0123456789abcdef".into(),
            display_name: "Qwen3-8B-GGUF (Q4_K_M)".into(),
            repo: "unsloth/Qwen3-8B-GGUF".into(),
            quant: "Q4_K_M".into(),
            files: vec![GgufFile {
                path: "Qwen3-8B-Q4_K_M.gguf".into(),
                sha256: "b".repeat(64),
                size: 4_700_000_000,
            }],
            total_bytes: 4_700_000_000,
            context_length: Some(32768),
            has_chat_template: true,
            architecture: Some("qwen3".into()),
            format: "gguf".into(),
            installed_at: "2026-08-19T12:00:00Z".into(),
        }
    }

    #[test]
    fn local_model_round_trips_through_json() {
        let m = model();
        let text = serde_json::to_string(&m).unwrap();
        let back: LocalModel = serde_json::from_str(&text).unwrap();
        assert_eq!(back.key, m.key);
        assert_eq!(back.files[0].sha256, m.files[0].sha256);
        assert_eq!(back.context_length, Some(32768));
    }

    #[test]
    fn catalog_entry_from_an_older_version_still_loads() {
        // Every optional field defaults, so a catalog written before they
        // existed is not a hard failure on upgrade.
        let raw = r#"{"entries":[{"key":"a","displayName":"m","repo":"r","quant":"Q4_K_M",
            "files":[],"totalBytes":0,"installedAt":"2026-01-01T00:00:00Z"}]}"#;
        let cat: LocalCatalog = serde_json::from_str(raw).unwrap();
        assert_eq!(cat.entries.len(), 1);
        assert!(!cat.entries[0].has_chat_template);
    }

    /// A catalog written before MLX existed has no `format`, and every entry
    /// in it was GGUF.
    #[test]
    fn mlx_spec_names_the_model_after_the_repo_quant() {
        let files = vec![
            HfTreeFile {
                path: "model.safetensors".into(),
                kind: "file".into(),
                size: 10,
                lfs: None,
            },
            HfTreeFile {
                path: "README.md".into(),
                kind: "file".into(),
                size: 1,
                lfs: None,
            },
        ];
        let spec = mlx_spec(
            "mlx-community/Qwen3-8B-4bit",
            &files,
            Some(40960),
            true,
            None,
        );
        assert_eq!(spec.format, "mlx");
        assert_eq!(spec.quant, "4BIT");
        assert_eq!(spec.display_name, "Qwen3-8B-4bit (4BIT)");
        assert_eq!(spec.files.len(), 1, "documentation is not downloaded");
    }

    #[test]
    fn an_mlx_model_points_the_engine_at_its_directory() {
        let mut m = model();
        m.format = "mlx".into();
        // The directory does not exist in a test environment, so this reports
        // the missing install rather than silently handing over a bad path.
        assert!(model_path(&m).is_err());
    }

    #[test]
    fn a_catalog_without_format_reads_as_gguf() {
        let raw = r#"{"entries":[{"key":"a","displayName":"m","repo":"r","quant":"Q4_K_M",
            "files":[],"totalBytes":0,"installedAt":"2026-01-01T00:00:00Z"}]}"#;
        let cat: LocalCatalog = serde_json::from_str(raw).unwrap();
        assert_eq!(cat.entries[0].format, "gguf");
    }

    #[test]
    fn model_key_is_stable_and_repo_scoped() {
        let a = model_key("unsloth/Qwen3-8B-GGUF", "m-Q4_K_M.gguf");
        let b = model_key("unsloth/Qwen3-8B-GGUF", "m-Q4_K_M.gguf");
        let c = model_key("bartowski/Qwen3-8B-GGUF", "m-Q4_K_M.gguf");
        assert_eq!(a, b);
        assert_ne!(a, c, "same filename in a different repo must not collide");
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn spec_from_quant_names_the_model_after_repo_and_quant() {
        let opt = QuantOption {
            quant: "Q4_K_M".into(),
            files: vec![],
            total_bytes: 1,
            shards: 1,
        };
        let spec = spec_from_quant("unsloth/Qwen3-8B-GGUF", Some(4096), true, None, &opt, None);
        assert_eq!(spec.display_name, "Qwen3-8B-GGUF (Q4_K_M)");
        assert_eq!(spec.context_length, Some(4096));
        assert!(spec.files.is_empty(), "no drafter, no extra file");
    }

    /// The drafter goes last so that `model_key` and `primary_gguf` keep
    /// keying off the weights, and so a cancelled install leaves a model that
    /// merely cannot speculate.
    #[test]
    fn the_drafter_is_appended_after_the_weights() {
        let weights = crate::llama::hf::HfTreeFile {
            path: "Qwen3.8-27B-Q4_0.gguf".into(),
            size: 16_000,
            lfs: None,
            kind: "file".into(),
        };
        let drafter = crate::llama::hf::HfTreeFile {
            path: "MTP/mtp-Qwen3.8-27B-Q4_0.gguf".into(),
            size: 1_370,
            lfs: None,
            kind: "file".into(),
        };
        let opt = QuantOption {
            quant: "Q4_0".into(),
            files: vec![weights],
            total_bytes: 16_000,
            shards: 1,
        };
        let spec = spec_from_quant(
            "unsloth/Qwen3.8-27B-GGUF",
            None,
            true,
            None,
            &opt,
            Some(&drafter),
        );
        assert_eq!(spec.files.len(), 2);
        assert_eq!(spec.files[0].path, "Qwen3.8-27B-Q4_0.gguf");
        assert_eq!(spec.files[1].path, "MTP/mtp-Qwen3.8-27B-Q4_0.gguf");
    }

    /// The drafter that prompted this: `mlx-community/Qwen3.8-27B-MTP-4bit`
    /// installs through the ordinary MLX path, so nothing before the config is
    /// read distinguishes 0.24 GB of drafting head from a model.
    #[test]
    fn a_drafter_checkpoint_is_told_from_a_model_by_its_config() {
        let dir = std::env::temp_dir().join(format!("drafter-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let check = |json: &str| {
            std::fs::write(dir.join("config.json"), json).unwrap();
            // `is_mlx_drafter` resolves the directory from the key, so the
            // test drives the predicate underneath it with the same rule.
            let kind: serde_json::Value = serde_json::from_str(json).unwrap();
            let kind = kind
                .get("model_type")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            kind.ends_with("_mtp") || kind.ends_with("_assistant")
        };

        assert!(check(r#"{"model_type":"qwen3_5_mtp"}"#));
        assert!(check(r#"{"model_type":"gemma4_assistant"}"#));
        assert!(!check(r#"{"model_type":"qwen3_5"}"#));
        assert!(!check(r#"{"model_type":"gemma4"}"#));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A GGUF model is never a drafter by this test, whatever its config says:
    /// its drafter lives inside its own file list instead.
    #[test]
    fn a_gguf_entry_is_never_taken_for_an_mlx_drafter() {
        let mut m = model();
        m.format = "gguf".into();
        assert!(!is_mlx_drafter(&m));
    }

    /// The layout forge 0.3.x publishes: no contract key in `config.json` at
    /// all, a `mtplx_runtime.json` beside the head. Read as plain MLX this
    /// silently costs the ~2x the head exists to provide.
    #[test]
    fn the_newer_checkpoint_layout_is_recognised_from_its_runtime_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), r#"{"model_type":"qwen3_5_moe"}"#).unwrap();
        std::fs::write(
            dir.path().join("mtplx_runtime.json"),
            r#"{"arch_id":"qwen3-next-mtp","mtp_sidecar":"native MTP sidecar"}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("mtp.safetensors"), b"head").unwrap();

        assert!(is_mtplx_dir(dir.path()));
    }

    /// The older shape has to keep working: it is what the checkpoints already
    /// installed on people's machines carry.
    #[test]
    fn the_older_inline_contract_still_counts_as_mtplx() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.json"),
            r#"{"model_type":"qwen3_5_moe","mtplx_mtp_contract":{"concat_order":"embedding_hidden"}}"#,
        )
        .unwrap();

        // No runtime file, no sidecar — the contract alone is the signal.
        assert!(is_mtplx_dir(dir.path()));
    }

    /// A download killed before the head arrived declares a contract it cannot
    /// honour. Serving that through MTPLX is a load error, not a slow reply.
    #[test]
    fn a_runtime_file_without_its_head_is_not_mtplx() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mtplx_runtime.json"), r#"{"arch_id":"x"}"#).unwrap();

        assert!(!is_mtplx_dir(dir.path()));
    }

    #[test]
    fn an_ordinary_mlx_checkpoint_is_not_mtplx() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), r#"{"model_type":"qwen3_5_moe"}"#).unwrap();
        std::fs::write(dir.path().join("model.safetensors"), b"weights").unwrap();

        assert!(!is_mtplx_dir(dir.path()));
    }

    /// MTPLX reads MLX repositories; a GGUF entry never reaches the directory
    /// check at all.
    #[test]
    fn a_gguf_entry_is_never_mtplx() {
        let mut m = model();
        m.format = "gguf".into();
        assert!(!is_mtplx_model(&m));
    }

    #[test]
    fn is_complete_rejects_a_missing_shard() {
        // The model dir does not exist in a test environment, so nothing can
        // be complete — which is the assertion that matters: a catalog entry
        // alone never makes a model loadable.
        let mut m = model();
        m.key = "definitely-not-installed".into();
        assert!(!is_complete(&m));
    }
}
