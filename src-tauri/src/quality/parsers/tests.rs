//! Test-result parsers.
//!
//! Where a runner offers machine-readable output we take it (`--reporter=json`,
//! `--json`). `cargo test` has no stable JSON on stable Rust, so its summary
//! line is parsed instead — and the exit code is always checked alongside it,
//! so a parse that finds nothing can never be mistaken for a green run.

use serde::{Deserialize, Serialize};

/// How many failing test names to keep. Enough to act on, small enough that a
/// catastrophically broken suite cannot flood the model's context.
const MAX_FAILURES_KEPT: usize = 20;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TestSummary {
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    /// Names (and messages, where available) of failing tests.
    pub failures: Vec<String>,
    /// False when the output could not be parsed at all, so the caller knows
    /// to fall back to the exit code rather than trust the zeroes.
    pub parsed: bool,
}

impl TestSummary {
    pub fn total(&self) -> u32 {
        self.passed + self.failed + self.skipped
    }

    pub fn headline(&self) -> String {
        if !self.parsed {
            return "test output could not be parsed; judging by exit code".into();
        }
        format!(
            "{} passed, {} failed, {} skipped",
            self.passed, self.failed, self.skipped
        )
    }
}

/// Parse `cargo test` output. A run covering several targets prints one
/// `test result:` line per target, so the counts are summed rather than taken
/// from the last line.
pub fn parse_cargo_test(output: &str) -> TestSummary {
    let mut summary = TestSummary::default();
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("test result:") {
            summary.parsed = true;
            // The first segment carries the verdict before the count
            // ("ok. 3 passed", "FAILED. 1 passed"), so pair up adjacent words
            // rather than assuming the number comes first.
            for part in rest.split(';') {
                let words: Vec<&str> = part.split_whitespace().collect();
                for pair in words.windows(2) {
                    let Ok(n) = pair[0].parse::<u32>() else {
                        continue;
                    };
                    match pair[1] {
                        "passed" => summary.passed += n,
                        "failed" => summary.failed += n,
                        "ignored" => summary.skipped += n,
                        _ => {}
                    }
                }
            }
        } else if let Some(name) = trimmed.strip_prefix("---- ")
            && let Some(name) = name.strip_suffix(" stdout ----")
            && summary.failures.len() < MAX_FAILURES_KEPT
        {
            summary.failures.push(name.to_string());
        }
    }
    // The `failures:` block lists names again; only fall back to it when the
    // panic-output headers gave us nothing.
    if summary.failures.is_empty() && summary.failed > 0 {
        summary.failures = cargo_failure_block(output);
    }
    summary
}

/// Names from the trailing `failures:` list `cargo test` prints.
fn cargo_failure_block(output: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_block = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed == "failures:" {
            in_block = true;
            continue;
        }
        if in_block {
            if trimmed.is_empty() || trimmed.starts_with("test result:") {
                in_block = false;
                continue;
            }
            if names.len() < MAX_FAILURES_KEPT {
                names.push(trimmed.to_string());
            }
        }
    }
    names
}

/// Parse vitest's `--reporter=json` output.
pub fn parse_vitest_json(json: &str) -> TestSummary {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return TestSummary::default();
    };
    let num = |key: &str| v.get(key).and_then(|n| n.as_u64()).unwrap_or(0) as u32;
    let mut summary = TestSummary {
        passed: num("numPassedTests"),
        failed: num("numFailedTests"),
        skipped: num("numPendingTests") + num("numTodoTests"),
        failures: Vec::new(),
        // Vitest and jest share this envelope; the presence of the counter is
        // what tells us we got a real report rather than arbitrary JSON.
        parsed: v.get("numTotalTests").is_some(),
    };
    collect_jest_style_failures(&v, &mut summary.failures);
    summary
}

/// Jest's `--json` uses the same envelope vitest emulates.
pub fn parse_jest_json(json: &str) -> TestSummary {
    parse_vitest_json(json)
}

/// Walk `testResults[].assertionResults[]` for anything that failed.
fn collect_jest_style_failures(v: &serde_json::Value, out: &mut Vec<String>) {
    let Some(files) = v.get("testResults").and_then(|t| t.as_array()) else {
        return;
    };
    for file in files {
        let Some(assertions) = file.get("assertionResults").and_then(|a| a.as_array()) else {
            continue;
        };
        for a in assertions {
            if a.get("status").and_then(|s| s.as_str()) != Some("failed") {
                continue;
            }
            if out.len() >= MAX_FAILURES_KEPT {
                return;
            }
            let name = a
                .get("fullName")
                .or_else(|| a.get("title"))
                .and_then(|n| n.as_str())
                .unwrap_or("<unnamed test>");
            let message = a
                .get("failureMessages")
                .and_then(|m| m.as_array())
                .and_then(|m| m.first())
                .and_then(|m| m.as_str())
                .unwrap_or("");
            let first_line = message.lines().next().unwrap_or("").trim();
            out.push(if first_line.is_empty() {
                name.to_string()
            } else {
                format!("{name}: {first_line}")
            });
        }
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    const CARGO_GREEN: &str = "\
running 3 tests
test a ... ok
test b ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
";

    const CARGO_RED: &str = "\
running 2 tests
test good ... ok
test bad ... FAILED

failures:

---- bad stdout ----
thread 'bad' panicked at src/lib.rs:10:5:
assertion failed

failures:
    bad

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
";

    #[test]
    fn cargo_green_run_is_parsed() {
        let s = parse_cargo_test(CARGO_GREEN);
        assert!(s.parsed);
        assert_eq!((s.passed, s.failed, s.skipped), (3, 0, 0));
        assert!(s.failures.is_empty());
    }

    #[test]
    fn cargo_failure_names_are_captured() {
        let s = parse_cargo_test(CARGO_RED);
        assert_eq!((s.passed, s.failed), (1, 1));
        assert!(
            s.failures.iter().any(|f| f.contains("bad")),
            "{:?}",
            s.failures
        );
    }

    #[test]
    fn cargo_sums_counts_across_targets() {
        // One `test result:` line per target — taking only the last would
        // under-report a workspace with unit and integration suites.
        let out = "test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out\n\
                   test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
        let s = parse_cargo_test(out);
        assert_eq!((s.passed, s.skipped), (7, 1));
    }

    #[test]
    fn unparseable_cargo_output_is_marked_unparsed() {
        let s = parse_cargo_test("error: could not compile `foo`");
        assert!(!s.parsed, "must not read as a green run");
        assert_eq!(s.total(), 0);
        assert!(s.headline().contains("exit code"));
    }

    #[test]
    fn vitest_json_counts_and_failures() {
        let json = r#"{
            "numTotalTests": 4, "numPassedTests": 2, "numFailedTests": 1,
            "numPendingTests": 1, "numTodoTests": 0,
            "testResults": [{"assertionResults": [
                {"status":"passed","fullName":"a works"},
                {"status":"failed","fullName":"b works",
                 "failureMessages":["AssertionError: expected 1 to be 2\n  at x.ts:3"]}
            ]}]
        }"#;
        let s = parse_vitest_json(json);
        assert!(s.parsed);
        assert_eq!((s.passed, s.failed, s.skipped), (2, 1, 1));
        assert_eq!(s.failures.len(), 1);
        assert!(s.failures[0].contains("b works"));
        assert!(s.failures[0].contains("expected 1 to be 2"));
    }

    #[test]
    fn arbitrary_json_is_not_a_test_report() {
        let s = parse_vitest_json(r#"{"hello":"world"}"#);
        assert!(!s.parsed);
    }

    #[test]
    fn broken_json_is_not_a_test_report() {
        assert!(!parse_vitest_json("{oops").parsed);
    }

    #[test]
    fn failure_list_is_bounded() {
        let assertions: Vec<String> = (0..100)
            .map(|i| format!(r#"{{"status":"failed","fullName":"t{i}"}}"#))
            .collect();
        let json = format!(
            r#"{{"numTotalTests":100,"numFailedTests":100,
                 "testResults":[{{"assertionResults":[{}]}}]}}"#,
            assertions.join(",")
        );
        let s = parse_jest_json(&json);
        assert_eq!(s.failed, 100);
        assert_eq!(s.failures.len(), MAX_FAILURES_KEPT);
    }
}
