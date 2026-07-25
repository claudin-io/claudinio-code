import { describe, it, expect } from "vitest";
import {
  MAX_FRAME,
  OuterKind,
  PROTOCOL_VERSION,
  WireError,
  bytesFromHex,
  channelFromHex,
  dataFrame,
  decodeFrame,
  encodeFrame,
  helloFrame,
  hexFromBytes,
} from "./wire";

/// Produced by the Rust encoder, not by this one.
///
/// `cargo test -p claudinio-protocol golden -- --ignored --nocapture` in
/// `src-tauri/` regenerates them. This is the only test here that can show the two
/// implementations agree: two codecs that round-trip against themselves can agree
/// with themselves and disagree with each other while every other test passes.
const GOLDEN_DATA_FRAME =
  "86a17601a46b696e6403a76368616e6e656cc410abcdef0123456789abcdef0123456789a3736571cd012ca361636b07a77061796c6f6164c404deadbeef";
const GOLDEN_HELLO_FRAME =
  "86a17601a46b696e6400a76368616e6e656cc41011111111111111111111111111111111a373657100a361636b00a77061796c6f6164c430" +
  "00".repeat(48);

const CHANNEL = channelFromHex("abcdef0123456789abcdef0123456789");

describe("golden vectors from the Rust encoder", () => {
  it("decodes a data frame Rust produced", () => {
    const frame = decodeFrame(bytesFromHex(GOLDEN_DATA_FRAME));

    expect(frame.v).toBe(PROTOCOL_VERSION);
    expect(frame.kind).toBe(OuterKind.Data);
    expect(hexFromBytes(frame.channel)).toBe("abcdef0123456789abcdef0123456789");
    expect(frame.seq).toBe(300);
    expect(frame.ack).toBe(7);
    expect(hexFromBytes(frame.payload)).toBe("deadbeef");
  });

  /// The direction that matters more: a frame this codec produces has to be a frame
  /// the device will accept.
  it("produces the same bytes Rust does", () => {
    const encoded = encodeFrame({
      v: PROTOCOL_VERSION,
      kind: OuterKind.Data,
      channel: CHANNEL,
      seq: 300,
      ack: 7,
      payload: bytesFromHex("deadbeef"),
    });

    expect(hexFromBytes(encoded)).toBe(GOLDEN_DATA_FRAME);
  });

  /// A handshake message is 48 bytes and crosses the one-byte-length boundary in
  /// MessagePack's bin format, which is exactly where a hand-rolled encoder would
  /// have differed.
  it("agrees on a handshake frame", () => {
    const encoded = encodeFrame(helloFrame(bytesFromHex("11".repeat(16)), new Uint8Array(48)));
    expect(hexFromBytes(encoded)).toBe(GOLDEN_HELLO_FRAME);

    const frame = decodeFrame(bytesFromHex(GOLDEN_HELLO_FRAME));
    expect(frame.kind).toBe(OuterKind.Hello);
    expect(frame.payload.length).toBe(48);
  });
});

describe("encodeFrame", () => {
  it("round-trips a frame", () => {
    const frame = dataFrame(CHANNEL, 42, 41, bytesFromHex("cafe"));
    expect(decodeFrame(encodeFrame(frame))).toEqual(frame);
  });

  it("refuses a channel that is not 16 bytes", () => {
    const frame = dataFrame(new Uint8Array(8), 1, 0, new Uint8Array(0));
    expect(() => encodeFrame(frame)).toThrow(WireError);
  });

  /// The relay refuses an oversized frame, so sending one is a silent black hole
  /// rather than an error. Better to fail here, where there is a stack.
  it("refuses a frame over the size limit", () => {
    const frame = dataFrame(CHANNEL, 1, 0, new Uint8Array(MAX_FRAME));
    expect(() => encodeFrame(frame)).toThrow(/over the/);
  });

  it("carries an empty payload", () => {
    const frame = dataFrame(CHANNEL, 0, 0, new Uint8Array(0));
    expect(decodeFrame(encodeFrame(frame)).payload.length).toBe(0);
  });

  it("carries a payload right up to the limit", () => {
    // Leaves room for the map, keys and bin header.
    const frame = dataFrame(CHANNEL, 1, 0, new Uint8Array(MAX_FRAME - 200));
    expect(decodeFrame(encodeFrame(frame)).payload.length).toBe(MAX_FRAME - 200);
  });

  it("carries a large seq", () => {
    const frame = dataFrame(CHANNEL, 4_000_000_000, 3_999_999_999, new Uint8Array(1));
    const back = decodeFrame(encodeFrame(frame));
    expect(back.seq).toBe(4_000_000_000);
    expect(back.ack).toBe(3_999_999_999);
  });
});

describe("decodeFrame", () => {
  /// Refused before decoding, so a frame claiming to contain more than the limit is
  /// never allocated.
  it("refuses an oversized frame without decoding it", () => {
    expect(() => decodeFrame(new Uint8Array(MAX_FRAME + 1))).toThrow(/over the/);
  });

  it("refuses bytes that are not MessagePack", () => {
    expect(() => decodeFrame(new Uint8Array([0xc1, 0xc1, 0xc1]))).toThrow(WireError);
  });

  it("refuses something that is not a map", () => {
    // A MessagePack array of three small ints.
    expect(() => decodeFrame(new Uint8Array([0x93, 0x01, 0x02, 0x03]))).toThrow(/not a map/);
  });

  it("refuses a map missing a field", () => {
    const partial = new Uint8Array([0x81, 0xa1, 0x76, 0x01]); // { v: 1 }
    expect(() => decodeFrame(partial)).toThrow(WireError);
  });

  /// A v2 frame read as v1 is read wrongly, because the version is what describes
  /// the layout. Refused rather than guessed at.
  it("refuses a protocol version it does not know", () => {
    const bytes = encodeFrame({ ...dataFrame(CHANNEL, 1, 0, new Uint8Array(1)), v: 2 });
    expect(() => decodeFrame(bytes)).toThrow(/protocol version 2/);
  });

  /// The relay routes kinds it has never heard of on purpose. A peer stricter than
  /// the relay would break against a newer device for no gain.
  it("decodes a frame kind it has never seen", () => {
    const bytes = encodeFrame({ ...dataFrame(CHANNEL, 1, 0, new Uint8Array(1)), kind: 99 });
    expect(decodeFrame(bytes).kind).toBe(99);
  });

  it("refuses a channel of the wrong length", () => {
    const bytes = encodeFrame({
      ...dataFrame(CHANNEL, 1, 0, new Uint8Array(1)),
      channel: CHANNEL,
    });
    // Re-encode by hand with an 8-byte channel, which encodeFrame would refuse.
    const tampered = decodeFrame(bytes);
    expect(() => encodeFrame({ ...tampered, channel: new Uint8Array(8) })).toThrow(WireError);
  });

  it("refuses a negative or fractional seq", () => {
    for (const seq of [-1, 1.5]) {
      const bytes = encodeFrame({ ...dataFrame(CHANNEL, 0, 0, new Uint8Array(1)), seq });
      expect(() => decodeFrame(bytes), String(seq)).toThrow(/not a non-negative integer/);
    }
  });

  /// A crafted frame can claim a seq that needs a full 64 bits. Decoded lossily it
  /// becomes a number that compares wrongly against every other seq, and the symptom
  /// is a gap nobody can explain — so it is refused.
  it("refuses a seq past what a number can hold", () => {
    // `seq` as a MessagePack uint64 of 0xffff_ffff_ffff_ffff, which is what a hostile
    // encoder would send and what rmp_serde never produces for a real sequence.
    const bytes = withUint64Seq(0xffffffffffffffffn);
    expect(() => decodeFrame(bytes)).toThrow(/out of range/);
  });

  /// But a 64-bit format carrying a small value is fine, because narrowing it is
  /// lossless. Refusing on the format rather than the value would break against an
  /// encoder that is merely less frugal than rmp_serde.
  it("accepts a 64-bit seq that fits in a number", () => {
    expect(decodeFrame(withUint64Seq(1234n)).seq).toBe(1234);
  });

  it("refuses a payload that is not binary", () => {
    // { v, kind, channel, seq, ack, payload: "text" } — payload as a string.
    const bytes = encodeFrame(dataFrame(CHANNEL, 1, 0, new Uint8Array(1)));
    const decoded = decodeFrame(bytes);
    // Round-trip through a hand-built map with a string payload.
    const bad = new Uint8Array([
      0x86,
      0xa1, 0x76, 0x01,
      0xa4, 0x6b, 0x69, 0x6e, 0x64, 0x03,
      0xa7, 0x63, 0x68, 0x61, 0x6e, 0x6e, 0x65, 0x6c, 0xc4, 0x10, ...decoded.channel,
      0xa3, 0x73, 0x65, 0x71, 0x01,
      0xa3, 0x61, 0x63, 0x6b, 0x00,
      0xa7, 0x70, 0x61, 0x79, 0x6c, 0x6f, 0x61, 0x64, 0xa1, 0x78,
    ]);
    expect(() => decodeFrame(bad)).toThrow(/payload is not binary/);
  });
});

describe("hex helpers", () => {
  it("round-trips", () => {
    const bytes = new Uint8Array([0x00, 0x0f, 0xff, 0x7f]);
    expect(bytesFromHex(hexFromBytes(bytes))).toEqual(bytes);
  });

  it("pads single digits", () => {
    expect(hexFromBytes(new Uint8Array([1, 2]))).toBe("0102");
  });

  it("refuses an odd number of digits", () => {
    expect(() => bytesFromHex("abc")).toThrow(/odd number/);
  });

  it("refuses non-hex", () => {
    expect(() => bytesFromHex("zzzz")).toThrow(/not hex/);
  });

  it("refuses a channel of the wrong length", () => {
    expect(() => channelFromHex("abcd")).toThrow(/32 hex characters/);
  });
});

/// A frame whose `seq` is encoded as a MessagePack uint64, built by hand because no
/// well-behaved encoder would emit one for a small value.
function withUint64Seq(seq: bigint): Uint8Array {
  const head = bytesFromHex(
    "86" +
      "a17601" + // v: 1
      "a46b696e6403" + // kind: 3
      "a76368616e6e656cc410" +
      hexFromBytes(CHANNEL) +
      "a3736571cf", // seq: uint64 follows
  );
  const seqBytes = new Uint8Array(8);
  new DataView(seqBytes.buffer).setBigUint64(0, seq, false);
  const tail = bytesFromHex("a361636b00a77061796c6f6164c401ff");

  const out = new Uint8Array(head.length + 8 + tail.length);
  out.set(head, 0);
  out.set(seqBytes, head.length);
  out.set(tail, head.length + 8);
  return out;
}
