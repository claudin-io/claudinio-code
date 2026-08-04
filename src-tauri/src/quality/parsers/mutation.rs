//! Mutation-testing result parsers.
//!
//! Mutation testing answers the question coverage cannot: are the tests
//! *load-bearing*, or do they merely execute the code? A model that writes
//! `assert!(result.is_ok())` gets full coverage and catches nothing; break the
//! logic underneath and the suite stays green. Mutants are that break, applied
//! deliberately.
//!
//! cargo-mutants deliberately documents its `outcomes.json` as "subject to
//! change", so nothing here parses it. The stable surface is the four outcome
//! text files (one mutant per line) plus the process exit code, and that is
//! what we read.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// How many surviving mutants to quote back. Each one is a concrete "change
/// this and no test complains", so a handful is already a full afternoon of
/// work — a hundred would just flood the context.
const MAX_SURVIVORS_KEPT: usize = 15;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MutationSummary {
    /// Mutants a test caught.
    pub caught: u32,
    /// Mutants that survived — the tests did not notice the broken logic.
    pub missed: u32,
    /// Mutants that hung. Counted as detected, matching every mainstream
    /// tool's convention: an infinite loop is the tests noticing, loudly.
    pub timeout: u32,
    /// Mutants that could not be compiled or run. Excluded from the score
    /// entirely — they measure nothing about the tests.
    pub unviable: u32,
    pub survivors: Vec<String>,
    /// False when no report could be read, so the caller falls back to the
    /// exit code instead of trusting the zeroes.
    pub parsed: bool,
}

impl MutationSummary {
    /// Mutants that actually tested something.
    pub fn valid(&self) -> u32 {
        self.caught + self.timeout + self.missed
    }

    pub fn detected(&self) -> u32 {
        self.caught + self.timeout
    }

    /// Percentage of viable mutants the tests caught.
    ///
    /// No viable mutants scores 100: the change generated nothing to detect,
    /// and failing that would block work the tool had no opinion about.
    pub fn score(&self) -> f64 {
        if self.valid() == 0 {
            return 100.0;
        }
        (self.detected() as f64 / self.valid() as f64) * 100.0
    }

    pub fn headline(&self) -> String {
        if !self.parsed {
            return "mutation report could not be read".into();
        }
        if self.valid() == 0 {
            return "no viable mutants generated for the changed code".into();
        }
        format!(
            "{:.1}% of mutants caught ({}/{}), {} survived",
            self.score(),
            self.detected(),
            self.valid(),
            self.missed
        )
    }
}

/// Read a `mutants.out` directory produced by cargo-mutants.
///
/// `parsed` tracks whether the directory exists at all, because cargo-mutants
/// writes **nothing** when a `--in-diff` run generates no mutants (verified
/// against 27.1.0). An absent directory therefore means "no mutants", not
/// "broken report" — the caller separates the two using the exit code.
pub fn parse_mutants_out(dir: &Path) -> MutationSummary {
    if !dir.is_dir() {
        return MutationSummary::default();
    }
    // Within an existing run, an absent outcome file means zero of that
    // outcome: cargo-mutants only writes the ones it has.
    let count = |name: &str| -> Vec<String> {
        std::fs::read_to_string(dir.join(name))
            .map(|text| {
                text.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };

    let missed = count("missed.txt");
    MutationSummary {
        caught: count("caught.txt").len() as u32,
        timeout: count("timeout.txt").len() as u32,
        unviable: count("unviable.txt").len() as u32,
        survivors: missed.iter().take(MAX_SURVIVORS_KEPT).cloned().collect(),
        missed: missed.len() as u32,
        parsed: true,
    }
}

/// What a cargo-mutants exit code says about the run.
///
/// The codes are documented and stable, which makes them a better primary
/// signal than any file: they distinguish "the tests are weak" from "we never
/// got to find out".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationOutcome {
    /// The run produced real data; score it.
    Scored,
    /// Nothing was learned, and the reason is worth showing the user.
    NotMeasured(&'static str),
}

pub fn interpret_exit_code(code: Option<i32>) -> MutationOutcome {
    match code {
        // Every viable mutant was caught.
        Some(0) => MutationOutcome::Scored,
        // Surviving mutants, and/or timeouts: both are real results.
        Some(2) | Some(3) => MutationOutcome::Scored,
        Some(1) => MutationOutcome::NotMeasured(
            "cargo-mutants rejected its arguments; check quality.mutation_cmd",
        ),
        // The suite was already red, so mutation could not start. The tests
        // layer already reported that — repeating it as a mutation failure
        // would double-punish one problem.
        Some(4) => MutationOutcome::NotMeasured(
            "the test suite was already failing, so no mutants were tested",
        ),
        Some(5) => MutationOutcome::NotMeasured(
            "the diff no longer matches the working tree; re-run after the code settles",
        ),
        Some(6) => MutationOutcome::NotMeasured("the generated diff was not valid for --in-diff"),
        Some(70) => MutationOutcome::NotMeasured("cargo-mutants hit an internal error"),
        Some(other) => {
            // Unknown codes are treated as "we do not know", never as a pass.
            if other == 0 {
                MutationOutcome::Scored
            } else {
                MutationOutcome::NotMeasured("cargo-mutants exited with an unrecognized status")
            }
        }
        None => MutationOutcome::NotMeasured("cargo-mutants did not report an exit status"),
    }
}

/// Parse a report in the standard mutation-testing-elements JSON schema, which
/// Stryker and several other tools emit. Used for stacks driven through
/// `quality.mutation_cmd`.
pub fn parse_mutation_json(json: &str) -> MutationSummary {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return MutationSummary::default();
    };
    let Some(files) = v.get("files").and_then(|f| f.as_object()) else {
        return MutationSummary::default();
    };
    let mut summary = MutationSummary {
        parsed: true,
        ..Default::default()
    };
    for (path, file) in files {
        let Some(mutants) = file.get("mutants").and_then(|m| m.as_array()) else {
            continue;
        };
        for mutant in mutants {
            let status = mutant.get("status").and_then(|s| s.as_str()).unwrap_or("");
            match status {
                "Killed" => summary.caught += 1,
                "Timeout" => summary.timeout += 1,
                // NoCoverage is a survivor with a name: no test even ran it.
                "Survived" | "NoCoverage" => {
                    summary.missed += 1;
                    if summary.survivors.len() < MAX_SURVIVORS_KEPT {
                        let line = mutant
                            .get("location")
                            .and_then(|l| l.get("start"))
                            .and_then(|s| s.get("line"))
                            .and_then(|l| l.as_u64())
                            .unwrap_or(0);
                        let mutator = mutant
                            .get("mutatorName")
                            .and_then(|m| m.as_str())
                            .unwrap_or("mutant");
                        summary
                            .survivors
                            .push(format!("{path}:{line}: {mutator} survived"));
                    }
                }
                // Invalid mutants measure nothing about the tests.
                "CompileError" | "RuntimeError" | "Ignored" | "Pending" => summary.unviable += 1,
                _ => {}
            }
        }
    }
    summary
}

#[cfg(test)]
mod mutation_tests {
    use super::*;

    fn out_dir(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cq-mut-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        for (file, body) in files {
            std::fs::write(dir.join(file), body).unwrap();
        }
        dir
    }

    #[test]
    fn counts_each_outcome_file() {
        let dir = out_dir(
            "counts",
            &[
                (
                    "caught.txt",
                    "src/a.rs:1: replace a with b\nsrc/a.rs:2: x\n",
                ),
                ("missed.txt", "src/a.rs:9: replace > with >=\n"),
                ("timeout.txt", "src/b.rs:3: loop\n"),
                ("unviable.txt", "src/c.rs:1: nope\n"),
            ],
        );
        let s = parse_mutants_out(&dir);
        assert!(s.parsed);
        assert_eq!((s.caught, s.missed, s.timeout, s.unviable), (2, 1, 1, 1));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn timeouts_count_as_detected_and_unviable_is_excluded() {
        // 2 caught + 1 timeout detected, out of 4 viable. The unviable mutant
        // says nothing about the tests, so it stays out of the ratio.
        let dir = out_dir(
            "score",
            &[
                ("caught.txt", "a\nb\n"),
                ("missed.txt", "c\n"),
                ("timeout.txt", "d\n"),
                ("unviable.txt", "e\nf\ng\n"),
            ],
        );
        let s = parse_mutants_out(&dir);
        assert_eq!(s.valid(), 4);
        assert_eq!(s.detected(), 3);
        assert_eq!(s.score(), 75.0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_run_that_generated_no_mutants_scores_full() {
        let dir = out_dir("empty", &[("caught.txt", ""), ("missed.txt", "")]);
        let s = parse_mutants_out(&dir);
        assert!(s.parsed);
        assert_eq!(s.valid(), 0);
        assert_eq!(s.score(), 100.0);
        assert!(s.headline().contains("no viable mutants"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_absent_directory_is_marked_unparsed_rather_than_perfect() {
        // cargo-mutants 27.1.0 writes no mutants.out at all when --in-diff
        // yields nothing, so the caller must decide with the exit code; the
        // summary alone must never read as a clean sweep.
        let s = parse_mutants_out(Path::new("/nonexistent-mutants-out"));
        assert!(!s.parsed);
        assert!(s.headline().contains("could not be read"));
    }

    #[test]
    fn an_outcome_file_that_is_absent_within_a_real_run_counts_as_zero() {
        // Verified shape: a run with only survivors writes missed.txt and
        // omits the rest.
        let dir = out_dir(
            "partial",
            &[("missed.txt", "src/a.rs:2:26: replace * with +\n")],
        );
        let s = parse_mutants_out(&dir);
        assert!(s.parsed);
        assert_eq!((s.caught, s.missed, s.timeout), (0, 1, 0));
        assert_eq!(s.score(), 0.0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn survivors_are_quoted_and_bounded() {
        let many: String = (0..50).map(|i| format!("src/a.rs:{i}: mutant\n")).collect();
        let dir = out_dir("survivors", &[("missed.txt", &many)]);
        let s = parse_mutants_out(&dir);
        assert_eq!(s.missed, 50);
        assert_eq!(s.survivors.len(), MAX_SURVIVORS_KEPT);
        assert!(s.survivors[0].contains("src/a.rs"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exit_codes_separate_weak_tests_from_never_finding_out() {
        assert_eq!(interpret_exit_code(Some(0)), MutationOutcome::Scored);
        assert_eq!(interpret_exit_code(Some(2)), MutationOutcome::Scored);
        assert_eq!(interpret_exit_code(Some(3)), MutationOutcome::Scored);
        // A red baseline is the tests layer's problem, not a mutation verdict.
        assert!(matches!(
            interpret_exit_code(Some(4)),
            MutationOutcome::NotMeasured(m) if m.contains("already failing")
        ));
        assert!(matches!(
            interpret_exit_code(Some(5)),
            MutationOutcome::NotMeasured(_)
        ));
        assert!(matches!(
            interpret_exit_code(None),
            MutationOutcome::NotMeasured(_)
        ));
    }

    #[test]
    fn stryker_style_json_is_scored_by_the_standard_convention() {
        let json = r#"{
          "schemaVersion": "1.0",
          "files": {
            "src/a.ts": { "language": "typescript", "source": "",
              "mutants": [
                {"id":"1","status":"Killed","mutatorName":"BooleanLiteral","location":{"start":{"line":3}}},
                {"id":"2","status":"Timeout","mutatorName":"Loop","location":{"start":{"line":9}}},
                {"id":"3","status":"Survived","mutatorName":"ConditionalExpression","location":{"start":{"line":12}}},
                {"id":"4","status":"NoCoverage","mutatorName":"ArithmeticOperator","location":{"start":{"line":20}}},
                {"id":"5","status":"CompileError","mutatorName":"X","location":{"start":{"line":1}}}
              ]
            }
          }
        }"#;
        let s = parse_mutation_json(json);
        assert!(s.parsed);
        // Killed + Timeout detected; Survived + NoCoverage missed; CompileError excluded.
        assert_eq!((s.caught, s.timeout, s.missed, s.unviable), (1, 1, 2, 1));
        assert_eq!(s.valid(), 4);
        assert_eq!(s.score(), 50.0);
        assert!(s.survivors.iter().any(|x| x.contains("src/a.ts:12")));
    }

    #[test]
    fn arbitrary_json_is_not_a_mutation_report() {
        assert!(!parse_mutation_json(r#"{"hello":"world"}"#).parsed);
        assert!(!parse_mutation_json("{oops").parsed);
    }
}
