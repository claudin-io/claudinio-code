# app.claudin.io — the browser peer

Drives a Claudinio Code session running on the developer's own machine, through a
relay that cannot read the traffic. The agent, the files, the shell and the transcript
stay on that machine; what travels is an encrypted stream.

Read-only in this phase, per §8 of `docs/plans/2026-07-24_remote-access.md`. The
session can send `Subscribe` and nothing else — there is no composer and no approval
button, and the write path is phase 4.

## Its own origin, on purpose

Separate from `claudin.io`, where the dashboard and billing live, and it needs no
account of its own.

The peer's authority comes from the Noise pairing with the device and the channel
token on the relay — not from being signed in anywhere. Putting it on the
authenticated origin would give it privileges it does not use and a blast radius it
does not need: any XSS anywhere on that origin could reach the pairing material and
drive a developer's machine, and conversely this page renders model output, so a
sanitizer bypass here would reach the billing session.

The dashboard is where you *find* it. A link, not an embed.

## Building it

```sh
pnpm --filter @claudinio/web build     # → apps/web/dist, a static bundle
pnpm --filter @claudinio/web dev       # against a local relay, port 1430
```

Nothing is server-rendered and there is no API of its own, so `dist/` can be served
by anything — which is also what makes the whole thing self-hostable, as §1.1
requires.

## Deploying it

The bundle is static. What matters is the headers.

```caddyfile
app.claudin.io {
	root * /srv/app.claudin.io
	encode gzip zstd
	file_server

	header {
		# The policy is also in a meta tag, so a mis-deploy degrades to the same
		# rules rather than to none. Two directives only work as a header:
		#
		# frame-ancestors — this page must not be embeddable. The origin boundary
		# that keeps it away from the dashboard's session is worth nothing if the
		# dashboard can wrap it in an iframe.
		#
		# connect-src allows any wss: rather than one host, because the relay URL
		# arrives in the pairing code and self-hosting has to keep working. Plain
		# ws: is still refused, which is the part that matters. The one https
		# origin is the account server, for the typed-code path only — exactly
		# one host, so an XSS here cannot exfiltrate to one of its choosing.
		Content-Security-Policy "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self' wss: https://claudin.io; worker-src 'self'; manifest-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"

		Referrer-Policy "no-referrer"
		X-Content-Type-Options "nosniff"
		Cross-Origin-Opener-Policy "same-origin"
		Strict-Transport-Security "max-age=31536000; includeSubDomains"
		Permissions-Policy "camera=(self), geolocation=(), microphone=(), payment=()"
	}

	# Hashed by content, so they can never be the wrong bytes under that name.
	@immutable path /assets/*
	header @immutable Cache-Control "public, max-age=31536000, immutable"

	# The worker is the one file that must not be cached by anything but the browser's
	# own update check. A stale sw.js is a build that cannot be replaced: it keeps
	# serving the assets it precached, and the fix never arrives.
	@worker path /sw.js
	header @worker Cache-Control "no-cache"

	# The pairing code lives in the fragment, which the server never sees — so this
	# is a single-page app with one entry point and nothing to route.
	try_files {path} /index.html
}
```

`camera=(self)` is deliberate: scanning a QR from within the page is how pairing works
on a phone that cannot open the code from another screen. Everything else is off.

Two things the file server has to get right on its own: `manifest.webmanifest` must be
served as `application/manifest+json` (Caddy knows the extension; a bare nginx does not),
and `/sw.js` must be served from the root — a worker's scope is the directory it comes
from, so a worker under `/assets/` could only ever control `/assets/`.

## Installable, and what that is for

Not for offline: this is a remote control for a machine somewhere else, and with no
network there is nothing to control. §0.1 moved the manifest and the worker into phase 3
for a different reason — **Web Push on iOS is available only to a page launched from the
home screen**, and phase 4's approval requests are worth little if they cannot reach a
phone in a pocket. So the install path has to exist before the write path does.

Offered, never required. Registration failure is swallowed, the offer is a dismissible
line of text shown only on iOS (where the browser offers nothing itself), and the
uninstalled tab is a fully working peer.

- `manifest.webmanifest` — `start_url` is `/` and carries no pairing code. It cannot: the
  code lives in the fragment, which is not part of a start URL. A launch from the home
  screen lands on "Scan a pairing code to begin", so the installed icon is never a stored
  key. No `share_target`, no `file_handlers`, no `protocol_handlers` either: this page's
  authority comes from a code someone read off a screen, and an entry point that hands it
  one from elsewhere is a way to skip that.
- `src/sw.ts` — the worker. Precaches the shell and this build's hashed assets, serves the
  shell network-first and assets cache-first, and **never writes to the cache at runtime**.
  Built separately by `vite.sw.config.ts` as a classic script named `sw.js`, with its
  precache list injected from the app build's own manifest.
- `src/install.ts` — registration, and the one place an install is offered.

No `vite-plugin-pwa`. It would do all of the above and a great deal more, including
generating the worker — and the worker is the one file on this origin that can intercept
every request forever. Forty lines that can be read in full beat a generated one that
cannot.

```sh
scripts/prova-real-pwa.sh
```

Builds, then loads `dist/sw.js` the way a browser would — a classic script in a bare
worker scope — and drives it against a cache and a network backed by the real `dist/`.
It exists because the interesting failures are invisible to the unit tests: the precache
list is injected from a manifest written by a *different* Vite build, so the two can
disagree and leave the offline page blank while the worker reports success. Cutting the
list down by one file is enough to make it fail, which is the property that makes it
worth running.

## Two ways in, and only one of them needs an account

**Scanning needs nothing.** The device shows a URL carrying the channel, its relay
token and its public key in the fragment; this page reads it and runs Noise IK
straight through the relay. No account, no lookup, nothing of `claudin.io` involved —
which §1.1 requires, because that origin must never be a hard dependency of remote
access.

**Typing a code needs a sign-in**, and that is not a compromise. A typed code cannot
carry 128 bits of channel plus 256 bits of key plus a relay token, so it has to be a
short handle that something resolves — and the account server refuses to resolve a
code for anyone but the account that minted it. Without that check a ten-character
code is a bearer token for a developer's machine. The sign-in exists for that one
check and nothing else.

```
this page                     claudin.io
  │  verifier kept, hash sent      │
  ├── GET /remote/authorize ──────▶│  (cookie-authenticated, here only)
  │◀── 302 …/#auth=<code> ─────────┤  fragment: never sent, never logged
  ├── POST /api/remote/token ─────▶│  code + verifier → a 15-minute token
  ├── POST …/pairings/claim ──────▶│  code → channel, token, device key
  └── DELETE /api/remote/token ───▶│  released the moment the claim lands
```

- The verifier is in `sessionStorage`: it must survive one navigation and must not
  survive the tab.
- The token is in a variable and nowhere else. §10 names XSS on this origin as the
  high risk, and a credential in storage is readable by any script here afterwards.
  A reload re-authorises, which for a signed-in user is a silent redirect.
- The claim's response goes through the **same** `validatePairing` as a URL somebody
  else may have written. Treating a body from our own server as already-checked is
  how that server would end up able to point this browser at a `ws://` relay — where
  the frames are still ciphertext, but an observer learns the channel token.
- None of it can pair anything. The token resolves a code; the device still stops at
  three words a human has to compare.

Self-hosting: `VITE_ACCOUNT_ORIGIN` at build time, and the CSP's `connect-src` in
`index.html` **and** in the deployed header have to name the same origin. A policy
still naming claudin.io refuses the fetch, and the failure looks like the account
server being down.

## Layout

- `pairing.ts` — reads the code out of `location.hash`. The untrusted edge: refuses a
  channel or key of the wrong shape, refuses a relay URL that is not `wss:`, strips
  credentials, and treats an unreadable expiry as already stale. `validatePairing` is
  the part both ways in share.
- `auth.ts` — the typed-code path: the handoff above, the device list, the claim.
- `wire.ts` — the outer frame, mirroring `claudinio-protocol`'s `wire.rs`.
- `noise.ts` — `Noise_IK_25519_AESGCM_SHA256`, initiator side, on plain WebCrypto.
  No cryptography of ours beyond the state machine.
- `session.ts` — dial, handshake, gate on the words, subscribe, reconnect.
- `explain.ts` — what the user is told for every state, and why each wording.
- `golden.ts` — a recorded exchange with the real device. See below.
- `main.tsx` — the bare page.

## The golden vectors are the point

Everything else in this app can be tested against itself, and that proves nothing
about whether it can talk to a device. `golden.ts` holds a handshake and an exchange
recorded from the Rust side with both Noise ephemerals fixed, so the browser's first
message, handshake hash, SAS words and transport frames are all checked byte for byte
against what `snow` produces.

That is not theoretical. The first time these existed they caught, within the hour, a
device that attached to the relay with no channel token — so nothing could ever have
connected, while both sides' test suites stayed green.

Regenerate with, from `src-tauri/`:

```sh
cargo test --features remote golden -- --ignored --nocapture
```

The static keys change on every run, so the constants are replaced as a set.
