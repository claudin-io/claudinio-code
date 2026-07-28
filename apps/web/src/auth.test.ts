import { describe, it, expect, vi } from "vitest";
import {
  ACCOUNT_ORIGIN,
  claimTypedCode,
  listDevices,
  parseAuthorization,
  redeemAuthorization,
  releaseToken,
  startAuthorization,
  type AuthHost,
} from "./auth";

const CHANNEL = "ab".repeat(16);
const DEVICE_KEY = "cd".repeat(32);
const TOKEN = "Zm9vYmFyLXRva2VuLTEyMw";
const RELAY = "wss://relay.claudin.io/attach";

/// A browser, as much of one as `auth.ts` reads.
function fakeHost(
  options: { responses?: (Response | Error)[]; stored?: Record<string, string> } = {},
) {
  const stored: Record<string, string> = { ...options.stored };
  const queued = [...(options.responses ?? [])];
  const assigned: string[] = [];
  const calls: { url: string; init?: RequestInit }[] = [];

  const fetch = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ url: String(url), init });
    const next = queued.shift();
    if (next instanceof Error) throw next;
    return next ?? new Response("{}", { status: 200 });
  });

  // Deterministic but *different every call*. The first version of this filled every
  // buffer with 0..31, which made the verifier and the state the same string — and
  // that quietly turned "the verifier never appears in the URL" into a test that could
  // only fail. A fake that hands out one secret twice is not a fake of a CSPRNG.
  let draws = 0;

  const host: AuthHost = {
    crypto: {
      getRandomValues: <T extends ArrayBufferView | null>(array: T): T => {
        if (!array) return array;
        const bytes = new Uint8Array(array.buffer, array.byteOffset, array.byteLength);
        draws += 1;
        bytes.forEach((_, index) => (bytes[index] = (index + draws * 31) & 0xff));
        return array;
      },
      subtle: { digest: globalThis.crypto.subtle.digest.bind(globalThis.crypto.subtle) },
    },
    sessionStorage: {
      getItem: (key: string) => stored[key] ?? null,
      setItem: (key: string, value: string) => {
        stored[key] = value;
      },
      removeItem: (key: string) => {
        delete stored[key];
      },
    },
    location: { assign: (url: string) => assigned.push(url) },
    fetch: fetch as unknown as typeof globalThis.fetch,
  };

  return { host, stored, assigned, calls, fetch };
}

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });

// ── leaving ──────────────────────────────────────────────────────────────────

describe("startAuthorization", () => {
  it("goes to the account server with a challenge and a state", async () => {
    const { host, assigned } = fakeHost();

    await startAuthorization(host);

    const url = new URL(assigned[0]);
    expect(url.origin + url.pathname).toBe(`${ACCOUNT_ORIGIN}/remote/authorize`);
    expect(url.searchParams.get("challenge")).toMatch(/^[0-9a-f]{64}$/);
    expect(url.searchParams.get("state")).toBeTruthy();
  });

  /// Only the hash leaves. The verifier is what makes a code that leaked through
  /// history or a screenshot useless on its own, and it is worth nothing if it is in
  /// the URL beside the code.
  it("never puts the verifier in the URL", async () => {
    const { host, assigned, stored } = fakeHost();

    await startAuthorization(host);

    const verifier = stored["claudinio.auth.verifier"];
    expect(verifier).toBeTruthy();
    expect(assigned[0]).not.toContain(verifier);
  });

  /// The property the test above depends on, asserted so it cannot quietly stop being
  /// true. If the two were ever drawn from the same value, the state — which travels
  /// in the URL — *would be* the verifier, and the verifier is the only thing standing
  /// between a leaked code and a token.
  it("does not use the same secret for the verifier and the state", async () => {
    const { host } = fakeHost();

    await startAuthorization(host);

    expect(host.sessionStorage.getItem("claudinio.auth.verifier")).not.toBe(
      host.sessionStorage.getItem("claudinio.auth.state"),
    );
  });

  /// A navigation that raced the write would come back with a code that can never be
  /// redeemed, and the user would be told to sign in again by something that had
  /// nothing to do with signing in.
  it("has stored everything it needs before it navigates", async () => {
    const { host, stored, assigned } = fakeHost();
    const order: string[] = [];
    host.sessionStorage.setItem = (key: string, value: string) => {
      order.push(`store ${key}`);
      stored[key] = value;
    };
    host.location.assign = () => {
      order.push("navigate");
      assigned.push("x");
    };

    await startAuthorization(host);

    expect(order[order.length - 1]).toBe("navigate");
    expect(order).toContain("store claudinio.auth.verifier");
    expect(order).toContain("store claudinio.auth.state");
  });
});

// ── coming back ──────────────────────────────────────────────────────────────

describe("parseAuthorization", () => {
  it("reads the code and the state out of the fragment", () => {
    expect(parseAuthorization("#auth=abc123&state=s1")).toEqual({
      kind: "returned",
      code: "abc123",
      state: "s1",
    });
  });

  it("says absent for an empty fragment", () => {
    expect(parseAuthorization("")).toEqual({ kind: "absent" });
    expect(parseAuthorization("#")).toEqual({ kind: "absent" });
  });

  /// A pairing-code fragment is not an authorisation. Both arrive at the same URL and
  /// confusing them would send the QR path through a sign-in it does not need.
  it("says absent for a pairing code", () => {
    expect(parseAuthorization(`#c=${CHANNEL}&t=${TOKEN}&k=${DEVICE_KEY}`)).toEqual({
      kind: "absent",
    });
  });
});

describe("redeemAuthorization", () => {
  async function returning(body: unknown, status = 200) {
    const { host } = fakeHost();
    await startAuthorization(host);
    const state = host.sessionStorage.getItem("claudinio.auth.state")!;
    const fetch = vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) =>
      json(body, status),
    );
    return { host: { ...host, fetch: fetch as unknown as typeof globalThis.fetch }, state, fetch };
  }

  it("exchanges the code for a token", async () => {
    const { host, state } = await returning({ token: "tok", user: { login: "alice" } });

    const result = await redeemAuthorization({ code: "c", state }, host);

    expect(result).toEqual({ ok: true, token: "tok", login: "alice" });
  });

  it("sends the verifier and not the challenge", async () => {
    const { host, state, fetch } = await returning({ token: "tok", user: { login: "a" } });
    const verifier = host.sessionStorage.getItem("claudinio.auth.verifier");

    await redeemAuthorization({ code: "c", state }, host);

    const body = JSON.parse((fetch.mock.calls[0][1] as RequestInit).body as string);
    expect(body).toEqual({ code: "c", verifier });
  });

  /// Cookies would put the dashboard's session in play on a cross-origin request. The
  /// route does not read one; asking for it anyway is how that stops being true.
  it("never sends cookies", async () => {
    const { host, state, fetch } = await returning({ token: "tok", user: { login: "a" } });

    await redeemAuthorization({ code: "c", state }, host);

    expect((fetch.mock.calls[0][1] as RequestInit).credentials).toBe("omit");
  });

  /// The state is what says this redirect answers *our* request. Sending a verifier in
  /// answer to a code we did not ask for is the shape of being used as an oracle.
  it("refuses a state it did not issue", async () => {
    const { host, fetch } = await returning({ token: "tok", user: { login: "a" } });

    const result = await redeemAuthorization({ code: "c", state: "someone-elses" }, host);

    expect(result.ok).toBe(false);
    expect(fetch).not.toHaveBeenCalled();
  });

  it("refuses a code that arrived with no request behind it", async () => {
    const { host, fetch } = fakeHost();

    const result = await redeemAuthorization({ code: "c", state: "s" }, host);

    expect(result).toMatchObject({ ok: false, retry: true });
    expect(fetch).not.toHaveBeenCalled();
  });

  /// A verifier that survived a failure would be reusable against whatever code
  /// arrives next — including one an attacker caused to arrive.
  it("consumes the verifier whether it worked or not", async () => {
    const { host, state } = await returning({ error: "no" }, 401);

    await redeemAuthorization({ code: "c", state }, host);

    expect(host.sessionStorage.getItem("claudinio.auth.verifier")).toBeNull();
    expect(host.sessionStorage.getItem("claudinio.auth.state")).toBeNull();
  });

  it("reports the account server's own wording", async () => {
    const { host, state } = await returning({ error: "that authorisation has expired" }, 401);

    const result = await redeemAuthorization({ code: "c", state }, host);

    expect(result).toMatchObject({ ok: false, why: "that authorisation has expired" });
  });

  it("survives a body that is not the shape it expected", async () => {
    const { host, state } = await returning({ user: { login: "alice" } });

    const result = await redeemAuthorization({ code: "c", state }, host);

    expect(result).toMatchObject({ ok: false });
  });

  it("survives the account server being unreachable", async () => {
    const { host } = fakeHost();
    await startAuthorization(host);
    const state = host.sessionStorage.getItem("claudinio.auth.state")!;
    const failing = {
      ...host,
      fetch: (async () => {
        throw new TypeError("Failed to fetch");
      }) as unknown as typeof globalThis.fetch,
    };

    const result = await redeemAuthorization({ code: "c", state }, failing);

    expect(result).toMatchObject({ ok: false, retry: true });
  });
});

// ── using the token ──────────────────────────────────────────────────────────

describe("listDevices", () => {
  it("reads the account's devices", async () => {
    const { host } = fakeHost({
      responses: [
        json({
          devices: [
            { device_key: DEVICE_KEY, label: "MacBook", last_seen: 1700000000 },
            { device_key: "ff".repeat(32), label: "", created_at: 1 },
          ],
        }),
      ],
    });

    const devices = await listDevices("tok", host);

    expect(devices).toEqual([
      { deviceKey: DEVICE_KEY, label: "MacBook", lastSeen: 1700000000000 },
      { deviceKey: "ff".repeat(32), label: "Claudinio Code", lastSeen: null },
    ]);
  });

  it("sends the token as a bearer and no cookie", async () => {
    const { host, calls } = fakeHost({ responses: [json({ devices: [] })] });

    await listDevices("tok", host);

    const init = calls[0].init as RequestInit;
    expect((init.headers as Record<string, string>).Authorization).toBe("Bearer tok");
    expect(init.credentials).toBe("omit");
  });

  /// A device list is a convenience. Failing it must not be the thing that stops
  /// someone pairing, because the pairing does not depend on it.
  it("is empty rather than throwing when the call fails", async () => {
    const { host } = fakeHost({ responses: [json({ error: "no" }, 401)] });

    await expect(listDevices("tok", host)).resolves.toEqual([]);
  });

  it("ignores a row that is not a device", async () => {
    const { host } = fakeHost({ responses: [json({ devices: ["nope", null, 7] })] });

    await expect(listDevices("tok", host)).resolves.toEqual([]);
  });
});

describe("claimTypedCode", () => {
  const claimed = {
    device_key: DEVICE_KEY,
    channel: CHANNEL,
    token: TOKEN,
    relay_url: RELAY,
  };

  it("turns a typed code into something to dial", async () => {
    const { host } = fakeHost({ responses: [json(claimed)] });

    const result = await claimTypedCode("A1B2C-D3E4F", "tok", host, 1_000);

    expect(result).toEqual({
      ok: true,
      code: {
        channel: CHANNEL,
        token: TOKEN,
        deviceKey: DEVICE_KEY,
        relayUrl: `${RELAY}`,
        expiresAt: 31_000,
      },
    });
  });

  /// The check the whole file exists to not skip. A body from our own account server
  /// goes through the same validator as a URL somebody else may have written —
  /// otherwise that server could point this browser at a `ws://` relay, where the
  /// frames are still ciphertext but an observer learns the channel token.
  it("refuses a relay that is not an encrypted socket", async () => {
    const { host } = fakeHost({
      responses: [json({ ...claimed, relay_url: "ws://relay.claudin.io/attach" })],
    });

    const result = await claimTypedCode("code", "tok", host);

    expect(result).toMatchObject({ ok: false, retry: false });
    expect((result as { why: string }).why).toContain("wss://");
  });

  it("refuses a pairing with no channel token", async () => {
    const { host } = fakeHost({ responses: [json({ ...claimed, token: "short" })] });

    const result = await claimTypedCode("code", "tok", host);

    expect(result).toMatchObject({ ok: false, retry: false });
  });

  it("refuses a device key of the wrong shape", async () => {
    const { host } = fakeHost({ responses: [json({ ...claimed, device_key: "nope" })] });

    expect(await claimTypedCode("code", "tok", host)).toMatchObject({ ok: false });
  });

  /// The token lasts fifteen minutes and a code lasts two. "Invalid code" would send
  /// the user hunting for a typo that is not there.
  it("says the sign-in lapsed rather than blaming the code", async () => {
    const { host } = fakeHost({ responses: [json({ error: "not authorised" }, 401)] });

    const result = await claimTypedCode("code", "tok", host);

    expect(result).toMatchObject({ ok: false, why: "this browser is no longer signed in" });
  });

  it("passes on the account server's wording for a bad code", async () => {
    const { host } = fakeHost({
      responses: [json({ error: "that pairing code is not valid, or has expired" }, 404)],
    });

    const result = await claimTypedCode("code", "tok", host);

    expect(result).toMatchObject({
      ok: false,
      why: "that pairing code is not valid, or has expired",
      retry: true,
    });
  });

  /// Only a short string, and only from a JSON `error` field: rendering arbitrary
  /// response text would put whatever answered on this page.
  it("does not render an unbounded body as an explanation", async () => {
    const { host } = fakeHost({ responses: [json({ error: "x".repeat(500) }, 404)] });

    const result = await claimTypedCode("code", "tok", host);

    expect((result as { why: string }).why).toBe("that code is not valid, or has expired");
  });

  it("survives the account server being unreachable", async () => {
    const { host } = fakeHost({ responses: [new TypeError("Failed to fetch")] });

    expect(await claimTypedCode("code", "tok", host)).toMatchObject({ ok: false, retry: true });
  });
});

describe("releaseToken", () => {
  it("tells the account server the token is finished with", () => {
    const { host, calls } = fakeHost();

    releaseToken("tok", host);

    expect(calls[0].url).toBe(`${ACCOUNT_ORIGIN}/api/remote/token`);
    expect((calls[0].init as RequestInit).method).toBe("DELETE");
  });

  /// Called as a page is closing, which is the ordinary case: a rejected promise there
  /// is an unhandled rejection in the last moment of the page's life.
  it("does not throw when the call fails", () => {
    const { host } = fakeHost({ responses: [new TypeError("Failed to fetch")] });

    expect(() => releaseToken("tok", host)).not.toThrow();
  });

  it("does nothing without a token", () => {
    const { host, fetch } = fakeHost();

    releaseToken("", host);

    expect(fetch).not.toHaveBeenCalled();
  });
});
