/// Reading a pairing code out of the URL.
///
/// # This is the untrusted edge
///
/// Everything here arrives from a URL somebody else may have written. A doctored
/// code is the one attack Noise IK cannot detect on its own — IK proves the device
/// holds the key the browser dialled with, so substituting the *key* fails, but a
/// code that named the wrong key from the start succeeds. The short authentication
/// string is what closes that, and it only works if the user compares it.
///
/// So this module's job is narrow: refuse anything malformed, and refuse anything
/// that would make the connection go somewhere it should not. It does not decide
/// whether the code is genuine — nothing here can.
///
/// # Why the fragment
///
/// The device puts the channel and its key after `#`, and a fragment is never sent
/// to a server. Reading them from `location.hash` is not a style choice: in a query
/// string they would be in the request line of every page load, handing this origin
/// — and its logs, and anything in front of it — the ability to attach to the
/// channel and the key a substitution attack needs.
///
/// Mirrors `src-tauri/src/remote/code.rs`, which writes it.

export interface PairingCode {
  /// 32 hex characters. Which channel on the relay.
  channel: string;
  /// The relay's routing capability for that channel.
  ///
  /// Distinct from the channel id: the id says *which* channel, this says the holder
  /// is allowed on it. The relay checks it before spending bandwidth on a stranger,
  /// and refuses anything under 16 characters — so a short one is refused here too,
  /// where the message can say why.
  token: string;
  /// 64 hex characters. The device's static public key — public by design.
  deviceKey: string;
  /// Where to dial. `wss:` only; see `parseRelayUrl`.
  relayUrl: string;
  /// Unix millis after which the *code* is stale. The pairing it creates is not.
  expiresAt: number | null;
}

export type PairingCodeError =
  | { kind: "absent" }
  | { kind: "malformed"; field: string; why: string };

export type PairingCodeResult =
  | { ok: true; code: PairingCode }
  | { ok: false; error: PairingCodeError };

const HEX = /^[0-9a-f]+$/;

/// Matches `MIN_TOKEN_LEN` in the relay's `auth.rs`. Refused here as well as there,
/// so a short token fails with an explanation instead of as an opaque 403 from a
/// server the user has no view of.
export const MIN_TOKEN_LENGTH = 16;

/// Parse a fragment into a pairing code.
///
/// Takes the fragment rather than reading `location` itself, so this is testable
/// without a browser and so the caller decides what counts as the current URL.
export function parsePairingCode(fragment: string): PairingCodeResult {
  const raw = fragment.startsWith("#") ? fragment.slice(1) : fragment;
  if (!raw.trim()) return { ok: false, error: { kind: "absent" } };

  const params = new URLSearchParams(raw);
  const channel = normaliseHex(params.get("c"));
  const token = (params.get("t") ?? "").trim();
  const deviceKey = normaliseHex(params.get("k"));

  if (channel.length !== 32 || !HEX.test(channel)) {
    return bad("channel", "a channel id is 32 hex characters");
  }
  if (token.length < MIN_TOKEN_LENGTH) {
    return bad("token", `a channel token is at least ${MIN_TOKEN_LENGTH} characters`);
  }
  if (deviceKey.length !== 64 || !HEX.test(deviceKey)) {
    return bad("deviceKey", "a device key is 64 hex characters");
  }

  const relay = parseRelayUrl(params.get("r"));
  if (!relay.ok) return bad("relayUrl", relay.why);

  // A missing or unreadable expiry is treated as "already stale" rather than
  // "never expires": a code with no deadline is not something the device emits, so
  // it is either corrupted or crafted, and the safe reading of both is to make the
  // user fetch a fresh one.
  const expiresAt = parseExpiry(params.get("e"));

  return { ok: true, code: { channel, token, deviceKey, relayUrl: relay.url, expiresAt } };
}

/// Has this code gone stale?
///
/// Separate from parsing, because a stale code is not malformed — the user needs a
/// different message ("get a new code") from the one a corrupted URL deserves.
export function isStale(code: PairingCode, now = Date.now()): boolean {
  return code.expiresAt === null || now >= code.expiresAt;
}

/// The relay URL, refusing anything that is not an encrypted WebSocket.
///
/// `wss:` only, and this is the check that matters most in this file. The URL comes
/// from the same untrusted fragment as everything else, so without it a doctored
/// code could point the browser at `ws://` — where the frames are still ciphertext,
/// but an observer on the path learns the channel token and can then attach to the
/// channel itself. `http(s):` is refused too: it would silently become a relative
/// fetch rather than a socket.
function parseRelayUrl(value: string | null): { ok: true; url: string } | { ok: false; why: string } {
  if (!value) return { ok: false, why: "no relay URL in the code" };

  let parsed: URL;
  try {
    parsed = new URL(value);
  } catch {
    return { ok: false, why: "the relay URL is not a URL" };
  }

  if (parsed.protocol !== "wss:") {
    return {
      ok: false,
      why: `the relay URL must be wss://, not ${parsed.protocol.replace(":", "") || "(none)"}`,
    };
  }
  if (!parsed.hostname) return { ok: false, why: "the relay URL has no host" };

  // Rebuilt from the parsed URL rather than passed through: it drops credentials,
  // normalises the host, and means what reaches `new WebSocket` is what was
  // validated rather than the original string.
  parsed.username = "";
  parsed.password = "";
  parsed.hash = "";
  return { ok: true, url: parsed.toString() };
}

function parseExpiry(value: string | null): number | null {
  if (!value) return null;
  const millis = Number(value);
  if (!Number.isFinite(millis) || millis <= 0) return null;
  return millis;
}

/// Hex, case-folded. Anything not hex is left in place so the length and character
/// checks reject it rather than a silent strip making a short code look valid.
function normaliseHex(value: string | null): string {
  return (value ?? "").trim().toLowerCase();
}

function bad(field: string, why: string): PairingCodeResult {
  return { ok: false, error: { kind: "malformed", field, why } };
}

/// Remove the code from the address bar, once it has been read.
///
/// A pairing code is single-use and short-lived, but the URL outlives both: it
/// lands in history, in a screenshot, in whatever the user shares when they ask for
/// help. Replacing rather than pushing so Back does not restore it.
export function forgetPairingCode(win: { history: History; location: Location } = window): void {
  try {
    win.history.replaceState(null, "", win.location.pathname + win.location.search);
  } catch {
    // A sandboxed context can refuse replaceState. Not worth failing over — the
    // code is stale in two minutes either way.
  }
}
