# Remote Access — Implementation Plan

**Status:** approved 2026-07-24, Phase 0 not started
**Scope:** `claudinio-code` (desktop harness) + `claudinio-relay` (new server component) + `claudin.io` (web control surface)
**Author:** design doc, written before the code — belongs in `docs/plans/`
**Language:** English, per the repo's English-only rule

---

## 0. Decisions taken at approval (2026-07-24)

The design below was audited against `main` at `88a1af3` before approval. The
repo map in §9, the layering rule in §4.1 and both `SECURITY.md` citations in
§6.4 were verified to match the tree as it stands. Four decisions were taken:

| Decision | Choice | Consequence |
|---|---|---|
| Branch `feat/cli-tui` (+27 commits, unmerged) | **Abandoned.** | That branch extracted `agent/`/`code_intel/`/`lsp/` into a Tauri-free `core/` crate, replaced `Channel<AgentEvent>` with a `trait EventSink`, and added a ratatui CLI. Abandoning it keeps `main` the single truth, so §9's repo map and the Phase 1 refactor stand as written. It also means the Phase 1 event-bus work is *not* partly done, and `claudiniod` (Phase 5) has no existing CLI to build on. The branch is not deleted; the npm packaging in `22c3a5d` may be worth salvaging later. |
| Where `remote/` lives | **`src-tauri/src/remote/`**, as §9 specifies. | Sibling of `agent/`, guarded by the architecture test in `lib.rs`. |
| Open question §11.1 — headless-first or GUI-first | **GUI-first**, phase order unchanged. | `claudiniod` stays in Phase 5. |
| Where the devices/pairings API lives (§9, `claudin.io`) | **The existing dashboard** (`claudinio_litellm`). | It already has accounts, Stripe, Postgres, Caddy and blue-green deploys. No new service; the `/v1/devices` endpoints and the `devices`/`pairings`/`pairing_tokens` tables are added there. |
| Target surface | **Mobile-first, always.** | The peer is assumed to be a phone until proven otherwise. Consequences are folded into §1.1, §6.2, §7 and §8 rather than left as a styling note — see §0.1. |

### 0.1 What mobile-first changes

Not a skin on a desktop design. Four things move:

1. **WebKit is the binding constraint on the crypto suite, not a footnote.**
   Every browser on iOS is WebKit, so the iOS version floor *is* the X25519
   floor. §6.2's suite survives Phase 0 on desktop; it is not confirmed until it
   runs on a real iPhone. This is now the gating measurement for Phase 2, not a
   nice-to-have.
2. **The browser tab is the baseline; the PWA is a required upgrade, not a
   gate.** Everything must work in mobile Safari with nothing installed —
   confirmed on a real iPhone 2026-07-24, straight in the browser. The one
   capability that cannot work that way is **Web Push, which on iOS fires only
   for a PWA installed to the home screen**, and §8 Phase 4 uses push so a
   remote approval does not stall silently.

   So the PWA is a must-have and leaves §8 Phase 6 ("Optional") for Phase 3 —
   but it buys notifications, not access. Two rules follow, and they are easy to
   violate by accident:

   - The pairing flow of §6.3 *offers* install, and never requires it.
   - **No approval may depend on push to be answerable.** An uninstalled peer
     must still see pending approvals on its next foreground, and `expires_at`
     with a local-side deny is what keeps a forgotten one from hanging. Push is
     what makes it timely, not what makes it possible.
3. **A backgrounded phone drops its socket constantly.** §7 treats reconnect as
   an exception; on mobile it is the steady state. The `Gap` → re-`Subscribe`
   → replay-from-JSONL path becomes the common case and has to be cheap, not
   merely correct. The 60 ms coalescing tick of §5.5 also deserves a lower rate
   on cellular — it was sized for a desktop socket.
4. **The approval gate is a thumb target.** §7 of the threat model rests on a
   human reading a diff before approving. On a phone, a diff that cannot be read
   produces reflexive approval, which is worse than no remote approval at all.
   Phase 3's `packages/timeline-ui` must therefore be touch-first from its first
   commit; retrofitting is how the gate gets quietly defeated.

One correction to the plan as originally written is folded into §8 Phase 3: the
pnpm workspace does **not** exist yet.

Open questions §11.2 through §11.5 remain open.

### 0.2 Deviations found while building (kept current)

This plan was written before the code. Where the code disagreed and the code was
right, it is recorded here rather than left buried in a commit message.

| § | Plan said | What was built, and why |
|---|---|---|
| §9 | `crates/claudinio-protocol` at the repo root | `src-tauri/crates/claudinio-protocol`, a member of the existing workspace. The repo root has no manifest **on purpose** — see the comment in `src-tauri/Cargo.toml` — and adding one to satisfy a path in a document would undo that. |
| §9 | one protocol crate | It is **split along the security boundary**: `wire` (the outer frame) always available, `inner` (the end-to-end messages) behind a default feature the relay turns off. I2 stops being a convention any later commit could quietly break in review and becomes a compile error — the relay has no inner types at all. CI builds the crate that way so the guarantee stays real. |
| §5.4 | `cmd_id` dedup as an LRU of seen ids | An LRU of ids cannot be correct on its own. Recording an id *after* executing lets a replay run the side effect twice; recording it *before* lets a crash swallow a legitimate retry. The log records **both edges**, and a command that started and never finished returns `Indeterminate` — the caller refuses and says why. Re-running an `rm` is worse than making someone tap approve again. |
| §5.2 | eight frame kinds | Plus `Other(u8)`, so a kind this build has never seen still decodes and can still be routed. The component in the middle is the one that must never need deploying in lockstep with the peers. |
| §8 Phase 1 | four items | Five. Event buses had to be keyed by session in a registry, or a second watcher could never find a running one — which is also the first thing the bridge needs. |
| §5.3 | a shared `Actor` | Deliberately **duplicated**. The agent owns the concept of who answered a gate; the protocol crate owns its wire form. Making `agent/` depend on the remote protocol to express a *local* approval would be the agent knowing remote access exists, which §4.1 forbids. The conversion lives in `remote/`, which may know both, and is tested there. |

| §6.3 | "confirm the SAS on both screens" | The SAS had to become a **gate**, not a display. As first built it went to `eprintln!` and the channel was served regardless, which made the words decoration. The connection now stops after `HelloAck` and serves nothing until a human answers, and **silence refuses** — a timeout, a dropped waiter and a superseded one all resolve to "no". |
| §6.3 | — | Refusing **revokes** rather than un-pairs. `admit` writes the pairing before the words can reach a screen, so removing it would leave the key free to pair again on the next attempt — and that key is exactly the one that must not. Consequence: `run()` needed a second ending, because redialling a revoked key loops against `admit` forever. |
| §6.3 | a 6-word typed code alongside the QR | The QR shipped; **the word code cannot**. `sas::WORDS` has 32 entries, so six words carry 30 bits — nowhere near a 128-bit channel plus a 256-bit key. A typed code has to be a *lookup* token, which needs `/v1/pairings/claim` in the dashboard. Blocked there, not here. |
| §8 Phase 3 | `TimelineRows.tsx` moves to the package | It is not a leaf: five sibling components plus `openExternalUrl` from `lib/ipc`. Moving it means inverting that into a host-supplied prop, which belongs with the web UI that will be its second consumer. `markdown.ts`, `records.ts` and `chatRecords.ts` moved. |
| §6.4 | the UI edits the policy | The **writer lives in `commands/remote.rs`, not `remote/`**. `remote/policy.rs` has no writer at all, so a command arriving over the wire cannot reach one — the module graph is the enforcement rather than a check inside a function. |

One temporary `#[allow(dead_code)]` remains, on `persist::SeqRecord::seq`, scoped
to builds without the `remote` feature. An earlier note here claimed the
module-wide suppression on `remote/` would come off with `remote/bridge.rs`; it did
not, because nothing consumed the bridge until the transport landed. It is gone
now. Chasing that lint paid for itself twice: it surfaced an `ApprovalResolved`
that was never emitted and an hourly rekey that was never called.

### 0.3 The pairing code is a cross-repo contract

`app.claudin.io` does not exist yet, and this is the format it will have to read.
Defined in `src-tauri/src/remote/code.rs` and pinned by tests there.

```
https://app.claudin.io/#c=<channel-hex-32>&k=<device-key-hex-64>&r=<relay-url>&e=<expiry-ms>
```

**Everything sits in the fragment, and that is not cosmetic.** A fragment is never
sent to the server. In a query string the channel token and the device key would
appear in the request line of every page load — handing the web origin, its logs
and whatever sits in front of it the two things it must not have: the ability to
attach to the channel, and the key a substitution attack needs. The web app reads
`location.hash`, and may not quietly change that.

`r` is percent-escaped for `%`, `&`, `=`, `#` and space, so a relay URL carrying
its own query string cannot truncate the parameters after it.

The device renders the QR itself (`qrcode` crate → SVG) because the code is already
in that process, and because the device has to be able to show it with the relay
unreachable — the relay may well be the thing being set up.

---

## 1. Objective

Let a developer drive a Claudinio Code session running on **their own machine** from **any browser**, with `claudin.io` as the control surface — without weakening a single guarantee currently listed in `SECURITY.md`.

Concretely: the agent loop, the filesystem, the shell, the index and the JSONL transcript **stay on the developer's machine**. What travels is an encrypted stream of `AgentEvent`s outbound and a small set of authenticated commands inbound.

### 1.1 Non-goals

Naming these now, because each one is a plausible-sounding scope explosion:

- **Not** running the agent in the cloud. No workspace upload, no remote execution environment. That is a different product with a different threat model.
- **Not** turning the desktop app into a multi-tenant server. One device serves the peers *its user* paired, and nothing else.
- **Not** a web IDE. The browser gets the timeline, the composer, the approval gates and diffs — not a file editor.
- **Not** making `claudin.io` a required dependency. The relay is self-hostable and the URL is configurable; remote access is opt-in and the app works fully offline without it.
- **Not** shipping a *native* mobile app. A responsive PWA is the mobile story — but per §0.1 that PWA is the primary surface, not a fallback, and it ships in Phase 3 rather than Phase 6.

### 1.2 The one-sentence architecture

> The desktop app opens a single outbound WebSocket to a **dumb, untrusted relay**; the browser opens another; the two endpoints run an end-to-end encrypted Noise session **through** the relay, so the server routes ciphertext it cannot read.

---

## 2. Invariants

These are load-bearing. A change that breaks one of them is a redesign, not a refinement.

| # | Invariant | Why |
|---|---|---|
| I1 | The JSONL transcript under `.claudinio/sessions/` remains the single source of truth. | Already true (`ARCHITECTURE.md`). It makes remote resume a *file read*, not a server-side buffer. The relay needs no durable storage. |
| I2 | The relay sees ciphertext and routing metadata only. Never plaintext code, prompts, output or diffs. | This is the security story *and* the marketing story. It also means a relay compromise is a denial-of-service, not a breach. |
| I3 | Remote capability is a **subset** of local capability, never a superset. | The remote peer must not be able to do anything the local user could not, and the local user must be able to grant strictly less. |
| I4 | Policy is edited locally only. A remote peer can read the policy; it can never widen it. | Otherwise the first thing a compromised peer does is grant itself everything. |
| I5 | No inbound listener on the developer's machine. Connection is outbound-only. | Works behind NAT/CGNAT (the common Brazilian home connection), no port forwarding, no new attack surface on the LAN. |
| I6 | The local user can revoke any peer instantly, and every grant expires by default. | Revocation that requires a server round-trip is revocation that fails when you need it. |
| I7 | Approval gates are never bypassed for remote. They are *answerable* remotely, subject to policy. | The approval gate is the entire mitigation for prompt injection (`SECURITY.md`). Weakening it remotely deletes the mitigation. |
| I8 | The desktop app never depends on the relay to function. | Offline is a first-class state, not a failure. |

---

## 3. Options considered

| Option | How | Verdict |
|---|---|---|
| **A. Expose a local HTTP server + tunnel** (ngrok/cloudflared style) | App binds `0.0.0.0:PORT`, user tunnels it | **Rejected.** Violates I5. Auth becomes bearer-token-in-URL. The tunnel provider terminates TLS and sees everything. |
| **B. TLS-only relay** — server terminates TLS from both sides and forwards plaintext | Simplest to build | **Rejected.** Violates I2. Every customer's source code, secrets in logs, and diffs pass through our servers in the clear. For a flat-rate API business this is an enormous liability with no compensating benefit. |
| **C. E2EE relay over WebSocket (Noise)** | Outbound WS from both ends, Noise handshake end-to-end | **Chosen.** Satisfies I2 and I5. Relay is stateless and cheap. Works everywhere a browser works. |
| **D. WebRTC data channel, relay as signaling only** | P2P, TURN fallback | **Deferred to Phase 6.** Lower latency and lower relay bandwidth, but ICE/TURN is a large operational surface. The relay path must exist anyway as fallback, so build it first. |
| **E. Run the agent server-side, sync the workspace** | Cloud agent | **Rejected.** Non-goal 1.1. Different product. |

**Decision: C now, D later as an optimisation behind the same protocol.**

---

## 4. Target architecture

```
  Developer's machine                    claudin.io                     Any browser
┌──────────────────────────┐      ┌─────────────────────┐      ┌────────────────────────┐
│ Claudinio Code (Tauri)   │      │  claudinio-relay    │      │  app.claudin.io        │
│                          │      │                     │      │  (separate origin)     │
│  agent/  code_intel/ lsp/│      │  • authn devices    │      │                        │
│        │                 │      │  • authn peers      │      │  @claudinio/timeline-ui│
│   ┌────┴─────┐           │      │  • route by channel │      │  (shared with desktop) │
│   │ eventbus │───────────┼──WSS─┤  • rate limit       ├─WSS──┤                        │
│   └────┬─────┘           │      │  • NO plaintext     │      │  WebCrypto X25519      │
│        │                 │      │  • NO durable store │      │  non-extractable keys  │
│   remote/  (feature-gated)│      └─────────────────────┘      └────────────────────────┘
│    transport · noise      │                 ▲                            ▲
│    pairing · policy       │                 └──── ciphertext only ───────┘
│    bridge   · protocol    │
│                           │      ┌─────────────────────┐
│  commands/ (thin IPC)     │      │  claudin.io (main)  │  device list, pairings,
└──────────────────────────┘      │  dashboard/billing  │  revocation, audit, push
                                   └─────────────────────┘
```

### 4.1 Why `remote/` is a sibling of `agent/`, not a child

`lib.rs::architecture_tests` fails the build if `agent/`, `code_intel/` or `lsp/` import `crate::commands`. The remote bridge must respect the same dependency rule:

```
commands/remote.rs  ──depends on──►  remote/  ──depends on──►  agent/
                                                                  │
                       agent/ NEVER imports remote/ ◄─────────────┘
```

`agent/` publishes events to an event bus and consumes approval resolutions from a resolver trait. It does not know a remote peer exists. `remote/` subscribes to the bus and implements a resolver. `commands/remote.rs` is a thin adapter exposing `remote_status`, `remote_enable`, `remote_disable`, `remote_pairings`, `remote_revoke` to the webview.

**Add an architecture test:** `agent/` must not import `crate::remote`. Same mechanism, same file.

---

## 5. Protocol

### 5.1 Layering

```
WSS (TLS, browser↔relay and device↔relay)
 └─ Outer frame        — cleartext to the relay, routing only
     └─ Noise session  — end-to-end, relay cannot read
         └─ Inner message — the actual command / event
```

### 5.2 Outer frame (relay-visible)

MessagePack. The relay parses only this, and only these fields:

```rust
struct OuterFrame {
    v: u8,                 // protocol version
    kind: OuterKind,       // Hello | HelloAck | Open | Data | Close | Ping | Pong | Error
    channel: ChannelId,    // 16 bytes, opaque to the relay
    seq: u64,              // per-channel, per-direction, monotonic
    ack: u64,              // highest contiguous seq received by the sender
    payload: Bytes,        // Noise ciphertext — opaque
}
```

The relay's entire job: authenticate the connection, look up which peer connection owns `channel`, forward the frame, enforce quotas. It never allocates a buffer larger than `MAX_FRAME` (256 KiB) and never writes `payload` to disk or logs.

### 5.3 Inner messages (end-to-end)

Defined once in Rust in `crates/claudinio-protocol`, with TypeScript types generated by `ts-rs` and **checked into the repo with a CI job that fails on drift**. One definition, three consumers (device, relay for the outer layer, browser). This is the "ligar as pontas" requirement — a hand-maintained TS mirror will diverge, and the divergence will be found in production.

**Peer → Device**

| Message | Notes |
|---|---|
| `ListWorkspaces` | Only workspaces on the policy allowlist are returned |
| `ListSessions { workspace }` | Metadata from the JSONL directory |
| `Subscribe { session_id, from_seq }` | `from_seq: 0` = full replay |
| `Unsubscribe { session_id }` | |
| `SendMessage { cmd_id, session_id, text, attachments? }` | `cmd_id` = client UUID, used for dedup |
| `Steer { cmd_id, session_id, text }` | Mid-reasoning guidance, matches the existing queue-and-inject behaviour |
| `Interrupt { cmd_id, session_id }` | Equivalent of `Esc` |
| `ResolveApproval { cmd_id, session_id, tool_use_id, decision, reason? }` | |
| `SetMode { cmd_id, session_id, mode }` | Brain ↔ Builder, policy-gated |
| `GetPolicy` | Read-only. There is deliberately no `SetPolicy`. |

**Device → Peer**

| Message | Notes |
|---|---|
| `Snapshot { session_id, records, seq }` | Replay chunk from JSONL |
| `Event { session_id, seq, event }` | A serialised `AgentEvent` |
| `ApprovalRequest { session_id, tool_use_id, tool, args, diff?, expires_at }` | |
| `ApprovalResolved { tool_use_id, decision, actor }` | `actor` = `Local` or `Peer(label)` — the loser of a race gets this |
| `Gap { session_id, from_seq, to_seq }` | Peer fell behind; it must re-`Subscribe { from_seq }` |
| `PolicyDenied { cmd_id, rule, human_reason }` | Always explain *which* rule, never fail silently |
| `Policy { .. }` | Current effective policy, so the UI can grey out what it cannot do |
| `Ack { cmd_id }` / `Error { cmd_id, code, message }` | |

### 5.4 Ordering, resume, idempotency

- **Sequencing.** Every device→peer message carries a `seq` derived from the JSONL append index of the session. Contiguity is assertable: `seq` gaps mean loss, and loss is detectable rather than silent.
- **Resume.** On reconnect the peer sends `Subscribe { from_seq: last_seen + 1 }`. The device re-reads the JSONL — which it already does on every message — and replays. **The relay needs zero persistence**, which is what makes I2 cheap to keep.
- **Idempotency.** Every peer→device command carries `cmd_id`. The device keeps a per-channel LRU of the last 512 `cmd_id`s, persisted next to the session. A replayed `SendMessage` after a flaky reconnect must not send twice; a replayed `ResolveApproval` must not run the command twice. **This is the single most important correctness requirement in the whole design** — a duplicated approval is a duplicated `rm`.
- **Approval races.** Local and remote can both answer. Resolution is a compare-and-swap on the oneshot; first writer wins, the loser receives `ApprovalResolved` with the winning `actor`. The actor is written into the JSONL record so the transcript answers "who approved this?" months later.

### 5.5 Backpressure

Token streaming produces a lot of small events. Three rules:

1. **The local agent loop never blocks on a remote subscriber.** The bus is `tokio::sync::broadcast`; a lagging receiver is dropped into a `Gap`, not awaited.
2. **Text deltas are coalesced on a 60 ms tick for remote channels only.** The local Tauri `Channel` keeps its current per-delta behaviour. Remote gets ~16 frames/s instead of hundreds.
3. **Per-channel outbound quota.** If a peer exceeds it, the device emits `Gap` and stops streaming until the peer re-subscribes. Bandwidth is a real cost line for a flat-rate business; it is capped at the source, not just at the relay.

---

## 6. Security design

### 6.1 Identity

| Key | Where | Lifetime |
|---|---|---|
| Device static (X25519) + signing (Ed25519) | OS config dir; **OS keychain from Phase 5** | Per install, rotatable |
| Peer static (X25519) | Browser IndexedDB, **non-extractable WebCrypto key** | Per browser profile, per pairing |
| Session keys | Memory only | Per connection, rekeyed hourly |

Non-extractable browser keys matter: an XSS on `app.claudin.io` can *use* the key while the page is open, but cannot exfiltrate it for offline use.

### 6.2 Handshake

**`Noise_IK_25519_AESGCM_SHA256`.**

`IK` because the initiator (browser) already knows the responder's (device's) static key — it came out of band via the pairing code. That kills relay-substitution MITM at the root: the relay never supplies a key, so it never gets to supply a *wrong* one.

`AESGCM` rather than the more usual ChaChaPoly because the browser side is then implementable on **plain WebCrypto** — X25519 + HKDF-SHA256 + AES-GCM, all natively supported in current Chrome/Safari/Firefox, with no crypto shipped in JavaScript. Rust side is `snow`, which supports the suite directly.

> **Verify before committing (Phase 0):** WebCrypto X25519 availability across the browser matrix we intend to support. If coverage is insufficient, fall back to `@noble/curves` + `@noble/ciphers` (audited, ~10 KB) and switch the suite to ChaChaPoly. Do not assume — measure on real browsers.
>
> **Resolved 2026-07-24. This decision is closed.** Interop with `snow` proved
> end to end; the suite confirmed on desktop macOS and, more to the point, on a
> real iPhone in mobile Safari with nothing installed. iOS was the binding case
> per §0.1 — every iOS browser is WebKit — and it passed, so the fallback to
> `@noble/curves` + ChaChaPoly is **not needed** and no crypto ships in
> JavaScript. Findings: `claudinio-relay/spikes/phase-0/FINDINGS.md`.
>
> One practical trap for anyone re-measuring: WebCrypto requires a **secure
> context**, so a phone loading the probe over `http://` on the LAN reports
> `crypto.subtle` missing — which reads as "no X25519" and is not. Serve it over
> TLS.

### 6.3 Pairing

1. User enables remote access in the desktop app. **Explicit local action, off by default.**
2. Device registers with `claudin.io` using the account credential it already has → `device_id`. Server stores the device's public keys and a fingerprint.
3. Desktop displays a **QR code and a 6-word code**, valid 120 s, single use. The payload carries `device_id`, the device static public key, and a one-time pairing token.
4. Browser scans/enters it, claims the token at the relay, and runs `Noise_IK` straight to the device.
5. **Both screens display a short authentication string** (emoji/word) derived from the handshake hash. The user confirms they match. This catches the manual-entry path where the key came from a typed code rather than a QR.
6. Device shows a naming prompt: "Pair *Firefox on Windows laptop*?" — the pairing is a named, listed, individually revocable object.

### 6.4 Remote policy — the actual guardrail

`~/.config/claudinio-code/remote-policy.json`, edited **only** through the local UI (I4):

```json
{
  "enabled": true,
  "expires_at": "2026-08-01T00:00:00Z",
  "workspaces": ["/Users/victor/dev/claudinio-code"],
  "idle_disconnect_minutes": 30,
  "allow": {
    "send_message": true,
    "steer": true,
    "interrupt": true,
    "set_mode": true,
    "approve_edit": true,
    "approve_bash": "allowlist",
    "read_attachment": false,
    "export_file": false
  },
  "bash_remote_denylist_extra": ["git push --force", "rm -rf", "curl|sh", "npm publish"],
  "max_unattended_minutes": 0
}
```

Two defaults deserve their own line, both drawn directly from the existing threat model:

- **`read_attachment: false`.** `SECURITY.md` states plainly that reads through the IPC surface are *not* workspace-scoped, because attaching a file from `~/Downloads` is the point of the feature. That reasoning is sound locally — a native picker means the user chose the file. It does **not** hold remotely. Exposing it to a peer turns "remote access to my agent" into "arbitrary read of my home directory". Deny by default, and if it is ever enabled, require a local native picker to satisfy it.
- **`approve_bash: "allowlist"`.** A remote peer approving arbitrary shell is the highest-consequence action in the system. Default to the read-only allowlist plus explicit per-command opt-in; full remote bash approval is a deliberate, expiring grant.

`export_file` stays remote-denied unconditionally — its whole security property is that the destination is chosen by a Rust-side native dialog and never crosses IPC as an argument. There is no remote equivalent of standing in front of the machine.

### 6.5 Threat model delta (goes into `SECURITY.md`)

**New defended boundaries**

| Boundary | Guarantee |
|---|---|
| Relay confidentiality | The relay observes ciphertext, `channel`, `seq` and frame sizes. It cannot read or forge inner messages. A malicious relay can drop, delay or reorder — all detectable via `seq` — but not read or inject. |
| Pairing authenticity | Pairing requires physical access to the machine (to read the code) plus SAS confirmation. Tokens are single-use with a 120 s TTL. The device static key arrives out of band, so relay key substitution fails closed. |
| Policy containment | A remote peer's capability is the intersection of local capability and the local policy. Policy cannot be widened remotely. Grants expire. |
| Revocation | Revoking a pairing locally drops the Noise session immediately and blacklists the peer key on-device, independent of relay availability. |

**Explicitly not defended (new)**

- **A compromised, currently-paired browser** has exactly what the policy grants. The mitigation is a tight policy, expiry and the audit trail — not the transport.
- **Traffic analysis.** The relay learns when you work, for how long, and roughly how much the agent produced. Padding is out of scope.
- **Prompt injection is still not solved, and remote access widens its blast radius** — it makes the approval gate answerable by someone who cannot see the machine. This must be stated loudly, not buried.
- **Relay-side denial of service.** Availability of remote access is not a security guarantee. The desktop app is fully functional without it (I8).

---

## 7. Reliability design

| Failure | Behaviour |
|---|---|
| Relay unreachable | Device retries with exponential backoff + full jitter (1 s → 60 s cap). Local operation is unaffected. UI shows a discreet "remote offline" state, not a modal. |
| Connection drops mid-run | The agent loop keeps running. Events continue to be appended to JSONL. On reconnect the peer resumes `from_seq` and sees everything it missed. **The run is never coupled to the socket.** |
| Peer falls behind | `Gap` → peer re-subscribes → replay from JSONL. No unbounded server buffer, no unbounded device buffer. |
| Duplicate command after reconnect | `cmd_id` LRU rejects it, replies `Ack` with the original result. |
| Approval pending, nobody watching | Approval requests carry `expires_at`. On expiry: deny, log, notify. Web Push fires when an approval request is raised, so a remote session does not silently stall. |
| Machine sleeps | Connection drops; on wake the device reconnects and the peer resumes. Sessions survive because JSONL survives. |
| Relay node restart / deploy | Device reconnects; routing is re-established from the device's `Hello`. Because the relay holds no durable state, deploys are trivially safe. |
| Clock skew | All expiries validated server-side and device-side; tokens carry `nbf`/`exp`; tolerance ±60 s. |

**SLOs to hold ourselves to:** p95 event delivery latency under 300 ms in-region; reconnect-to-resume under 5 s; zero `seq` gaps unreported. Measure these before claiming "reliable".

---

## 8. Phased plan

Every phase ends with a **prova real** — an observable, falsifiable check. A phase is not done because the code compiles.

### Phase 0 — De-risking spike (~1 week)

Throwaway code. Answers the questions that would invalidate the design.

- Rust `snow` ↔ browser WebCrypto `Noise_IK_25519_AESGCM_SHA256` interop.
- WebCrypto X25519 availability across the target browser matrix.
- Relay throughput with realistic token-stream frame sizes.

**Prova real:** a browser sends an encrypted message through a trivial relay; Rust decrypts and echoes; a packet capture *and* the relay's own logs show nothing but ciphertext. If interop fails, the crypto suite decision changes here — cheaply — instead of in Phase 3.

### Phase 1 — Event bus refactor (local only, no network) (~1–2 weeks)

The highest-value phase, and it ships value even if remote is cancelled.

- Replace the single Tauri `Channel` with a `tokio::sync::broadcast` fan-out plus a subscriber registry.
- Approvals: oneshot → compare-and-swap resolver with `actor` attribution, plus `expires_at`.
- Add monotonic `seq` to session records; expose `replay(session_id, from_seq)` in `agent/persist.rs`.
- Add the architecture test forbidding `agent/ → remote/`.

**Prova real:** two local windows attached to the same session both render the live stream; approving in one immediately closes the gate in the other and the JSONL records which one approved. Full existing test suite green — this refactor touches the heart of `agent/session.rs`, so a regression here is a regression everywhere.

### Phase 2 — Protocol crate + relay MVP (~2–3 weeks)

- `crates/claudinio-protocol`: outer frame, inner messages, `ts-rs` codegen, CI drift check.
- `claudinio-relay` (separate repo, Rust/axum/tokio-tungstenite): device auth, peer auth, in-memory routing, quotas. Single node.
- `remote/transport.rs` + `remote/noise.rs` + `remote/bridge.rs` behind `--features remote`.
- A **CLI peer** for testing. No web UI yet — the UI is not what is risky here.

**Prova real:** start a long agent run, `kill -9` the relay mid-stream, restart it. The device reconnects within backoff, the CLI peer resumes, and an automated assertion confirms **`seq` contiguity with zero gaps** across the outage.

### Phase 3 — Pairing, policy, read-only web UI (~3 weeks)

- Extract `src/lib/chatRecords.ts`, `src/components/chat/TimelineRows.tsx` and `src/lib/markdown.ts` into `packages/timeline-ui`, consumed by both the Tauri app and the web app. **Correction to the original draft:** the pnpm workspace does *not* exist — `pnpm-workspace.yaml` declares only `allowBuilds`, with no `packages:` key, and there is no `packages/` directory. The three files are real and the extraction is a move, but the workspace itself has to be created first. Budget for it.
- Pairing flow end to end: QR, 6-word fallback, SAS confirmation, named pairings, local revocation list.
- `remote/policy.rs` + the local policy editor UI.
- `app.claudin.io`: **its own origin**, separate from the dashboard/billing origin. Strict CSP, no inline script, and the same DOMPurify allowlist as the desktop. XSS here costs more than XSS in a Tauri webview, because here it is adjacent to a session cookie.
- Web UI is **read-only** in this phase: timeline, diffs, subagents. It cannot send anything.
- **Mobile-first (§0.1):** `timeline-ui` is touch-first from its first commit, and the PWA manifest plus service worker land here rather than in Phase 6 — Web Push in Phase 4 depends on home-screen install on iOS, so the install path has to exist before the write path does. It is offered, never required: the uninstalled browser tab stays a fully working peer.

**Prova real:** the SAS matches on both screens; a pairing revoked locally drops the channel in under one second **with the relay deliberately unreachable**; a frame tampered with in transit is rejected and logged rather than processed; **a diff is legible and approvable on a phone held in one hand**, checked on a real device rather than in a narrow desktop window.

### Phase 4 — Write path (~3 weeks)

- `SendMessage`, `Steer`, `Interrupt`, `SetMode`, `ResolveApproval`.
- Full policy enforcement with `PolicyDenied { rule, human_reason }` surfaced in the UI.
- Web Push for approval requests.
- Audit log: every remote command, with actor and decision, written to JSONL and to a local audit file.

**Prova real:** a `ResolveApproval` frame replayed three times executes the command **exactly once** (assert on process spawn count, not on log lines); a policy-denied tool call shows the user *which rule* blocked it; a `SendMessage` sent during a network partition and retried on reconnect appears once in the transcript.

### Phase 5 — Headless daemon + hardening → GA (~3–4 weeks)

- `claudiniod`: run the agent without a GUI session, so a workstation or home server can serve remote sessions after a reboot. This is what makes remote access genuinely useful rather than a demo.
- OS keychain backend for device keys (already on the roadmap in `SECURITY.md`; remote access makes it a prerequisite, not a nice-to-have).
- Relay horizontal scale: sticky routing by `device_id` via Redis or NATS.
- Multi-device, multi-pairing management in the dashboard.
- External security review of the pairing and policy code specifically.
- `SECURITY.md` and `ARCHITECTURE.md` updated with §6.5 and §4.1.

**Prova real:** a device rebooted headless reconnects and serves a session with no human at the keyboard; the security review finds no high-severity issue in pairing or policy; SLOs from §7 measured and met over a week of real usage.

### Phase 6 — Optional

WebRTC data channel with the relay as signalling and TURN fallback (cuts latency and relay bandwidth); self-hosted relay documentation and a `docker-compose.yml`.

*(PWA install and the mobile-shaped approval UI moved out of this phase — see §0.1. They are Phase 3.)*

**Rough total: 12–16 weeks of focused work.** Phases 0–2 carry most of the technical risk; 3–5 carry most of the surface area.

---

## 9. Concrete repository changes

### `claudinio-code`

```
src-tauri/src/
  remote/
    mod.rs           feature-gated behind `remote`
    transport.rs     outbound WSS, backoff+jitter, heartbeats
    noise.rs         snow wrapper, rekey, replay window
    pairing.rs       device identity, tokens, SAS, peer registry
    policy.rs        the guardrail — pure, heavily unit-tested
    bridge.rs        bus ⇄ protocol translation, cmd_id dedup
  agent/
    eventbus.rs      NEW — broadcast fan-out
    approval.rs      NEW — CAS resolver with actor attribution
    persist.rs       + seq, + replay(session_id, from_seq)
  commands/
    remote.rs        NEW — thin IPC adapter only
  lib.rs             + architecture test: agent/ ⊥ remote/

crates/
  claudinio-protocol/   shared wire definitions + ts-rs codegen

packages/
  timeline-ui/          extracted SolidJS timeline, shared desktop/web

src/
  components/settings/RemotePanel.tsx    enable, pair, policy, revoke
```

### `claudinio-relay` (new repo)

Separate repo, so the desktop app stays self-contained and self-hostable, and so relay deploys never touch the signed desktop release train.

```
src/
  main.rs        axum
  auth.rs        device challenge-response; peer session + pairing proof
  route.rs       channel table, forwarding, Redis/NATS for multi-node
  quota.rs       per-account, per-device, per-channel limits
  metrics.rs     frame counts, bytes, latency — never payloads
```

### `claudin.io`

Decided 2026-07-24: this is the **existing dashboard** (`claudinio_litellm`),
not a new service. Accounts, Stripe, Postgres, Caddy and blue-green deploys are
already there; the endpoints and tables below are added to it.

```
POST   /v1/devices                       register (account auth)
POST   /v1/devices/:id/pairing-tokens    device-signed
POST   /v1/pairings/claim                browser, one-time token
GET    /v1/devices | /v1/pairings        dashboard
DELETE /v1/pairings/:id | /v1/devices/:id
POST   /v1/push/subscribe                web push for approvals
WSS    /v1/device                        device uplink
WSS    /v1/peer                          browser downlink
```

Postgres: `devices`, `pairings`, `pairing_tokens` (hashed, TTL, single-use), `audit_events` (**metadata only**), `device_connections`. No message storage — there is nothing readable to store, and storing ciphertext would only create a liability with no product benefit.

---

## 10. Risks

| Risk | Severity | Mitigation |
|---|---|---|
| Remote access widens the blast radius of prompt injection | **High** | Deny-by-default policy, expiring grants, `read_attachment` denied remotely, approval expiry, prominent documentation. Accept and disclose; do not claim it is solved. |
| XSS on `app.claudin.io` reaches a paired key | **High** | Separate origin from billing, strict CSP, non-extractable WebCrypto keys, shared DOMPurify allowlist, no inline script. |
| Duplicate command execution on reconnect | **High** | `cmd_id` dedup with a persisted LRU; Phase 4 prova real asserts exactly-once on process spawns. |
| Crypto interop fails late | Medium | Phase 0 spike exists precisely to fail this early and cheaply. |
| Relay bandwidth cost on a flat-rate business | Medium | Coalescing at the source, per-channel quotas, tier limits, WebRTC in Phase 6. |
| Complexity creep in an OSS desktop app | Medium | `--features remote`, relay in a separate repo, `remote/` isolated by an enforced architecture test. |
| Users assume "remote" means "the cloud runs my code" | Low | Documentation and UI copy — the same care already applied to "flat-rate" over "unlimited". |

---

## 11. Open questions

1. ~~**Headless-first or GUI-first?**~~ **Decided 2026-07-24: GUI-first.** `claudiniod` stays in Phase 5 and the phase order is unchanged. See §0.
2. **Does the relay ship with a free tier?** Bandwidth is a real cost. Deciding this before Phase 4 avoids retrofitting quota logic.
3. **Team sessions** — one device, several paired humans, shared timeline. The protocol supports it (`channel` is per-peer); the *policy* model does not yet distinguish peers by role. Out of scope for v1, but do not design the policy schema in a way that forecloses it.
4. **Session handoff.** Should a remote peer be able to *start* a session on a device with no local user present, or only attach to existing ones? Starting implies workspace selection remotely, which needs its own allowlist semantics.
5. **Self-hosted relay** — v1 config flag, or documented from day one? It is a meaningful trust signal for an OSS project and cheap if designed in rather than bolted on.

---

## 12. Definition of done for v1

- [ ] A session running on a machine in Resende is driven **from a phone** in São Paulo, over CGNAT and over cellular, with no port forwarding.
- [ ] An approval is raised, pushed to that phone, read and answered there — with the diff legible on the first look, not after pinch-zooming.
- [ ] A packet capture at the relay and the relay's own logs contain no plaintext prompt, code, diff or output.
- [ ] Revoking a pairing kills the channel in under a second, with the relay unreachable.
- [ ] Killing the relay mid-run loses zero events; `seq` contiguity is asserted automatically.
- [ ] Replayed approval frames execute exactly once, asserted on process spawns.
- [ ] `SECURITY.md` states the new boundaries and, just as clearly, the new things we do not defend.
- [ ] Every SLO in §7 is measured, not assumed.
