//! Turning tool output into evidence.
//!
//! Every parser here is a pure function over text, which is the whole point:
//! the trustworthy part of the harness is the part with no I/O and no model in
//! it, so it can be tested exhaustively against real fixtures.

pub mod coverage;
pub mod mutation;
pub mod tests;

pub use coverage::{CoverageSummary, parse_lcov};
pub use mutation::{MutationOutcome, MutationSummary, interpret_exit_code, parse_mutants_out};
pub use tests::{TestSummary, parse_cargo_test, parse_jest_json, parse_vitest_json};
