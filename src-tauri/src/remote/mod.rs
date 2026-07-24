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

// Every item here is reachable only from tests until something constructs a
// `Bridge`, which is the transport's job. The lint is right, so this is one
// suppression at the module boundary rather than a scattering of them.
//
// This was previously annotated as coming off "when the bridge lands". That was
// wrong: the bridge consumes `dedup`, but nothing consumes the bridge, so the
// module stayed unreachable. It comes off when `transport.rs` drives a bridge —
// and if that turns out to be wrong again, the fix is to stop writing this
// module ahead of its caller, not to widen this comment a third time.
#![allow(dead_code)]

pub mod bridge;
pub mod dedup;
pub mod noise;
pub mod transport;
