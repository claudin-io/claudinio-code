import { describe, it, expect } from "vitest";
import { formatSas } from "@claudinio/protocol/sas";
import { bytesFromHex, hexFromBytes } from "./wire";
import type { Bytes } from "./wire";
import { GOLDEN } from "./golden";
import {
  MSG1_LENGTH,
  NoiseError,
  NoiseInitiator,
  generateKeyPair,
  importPrivateKeyForTesting,
} from "./noise";

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

const text = (bytes: Bytes) => new TextDecoder().decode(bytes);

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
