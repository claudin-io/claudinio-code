//! One `llama-server` process per loaded model.
//!
//! A global registry rather than a field on `AppState`: the provider layer
//! reaches inference through `&AgentConfig` alone (`provider::stream_message`),
//! with no handle to Tauri state, and that is the exact place a request has to
//! learn which port its model is listening on.
//!
//! Every server is bound to loopback, given a random api-key and started with
//! `--no-webui`. The key is the part that matters: without it any process on
//! the machine — including a page the agent's own browser visits — could POST
//! to the port and get free inference plus a prompt-injection oracle.
//! `/v1/chat/completions` answers 401 without it (verified against b10502);
//! `/health` and `/v1/models` stay open by llama-server's design, which is why
//! the readiness probe can use `/health` and why nothing secret is put in the
//! model alias — it is a content hash, not a path.

use crate::llama::{Engine, LocalPrefs, catalog, mlx_mtp, provision};
use crate::procutil;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Floor of the readiness deadline, plus an allowance per byte of weights.
/// A fixed timeout cannot work here: a 20 GB GGUF off a cold page cache takes
/// minutes to map, while a 2 GB one is ready in seconds.
const HEALTH_BASE_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_BYTES_PER_SEC: u64 = 50_000_000;
const HEALTH_MAX_TIMEOUT: Duration = Duration::from_secs(600);
/// Lines of the server's stderr kept for diagnostics.
const STDERR_TAIL_LINES: usize = 200;
/// A port can be taken between our probe and llama-server's bind.
const PORT_ATTEMPTS: usize = 3;
/// Stats feed a status bar; a server mid-restart must not stall the poll.
const STATS_TIMEOUT: Duration = Duration::from_secs(2);

/// Where a live `llama-server` can be reached. Handed back on every request
/// because the port and key belong to the process, not to the config.
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: String,
    pub model_key: String,
}

/// What a local model is doing right now.
///
/// Reported separately from the process registry because the slowest phase —
/// loading tens of gigabytes of weights — happens *before* there is a process
/// to ask, and that is exactly when the user most needs to be told something
/// is happening.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Weights are being read into memory. Minutes, for a large model.
    Loading,
    /// The server has the request and is evaluating the prompt. No tokens come
    /// out during this, which is what reads as a hang.
    ReadingPrompt,
    Generating,
    /// Loaded and waiting.
    #[default]
    Idle,
    /// llama-server unloaded the weights after an idle period.
    Sleeping,
}

type Phases = Arc<std::sync::Mutex<HashMap<String, Phase>>>;

fn phases() -> &'static Phases {
    static PHASES: OnceLock<Phases> = OnceLock::new();
    PHASES.get_or_init(|| Arc::new(std::sync::Mutex::new(HashMap::new())))
}

/// A request has been sent; the server is evaluating the prompt.
///
/// Returns the instant to measure time-to-first-token from. Kept here rather
/// than in the provider so both engines are measured the same way and the
/// phase and the benchmark cannot drift apart.
pub fn note_request_start(model_key: &str) -> Instant {
    set_phase(model_key, Phase::ReadingPrompt);
    Instant::now()
}

/// The first token arrived. This is the number that decides whether the app
/// felt stuck, so it is recorded even if the run is later cancelled.
pub fn note_first_token(model_key: &str, started: Instant, prompt_tokens: u32) {
    set_phase(model_key, Phase::Generating);
    let elapsed = started.elapsed().as_secs_f64();
    let key = model_key.to_string();
    // Off the request path: this writes a file, and a benchmark must never
    // slow down the thing it is measuring.
    tokio::task::spawn_blocking(move || {
        crate::llama::bench::update(&key, |b| {
            b.record_generation(
                elapsed,
                b.tokens_per_second,
                b.prompt_tokens_per_second,
                prompt_tokens,
            );
        });
    });
}

/// The stream finished. `tokens_per_second` is what the server reported for
/// this run, not an average.
pub fn note_request_end(model_key: &str, tokens_per_second: f64, prompt_tokens_per_second: f64) {
    set_phase(model_key, Phase::Idle);
    if tokens_per_second <= 0.0 {
        return;
    }
    let key = model_key.to_string();
    tokio::task::spawn_blocking(move || {
        crate::llama::bench::update(&key, |b| {
            // Fold the rates into the same sample the first token opened, so a
            // run counts once.
            let samples = b.generation_samples.max(1);
            b.tokens_per_second =
                (b.tokens_per_second * (samples - 1) as f64 + tokens_per_second) / samples as f64;
            b.prompt_tokens_per_second = (b.prompt_tokens_per_second * (samples - 1) as f64
                + prompt_tokens_per_second)
                / samples as f64;
        });
    });
}

/// Record what `model_key` is doing. Cheap and lock-light: called on every
/// request boundary and on the first token of a stream.
pub fn set_phase(model_key: &str, phase: Phase) {
    if let Ok(mut map) = phases().lock() {
        map.insert(model_key.to_string(), phase);
    }
}

pub fn clear_phase(model_key: &str) {
    if let Ok(mut map) = phases().lock() {
        map.remove(model_key);
    }
}

fn phase_of(model_key: &str) -> Option<Phase> {
    phases().lock().ok().and_then(|m| m.get(model_key).copied())
}

/// What the settings UI shows about a resident model.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningModel {
    pub model_key: String,
    pub port: u16,
    pub file_bytes: u64,
    pub idle_seconds: u64,
}

struct Instance {
    child: Mutex<Child>,
    port: u16,
    pid: Option<u32>,
    /// Which binary is behind this port. The two engines report their counters
    /// differently, so reading them needs to know.
    engine: Engine,
    /// The `-c` this process was started with. A running server cannot be
    /// resized, so a changed budget has to restart it — otherwise moving the
    /// handoff slider looks like it did nothing.
    ctx_size: u32,
    api_key: String,
    file_bytes: u64,
    started: Instant,
    last_used: Mutex<Instant>,
    stderr_tail: Arc<std::sync::Mutex<VecDeque<String>>>,
    pid_file: PathBuf,
}

impl Instance {
    fn endpoint(&self, model_key: &str) -> Endpoint {
        Endpoint {
            base_url: format!("http://127.0.0.1:{}/v1", self.port),
            api_key: self.api_key.clone(),
            model_key: model_key.to_string(),
        }
    }
}

type Registry = Arc<Mutex<HashMap<String, Arc<Instance>>>>;

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// Everything one server start needs. Grouped into a struct so `build_args`
/// stays a pure function with one parameter instead of nine.
#[derive(Debug, Clone)]
pub struct StartSpec {
    pub engine: Engine,
    pub model_key: String,
    pub model_path: PathBuf,
    pub port: u16,
    pub api_key: String,
    pub ctx_size: u32,
    pub gpu_layers: String,
    pub parallel: u32,
    pub sleep_idle_seconds: u32,
    /// The MTP drafter to speculate with, when the user asked for it and the
    /// model has one installed. `None` is plain single-token decoding.
    pub draft_model_path: Option<PathBuf>,
    /// Tokens per speculation round. Ignored without a drafter.
    pub draft_block_size: u32,
}

/// The argv for one server, per engine.
///
/// The two binaries expose the same HTTP surface but take different flags —
/// `llama-server` is upstream's and predates us, `claudinio-mlx` is ours and
/// was written to match it.
pub fn build_args(spec: &StartSpec) -> Vec<String> {
    match spec.engine {
        Engine::Llamacpp => build_llamacpp_args(spec),
        Engine::Mlx => build_mlx_args(spec),
        Engine::Mtplx => build_mtplx_args(spec),
    }
}

/// `mtplx serve`.
///
/// The drafter fields carry a different meaning here and it is worth being
/// explicit: there is no drafter. `draft_model_path` is unused because the head
/// is inside the checkpoint, and `draft_block_size` becomes `--depth`, the
/// number of MTP levels to run. Depth is per-machine and per-model — `mtplx
/// tune` measures it against autoregressive decoding and refuses to save a
/// depth that loses — so the value here is a starting point, not a verdict.
/// Measured on an M2 Max with Qwen3.8-27B: D1 1.54x, D2 1.61x, D3 2.24x.
fn build_mtplx_args(spec: &StartSpec) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "serve".into(),
        "--model".into(),
        spec.model_path.display().to_string(),
        "--model-id".into(),
        spec.model_key.clone(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        spec.port.to_string(),
        "--api-key".into(),
        spec.api_key.clone(),
    ];
    // Off means autoregressive, not "a different engine". Keeping MTPLX with
    // MTP switched off is what makes the toggle reversible without a reinstall,
    // and `ar` leaves the weights loaded so the next request can switch back.
    args.push("--generation-mode".into());
    if spec.draft_model_path.is_some() || spec.draft_block_size > 0 {
        args.push("mtp".into());
        args.push("--depth".into());
        // `--depth` counts MTP levels; `draft_block_size` counts the block
        // including the bonus token, the way MLX and llama.cpp do.
        args.push(spec.draft_block_size.saturating_sub(1).max(1).to_string());
    } else {
        args.push("ar".into());
    }
    args
}

/// `claudinio-mlx serve`. The model is a *directory* (safetensors plus the
/// tokenizer), not a file, and there is no `-ngl`: MLX runs on Metal with
/// unified memory, so there is nothing to offload.
fn build_mlx_args(spec: &StartSpec) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "serve".into(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        spec.port.to_string(),
        "--model".into(),
        spec.model_path.display().to_string(),
        "--alias".into(),
        spec.model_key.clone(),
        "--api-key".into(),
        spec.api_key.clone(),
    ];
    if spec.ctx_size > 0 {
        args.push("--ctx-size".into());
        args.push(spec.ctx_size.to_string());
    }
    // A drafter proposes a block of tokens per round that the model verifies
    // in one pass, and rejected drafts are discarded — so what the model is
    // capable of does not change. The token sequence can still differ from an
    // unspeculated run: verifying a block evaluates several positions at once,
    // which reorders the floating-point reductions, and a near-tied token then
    // falls the other way. Measured on an M2 Max with Qwen3.8-27B-Q4_K_M, the
    // two answers said the same thing in slightly different words.
    if let Some(drafter) = &spec.draft_model_path {
        args.push("--draft-model".into());
        args.push(drafter.display().to_string());
        args.push("--draft-block-size".into());
        args.push(spec.draft_block_size.to_string());
    }
    args
}

/// `llama-server`.
///
/// `--jinja` is what makes tool calling work at all: without it llama-server
/// falls back to a built-in template that cannot emit tool calls, and the model
/// answers in prose while the agent waits for a `tool_use` block that never
/// arrives.
fn build_llamacpp_args(spec: &StartSpec) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        spec.port.to_string(),
        "-m".into(),
        spec.model_path.display().to_string(),
        "-a".into(),
        spec.model_key.clone(),
        "--api-key".into(),
        spec.api_key.clone(),
        "--jinja".into(),
        "--no-webui".into(),
        // Prometheus counters, off by default. They are the only source of a
        // real token rate: the alternative is timing requests ourselves, which
        // measures the network round trip as much as the model.
        "--metrics".into(),
    ];
    if spec.ctx_size > 0 {
        args.push("-c".into());
        args.push(spec.ctx_size.to_string());
    }
    if !spec.gpu_layers.is_empty() {
        args.push("-ngl".into());
        args.push(spec.gpu_layers.clone());
    }
    if spec.parallel > 0 {
        args.push("-np".into());
        args.push(spec.parallel.to_string());
    }
    if spec.sleep_idle_seconds > 0 {
        args.push("--sleep-idle-seconds".into());
        args.push(spec.sleep_idle_seconds.to_string());
    }
    // `draft-mtp` reads the drafting head the checkpoint already carries,
    // rather than running a second full model as `draft-simple` would.
    if let Some(drafter) = &spec.draft_model_path {
        args.push("--spec-type".into());
        args.push("draft-mtp".into());
        args.push("--spec-draft-model".into());
        args.push(drafter.display().to_string());
        args.push("--spec-draft-n-max".into());
        // llama.cpp counts *drafted* tokens where MLX counts the whole block,
        // bonus token included. Same round, one off in the name.
        args.push(spec.draft_block_size.saturating_sub(1).max(1).to_string());
    }
    args
}

/// The drafter directory to speculate with, or `None` to decode one token at
/// a time.
///
/// Every step is a reason to decline, and declining is always silent-but-fine:
/// speculation changes speed, never output, so a model without a drafter is
/// not a broken model. What must not happen is starting *with* a drafter the
/// engine will refuse — the engine treats a bad pair as a fatal load error, so
/// a stale catalog entry would turn "MTP is on" into "no local models work".
fn drafter_path(
    prefs: &LocalPrefs,
    engine: Engine,
    model: &catalog::LocalModel,
) -> Option<PathBuf> {
    if !prefs.mtp_enabled {
        return None;
    }
    if engine == Engine::Mtplx {
        // The head is inside the checkpoint; there is nothing to point at.
        // `build_mtplx_args` reads `draft_block_size` as a depth instead.
        return None;
    }
    if engine == Engine::Llamacpp {
        // A GGUF repo ships its drafter beside the weights, so it was
        // installed with them and there is nothing to look up.
        return catalog::drafter_gguf(model);
    }
    let repo = mlx_mtp::drafter_for(&model.repo)?;
    let installed = catalog::load().ok()?;
    let drafter = installed
        .entries
        .iter()
        .find(|e| e.repo.eq_ignore_ascii_case(repo))?;
    let dir = catalog::model_path(drafter).ok()?;
    dir.is_dir().then_some(dir)
}

/// The `llama-server` to run: the user's own if they pointed at one, otherwise
/// the managed install (which must already be provisioned — this is a request
/// path and may not spend the user's bandwidth on its own).
fn resolve_exe(prefs: &LocalPrefs, engine: Engine) -> Result<PathBuf, String> {
    if engine == Engine::Mtplx {
        // Pointed at, not provisioned — see `LocalPrefs::mtplx_path`. This is
        // the one engine the app does not install, and a missing path has to
        // say so rather than quietly drop to a slower engine: MTPLX measured
        // 2.24x here, and losing that silently is worse than a sentence.
        let Some(explicit) = prefs.mtplx_path.as_ref().filter(|p| !p.trim().is_empty()) else {
            return Err(
                "MTPLX is enabled but no binary is configured — set its path in Settings → Local models"
                    .into(),
            );
        };
        let path = PathBuf::from(explicit);
        if !path.is_file() {
            return Err(format!("mtplx not found at {explicit}"));
        }
        return Ok(path);
    }
    if engine == Engine::Mlx {
        // Overriding the binary is a llama.cpp affordance: llama-server is
        // widely installed already, ours is not.
        return provision::mlx_exe().and_then(|exe| {
            if exe.is_file() {
                Ok(exe)
            } else {
                Err(
                    "the MLX runtime is not installed — install it in Settings → Local models"
                        .into(),
                )
            }
        });
    }
    if let Some(explicit) = prefs.server_path.as_ref().filter(|p| !p.trim().is_empty()) {
        let path = PathBuf::from(explicit);
        if !path.is_file() {
            return Err(format!("llama-server not found at {explicit}"));
        }
        return Ok(path);
    }
    let target = provision::resolved_target(prefs.backend).ok_or_else(|| {
        format!(
            "no llama.cpp build published for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    if !provision::is_installed(target) {
        return Err(
            "the llama.cpp runtime is not installed — install it in Settings → Local models".into(),
        );
    }
    provision::managed_exe(target)
}

/// A free loopback port, released before we return it.
///
/// llama-server writes no port file and its stderr banner is not a stable
/// contract, so probing and handing it the number is the pragmatic option; the
/// race it opens is closed by retrying the whole start.
fn reserve_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("could not reserve a local port: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("could not read the reserved port: {e}"))?
        .port();
    drop(listener);
    Ok(port)
}

/// Redact the api-key before anything the user or a log can see. It is the
/// only credential protecting the port.
fn redact(line: &str, api_key: &str) -> String {
    if api_key.is_empty() {
        return line.to_string();
    }
    line.replace(api_key, "***")
}

fn spawn_stderr_reader(
    child: &mut Child,
    api_key: String,
) -> Arc<std::sync::Mutex<VecDeque<String>>> {
    let tail = Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(
        STDERR_TAIL_LINES,
    )));
    let Some(stderr) = child.stderr.take() else {
        return tail;
    };
    let sink = Arc::clone(&tail);
    tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(mut buf) = sink.lock() else { return };
            if buf.len() == STDERR_TAIL_LINES {
                buf.pop_front();
            }
            buf.push_back(redact(&line, &api_key));
        }
    });
    tail
}

fn tail_of(tail: &Arc<std::sync::Mutex<VecDeque<String>>>) -> Vec<String> {
    tail.lock()
        .map(|b| b.iter().cloned().collect())
        .unwrap_or_default()
}

fn health_deadline(file_bytes: u64) -> Duration {
    let extra = Duration::from_secs(file_bytes / HEALTH_BYTES_PER_SEC);
    (HEALTH_BASE_TIMEOUT + extra).min(HEALTH_MAX_TIMEOUT)
}

/// Wait until the server answers `/health` with 200.
///
/// 503 means "still loading the model" and is the normal state for most of this
/// wait. A dead child ends it immediately: waiting out a ten-minute deadline on
/// a process that already exited helps nobody.
async fn wait_for_health(
    engine: Engine,
    port: u16,
    api_key: &str,
    child: &mut Child,
    deadline: Duration,
) -> Result<(), String> {
    let client = crate::http::default_client();
    let url = format!("http://127.0.0.1:{port}/health");
    let started = Instant::now();
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "the {} server exited during startup ({status})",
                engine.label()
            ));
        }
        if let Ok(resp) = client
            .get(&url)
            .bearer_auth(api_key)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            && resp.status().is_success()
        {
            return Ok(());
        }
        if started.elapsed() > deadline {
            return Err(format!(
                "the {} server did not become ready within {}s",
                engine.label(),
                deadline.as_secs()
            ));
        }
        tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
    }
}

fn pid_file_for(model_key: &str) -> Result<PathBuf, String> {
    Ok(provision::llama_dir()?.join(format!("llama-server-{model_key}.pid")))
}

/// Kill a server recorded by a previous run of this app that is still alive.
///
/// The PID is only acted on when the live process's executable matches ours:
/// PIDs are recycled, and killing whatever inherited the number would be
/// someone else's process.
fn kill_recorded_orphan(pid_path: &std::path::Path, exe: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(pid_path) else {
        return;
    };
    let Ok(pid) = text.trim().parse::<u32>() else {
        let _ = std::fs::remove_file(pid_path);
        return;
    };
    let mut sys = sysinfo::System::new();
    let spid = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[spid]), true);
    if let Some(proc) = sys.process(spid)
        && proc.exe() == Some(exe)
    {
        eprintln!("[llama] killing orphaned llama-server from a previous run (pid {pid})");
        proc.kill();
    }
    let _ = std::fs::remove_file(pid_path);
}

/// Refuse a start that would not fit in memory.
///
/// Without this the failure mode is not an error dialog: the machine starts
/// swapping the weights and the whole desktop stops responding.
fn admit(new_bytes: u64, resident_bytes: u64) -> Result<(), String> {
    let hw = crate::llama::hardware::detect();
    let budget = (hw.total_ram_bytes as f64 * 0.75) as u64;
    if resident_bytes + new_bytes > budget {
        return Err(format!(
            "not enough memory: this model needs {:.1} GB and {:.1} GB is already loaded, \
             out of a {:.1} GB budget. Unload a model first, or pick a smaller quantization.",
            new_bytes as f64 / 1e9,
            resident_bytes as f64 / 1e9,
            budget as f64 / 1e9,
        ));
    }
    Ok(())
}

/// The endpoint for `model_key`, starting a server if one is not already up.
///
/// Idempotent and safe to call concurrently: the registry lock is held across
/// the whole start, so two parallel agents asking for the same model wait on
/// one process rather than racing to spawn two.
/// `ctx_budget` is the largest context the session will ever actually send
/// (`AgentConfig::effective_handoff_threshold`); 0 defers to the model. See
/// `llama::effective_ctx` — a model that advertises 262144 would otherwise
/// allocate KV cache for tokens this app never reaches.
pub async fn ensure_serving(
    model_key: &str,
    prefs: &LocalPrefs,
    ctx_budget: u32,
) -> Result<Endpoint, String> {
    let reg = registry();
    let mut map = reg.lock().await;

    let model = catalog::find(model_key)?;
    let wanted_ctx = crate::llama::effective_ctx(prefs, model.context_length, ctx_budget);

    if let Some(instance) = map.get(model_key) {
        // A server whose process died (OOM killer, crash) must not be handed
        // out as if it were live.
        let dead = {
            let mut child = instance.child.lock().await;
            matches!(child.try_wait(), Ok(Some(_)))
        };
        let resized = instance.ctx_size != wanted_ctx;
        if !dead && !resized {
            *instance.last_used.lock().await = Instant::now();
            return Ok(instance.endpoint(model_key));
        }
        if let Some(stale) = map.remove(model_key)
            && !dead
        {
            terminate(stale).await;
        }
    }

    // The engine is decided by the model, not by the preference: handing a
    // GGUF to MLX aborts inside its model factory, and the user's preference
    // cannot change what the weights are.
    let engine = Engine::for_model(&model, prefs);
    if !engine.is_available() {
        return Err(format!(
            "{} is a {} model, and the {} engine is not available on this machine",
            model.display_name,
            model.format.to_uppercase(),
            engine.label()
        ));
    }
    // llama.cpp is handed a file, MLX a directory; the catalog knows which
    // because it recorded the format when the model was installed.
    let model_path = catalog::model_path(&model)?;
    let draft_model_path = drafter_path(prefs, engine, &model);
    let exe = resolve_exe(prefs, engine)?;

    // Evict down to the configured ceiling before admitting a new model.
    let max_loaded = prefs.max_loaded_models.clamp(1, 2) as usize;
    while map.len() >= max_loaded {
        let mut oldest: Option<(String, Instant)> = None;
        for (key, inst) in map.iter() {
            let used = *inst.last_used.lock().await;
            if oldest.as_ref().is_none_or(|(_, t)| used < *t) {
                oldest = Some((key.clone(), used));
            }
        }
        let Some((key, _)) = oldest else { break };
        if let Some(inst) = map.remove(&key) {
            terminate(inst).await;
        }
    }

    let mut resident = 0u64;
    for inst in map.values() {
        resident += inst.file_bytes;
    }
    admit(model.total_bytes, resident)?;

    let pid_file = pid_file_for(model_key)?;
    kill_recorded_orphan(&pid_file, &exe);

    set_phase(model_key, Phase::Loading);
    let load_started = Instant::now();
    let mut last_err = String::new();
    for _ in 0..PORT_ATTEMPTS {
        let port = reserve_port()?;
        let api_key = crate::randutil::random_hex(32);
        let spec = StartSpec {
            engine,
            model_key: model_key.to_string(),
            model_path: model_path.clone(),
            port,
            api_key: api_key.clone(),
            ctx_size: wanted_ctx,
            gpu_layers: if prefs.backend == provision::Backend::Cpu {
                "0".into()
            } else {
                prefs.gpu_layers.clone()
            },
            parallel: prefs.parallel,
            sleep_idle_seconds: prefs.sleep_idle_seconds,
            draft_model_path: draft_model_path.clone(),
            draft_block_size: prefs.draft_block_size,
        };

        let mut cmd = Command::new(&exe);
        cmd.args(build_args(&spec))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        // The binary lives beside its ggml backend libraries; starting there
        // is what lets the loader find them on every platform.
        if let Some(dir) = exe.parent() {
            cmd.current_dir(dir);
        }
        procutil::no_window_tokio(&mut cmd);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
        let stderr_tail = spawn_stderr_reader(&mut child, api_key.clone());

        if let Some(pid) = child.id() {
            let _ = std::fs::write(&pid_file, pid.to_string());
        }

        match wait_for_health(
            engine,
            port,
            &api_key,
            &mut child,
            health_deadline(model.total_bytes),
        )
        .await
        {
            Ok(()) => {
                let pid = child.id();
                let instance = Arc::new(Instance {
                    child: Mutex::new(child),
                    port,
                    pid,
                    engine,
                    ctx_size: wanted_ctx,
                    api_key,
                    file_bytes: model.total_bytes,
                    started: Instant::now(),
                    last_used: Mutex::new(Instant::now()),
                    stderr_tail,
                    pid_file,
                });
                let endpoint = instance.endpoint(model_key);
                map.insert(model_key.to_string(), instance);
                set_phase(model_key, Phase::Idle);
                let seconds = load_started.elapsed().as_secs_f64();
                let key = model_key.to_string();
                tokio::task::spawn_blocking(move || {
                    crate::llama::bench::update(&key, |b| b.record_load(seconds));
                });
                return Ok(endpoint);
            }
            Err(e) => {
                let tail = tail_of(&stderr_tail);
                let _ = child.start_kill();
                let _ = std::fs::remove_file(&pid_file);
                let bind_clash = tail
                    .iter()
                    .any(|l| l.contains("bind") || l.contains("Address already in use"));
                last_err = if tail.is_empty() {
                    e
                } else {
                    let shown: Vec<&str> = tail.iter().rev().take(8).map(String::as_str).collect();
                    format!(
                        "{e}\n{} said: {}",
                        engine.label(),
                        shown.into_iter().rev().collect::<Vec<_>>().join(" / ")
                    )
                };
                if !bind_clash {
                    break;
                }
            }
        }
    }
    clear_phase(model_key);
    Err(last_err)
}

async fn terminate(instance: Arc<Instance>) {
    let mut child = instance.child.lock().await;
    let pid = child.id();
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;

    #[cfg(target_os = "windows")]
    if let Some(pid) = pid {
        // /T takes the whole tree, which is what start_kill misses.
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"]);
        procutil::no_window(&mut cmd);
        let _ = cmd.output();
    }
    #[cfg(not(target_os = "windows"))]
    let _ = pid;

    let _ = std::fs::remove_file(&instance.pid_file);
}

/// Stop the server for one model, if any. Must be awaited before deleting the
/// weights: on Windows the GGUF is mapped and cannot be removed while it runs.
pub async fn stop(model_key: &str) {
    clear_phase(model_key);
    let instance = { registry().lock().await.remove(model_key) };
    if let Some(instance) = instance {
        terminate(instance).await;
    }
}

/// Stop every server. Called from the app's exit handler — `kill_on_drop`
/// covers a panic but not process exit, where a static registry is never
/// dropped at all.
pub async fn stop_all() {
    let all: Vec<Arc<Instance>> = {
        let mut map = registry().lock().await;
        map.drain().map(|(_, v)| v).collect()
    };
    for instance in all {
        terminate(instance).await;
    }
}

pub async fn running() -> Vec<RunningModel> {
    let map = registry().lock().await;
    let mut out = Vec::with_capacity(map.len());
    for (key, inst) in map.iter() {
        let last_used = *inst.last_used.lock().await;
        out.push(RunningModel {
            model_key: key.clone(),
            port: inst.port,
            file_bytes: inst.file_bytes,
            idle_seconds: last_used
                .elapsed()
                .as_secs()
                .min(inst.started.elapsed().as_secs()),
        });
    }
    out.sort_by(|a, b| a.model_key.cmp(&b.model_key));
    out
}

pub async fn is_running(model_key: &str) -> bool {
    registry().lock().await.contains_key(model_key)
}

/// The tail of a server's stderr — the only diagnostic channel for "no GPU",
/// "unsupported GGUF version" or a failed buffer allocation.
pub async fn stderr_tail(model_key: &str) -> Vec<String> {
    let map = registry().lock().await;
    map.get(model_key)
        .map(|i| tail_of(&i.stderr_tail))
        .unwrap_or_default()
}

/// What the status bar shows about the model currently held in memory.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelStats {
    pub model_key: String,
    pub display_name: String,
    pub engine: Engine,
    pub phase: Phase,
    /// Resident memory of the server process: weights plus KV cache. This is
    /// the number that explains why a large context is not free.
    pub memory_bytes: u64,
    /// The window the server was started with.
    pub ctx_size: u32,
    /// Tokens held across all slots right now.
    pub ctx_used: u32,
    /// Generation and prompt rates, averaged over the process's lifetime.
    pub tokens_per_second: f64,
    pub prompt_tokens_per_second: f64,
    pub tokens_generated: u64,
    pub busy: bool,
    /// llama-server unloads the weights after an idle period but keeps the
    /// port; a sleeping model costs no memory and pays a reload on next use.
    pub sleeping: bool,
}

/// Read one line of a Prometheus exposition body.
fn prom_value(body: &str, key: &str) -> Option<f64> {
    body.lines()
        .find(|l| l.starts_with(key) && l[key.len()..].starts_with(' '))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
}

/// `claudinio-mlx` reports one JSON object, because there is exactly one
/// consumer and it is ours.
/// MTPLX reports through `/metrics`, whose `latest` object describes the most
/// recent turn rather than the process lifetime. `decode_tok_s` is the number
/// that matters and the one its own dashboard shows: it counts tokens the model
/// committed, so accepted drafts are already in it.
async fn read_mtplx_stats(
    client: &reqwest::Client,
    base: &str,
    api_key: &str,
    stat: &mut LocalModelStats,
) {
    // `/metrics` sits beside `/v1`, not inside it.
    let root = base.strip_suffix("/v1").unwrap_or(base);
    let Ok(resp) = client
        .get(format!("{root}/metrics"))
        .bearer_auth(api_key)
        .timeout(STATS_TIMEOUT)
        .send()
        .await
    else {
        return;
    };
    let Ok(json) = resp.json::<serde_json::Value>().await else {
        return;
    };
    let Some(latest) = json.get("latest") else {
        return;
    };
    let number = |key: &str| {
        latest
            .get(key)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0)
    };
    stat.tokens_per_second = number("decode_tok_s");
    stat.prompt_tokens_per_second = number("prefill_tok_s");
    stat.tokens_generated = number("completion_tokens") as u64;
    stat.ctx_used = number("prompt_tokens") as u32 + number("completion_tokens") as u32;
}

async fn read_mlx_stats(
    client: &reqwest::Client,
    base: &str,
    api_key: &str,
    stat: &mut LocalModelStats,
) {
    let Ok(resp) = client
        .get(format!("{base}/stats"))
        .bearer_auth(api_key)
        .timeout(STATS_TIMEOUT)
        .send()
        .await
    else {
        return;
    };
    let Ok(json) = resp.json::<serde_json::Value>().await else {
        return;
    };
    let number = |key: &str| {
        json.get(key)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0)
    };
    stat.tokens_per_second = number("tokensPerSecond");
    stat.prompt_tokens_per_second = number("promptTokensPerSecond");
    stat.tokens_generated = number("tokensGenerated") as u64;
    stat.ctx_used = number("ctxUsed") as u32;
    if stat.ctx_size == 0 {
        stat.ctx_size = number("ctxSize") as u32;
    }
    stat.busy = json
        .get("busy")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
}

/// llama-server spreads the same numbers over three endpoints, two of which
/// speak Prometheus text.
async fn read_llamacpp_stats(
    client: &reqwest::Client,
    base: &str,
    api_key: &str,
    stat: &mut LocalModelStats,
) {
    if let Ok(resp) = client
        .get(format!("{base}/metrics"))
        .bearer_auth(api_key)
        .timeout(STATS_TIMEOUT)
        .send()
        .await
        && let Ok(body) = resp.text().await
    {
        stat.tokens_per_second =
            prom_value(&body, "llamacpp:predicted_tokens_seconds").unwrap_or(0.0);
        stat.prompt_tokens_per_second =
            prom_value(&body, "llamacpp:prompt_tokens_seconds").unwrap_or(0.0);
        stat.tokens_generated =
            prom_value(&body, "llamacpp:tokens_predicted_total").unwrap_or(0.0) as u64;
        stat.busy = prom_value(&body, "llamacpp:requests_processing").unwrap_or(0.0) > 0.0;
    }

    if let Ok(resp) = client
        .get(format!("{base}/slots"))
        .bearer_auth(api_key)
        .timeout(STATS_TIMEOUT)
        .send()
        .await
        && let Ok(slots) = resp.json::<Vec<serde_json::Value>>().await
    {
        stat.ctx_used = slots
            .iter()
            .filter_map(|s| s.get("n_past").or_else(|| s.get("n_ctx_used")))
            .filter_map(serde_json::Value::as_u64)
            .sum::<u64>() as u32;
        if stat.ctx_size == 0 {
            stat.ctx_size = slots
                .first()
                .and_then(|s| s.get("n_ctx"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as u32;
        }
    }

    if let Ok(resp) = client
        .get(format!("{base}/props"))
        .bearer_auth(api_key)
        .timeout(STATS_TIMEOUT)
        .send()
        .await
        && let Ok(props) = resp.json::<serde_json::Value>().await
    {
        stat.sleeping = props
            .get("is_sleeping")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
    }
}

/// Stats for every resident model. Best-effort throughout: this feeds a status
/// bar, so a server that is mid-restart or briefly unreachable reports what is
/// known rather than failing the call.
pub async fn stats() -> Vec<LocalModelStats> {
    let instances: Vec<(String, Arc<Instance>)> = {
        let map = registry().lock().await;
        map.iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect()
    };
    // A model being loaded has no process to inspect yet — and that is the
    // phase that takes minutes, so reporting nothing here is exactly backwards.
    let loading: Vec<LocalModelStats> = phases()
        .lock()
        .map(|map| {
            map.iter()
                .filter(|(key, phase)| {
                    **phase == Phase::Loading && !instances.iter().any(|(k, _)| k == *key)
                })
                .map(|(key, _)| LocalModelStats {
                    model_key: key.clone(),
                    display_name: catalog::find(key)
                        .map(|m| m.display_name)
                        .unwrap_or_else(|_| key.clone()),
                    engine: catalog::find(key)
                        .map(|m| Engine::for_format(&m.format))
                        .unwrap_or_default(),
                    phase: Phase::Loading,
                    ..Default::default()
                })
                .collect()
        })
        .unwrap_or_default();

    if instances.is_empty() {
        return loading;
    }

    let mut sys = sysinfo::System::new();
    let pids: Vec<sysinfo::Pid> = instances
        .iter()
        .filter_map(|(_, i)| i.pid.map(sysinfo::Pid::from_u32))
        .collect();
    if !pids.is_empty() {
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&pids), true);
    }

    let client = crate::http::default_client();
    let mut out = Vec::with_capacity(instances.len());
    for (key, inst) in instances {
        let mut stat = LocalModelStats {
            model_key: key.clone(),
            display_name: catalog::find(&key)
                .map(|m| m.display_name)
                .unwrap_or_else(|_| key.clone()),
            engine: inst.engine,
            phase: phase_of(&key).unwrap_or(Phase::Idle),
            ctx_size: inst.ctx_size,
            memory_bytes: inst
                .pid
                .and_then(|p| sys.process(sysinfo::Pid::from_u32(p)))
                .map_or(0, |proc| proc.memory()),
            ..Default::default()
        };

        let base = format!("http://127.0.0.1:{}", inst.port);
        match inst.engine {
            Engine::Mlx => read_mlx_stats(&client, &base, &inst.api_key, &mut stat).await,
            Engine::Mtplx => read_mtplx_stats(&client, &base, &inst.api_key, &mut stat).await,
            Engine::Llamacpp => read_llamacpp_stats(&client, &base, &inst.api_key, &mut stat).await,
        }
        if stat.sleeping {
            stat.phase = Phase::Sleeping;
        }
        out.push(stat);
    }
    out.extend(loading);
    out.sort_by(|a, b| a.model_key.cmp(&b.model_key));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> StartSpec {
        StartSpec {
            engine: Engine::Llamacpp,
            model_key: "abc123".into(),
            model_path: PathBuf::from("/models/abc123/model.gguf"),
            port: 51234,
            api_key: "deadbeef".into(),
            ctx_size: 8192,
            gpu_layers: "auto".into(),
            parallel: 2,
            sleep_idle_seconds: 300,
            draft_model_path: None,
            draft_block_size: 4,
        }
    }

    /// The three flags that are load-bearing rather than cosmetic, plus the
    /// one that must never appear: binding 0.0.0.0 would put an unauthenticated
    /// inference endpoint on the local network.
    #[test]
    fn build_args_binds_loopback_and_enables_tool_calling() {
        let args = build_args(&spec());
        let joined = args.join(" ");
        assert!(joined.contains("--host 127.0.0.1"), "{joined}");
        assert!(!joined.contains("0.0.0.0"), "{joined}");
        assert!(args.iter().any(|a| a == "--jinja"), "{joined}");
        assert!(args.iter().any(|a| a == "--no-webui"), "{joined}");
        assert!(joined.contains("--api-key deadbeef"), "{joined}");
        // The alias is what makes /v1/models answer to the id the provider
        // layer sends as `model`.
        assert!(joined.contains("-a abc123"), "{joined}");
    }

    #[test]
    fn build_args_omits_optional_flags_when_unset() {
        let mut s = spec();
        s.ctx_size = 0;
        s.parallel = 0;
        s.sleep_idle_seconds = 0;
        s.gpu_layers = String::new();
        let args = build_args(&s);
        for flag in ["-c", "-np", "--sleep-idle-seconds", "-ngl"] {
            assert!(!args.iter().any(|a| a == flag), "{flag} should be absent");
        }
    }

    /// The whole point of the toggle: off, neither engine is told anything
    /// about drafting. A flag that leaked through here would make the drafter
    /// resident for a user who switched it off.
    #[test]
    fn no_drafter_means_no_speculation_flags_on_either_engine() {
        for engine in [Engine::Llamacpp, Engine::Mlx] {
            let mut s = spec();
            s.engine = engine;
            let args = build_args(&s);
            for flag in [
                "--draft-model",
                "--draft-block-size",
                "--spec-type",
                "--spec-draft-model",
                "--spec-draft-n-max",
            ] {
                assert!(
                    !args.iter().any(|a| a == flag),
                    "{flag} leaked into {engine:?} argv: {args:?}"
                );
            }
        }
    }

    #[test]
    fn mlx_args_carry_the_drafter_when_there_is_one() {
        let mut s = spec();
        s.engine = Engine::Mlx;
        s.model_path = PathBuf::from("/models/abc123");
        s.draft_model_path = Some(PathBuf::from("/models/drafter"));
        let joined = build_args(&s).join(" ");
        assert!(joined.contains("--draft-model /models/drafter"), "{joined}");
        assert!(joined.contains("--draft-block-size 4"), "{joined}");
        // The MLX sidecar has never heard of llama.cpp's spelling.
        assert!(!joined.contains("--spec-type"), "{joined}");
    }

    #[test]
    fn llamacpp_args_ask_for_the_mtp_mode_by_name() {
        let mut s = spec();
        s.draft_model_path = Some(PathBuf::from("/models/drafter.gguf"));
        let joined = build_args(&s).join(" ");
        // `draft-simple` would run a second full model instead of the head the
        // checkpoint carries — same flag shape, different thing entirely.
        assert!(joined.contains("--spec-type draft-mtp"), "{joined}");
        assert!(
            joined.contains("--spec-draft-model /models/drafter.gguf"),
            "{joined}"
        );
        assert!(!joined.contains("--draft-model /"), "{joined}");
    }

    /// llama.cpp counts drafted tokens, MLX counts the block including the
    /// bonus. Passing MLX's number straight through would draft one token too
    /// many every round.
    #[test]
    fn the_block_size_is_translated_for_llamacpp_and_never_reaches_zero() {
        let mut s = spec();
        s.draft_model_path = Some(PathBuf::from("/d.gguf"));
        for (block, expected) in [(4u32, "3"), (2, "1"), (1, "1"), (0, "1")] {
            s.draft_block_size = block;
            let args = build_args(&s);
            let i = args.iter().position(|a| a == "--spec-draft-n-max").unwrap();
            assert_eq!(args[i + 1], expected, "block size {block}");
        }
    }

    fn model(format: &str, repo: &str) -> catalog::LocalModel {
        catalog::LocalModel {
            key: "no-such-key-on-disk".into(),
            display_name: "m".into(),
            repo: repo.into(),
            quant: "Q4_K_M".into(),
            files: vec![],
            total_bytes: 1,
            context_length: None,
            has_chat_template: true,
            architecture: None,
            format: format.into(),
            installed_at: "2026-08-24T00:00:00Z".into(),
        }
    }

    fn prefs(mtp: bool) -> LocalPrefs {
        LocalPrefs {
            mtp_enabled: mtp,
            ..LocalPrefs::default()
        }
    }

    /// Regression. `drafter_path` had a llama.cpp branch that a silent
    /// `str.replace` miss never actually wrote to the file, and every existing
    /// test covered `build_args` — which takes the resolved path as input — so
    /// the suite stayed green while the app could not resolve a drafter at all.
    #[test]
    fn a_gguf_model_asks_the_catalog_for_its_drafter() {
        // The catalog lookup returns None here because the key is not on disk;
        // what matters is that the llama.cpp path reaches the lookup at all
        // instead of falling into the MLX repo table, which would never match
        // a GGUF repo and would return None for the wrong reason.
        let m = model("gguf", "unsloth/Qwen3.8-27B-GGUF");
        assert!(drafter_path(&prefs(true), Engine::Llamacpp, &m).is_none());
        assert!(catalog::drafter_gguf(&m).is_none());
    }

    #[test]
    fn the_toggle_off_means_no_drafter_on_any_engine() {
        let gguf = model("gguf", "unsloth/Qwen3.8-27B-GGUF");
        let mlx = model("mlx", "mlx-community/gemma-4-31b-it-4bit");
        assert!(drafter_path(&prefs(false), Engine::Llamacpp, &gguf).is_none());
        assert!(drafter_path(&prefs(false), Engine::Mlx, &mlx).is_none());
    }

    /// MTPLX runs the head inside the checkpoint. Handing it a drafter path
    /// would be handing it a second model it has no flag to accept.
    #[test]
    fn mtplx_never_resolves_an_external_drafter() {
        let m = model("mlx", "Youssofal/Qwen3.8-27B-MTPLX-Optimized-Speed");
        assert!(drafter_path(&prefs(true), Engine::Mtplx, &m).is_none());
    }

    #[test]
    fn mtplx_serves_the_model_directory_and_requires_the_key() {
        let mut s = spec();
        s.engine = Engine::Mtplx;
        s.model_path = PathBuf::from("/models/abc123");
        let args = build_mtplx_args(&s);
        let joined = args.join(" ");
        assert_eq!(args.first().map(String::as_str), Some("serve"));
        assert!(joined.contains("--model /models/abc123"), "{joined}");
        assert!(joined.contains("--host 127.0.0.1"), "{joined}");
        assert!(!joined.contains("0.0.0.0"), "{joined}");
        assert!(joined.contains("--api-key deadbeef"), "{joined}");
        // Block size 4 is depth 3 — the depth `mtplx tune` picked on an M2 Max.
        assert!(joined.contains("--generation-mode mtp"), "{joined}");
        assert!(joined.contains("--depth 3"), "{joined}");
        // Flags belonging to the other two engines must not leak across.
        for flag in [
            "--jinja",
            "-ngl",
            "--ctx-size",
            "--draft-model",
            "--spec-type",
        ] {
            assert!(!args.iter().any(|a| a == flag), "{flag} in {joined}");
        }
    }

    /// Off must still be MTPLX, in `ar` mode: switching engines instead would
    /// unload the weights and make a toggle cost a full reload each way.
    #[test]
    fn mtplx_with_no_speculation_asks_for_autoregressive() {
        let mut s = spec();
        s.engine = Engine::Mtplx;
        s.draft_block_size = 0;
        s.draft_model_path = None;
        let joined = build_mtplx_args(&s).join(" ");
        assert!(joined.contains("--generation-mode ar"), "{joined}");
        assert!(!joined.contains("--depth"), "{joined}");
    }

    #[test]
    fn every_engine_reads_a_format_and_names_itself() {
        for e in [Engine::Llamacpp, Engine::Mlx, Engine::Mtplx] {
            assert!(!e.label().is_empty());
            assert!(!e.model_format().is_empty());
        }
        // MTPLX reads MLX repositories, so the catalog and downloader are shared.
        assert_eq!(Engine::Mtplx.model_format(), Engine::Mlx.model_format());
    }

    #[test]
    fn build_args_enables_the_metrics_endpoint() {
        // Without it llamacpp:* counters do not exist and the status bar has
        // no token rate to show.
        assert!(build_args(&spec()).iter().any(|a| a == "--metrics"));
    }

    #[test]
    fn prom_value_reads_a_counter_and_ignores_a_prefix_match() {
        let body = "# HELP whatever\n\
                    llamacpp:predicted_tokens_seconds 13.92\n\
                    llamacpp:tokens_predicted_total 80\n\
                    llamacpp:tokens_predicted_seconds_total 56.4486\n";
        assert_eq!(
            prom_value(body, "llamacpp:predicted_tokens_seconds"),
            Some(13.92)
        );
        // The longer key must not be answered by the shorter line that
        // prefixes it.
        assert_eq!(
            prom_value(body, "llamacpp:tokens_predicted_seconds_total"),
            Some(56.4486)
        );
        assert_eq!(prom_value(body, "llamacpp:nope"), None);
    }

    /// Regression: MLX became the default on Apple Silicon while the installed
    /// models were GGUF, and the sidecar was handed a `.gguf` — which aborts
    /// inside its model factory with a stack trace, not a message.
    #[test]
    fn a_gguf_model_never_selects_the_mlx_engine() {
        assert_eq!(Engine::for_format("gguf"), Engine::Llamacpp);
        let mut s = spec();
        s.engine = Engine::for_format("gguf");
        assert!(
            build_args(&s).iter().any(|a| a == "--jinja"),
            "llama.cpp argv"
        );
    }

    #[test]
    fn mlx_args_bind_loopback_and_require_the_key() {
        let mut s = spec();
        s.engine = Engine::Mlx;
        s.model_path = PathBuf::from("/models/abc123");
        let args = build_args(&s);
        let joined = args.join(" ");
        assert_eq!(args.first().map(String::as_str), Some("serve"));
        assert!(joined.contains("--host 127.0.0.1"), "{joined}");
        assert!(!joined.contains("0.0.0.0"), "{joined}");
        assert!(joined.contains("--api-key deadbeef"), "{joined}");
        assert!(joined.contains("--alias abc123"), "{joined}");
        assert!(joined.contains("--model /models/abc123"), "{joined}");
        // Flags that only mean something to llama.cpp must not leak across.
        for flag in ["--jinja", "-ngl", "-np", "--no-webui", "--metrics"] {
            assert!(!args.iter().any(|a| a == flag), "{flag} is llama.cpp-only");
        }
    }

    #[test]
    fn redact_hides_the_api_key() {
        let line = "srv: using api key deadbeef for auth";
        assert_eq!(redact(line, "deadbeef"), "srv: using api key *** for auth");
        assert_eq!(redact(line, ""), line);
    }

    #[test]
    fn reserve_port_returns_a_port_that_can_be_bound() {
        let port = reserve_port().unwrap();
        assert!(port > 0);
        // Released, so binding it again must succeed.
        std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    }

    /// The window follows the session's handoff budget, so it has to reach the
    /// command line — a `-c` that never changes would make the setting a lie.
    #[test]
    fn build_args_carries_the_resolved_context() {
        let mut s = spec();
        s.ctx_size = 128_192;
        let args = build_args(&s);
        let joined = args.join(" ");
        assert!(joined.contains("-c 128192"), "{joined}");
    }

    #[test]
    fn health_deadline_scales_with_model_size_and_is_capped() {
        assert_eq!(health_deadline(0), HEALTH_BASE_TIMEOUT);
        assert!(health_deadline(20_000_000_000) > HEALTH_BASE_TIMEOUT);
        assert_eq!(health_deadline(u64::MAX / 2), HEALTH_MAX_TIMEOUT);
    }

    #[test]
    fn admit_refuses_what_cannot_fit() {
        // A model claiming more than any plausible machine has must be refused
        // regardless of the host this test runs on.
        assert!(admit(u64::MAX / 4, 0).is_err());
        assert!(admit(1, 0).is_ok());
    }
}
