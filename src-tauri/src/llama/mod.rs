//! Local inference with llama.cpp.
//!
//! The app talks to a `llama-server` sidecar over its OpenAI-compatible
//! `/v1/chat/completions` endpoint, which means the entire wire protocol —
//! streaming, tool calls, usage — is the one `agent::provider::openai` already
//! speaks to remote providers. What lives here is everything that endpoint
//! needs to exist: fetching the binary, downloading GGUF weights, and keeping
//! exactly one server process per loaded model.

pub mod bench;
pub mod catalog;
pub mod curated;
pub mod hardware;
pub mod hf;
pub mod mlx_mtp;
pub mod mlx_tiers;
pub mod provision;
pub mod supervisor;

use serde::{Deserialize, Serialize};

/// Provider id for locally served models. Model ids are `local/<model_key>`,
/// which `AgentConfig::resolve_provider` already routes by its first-slash
/// split — no change needed there.
pub const LOCAL_PROVIDER_ID: &str = "local";

/// Which local inference engine to run.
///
/// Three engines rather than one because they are good at different things:
/// llama.cpp runs everywhere and reads GGUF, MLX is Apple-Silicon-only and
/// measurably faster there (~27% on generation, measured on an M2 Max with
/// Qwen3-0.6B 4-bit: 277 tok/s llama.cpp vs 353 tok/s MLX). They also take
/// different model formats, so the engine decides which catalog applies.
///
/// `Mtplx` is the odd one out and earns its place on one number. It reads the
/// same MLX repositories as `Mlx`, but runs the multi-token-prediction head the
/// checkpoint already carries instead of a separate drafter — measured at
/// **2.24x** over autoregressive decoding on an M2 Max with
/// `Youssofal/Qwen3.8-27B-MTPLX-Optimized-Speed` (14.7 → 33.0 tok/s), where the
/// external-drafter path we built for `Mlx` came out *slower* than no
/// speculation at all. It is also the only Python one, which is a real cost:
/// see `provision::mtplx_exe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Llamacpp,
    Mlx,
    Mtplx,
}

impl Default for Engine {
    /// MLX on Apple Silicon, llama.cpp everywhere else.
    ///
    /// Platform-dependent because the answer genuinely is: on an M-series Mac
    /// MLX generates measurably faster on the same weights, and llama.cpp is
    /// the only thing that runs at all anywhere else. A config carried between
    /// machines is handled by `LocalPrefs::effective_engine`, not by pretending
    /// the default is universal.
    fn default() -> Self {
        if Engine::Mlx.is_available() {
            Engine::Mlx
        } else {
            Engine::Llamacpp
        }
    }
}

impl Engine {
    /// MLX is built on Metal and ships only for Apple Silicon; offering it
    /// anywhere else would be an option that can only fail.
    pub fn is_available(self) -> bool {
        match self {
            Engine::Llamacpp => true,
            Engine::Mlx | Engine::Mtplx => {
                cfg!(all(target_os = "macos", target_arch = "aarch64"))
            }
        }
    }

    /// The model format this engine loads. GGUF is a single file (or a shard
    /// set); MLX is a whole repository of safetensors plus its tokenizer.
    pub fn model_format(self) -> &'static str {
        match self {
            Engine::Llamacpp => "gguf",
            // MTPLX reads an ordinary MLX repository — safetensors plus
            // tokenizer — that happens to carry MTP weights and a contract
            // describing them. Same format, same catalog, same downloader.
            Engine::Mlx | Engine::Mtplx => "mlx",
        }
    }

    /// The engine that can actually load a given model format.
    ///
    /// Which engine serves a model is a property of the *model*, not a user
    /// preference: handing a GGUF to MLX aborts inside the model factory, and
    /// no setting can make it work. `LocalPrefs::engine` decides what to
    /// download next; this decides what to run.
    pub fn for_format(format: &str) -> Engine {
        match format {
            "mlx" => Engine::Mlx,
            _ => Engine::Llamacpp,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Engine::Llamacpp => "llama.cpp",
            Engine::Mlx => "MLX",
            Engine::Mtplx => "MTPLX",
        }
    }

    /// The engine to actually run `model` under.
    ///
    /// `for_format` cannot answer this alone any more: `Mlx` and `Mtplx` read
    /// the same format, and which one applies is a fact about the checkpoint —
    /// whether it carries an MTP head and the contract describing it. A model
    /// without one served through MTPLX is MTPLX doing MLX's job with a Python
    /// interpreter attached, so the fallback is always plain `Mlx`.
    pub fn for_model(model: &catalog::LocalModel, prefs: &LocalPrefs) -> Engine {
        let base = Engine::for_format(&model.format);
        if base == Engine::Mlx
            && prefs.mtp_enabled
            && Engine::Mtplx.is_available()
            && catalog::is_mtplx_model(model)
        {
            return Engine::Mtplx;
        }
        base
    }
}

/// User-facing knobs for local inference, persisted inside `AgentConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPrefs {
    pub enabled: bool,
    /// An explicit `llama-server` to use instead of the managed one.
    #[serde(default)]
    pub server_path: Option<String>,
    #[serde(default)]
    pub backend: provision::Backend,
    /// Which engine to *prefer for new downloads*, not which one runs a given
    /// model — that follows the model's own format (see `Engine::for_format`).
    #[serde(default)]
    pub engine: Engine,
    /// Context window in tokens; 0 means "whatever the model declares".
    #[serde(default)]
    pub ctx_size: u32,
    /// Layers to offload to the GPU: "auto", "all", or a number. Ignored when
    /// `backend` is Cpu, where it is forced to 0.
    #[serde(default = "default_gpu_layers")]
    pub gpu_layers: String,
    /// Server slots. Each one costs its own KV cache, so this is not free.
    #[serde(default = "default_parallel")]
    pub parallel: u32,
    /// Seconds of inactivity after which llama-server unloads the weights but
    /// keeps the port. 0 disables it.
    #[serde(default = "default_sleep_idle")]
    pub sleep_idle_seconds: u32,
    /// How many models may be resident at once. Brain and Builder can both be
    /// local, but a pair of 7Bs is already ~9 GB, so the default is one.
    #[serde(default = "default_max_loaded")]
    pub max_loaded_models: u32,
    /// Turn on MTP speculative decoding for models that have a drafter.
    ///
    /// Off by default, and deliberately a preference rather than something
    /// inferred from the model: the drafter is another gigabyte resident and
    /// the speed-up depends on how predictable *this user's* prompts are, so
    /// it is theirs to switch off when it does not pay.
    #[serde(default)]
    pub mtp_enabled: bool,
    /// Tokens proposed per speculation round. 4 is upstream's default; below 2
    /// there is nothing to speculate on.
    #[serde(default = "default_draft_block_size")]
    pub draft_block_size: u32,
    /// An `mtplx` binary to serve MTPLX-capable checkpoints with.
    ///
    /// Pointed at rather than provisioned, deliberately and only for now.
    /// Every other engine here is a pinned archive verified against a sha256
    /// before it runs; MTPLX is a Python package, and `pip install` resolves an
    /// unpinned dependency tree over the network at install time. That is a
    /// different security posture than the rest of this module holds itself to,
    /// and it deserves its own decision rather than arriving behind a 2.24x.
    #[serde(default)]
    pub mtplx_path: Option<String>,
}

fn default_draft_block_size() -> u32 {
    4
}

fn default_gpu_layers() -> String {
    "auto".into()
}
fn default_parallel() -> u32 {
    1
}
fn default_sleep_idle() -> u32 {
    300
}
fn default_max_loaded() -> u32 {
    1
}

impl LocalPrefs {
    /// The engine to prefer when downloading something new.
    ///
    /// Falls back to llama.cpp when the configured one is not available here —
    /// config.json syncs between machines, and "mlx" arriving on a Windows box
    /// should degrade to the engine that exists rather than fail every request.
    pub fn effective_engine(&self) -> Engine {
        if self.engine.is_available() {
            self.engine
        } else {
            Engine::Llamacpp
        }
    }
}

impl Default for LocalPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            server_path: None,
            backend: provision::Backend::default(),
            engine: Engine::default(),
            ctx_size: 0,
            gpu_layers: default_gpu_layers(),
            parallel: default_parallel(),
            sleep_idle_seconds: default_sleep_idle(),
            max_loaded_models: default_max_loaded(),
            mtp_enabled: false,
            draft_block_size: default_draft_block_size(),
            mtplx_path: None,
        }
    }
}

/// Headroom above the handoff threshold for the model's own reply, since the
/// context has to hold both the prompt we send and the answer we read back.
const REPLY_HEADROOM_TOKENS: u32 = 8192;

/// The `-c` to start a model with.
///
/// A GGUF often advertises far more context than this app will ever use — a
/// 27B declaring 262144 is common — and every unused token still costs KV cache
/// RAM at load time. So when the user has not pinned a value, the window is cut
/// to what the session can actually reach before it hands off, plus room for
/// the reply. Bounded by what the model itself supports.
///
/// `ctx_budget` is `AgentConfig::effective_handoff_threshold`; 0 means "no
/// opinion", which falls back to the model's own declaration.
pub fn effective_ctx(prefs: &LocalPrefs, model_ctx: Option<u32>, ctx_budget: u32) -> u32 {
    // An explicit setting is an instruction, not a hint — but still cannot
    // exceed what the weights were trained for.
    if prefs.ctx_size > 0 {
        return match model_ctx {
            Some(max) => prefs.ctx_size.min(max),
            None => prefs.ctx_size,
        };
    }
    if ctx_budget == 0 {
        // 0 tells llama-server to use the model's own value.
        return 0;
    }
    // The headroom is itself the floor: even a budget of 1 leaves 8k, which is
    // the smallest window the agent's system prompt has a chance of fitting in.
    let wanted = ctx_budget.saturating_add(REPLY_HEADROOM_TOKENS);
    match model_ctx {
        Some(max) if max > 0 => wanted.min(max),
        _ => wanted,
    }
}

/// Where the runtime stands: what is pinned, what is installed, and what the
/// user could use instead.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalStatus {
    /// False on a platform llama.cpp publishes no build for.
    pub supported: bool,
    pub build: String,
    pub target: Option<String>,
    pub server_installed: bool,
    pub exe_path: Option<String>,
    pub download_size: u64,
    pub system_server: Option<String>,
    /// The MLX runtime is independent of the llama.cpp one: a user can have
    /// either, both, or neither, and the engine picker has to say which.
    pub mlx_supported: bool,
    pub mlx_installed: bool,
    pub mlx_version: String,
    pub mlx_download_size: u64,
    /// MTPLX is provisioned separately again, and unlike the other two it is a
    /// Python environment rather than a binary — so "installed" means an
    /// interpreter, a venv and a pinned release, all of which the app builds.
    pub mtplx_supported: bool,
    pub mtplx_installed: bool,
    pub mtplx_version: String,
    /// The pinned interpreter only. The wheels `pip` resolves afterwards have
    /// no size known ahead of time, so the UI calls this a floor.
    pub mtplx_download_size: u64,
    pub engine: Engine,
}

pub fn status(prefs: &LocalPrefs) -> LocalStatus {
    let target = provision::resolved_target(prefs.backend);
    let installed = target.is_some_and(provision::is_installed);
    LocalStatus {
        supported: target.is_some(),
        build: provision::LLAMA_BUILD.to_string(),
        target: target.map(String::from),
        server_installed: installed,
        exe_path: target
            .filter(|_| installed)
            .and_then(|t| provision::managed_exe(t).ok())
            .map(|p| p.display().to_string()),
        download_size: target.and_then(provision::asset_for).map_or(0, |a| a.size),
        system_server: provision::detect_system_llama_server().map(|p| p.display().to_string()),
        mlx_supported: Engine::Mlx.is_available(),
        mlx_installed: provision::mlx_installed(),
        mlx_version: provision::mlx_version().to_string(),
        mlx_download_size: provision::mlx_download_size(),
        mtplx_supported: Engine::Mtplx.is_available(),
        mtplx_installed: provision::mtplx_installed(),
        mtplx_version: provision::mtplx_version().to_string(),
        mtplx_download_size: provision::mtplx_download_size(),
        engine: prefs.effective_engine(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_prefs_defaults_are_conservative() {
        let p = LocalPrefs::default();
        assert!(!p.enabled, "local inference must be opt-in");
        assert_eq!(p.max_loaded_models, 1);
        assert_eq!(p.sleep_idle_seconds, 300);
    }

    /// The switch has to reach both the picker and the request path; a model
    /// selected before it was turned off must fail with words, not with a
    /// connection error to a port nobody is listening on.
    #[test]
    fn effective_ctx_follows_the_handoff_budget_not_the_models_boast() {
        let prefs = LocalPrefs::default();
        // A model declaring 262144 served for a session that hands off at
        // 120k: the other 134k would be KV cache nobody ever fills.
        assert_eq!(
            effective_ctx(&prefs, Some(262_144), 120_000),
            120_000 + REPLY_HEADROOM_TOKENS
        );
    }

    #[test]
    fn effective_ctx_never_exceeds_what_the_model_supports() {
        let prefs = LocalPrefs::default();
        assert_eq!(effective_ctx(&prefs, Some(32_768), 256_000), 32_768);
    }

    #[test]
    fn effective_ctx_respects_an_explicit_setting() {
        let prefs = LocalPrefs {
            ctx_size: 16_384,
            ..LocalPrefs::default()
        };
        assert_eq!(effective_ctx(&prefs, Some(262_144), 120_000), 16_384);
        // …but not past the weights.
        assert_eq!(effective_ctx(&prefs, Some(8_192), 120_000), 8_192);
    }

    #[test]
    fn effective_ctx_defers_to_the_model_when_there_is_no_budget() {
        assert_eq!(effective_ctx(&LocalPrefs::default(), Some(262_144), 0), 0);
    }

    /// `CLAUDINIO_HANDOFF_TOKENS` bypasses the slider floor, so a tiny value
    /// can reach here and must not produce a window the prompt cannot fit in.
    #[test]
    fn a_tiny_budget_still_leaves_room_for_a_prompt() {
        assert!(effective_ctx(&LocalPrefs::default(), None, 10) >= REPLY_HEADROOM_TOKENS);
    }

    #[test]
    fn mlx_is_offered_only_on_apple_silicon() {
        assert!(Engine::Llamacpp.is_available());
        assert_eq!(
            Engine::Mlx.is_available(),
            cfg!(all(target_os = "macos", target_arch = "aarch64"))
        );
    }

    /// Whatever the default is on this platform, it has to be runnable here.
    #[test]
    fn the_default_engine_is_available_on_this_platform() {
        assert!(Engine::default().is_available());
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert_eq!(
                Engine::default(),
                Engine::Mlx,
                "MLX is faster on Apple Silicon"
            );
        } else {
            assert_eq!(Engine::default(), Engine::Llamacpp);
        }
    }

    /// A config written before engines existed picks up the platform default.
    #[test]
    fn engine_defaults_when_absent_from_the_config() {
        let p: LocalPrefs = serde_json::from_str("{\"enabled\":true}").unwrap();
        assert_eq!(p.engine, Engine::default());
    }

    /// The case that makes the platform-dependent default safe: config.json
    /// synced from a Mac names an engine a Windows box cannot run.
    /// The bug this prevents: MLX became the default on Apple Silicon while a
    /// user's installed models were all GGUF, so the sidecar was handed a
    /// `.gguf` and aborted inside the model factory.
    #[test]
    fn the_engine_follows_the_model_format_not_the_preference() {
        assert_eq!(Engine::for_format("gguf"), Engine::Llamacpp);
        assert_eq!(Engine::for_format("mlx"), Engine::Mlx);
        // An unknown format is GGUF: that is what every catalog entry written
        // before formats existed was.
        assert_eq!(Engine::for_format(""), Engine::Llamacpp);
    }

    #[test]
    fn a_synced_mlx_config_degrades_instead_of_failing() {
        let prefs = LocalPrefs {
            engine: Engine::Mlx,
            ..LocalPrefs::default()
        };
        assert!(prefs.effective_engine().is_available());
        if !Engine::Mlx.is_available() {
            assert_eq!(prefs.effective_engine(), Engine::Llamacpp);
        }
    }

    #[test]
    fn disabled_is_the_default_so_nothing_starts_unasked() {
        assert!(!LocalPrefs::default().enabled);
    }

    #[test]
    fn local_prefs_round_trip_from_an_empty_object() {
        // Every field defaults, so an older config.json that predates this
        // struct still deserializes.
        let p: LocalPrefs = serde_json::from_str("{\"enabled\":false}").unwrap();
        assert_eq!(p.gpu_layers, "auto");
        assert_eq!(p.backend, provision::Backend::Auto);
    }
}
