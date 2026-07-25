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
		# ws: is still refused, which is the part that matters.
		Content-Security-Policy "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self' wss:; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"

		Referrer-Policy "no-referrer"
		X-Content-Type-Options "nosniff"
		Cross-Origin-Opener-Policy "same-origin"
		Strict-Transport-Security "max-age=31536000; includeSubDomains"
		Permissions-Policy "camera=(self), geolocation=(), microphone=(), payment=()"
	}

	# The pairing code lives in the fragment, which the server never sees — so this
	# is a single-page app with one entry point and nothing to route.
	try_files {path} /index.html
}
```

`camera=(self)` is deliberate: scanning a QR from within the page is how pairing works
on a phone that cannot open the code from another screen. Everything else is off.

## Layout

- `pairing.ts` — reads the code out of `location.hash`. The untrusted edge: refuses a
  channel or key of the wrong shape, refuses a relay URL that is not `wss:`, strips
  credentials, and treats an unreadable expiry as already stale.
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
