//! Remote access: driving a local session from a browser, through a relay that
//! cannot read the traffic.
//!
//! Feature-gated behind `remote`, so a build without it carries none of this.
//!
//! # Which way the arrows point
//!
//! `remote/` subscribes to the agent's event bus and answers its approval gates.
//! The agent does not know remote access exists — `architecture_tests` in
//! `lib.rs` fails the build if anything under `agent/` names `crate::remote`.
//!
//! That is not tidiness. It is what keeps "a remote peer can do no more than the
//! local user, and possibly less" enforceable in one place. The moment the agent
//! loop can ask whether a peer is connected, the capability rules start being
//! decided in two places, and one of them will be wrong.
//!
//! ```text
//! commands/remote.rs  ──depends on──►  remote/  ──depends on──►  agent/
//!                                                                  │
//!                        agent/ NEVER imports remote/ ◄─────────────┘
//! ```

// The feature is under construction and off by default: `dedup` is complete and
// tested, but `bridge.rs` — the thing that calls it — does not exist yet, so
// every item here is genuinely unreachable and the lint is right. One
// suppression at the module boundary rather than a scattering of them, and it
// comes off when the bridge lands.
#![allow(dead_code)]

pub mod dedup;
