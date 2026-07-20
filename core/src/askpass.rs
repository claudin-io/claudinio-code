//! Askpass bridge: routes git/ssh credential prompts into the app UI.
//!
//! Problem: agent bash commands (e.g. `git push` over SSH) cannot receive
//! interactive input — ssh prompts on the controlling TTY (invisible or
//! nonexistent for a GUI app) and the command hangs until the tool timeout
//! kills it, leaving the model to guess (wrongly) why it failed.
//!
//! Mechanism: the app runs a loopback-only HTTP listener with a secret token.
//! Agent shell commands get `SSH_ASKPASS`/`GIT_ASKPASS` pointed at a tiny
//! shell script that curls the prompt text to this listener. The listener
//! emits an `askpass-request` Tauri event; the frontend shows a password
//! modal; the reply (or cancellation) is resolved back to the waiting HTTP
//! request, whose body becomes the secret ssh/git reads from the helper's
//! stdout.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

pub const EVENT_NAME: &str = "askpass-request";
/// How long a prompt waits for the user before failing the git/ssh command.
const ANSWER_TIMEOUT_SECS: u64 = 300;

/// Observador de um novo prompt `(id, prompt)`. O app Tauri emite o evento
/// `askpass-request` e mostra o modal; o CLI mostra um prompt no terminal.
pub type AskpassObserver = Box<dyn Fn(u64, String) + Send + Sync>;

static OBSERVER: OnceLock<AskpassObserver> = OnceLock::new();
static BRIDGE: OnceLock<Bridge> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// Number of prompts currently waiting on the user — the bash tool pauses its
/// command timeout while this is nonzero so the command isn't killed mid-type.
static PENDING: AtomicUsize = AtomicUsize::new(0);

struct Bridge {
    url: String,
    token: String,
    script_path: std::path::PathBuf,
    waiting: Mutex<HashMap<u64, oneshot::Sender<Option<String>>>>,
}

/// Registra o observador de prompts (uma vez, no startup do frontend).
pub fn set_observer(observer: AskpassObserver) {
    let _ = OBSERVER.set(observer);
}

pub fn pending_count() -> usize {
    PENDING.load(Ordering::SeqCst)
}

/// Environment variables that make git/ssh route every credential prompt
/// through the bridge instead of a (possibly nonexistent) terminal. Returns
/// an empty list if the bridge failed to start — commands then run without
/// prompting (`GIT_TERMINAL_PROMPT=0` is always included so they fail fast
/// instead of hanging).
pub fn env_for_child() -> Vec<(String, String)> {
    let mut vars = vec![
        // git's own username/password prompts must never go to a terminal.
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
    ];
    if let Some(b) = BRIDGE.get() {
        let script = b.script_path.to_string_lossy().to_string();
        vars.push(("SSH_ASKPASS".into(), script.clone()));
        vars.push(("GIT_ASKPASS".into(), script));
        // "force": use askpass even when a TTY exists (dev runs from a
        // terminal) and without requiring DISPLAY.
        vars.push(("SSH_ASKPASS_REQUIRE".into(), "force".into()));
        vars.push(("CLAUDINIO_ASKPASS_URL".into(), b.url.clone()));
        vars.push(("CLAUDINIO_ASKPASS_TOKEN".into(), b.token.clone()));
        // Older OpenSSH only honors SSH_ASKPASS when DISPLAY is set.
        if std::env::var_os("DISPLAY").is_none() {
            vars.push(("DISPLAY".into(), ":0".into()));
        }
    }
    vars
}

/// Resolve a pending prompt from the UI. `secret: None` means the user
/// cancelled — the helper exits nonzero and git/ssh abort immediately.
/// O wrapper `#[tauri::command]` vive no desktop e chama esta função.
pub fn answer_askpass(id: u64, secret: Option<String>) {
    if let Some(b) = BRIDGE.get() {
        let tx = b.waiting.lock().ok().and_then(|mut m| m.remove(&id));
        if let Some(tx) = tx {
            let _ = tx.send(secret);
        }
    }
}

/// Conveniência para o CLI: spawna `serve()` na runtime tokio atual. O desktop
/// spawna `serve()` diretamente na runtime do Tauri (ver lib.rs).
pub fn start() {
    tokio::spawn(async {
        if let Err(e) = serve().await {
            eprintln!("[askpass] bridge unavailable: {e}");
        }
    });
}

/// Bind do listener loopback + escrita do helper + loop de accept. Deve ser
/// chamada dentro de uma runtime tokio (o desktop usa `tauri::async_runtime`).
pub async fn serve() -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("bind: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();

    let token: String = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
            ^ (std::process::id() as u128) << 64;
        format!("{seed:032x}")
    };

    let script_path = write_helper_script()?;

    BRIDGE
        .set(Bridge {
            url: format!("http://127.0.0.1:{port}/askpass"),
            token: token.clone(),
            script_path,
            waiting: Mutex::new(HashMap::new()),
        })
        .map_err(|_| "askpass bridge already started".to_string())?;

    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        tokio::spawn(async move {
            let _ = handle_conn(stream).await;
        });
    }
}

/// POSIX-sh helper used as SSH_ASKPASS/GIT_ASKPASS. Works on macOS/Linux and
/// under Git for Windows (whose git/ssh exec shell scripts via their bundled
/// sh; curl ships with both). The prompt text arrives as $1.
fn write_helper_script() -> Result<std::path::PathBuf, String> {
    let path = std::env::temp_dir().join("claudinio-askpass.sh");
    let script = "#!/bin/sh\n\
        # Claudinio Code askpass bridge - forwards git/ssh prompts to the app UI.\n\
        [ -n \"$CLAUDINIO_ASKPASS_URL\" ] || exit 1\n\
        exec curl -fsS --max-time 310 \\\n\
        \x20\x20-H \"X-Claudinio-Askpass-Token: $CLAUDINIO_ASKPASS_TOKEN\" \\\n\
        \x20\x20--data-binary \"$*\" \"$CLAUDINIO_ASKPASS_URL\"\n";
    std::fs::write(&path, script).map_err(|e| format!("write helper: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("chmod helper: {e}"))?;
    }
    Ok(path)
}

async fn handle_conn(mut stream: tokio::net::TcpStream) -> Result<(), String> {
    // Minimal HTTP request parse: headers, then Content-Length body bytes.
    let mut raw = Vec::with_capacity(1024);
    let mut buf = [0u8; 1024];
    let (headers, mut body) = loop {
        let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("closed before headers".into());
        }
        raw.extend_from_slice(&buf[..n]);
        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&raw[..pos]).to_string();
            break (headers, raw[pos + 4..].to_vec());
        }
        if raw.len() > 64 * 1024 {
            return Err("headers too large".into());
        }
    };

    let header = |name: &str| -> Option<String> {
        headers.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    };

    let expected = BRIDGE.get().map(|b| b.token.as_str()).unwrap_or("");
    if header("X-Claudinio-Askpass-Token").as_deref() != Some(expected) || expected.is_empty() {
        let _ = respond(&mut stream, 403, "forbidden").await;
        return Err("bad token".into());
    }

    let content_length: usize = header("Content-Length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        .min(16 * 1024);
    while body.len() < content_length {
        let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    let prompt = String::from_utf8_lossy(&body[..body.len().min(content_length)])
        .trim()
        .to_string();

    // Register the pending prompt and ask the UI.
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel::<Option<String>>();
    if let Some(b) = BRIDGE.get() {
        if let Ok(mut m) = b.waiting.lock() {
            m.insert(id, tx);
        }
    }
    PENDING.fetch_add(1, Ordering::SeqCst);
    if let Some(obs) = OBSERVER.get() {
        obs(id, prompt);
    }

    let answer = tokio::time::timeout(
        std::time::Duration::from_secs(ANSWER_TIMEOUT_SECS),
        rx,
    )
    .await;
    PENDING.fetch_sub(1, Ordering::SeqCst);
    if let Some(b) = BRIDGE.get() {
        if let Ok(mut m) = b.waiting.lock() {
            m.remove(&id);
        }
    }

    match answer {
        Ok(Ok(Some(secret))) => respond(&mut stream, 200, &secret).await,
        _ => respond(&mut stream, 403, "cancelled").await,
    }
}

async fn respond(stream: &mut tokio::net::TcpStream, status: u16, body: &str) -> Result<(), String> {
    let reason = if status == 200 { "OK" } else { "Forbidden" };
    let msg = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(msg.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let _ = stream.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn ensure_bridge() -> &'static Bridge {
        // serve loops forever accepting; run it in the background once.
        if BRIDGE.get().is_none() {
            tokio::spawn(async {
                let _ = serve().await;
            });
        }
        for _ in 0..100 {
            if let Some(b) = BRIDGE.get() {
                return b;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("bridge did not start");
    }

    async fn request(addr: &str, token: &str, prompt: &str) -> String {
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let req = format!(
            "POST /askpass HTTP/1.1\r\nHost: x\r\nX-Claudinio-Askpass-Token: {token}\r\nContent-Length: {}\r\n\r\n{prompt}",
            prompt.len()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out).await;
        String::from_utf8_lossy(&out).to_string()
    }

    fn bridge_addr(b: &Bridge) -> String {
        b.url
            .trim_start_matches("http://")
            .trim_end_matches("/askpass")
            .to_string()
    }

    // One test covers both scenarios: the accept loop lives on this test's
    // runtime (BRIDGE is a process-global), so a second #[tokio::test] would
    // race against this runtime being torn down.
    #[tokio::test]
    async fn rejects_bad_token_and_answers_prompt() {
        let b = ensure_bridge().await;

        // Bad token → 403, prompt never reaches the UI.
        let resp = request(&bridge_addr(b), "wrong-token", "Enter passphrase:").await;
        assert!(resp.starts_with("HTTP/1.1 403"), "got: {resp}");
        // Answerer: waits for the prompt to register, then resolves it like
        // the UI would (answer_askpass command).
        tokio::spawn(async {
            for _ in 0..200 {
                let id = BRIDGE
                    .get()
                    .and_then(|b| b.waiting.lock().ok())
                    .and_then(|m| m.keys().next().copied());
                if let Some(id) = id {
                    assert!(pending_count() > 0);
                    answer_askpass(id, Some("s3cret".into()));
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });
        let token = b.token.clone();
        let resp = request(&bridge_addr(b), &token, "Enter passphrase for key:").await;
        assert!(resp.starts_with("HTTP/1.1 200"), "got: {resp}");
        assert!(resp.ends_with("s3cret"), "got: {resp}");
    }
}
