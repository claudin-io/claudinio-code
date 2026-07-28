/// The outer frame, as the browser speaks it.
///
/// Mirrors `src-tauri/crates/claudinio-protocol/src/wire.rs`. That crate is the
/// definition; this is the second implementation, and the two are held together by
/// golden vectors in `wire.test.ts` produced by the Rust encoder itself. Two codecs
/// that only round-trip against themselves can agree with themselves and disagree
/// with each other while every test passes.
///
/// # Why a MessagePack dependency and not a hand-rolled one
///
/// The outer frame is a fixed six-key map and would be easy enough by hand. The
/// *inner* messages are not: they carry `serde_json::Value` transcript records, so
/// decoding means arbitrary maps, arrays, strings, integers of every width, floats,
/// nulls. A subtle bug there does not crash — it silently renders a transcript
/// wrong, which is the failure mode this whole feature is least able to afford.

import { encode as msgpackEncode, decode as msgpackDecode } from "@msgpack/msgpack";

/// A `Uint8Array` that is definitely backed by an `ArrayBuffer`, never a
/// `SharedArrayBuffer`.
///
/// WebCrypto's `BufferSource` excludes the shared case — another thread can mutate a
/// `SharedArrayBuffer` while the operation reads it — so a bare `Uint8Array`, whose
/// buffer is `ArrayBufferLike`, is not assignable to it. Every array on this path
/// comes from `new Uint8Array(...)`, `TextEncoder.encode` or a `slice`, all unshared,
/// so naming that once here keeps the crypto calls in `noise.ts` typed without a cast
/// at every call site.
export type Bytes = Uint8Array<ArrayBuffer>;

/// Matches `PROTOCOL_VERSION`.
export const PROTOCOL_VERSION = 1;

/// Matches `MAX_FRAME`. Enforced on both sides: the relay refuses larger frames, so
/// sending one is a silent black hole rather than an error.
export const MAX_FRAME = 256 * 1024;

export enum OuterKind {
  Hello = 0,
  HelloAck = 1,
  Open = 2,
  Data = 3,
  Close = 4,
  Ping = 5,
  Pong = 6,
  Error = 7,
}

export interface OuterFrame {
  v: number;
  /// A number rather than the enum, because an unknown kind must survive decoding.
  /// The relay routes frames it does not recognise on purpose — the component in
  /// the middle is the one that must never need deploying in lockstep — and a peer
  /// that threw on an unfamiliar kind would be stricter than the relay for no gain.
  kind: number;
  /// 16 bytes.
  channel: Bytes;
  seq: number;
  ack: number;
  payload: Bytes;
}

export class WireError extends Error {}

export function encodeFrame(frame: OuterFrame): Bytes {
  if (frame.channel.length !== 16) {
    throw new WireError(`channel id must be 16 bytes, got ${frame.channel.length}`);
  }
  // Key order matches the Rust struct. MessagePack maps are unordered by
  // specification and rmp_serde does not care, but matching it keeps a hex dump of
  // one side comparable to the other by eye — which is how the golden vectors are
  // checked when they ever disagree.
  const bytes = msgpackEncode({
    v: frame.v,
    kind: frame.kind,
    channel: frame.channel,
    seq: frame.seq,
    ack: frame.ack,
    payload: frame.payload,
  });
  if (bytes.length > MAX_FRAME) {
    throw new WireError(`frame is ${bytes.length} bytes, over the ${MAX_FRAME} byte limit`);
  }
  return bytes;
}

export function decodeFrame(bytes: Bytes): OuterFrame {
  // Checked before decoding, not after. A frame over the limit is refused without
  // allocating whatever it claims to contain.
  if (bytes.length > MAX_FRAME) {
    throw new WireError(`frame is ${bytes.length} bytes, over the ${MAX_FRAME} byte limit`);
  }

  let value: unknown;
  try {
    // `useBigInt64` so a 64-bit integer past 2^53 arrives as a bigint and is
    // refused below. The default is to return a lossy number, and a sequence
    // number that quietly lost precision would show up later as a gap nobody could
    // explain — the worst shape of bug this protocol can have, because gaps are
    // supposed to mean loss.
    value = msgpackDecode(bytes, { useBigInt64: true });
  } catch (e) {
    throw new WireError(`malformed frame: ${e instanceof Error ? e.message : String(e)}`);
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new WireError("malformed frame: not a map");
  }

  const map = value as Record<string, unknown>;
  const frame: OuterFrame = {
    v: requireNumber(map.v, "v"),
    kind: requireNumber(map.kind, "kind"),
    channel: requireBytes(map.channel, "channel", 16),
    seq: requireNumber(map.seq, "seq"),
    ack: requireNumber(map.ack, "ack"),
    payload: requireBytes(map.payload, "payload"),
  };

  // The version is checked here rather than by the caller so no path can forget.
  // A future version is refused rather than guessed at: the frame layout is what
  // the version describes, so a v2 frame read as v1 is read wrongly.
  if (frame.v !== PROTOCOL_VERSION) {
    throw new WireError(`frame protocol version ${frame.v}, expected ${PROTOCOL_VERSION}`);
  }
  return frame;
}

/// A data frame at the current version.
export function dataFrame(
  channel: Bytes,
  seq: number,
  ack: number,
  payload: Bytes,
): OuterFrame {
  return { v: PROTOCOL_VERSION, kind: OuterKind.Data, channel, seq, ack, payload };
}

export function helloFrame(channel: Bytes, payload: Bytes): OuterFrame {
  return { v: PROTOCOL_VERSION, kind: OuterKind.Hello, channel, seq: 0, ack: 0, payload };
}

export function channelFromHex(hex: string): Bytes {
  if (hex.length !== 32) {
    throw new WireError(`channel id must be 32 hex characters, got ${hex.length}`);
  }
  return bytesFromHex(hex, "channel id");
}

export function bytesFromHex(hex: string, what = "value"): Bytes {
  if (hex.length % 2 !== 0) throw new WireError(`${what} has an odd number of hex digits`);
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    const byte = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
    if (Number.isNaN(byte)) throw new WireError(`${what} is not hex`);
    out[i] = byte;
  }
  return out;
}

export function hexFromBytes(bytes: Bytes): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function requireNumber(value: unknown, field: string): number {
  // Decoded with `useBigInt64`, so a 64-bit integer arrives as a bigint. Small ones
  // are safe to narrow — rmp_serde emits the smallest format, so a bigint here means
  // the value genuinely needed 64 bits. Past 2^53 it is refused rather than coerced:
  // `seq` would have to run longer than the universe to get there honestly, so this
  // is a crafted frame, and a silently truncated sequence number would surface later
  // as a gap nobody could explain.
  if (typeof value === "bigint") {
    if (value < 0n || value > BigInt(Number.MAX_SAFE_INTEGER)) {
      throw new WireError(`malformed frame: ${field} is out of range`);
    }
    return Number(value);
  }
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
    throw new WireError(`malformed frame: ${field} is not a non-negative integer`);
  }
  return value;
}

function requireBytes(value: unknown, field: string, exactly?: number): Bytes {
  if (!(value instanceof Uint8Array)) {
    throw new WireError(`malformed frame: ${field} is not binary`);
  }
  if (exactly !== undefined && value.length !== exactly) {
    throw new WireError(`malformed frame: ${field} must be ${exactly} bytes, got ${value.length}`);
  }
  // `instanceof` only narrows as far as `Uint8Array<ArrayBufferLike>`; the decoder
  // allocates its own buffers and has no way to hand back a shared one, so this is
  // the one place the unshared claim is asserted rather than tracked.
  return value as Bytes;
}
