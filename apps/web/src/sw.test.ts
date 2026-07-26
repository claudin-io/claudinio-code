import { describe, it, expect, vi } from "vitest";
import { dropOldCaches, handle, precache, wire, type Build, type SwScope } from "./sw";

const BUILD: Build = {
  cacheName: "claudinio-remote-abc123",
  assets: ["/assets/index-deadbeef.js", "/assets/index-deadbeef.css"],
};

/// A cache that records what was put in it and can be asked what it holds.
function fakeCache(holding: string[] = []) {
  const held = new Set(holding);
  return {
    held,
    addAll: vi.fn(async (urls: string[]) => urls.forEach((url) => held.add(url))),
    match: vi.fn(async (key: RequestInfo) => {
      const url = typeof key === "string" ? key : key.url;
      const path = url.startsWith("http") ? new URL(url).pathname : url;
      return held.has(path) ? new Response(`cached ${path}`) : undefined;
    }),
    put: vi.fn(),
  };
}

function fakeScope(options: { cache?: ReturnType<typeof fakeCache>; names?: string[] } = {}) {
  const cache = options.cache ?? fakeCache();
  const deleted: string[] = [];
  const listeners = new Map<string, (event: unknown) => void>();
  const fetch = vi.fn(async (request: RequestInfo) => {
    const url = typeof request === "string" ? request : request.url;
    return new Response(`network ${url}`);
  });

  const scope = {
    caches: {
      open: vi.fn(async () => cache),
      keys: vi.fn(async () => options.names ?? [BUILD.cacheName]),
      delete: vi.fn(async (name: string) => {
        deleted.push(name);
        return true;
      }),
    },
    location: { origin: "https://app.claudin.io" },
    fetch,
    clients: {},
    addEventListener: vi.fn((type: string, listener: (event: unknown) => void) => {
      listeners.set(type, listener);
    }),
  };

  return { scope: scope as unknown as SwScope, cache, deleted, fetch, listeners };
}

const request = (url: string, init: Partial<Request> = {}) =>
  ({
    url,
    method: "GET",
    mode: "no-cors",
    cache: "default",
    headers: new Headers(),
    ...init,
  }) as Request;

const navigation = (url: string) => request(url, { mode: "navigate" } as Partial<Request>);

describe("precache", () => {
  /// The shell plus the build's assets, in one atomic `addAll`. A per-file loop that
  /// swallowed one failure would leave a cache that boots to a page with no script —
  /// worse than having no worker.
  it("caches the shell and every asset of this build", async () => {
    const { scope, cache } = fakeScope();

    await precache(scope, BUILD);

    expect(cache.addAll).toHaveBeenCalledTimes(1);
    expect(cache.addAll).toHaveBeenCalledWith(["/", ...BUILD.assets]);
  });

  it("fails the install when an asset cannot be fetched", async () => {
    const { scope, cache } = fakeScope();
    cache.addAll.mockRejectedValueOnce(new Error("504"));

    await expect(precache(scope, BUILD)).rejects.toThrow("504");
  });
});

describe("dropOldCaches", () => {
  it("deletes the caches of earlier builds", async () => {
    const { scope, deleted } = fakeScope({
      names: ["claudinio-remote-old111", BUILD.cacheName, "claudinio-remote-old222"],
    });

    await dropOldCaches(scope, BUILD);

    expect(deleted).toEqual(["claudinio-remote-old111", "claudinio-remote-old222"]);
  });

  it("keeps this build's own cache", async () => {
    const { scope, deleted } = fakeScope({ names: [BUILD.cacheName] });

    await dropOldCaches(scope, BUILD);

    expect(deleted).toEqual([]);
  });
});

describe("handle", () => {
  // ── what it refuses to touch ───────────────────────────────────────────────

  /// The relay above all. WebSocket traffic never reaches a worker's `fetch` in the
  /// first place, but nothing cross-origin is answered from here either: I2 says the
  /// relay sees ciphertext only, and a worker that proxied anything to it would be a
  /// place where that stopped being true by accident.
  it("does not touch another origin", () => {
    const { scope } = fakeScope();

    expect(handle(scope, BUILD, request("https://relay.claudin.io/attach"))).toBeUndefined();
  });

  it("does not touch a request that is not a GET", () => {
    const { scope } = fakeScope();

    expect(
      handle(scope, BUILD, request("https://app.claudin.io/thing", { method: "POST" })),
    ).toBeUndefined();
  });

  /// A range request wants a slice of something the cache holds whole, and answering it
  /// with the whole thing is a correctness bug in whatever asked.
  it("does not touch a range request", () => {
    const { scope } = fakeScope();
    const ranged = request("https://app.claudin.io/icon-512.png");
    ranged.headers.set("range", "bytes=0-99");

    expect(handle(scope, BUILD, ranged)).toBeUndefined();
  });

  /// `only-if-cached` outside `same-origin` mode is a devtools artefact. Answering it
  /// from here throws.
  it("does not touch an only-if-cached probe", () => {
    const { scope } = fakeScope();

    expect(
      handle(
        scope,
        BUILD,
        request("https://app.claudin.io/", { cache: "only-if-cached", mode: "no-cors" } as Partial<Request>),
      ),
    ).toBeUndefined();
  });

  // ── the shell ──────────────────────────────────────────────────────────────

  /// Network first for the document, so an online browser always gets the newest build.
  /// This page holds a session key and renders model output; a cache-first shell would
  /// mean a fix reaching people whenever they happened to revalidate.
  it("goes to the network for a navigation", async () => {
    const { scope, fetch, cache } = fakeScope({ cache: fakeCache(["/"]) });

    const response = await handle(scope, BUILD, navigation("https://app.claudin.io/"))!;

    expect(fetch).toHaveBeenCalledTimes(1);
    expect(await response.text()).toBe("network https://app.claudin.io/");
    expect(cache.match).not.toHaveBeenCalled();
  });

  /// The reason the worker exists at all: a phone coming out of a tunnel shows "Looking
  /// for your machine" rather than the browser's offline page.
  it("falls back to the cached shell when the network is gone", async () => {
    const { scope, fetch } = fakeScope({ cache: fakeCache(["/"]) });
    fetch.mockRejectedValueOnce(new TypeError("Failed to fetch"));

    const response = await handle(scope, BUILD, navigation("https://app.claudin.io/"))!;

    expect(await response.text()).toBe("cached /");
  });

  /// Any path, because the pairing code is in the fragment and the server routes on
  /// nothing — every navigation is this one document.
  it("serves the shell for a navigation to any path", async () => {
    const { scope, fetch } = fakeScope({ cache: fakeCache(["/"]) });
    fetch.mockRejectedValueOnce(new TypeError("Failed to fetch"));

    const response = await handle(scope, BUILD, navigation("https://app.claudin.io/whatever"))!;

    expect(await response.text()).toBe("cached /");
  });

  /// With nothing cached — a first visit that failed — the browser's own offline page is
  /// the right answer. Inventing one here would mean writing a second copy of every
  /// message in `explain.ts`, in a file that cannot be updated without a deploy.
  it("lets the failure through when there is no cached shell", async () => {
    const { scope, fetch } = fakeScope({ cache: fakeCache([]) });
    fetch.mockRejectedValueOnce(new TypeError("Failed to fetch"));

    await expect(handle(scope, BUILD, navigation("https://app.claudin.io/"))!).rejects.toThrow(
      "Failed to fetch",
    );
  });

  // ── assets ─────────────────────────────────────────────────────────────────

  it("serves a precached asset from the cache", async () => {
    const { scope, fetch } = fakeScope({ cache: fakeCache(BUILD.assets) });

    const response = await handle(
      scope,
      BUILD,
      request("https://app.claudin.io/assets/index-deadbeef.js"),
    )!;

    expect(await response.text()).toBe("cached /assets/index-deadbeef.js");
    expect(fetch).not.toHaveBeenCalled();
  });

  /// A miss is something this build does not contain — an icon, the manifest — and goes
  /// to the network untouched.
  it("passes a miss to the network", async () => {
    const { scope, fetch } = fakeScope({ cache: fakeCache(BUILD.assets) });

    const response = await handle(scope, BUILD, request("https://app.claudin.io/icon-192.png"))!;

    expect(fetch).toHaveBeenCalledTimes(1);
    expect(await response.text()).toBe("network https://app.claudin.io/icon-192.png");
  });

  /// The invariant that would be expensive to discover later: **the cache only ever
  /// holds what the build put there.** A worker that wrote back what it saw would be a
  /// place where a response someone else provoked could be stored and then served as
  /// this origin's own code.
  it("never writes to the cache", async () => {
    const { scope, cache } = fakeScope({ cache: fakeCache(BUILD.assets) });

    await handle(scope, BUILD, request("https://app.claudin.io/icon-192.png"));
    await handle(scope, BUILD, navigation("https://app.claudin.io/"));

    expect(cache.put).not.toHaveBeenCalled();
    expect(cache.addAll).not.toHaveBeenCalled();
  });

  /// The pairing code lives in the fragment and the browser strips it before a worker
  /// sees the request. Asserted rather than assumed: if a future version keyed anything
  /// on a URL it built itself, the channel token and the device key would land in
  /// storage that outlives the tab.
  it("looks up a key that cannot contain a pairing code", async () => {
    const { scope, cache, fetch } = fakeScope({ cache: fakeCache(["/"]) });
    fetch.mockRejectedValueOnce(new TypeError("Failed to fetch"));

    await handle(scope, BUILD, navigation("https://app.claudin.io/"))!;

    for (const call of cache.match.mock.calls) {
      const key = typeof call[0] === "string" ? call[0] : (call[0] as Request).url;
      expect(key).not.toContain("#");
      expect(key).not.toContain("c=");
    }
  });
});

describe("wire", () => {
  it("registers the three handlers and nothing else", () => {
    const { scope, listeners } = fakeScope();

    wire(scope, BUILD);

    expect([...listeners.keys()].sort()).toEqual(["activate", "fetch", "install"]);
  });

  /// No `skipWaiting` and no `clients.claim`. A new build waits for every tab to close
  /// rather than claiming pages a running session is using — claiming mid-session would
  /// let the new worker fail to serve an asset the loaded bundle still asks for, and the
  /// symptom would be a session that died for no stated reason.
  it("does not take over the pages a running session is using", () => {
    const { scope } = fakeScope();
    const claimed = { skipWaiting: vi.fn(), clients: { claim: vi.fn() } };

    wire(Object.assign(scope, claimed) as unknown as SwScope, BUILD);

    expect(claimed.skipWaiting).not.toHaveBeenCalled();
    expect(claimed.clients.claim).not.toHaveBeenCalled();
  });

  /// `waitUntil`, so the browser does not kill the worker halfway through filling the
  /// cache — which is how a half-precached build would happen.
  it("holds the install open until the cache is filled", async () => {
    const { scope, listeners, cache } = fakeScope();
    wire(scope, BUILD);

    let held: Promise<unknown> | undefined;
    (listeners.get("install") as (event: unknown) => void)({
      waitUntil: (promise: Promise<unknown>) => {
        held = promise;
      },
    });

    expect(held).toBeInstanceOf(Promise);
    await held;
    expect(cache.addAll).toHaveBeenCalled();
  });

  /// A request the worker declines must reach the network as the browser would have sent
  /// it. Calling `respondWith` for everything and then proxying would put this file in
  /// the path of the relay handshake.
  it("leaves a request it declines to the browser", () => {
    const { scope, listeners } = fakeScope();
    wire(scope, BUILD);

    const respondWith = vi.fn();
    (listeners.get("fetch") as (event: unknown) => void)({
      request: request("https://relay.claudin.io/attach"),
      respondWith,
      waitUntil: vi.fn(),
    });

    expect(respondWith).not.toHaveBeenCalled();
  });

  it("answers a request it owns", () => {
    const { scope, listeners } = fakeScope({ cache: fakeCache(BUILD.assets) });
    wire(scope, BUILD);

    const respondWith = vi.fn();
    (listeners.get("fetch") as (event: unknown) => void)({
      request: request("https://app.claudin.io/assets/index-deadbeef.js"),
      respondWith,
      waitUntil: vi.fn(),
    });

    expect(respondWith).toHaveBeenCalledTimes(1);
  });
});

/// Importing this file must not register anything.
///
/// The auto-wire at the bottom of `sw.ts` is guarded on `clients`, which exists in no
/// global but a worker's. If that guard were dropped, importing the module from the page
/// — or from here — would attach a fetch handler to the window, and the failure mode is
/// a page that works in tests and not in a browser.
describe("the module itself", () => {
  it("wires nothing when it is not running as a worker", () => {
    expect((globalThis as { clients?: unknown }).clients).toBeUndefined();
    // Reaching this line at all is the assertion: the import at the top of this file
    // evaluated the guard, and a `wire(window)` would have thrown on `caches`.
    expect(typeof handle).toBe("function");
  });
});
