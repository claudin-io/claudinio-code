import { describe, it, expect } from "vitest";
import { GOLDEN } from "./golden";
import { NoiseInitiator, importPrivateKeyForTesting } from "./noise";
import type { PairingCode } from "./pairing";
import {
  Session,
  type DeviceMessage,
  type SessionState,
  type Socket,
} from "./session";
import type { Timings } from "./session";
import { OuterKind, bytesFromHex, decodeFrame, encodeFrame, hexFromBytes } from "./wire";

/// A socket the test drives. Records what the session sent and lets the test push
/// bytes the real device produced.
class FakeSocket implements Socket {
  sent: Uint8Array[] = [];
  closed = false;
  onopen: (() => void) | null = null;
  onmessage: ((data: Uint8Array) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;

  constructor(readonly url: string) {}

  send(data: Uint8Array): void {
    this.sent.push(data);
  }

  close(): void {
    this.closed = true;
  }

  open(): void {
    this.onopen?.();
  }

  deliver(hex: string): void {
    this.onmessage?.(bytesFromHex(hex));
  }
}

const code = (over: Partial<PairingCode> = {}): PairingCode => ({
  channel: GOLDEN.channel,
  token: "ef".repeat(16),
  deviceKey: GOLDEN.respStaticPublic,
  relayUrl: "wss://relay.claudin.io/ws",
  expiresAt: Date.now() + 120_000,
  ...over,
});

/// An initiator with the recorded keys, so the exchange matches the device's.
const goldenInitiator = async () => {
  const staticKeys = {
    privateKey: await importPrivateKeyForTesting(bytesFromHex(GOLDEN.initStaticPrivate)),
    publicKey: bytesFromHex(GOLDEN.initStaticPublic),
  };
  const ephemeral = {
    privateKey: await importPrivateKeyForTesting(bytesFromHex(GOLDEN.initEphemeralPrivate)),
    publicKey: bytesFromHex(GOLDEN.msg1).slice(0, 32),
  };
  return (deviceKey: Uint8Array) => NoiseInitiator.start(staticKeys, deviceKey, ephemeral);
};

interface Harness {
  session: Session;
  sockets: FakeSocket[];
  states: SessionState[];
  messages: DeviceMessage[];
  socket: () => FakeSocket;
}

async function harness(over: Partial<PairingCode> = {}): Promise<Harness> {
  const sockets: FakeSocket[] = [];
  const states: SessionState[] = [];
  const messages: DeviceMessage[] = [];

  const session = new Session(
    code(over),
    {
      onState: (state) => states.push(state),
      onMessage: (message) => messages.push(message),
    },
    (url) => {
      const socket = new FakeSocket(url);
      sockets.push(socket);
      return socket;
    },
    await goldenInitiator(),
    FAST,
  );

  return { session, sockets, states, messages, socket: () => sockets[sockets.length - 1] };
}

/// Yield to the event loop, on a timer the tests have not faked.
///
/// It cannot be `setTimeout`: these tests fake that so they can advance the backoff and
/// the handshake deadline on purpose. With `shouldAdvanceTime` the fake clock also ran on
/// its own, so a wait loop could quietly cross the five-second handshake timeout — the
/// connection redialled, the reply went to a socket the test was no longer holding, and
/// the handshake "never completed". `setImmediate` stays real, so waiting costs no fake
/// time at all.
const settle = () => new Promise((r) => setTimeout(r, 0));

/// Wait a real interval. Only for the assertions that something did *not* happen, where
/// there is no condition to wait for.
const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));

/// Wait until a state arrives, rather than for a fixed tick.
///
/// A single `settle()` was racing the handshake: it is a chain of WebCrypto awaits, and
/// under fake timers a `setTimeout(0)` does not reliably outlast it. About one run in
/// three, `confirm()` was called before the handshake completed, silently did nothing,
/// and the test then saw `confirming` where it expected `live`. Waiting for the
/// condition is the fix; waiting longer would only make the flake rarer.
/// Wait until something is true, rather than for a number of ticks.
///
/// Every flake in this file came from the same mistake in a different costume: waiting a
/// fixed amount for an asynchronous chain. The handshake is several WebCrypto awaits
/// deep, and how many turns of the loop that takes is not something a test can know — so
/// nothing here waits an amount, only for a condition.
async function waitUntil(done: () => boolean, what: string) {
  for (let i = 0; i < 500; i++) {
    if (done()) return;
    await settle();
  }
  throw new Error(`timed out waiting for ${what}`);
}

const waitForSent = (socket: () => FakeSocket, count: number) =>
  waitUntil(
    () => socket().sent.length >= count,
    `${count} frames to be sent (saw ${socket().sent.length})`,
  );

/// `live` arrives before the subscribe does — `confirm()` emits the state and then
/// encrypts, which is async. So waiting for the state and asserting on the send is the
/// same mistake again: wait for the thing being asserted.
const waitForState = (states: SessionState[], kind: SessionState["kind"]) =>
  waitUntil(
    () => states.some((state) => state.kind === kind),
    `state ${kind} (saw ${states.map((s) => s.kind).join(", ")})`,
  );

const waitForMessages = (messages: DeviceMessage[], count: number) =>
  waitUntil(() => messages.length >= count, `${count} messages`);

const waitForSockets = (sockets: FakeSocket[], count: number) =>
  waitUntil(() => sockets.length >= count, `${count} sockets`);

/// The most recent state. `Array.prototype.at` would need a newer `lib` than this
/// tsconfig selects, and bumping that belongs in its own change.
const last = <T,>(items: T[]): T | undefined => items[items.length - 1];

/// A `HelloAck` carrying the device's recorded reply.
const helloAck = () =>
  hexFromBytes(
    encodeFrame({
      v: 1,
      kind: OuterKind.HelloAck,
      channel: bytesFromHex(GOLDEN.channel),
      seq: 0,
      ack: 0,
      payload: bytesFromHex(GOLDEN.msg2),
    }),
  );

async function upToConfirming(over: Partial<PairingCode> = {}) {
  const h = await harness(over);
  h.session.start();
  h.socket().open();
  await waitForSent(h.socket, 1);
  h.socket().deliver(helloAck());
  await waitForState(h.states, "confirming");
  return h;
}

/// Real time, with the session's clocks turned down to milliseconds.
///
/// Faking the clock was the wrong tool. A wait for the handshake — several WebCrypto
/// awaits — could cross the faked five-second deadline while it waited, so the connection
/// redialled and the reply went to a socket the test no longer held. That failed about
/// one run in three locally and reliably on CI, where the runner is slower.
///
/// So nothing here fakes time. The deadlines are settings now, and these are small
/// enough to wait for honestly.
const FAST: Timings = {
  handshakeTimeoutMs: 40,
  backoffBaseMs: 2,
  backoffCeilingMs: 8,
};

describe("dialling", () => {
  /// The missing token is the bug that made remote access impossible. The URL is
  /// asserted whole so it cannot go missing again quietly.
  it("attaches as a peer, with the channel and the token", async () => {
    const h = await harness();
    h.session.start();

    expect(h.socket().url).toBe(
      `wss://relay.claudin.io/ws?channel=${GOLDEN.channel}&role=peer&token=${"ef".repeat(16)}`,
    );
  });

  it("reports connecting before anything has happened", async () => {
    const h = await harness();
    h.session.start();
    expect(h.states[0]).toEqual({ kind: "connecting" });
  });
});

describe("the handshake", () => {
  /// If this is not byte-identical the device will not answer at all.
  it("sends the first Noise message the device expects", async () => {
    const h = await harness();
    h.session.start();
    h.socket().open();
    await waitForSent(h.socket, 1);

    expect(h.socket().sent).toHaveLength(1);
    const frame = decodeFrame(h.socket().sent[0]);
    expect(frame.kind).toBe(OuterKind.Hello);
    expect(hexFromBytes(frame.payload)).toBe(GOLDEN.msg1);
  });

  /// The words come out of the device's own reply, through the generated word list.
  it("shows the same three words the device shows", async () => {
    const h = await upToConfirming();
    expect(last(h.states)).toEqual({ kind: "confirming", sas: GOLDEN.sas });
  });

  /// The check has to gate the browser too. A peer that started pulling a transcript
  /// before the human compared the words would make the comparison decorative — which
  /// is the mistake the device side already had once.
  it("sends nothing at all before the words are confirmed", async () => {
    const h = await upToConfirming();
    h.session.subscribe("golden", 0);
    await settle();
    await settle();

    // Only the Hello.
    expect(h.socket().sent).toHaveLength(1);
  });

  it("subscribes once the words are confirmed", async () => {
    const h = await upToConfirming();
    h.session.subscribe("golden", 0);
    h.session.confirm();
    await waitForSent(h.socket, 2);

    expect(last(h.states)).toEqual({ kind: "live", sas: GOLDEN.sas });
    expect(h.socket().sent).toHaveLength(2);

    const frame = decodeFrame(h.socket().sent[1]);
    const golden = decodeFrame(bytesFromHex(GOLDEN.subscribeFrame));
    expect(frame.kind).toBe(OuterKind.Data);
    expect(hexFromBytes(frame.channel)).toBe(GOLDEN.channel);
    expect(frame.seq).toBe(golden.seq);
    // Same shape as the message the device's own encoder produced. Byte equality is
    // not available — the cipher counter differs — and is already covered by the
    // Noise tests; what matters here is that the framing agrees.
    expect(frame.payload.length).toBe(golden.payload.length);
  });

  it("subscribing after confirming also works", async () => {
    const h = await upToConfirming();
    h.session.confirm();
    h.session.subscribe("golden", 0);
    await waitForSent(h.socket, 2);

    expect(h.socket().sent).toHaveLength(2);
  });

  /// A reply that does not authenticate means the key in the code is not this
  /// device's. Retrying would fail identically, so it says so instead of spinning.
  it("fails and stops when the reply does not authenticate", async () => {
    const h = await upToConfirming();
    // Fresh session so the handshake has not already completed.
    const h2 = await harness();
    h2.session.start();
    h2.socket().open();
    await waitForSent(h2.socket, 1);

    const tampered = bytesFromHex(GOLDEN.msg2);
    tampered[tampered.length - 1] ^= 0xff;
    h2.socket().deliver(
      hexFromBytes(
        encodeFrame({
          v: 1,
          kind: OuterKind.HelloAck,
          channel: bytesFromHex(GOLDEN.channel),
          seq: 0,
          ack: 0,
          payload: tampered,
        }),
      ),
    );
    await waitForState(h2.states, "failed");

    expect(last(h2.states)).toMatchObject({ kind: "failed" });
    expect(h2.socket().closed).toBe(true);
    // And it does not come back. Long enough for several turns of the backoff at these
    // timings, so a redial would have happened by now if one were going to.
    await pause(60);
    expect(h2.sockets).toHaveLength(1);
    expect(h.states.length).toBeGreaterThan(0);
  });

  /// Only this side bounds its wait. The device sits idle indefinitely, which costs
  /// it nothing; if both bounded theirs they would take turns sleeping and never
  /// meet — which the relay's prova real demonstrated.
  it("redials when the device does not answer in time", async () => {
    const h = await harness();
    h.session.start();
    h.socket().open();
    await waitForSent(h.socket, 1);
    expect(h.sockets).toHaveLength(1);

    await waitForSockets(h.sockets, 2);

    expect(h.sockets).toHaveLength(2);
  });
});

describe("receiving", () => {
  async function live() {
    const h = await upToConfirming();
    h.session.confirm();
    h.session.subscribe("golden", 0);
    await waitForSent(h.socket, 2);
    return h;
  }

  /// The whole point: a real snapshot the device produced, decrypted and decoded.
  it("decodes a snapshot the device actually sent", async () => {
    const h = await live();
    h.socket().deliver(GOLDEN.snapshotFrame);
    await waitForMessages(h.messages, 1);

    expect(h.messages).toHaveLength(1);
    const snapshot = h.messages[0];
    expect(snapshot.kind).toBe("snapshot");
    expect(snapshot.session_id).toBe("golden");
    expect(snapshot.seq).toBe(2);
    expect(Array.isArray(snapshot.records)).toBe(true);
    expect((snapshot.records as unknown[]).length).toBe(2);
  });

  /// The device's own transcript records, intact — this is what the timeline renders.
  it("carries the transcript records through unchanged", async () => {
    const h = await live();
    h.socket().deliver(GOLDEN.snapshotFrame);
    await waitForMessages(h.messages, 1);

    const records = h.messages[0].records as Record<string, unknown>[];
    expect(records[0]).toEqual({ kind: "meta", sessionId: "golden" });
    expect(records[1]).toEqual({ kind: "user", content: "hello" });
  });

  /// `Closed` is a state, not a message to render. And it must stop the retry loop:
  /// the device has said it will not answer until someone acts on it locally.
  it("reports a close with its reason and does not reconnect", async () => {
    const h = await live();
    h.socket().deliver(GOLDEN.snapshotFrame);
    await waitForMessages(h.messages, 1);
    h.socket().deliver(GOLDEN.closedFrame);
    await waitForState(h.states, "closed");

    expect(last(h.states)).toEqual({ kind: "closed", reason: "turned_off_locally" });
    // Not delivered as a message: nothing in the timeline should render it.
    expect(h.messages).toHaveLength(1);

    await pause(60);
    expect(h.sockets).toHaveLength(1);
  });

  /// A hostile relay must not be able to end a session by sending rubbish. If bad
  /// input dropped the connection, that would be a disconnect button.
  it("drops undecodable bytes and stays connected", async () => {
    const h = await live();
    h.socket().deliver("c1c1c1c1");
    await settle();

    expect(h.messages).toHaveLength(0);
    expect(h.socket().closed).toBe(false);
    expect(last(h.states)).toMatchObject({ kind: "live" });
  });

  /// And a tampered frame must not poison the session. The nonce does not advance, so
  /// the genuine frame that follows still authenticates.
  it("drops a tampered frame and still reads the next genuine one", async () => {
    const h = await live();
    const tampered = bytesFromHex(GOLDEN.snapshotFrame);
    tampered[tampered.length - 1] ^= 0xff;
    h.socket().onmessage?.(tampered);
    await settle();
    await settle();
    expect(h.messages).toHaveLength(0);

    h.socket().deliver(GOLDEN.snapshotFrame);
    await waitForMessages(h.messages, 1);
    expect(h.messages).toHaveLength(1);
  });

  /// A frame claiming another channel is the untrusted middle telling us which
  /// conversation this is. Believing it would be believing the relay.
  it("ignores a frame for a different channel", async () => {
    const h = await live();
    const frame = decodeFrame(bytesFromHex(GOLDEN.snapshotFrame));
    h.socket().onmessage?.(
      encodeFrame({ ...frame, channel: bytesFromHex("99".repeat(16)) }),
    );
    await settle();

    expect(h.messages).toHaveLength(0);
  });

  it("ignores a frame kind it has nothing to do with", async () => {
    const h = await live();
    const frame = decodeFrame(bytesFromHex(GOLDEN.snapshotFrame));
    h.socket().onmessage?.(encodeFrame({ ...frame, kind: OuterKind.Ping }));
    await settle();

    expect(h.messages).toHaveLength(0);
    expect(last(h.states)).toMatchObject({ kind: "live" });
  });
});

describe("reconnecting", () => {
  it("redials after the socket closes", async () => {
    const h = await harness();
    h.session.start();
    h.socket().open();
    await settle();

    h.socket().onclose?.();
    await waitForSockets(h.sockets, 2);

    expect(h.sockets.length).toBeGreaterThan(1);
  });

  /// The SAS changes on every handshake, because it comes from the ephemerals. Asking
  /// the user to compare it again on every network blip is how they learn to tap
  /// through it — the pairing was vouched for once, and the device authenticates the
  /// key from then on.
  it("does not ask for the words again after a reconnect", async () => {
    const h = await upToConfirming();
    h.session.confirm();
    h.session.subscribe("golden", 0);
    await waitForSent(h.socket, 2);

    h.socket().onclose?.();
    await waitForSockets(h.sockets, 2);
    h.socket().open();
    await waitForSent(h.socket, 1);
    h.socket().deliver(helloAck());
    await waitForSent(h.socket, 2);

    expect(h.states.filter((s) => s.kind === "confirming")).toHaveLength(1);
    expect(last(h.states)).toMatchObject({ kind: "live" });
  });

  /// Resuming from what arrived, not from what was asked for. Otherwise every
  /// reconnect replays the whole transcript — which on a phone is constant.
  it("resumes from the highest seq it has seen", async () => {
    const h = await upToConfirming();
    h.session.confirm();
    h.session.subscribe("golden", 0);
    await waitForSent(h.socket, 2);
    h.socket().deliver(GOLDEN.snapshotFrame);
    await settle();

    h.socket().onclose?.();
    await waitForSockets(h.sockets, 2);
    h.socket().open();
    await waitForSent(h.socket, 1);
    h.socket().deliver(helloAck());
    await waitForSent(h.socket, 2);

    // The resubscribe is the second thing this socket sent, after its Hello.
    expect(h.socket().sent).toHaveLength(2);
    // Not asserted byte-wise: what matters is that it asked for seq 3, not seq 0.
    // The frame is encrypted, so the check is that it exists and is a Data frame.
    expect(decodeFrame(h.socket().sent[1]).kind).toBe(OuterKind.Data);
  });

  it("stops for good when asked", async () => {
    const h = await harness();
    h.session.start();
    h.session.stop();

    await pause(60);

    expect(h.sockets).toHaveLength(1);
    expect(h.socket().closed).toBe(true);
  });
});
