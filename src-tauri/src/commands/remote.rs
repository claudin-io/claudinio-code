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
use tauri::State;

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

/// The effective policy, for the local editor to show.
///
/// Read-only here as everywhere: widening happens by editing the file through the
/// local UI, never through an IPC call a compromised webview could make.
#[tauri::command]
pub async fn remote_policy(_state: State<'_, AppState>) -> Result<PolicyView, String> {
    use crate::remote::policy::{Effective, Inert, StoredPolicy};

    let path = identity_path()?
        .parent()
        .ok_or_else(|| "no config directory".to_string())?
        .join("remote-policy.json");

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
/// The policy handed to the bridge is `Policy::default()`, which grants nothing.
/// Editing it is phase 3 work, and until that exists this command opens a channel
/// a peer can watch and cannot act through. That ordering is deliberate: the
/// enforcement point ships before the capability, so no intermediate state grants
/// more than the finished one would.
#[tauri::command]
pub async fn remote_enable(args: EnableArgs, state: State<'_, AppState>) -> Result<(), String> {
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
        let policy_path = identity_path()?
            .parent()
            .ok_or_else(|| "no config directory".to_string())?
            .join("remote-policy.json");
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
    };

    // Detached: the connection outlives this call and must survive the relay
    // being down, which is a retry loop rather than an error (I8).
    tokio::spawn(crate::remote::transport::run(connection));
    Ok(())
}
