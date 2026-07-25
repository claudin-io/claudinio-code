//! The IPC adapter for remote access: thin on purpose.
//!
//! Everything here either translates a webview call into something `remote/`
//! already knows how to do, or implements `DeviceActions` — the seam through
//! which the bridge is allowed to touch the app.
//!
//! No policy decision is made in this file. Policy is checked in
//! `remote/bridge.rs`, in one place, before anything reaches these methods. A
//! second gate here would be a second place to get it wrong.

use serde::Serialize;
use tauri::{Emitter, State};

use crate::agent::approval::{Actor, ResolveError};
use crate::remote::bridge::{ApprovalOutcome, DeviceActions};
use crate::remote::noise::DeviceIdentity;
use crate::state::AppState;

/// What `remote/` may ask of the running app.
///
/// Holds clones rather than a `State<'_, AppState>` borrow: the bridge outlives
/// any single IPC call, so it cannot hold a Tauri state guard.
pub struct AppActions {
    approvals: crate::agent::session::ApprovalMap,
    steering: std::sync::Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<String, std::sync::Arc<crate::agent::session::SteeringCtl>>,
        >,
    >,
}

impl AppActions {
    pub fn new(state: &AppState) -> Self {
        Self {
            approvals: state.approvals.clone(),
            steering: state.steering_map(),
        }
    }
}

impl DeviceActions for AppActions {
    async fn resolve_approval(
        &self,
        session_id: &str,
        tool_use_id: &str,
        approved: bool,
        actor: Actor,
    ) -> Result<ApprovalOutcome, String> {
        let key = format!("{session_id}:{tool_use_id}");
        match self.approvals.resolve(&key, approved, actor).await {
            Ok(_) => Ok(ApprovalOutcome::Resolved),
            // Losing the race is not a failure to report as one: the gate is
            // closed, which is what the peer wanted. Handing back the winning
            // decision is what lets the bridge send `ApprovalResolved`, so the
            // remote UI closes its gate showing the answer that won instead of
            // an error for something that worked.
            Err(ResolveError::AlreadyResolved(decision)) => {
                Ok(ApprovalOutcome::AlreadyResolved(decision))
            }
            Err(ResolveError::NotFound) => Err("approval request not found".into()),
        }
    }

    async fn interrupt(&self, session_id: &str) -> Result<(), String> {
        let map = self.steering.lock().await;
        match map.get(session_id) {
            Some(ctl) => {
                ctl.interrupt
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
            None => Err("no run in progress for that session".into()),
        }
    }

    async fn send_message(&self, _session_id: &str, _text: &str) -> Result<(), String> {
        // Starting a run from a peer means going through the same path
        // `send_message` takes, including workspace resolution and the tool
        // context. That is phase 4, and the default policy denies it, so the
        // bridge never reaches here — but returning an explicit error rather
        // than silently succeeding is what keeps that true if the policy is
        // widened before the plumbing exists.
        Err("sending messages from a remote peer is not implemented yet".into())
    }

    async fn steer(&self, _session_id: &str, _text: &str) -> Result<(), String> {
        Err("steering from a remote peer is not implemented yet".into())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    /// Off until the user turns it on. Remote access is opt-in, per §6.3 step 1.
    pub enabled: bool,
    /// The device's static public key, hex. This is what goes into a pairing
    /// code, out of band — it is public by design and safe to show.
    pub device_key: Option<String>,
    /// Present when the identity could not be read, so the UI can say why rather
    /// than showing an empty panel.
    pub error: Option<String>,
}

/// Where the device identity lives.
///
/// Under the app config directory, never in a workspace — the same rule
/// `SECURITY.md` already states for credentials at rest.
fn identity_path() -> Result<std::path::PathBuf, String> {
    let dir = dirs::config_dir().ok_or_else(|| "no config directory".to_string())?;
    Ok(dir.join("claudinio-code").join("device.key"))
}

/// Read the local remote-access state. Does not create an identity: opening the
/// settings panel must not generate a key the user never asked for.
#[tauri::command]
pub async fn remote_status(_state: State<'_, AppState>) -> Result<RemoteStatus, String> {
    let path = match identity_path() {
        Ok(path) => path,
        Err(e) => {
            return Ok(RemoteStatus {
                enabled: false,
                device_key: None,
                error: Some(e),
            });
        }
    };

    if !path.exists() {
        return Ok(RemoteStatus {
            enabled: false,
            device_key: None,
            error: None,
        });
    }

    match DeviceIdentity::load_or_create(&path) {
        Ok(identity) => Ok(RemoteStatus {
            enabled: false,
            device_key: Some(identity.public_hex()),
            error: None,
        }),
        Err(e) => Ok(RemoteStatus {
            enabled: false,
            device_key: None,
            error: Some(e),
        }),
    }
}

/// Create the device identity if it does not exist, and return its public key.
///
/// Deliberately separate from `remote_status`: generating a long-term key is an
/// explicit local action, not a side effect of looking at a screen.
#[tauri::command]
pub async fn remote_create_identity(_state: State<'_, AppState>) -> Result<String, String> {
    let path = identity_path()?;
    let identity = DeviceIdentity::load_or_create(&path)?;
    Ok(identity.public_hex())
}

/// Where the pairing book lives, next to the device key.
fn pairings_path() -> Result<std::path::PathBuf, String> {
    Ok(identity_path()?
        .parent()
        .ok_or_else(|| "no config directory".to_string())?
        .join("remote")
        .join("pairings.json"))
}

/// The paired peers, for the local list.
#[tauri::command]
pub async fn remote_pairings(
    _state: State<'_, AppState>,
) -> Result<Vec<crate::remote::pairing::Pairing>, String> {
    Ok(crate::remote::pairing::Pairings::load(pairings_path()?)?
        .list()
        .to_vec())
}

/// Revoke a pairing.
///
/// Local only, and it takes effect without asking anyone: the book is on the
/// device and is consulted at handshake time, so this works with the relay
/// unreachable — which is the whole point of §6.5. A revocation that needed a
/// server would fail exactly when someone needs it.
#[tauri::command]
pub async fn remote_revoke(peer_key: String, _state: State<'_, AppState>) -> Result<(), String> {
    // The live session goes first. Writing the book only bites at the next
    // handshake, so a peer that stays connected would keep being served after being
    // revoked — which is the one moment revocation has to work. Done before the
    // write, so a failed write still leaves the peer disconnected rather than
    // connected and believed revoked.
    crate::remote::control::shared()
        .stop_peer(&peer_key, claudinio_protocol::inner::CloseReason::Revoked);

    let mut pairings = crate::remote::pairing::Pairings::load(pairings_path()?)?;
    pairings.revoke(&peer_key)
}

/// The keys that were paired and are not any more.
///
/// Shown so the local list can say "you revoked this" rather than making a
/// revoked device look like a stranger, which reads like a bug.
#[tauri::command]
pub async fn remote_revoked(_state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(crate::remote::pairing::Pairings::load(pairings_path()?)?
        .revoked()
        .to_vec())
}

/// Un-revoke a key so it can be paired again.
///
/// Deliberately separate from pairing: if pairing silently un-revoked, a peer that
/// simply kept trying would let itself back in, and revocation would be something
/// the revoked party could undo.
#[tauri::command]
pub async fn remote_unrevoke(peer_key: String, _state: State<'_, AppState>) -> Result<(), String> {
    let mut pairings = crate::remote::pairing::Pairings::load(pairings_path()?)?;
    pairings.unrevoke(&peer_key)
}

/// Rename a pairing, so the list reads as prose rather than as key material.
#[tauri::command]
pub async fn remote_rename_pairing(
    peer_key: String,
    label: String,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    let mut pairings = crate::remote::pairing::Pairings::load(pairings_path()?)?;
    pairings.rename(&peer_key, &label)
}

/// Where the policy file lives.
fn policy_path() -> Result<std::path::PathBuf, String> {
    Ok(identity_path()?
        .parent()
        .ok_or_else(|| "no config directory".to_string())?
        .join("remote-policy.json"))
}

/// Write the policy, from the local UI.
///
/// # Why the writer is here and not in `remote/policy.rs`
///
/// I4 says a peer can never widen its own policy, and §6.4 says the policy is
/// edited only through the local UI. Both are true at once because of where this
/// function lives, not because of a check inside it.
///
/// `remote/` is what the bridge can reach, and it has no writer at all — a peer's
/// command cannot call something that does not exist in its dependency graph. This
/// module is the local IPC adapter: reachable from the webview on the machine,
/// never from a frame off the wire. The boundary is the module graph, which is why
/// there is an architecture test about it rather than a comment.
#[tauri::command]
pub async fn remote_set_policy(
    policy: crate::remote::policy::StoredPolicy,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    let path = policy_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
    }
    let body =
        serde_json::to_string_pretty(&policy).map_err(|e| format!("serialize policy: {e}"))?;

    // Through a temporary and renamed, so a crash mid-write cannot leave a file
    // that fails to parse — which would lock the user out of their own remote
    // access until they found and fixed it by hand.
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &body).map_err(|e| format!("write policy: {e}"))?;
    std::fs::rename(&temp, &path).map_err(|e| format!("replace policy: {e}"))?;
    Ok(())
}

/// The effective policy, for the local editor to show.
///
/// Read-only here as everywhere: widening happens by editing the file through the
/// local UI, never through an IPC call a compromised webview could make.
#[tauri::command]
pub async fn remote_policy(_state: State<'_, AppState>) -> Result<PolicyView, String> {
    use crate::remote::policy::{Effective, Inert, StoredPolicy};

    let path = policy_path()?;
    let stored = StoredPolicy::load(&path)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let effective = stored.effective(now);
    Ok(PolicyView {
        path: path.to_string_lossy().to_string(),
        active: effective.is_active(),
        inert_because: match &effective {
            Effective::Active(_) => None,
            Effective::Nothing(Inert::Disabled) => Some("remote access is switched off".into()),
            Effective::Nothing(Inert::Expired) => Some("the grant has expired".into()),
        },
        effective: effective.wire(),
        workspaces: stored.workspaces.clone(),
        bash_denylist: stored.remote_bash_denylist(),
        stored,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyView {
    /// Shown so the user knows which file to edit — §6.4 is edited locally, by
    /// hand or through the panel, and never over IPC.
    pub path: String,
    pub active: bool,
    /// Why nothing is granted, when nothing is granted. Better than an empty
    /// panel that looks broken.
    pub inert_because: Option<String>,
    pub effective: claudinio_protocol::inner::Policy,
    pub workspaces: Vec<String>,
    pub bash_denylist: Vec<String>,
    /// What is actually on disk, so the editor can round-trip it.
    ///
    /// `effective` is what a peer gets after the switch and the expiry are applied,
    /// which is what the panel *displays* — but an editor that saved `effective`
    /// would silently rewrite a switched-off policy as a policy that grants nothing,
    /// losing everything the user had configured.
    pub stored: crate::remote::policy::StoredPolicy,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnableArgs {
    pub relay_url: String,
    /// The workspace whose active session is being served. Needed to resolve the
    /// transcript that answers `Subscribe`.
    pub workspace: String,
    /// Hex, 32 characters. Comes from the pairing exchange.
    pub channel: String,
    /// The relay's routing capability for that channel, from the same exchange.
    pub channel_token: String,
    pub session_id: String,
    /// How this peer appears in the transcript: "Safari on iPhone".
    pub peer_label: String,
    /// Set when this connection is meant to pair a new peer rather than serve an
    /// already-paired one. Opening the window is an explicit local action.
    #[serde(default)]
    pub pair_new_peer: bool,
    /// Unix millis after which the *pairing* lapses. `None` means it does not,
    /// which the local UI has to choose deliberately.
    #[serde(default)]
    pub pairing_expires_at: Option<u64>,
}

/// Start serving a paired peer for one session.
///
/// Refuses before it opens anything: an inert policy and a workspace off the
/// allowlist are both errors here rather than a channel that denies everything,
/// because a peer that connects and finds it can do nothing cannot tell that from
/// a bug.
///
/// Pairing a new peer does not finish here. The connection stops after the
/// handshake and waits for the user to compare the words on both screens; see the
/// human check in `remote/transport.rs`.
#[tauri::command]
pub async fn remote_enable(
    args: EnableArgs,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    open_connection(args, app, state).await
}

/// The body both `remote_enable` and `remote_start_pairing` run.
///
/// Shared rather than duplicated because every refusal in here — inert policy,
/// workspace off the allowlist, a session that is not the workspace's active one —
/// has to apply to pairing too. A second copy is a second place for one of them to
/// go missing, and the one that went missing would be the one that mattered.
async fn open_connection(
    args: EnableArgs,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use claudinio_protocol::wire::ChannelId;

    let channel = ChannelId::from_hex(&args.channel).map_err(|e| e.to_string())?;
    let identity = std::sync::Arc::new(DeviceIdentity::load_or_create(&identity_path()?)?);
    let bus = state.bus_for(&args.session_id).await;

    // The transcript is the source of truth for replay, so a peer can only be
    // served a session the app actually has open.
    let ws = state.workspace(&args.workspace).await?;
    let store_path = {
        let active = ws.active_session.lock().await;
        match active.as_ref() {
            Some(handle) if handle.id == args.session_id => handle.store_path.clone(),
            _ => return Err("that session is not the workspace's active one".into()),
        }
    };

    // Two refusals before anything is opened.
    //
    // An inert policy refuses outright rather than opening a channel that will deny
    // everything: a peer that connects and then finds it can do nothing cannot tell
    // that from a bug. And a workspace the policy does not list is refused here,
    // which is what makes the `workspaces` field a boundary rather than a note —
    // without this check a peer could be served any session the app had open.
    let effective_policy = {
        use crate::remote::policy::StoredPolicy;
        let policy_path = policy_path()?;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let stored = StoredPolicy::load(&policy_path)?;
        let effective = stored.effective(now_ms);
        if !effective.is_active() {
            return Err("remote access is not enabled locally; edit the policy first".into());
        }
        if !stored.allows_workspace(std::path::Path::new(&args.workspace)) {
            return Err(format!(
                "{} is not on the policy's workspace allowlist",
                args.workspace
            ));
        }
        // The policy the file actually grants, not the all-denying default. It was
        // proved active just above, so serving less than it says would be a second,
        // quieter policy nobody wrote.
        effective.wire()
    };

    let command_log = identity_path()?
        .parent()
        .ok_or_else(|| "no config directory".to_string())?
        .join("remote")
        .join(format!("{}.commands.jsonl", args.channel));

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let connection = crate::remote::transport::Connection {
        relay_url: args.relay_url,
        channel,
        channel_token: args.channel_token,
        identity,
        session_id: args.session_id,
        // The policy the file actually grants, not the all-denying default. It was
        // already proved active above, so serving less than it says would be a
        // second, quieter policy nobody wrote.
        policy: effective_policy,
        pairing_offer: args.pair_new_peer.then(|| {
            crate::remote::transport::PairingOffer {
                label: args.peer_label.clone(),
                // §6.3: a pairing code is good for 120 s. Long enough to read
                // across a desk, short enough that a screen left unlocked is not
                // an open door.
                expires_at: now + crate::remote::pairing::TOKEN_TTL_MS,
                pairing_expires_at: args.pairing_expires_at,
            }
        }),
        command_log,
        store_path,
        pairings_path: identity_path()?
            .parent()
            .ok_or_else(|| "no config directory".to_string())?
            .join("remote")
            .join("pairings.json"),
        bus,
        actions: AppActions::new(&state),
        notices: notice_sink(app),
        confirmations: crate::remote::pairing::shared().clone(),
        control: crate::remote::control::shared().clone(),
    };

    // Detached: the connection outlives this call and must survive the relay
    // being down, which is a retry loop rather than an error (I8).
    tokio::spawn(crate::remote::transport::run(connection));
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartPairingArgs {
    pub relay_url: String,
    pub workspace: String,
    /// How the new peer will appear in the transcript. Chosen before pairing
    /// rather than after, so the name is the user's rather than a browser's
    /// user-agent string.
    pub peer_label: String,
    /// When the pairing itself lapses. `None` means never, which the UI has to
    /// choose deliberately — §6.3 wants every grant to expire by default.
    #[serde(default)]
    pub pairing_expires_at: Option<u64>,
    /// Overridable for a self-hosted web app. Defaults to app.claudin.io.
    #[serde(default)]
    pub web_origin: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCodeView {
    /// What the QR encodes, shown as text too — a camera is not always the way in.
    pub url: String,
    pub channel: String,
    pub device_key: String,
    /// Unix millis. The *code* expires here; the pairing it creates does not.
    pub expires_at: u64,
    pub qr_svg: String,
}

// The channel token is deliberately absent from `PairingCodeView`. It is inside the
// URL the QR encodes, which is where the browser reads it from — and a field the
// webview could pick out separately is a field something will eventually log.

/// Open a pairing window and return the code that fills it.
///
/// One command rather than "mint a code" and "start serving", because a code
/// nobody is listening for is a QR that silently does nothing, and a listener with
/// no code is a channel waiting for a stranger.
///
/// The window closes on its own after `TOKEN_TTL_MS`. Pairing does not complete
/// here: the connection stops after the handshake and waits for the user to
/// compare the words.
#[tauri::command]
pub async fn remote_start_pairing(
    args: StartPairingArgs,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<PairingCodeView, String> {
    use crate::remote::code::{DEFAULT_WEB_ORIGIN, PairingCode};

    // Resolved here rather than passed in. The webview does not track which
    // session is active — that lives inside the chat panel — and `open_connection`
    // would refuse anything but this one anyway, so asking a caller for it is a
    // question with exactly one right answer.
    let session_id = {
        let ws = state.workspace(&args.workspace).await?;
        let active = ws.active_session.lock().await;
        match active.as_ref() {
            Some(handle) => handle.id.clone(),
            None => return Err("that workspace has no open session to share".into()),
        }
    };

    let identity = DeviceIdentity::load_or_create(&identity_path()?)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let code = PairingCode::mint(
        identity.public_hex(),
        args.relay_url.clone(),
        now + crate::remote::pairing::TOKEN_TTL_MS,
    )?;

    let origin = args.web_origin.as_deref().unwrap_or(DEFAULT_WEB_ORIGIN);
    let view = PairingCodeView {
        url: code.url(origin),
        channel: code.channel.to_hex(),
        device_key: code.device_key.clone(),
        expires_at: code.expires_at,
        qr_svg: code.qr_svg(origin)?,
    };

    // Listening starts only after the code rendered. A QR shown for a channel
    // that failed to open would be a code the user reads out for nothing.
    open_connection(
        EnableArgs {
            relay_url: args.relay_url,
            workspace: args.workspace,
            channel: view.channel.clone(),
            channel_token: code.token.clone(),
            session_id,
            peer_label: args.peer_label,
            pair_new_peer: true,
            pairing_expires_at: args.pairing_expires_at,
        },
        app,
        state,
    )
    .await?;

    Ok(view)
}

/// The event a notice arrives on in the webview.
pub const NOTICE_EVENT: &str = "remote://notice";

/// A notice as the frontend sees it: a tag and the fields, flattened, because a
/// Rust enum's default serialization is awkward to narrow on in TypeScript.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct NoticePayload {
    kind: &'static str,
    peer_key: String,
    label: Option<String>,
    /// Only on the two notices that carry words to compare.
    sas: Option<String>,
    /// Only on `stopped`. Why it stopped, so the panel can say so.
    reason: Option<&'static str>,
}

/// The wire tag for a close reason, so the panel can explain rather than just going
/// grey.
fn close_reason_tag(reason: claudinio_protocol::inner::CloseReason) -> &'static str {
    use claudinio_protocol::inner::CloseReason;
    match reason {
        CloseReason::PeerAsked => "peerAsked",
        CloseReason::TurnedOffLocally => "turnedOffLocally",
        CloseReason::GrantExpired => "grantExpired",
        CloseReason::Revoked => "revoked",
    }
}

/// Bridge `remote/`'s notices onto Tauri events.
///
/// This closure is why `Notice` is not a Tauri type: the transport emits into a
/// `Fn`, and the only code that knows about `AppHandle` is here, in the adapter
/// layer. `remote/` stays buildable — and testable — without Tauri.
fn notice_sink(app: tauri::AppHandle) -> crate::remote::transport::NoticeSink {
    use crate::remote::transport::Notice;
    std::sync::Arc::new(move |notice: Notice| {
        let payload = match notice {
            Notice::ConfirmPairing {
                peer_key,
                label,
                sas,
            } => NoticePayload {
                kind: "confirmPairing",
                peer_key,
                label: Some(label),
                sas: Some(sas),
                reason: None,
            },
            Notice::Paired { peer_key, label } => NoticePayload {
                kind: "paired",
                peer_key,
                label: Some(label),
                sas: None,
                reason: None,
            },
            Notice::PairingRefused { peer_key } => NoticePayload {
                kind: "pairingRefused",
                peer_key,
                label: None,
                sas: None,
                reason: None,
            },
            Notice::Stopped { reason } => NoticePayload {
                kind: "stopped",
                peer_key: String::new(),
                label: None,
                sas: None,
                reason: Some(close_reason_tag(reason)),
            },
            Notice::Connected {
                peer_key,
                label,
                sas,
            } => NoticePayload {
                kind: "connected",
                peer_key,
                label: Some(label),
                sas: Some(sas),
                reason: None,
            },
        };
        // A dropped event means the window went away. Nothing to recover: the
        // pairing then times out and refuses, which is the safe direction.
        let _ = app.emit(NOTICE_EVENT, payload);
    })
}

/// The user's answer to "do these three words match?".
///
/// `matched: false` revokes the key, which is stronger than not pairing: `admit`
/// has already written the pairing by the time the words are on screen, so
/// removing it would leave a key free to pair again on the next attempt — and the
/// key that failed the word check is exactly the one that must not.
///
/// Local IPC only. There is deliberately no wire command for this: a peer able to
/// confirm its own pairing is a peer that has replaced the human the check exists
/// for.
#[tauri::command]
pub async fn remote_confirm_pairing(
    peer_key: String,
    matched: bool,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    crate::remote::pairing::shared().resolve(&peer_key, matched)
}

/// Turn remote access off, now.
///
/// Every live connection is stopped and none of them redial. The pairing survives:
/// this is the switch, not a revocation. Turning it back on serves the same paired
/// browsers without a new code — which is why the peer's own "close" sends
/// `EndRemote` rather than asking to be revoked.
///
/// Works with the relay unreachable, because the stop is a local signal to a local
/// task. The same property §6.5 gives revocation, for the same reason: a switch that
/// needed a server would fail exactly when someone wanted it.
#[tauri::command]
pub async fn remote_disable(_state: State<'_, AppState>) -> Result<usize, String> {
    use claudinio_protocol::inner::CloseReason;
    Ok(crate::remote::control::shared().stop_all(CloseReason::TurnedOffLocally))
}

/// Which channels are being served.
///
/// Read from the registry rather than from a flag set when it was switched on: a
/// connection that stopped by itself — a revoked pairing, a lapsed grant — must not
/// leave the panel claiming remote access is live.
#[tauri::command]
pub async fn remote_running(_state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(crate::remote::control::shared().running())
}
