# Fix MCP Remote HTTPS Connection Failure

## Context / Problem Statement

MCP remote connections to HTTPS servers (e.g., `https://mcp.context7.com/mcp`) fail with:

```
MCP initialize failed for 'context7': Send message error Transport error:
Client error: error sending request for url (https://mcp.context7.com/mcp),
when send initialize request
```

**Root cause:** The `rmcp` v2.2.0 dependency brings in `reqwest` v0.13.x with only `features = ["json", "stream"]` — no TLS backend is enabled. When the MCP client attempts an HTTPS connection, reqwest has no way to establish the TLS handshake, resulting in "error sending request for url".

The same server works perfectly via `curl` (HTTP 200, SSE response), proving the endpoint is reachable and responding correctly.

## Key Findings (Prova Real)

| Finding | Method | Proof |
|---------|--------|-------|
| TLS works from curl | `curl -X POST https://mcp.context7.com/mcp -H "Content-Type: application/json" -d '...'` | HTTP 200 with valid SSE/JSON-RPC initialize response |
| rmcp 2.2.0 depends on reqwest 0.13.x with no TLS | `grep -A5 'reqwest' /tmp/rmcp-2.2.0/Cargo.toml` | `features = ["json", "stream"]` — no `rustls` or `native-tls` |
| rmcp exposes TLS feature `"reqwest"` | `grep 'reqwest =' /tmp/rmcp-2.2.0/Cargo.toml` (features section) | `reqwest = ["__reqwest", "reqwest?/rustls"]` |
| Project Cargo.toml does NOT activate rmcp's `"reqwest"` TLS feature | `src-tauri/Cargo.toml` line 128-133 | Features list: `["client", "macros", "transport-child-process", "transport-streamable-http-client-reqwest", "base64"]` — no `"reqwest"` |
| reqwest 0.12.28 (project's direct dep) HAS TLS | `Cargo.lock` reqwest 0.12.28 dependencies | Has `hyper-rustls`, `hyper-tls`, `native-tls` |
| reqwest 0.13.4 (rmcp's dep) has NO TLS | `Cargo.lock` reqwest 0.13.4 dependencies | Missing `hyper-rustls`, `hyper-tls`, `native-tls` |
| reqwest 0.13 `rustls` feature activates TLS stack | `/tmp/reqwest-0.13*/Cargo.toml.orig` | `rustls = ["__rustls-aws-lc-rs", "dep:rustls-platform-verifier", "__rustls"]` |

## Authoritative Inputs

- **rmcp feature `"reqwest"`** activates `reqwest?/rustls` (per rmcp 2.2.0 Cargo.toml)
- **User preference:** rustls (pure Rust, no system dependency) (confirmed)
- **Current rmcp features in Cargo.toml** (line 128-133): `["client", "macros", "transport-child-process", "transport-streamable-http-client-reqwest", "base64"]`

## Changes (Steps)

### Step 1: Add `"reqwest"` feature to rmcp dependency

- **Target:** `src-tauri/Cargo.toml` line 128-133 (rmcp dependency features list)
- **Mutation:** Add `"reqwest"` to the features array, between `"macros"` and `"transport-child-process"` (alphabetical-ish order)
- **Why:** This activates `reqwest?/rustls` in the rmcp dependency chain, giving reqwest 0.13.x the TLS backend (rustls) needed for HTTPS connections
- **Constraints:** Single feature addition; no other dependencies change

### Step 2: Rebuild and verify

- **Target:** `cargo build` in `src-tauri/`
- **Mutation:** None (build step)
- **Why:** Ensure the new feature compiles and doesn't introduce conflicts with existing reqwest 0.12 (used directly by the project) — the two reqwest versions are independent dependency trees
- **Constraints:** Must produce a successful build with no warnings

## Verification Plan

1. **Build check:** `cargo build --manifest-path src-tauri/Cargo.toml` exits 0 — proves the feature doesn't cause dep conflicts
2. **TLS dep check:** `grep -A40 'name = "reqwest"' src-tauri/Cargo.lock | grep -E '(rustls|native-tls)'` shows `hyper-rustls` or `rustls` present under reqwest 0.13.4 — proves TLS backend is now linked
3. **Manual regression:** User reconnects MCP with the Context7 config and confirms success
4. **Edge case:** Stdio-based MCP servers continue working (no regression) — the `"reqwest"` feature only affects the reqwest dep, not stdio transport

## Risks

- **Low risk:** Adding a feature to an existing dependency; no code changes
- **Compile time:** rustls (aws-lc-rs) may require some native build dependencies on certain platforms — macOS should be fine with the prebuilt crate
- **Size:** Binary will grow slightly (~1-2MB) from rustls crypto dependency


## Implementation Log — 2026-07-11 19:44
**Summary:** Fix MCP remote HTTPS connections by adding "reqwest" feature to rmcp dependency (enables reqwest?/rustls TLS backend)
**Changed files:** M src-tauri/Cargo.lock, M src-tauri/Cargo.toml, M src-tauri/src/agent/mod.rs, M src-tauri/src/agent/permissions.rs, M src-tauri/src/agent/provider.rs, M src-tauri/src/agent/session.rs, M src-tauri/src/agent/subagent.rs, M src-tauri/src/agent/tools/bash.rs, M src-tauri/src/agent/tools/finalize_plan.rs, M src-tauri/src/agent/tools/mod.rs, M src-tauri/src/commands/agent.rs, M src-tauri/src/commands/code_intel.rs, M src-tauri/src/commands/mod.rs, M src-tauri/src/lib.rs, M src-tauri/src/state.rs, M src/App.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? .commandcode/, ?? docs/plans/2026-07-11_mcp-remote-https-tls-fix.md, ?? src-tauri/src/agent/mcp.rs, ?? src-tauri/src/commands/mcp.rs
**Commits:** _(git unavailable or none)_
**Journal:** **Root cause:** MCP remote connections to HTTPS servers failed because `rmcp` v2.2.0 depends on `reqwest` v0.13.x, but the feature `"transport-streamable-http-client-reqwest"` only activates reqwest with `features = ["json", "stream"]` — no TLS backend. The project never activated rmcp's `"reqwest"` feature, which is what passes `reqwest?/rustls` to enable TLS via rustls.

**Fix:** Added `"reqwest"` to the rmcp features list in `src-tauri/Cargo.toml` line 130. This single-line change cascades `reqwest?/rustls` → `reqwest 0.13.4` → `hyper-rustls` + `rustls` + `rustls-platform-verifier` + `tokio-rustls` + `quinn`.

**Verification:**
- `cargo build` succeeded (1m 27s)
- `Cargo.lock` inspection confirmed `hyper-rustls`, `rustls`, `rustls-platform-verifier`, `tokio-rustls`, `quinn` all present under `reqwest 0.13.4` — previously absent.

**Key lesson:** The `rmcp` crate separates its features very granularly — `transport-streamable-http-client-reqwest` only enables the reqwest *dependency* but not its TLS features. You must also add the `"reqwest"` (or `"reqwest-native-tls"`) feature to rmcp to enable the TLS backend. This is a subtle but critical distinction that can take hours to debug if you don't check the Cargo.lock for the actual linked dependencies.

**Task journal:**
- Add "reqwest" feature to rmcp dependency for TLS support: Added "reqwest" feature to rmcp dependency in Cargo.toml line 130 — this activates reqwest?/rustls which enables TLS for HTTPS connections via the Rustls backend (pure Rust, no OpenSSL dependency)
- Build and verify TLS backend is linked: Build succeeded — cargo build exited 0 (1m 27s); Cargo.lock confirmed: reqwest 0.13.4 now has hyper-rustls, rustls, rustls-platform-verifier, tokio-rustls, quinn linked. Previously had none of these.
