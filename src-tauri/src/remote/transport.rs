//! The outbound connection to the relay.
//!
//! Outbound only, which is invariant I5: no listener is opened on the
//! developer's machine, so remote access works behind NAT and CGNAT with no port
//! forwarding and adds no attack surface to the LAN.
//!
//! Nothing here is allowed to matter to the agent. The relay being unreachable is
//! a discreet offline state, never an error the run can see — I8 says the app is
//! fully functional without any of this.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use claudinio_protocol::inner::{DeviceToPeer, PeerToDevice, Policy};
use claudinio_protocol::wire::{self, ChannelId, OuterFrame, OuterKind};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use super::bridge::{Bridge, DeviceActions};
use super::dedup::CommandLog;
use super::noise::{self, DeviceIdentity};
use super::pairing::{Confirmations, Pairings, Refusal};
use crate::agent::eventbus::EventBus;

/// How often session keys are rolled. §6.1.
///
/// A shared constant, not a negotiated value: both ends run it off their own
/// clock, so a mismatch would desynchronise the session rather than degrade it.
pub const REKEY_INTERVAL: Duration = Duration::from_secs(3600);

/// Everything a connection needs. Bundled because threading nine arguments
/// through a reconnect loop is how one of them ends up stale.
///
/// Note what is *not* here: the peer's label. It comes from the pairing book at
/// handshake time, so a caller cannot pass one that disagrees with what the user
/// named the device — and the transcript records the name the user chose rather
/// than whatever the connection was started with.
pub struct Connection<A: DeviceActions + Sync> {
    pub relay_url: String,
    pub channel: ChannelId,
    pub identity: Arc<DeviceIdentity>,
    pub session_id: String,
    pub policy: Policy,
    pub command_log: PathBuf,
    /// The session's JSONL. `Subscribe` is answered from this file, which is why
    /// the relay needs no durable storage of its own.
    pub store_path: PathBuf,
    /// The pairing book, consulted at handshake time. On the device, so revocation
    /// works with the relay unreachable — §6.5.
    pub pairings_path: PathBuf,
    /// Set only while the user is actively pairing a new peer.
    ///
    /// Without a window like this a device could never accept a first-time key,
    /// and with a permanent one it would accept every key. The window is opened by
    /// an explicit local action and closes on its own.
    pub pairing_offer: Option<PairingOffer>,
    pub bus: EventBus,
    pub actions: A,
    /// Where the words go so a human can compare them. Never the peer.
    pub notices: NoticeSink,
    /// The registry that human's answer comes back through.
    pub confirmations: Arc<Confirmations>,
}

/// Something the local user needs to see. Deliberately not on the wire: a peer
/// that learned it had failed the word check could retry until the user clicked
/// through, and a peer told it was "revoked" rather than "unknown" learns whether
/// it was ever paired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Two screens are showing three words each and nothing is served until the
    /// user says they agree.
    ConfirmPairing {
        peer_key: String,
        label: String,
        sas: String,
    },
    Paired {
        peer_key: String,
        label: String,
    },
    /// The words did not match, or nobody answered. The key is revoked by the
    /// time this arrives.
    PairingRefused {
        peer_key: String,
    },
    /// An already-paired peer connected. The SAS travels with it so the user can
    /// check it if they want to; there is nothing to confirm, because the key was
    /// vouched for when it was paired.
    Connected {
        peer_key: String,
        label: String,
        sas: String,
    },
}

/// How notices reach the UI. A closure rather than a Tauri handle, so this module
/// stays free of Tauri: the command layer, which has the handle, supplies one.
pub type NoticeSink = Arc<dyn Fn(Notice) + Send + Sync>;

/// How one turn at serving ended, and whether to try again.
#[derive(Debug, PartialEq, Eq)]
enum Served {
    /// The peer went away. It may come back, so the channel is redialled.
    PeerLeft,
    /// Do not retry. The pairing was refused, which revoked the key — every
    /// further attempt would be turned away by `admit`, so retrying would be an
    /// endless loop whose only effect is noise in the log.
    Abandoned,
}

/// An open invitation for one new peer.
///
/// The relay already refuses an attach without the channel token, so a peer that
/// got this far holds it. What this adds is the device's own consent: a first-time
/// key is admitted once, inside a window the user opened, and the human check is
/// the SAS both screens show.
#[derive(Clone)]
pub struct PairingOffer {
    /// What to call the peer once it is paired.
    pub label: String,
    /// Unix millis. §6.3 gives a pairing code 120 s.
    pub expires_at: u64,
    /// How long the pairing itself lasts. `None` is a deliberate local choice.
    pub pairing_expires_at: Option<u64>,
}

/// Dial the relay and serve one peer, reconnecting for as long as the caller
/// keeps the task alive.
///
/// Never returns an error to its caller: the relay being unreachable is an
/// offline state, not a failure the app should surface as one (I8). Failures are
/// logged and retried.
pub async fn run<A: DeviceActions + Sync>(connection: Connection<A>) {
    let mut backoff = Backoff::default();

    loop {
        match serve_once(&connection).await {
            Ok(Served::PeerLeft) => {
                // A clean close means the peer went away, not that anything is
                // wrong, so the next attempt starts from the bottom of the curve.
                backoff.reset();
            }
            Ok(Served::Abandoned) => return,
            Err(e) => {
                eprintln!("[remote] connection ended: {e}");
            }
        }

        let delay = backoff.next_delay();
        eprintln!("[remote] reconnecting in {delay:?}");
        tokio::time::sleep(delay).await;
    }
}

/// One connection, from dial to disconnect.
async fn serve_once<A: DeviceActions + Sync>(connection: &Connection<A>) -> Result<Served, String> {
    let url = format!(
        "{}?channel={}&role=device",
        connection.relay_url, connection.channel
    );
    let (socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| format!("dial relay: {e}"))?;
    let (mut sink, mut stream) = socket.split();

    // --- handshake --------------------------------------------------------
    //
    // The device is the responder, so it waits indefinitely. Only the initiator
    // retries — bounding both sides makes them wake and sleep alternately and miss
    // each other, which the relay's prova real demonstrated. A device with no peer
    // sits attached and idle, and that costs nothing.
    let (kind, msg1) = next_frame(&mut stream, connection.channel)
        .await?
        .ok_or_else(|| "closed before the handshake".to_string())?;
    if !matches!(kind, OuterKind::Hello) {
        return Err("first frame was not a handshake".into());
    }

    let (mut session, msg2) = noise::accept(&connection.identity, &msg1)?;
    let peer_label = admit(
        &connection.pairings_path,
        &session,
        connection.pairing_offer.as_ref(),
    )?;
    send_frame(
        &mut sink,
        connection.channel,
        OuterKind::HelloAck,
        0,
        0,
        msg2,
    )
    .await?;

    // --- the human check --------------------------------------------------
    //
    // IK already makes a relay that substitutes its own static key fail the
    // handshake, because the browser dialled with a key it held beforehand. What
    // IK cannot see is that key having been wrong from the start — a doctored QR,
    // a code read off someone else's screen. The SAS closes that: three words off
    // the handshake hash, identical on both ends only if the handshake was
    // genuinely end to end.
    //
    // So the comparison has to gate serving, and until now it did not: the words
    // went to stderr and the channel opened regardless, which made them
    // decoration. A peer waits here, attached and served nothing.
    let peer_key = peer_key_hex(&session)?;
    if connection.pairing_offer.is_some() {
        let answer = connection.confirmations.expect(&peer_key);
        (connection.notices)(Notice::ConfirmPairing {
            peer_key: peer_key.clone(),
            label: peer_label.clone(),
            sas: session.sas(),
        });

        if !Confirmations::wait(answer).await {
            connection.confirmations.forget(&peer_key);
            // Revoked rather than merely forgotten. `admit` has already written the
            // pairing, so removing it would leave a key that pairs again on the
            // next attempt — and a key that failed the word check is precisely the
            // one that must not.
            Pairings::load(&connection.pairings_path)?.revoke(&peer_key)?;
            (connection.notices)(Notice::PairingRefused {
                peer_key: peer_key.clone(),
            });
            eprintln!("[remote] pairing refused at the word check; key revoked");
            return Ok(Served::Abandoned);
        }

        (connection.notices)(Notice::Paired {
            peer_key: peer_key.clone(),
            label: peer_label.clone(),
        });
    } else {
        (connection.notices)(Notice::Connected {
            peer_key: peer_key.clone(),
            label: peer_label.clone(),
            sas: session.sas(),
        });
    }

    let mut bridge = Bridge::new(
        connection.session_id.clone(),
        peer_label.clone(),
        connection.policy.clone(),
        CommandLog::open(&connection.command_log),
        &connection.actions,
        &connection.store_path,
    );

    let mut watcher = connection.bus.subscribe();
    let mut out_seq = 0u64;

    // §6.1 rekeys hourly, so a session key recovered from a memory dump stops
    // being useful an hour later rather than for the life of the connection.
    let mut rekey = tokio::time::interval(REKEY_INTERVAL);
    rekey.tick().await; // the first tick completes immediately

    // --- serve ------------------------------------------------------------
    loop {
        tokio::select! {
            // Both ends advance on their own clock and nothing is sent, which is
            // why the interval is a shared constant rather than negotiated.
            _ = rekey.tick() => session.rekey(),

            // Events from the agent, outbound.
            delivery = watcher.recv() => {
                let Some(message) = bridge.translate(delivery) else {
                    // The bus closed: the session ended. Leave cleanly so the
                    // peer sees a close rather than a stall.
                    return Ok(Served::PeerLeft);
                };
                out_seq += 1;
                send_message(&mut sink, connection.channel, &mut session, out_seq, &message).await?;
            }

            // Commands from the peer, inbound.
            frame = next_frame(&mut stream, connection.channel) => {
                let Some((kind, payload)) = frame? else { return Ok(Served::PeerLeft) };

                // A `Hello` on an established session means the peer gave up
                // mid-handshake and came back. Treating it as transport data —
                // which is what ignoring `kind` does — fails to decrypt and then
                // rejects every frame after it, leaving a session that cannot
                // recover until one side restarts. The relay's prova real found
                // exactly that; `kind` exists so it is distinguishable.
                if matches!(kind, OuterKind::Hello) {
                    let (fresh, msg2) = noise::accept(&connection.identity, &payload)?;
                    // Checked again, not just on the first handshake: a peer
                    // revoked while it was disconnected must not be let back in by
                    // reconnecting.
                    // No pairing offer on a re-handshake: an offer is consumed by
                    // the peer it paired. Otherwise a single pairing window could
                    // admit a different key on every reconnect.
                    admit(&connection.pairings_path, &fresh, None)?;
                    send_frame(&mut sink, connection.channel, OuterKind::HelloAck, 0, 0, msg2)
                        .await?;
                    (connection.notices)(Notice::Connected {
                        peer_key: peer_key_hex(&fresh)?,
                        label: peer_label.clone(),
                        sas: fresh.sas(),
                    });
                    session = fresh;
                    out_seq = 0;
                    continue;
                }

                let ciphertext = payload;
                let plaintext = match session.decrypt(&ciphertext) {
                    Ok(plaintext) => plaintext,
                    Err(e) => {
                        // A frame that fails to authenticate was tampered with
                        // or replayed. Log it and keep the session: dropping the
                        // connection would let a hostile relay disconnect a peer
                        // at will.
                        eprintln!("[remote] rejected a frame: {e}");
                        continue;
                    }
                };

                let command: PeerToDevice = match rmp_serde::from_slice(&plaintext) {
                    Ok(command) => command,
                    Err(e) => {
                        eprintln!("[remote] undecodable command: {e}");
                        continue;
                    }
                };

                for reply in bridge.handle(command).await {
                    out_seq += 1;
                    send_message(&mut sink, connection.channel, &mut session, out_seq, &reply).await?;
                }
            }
        }
    }
}

/// The peer's static public key, hex — the form the pairing book and the UI both
/// use. A peer that completed IK always sent one; the `None` case is a handshake
/// that is not what it claims to be.
fn peer_key_hex(session: &noise::Session) -> Result<String, String> {
    let key = session
        .peer_static()
        .ok_or_else(|| "the peer sent no static key".to_string())?;
    Ok(key.iter().map(|b| format!("{b:02x}")).collect())
}

/// Is this peer still allowed in, and under what name?
///
/// Consulted after the handshake, because the peer's static key only becomes known
/// once it completes. Reading the book from disk on every handshake rather than
/// caching it is deliberate: a revocation made a moment ago has to take effect on
/// the next connection, and a cache is how it would not.
fn admit(
    pairings_path: &PathBuf,
    session: &noise::Session,
    offer: Option<&PairingOffer>,
) -> Result<String, String> {
    let key_hex = peer_key_hex(session)?;

    let mut pairings = Pairings::load(pairings_path)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // A first-time key, inside a window the user opened, becomes a pairing.
    //
    // Checked *after* `admit` below would have refused it, and only for `Unknown`
    // — a revoked key is never re-admitted by pairing again, which would make
    // revocation something a peer could undo by asking twice.
    if let Some(offer) = offer.filter(|offer| now < offer.expires_at)
        && matches!(pairings.admit(&key_hex, now), Err(Refusal::Unknown))
    {
        pairings.add(super::pairing::Pairing {
            peer_key: key_hex.clone(),
            label: offer.label.clone(),
            paired_at: now,
            expires_at: offer.pairing_expires_at,
        })?;
        eprintln!(
            "[remote] paired a new peer as \"{}\" — confirm the SAS on both screens",
            offer.label
        );
        return Ok(offer.label.clone());
    }

    match pairings.admit(&key_hex, now) {
        Ok(pairing) => Ok(pairing.label.clone()),
        // The reason is logged locally and not sent: a peer that could tell
        // "revoked" from "unknown" learns whether it was ever paired.
        Err(refusal) => {
            let why = match refusal {
                Refusal::Unknown => "not paired",
                Refusal::Revoked => "revoked",
                Refusal::Expired => "pairing expired",
            };
            eprintln!("[remote] refused a peer: {why}");
            Err("peer refused".into())
        }
    }
}

/// Encrypt an inner message and put it on the wire.
async fn send_message<S>(
    sink: &mut S,
    channel: ChannelId,
    session: &mut noise::Session,
    seq: u64,
    message: &DeviceToPeer,
) -> Result<(), String>
where
    S: SinkExt<Message> + Unpin,
{
    let plaintext = rmp_serde::to_vec_named(message).map_err(|e| format!("encode: {e}"))?;
    let ciphertext = session.encrypt(&plaintext)?;
    send_frame(sink, channel, OuterKind::Data, seq, 0, ciphertext).await
}

async fn send_frame<S>(
    sink: &mut S,
    channel: ChannelId,
    kind: OuterKind,
    seq: u64,
    ack: u64,
    payload: Vec<u8>,
) -> Result<(), String>
where
    S: SinkExt<Message> + Unpin,
{
    let frame = OuterFrame {
        v: wire::PROTOCOL_VERSION,
        kind,
        channel,
        seq,
        ack,
        payload: serde_bytes::ByteBuf::from(payload),
    };
    let bytes = wire::encode(&frame).map_err(|e| e.to_string())?;
    sink.send(Message::Binary(bytes.into()))
        .await
        .map_err(|_| "relay connection lost".to_string())
}

/// The next frame's kind and payload, or `None` when the socket closes.
///
/// The kind is returned rather than discarded because the caller has to act on it:
/// a `Hello` is not data, and a device that cannot tell the difference cannot
/// recover from a peer that reconnected.
async fn next_frame<S>(
    stream: &mut S,
    channel: ChannelId,
) -> Result<Option<(OuterKind, Vec<u8>)>, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    while let Some(message) = stream.next().await {
        let message = message.map_err(|e| format!("relay read: {e}"))?;
        let Message::Binary(bytes) = message else {
            continue;
        };
        let frame = wire::decode(&bytes).map_err(|e| e.to_string())?;
        if frame.channel != channel {
            // The relay routes by channel, so this should not happen. If it
            // does, the frame is not ours and must not be fed to our session.
            eprintln!("[remote] dropped a frame for another channel");
            continue;
        }
        return Ok(Some((frame.kind, frame.payload.into_vec())));
    }
    Ok(None)
}

/// Reconnect timing: exponential with full jitter, capped.
///
/// Full jitter rather than a fixed sequence because every device that was
/// connected when a relay node restarted will otherwise come back at the same
/// instant, and the reconnect storm finishes what the restart started. Randomised
/// across the whole interval spreads them out.
#[derive(Debug, Clone)]
pub struct Backoff {
    base: Duration,
    cap: Duration,
    attempt: u32,
}

impl Default for Backoff {
    /// 1 s to 60 s, from §7.
    fn default() -> Self {
        Self::new(Duration::from_secs(1), Duration::from_secs(60))
    }
}

impl Backoff {
    pub fn new(base: Duration, cap: Duration) -> Self {
        Self {
            base,
            cap,
            attempt: 0,
        }
    }

    /// The ceiling for the next delay, before jitter. Exposed so the jitter and
    /// the growth can be tested separately — a test that only looked at the
    /// jittered value could not tell a broken curve from an unlucky draw.
    pub fn ceiling(&self) -> Duration {
        let doubled = self
            .base
            .checked_mul(2u32.saturating_pow(self.attempt))
            .unwrap_or(self.cap);
        doubled.min(self.cap)
    }

    /// How long to wait before the next attempt, and advance.
    pub fn next_delay(&mut self) -> Duration {
        let ceiling = self.ceiling();
        self.attempt = self.attempt.saturating_add(1);
        jitter(ceiling)
    }

    /// Call on a successful connection. Without this a device that reconnects
    /// once an hour would eventually always wait the full minute, because the
    /// attempt counter would never come down.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

/// Uniform in `[0, ceiling]`.
///
/// Uses the system clock rather than pulling in an rng: this picks a retry delay,
/// not a key, and adding a crypto dependency to jitter a reconnect would be the
/// wrong trade.
fn jitter(ceiling: Duration) -> Duration {
    let nanos = ceiling.as_nanos().max(1);
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u128)
        .unwrap_or(0);
    // A multiply-shift mix, so consecutive calls in the same millisecond do not
    // return the same value.
    let mixed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    Duration::from_nanos((mixed % nanos) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ceiling_doubles_from_the_base() {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));

        let ceilings: Vec<u64> = (0..6)
            .map(|_| {
                let c = backoff.ceiling().as_secs();
                backoff.next_delay();
                c
            })
            .collect();

        assert_eq!(ceilings, vec![1, 2, 4, 8, 16, 32]);
    }

    #[test]
    fn the_ceiling_stops_at_the_cap() {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));

        for _ in 0..20 {
            backoff.next_delay();
        }

        assert_eq!(backoff.ceiling(), Duration::from_secs(60));
    }

    /// A long-lived device makes a lot of attempts. The doubling must saturate
    /// rather than overflow into a tiny or absurd delay.
    #[test]
    fn a_very_large_attempt_count_does_not_overflow() {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));

        for _ in 0..200 {
            backoff.next_delay();
        }

        assert_eq!(backoff.ceiling(), Duration::from_secs(60));
        assert!(backoff.next_delay() <= Duration::from_secs(60));
    }

    #[test]
    fn every_delay_stays_within_its_ceiling() {
        let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(5));

        for _ in 0..50 {
            let ceiling = backoff.ceiling();
            let delay = backoff.next_delay();
            assert!(
                delay <= ceiling,
                "delay {delay:?} exceeded its ceiling {ceiling:?}"
            );
        }
    }

    /// The point of full jitter: a relay restart must not bring every device back
    /// at the same instant. Identical delays across calls would mean it does.
    #[test]
    fn delays_are_jittered_rather_than_fixed() {
        let mut distinct = std::collections::HashSet::new();

        for _ in 0..40 {
            let mut backoff = Backoff::new(Duration::from_secs(30), Duration::from_secs(60));
            // Advance a few steps so the ceiling is wide enough for the spread to
            // be visible.
            backoff.next_delay();
            backoff.next_delay();
            distinct.insert(backoff.next_delay().as_nanos());
        }

        assert!(
            distinct.len() > 1,
            "every device would reconnect at the same moment"
        );
    }

    /// Without a reset, a device that drops once an hour would creep up to always
    /// waiting the full cap, and a brief blip would look like an outage.
    #[test]
    fn a_successful_connection_resets_the_curve() {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
        for _ in 0..8 {
            backoff.next_delay();
        }
        assert_eq!(backoff.ceiling(), Duration::from_secs(60));

        backoff.reset();

        assert_eq!(backoff.ceiling(), Duration::from_secs(1));
    }

    #[test]
    fn a_zero_ceiling_does_not_divide_by_zero() {
        assert_eq!(jitter(Duration::ZERO), Duration::ZERO);
    }
}
