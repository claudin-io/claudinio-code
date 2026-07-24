//! The outer frame: everything the relay is allowed to understand.
//!
//! Six fields, MessagePack-encoded. The relay authenticates the connection,
//! looks up which peer owns `channel`, forwards the frame, and enforces quotas.
//! It never allocates more than `MAX_FRAME` and never writes `payload` anywhere.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Bumped only for a breaking change to *this* frame. Inner messages version
/// themselves; the relay neither sees nor cares about that.
pub const PROTOCOL_VERSION: u8 = 1;

/// Hard ceiling on one encoded frame, enforced on both encode and decode.
///
/// The decode-side check is the load-bearing one: it is what stops a hostile
/// peer from making the relay allocate on demand.
pub const MAX_FRAME: usize = 256 * 1024;

/// Opaque to the relay by construction — it is a routing key and nothing else.
/// 16 bytes so it can be a random v4 UUID without collision worry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelId([u8; 16]);

impl ChannelId {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(hex: &str) -> Result<Self, WireError> {
        if hex.len() != 32 {
            return Err(WireError::Malformed(format!(
                "channel id must be 32 hex characters, got {}",
                hex.len()
            )));
        }
        let mut out = [0u8; 16];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
                .map_err(|e| WireError::Malformed(format!("channel id is not hex: {e}")))?;
        }
        Ok(Self(out))
    }
}

impl std::fmt::Display for ChannelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

// Encoded as a MessagePack `bin`, not as a 16-element array. The array form
// would trade 16 bytes for 17-plus and read as a list of integers in every log
// and packet capture.
impl Serialize for ChannelId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for ChannelId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = serde_bytes::ByteBuf::deserialize(deserializer)?;
        let slice = bytes.as_ref();
        if slice.len() != 16 {
            return Err(serde::de::Error::custom(format!(
                "channel id must be 16 bytes, got {}",
                slice.len()
            )));
        }
        let mut out = [0u8; 16];
        out.copy_from_slice(slice);
        Ok(Self(out))
    }
}

/// What a frame is for.
///
/// `Other` exists so a frame kind this build has never heard of still decodes.
/// The relay's job is to route, and a relay that dropped frames it did not
/// recognise would break every peer newer than itself — the one component that
/// must never need to be deployed in lockstep is the one in the middle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OuterKind {
    Hello,
    HelloAck,
    Open,
    Data,
    Close,
    Ping,
    Pong,
    Error,
    Other(u8),
}

impl OuterKind {
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Hello => 0,
            Self::HelloAck => 1,
            Self::Open => 2,
            Self::Data => 3,
            Self::Close => 4,
            Self::Ping => 5,
            Self::Pong => 6,
            Self::Error => 7,
            Self::Other(n) => n,
        }
    }

    pub const fn from_u8(n: u8) -> Self {
        match n {
            0 => Self::Hello,
            1 => Self::HelloAck,
            2 => Self::Open,
            3 => Self::Data,
            4 => Self::Close,
            5 => Self::Ping,
            6 => Self::Pong,
            7 => Self::Error,
            other => Self::Other(other),
        }
    }
}

impl Serialize for OuterKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.as_u8())
    }
}

impl<'de> Deserialize<'de> for OuterKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_u8(u8::deserialize(deserializer)?))
    }
}

/// The frame. `payload` is Noise ciphertext to the relay and nothing more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OuterFrame {
    pub v: u8,
    pub kind: OuterKind,
    pub channel: ChannelId,
    /// Per-channel, per-direction, monotonic. Gaps mean loss, and loss being
    /// detectable rather than silent is the whole reason this is on the wire.
    pub seq: u64,
    /// Highest contiguous `seq` the sender has received.
    pub ack: u64,
    pub payload: serde_bytes::ByteBuf,
}

impl OuterFrame {
    /// A data frame at the current protocol version.
    pub fn data(channel: ChannelId, seq: u64, ack: u64, payload: Vec<u8>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            kind: OuterKind::Data,
            channel,
            seq,
            ack,
            payload: serde_bytes::ByteBuf::from(payload),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// Refused before allocating. Carries both numbers so a log says why.
    TooLarge {
        size: usize,
        max: usize,
    },
    Malformed(String),
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { size, max } => {
                write!(f, "frame is {size} bytes, over the {max} byte limit")
            }
            Self::Malformed(why) => write!(f, "malformed frame: {why}"),
        }
    }
}

impl std::error::Error for WireError {}

pub fn encode(frame: &OuterFrame) -> Result<Vec<u8>, WireError> {
    let bytes = rmp_serde::to_vec_named(frame)
        .map_err(|e| WireError::Malformed(format!("encode failed: {e}")))?;
    if bytes.len() > MAX_FRAME {
        return Err(WireError::TooLarge {
            size: bytes.len(),
            max: MAX_FRAME,
        });
    }
    Ok(bytes)
}

pub fn decode(bytes: &[u8]) -> Result<OuterFrame, WireError> {
    // Length first: refusing before parsing is what keeps a hostile peer from
    // choosing how much the relay allocates.
    if bytes.len() > MAX_FRAME {
        return Err(WireError::TooLarge {
            size: bytes.len(),
            max: MAX_FRAME,
        });
    }
    rmp_serde::from_slice(bytes).map_err(|e| WireError::Malformed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> ChannelId {
        ChannelId::from_bytes([
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54,
            0x32, 0x10,
        ])
    }

    #[test]
    fn a_frame_survives_a_round_trip() {
        let frame = OuterFrame::data(channel(), 42, 41, b"ciphertext".to_vec());

        let encoded = encode(&frame).unwrap();

        assert_eq!(decode(&encoded).unwrap(), frame);
    }

    /// The payload must go on the wire as a MessagePack `bin`. As an array of
    /// integers it would cost several bytes per byte, which on a token stream is
    /// the difference between a rounding error and a bandwidth bill.
    #[test]
    fn the_payload_is_encoded_as_binary_not_as_a_list_of_numbers() {
        let payload = vec![0xAAu8; 1000];
        let frame = OuterFrame::data(channel(), 1, 0, payload);

        let encoded = encode(&frame).unwrap();

        // 1000 bytes of payload plus a small header. An array encoding would be
        // at least 2000.
        assert!(
            encoded.len() < 1100,
            "expected a compact binary payload, got {} bytes",
            encoded.len()
        );
    }

    #[test]
    fn a_frame_over_the_limit_is_refused_on_encode() {
        let frame = OuterFrame::data(channel(), 1, 0, vec![0u8; MAX_FRAME + 1]);

        match encode(&frame) {
            Err(WireError::TooLarge { max, .. }) => assert_eq!(max, MAX_FRAME),
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    /// The decode-side check is the one that matters: it is what stops a peer
    /// from deciding how much the relay allocates.
    #[test]
    fn oversized_input_is_refused_before_it_is_parsed() {
        let hostile = vec![0u8; MAX_FRAME + 1];

        match decode(&hostile) {
            Err(WireError::TooLarge { size, max }) => {
                assert_eq!(size, MAX_FRAME + 1);
                assert_eq!(max, MAX_FRAME);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    /// Forward compatibility, and it is the relay's problem specifically: the
    /// component in the middle must never need to be deployed in lockstep with
    /// the peers, so a kind it has never seen still routes.
    #[test]
    fn a_frame_kind_from_a_newer_peer_still_decodes() {
        let mut frame = OuterFrame::data(channel(), 7, 6, b"x".to_vec());
        frame.kind = OuterKind::Other(200);

        let decoded = decode(&encode(&frame).unwrap()).unwrap();

        assert_eq!(decoded.kind, OuterKind::Other(200));
        assert_eq!(decoded.channel, channel(), "it is still routable");
        assert_eq!(decoded.seq, 7);
    }

    #[test]
    fn known_kinds_round_trip_through_their_wire_numbers() {
        for kind in [
            OuterKind::Hello,
            OuterKind::HelloAck,
            OuterKind::Open,
            OuterKind::Data,
            OuterKind::Close,
            OuterKind::Ping,
            OuterKind::Pong,
            OuterKind::Error,
        ] {
            assert_eq!(OuterKind::from_u8(kind.as_u8()), kind);
        }
    }

    /// A version mismatch must be answerable, which means the frame has to parse
    /// far enough to reply. Dropping it would leave the other end waiting.
    #[test]
    fn a_frame_from_another_protocol_version_still_parses() {
        let mut frame = OuterFrame::data(channel(), 1, 0, b"x".to_vec());
        frame.v = PROTOCOL_VERSION + 9;

        let decoded = decode(&encode(&frame).unwrap()).unwrap();

        assert_eq!(decoded.v, PROTOCOL_VERSION + 9);
    }

    #[test]
    fn garbage_is_an_error_and_not_a_panic() {
        assert!(matches!(
            decode(&[0xff, 0x00, 0x13, 0x37]),
            Err(WireError::Malformed(_))
        ));
        assert!(matches!(decode(&[]), Err(WireError::Malformed(_))));
    }

    #[test]
    fn a_channel_id_of_the_wrong_length_is_rejected() {
        // 15 bytes where 16 are required, hand-built so the check is exercised
        // rather than assumed.
        #[derive(Serialize)]
        struct Bad {
            v: u8,
            kind: u8,
            channel: serde_bytes::ByteBuf,
            seq: u64,
            ack: u64,
            payload: serde_bytes::ByteBuf,
        }
        let bad = Bad {
            v: 1,
            kind: 3,
            channel: serde_bytes::ByteBuf::from(vec![0u8; 15]),
            seq: 0,
            ack: 0,
            payload: serde_bytes::ByteBuf::new(),
        };

        let bytes = rmp_serde::to_vec_named(&bad).unwrap();

        assert!(matches!(decode(&bytes), Err(WireError::Malformed(_))));
    }

    #[test]
    fn channel_ids_render_and_parse_as_hex() {
        let hex = channel().to_hex();

        assert_eq!(hex, "0123456789abcdeffedcba9876543210");
        assert_eq!(ChannelId::from_hex(&hex).unwrap(), channel());
        assert_eq!(channel().to_string(), hex, "Display matches to_hex");
    }

    #[test]
    fn a_bad_hex_channel_id_is_an_error() {
        assert!(ChannelId::from_hex("tooshort").is_err());
        assert!(ChannelId::from_hex(&"z".repeat(32)).is_err());
    }
}
