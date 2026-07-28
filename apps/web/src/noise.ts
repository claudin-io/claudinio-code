/// `Noise_IK_25519_AESGCM_SHA256`, initiator side, on plain WebCrypto.
///
/// # Why no crypto library ships to the browser
///
/// Every primitive this needs — X25519, SHA-256, HKDF, AES-GCM — is in WebCrypto.
/// That is the whole reason the suite is AESGCM rather than ChaChaPoly: it means no
/// hand-audited cryptography in JavaScript, no wasm blob, and nothing in the bundle
/// that a supply-chain compromise could replace. The state machine below is the only
/// part that is ours, and it is held to `snow`'s output by golden vectors.
///
/// # Why IK
///
/// The initiator already knows the responder's static key, because the pairing code
/// carried it. The relay is therefore never in a position to supply one, and a
/// substituted key fails the handshake rather than succeeding quietly. What IK
/// cannot see is that key having been wrong *from the start* — a doctored code —
/// which is what the short authentication string closes, and why `sas()` exists.
///
/// Mirrors `src-tauri/src/remote/noise.rs`, which is the responder.

// Type-only: no runtime dependency, so nothing but the crypto below still ships here.
import type { Bytes } from "./wire";

const HASHLEN = 32;
const DHLEN = 32;
const TAGLEN = 16;
const PROTOCOL_NAME = "Noise_IK_25519_AESGCM_SHA256";

export class NoiseError extends Error {}

/// Which name this runtime knows X25519 by.
///
/// Chrome and Firefox use `X25519`; older Safari exposed the same curve as
/// `ECDH` over `namedCurve: "X25519"`. Detected once rather than assumed, because
/// getting it wrong is a blank page on one browser and the spike found real
/// divergence here.
let x25519: { algorithm: { name: string; namedCurve?: string } } | null = null;

export async function detectX25519(): Promise<{ algorithm: { name: string; namedCurve?: string } }> {
  if (x25519) return x25519;
  for (const algorithm of [{ name: "X25519" }, { name: "ECDH", namedCurve: "X25519" }]) {
    try {
      await crypto.subtle.generateKey(algorithm, false, ["deriveBits"]);
      x25519 = { algorithm };
      return x25519;
    } catch {
      // Next form.
    }
  }
  throw new NoiseError("this browser has no X25519 in WebCrypto");
}

export interface KeyPair {
  privateKey: CryptoKey;
  publicKey: Bytes;
}

export async function generateKeyPair(): Promise<KeyPair> {
  const { algorithm } = await detectX25519();
  // `extractable: false`. An XSS on this origin can *use* the key while the page is
  // open, which no browser-side design can prevent — but it cannot copy it out and
  // keep driving the machine afterwards. That is the difference between an incident
  // and a persistent foothold.
  const pair = (await crypto.subtle.generateKey(algorithm, false, [
    "deriveBits",
  ])) as CryptoKeyPair;
  const publicKey = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
  return { privateKey: pair.privateKey, publicKey };
}

/// Import a raw 32-byte X25519 private key.
///
/// WebCrypto will not take a raw private key, so it is wrapped in the PKCS#8
/// envelope of RFC 8410. Exists for the golden-vector tests, and for nothing else:
/// a real session generates its key with `generateKeyPair`, where it is
/// non-extractable. A key that arrived as bytes is a key that exists as bytes
/// somewhere, which is exactly what the other path avoids.
export async function importPrivateKeyForTesting(raw: Bytes): Promise<CryptoKey> {
  if (raw.length !== 32) throw new NoiseError("an X25519 private key is 32 bytes");
  const { algorithm } = await detectX25519();
  const pkcs8 = new Uint8Array([
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x04, 0x22, 0x04, 0x20,
    ...raw,
  ]);
  return crypto.subtle.importKey("pkcs8", pkcs8, algorithm, false, ["deriveBits"]);
}

function concat(...parts: Bytes[]): Bytes {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

async function sha256(data: Bytes): Promise<Bytes> {
  return new Uint8Array(await crypto.subtle.digest("SHA-256", data));
}

/// Noise's HKDF is HKDF-Extract(salt = chaining key) followed by HKDF-Expand with an
/// empty info — which is exactly what `deriveBits` computes, so the outputs fall out
/// as consecutive 32-byte slices with no hand-rolled HMAC anywhere.
async function hkdf(chainingKey: Bytes, ikm: Bytes, outputs: number): Promise<Bytes[]> {
  const key = await crypto.subtle.importKey("raw", ikm, "HKDF", false, ["deriveBits"]);
  const bits = await crypto.subtle.deriveBits(
    { name: "HKDF", hash: "SHA-256", salt: chainingKey, info: new Uint8Array(0) },
    key,
    HASHLEN * outputs * 8,
  );
  const all = new Uint8Array(bits);
  return Array.from({ length: outputs }, (_, i) => all.slice(i * HASHLEN, (i + 1) * HASHLEN));
}

async function dh(privateKey: CryptoKey, publicKeyBytes: Bytes): Promise<Bytes> {
  const { algorithm } = await detectX25519();
  const pub = await crypto.subtle.importKey("raw", publicKeyBytes, algorithm, false, []);
  const bits = await crypto.subtle.deriveBits({ ...algorithm, public: pub }, privateKey, 256);
  return new Uint8Array(bits);
}

/// Noise's AESGCM nonce: 32 zero bits, then the counter as 64 bits big-endian.
function nonceBytes(n: number): Bytes {
  const iv = new Uint8Array(12);
  new DataView(iv.buffer).setBigUint64(4, BigInt(n), false);
  return iv;
}

class CipherState {
  private key: CryptoKey | null = null;
  private n = 0;

  async initializeKey(keyBytes: Bytes): Promise<void> {
    this.key = await crypto.subtle.importKey("raw", keyBytes, "AES-GCM", false, [
      "encrypt",
      "decrypt",
    ]);
    this.n = 0;
  }

  async encryptWithAd(ad: Bytes, plaintext: Bytes): Promise<Bytes> {
    if (!this.key) return plaintext;
    const ct = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: nonceBytes(this.n), additionalData: ad, tagLength: 128 },
      this.key,
      plaintext,
    );
    this.n += 1;
    return new Uint8Array(ct);
  }

  async decryptWithAd(ad: Bytes, ciphertext: Bytes): Promise<Bytes> {
    if (!this.key) return ciphertext;
    try {
      const pt = await crypto.subtle.decrypt(
        { name: "AES-GCM", iv: nonceBytes(this.n), additionalData: ad, tagLength: 128 },
        this.key,
        ciphertext,
      );
      this.n += 1;
      return new Uint8Array(pt);
    } catch {
      // The nonce is deliberately *not* advanced on failure. A frame that did not
      // authenticate was tampered with or replayed, and it must not consume a
      // counter the legitimate next frame is going to use — otherwise anyone able
      // to inject one frame could desynchronise the session permanently.
      throw new NoiseError("frame failed to authenticate");
    }
  }
}

class SymmetricState {
  cipher = new CipherState();
  ck: Bytes = new Uint8Array(HASHLEN);
  h: Bytes = new Uint8Array(HASHLEN);

  async initialize(): Promise<void> {
    const name = new TextEncoder().encode(PROTOCOL_NAME);
    // Zero-padded when the name fits in a hash, hashed when it does not.
    if (name.length <= HASHLEN) {
      this.h = new Uint8Array(HASHLEN);
      this.h.set(name);
    } else {
      this.h = await sha256(name);
    }
    this.ck = this.h.slice();
  }

  async mixHash(data: Bytes): Promise<void> {
    this.h = await sha256(concat(this.h, data));
  }

  async mixKey(ikm: Bytes): Promise<void> {
    const [ck, tempK] = await hkdf(this.ck, ikm, 2);
    this.ck = ck;
    await this.cipher.initializeKey(tempK);
  }

  async encryptAndHash(plaintext: Bytes): Promise<Bytes> {
    const ct = await this.cipher.encryptWithAd(this.h, plaintext);
    await this.mixHash(ct);
    return ct;
  }

  async decryptAndHash(ciphertext: Bytes): Promise<Bytes> {
    const pt = await this.cipher.decryptWithAd(this.h, ciphertext);
    await this.mixHash(ciphertext);
    return pt;
  }

  async split(): Promise<{ send: CipherState; recv: CipherState }> {
    const [k1, k2] = await hkdf(this.ck, new Uint8Array(0), 2);
    const send = new CipherState();
    const recv = new CipherState();
    await send.initializeKey(k1);
    await recv.initializeKey(k2);
    return { send, recv };
  }
}

/// How long message one is: ephemeral, encrypted static, encrypted empty payload.
export const MSG1_LENGTH = DHLEN + (DHLEN + TAGLEN) + TAGLEN;

export class NoiseInitiator {
  private sym = new SymmetricState();
  private ephemeral: KeyPair | null = null;
  private transport: { send: CipherState; recv: CipherState } | null = null;
  private hash: Bytes | null = null;

  private constructor(
    private readonly staticKeys: KeyPair,
    private readonly remoteStatic: Bytes,
  ) {}

  /// Start a handshake towards a device whose static key came from a pairing code.
  ///
  /// `ephemeral` is for the golden-vector tests only. A real session must generate
  /// a fresh one, and does, because leaving it out is the default.
  static async start(
    staticKeys: KeyPair,
    remoteStatic: Bytes,
    ephemeral?: KeyPair,
  ): Promise<{ initiator: NoiseInitiator; message1: Bytes }> {
    if (remoteStatic.length !== DHLEN) {
      throw new NoiseError(`a device key is ${DHLEN} bytes, got ${remoteStatic.length}`);
    }
    const initiator = new NoiseInitiator(staticKeys, remoteStatic);
    const message1 = await initiator.writeMessage1(ephemeral);
    return { initiator, message1 };
  }

  /// `-> e, es, s, ss`
  private async writeMessage1(ephemeral?: KeyPair): Promise<Bytes> {
    await this.sym.initialize();
    await this.sym.mixHash(new Uint8Array(0)); // empty prologue
    await this.sym.mixHash(this.remoteStatic); // pre-message: <- s

    this.ephemeral = ephemeral ?? (await generateKeyPair());
    await this.sym.mixHash(this.ephemeral.publicKey); // e
    await this.sym.mixKey(await dh(this.ephemeral.privateKey, this.remoteStatic)); // es
    const encryptedStatic = await this.sym.encryptAndHash(this.staticKeys.publicKey); // s
    await this.sym.mixKey(await dh(this.staticKeys.privateKey, this.remoteStatic)); // ss
    const encryptedPayload = await this.sym.encryptAndHash(new Uint8Array(0));

    return concat(this.ephemeral.publicKey, encryptedStatic, encryptedPayload);
  }

  /// `<- e, ee, se`, which completes the handshake.
  async readMessage2(message: Bytes): Promise<void> {
    if (!this.ephemeral) throw new NoiseError("handshake not started");
    if (this.transport) throw new NoiseError("handshake already complete");
    if (message.length < DHLEN + TAGLEN) {
      throw new NoiseError("the second handshake message is too short");
    }

    const re = message.slice(0, DHLEN);
    const rest = message.slice(DHLEN);
    await this.sym.mixHash(re); // e
    await this.sym.mixKey(await dh(this.ephemeral.privateKey, re)); // ee
    await this.sym.mixKey(await dh(this.staticKeys.privateKey, re)); // se
    await this.sym.decryptAndHash(rest);

    this.hash = this.sym.h.slice();
    this.transport = await this.sym.split();
  }

  get complete(): boolean {
    return this.transport !== null;
  }

  /// The handshake hash. Identical on both ends only if the handshake was genuinely
  /// end to end, which is what makes the SAS worth comparing.
  get handshakeHash(): Bytes {
    if (!this.hash) throw new NoiseError("handshake not complete");
    return this.hash;
  }

  async encrypt(plaintext: Bytes): Promise<Bytes> {
    if (!this.transport) throw new NoiseError("handshake not complete");
    return this.transport.send.encryptWithAd(new Uint8Array(0), plaintext);
  }

  async decrypt(ciphertext: Bytes): Promise<Bytes> {
    if (!this.transport) throw new NoiseError("handshake not complete");
    return this.transport.recv.decryptWithAd(new Uint8Array(0), ciphertext);
  }
}
