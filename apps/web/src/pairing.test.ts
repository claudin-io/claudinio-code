import { describe, it, expect, vi } from "vitest";
import { parsePairingCode, isStale, forgetPairingCode } from "./pairing";

const CHANNEL = "ab".repeat(16);
const KEY = "cd".repeat(32);
const RELAY = "wss://relay.claudin.io/ws";

const fragment = (over: Record<string, string | null> = {}) => {
  const base: Record<string, string | null> = {
    c: CHANNEL,
    k: KEY,
    r: RELAY,
    e: String(Date.now() + 120_000),
    ...over,
  };
  return (
    "#" +
    Object.entries(base)
      .filter(([, v]) => v !== null)
      .map(([k, v]) => `${k}=${v}`)
      .join("&")
  );
};

const parse = (f: string) => parsePairingCode(f);

describe("parsePairingCode", () => {
  it("reads a code the device wrote", () => {
    const result = parse(fragment());
    expect(result.ok).toBe(true);
    if (!result.ok) return;

    expect(result.code.channel).toBe(CHANNEL);
    expect(result.code.deviceKey).toBe(KEY);
    expect(result.code.relayUrl).toBe(RELAY);
    expect(result.code.expiresAt).toBeGreaterThan(Date.now());
  });

  it("works whether or not the leading hash is included", () => {
    const withHash = parse(fragment());
    const without = parse(fragment().slice(1));
    expect(withHash).toEqual(without);
  });

  /// Opening the app directly is the ordinary case, not an error. It needs a
  /// different message from a corrupted code.
  it("reports an absent code separately from a broken one", () => {
    for (const empty of ["", "#", "#   "]) {
      const result = parse(empty);
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.error.kind).toBe("absent");
    }
  });

  it("is case-insensitive about hex", () => {
    const result = parse(fragment({ c: CHANNEL.toUpperCase(), k: KEY.toUpperCase() }));
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.code.channel).toBe(CHANNEL);
    expect(result.code.deviceKey).toBe(KEY);
  });

  it("refuses a channel that is not 32 hex characters", () => {
    for (const bad of ["", "abc", "z".repeat(32), CHANNEL + "ab"]) {
      const result = parse(fragment({ c: bad }));
      expect(result.ok, bad).toBe(false);
      if (result.ok) return;
      expect(result.error).toMatchObject({ kind: "malformed", field: "channel" });
    }
  });

  it("refuses a device key that is not 64 hex characters", () => {
    for (const bad of ["", "abc", "z".repeat(64), KEY + "cd"]) {
      const result = parse(fragment({ k: bad }));
      expect(result.ok, bad).toBe(false);
      if (result.ok) return;
      expect(result.error).toMatchObject({ kind: "malformed", field: "deviceKey" });
    }
  });

  /// Non-hex must fail on the character check rather than be stripped: stripping
  /// would let `ab!!…` become a short-but-valid-looking channel.
  it("does not strip non-hex to make a short code look valid", () => {
    const result = parse(fragment({ c: "ab!!" + "ab".repeat(14) }));
    expect(result.ok).toBe(false);
  });

  // ── the relay URL ────────────────────────────────────────────────────────

  /// The check that matters most here. A doctored code pointing at ws:// still
  /// carries ciphertext, but an observer on the path learns the channel token — and
  /// can then attach to the channel themselves.
  it("refuses a relay URL that is not wss", () => {
    for (const bad of [
      "ws://relay.claudin.io/ws",
      "http://relay.claudin.io/ws",
      "https://relay.claudin.io/ws",
      "javascript:alert(1)",
      "file:///etc/passwd",
    ]) {
      const result = parse(fragment({ r: bad }));
      expect(result.ok, bad).toBe(false);
      if (result.ok) return;
      expect(result.error).toMatchObject({ kind: "malformed", field: "relayUrl" });
    }
  });

  it("refuses a relay URL that is not a URL at all", () => {
    const result = parse(fragment({ r: "not a url" }));
    expect(result.ok).toBe(false);
  });

  it("refuses a missing relay URL", () => {
    const result = parse(fragment({ r: null }));
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.error).toMatchObject({ field: "relayUrl" });
  });

  /// Credentials in the URL would be sent on the handshake. Dropped rather than
  /// refused, because a code carrying them is more likely mangled than hostile —
  /// but what reaches the socket must not have them.
  it("strips credentials from the relay URL", () => {
    const result = parse(fragment({ r: "wss://user:secret@relay.claudin.io/ws" }));
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.code.relayUrl).not.toContain("secret");
    expect(result.code.relayUrl).not.toContain("user");
  });

  it("keeps a relay URL's path and query", () => {
    const result = parse(
      fragment({ r: encodeURIComponent("wss://relay.example.com/ws?region=eu") }),
    );
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.code.relayUrl).toContain("/ws");
    expect(result.code.relayUrl).toContain("region=eu");
  });

  // ── expiry ───────────────────────────────────────────────────────────────

  /// A code with no readable deadline is not something the device emits, so it is
  /// corrupted or crafted. Both read as stale, which sends the user for a fresh one
  /// instead of onto a channel of unknown age.
  it("treats a missing or unreadable expiry as already stale", () => {
    for (const bad of [null, "", "soon", "-1", "0", "NaN"]) {
      const result = parse(fragment({ e: bad }));
      expect(result.ok, String(bad)).toBe(true);
      if (!result.ok) return;
      expect(result.code.expiresAt).toBeNull();
      expect(isStale(result.code)).toBe(true);
    }
  });

  it("knows a fresh code from a lapsed one", () => {
    const fresh = parse(fragment({ e: String(1_000) }));
    expect(fresh.ok).toBe(true);
    if (!fresh.ok) return;

    expect(isStale(fresh.code, 500)).toBe(false);
    expect(isStale(fresh.code, 1_000)).toBe(true);
    expect(isStale(fresh.code, 2_000)).toBe(true);
  });

  /// Staleness is not a parse error: "get a new code" and "this URL is corrupted"
  /// are different problems with different fixes.
  it("still parses a lapsed code", () => {
    const result = parse(fragment({ e: "1000" }));
    expect(result.ok).toBe(true);
  });

  it("ignores parameters it does not know", () => {
    const result = parse(fragment() + "&surprise=1");
    expect(result.ok).toBe(true);
  });
});

describe("forgetPairingCode", () => {
  /// The code is single-use and short-lived; the URL is not. It lands in history, in
  /// a screenshot, in whatever gets pasted into a support request.
  it("takes the code out of the address bar without adding history", () => {
    const replaceState = vi.fn();
    forgetPairingCode({
      history: { replaceState } as unknown as History,
      location: { pathname: "/", search: "" } as unknown as Location,
    });

    expect(replaceState).toHaveBeenCalledWith(null, "", "/");
  });

  it("keeps the query string, which is not secret", () => {
    const replaceState = vi.fn();
    forgetPairingCode({
      history: { replaceState } as unknown as History,
      location: { pathname: "/app", search: "?lang=pt" } as unknown as Location,
    });

    expect(replaceState).toHaveBeenCalledWith(null, "", "/app?lang=pt");
  });

  /// A sandboxed context can refuse replaceState. Not worth failing over — the code
  /// is stale in two minutes either way.
  it("survives a context that will not let it", () => {
    const replaceState = vi.fn(() => {
      throw new Error("SecurityError");
    });
    expect(() =>
      forgetPairingCode({
        history: { replaceState } as unknown as History,
        location: { pathname: "/", search: "" } as unknown as Location,
      }),
    ).not.toThrow();
  });
});
