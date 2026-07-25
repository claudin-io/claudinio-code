//! The device side of the Noise session.
//!
//! `Noise_IK_25519_AESGCM_SHA256`, with the device as **responder** — the
//! browser initiates, and in `IK` the initiator already knows the responder's
//! static key because it came out of band through the pairing code. That is what
//! kills relay-substitution MITM at the root: the relay never supplies a key, so
//! it never gets to supply a wrong one, and a handshake against a substituted key
//! fails rather than succeeding quietly.
//!
//! `AESGCM` rather than the more usual ChaChaPoly so the browser side is
//! implementable on plain WebCrypto, with no crypto shipped in JavaScript. Phase
//! 0 verified that end to end, including on iOS.
//!
//! # On the replay window the plan asks for
//!
//! §9 lists a replay window here. It turned out to be unnecessary, and it is
//! worth saying why rather than quietly omitting it. Frames arrive over an
//! ordered, reliable WebSocket, and Noise transport mode advances a nonce per
//! message; a frame the relay replays therefore arrives with a nonce that has
//! already been used and fails authentication. Replay is rejected by
//! construction. There is a test for it, because "by construction" is a claim
//! that stops being true the moment someone reorders the transport.

use std::path::Path;

use claudinio_protocol::sas;
use snow::params::NoiseParams;
use snow::{Builder, TransportState};

pub const NOISE_PARAMS: &str = "Noise_IK_25519_AESGCM_SHA256";

/// Noise caps a message at 65535 bytes.
const NOISE_MAX: usize = 65535;

/// The device's long-term identity.
///
/// Stored as a file for now. `SECURITY.md` already lists an OS keychain backend
/// as a roadmap item for credentials at rest; remote access makes it a
/// prerequisite rather than a nice-to-have, which is phase 5 in the plan.
pub struct DeviceIdentity {
    private: Vec<u8>,
    public: Vec<u8>,
}

impl DeviceIdentity {
    pub fn generate() -> Result<Self, String> {
        let params: NoiseParams = NOISE_PARAMS
            .parse()
            .map_err(|e| format!("bad noise params: {e}"))?;
        let keypair = Builder::new(params)
            .generate_keypair()
            .map_err(|e| format!("generate keypair: {e}"))?;
        Ok(Self {
            private: keypair.private,
            public: keypair.public,
        })
    }

    /// Load the identity at `path`, creating it on first use.
    ///
    /// The key is written with owner-only permissions. A device key readable by
    /// another account on the machine would let it impersonate this device to
    /// every paired peer.
    pub fn load_or_create(path: &Path) -> Result<Self, String> {
        if let Ok(bytes) = std::fs::read(path) {
            if bytes.len() == 64 {
                return Ok(Self {
                    private: bytes[..32].to_vec(),
                    public: bytes[32..].to_vec(),
                });
            }
            // A truncated or foreign file is not silently replaced: overwriting
            // it would revoke every existing pairing without saying so.
            return Err(format!(
                "{} is not a device identity ({} bytes, expected 64)",
                path.display(),
                bytes.len()
            ));
        }

        let identity = Self::generate()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create key dir: {e}"))?;
        }
        let mut bytes = identity.private.clone();
        bytes.extend_from_slice(&identity.public);
        std::fs::write(path, &bytes).map_err(|e| format!("write device key: {e}"))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("restrict device key permissions: {e}"))?;
        }

        Ok(identity)
    }

    /// What goes into the pairing code, out of band.
    pub fn public_hex(&self) -> String {
        self.public.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// A completed Noise session.
pub struct Session {
    transport: TransportState,
    handshake_hash: Vec<u8>,
}

/// Complete the handshake as responder.
///
/// Returns the session and the second handshake message to send back.
pub fn accept(identity: &DeviceIdentity, msg1: &[u8]) -> Result<(Session, Vec<u8>), String> {
    let params: NoiseParams = NOISE_PARAMS
        .parse()
        .map_err(|e| format!("bad noise params: {e}"))?;
    let mut handshake = Builder::new(params)
        .local_private_key(&identity.private)
        .map_err(|e| format!("load device key: {e}"))?
        .build_responder()
        .map_err(|e| format!("build responder: {e}"))?;

    let mut scratch = vec![0u8; NOISE_MAX];
    handshake
        .read_message(msg1, &mut scratch)
        // Deliberately not echoing the underlying error to the peer: a
        // handshake failure is a handshake failure, and the detail only helps
        // someone probing for which part failed.
        .map_err(|_| "handshake rejected".to_string())?;

    let mut response = vec![0u8; NOISE_MAX];
    let written = handshake
        .write_message(&[], &mut response)
        .map_err(|e| format!("write handshake response: {e}"))?;
    response.truncate(written);

    let handshake_hash = handshake.get_handshake_hash().to_vec();
    let transport = handshake
        .into_transport_mode()
        .map_err(|e| format!("enter transport mode: {e}"))?;

    Ok((
        Session {
            transport,
            handshake_hash,
        },
        response,
    ))
}

impl Session {
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = vec![0u8; plaintext.len() + 16];
        let written = self
            .transport
            .write_message(plaintext, &mut out)
            .map_err(|e| format!("encrypt: {e}"))?;
        out.truncate(written);
        Ok(out)
    }

    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = vec![0u8; ciphertext.len()];
        let written = self
            .transport
            .read_message(ciphertext, &mut out)
            .map_err(|_| "frame rejected".to_string())?;
        out.truncate(written);
        Ok(out)
    }

    /// The words the user compares against the browser's.
    pub fn sas(&self) -> String {
        sas::format(&self.handshake_hash)
    }

    /// The peer's static key, for the revocation list: revoking a pairing has to
    /// work with the relay unreachable, which means matching on this rather than
    /// asking anyone.
    ///
    // No caller until the revocation list exists, which is phase 3. Kept because
    // the key is only available here, at handshake time.
    #[allow(dead_code)]
    pub fn peer_static(&self) -> Option<Vec<u8>> {
        self.transport.get_remote_static().map(|k| k.to_vec())
    }

    /// Fresh session keys without a new handshake. §6.1 rekeys hourly.
    pub fn rekey(&mut self) {
        self.transport.rekey_outgoing();
        self.transport.rekey_incoming();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A browser-shaped initiator. The real one is WebCrypto — phase 0 proved
    /// they interoperate — so this stands in for it to test the device's half.
    struct Initiator {
        handshake: Option<snow::HandshakeState>,
        transport: Option<TransportState>,
        keypair: snow::Keypair,
    }

    impl Initiator {
        fn new(device_public: &[u8]) -> Self {
            let params: NoiseParams = NOISE_PARAMS.parse().unwrap();
            let keypair = Builder::new(params.clone()).generate_keypair().unwrap();
            let handshake = Builder::new(params)
                .local_private_key(&keypair.private)
                .unwrap()
                .remote_public_key(device_public)
                .unwrap()
                .build_initiator()
                .unwrap();
            Self {
                handshake: Some(handshake),
                transport: None,
                keypair,
            }
        }

        fn msg1(&mut self) -> Vec<u8> {
            let mut out = vec![0u8; NOISE_MAX];
            let n = self
                .handshake
                .as_mut()
                .unwrap()
                .write_message(&[], &mut out)
                .unwrap();
            out.truncate(n);
            out
        }

        fn finish(&mut self, msg2: &[u8]) -> String {
            let mut handshake = self.handshake.take().unwrap();
            let mut scratch = vec![0u8; NOISE_MAX];
            handshake.read_message(msg2, &mut scratch).unwrap();
            let hash = handshake.get_handshake_hash().to_vec();
            self.transport = Some(handshake.into_transport_mode().unwrap());
            sas::format(&hash)
        }

        fn encrypt(&mut self, plaintext: &[u8]) -> Vec<u8> {
            let mut out = vec![0u8; plaintext.len() + 16];
            let n = self
                .transport
                .as_mut()
                .unwrap()
                .write_message(plaintext, &mut out)
                .unwrap();
            out.truncate(n);
            out
        }

        fn decrypt(&mut self, ciphertext: &[u8]) -> Vec<u8> {
            let mut out = vec![0u8; ciphertext.len()];
            let n = self
                .transport
                .as_mut()
                .unwrap()
                .read_message(ciphertext, &mut out)
                .unwrap();
            out.truncate(n);
            out
        }
    }

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap())
            .collect()
    }

    fn paired() -> (Session, Initiator) {
        let identity = DeviceIdentity::generate().unwrap();
        let mut initiator = Initiator::new(&hex_to_bytes(&identity.public_hex()));
        let (session, msg2) = accept(&identity, &initiator.msg1()).unwrap();
        initiator.finish(&msg2);
        (session, initiator)
    }

    #[test]
    fn a_handshake_completes_and_both_sides_agree_on_the_sas() {
        let identity = DeviceIdentity::generate().unwrap();
        let mut initiator = Initiator::new(&hex_to_bytes(&identity.public_hex()));

        let (session, msg2) = accept(&identity, &initiator.msg1()).unwrap();
        let peer_sas = initiator.finish(&msg2);

        assert_eq!(
            session.sas(),
            peer_sas,
            "the two screens would show different words"
        );
        assert_eq!(session.sas().split(" · ").count(), 3);
    }

    #[test]
    fn traffic_flows_both_ways() {
        let (mut session, mut initiator) = paired();

        let to_device = initiator.encrypt(b"approve tool-1");
        assert_eq!(session.decrypt(&to_device).unwrap(), b"approve tool-1");

        let to_peer = session.encrypt(b"tool result").unwrap();
        assert_eq!(initiator.decrypt(&to_peer), b"tool result");
    }

    /// The whole point of IK: the initiator must already know the device's key.
    /// A relay that substitutes one cannot complete a handshake, so substitution
    /// fails closed instead of succeeding invisibly.
    #[test]
    fn a_handshake_against_a_substituted_key_fails() {
        let real = DeviceIdentity::generate().unwrap();
        let impostor = DeviceIdentity::generate().unwrap();

        // The browser was given the impostor's key, as a hostile relay would.
        let mut initiator = Initiator::new(&hex_to_bytes(&impostor.public_hex()));

        assert!(
            accept(&real, &initiator.msg1()).is_err(),
            "the device accepted a handshake meant for another key"
        );
    }

    #[test]
    fn a_tampered_frame_is_rejected() {
        let (mut session, mut initiator) = paired();
        let mut frame = initiator.encrypt(b"approve tool-1");

        let middle = frame.len() / 2;
        frame[middle] ^= 0x01;

        assert!(session.decrypt(&frame).is_err());
    }

    /// §9 asks for a replay window. It is unnecessary, and this is why: transport
    /// mode advances a nonce per message, so a frame the relay sends twice
    /// arrives with a nonce already spent and fails authentication.
    #[test]
    fn a_replayed_frame_is_rejected_without_a_replay_window() {
        let (mut session, mut initiator) = paired();
        let frame = initiator.encrypt(b"approve tool-1");

        assert_eq!(session.decrypt(&frame).unwrap(), b"approve tool-1");
        assert!(
            session.decrypt(&frame).is_err(),
            "the same frame decrypted twice — a replayed approval would execute twice"
        );
    }

    #[test]
    fn the_device_learns_the_peers_static_key_for_the_revocation_list() {
        let (session, initiator) = paired();

        assert_eq!(
            session.peer_static().as_deref(),
            Some(initiator.keypair.public.as_slice())
        );
    }

    #[test]
    fn an_identity_persists_and_reloads_as_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.key");

        let first = DeviceIdentity::load_or_create(&path).unwrap();
        let second = DeviceIdentity::load_or_create(&path).unwrap();

        assert_eq!(first.public_hex(), second.public_hex());
        assert_eq!(first.public_hex().len(), 64);
    }

    /// Overwriting a damaged key file would revoke every existing pairing
    /// without telling anyone. Failing loudly is the lesser harm.
    #[test]
    fn a_damaged_identity_file_is_an_error_rather_than_a_silent_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.key");
        std::fs::write(&path, b"not a key").unwrap();

        assert!(DeviceIdentity::load_or_create(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_new_identity_is_not_readable_by_other_accounts() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.key");
        DeviceIdentity::load_or_create(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "device key is readable beyond its owner");
    }

    #[test]
    fn a_rekey_keeps_the_session_usable() {
        let (mut session, mut initiator) = paired();

        let before = initiator.encrypt(b"before");
        assert_eq!(session.decrypt(&before).unwrap(), b"before");

        session.rekey();
        initiator.transport.as_mut().unwrap().rekey_outgoing();
        initiator.transport.as_mut().unwrap().rekey_incoming();

        let after = initiator.encrypt(b"after");
        assert_eq!(session.decrypt(&after).unwrap(), b"after");
    }
}

#[cfg(test)]
mod golden {
    use super::*;
    use snow::Builder;

    /// A recorded IK handshake and exchange, printed as hex.
    ///
    /// Both ephemerals are fixed and both static keypairs are printed, so the whole
    /// transcript is reproducible from the values below — which is what makes it a
    /// golden vector for the browser in `apps/web/src/golden.ts`.
    ///
    /// This is the only thing that can show the two implementations agree. A browser
    /// initiator tested against a browser responder proves they agree with each other
    /// and nothing about whether either agrees with the device. The first time this
    /// existed it caught a real one within the hour: the device attached to the relay
    /// with no channel token, so nothing could ever have connected.
    ///
    /// `cargo test --features remote golden -- --ignored --nocapture` to regenerate.
    /// The static keys change on every run, so the TypeScript constants are replaced
    /// as a set.
    ///
    /// # Two handshakes, on purpose
    ///
    /// A Noise cipher advances its nonce per message, so a vector's position in the
    /// stream is part of it. The bare-ciphertext vectors and the whole-frame vectors
    /// are therefore taken from *separate* handshakes, both starting at counter zero,
    /// so a test can use either without having to consume the other first. With the
    /// ephemerals fixed the two handshakes are identical, which is what makes that
    /// sound rather than a fudge.
    #[test]
    #[ignore = "prints a golden Noise transcript for apps/web/src/golden.ts"]
    fn print_golden_handshake() {
        let init_ephemeral = [0x02u8; 32];
        let resp_ephemeral = [0x04u8; 32];
        let params: snow::params::NoiseParams = NOISE_PARAMS.parse().unwrap();
        let init_keys = Builder::new(params.clone()).generate_keypair().unwrap();
        let resp_keys = Builder::new(params.clone()).generate_keypair().unwrap();
        let hex = |b: &[u8]| -> String { b.iter().map(|x| format!("{x:02x}")).collect() };

        // One complete handshake, returning both transport states and what it took.
        let run = || {
            let params: snow::params::NoiseParams = NOISE_PARAMS.parse().unwrap();
            let mut initiator = Builder::new(params.clone())
                .local_private_key(&init_keys.private)
                .unwrap()
                .remote_public_key(&resp_keys.public)
                .unwrap()
                .fixed_ephemeral_key_for_testing_only(&init_ephemeral)
                .build_initiator()
                .unwrap();
            let mut responder = Builder::new(params)
                .local_private_key(&resp_keys.private)
                .unwrap()
                .fixed_ephemeral_key_for_testing_only(&resp_ephemeral)
                .build_responder()
                .unwrap();

            let mut msg1 = [0u8; 1024];
            let n1 = initiator.write_message(&[], &mut msg1).unwrap();
            let mut scratch = [0u8; 1024];
            responder.read_message(&msg1[..n1], &mut scratch).unwrap();
            let mut msg2 = [0u8; 1024];
            let n2 = responder.write_message(&[], &mut msg2).unwrap();
            initiator.read_message(&msg2[..n2], &mut scratch).unwrap();

            let hash = initiator.get_handshake_hash().to_vec();
            (
                msg1[..n1].to_vec(),
                msg2[..n2].to_vec(),
                hash,
                initiator.into_transport_mode().unwrap(),
                responder.into_transport_mode().unwrap(),
            )
        };

        let (msg1, msg2, hash, mut peer, mut device) = run();

        println!("GOLDEN_INIT_STATIC_PRIVATE={}", hex(&init_keys.private));
        println!("GOLDEN_INIT_STATIC_PUBLIC={}", hex(&init_keys.public));
        println!("GOLDEN_INIT_EPHEMERAL_PRIVATE={}", hex(&init_ephemeral));
        println!("GOLDEN_RESP_STATIC_PUBLIC={}", hex(&resp_keys.public));
        println!("GOLDEN_MSG1={}", hex(&msg1));
        println!("GOLDEN_MSG2={}", hex(&msg2));
        println!("GOLDEN_HANDSHAKE_HASH={}", hex(&hash));
        println!("GOLDEN_SAS={}", claudinio_protocol::sas::format(&hash));

        // First handshake: bare ciphertext, one message each way, both at counter 0.
        let mut buf = [0u8; 1024];
        let n = device
            .write_message(b"hello from the device", &mut buf)
            .unwrap();
        println!("GOLDEN_DEVICE_CIPHERTEXT={}", hex(&buf[..n]));
        let n = peer
            .write_message(b"hello from the browser", &mut buf)
            .unwrap();
        println!("GOLDEN_PEER_CIPHERTEXT={}", hex(&buf[..n]));

        // Second handshake: whole frames, as they go on the wire. Starting fresh so
        // the browser's session layer can consume these without having consumed the
        // bare vectors above.
        let (_, _, _, mut peer, mut device) = run();
        let channel = claudinio_protocol::wire::ChannelId::from_bytes([0xab; 16]);
        let frame = |transport: &mut snow::TransportState, seq: u64, plaintext: &[u8]| -> String {
            let mut buf = vec![0u8; plaintext.len() + 1024];
            let n = transport.write_message(plaintext, &mut buf).unwrap();
            let encoded =
                claudinio_protocol::wire::OuterFrame::data(channel, seq, 0, buf[..n].to_vec());
            hex(&claudinio_protocol::wire::encode(&encoded).unwrap())
        };

        let snapshot = claudinio_protocol::inner::DeviceToPeer::Snapshot {
            session_id: "golden".into(),
            records: vec![
                serde_json::json!({ "kind": "meta", "sessionId": "golden" }),
                serde_json::json!({ "kind": "user", "content": "hello" }),
            ],
            seq: 2,
        };
        println!(
            "GOLDEN_SNAPSHOT_FRAME={}",
            frame(&mut device, 2, &rmp_serde::to_vec_named(&snapshot).unwrap())
        );

        let closed = claudinio_protocol::inner::DeviceToPeer::Closed {
            reason: claudinio_protocol::inner::CloseReason::TurnedOffLocally,
        };
        println!(
            "GOLDEN_CLOSED_FRAME={}",
            frame(&mut device, 3, &rmp_serde::to_vec_named(&closed).unwrap())
        );

        let subscribe = claudinio_protocol::inner::PeerToDevice::Subscribe {
            session_id: "golden".into(),
            from_seq: 0,
        };
        println!(
            "GOLDEN_SUBSCRIBE_FRAME={}",
            frame(&mut peer, 1, &rmp_serde::to_vec_named(&subscribe).unwrap())
        );
    }
}
