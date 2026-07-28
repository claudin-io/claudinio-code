/// The service worker.
///
/// # Why there is one at all
///
/// Not for offline: this page is a remote control for a machine somewhere else, and
/// without a network there is nothing to control. It is here for two other reasons.
///
/// 1. **Web Push on iOS requires a home-screen install, and an install requires a
///    service worker.** That is the phase-4 dependency §0.1 moved forward: the write
///    path is worth much less if an approval request cannot reach a locked phone, so the
///    install path has to exist before the notification does.
/// 2. **A launch from the home screen must not show the browser's offline page.** With
///    the shell cached, a phone coming out of a tunnel shows "Looking for your machine",
///    which is true and recoverable, instead of a dinosaur.
///
/// It is offered, never required (§0.1). Registration failure is swallowed: a browser in
/// private mode, an insecure context, or a user who blocked storage all get a fully
/// working peer in a tab. Nothing here is on the path to pairing.
///
/// # What it deliberately does not do
///
/// - **It never populates the cache at runtime.** What is in the cache is exactly what
///   the build put there, named by content hash. A worker that cached whatever it saw
///   would be a place where a same-origin fetch someone else provoked could be stored
///   and then served back as this origin's own code.
/// - **It never touches the relay.** WebSocket traffic does not pass through a service
///   worker's `fetch` at all, and cross-origin requests are not intercepted regardless.
///   I2 is not weakened by this file: there is nothing here that could see plaintext.
/// - **It never sees a pairing code.** The browser strips the fragment from
///   `event.request.url`, and the code lives in the fragment. There is no path by which
///   the channel token or the device key reaches a cache. Asserted below, because it is
///   the kind of thing a later refactor could quietly break.
/// - **It does not call `skipWaiting`.** A new build waits for every tab to close rather
///   than claiming the pages a running session is using. Claiming mid-session would let
///   the new worker serve — or fail to serve — assets the loaded bundle still asks for,
///   and the visible symptom would be a session that died for no stated reason.

/// The navigation fallback.
///
/// One entry point: the pairing code is in the fragment, so the server never routes on
/// a path and every navigation is this document.
const SHELL = "/";

/// What one build consists of, passed in rather than read from a global.
///
/// Injected at build time from Vite's own asset manifest — see `vite.sw.config.ts`. A
/// build-time list rather than a runtime one is what makes the cache auditable: it holds
/// the shell and the hashed assets of exactly one build. And passing it as an argument is
/// what lets the tests below exercise every handler without pretending to be a worker.
export interface Build {
  /// Named for the content of the asset list, so a deploy lands in a new cache and
  /// `activate` can delete every older one without knowing what they were called.
  cacheName: string;
  /// The hashed assets. The shell is added separately; it is the one URL that is not
  /// named by its content.
  assets: string[];
}

// ── the scope, structurally ──────────────────────────────────────────────────
//
// Minimal interfaces rather than `lib.webworker`. Pulling the worker lib into a file
// that shares a tsconfig with the DOM app produces two conflicting definitions of half
// the globals, and typing the scope structurally costs nothing here.

interface SwExtendableEvent {
  waitUntil(promise: Promise<unknown>): void;
}

interface SwFetchEvent extends SwExtendableEvent {
  request: Request;
  respondWith(response: Response | Promise<Response>): void;
}

export interface SwScope {
  addEventListener(type: "install", listener: (event: SwExtendableEvent) => void): void;
  addEventListener(type: "activate", listener: (event: SwExtendableEvent) => void): void;
  addEventListener(type: "fetch", listener: (event: SwFetchEvent) => void): void;
  caches: CacheStorage;
  location: { origin: string };
  fetch(request: RequestInfo): Promise<Response>;
  /// Present in no global but a worker's. Used as the guard for auto-wiring.
  clients: unknown;
}

// ── the handlers ─────────────────────────────────────────────────────────────

/// Fill a fresh cache with this build's shell and assets.
///
/// `addAll` is atomic in the way that matters: if any one of them fails the install
/// fails, the worker never activates, and the previous one keeps serving. A half-filled
/// cache would boot to a page whose script is missing — the one failure mode worse than
/// having no worker at all.
export async function precache(scope: SwScope, build: Build): Promise<void> {
  const cache = await scope.caches.open(build.cacheName);
  await cache.addAll([SHELL, ...build.assets]);
}

/// Drop every cache but this build's.
///
/// Only reached once no client is left, because `skipWaiting` is not called — so this
/// cannot pull an asset out from under a page that is still running.
export async function dropOldCaches(scope: SwScope, build: Build): Promise<void> {
  const names = await scope.caches.keys();
  await Promise.all(
    names.filter((name) => name !== build.cacheName).map((name) => scope.caches.delete(name)),
  );
}

/// Whether this request is ours to answer.
///
/// Everything that is not a plain same-origin GET is left to the browser. The relay is
/// another origin, a range request wants a slice of something we would hand over whole,
/// and `only-if-cached` outside `same-origin` mode is a devtools artefact that throws if
/// answered from here.
function mine(scope: SwScope, request: Request): boolean {
  if (request.method !== "GET") return false;
  if (request.headers.has("range")) return false;
  if (request.cache === "only-if-cached" && request.mode !== "same-origin") return false;
  return new URL(request.url).origin === scope.location.origin;
}

/// The shell, newest first.
///
/// Network first, deliberately. Cache first would launch a few hundred milliseconds
/// sooner and would also mean a browser running an old bundle until it happened to
/// check — and this bundle holds a session key and renders model output, so "a fix
/// reaches everyone on their next launch" is worth more than the milliseconds.
async function shell(scope: SwScope, build: Build, request: Request): Promise<Response> {
  try {
    return await scope.fetch(request);
  } catch (error) {
    const cache = await scope.caches.open(build.cacheName);
    const cached = await cache.match(SHELL);
    if (cached) return cached;
    throw error;
  }
}

/// A precached asset, from the cache.
///
/// Cache first with no revalidation and no write-back: every asset in that list is named
/// by the hash of its content, so a hit is by construction the right bytes. A miss means
/// the request is not part of this build — a `public/` file, the manifest, an icon — and
/// goes to the network untouched.
async function asset(scope: SwScope, build: Build, request: Request): Promise<Response> {
  const cache = await scope.caches.open(build.cacheName);
  const cached = await cache.match(request);
  return cached ?? scope.fetch(request);
}

/// Answer one request, or hand it back to the browser by returning nothing.
export function handle(
  scope: SwScope,
  build: Build,
  request: Request,
): Promise<Response> | undefined {
  if (!mine(scope, request)) return undefined;
  if (request.mode === "navigate") return shell(scope, build, request);
  return asset(scope, build, request);
}

/// Attach the handlers to a scope.
export function wire(scope: SwScope, build: Build): void {
  scope.addEventListener("install", (event) => event.waitUntil(precache(scope, build)));
  scope.addEventListener("activate", (event) => event.waitUntil(dropOldCaches(scope, build)));
  scope.addEventListener("fetch", (event) => {
    const response = handle(scope, build, event.request);
    if (response) event.respondWith(response);
  });
}

// ── the build, and the wiring ────────────────────────────────────────────────

/// Replaced at build time by `vite.sw.config.ts`.
declare const __CACHE_NAME__: string;
declare const __ASSETS__: string[];

// Wired only in a real worker. `clients` exists in no other global, so importing this
// file from a test — or, one day, from the page by mistake — registers nothing and never
// evaluates the two build-time constants.
const global: Partial<SwScope> | undefined =
  typeof self === "undefined" ? undefined : (self as unknown as Partial<SwScope>);
if (global?.clients && global.caches) {
  wire(global as SwScope, { cacheName: __CACHE_NAME__, assets: __ASSETS__ });
}
