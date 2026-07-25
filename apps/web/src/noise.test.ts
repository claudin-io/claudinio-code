import { describe, it, expect } from "vitest";
import { formatSas } from "@claudinio/protocol/sas";
import { bytesFromHex, hexFromBytes } from "./wire";
import {
  MSG1_LENGTH,
  NoiseError,
  NoiseInitiator,
  generateKeyPair,
  importPrivateKeyForTesting,
} from "./noise";

/// A recorded handshake from `snow`, with both ephemerals fixed.
///
/// Regenerate with, in `src-tauri/`:
///   cargo test --features remote golden -- --ignored --nocapture
/// The static keys change on every run, so these are replaced as a set.
///
/// This is the only thing here that can show the two implementations agree. An
/// initiator tested against a responder written in the same language proves they
/// agree with each other and nothing about whether either agrees with the device.
const GOLDEN = {
  initStaticPrivate: "85bdf2a5f60f1ab0ea933480e437ee04d628491d06f1153cd70d1e1fc0a10634",
  initStaticPublic: "eb53c549ef61da2784d51f7d3a58476b74932922a3bfa5f227b510cef11eba1d",
  initEphemeralPrivate: "0202020202020202020202020202020202020202020202020202020202020202",
  respStaticPublic: "1f4ff8f62422c38587c0d7b30b5485cbd2b05b48f75b8b1ec80d766bb5b3453a",
  msg1: "ce8d3ad1ccb633ec7b70c17814a5c76ecd029685050d344745ba05870e587d594b62ca9e27c6ef8b5b28b8762d4c0e96f8991e0bbd870902aff2716795de38e4681ac6f05239ccb61011847448bf006d36e5eb528ad122894bf236606f571d45",
  msg2: "ac01b2209e86354fb853237b5de0f4fab13c7fcbf433a61c019369617fecf10ba33a87d1017a524080da269a4e168359",
  handshakeHash: "f4f92fd16e4182c365e71483fbcc809eb2a17facf878c58f15ecbe19d7a6edc4",
  sas: "zephyr · rivet · basalt",
  deviceCiphertext: "68c38ada9d1f1bad0b3f97507d88b69c0b0c6f0ddbd5860b5a92f6dfba8e828feb4e6d77f8",
  devicePlaintext: "hello from the device",
  peerCiphertext: "c34cab1aa28b9eefbc77d762288a2faa8d1ed3a8bcd13eeb00c772ef432332fbaecc11ea01b0",
  peerPlaintext: "hello from the browser",
};

async function goldenInitiator() {
  const staticKeys = {
    privateKey: await importPrivateKeyForTesting(bytesFromHex(GOLDEN.initStaticPrivate)),
    publicKey: bytesFromHex(GOLDEN.initStaticPublic),
  };
  const ephemeral = {
    privateKey: await importPrivateKeyForTesting(bytesFromHex(GOLDEN.initEphemeralPrivate)),
    // Not used by the state machine's DH steps — only mixed into the hash — but it
    // has to be the public half of the fixed private key or msg1 will not match.
    publicKey: bytesFromHex(GOLDEN.msg1).slice(0, 32),
  };
  return NoiseInitiator.start(staticKeys, bytesFromHex(GOLDEN.respStaticPublic), ephemeral);
}

const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

describe("golden handshake against snow", () => {
  /// If this fails, the browser cannot pair with the device at all — whatever else
  /// passes.
  it("produces the first message snow expects", async () => {
    const { message1 } = await goldenInitiator();
    expect(hexFromBytes(message1)).toBe(GOLDEN.msg1);
  });

  it("completes the handshake from snow's reply", async () => {
    const { initiator } = await goldenInitiator();
    await initiator.readMessage2(bytesFromHex(GOLDEN.msg2));

    expect(initiator.complete).toBe(true);
    expect(hexFromBytes(initiator.handshakeHash)).toBe(GOLDEN.handshakeHash);
  });

  /// The words are the security boundary of pairing. Deriving them from the same
  /// hash is necessary but not sufficient — they also have to come out of the same
  /// word list, which is why this imports the generated bindings rather than
  /// carrying its own copy.
  it("derives the same three words the device shows", async () => {
    const { initiator } = await goldenInitiator();
    await initiator.readMessage2(bytesFromHex(GOLDEN.msg2));

    expect(formatSas(initiator.handshakeHash)).toBe(GOLDEN.sas);
  });

  /// Split and nonce handling, not just the handshake. A transport key derived in
  /// the wrong order produces a session that handshakes and then says nothing.
  it("decrypts what the device sends", async () => {
    const { initiator } = await goldenInitiator();
    await initiator.readMessage2(bytesFromHex(GOLDEN.msg2));

    const plaintext = await initiator.decrypt(bytesFromHex(GOLDEN.deviceCiphertext));
    expect(text(plaintext)).toBe(GOLDEN.devicePlaintext);
  });

  /// And the other direction, which is what the device has to be able to read.
  it("encrypts what the device can decrypt", async () => {
    const { initiator } = await goldenInitiator();
    await initiator.readMessage2(bytesFromHex(GOLDEN.msg2));

    const ciphertext = await initiator.encrypt(new TextEncoder().encode(GOLDEN.peerPlaintext));
    expect(hexFromBytes(ciphertext)).toBe(GOLDEN.peerCiphertext);
  });

  it("uses the message length the protocol fixes", () => {
    expect(bytesFromHex(GOLDEN.msg1).length).toBe(MSG1_LENGTH);
  });
});

describe("NoiseInitiator", () => {
  it("generates a fresh ephemeral when none is given", async () => {
    const staticKeys = await generateKeyPair();
    const remote = bytesFromHex(GOLDEN.respStaticPublic);

    const first = await NoiseInitiator.start(staticKeys, remote);
    const second = await NoiseInitiator.start(staticKeys, remote);

    expect(hexFromBytes(first.message1)).not.toBe(hexFromBytes(second.message1));
  });

  /// A key that arrived as bytes exists as bytes somewhere. The generated one does
  /// not: an XSS on this origin can use it while the page is open but cannot copy it
  /// out and keep driving the machine afterwards.
  it("generates a static key that cannot be exported", async () => {
    const keys = await generateKeyPair();
    expect(keys.privateKey.extractable).toBe(false);
    await expect(crypto.subtle.exportKey("pkcs8", keys.privateKey)).rejects.toThrow();
  });

  it("refuses a device key of the wrong length", async () => {
    const staticKeys = await generateKeyPair();
    await expect(NoiseInitiator.start(staticKeys, new Uint8Array(16))).rejects.toThrow(NoiseError);
  });

  it("refuses to encrypt before the handshake finishes", async () => {
    const { initiator } = await goldenInitiator();
    expect(initiator.complete).toBe(false);
    await expect(initiator.encrypt(new Uint8Array(1))).rejects.toThrow(/not complete/);
    await expect(initiator.decrypt(new Uint8Array(32))).rejects.toThrow(/not complete/);
    expect(() => initiator.handshakeHash).toThrow(/not complete/);
  });

  it("refuses a second handshake on the same session", async () => {
    const { initiator } = await goldenInitiator();
    await initiator.readMessage2(bytesFromHex(GOLDEN.msg2));
    await expect(initiator.readMessage2(bytesFromHex(GOLDEN.msg2))).rejects.toThrow(
      /already complete/,
    );
  });

  it("refuses a second message that is too short to be one", async () => {
    const { initiator } = await goldenInitiator();
    await expect(initiator.readMessage2(new Uint8Array(8))).rejects.toThrow(/too short/);
  });

  /// A tampered reply must fail the handshake rather than produce a session. This is
  /// the property that makes a substituted device key useless: the relay cannot
  /// forge a reply that authenticates.
  it("fails the handshake when the reply is tampered with", async () => {
    const { initiator } = await goldenInitiator();
    const msg2 = bytesFromHex(GOLDEN.msg2);
    msg2[msg2.length - 1] ^= 0xff;

    await expect(initiator.readMessage2(msg2)).rejects.toThrow(NoiseError);
    expect(initiator.complete).toBe(false);
  });

  /// A wrong device key — which is what a doctored pairing code carries — must not
  /// silently produce a working session. It cannot: the reply will not authenticate.
  it("fails against a device key that is not the device's", async () => {
    const wrong = bytesFromHex(GOLDEN.respStaticPublic);
    wrong[0] ^= 0xff;
    const staticKeys = {
      privateKey: await importPrivateKeyForTesting(bytesFromHex(GOLDEN.initStaticPrivate)),
      publicKey: bytesFromHex(GOLDEN.initStaticPublic),
    };

    const { initiator } = await NoiseInitiator.start(staticKeys, wrong);
    await expect(initiator.readMessage2(bytesFromHex(GOLDEN.msg2))).rejects.toThrow(NoiseError);
  });

  /// The nonce must not advance on a frame that failed to authenticate. If it did,
  /// anyone able to inject one frame could desynchronise the session for good — a
  /// denial of service available to the relay, which is meant to be untrusted but
  /// not meant to be able to do that.
  it("keeps the session usable after rejecting a tampered frame", async () => {
    const { initiator } = await goldenInitiator();
    await initiator.readMessage2(bytesFromHex(GOLDEN.msg2));

    const tampered = bytesFromHex(GOLDEN.deviceCiphertext);
    tampered[0] ^= 0xff;
    await expect(initiator.decrypt(tampered)).rejects.toThrow(NoiseError);

    // The genuine frame still decrypts, at the same counter.
    const plaintext = await initiator.decrypt(bytesFromHex(GOLDEN.deviceCiphertext));
    expect(text(plaintext)).toBe(GOLDEN.devicePlaintext);
  });
});
