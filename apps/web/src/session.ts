/// The connection: dial the relay, handshake, subscribe, stay connected.
///
/// Mirrors `src-tauri/src/remote/transport.rs` from the other side, and inherits
/// three lessons the relay's prova real paid for:
///
/// - **Only the initiator retries.** The device waits for a first frame
///   indefinitely; if this side also bounded its wait and slept, the two would wake
///   and sleep alternately and miss each other. So the browser retries the
///   handshake and the device never does.
/// - **A `Hello` on an established session is a re-handshake**, not data. Treating
///   it as data fails to decrypt and then rejects every frame after it, leaving a
///   session that cannot recover until one side restarts.
/// - **A frame that fails to authenticate is dropped, not fatal.** Dropping the
///   connection instead would let a hostile relay disconnect a peer at will.

import { formatSas } from "@claudinio/protocol/sas";
import { NoiseInitiator, generateKeyPair } from "./noise";
import type { PairingCode } from "./pairing";
import {
  MAX_FRAME,
  OuterKind,
  bytesFromHex,
  channelFromHex,
  dataFrame,
  decodeFrame,
  encodeFrame,
  helloFrame,
} from "./wire";
import type { Bytes } from "./wire";
import { decode as msgpackDecode, encode as msgpackEncode } from "@msgpack/msgpack";

/// How long to wait for the device's reply before dialling again.
///
/// The device may have no peer and be sitting idle, or the relay may have accepted
/// the socket and have nothing on the other side. Neither is an error; both look
/// identical from here, and the only way through is to try again.
export const HANDSHAKE_TIMEOUT_MS = 5_000;

/// Backoff for redialling, matching the device's curve closely enough that the two
/// do not settle into lockstep.
export const BACKOFF_BASE_MS = 500;
export const BACKOFF_CEILING_MS = 30_000;

/// The clocks a session runs on.
///
/// Settings rather than constants for two reasons. §0.1 point 3 says a backgrounded
/// phone drops its socket constantly and the plan expects these tuned for cellular, so
/// they were always going to need to differ by surface. And the tests need them small:
/// faking the clock instead meant a wait for the handshake — several WebCrypto awaits —
/// could cross the faked five-second deadline, which is how three flakes got in and one
/// of them only ever failed on CI.
export interface Timings {
  handshakeTimeoutMs: number;
  backoffBaseMs: number;
  backoffCeilingMs: number;
}

export const DEFAULT_TIMINGS: Timings = {
  handshakeTimeoutMs: HANDSHAKE_TIMEOUT_MS,
  backoffBaseMs: BACKOFF_BASE_MS,
  backoffCeilingMs: BACKOFF_CEILING_MS,
};

export type SessionState =
  | { kind: "connecting" }
  /// Handshake done, waiting for the user to compare the words. Nothing is requested
  /// until they do — the device is not serving yet either.
  | { kind: "confirming"; sas: string }
  | { kind: "live"; sas: string }
  /// The device said it stopped, and will not answer again until someone acts on it.
  | { kind: "closed"; reason: CloseReason }
  | { kind: "failed"; why: string };

export type CloseReason = "peer_asked" | "turned_off_locally" | "grant_expired" | "revoked";

/// What the device says. A subset: this app is read-only in phase 3, so the write
/// path's replies are decoded and ignored rather than absent.
export interface DeviceMessage {
  kind: string;
  [key: string]: unknown;
}

export interface SessionEvents {
  onState: (state: SessionState) => void;
  onMessage: (message: DeviceMessage) => void;
}

/// A socket, narrowed to what this uses. Injectable so the session can be tested
/// without a network — and so the test drives it with bytes the real device produced.
export interface Socket {
  send(data: Bytes): void;
  close(): void;
  onopen: (() => void) | null;
  onmessage: ((data: Bytes) => void) | null;
  onclose: (() => void) | null;
  onerror: (() => void) | null;
}

export type SocketFactory = (url: string) => Socket;

/// How a handshake is started. Injectable for the same reason as the socket: the
/// golden-vector tests need the recorded keys, and a session that could only ever
/// generate its own could not be checked against the device's real output.
///
/// The default generates a fresh, non-extractable static key per connection. The
/// browser is not a paired identity that persists — the device authenticates the key
/// it sees against its pairing book, and a browser that kept a key across reloads
/// would be storing one, which is what the non-extractable default avoids.
export type InitiatorFactory = (
  deviceKey: Bytes,
) => Promise<{ initiator: NoiseInitiator; message1: Bytes }>;

export const freshInitiator: InitiatorFactory = async (deviceKey) => {
  const staticKeys = await generateKeyPair();
  return NoiseInitiator.start(staticKeys, deviceKey);
};

/// The default factory: a real browser WebSocket.
export const browserSocket: SocketFactory = (url) => {
  const ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";
  const socket: Socket = {
    send: (data) => ws.send(data),
    close: () => ws.close(),
    onopen: null,
    onmessage: null,
    onclose: null,
    onerror: null,
  };
  ws.onopen = () => socket.onopen?.();
  ws.onmessage = (event) => socket.onmessage?.(new Uint8Array(event.data as ArrayBuffer));
  ws.onclose = () => socket.onclose?.();
  ws.onerror = () => socket.onerror?.();
  return socket;
};

export class Session {
  private socket: Socket | null = null;
  private noise: NoiseInitiator | null = null;
  private channel: Bytes;
  private outSeq = 0;
  private highestSeq = 0;
  private attempt = 0;
  private stopped = false;
  private confirmed = false;
  private handshakeTimer: ReturnType<typeof setTimeout> | null = null;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private sasWords: string | null = null;
  /// Which session to follow, once the user has confirmed. Set by `subscribe`.
  private subscribeTo: { sessionId: string; fromSeq: number } | null = null;

  constructor(
    private readonly code: PairingCode,
    private readonly events: SessionEvents,
    private readonly makeSocket: SocketFactory = browserSocket,
    private readonly makeInitiator: InitiatorFactory = freshInitiator,
    private readonly timings: Timings = DEFAULT_TIMINGS,
  ) {
    this.channel = channelFromHex(code.channel);
  }

  start(): void {
    this.stopped = false;
    this.dial();
  }

  /// Give up for good. Called when the user leaves, and when the device says it has
  /// stopped — after which redialling would hammer something that will not answer.
  stop(): void {
    this.stopped = true;
    this.clearTimers();
    this.socket?.close();
    this.socket = null;
  }

  /// The user compared the words and they matched.
  ///
  /// Until this is called the session sends nothing at all. The device is doing the
  /// same on its side, so requesting anything earlier would only be refused — but
  /// more to the point, a browser that started pulling a transcript before the human
  /// check would make the check decorative, which is the mistake the device side
  /// already had once.
  confirm(): void {
    if (this.confirmed || !this.noise?.complete) return;
    this.confirmed = true;
    this.emitState({ kind: "live", sas: this.sasWords ?? "" });
    void this.flushSubscribe();
  }

  /// Ask to follow a session from a given point. Safe before confirmation: it is
  /// remembered and sent once the words are agreed.
  subscribe(sessionId: string, fromSeq = 0): void {
    this.subscribeTo = { sessionId, fromSeq };
    void this.flushSubscribe();
  }

  private async flushSubscribe(): Promise<void> {
    if (!this.confirmed || !this.subscribeTo || !this.noise?.complete) return;
    await this.send({
      kind: "subscribe",
      session_id: this.subscribeTo.sessionId,
      // Resume from what has already arrived rather than from what was asked for, so
      // a reconnect does not replay ground already covered.
      from_seq: Math.max(this.subscribeTo.fromSeq, this.highestSeq + 1),
    });
  }

  private dial(): void {
    if (this.stopped) return;
    this.clearTimers();
    this.noise = null;
    this.outSeq = 0;
    this.emitState({ kind: "connecting" });

    const url =
      `${this.code.relayUrl}?channel=${encodeURIComponent(this.code.channel)}` +
      `&role=peer&token=${encodeURIComponent(this.code.token)}`;

    let socket: Socket;
    try {
      socket = this.makeSocket(url);
    } catch (e) {
      this.emitState({ kind: "failed", why: message(e) });
      this.scheduleRetry();
      return;
    }
    this.socket = socket;

    socket.onopen = () => void this.handshake();
    socket.onmessage = (data) => void this.receive(data);
    // A closed socket is not a failure. The relay restarts, a phone changes network,
    // a tunnel drops — all of it is ordinary, and none of it should say "error".
    socket.onclose = () => this.scheduleRetry();
    socket.onerror = () => this.scheduleRetry();
  }

  private async handshake(): Promise<void> {
    try {
      const { initiator, message1 } = await this.makeInitiator(
        bytesFromHex(this.code.deviceKey),
      );
      this.noise = initiator;
      this.socket?.send(encodeFrame(helloFrame(this.channel, message1)));

      // Bounded, and only on this side. A device with no peer waits indefinitely and
      // that costs it nothing; if both ends bounded their wait they would take turns
      // sleeping and never meet.
      this.handshakeTimer = setTimeout(() => {
        if (!this.noise?.complete) this.redial();
      }, this.timings.handshakeTimeoutMs);
    } catch (e) {
      this.emitState({ kind: "failed", why: message(e) });
      this.scheduleRetry();
    }
  }

  private async receive(bytes: Bytes): Promise<void> {
    if (bytes.length > MAX_FRAME) return; // refused without decoding

    let frame;
    try {
      frame = decodeFrame(bytes);
    } catch {
      // Undecodable. Dropped rather than fatal: the relay is untrusted, and letting
      // it end the session by sending rubbish would hand it a disconnect button.
      return;
    }

    // A frame claiming a different channel is not ours. The relay should never route
    // one here, and if it does, believing it would be believing the untrusted party
    // about which conversation this is.
    if (!sameBytes(frame.channel, this.channel)) return;

    if (frame.kind === OuterKind.HelloAck) {
      await this.completeHandshake(frame.payload);
      return;
    }
    if (frame.kind !== OuterKind.Data) return; // Ping, Close, or something newer

    if (!this.noise?.complete) return;

    let plaintext: Bytes;
    try {
      plaintext = await this.noise.decrypt(frame.payload);
    } catch {
      // Tampered or replayed. Dropped, and the nonce did not advance, so the next
      // genuine frame still authenticates.
      return;
    }

    let inner: unknown;
    try {
      inner = msgpackDecode(plaintext);
    } catch {
      return;
    }
    if (typeof inner !== "object" || inner === null || Array.isArray(inner)) return;

    const record = inner as DeviceMessage;
    if (typeof record.kind !== "string") return;

    // Track what has arrived, so a reconnect resumes rather than replaying.
    if (typeof record.seq === "number" && record.seq > this.highestSeq) {
      this.highestSeq = record.seq;
    }

    if (record.kind === "closed") {
      const reason = (record.reason as CloseReason) ?? "turned_off_locally";
      this.emitState({ kind: "closed", reason });
      // Told, not guessed. Retrying would hammer a device that has said it is done,
      // and would hide from the user the one thing they need to know: that getting
      // back in takes an action on the machine.
      this.stop();
      return;
    }

    this.events.onMessage(record);
  }

  private async completeHandshake(payload: Bytes): Promise<void> {
    if (!this.noise || this.noise.complete) return;
    try {
      await this.noise.readMessage2(payload);
    } catch {
      // The reply did not authenticate, which means the key in the pairing code is
      // not this device's — or something tried to answer for it. Not retried: a
      // fresh dial would fail identically, and saying so is more use than a spinner.
      this.emitState({
        kind: "failed",
        why: "the device did not answer with the key this code names",
      });
      this.stop();
      return;
    }

    this.clearHandshakeTimer();
    this.attempt = 0;
    this.sasWords = formatSas(this.noise.handshakeHash);

    if (this.confirmed) {
      // A reconnect after the words were already agreed. The SAS changes with every
      // handshake — it is derived from ephemerals — so it is *not* re-confirmed here:
      // asking again on every network blip is how a user learns to tap through it.
      // The pairing was vouched for once; the device authenticates the key from then
      // on.
      this.emitState({ kind: "live", sas: this.sasWords });
      void this.flushSubscribe();
    } else {
      this.emitState({ kind: "confirming", sas: this.sasWords });
    }
  }

  private async send(message: Record<string, unknown>): Promise<void> {
    if (!this.noise?.complete || !this.socket) return;
    const plaintext = msgpackEncode(message);
    const ciphertext = await this.noise.encrypt(plaintext);
    this.outSeq += 1;
    this.socket.send(
      encodeFrame(dataFrame(this.channel, this.outSeq, this.highestSeq, ciphertext)),
    );
  }

  private redial(): void {
    this.socket?.close();
    this.socket = null;
    this.dial();
  }

  private scheduleRetry(): void {
    if (this.stopped || this.retryTimer) return;
    this.clearHandshakeTimer();
    this.attempt += 1;
    // Full jitter. Several peers reconnecting after a relay restart must not arrive
    // together, which is what an unjittered curve guarantees.
    const ceiling = Math.min(
      this.timings.backoffCeilingMs,
      this.timings.backoffBaseMs * 2 ** (this.attempt - 1),
    );
    const delay = Math.random() * ceiling;
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      this.dial();
    }, delay);
  }

  private clearHandshakeTimer(): void {
    if (this.handshakeTimer !== null) {
      clearTimeout(this.handshakeTimer);
      this.handshakeTimer = null;
    }
  }

  private clearTimers(): void {
    this.clearHandshakeTimer();
    if (this.retryTimer !== null) {
      clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
  }

  private emitState(state: SessionState): void {
    this.events.onState(state);
  }
}

function sameBytes(a: Bytes, b: Bytes): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
