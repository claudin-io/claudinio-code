//! The device half of the phase-3 prova real.
//!
//! # What this is for
//!
//! Everything else about remote access is tested in units, and units are exactly what
//! missed the bug that mattered: the device attached to the relay without a channel
//! token, so nothing could ever have connected, while both sides' suites stayed green.
//! Two things that are each correct alone are not a working system.
//!
//! So this drives the **real** device stack — `transport::run`, `noise`, `bridge`,
//! `dedup`, `policy`, `pairing`, `control` — against a real relay, and lets the real
//! browser modules connect to it. The only double is `DeviceActions`, the four-method
//! seam through which the bridge touches the running app; going further would mean
//! standing up Tauri, and nothing on the other side of that seam is on the wire.
//!
//! It is a test rather than an example because `remote` is a private module: exposing
//! it publicly to make a harness reachable would widen the crate's surface for the
//! benefit of a script.
//!
//! # Running it
//!
//! Not in CI. It needs a relay listening, so it is `#[ignore]`d and driven by
//! `scripts/prova-real-remote.sh`, which brings up the relay, runs this, runs the
//! browser peer against it, and checks the claims.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::agent::approval::Actor;
use crate::agent::eventbus::EventBus;
use crate::remote::bridge::{ApprovalOutcome, DeviceActions};
use crate::remote::code::PairingCode;
use crate::remote::control::Control;
use crate::remote::noise::DeviceIdentity;
use crate::remote::pairing::Confirmations;
use crate::remote::policy::StoredPolicy;
use crate::remote::transport::{Connection, Notice, PairingOffer};

/// Stands in for the running app.
///
/// Records what was asked rather than doing it. Phase 3 is read-only so nothing should
/// reach these at all — a call here is a finding, not a step, which is why they are
/// counted and printed rather than silently succeeding.
struct Recorded {
    calls: std::sync::Mutex<Vec<String>>,
}

impl DeviceActions for Recorded {
    async fn resolve_approval(
        &self,
        session_id: &str,
        tool_use_id: &str,
        approved: bool,
        actor: Actor,
    ) -> Result<ApprovalOutcome, String> {
        self.note(format!(
            "resolve_approval {session_id} {tool_use_id} {approved} {actor:?}"
        ));
        Ok(ApprovalOutcome::Resolved)
    }

    async fn send_message(&self, session_id: &str, text: &str) -> Result<(), String> {
        self.note(format!("send_message {session_id} {}", text.len()));
        Ok(())
    }

    async fn steer(&self, session_id: &str, text: &str) -> Result<(), String> {
        self.note(format!("steer {session_id} {}", text.len()));
        Ok(())
    }

    async fn interrupt(&self, session_id: &str) -> Result<(), String> {
        self.note(format!("interrupt {session_id}"));
        Ok(())
    }
}

impl Recorded {
    fn new() -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn note(&self, what: String) {
        eprintln!("[device] action requested: {what}");
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(what);
        }
    }

    fn count(&self) -> usize {
        self.calls.lock().map(|c| c.len()).unwrap_or(0)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A transcript on disk, which is what `Subscribe` is answered from.
///
/// Written in the shape `SessionRecord` actually deserialises, verified against a real
/// session file rather than guessed. The first version of this guessed — camelCase keys
/// and a string `content` — and every line was silently dropped, so the peer got an
/// empty snapshot and said so without anything erroring. Which is a finding in itself:
/// `replay` skips a line it cannot parse without a word, so a corrupted transcript is
/// served short rather than reported. Recorded in the plan's deviations; not changed
/// here, because the local app reads through the same function.
fn write_transcript(path: &Path, records: usize) {
    use std::io::Write;
    let mut file = std::fs::File::create(path).expect("create transcript");
    let ts = now_ms();
    writeln!(
        file,
        r#"{{"kind":"meta","session_id":"prova-real","created_at":{ts},"workspace":null}}"#
    )
    .unwrap();
    for i in 1..records.saturating_sub(1) {
        writeln!(file, r#"{{"kind":"user","text":"record {i}","ts":{ts}}}"#).unwrap();
    }

    // One real edit, so the browser's diff path is exercised end to end. The transcript
    // carries `old_string` and `new_string` and no diff at all, and computing one from
    // them in the browser is what §7's "a human reads the change before approving"
    // rests on. Shaped exactly as a session file writes an `edit_file` call.
    if records >= 2 {
        let edit = concat!(
            r#"{"kind":"turn","role":"assistant","content":[{"type":"tool_use","#,
            r#""id":"toolu_prova","name":"edit_file","input":{"path":"src/main.rs","#,
            r#""old_string":"let timeout = 30;\nreturn go(timeout);","#,
            r#""new_string":"let timeout = 60;\nreturn go(timeout, 2);"}}],"ts":"#
        );
        writeln!(file, "{edit}{ts}}}").unwrap();
    }
}

/// A policy that grants the read path and nothing else.
///
/// Written as JSON and loaded back through `StoredPolicy::load`, so the file format is
/// exercised too rather than a struct being handed straight to the transport.
fn write_policy(path: &Path, workspace: &str) {
    let policy = serde_json::json!({
        "enabled": true,
        "expires_at": now_ms() + 600_000,
        "workspaces": [workspace],
        "idle_disconnect_minutes": 30,
        "allow": {
            "send_message": false,
            "steer": false,
            "interrupt": false,
            "set_mode": false,
            "approve_edit": false,
            "approve_bash": "never",
            "read_attachment": false,
            "export_file": false
        },
        "bash_remote_denylist_extra": [],
        "max_unattended_minutes": 60
    });
    std::fs::write(path, serde_json::to_string_pretty(&policy).unwrap()).expect("write policy");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a relay; run through scripts/prova-real-remote.sh"]
async fn serve_a_real_browser_peer() {
    let relay_url = std::env::var("RELAY_URL").expect("RELAY_URL");
    let dir = PathBuf::from(std::env::var("PROVA_DIR").expect("PROVA_DIR"));
    let records: usize = std::env::var("PROVA_RECORDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);

    std::fs::create_dir_all(&dir).unwrap();
    let transcript = dir.join("transcript.jsonl");
    write_transcript(&transcript, records);
    let readable = crate::agent::persist::replay(&transcript, 1).expect("replay the transcript");
    assert_eq!(
        readable.len(),
        records,
        "the transcript this test wrote is not in a shape SessionRecord parses, so the \
         peer would receive an empty snapshot and the failure would look like transport"
    );
    println!("PROVA_DEVICE_TRANSCRIPT_RECORDS={}", readable.len());

    let policy_path = dir.join("remote-policy.json");
    write_policy(&policy_path, dir.to_str().unwrap());

    // The real identity, the real policy file, the real pairing book.
    let identity = Arc::new(DeviceIdentity::load_or_create(&dir.join("device.key")).unwrap());
    let stored = StoredPolicy::load(&policy_path).unwrap();
    let effective = stored.effective(now_ms());
    assert!(
        effective.is_active(),
        "the policy this test wrote is inert, so nothing would be served"
    );

    let code = PairingCode::mint(identity.public_hex(), relay_url.clone(), now_ms() + 120_000)
        .expect("mint a pairing code");

    // Machine-readable, because the script reads these and hands them to the browser.
    println!("PROVA_CHANNEL={}", code.channel.to_hex());
    println!("PROVA_TOKEN={}", code.token);
    println!("PROVA_DEVICE_KEY={}", code.device_key);
    println!("PROVA_RELAY_URL={relay_url}");
    println!("PROVA_URL={}", code.url("https://app.claudin.io"));

    let confirmations = Arc::new(Confirmations::new());
    let control = Arc::new(Control::new());
    // Leaked so the spawned connection can hold it for 'static, while this test keeps
    // reading the count. A test process that is about to exit is the one place where
    // leaking a few bytes is cheaper than an Arc wrapper and a second trait impl.
    let actions: &'static Recorded = Box::leak(Box::new(Recorded::new()));

    // The word check, done for real: the SAS is printed, and the connection stops here
    // until the script has compared it against the browser's and written the file. If
    // they differ the script writes nothing, this times out, and the pairing is
    // refused — which is the behaviour, not a shortcut around it.
    let confirm_flag = dir.join("confirmed");
    let _ = std::fs::remove_file(&confirm_flag);
    let waiting = Arc::clone(&confirmations);
    let flag = confirm_flag.clone();
    let notices: crate::remote::transport::NoticeSink =
        Arc::new(move |notice: Notice| match notice {
            Notice::ConfirmPairing { peer_key, sas, .. } => {
                println!("PROVA_DEVICE_SAS={sas}");
                let waiting = Arc::clone(&waiting);
                let flag = flag.clone();
                std::thread::spawn(move || {
                    let deadline = Instant::now() + Duration::from_secs(60);
                    while Instant::now() < deadline {
                        if flag.exists() {
                            println!("PROVA_DEVICE_CONFIRMED=1");
                            let _ = waiting.resolve(&peer_key, true);
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    println!("PROVA_DEVICE_CONFIRMED=0");
                    let _ = waiting.resolve(&peer_key, false);
                });
            }
            Notice::Paired { label, .. } => println!("PROVA_DEVICE_PAIRED={label}"),
            Notice::PairingRefused { .. } => println!("PROVA_DEVICE_REFUSED=1"),
            Notice::Connected { label, .. } => println!("PROVA_DEVICE_CONNECTED={label}"),
            Notice::Stopped { reason } => println!("PROVA_DEVICE_STOPPED={reason:?}"),
        });

    let connection = Connection {
        relay_url,
        channel: code.channel,
        channel_token: code.token.clone(),
        identity,
        session_id: "prova-real".to_string(),
        policy: effective.wire(),
        command_log: dir.join("commands.jsonl"),
        store_path: transcript,
        pairings_path: dir.join("pairings.json"),
        pairing_offer: Some(PairingOffer {
            label: "the prova real's browser".to_string(),
            expires_at: now_ms() + crate::remote::pairing::TOKEN_TTL_MS,
            pairing_expires_at: Some(now_ms() + 600_000),
        }),
        bus: EventBus::new(1024),
        actions,
        notices,
        confirmations,
        control: Arc::clone(&control),
    };

    let channel_hex = code.channel.to_hex();
    let serving = tokio::spawn(crate::remote::transport::run(connection));

    // The script signals it is finished by writing this, so the device does not have to
    // guess how long the browser needs.
    let done_flag = dir.join("peer-done");
    let _ = std::fs::remove_file(&done_flag);
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline && !done_flag.exists() {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    println!("PROVA_DEVICE_ACTIONS_CALLED={}", actions.count());
    println!(
        "PROVA_DEVICE_STILL_RUNNING={}",
        control.running().contains(&channel_hex)
    );

    // The pairing book should now name the browser, written by the real `admit` path.
    let book = crate::remote::pairing::Pairings::load(dir.join("pairings.json")).unwrap();
    println!("PROVA_DEVICE_PAIRINGS={}", book.list().len());
    for pairing in book.list() {
        println!("PROVA_DEVICE_PAIRED_KEY={}", pairing.peer_key);
    }

    // Read-only means read-only: nothing the browser did should have reached the app.
    assert_eq!(
        actions.count(),
        0,
        "the read-only peer reached the app through DeviceActions"
    );

    control.stop_all(claudinio_protocol::inner::CloseReason::TurnedOffLocally);
    let _ = tokio::time::timeout(Duration::from_secs(5), serving).await;
    println!("PROVA_DEVICE_DONE=1");
}
