/// The prova real for the installable page: the built worker, against the built bundle.
///
/// The unit tests in `src/sw.test.ts` exercise the handlers with a fake scope and prove
/// the logic. What they cannot see is the build — and the build is where this feature can
/// break in ways that only show up on a phone in a tunnel:
///
///   - the precache list injected from Vite's manifest could miss the very script the
///     shell loads, and every unit test would still pass;
///   - the worker could be emitted as an ES module, which some browsers refuse to
///     register, and the failure would be silent because registration failure is
///     deliberately swallowed;
///   - a file in the list could simply not be in `dist/`, and `addAll` would reject at
///     install time — leaving the previous worker serving an older build forever.
///
/// So this loads `dist/sw.js` as the browser would (a classic script, in a context whose
/// only global is a service-worker scope) and drives it with a cache and a network backed
/// by the real `dist/` directory. Then it cuts the network and asks for the page.
///
/// Run through `scripts/prova-real-pwa.sh`, which builds first.

import { createContext, runInContext } from "node:vm";
import { readFileSync, existsSync, readdirSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const DIST = resolve(HERE, "../dist");
const ORIGIN = "https://app.claudin.io";

let failures = 0;
const pass = (what) => console.log(`PASS  ${what}`);
const fail = (what, detail) => {
  failures += 1;
  console.error(`FAIL  ${what}\n      ${detail}`);
};
const check = (condition, what, detail) => (condition ? pass(what) : fail(what, detail));

if (!existsSync(join(DIST, "sw.js"))) {
  console.error(`no ${DIST}/sw.js — run scripts/prova-real-pwa.sh, which builds first`);
  process.exit(1);
}

// ── a network and a cache, backed by dist/ ───────────────────────────────────

let offline = false;
const fetched = [];

/// The network. Real `Response` objects, so the worker's `ok`/`text()` behave as they do
/// in a browser; the request objects are literals because a real `Request` cannot be
/// constructed with `mode: "navigate"` — that mode is reserved for the browser.
async function fetchFromDist(request) {
  if (offline) throw new TypeError("Failed to fetch");
  const url = typeof request === "string" ? request : request.url;
  fetched.push(url);
  let path = new URL(url, ORIGIN).pathname;
  if (path.endsWith("/")) path += "index.html";
  try {
    return new Response(readFileSync(join(DIST, path)));
  } catch {
    return new Response("not found", { status: 404 });
  }
}

class FakeCache {
  constructor() {
    this.entries = new Map();
  }
  async addAll(urls) {
    for (const url of urls) {
      const response = await fetchFromDist(url);
      if (!response.ok) throw new Error(`${url} → ${response.status}`);
      this.entries.set(key(url), Buffer.from(await response.arrayBuffer()));
    }
  }
  async match(request) {
    const body = this.entries.get(key(typeof request === "string" ? request : request.url));
    return body === undefined ? undefined : new Response(body);
  }
  async put() {
    throw new Error("the worker wrote to the cache, which it must never do");
  }
}

const key = (url) => new URL(url, ORIGIN).pathname;

const caches = {
  store: new Map(),
  async open(name) {
    if (!this.store.has(name)) this.store.set(name, new FakeCache());
    return this.store.get(name);
  },
  async keys() {
    return [...this.store.keys()];
  },
  async delete(name) {
    return this.store.delete(name);
  },
};

// ── the worker, loaded as a browser would load it ────────────────────────────

const listeners = new Map();
const scope = {
  addEventListener: (type, listener) => listeners.set(type, listener),
  caches,
  location: { origin: ORIGIN },
  fetch: fetchFromDist,
  // What the auto-wire at the bottom of sw.ts is guarded on.
  clients: {},
};

const code = readFileSync(join(DIST, "sw.js"), "utf8");
const context = createContext({ self: scope, URL, Response, Headers, TypeError, console });

try {
  // A classic script, not a module. `import`/`export` are a syntax error here — which is
  // the assertion: a module worker would fail to register in browsers that do not support
  // one, and because registration failure is swallowed on purpose, nobody would be told.
  runInContext(code, context, { filename: "sw.js" });
  pass("the built worker is a classic script and evaluates");
} catch (error) {
  fail("the built worker is a classic script and evaluates", String(error));
  process.exit(1);
}

check(
  listeners.has("install") && listeners.has("activate") && listeners.has("fetch"),
  "it wired itself on load",
  `registered ${[...listeners.keys()].join(", ") || "nothing"}`,
);

// ── install ──────────────────────────────────────────────────────────────────

const waited = [];
const event = (extra = {}) => ({ waitUntil: (promise) => waited.push(promise), ...extra });

listeners.get("install")(event());
try {
  await Promise.all(waited);
  pass("install precached every file it asked for");
} catch (error) {
  fail("install precached every file it asked for", `${error} — a file in the list is not in dist/`);
}

const [cacheName] = await caches.keys();
const cache = await caches.open(cacheName);
check(
  /^claudinio-remote-[0-9a-f]{12}$/.test(cacheName ?? ""),
  "the cache is named for the build",
  `got ${cacheName}`,
);

/// Everything the shell loads has to be in the cache.
///
/// The assertion that justifies this whole file. The precache list is injected from a
/// manifest written by a different build, and if the two ever disagree the page boots
/// offline to a document whose script 404s — a blank screen, with the worker reporting
/// success.
const shell = readFileSync(join(DIST, "index.html"), "utf8");
const referenced = [...shell.matchAll(/(?:src|href)="(\/[^"]+)"/g)]
  .map(([, url]) => url)
  .filter((url) => url.endsWith(".js") || url.endsWith(".css"));

check(referenced.length >= 2, "the shell references a script and a stylesheet", `found ${referenced}`);
const missing = referenced.filter((url) => !cache.entries.has(url));
check(
  missing.length === 0,
  "every script and stylesheet the shell loads was precached",
  `not cached: ${missing.join(", ")}`,
);
check(cache.entries.has("/"), "the shell itself was precached", "no entry for /");

// ── offline ──────────────────────────────────────────────────────────────────
//
// The reason the worker exists: a phone coming out of a tunnel gets "Looking for your
// machine", which is true and recoverable, rather than the browser's offline page.

offline = true;

async function serve(request) {
  let served;
  listeners.get("fetch")({
    request,
    respondWith: (response) => {
      served = response;
    },
    waitUntil: () => {},
  });
  return served === undefined ? undefined : await served;
}

const navigation = {
  url: `${ORIGIN}/`,
  method: "GET",
  mode: "navigate",
  cache: "default",
  headers: new Headers(),
};

const offlineShell = await serve(navigation);
if (offlineShell === undefined) {
  fail("the page is served with no network", "the worker declined the navigation");
} else {
  const html = await offlineShell.text();
  check(
    html.includes('<div id="root">'),
    "the page is served with no network",
    "the response was not the shell",
  );
}

/// And its script with it, from the cache, without a network.
const asset = {
  url: `${ORIGIN}${referenced.find((url) => url.endsWith(".js"))}`,
  method: "GET",
  mode: "no-cors",
  cache: "default",
  headers: new Headers(),
};
const offlineAsset = await serve(asset);
check(
  offlineAsset !== undefined && (await offlineAsset.text()).length > 1000,
  "its script is served with no network",
  "the script did not come back from the cache",
);

/// The relay is untouched, online or off. WebSocket traffic never reaches a worker's
/// `fetch` at all, and nothing cross-origin is answered here either — I2 says the relay
/// sees ciphertext only, and this file must not become a place where that changes.
const relay = {
  url: "wss://relay.claudin.io/attach?channel=c&role=browser&token=t",
  method: "GET",
  mode: "no-cors",
  cache: "default",
  headers: new Headers(),
};
check(
  (await serve(relay)) === undefined,
  "it does not answer for the relay",
  "the worker took a request bound for the relay",
);

// ── activate ─────────────────────────────────────────────────────────────────

caches.store.set("claudinio-remote-000000000000", new FakeCache());
waited.length = 0;
listeners.get("activate")(event());
await Promise.all(waited);

const left = await caches.keys();
check(
  left.length === 1 && left[0] === cacheName,
  "activating drops the caches of older builds",
  `left with ${left.join(", ")}`,
);

// ── what gets deployed ───────────────────────────────────────────────────────

check(
  !existsSync(join(DIST, ".vite")),
  "dist holds nothing but the deployable set",
  "dist/.vite survived the build; the precache manifest would be published",
);

const manifest = JSON.parse(readFileSync(join(DIST, "manifest.webmanifest"), "utf8"));
check(
  manifest.start_url === "/" && !manifest.start_url.includes("?") && !manifest.start_url.includes("#"),
  "the installed icon carries no pairing material",
  `start_url is ${manifest.start_url}`,
);
const iconsMissing = manifest.icons
  .map(({ src }) => src)
  .filter((src) => !existsSync(join(DIST, src)));
check(
  iconsMissing.length === 0 && existsSync(join(DIST, "apple-touch-icon.png")),
  "the icons the manifest names are there",
  `missing ${iconsMissing.join(", ") || "apple-touch-icon.png"}`,
);

// ── and nothing in the bundle is a secret ────────────────────────────────────
//
// Cheap, and it has caught its kind of mistake before: the worker's injected constants
// are a build artefact, and the day someone injects a relay URL or a key into them is the
// day this origin ships a credential to every visitor.
const suspicious = ["token", "secret", "privateKey", "device_key"].filter((word) =>
  code.includes(word),
);
check(
  suspicious.length === 0,
  "the worker embeds nothing that looks like a credential",
  `found ${suspicious.join(", ")} in dist/sw.js`,
);

console.log(
  failures === 0
    ? `\nAll checks passed. ${readdirSync(DIST).length} files in dist, cache ${cacheName}.`
    : `\n${failures} check(s) failed.`,
);
process.exit(failures === 0 ? 0 : 1);
