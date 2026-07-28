//! The wire format for remote access, defined once and consumed three times:
//! by the device (Claudinio Code), by the relay, and by the browser peer.
//!
//! # Why this crate is split
//!
//! The security claim of the whole design is that the relay routes ciphertext it
//! cannot read. `wire` — the outer frame — is everything the relay is allowed to
//! understand: a version, a kind, a channel to route by, sequence numbers, and
//! an opaque payload. `inner` is what travels *inside* that payload, encrypted
//! end to end.
//!
//! The relay depends on this crate with `default-features = false`. So the claim
//! is not a convention anyone could quietly break in a later commit: the relay
//! has no inner message types at all, and reaching for one does not compile.
//!
//! ```toml
//! # in the relay
//! claudinio-protocol = { git = "...", default-features = false }
//! ```

pub mod sas;
pub mod wire;

#[cfg(feature = "inner")]
pub mod inner;
