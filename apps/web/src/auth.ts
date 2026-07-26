/// The typed-code path: proving which account this browser belongs to.
///
/// # Why this exists at all, and why it is not the main path
///
/// The QR path needs none of it. The device shows a URL carrying the channel, its
/// token and its public key in the fragment; the browser reads it and runs Noise IK
/// straight through the relay. No account, no server, nothing of `claudin.io`
/// involved — which §1.1 requires, because that origin must never be a hard
/// dependency of remote access.
///
/// A *typed* code cannot carry 128 bits of channel plus 256 bits of key plus a relay
/// token. So it has to be a short handle that something resolves, and the something
/// is the account server. It refuses to resolve a code for anyone but the account
/// that minted it — which is the check that makes a ten-character code acceptable
/// rather than a bearer token for a developer's machine. That check is the only
/// reason any of this is here.
///
/// # The shape, and what it deliberately does not do
///
/// The browser generates a verifier, keeps it, and sends only its hash. The account
/// server hands back an authorisation code **in a fragment**, which is never sent to
/// a server, never logged and never leaves as a referrer. The code is then redeemed
/// for a token by presenting the verifier.
///
/// - **The verifier lives in `sessionStorage`, not `localStorage`.** It has to survive
///   one navigation away and back, and it must not survive the tab. There is no
///   reason for a verifier to be on disk tomorrow.
/// - **The token lives in a variable and nowhere else.** Not `sessionStorage`: a
///   credential in storage outlives the moment anyone is looking at the page and is
///   readable by any script on this origin after an XSS — and §10 names XSS here as
///   the high risk. Holding it in memory means it dies with the page. The cost is
///   that a reload re-authorises, which for a signed-in user is a silent redirect.
/// - **Nothing here can pair anything.** The token resolves a code into a channel;
///   the device still gates on a human comparing three words. If everything in this
///   file were compromised, the SAS is what stands between that and a session.

import { validatePairing, type PairingCode, type PairingCodeResult } from "./pairing";

/// Where the account lives.
///
/// Configurable at build time because §1.1 cuts both ways: someone running their own
/// peer has to be able to point it at their own account server. Note that changing it
/// also means changing `connect-src` in `index.html` and in the deployed header —
/// stated in the README, because a CSP that still named claudin.io would refuse the
/// fetch and the failure would look like the server being down.
export const ACCOUNT_ORIGIN = (
  import.meta.env?.VITE_ACCOUNT_ORIGIN ?? "https://claudin.io"
).replace(/\/+$/, "");

const VERIFIER_KEY = "claudinio.auth.verifier";
const STATE_KEY = "claudinio.auth.state";

/// How long a claimed pairing may sit before it is dialled.
///
/// A claimed code has no code left to expire — the server consumed it — so the
/// deadline `PairingCode` carries has to come from somewhere. It is a local one, and
/// it means what the field means everywhere else: do not dial after this. Thirty
/// seconds, because what actually limits the window is the device's own pairing
/// window, which started when it minted the code and which this browser cannot see.
export const DIAL_WINDOW_MS = 30_000;

/// Just enough of the browser to fake.
export interface AuthHost {
  crypto: Pick<Crypto, "getRandomValues"> & { subtle: Pick<SubtleCrypto, "digest"> };
  sessionStorage: Pick<Storage, "getItem" | "setItem" | "removeItem">;
  location: { assign(url: string): void };
  fetch: typeof fetch;
}

const browser = (): AuthHost => window as unknown as AuthHost;

// ── leaving, and coming back ─────────────────────────────────────────────────

/// Send the user to the account server to authorise this browser.
///
/// Navigates away. Everything needed to finish is in `sessionStorage` before the
/// navigation happens, and the verifier is written *first*: a navigation that raced a
/// storage write would come back with a code that can never be redeemed, and the user
/// would be told to sign in again by something that had nothing to do with signing in.
export async function startAuthorization(host: AuthHost = browser()): Promise<void> {
  const verifier = randomToken(host);
  const state = randomToken(host);
  const challenge = await sha256Hex(host, verifier);

  host.sessionStorage.setItem(VERIFIER_KEY, verifier);
  host.sessionStorage.setItem(STATE_KEY, state);

  const url = new URL(`${ACCOUNT_ORIGIN}/remote/authorize`);
  url.searchParams.set("challenge", challenge);
  url.searchParams.set("state", state);
  host.location.assign(url.toString());
}

export type Authorization =
  | { kind: "absent" }
  | { kind: "returned"; code: string; state: string };

/// Read an authorisation out of the fragment we came back with.
///
/// Takes the fragment rather than reading `location`, for the same reason
/// `parsePairingCode` does: this has to be testable without a browser, and the caller
/// decides what counts as the current URL.
export function parseAuthorization(fragment: string): Authorization {
  const raw = fragment.startsWith("#") ? fragment.slice(1) : fragment;
  if (!raw.trim()) return { kind: "absent" };

  const params = new URLSearchParams(raw);
  const code = (params.get("auth") ?? "").trim();
  if (!code) return { kind: "absent" };
  return { kind: "returned", code, state: (params.get("state") ?? "").trim() };
}

export type Redemption =
  | { ok: true; token: string; login: string }
  | { ok: false; why: string; retry: boolean };

/// Turn the code we came back with into a token.
///
/// The state is compared before anything is sent. It is what says this redirect
/// answers *our* request rather than one someone else started in our browser — and a
/// mismatch is refused rather than attempted, because sending a verifier in answer to
/// a code we did not ask for is exactly the shape of being used as an oracle.
export async function redeemAuthorization(
  returned: { code: string; state: string },
  host: AuthHost = browser(),
): Promise<Redemption> {
  const verifier = host.sessionStorage.getItem(VERIFIER_KEY);
  const state = host.sessionStorage.getItem(STATE_KEY);
  // Consumed either way. A verifier that survived a failure would be reusable against
  // whatever code arrives next.
  forgetAuthorization(host);

  if (!verifier || !state) {
    return {
      ok: false,
      why: "this browser did not ask for that sign-in",
      retry: true,
    };
  }
  if (state !== returned.state) {
    return { ok: false, why: "that sign-in answers a different request", retry: true };
  }

  let response: Response;
  try {
    response = await host.fetch(`${ACCOUNT_ORIGIN}/api/remote/token`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ code: returned.code, verifier }),
      // No cookies, ever. The route does not read one, and asking for credentials on
      // a cross-origin request is how the dashboard's session ends up in play.
      credentials: "omit",
    });
  } catch {
    return { ok: false, why: "could not reach your account", retry: true };
  }

  if (!response.ok) {
    return { ok: false, why: await reason(response, "that sign-in did not work"), retry: true };
  }

  const body = (await response.json().catch(() => null)) as {
    token?: unknown;
    user?: { login?: unknown };
  } | null;
  const token = typeof body?.token === "string" ? body.token : "";
  if (!token) return { ok: false, why: "your account did not return a token", retry: true };

  const login = typeof body?.user?.login === "string" ? body.user.login : "";
  return { ok: true, token, login };
}

/// Drop the verifier and the state.
export function forgetAuthorization(host: AuthHost = browser()): void {
  try {
    host.sessionStorage.removeItem(VERIFIER_KEY);
    host.sessionStorage.removeItem(STATE_KEY);
  } catch {
    // Storage can be blocked outright. Nothing here is worth failing over: without
    // it the redemption fails closed, which is the direction it should fail.
  }
}

// ── using the token ──────────────────────────────────────────────────────────

export interface Device {
  deviceKey: string;
  label: string;
  lastSeen: number | null;
}

/// The account's machines, so the user can be told which one they are pairing with.
export async function listDevices(token: string, host: AuthHost = browser()): Promise<Device[]> {
  const response = await host.fetch(`${ACCOUNT_ORIGIN}/api/remote/devices`, {
    headers: { Authorization: `Bearer ${token}` },
    credentials: "omit",
  });
  if (!response.ok) return [];

  const body = (await response.json().catch(() => null)) as { devices?: unknown } | null;
  const devices = Array.isArray(body?.devices) ? body.devices : [];
  return devices
    .map((raw) => raw as Record<string, unknown>)
    .filter((raw) => typeof raw?.device_key === "string")
    .map((raw) => ({
      deviceKey: raw.device_key as string,
      // A blank label counts as absent. The account server defaults it on
      // registration, so this is for a row that was written before it did — and a
      // nameless row in a list of machines is a row nobody can choose between.
      label: (typeof raw.label === "string" ? raw.label.trim() : "") || "Claudinio Code",
      lastSeen: typeof raw.last_seen === "number" ? raw.last_seen * 1000 : null,
    }));
}

export type ClaimResult =
  | { ok: true; code: PairingCode }
  | { ok: false; why: string; retry: boolean };

/// Resolve a typed code into a pairing to dial.
///
/// The response goes through `validatePairing`, the same check the URL fragment gets.
/// Treating a body from our own account server as already-checked is how the account
/// server would end up able to point this browser at a `ws://` relay — where the
/// frames are still ciphertext but an observer learns the channel token.
export async function claimTypedCode(
  typed: string,
  token: string,
  host: AuthHost = browser(),
  now = Date.now(),
): Promise<ClaimResult> {
  let response: Response;
  try {
    response = await host.fetch(`${ACCOUNT_ORIGIN}/api/remote/pairings/claim`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
      body: JSON.stringify({ code: typed }),
      credentials: "omit",
    });
  } catch {
    return { ok: false, why: "could not reach your account", retry: true };
  }

  if (response.status === 401) {
    // The token lasts fifteen minutes and a code lasts two. Being told to sign in
    // again is the actual fix here, and saying "invalid code" would send the user
    // hunting for a typo that is not there.
    return { ok: false, why: "this browser is no longer signed in", retry: true };
  }
  if (!response.ok) {
    return {
      ok: false,
      why: await reason(response, "that code is not valid, or has expired"),
      retry: true,
    };
  }

  const body = (await response.json().catch(() => null)) as Record<string, unknown> | null;
  const checked: PairingCodeResult = validatePairing({
    channel: asString(body?.channel),
    token: asString(body?.token),
    deviceKey: asString(body?.device_key),
    relayUrl: asString(body?.relay_url),
    expiresAt: now + DIAL_WINDOW_MS,
  });
  if (!checked.ok) {
    const why =
      checked.error.kind === "absent"
        ? "your account returned nothing to connect to"
        : `your account returned a pairing this browser will not use: ${checked.error.why}`;
    // Not retryable: typing the code again produces the same answer. Something is
    // wrong between the device and the account server, not with what the user did.
    return { ok: false, why, retry: false };
  }
  return { ok: true, code: checked.code };
}

/// Tell the account server the token is finished with.
///
/// So that closing the app means something on that side too. It does not end the
/// remote session — that lives on the device and ends there or when this peer
/// disconnects — and it is best-effort: a browser being closed is the ordinary case.
export function releaseToken(token: string, host: AuthHost = browser()): void {
  if (!token) return;
  void host
    .fetch(`${ACCOUNT_ORIGIN}/api/remote/token`, {
      method: "DELETE",
      headers: { Authorization: `Bearer ${token}` },
      credentials: "omit",
      keepalive: true,
    })
    .catch(() => {});
}

// ── the small parts ──────────────────────────────────────────────────────────

/// 256 bits, base64url. The verifier and the state are both this: neither has any
/// structure worth having, and both need to survive a URL.
function randomToken(host: AuthHost): string {
  const bytes = host.crypto.getRandomValues(new Uint8Array(32));
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

async function sha256Hex(host: AuthHost, value: string): Promise<string> {
  const digest = await host.crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/// The server's own wording when it has one, ours when it does not.
///
/// The account server writes better messages than a status code does — "that pairing
/// code belongs to another account" is a different problem from "expired" — and it is
/// our own server. But only a string, and only from a JSON `error` field: rendering
/// arbitrary response text would put whatever answered on this page.
async function reason(response: Response, fallback: string): Promise<string> {
  try {
    const body = (await response.json()) as { error?: unknown };
    return typeof body?.error === "string" && body.error.length <= 200 ? body.error : fallback;
  } catch {
    return fallback;
  }
}

function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}
