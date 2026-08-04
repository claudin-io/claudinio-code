//! Gherkin feature parsing.
//!
//! Specs are the one part of the system that does **not** come out of the
//! model's head. Tests, coverage and mutation all ask "is this code sound?" —
//! only the spec asks "is this the thing we were asked to build?", which is the
//! failure nothing else catches: an implementation that is flawless, fully
//! tested, and solves the wrong problem.
//!
//! This is a deliberate subset. The harness never *executes* scenarios — a real
//! BDD runner does that — so it needs names, tags and steps for indexing and
//! reporting, not full Gherkin fidelity. Keeping it here as a pure function
//! over text, rather than pulling in a parser crate, matches the rest of the
//! module and keeps the surface testable.

use serde::{Deserialize, Serialize};

/// Keywords that begin a step. `And`/`But` continue whatever came before.
const STEP_KEYWORDS: &[&str] = &["Given ", "When ", "Then ", "And ", "But ", "* "];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    pub name: String,
    /// 1-based line of the `Scenario:` keyword, for pointing the user at it.
    pub line: usize,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub steps: Vec<String>,
    /// `Scenario Outline:` — one scenario standing for a table of cases.
    #[serde(default)]
    pub outline: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub name: String,
    /// Path as given to the parser, for reporting.
    pub path: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub scenarios: Vec<Scenario>,
}

/// Parse one `.feature` file.
///
/// Never fails: a malformed feature yields whatever could be read. Refusing to
/// parse would make a typo in a spec silently remove it from the index, which
/// is the opposite of what a spec is for.
pub fn parse_feature(path: &str, text: &str) -> Feature {
    let mut feature = Feature {
        name: String::new(),
        path: path.to_string(),
        tags: Vec::new(),
        scenarios: Vec::new(),
    };
    // Tags attach to whatever declaration comes next, so they are collected
    // ahead of it and flushed on use.
    let mut pending_tags: Vec<String> = Vec::new();
    let mut in_scenario = false;

    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('@') {
            pending_tags.extend(
                line.split_whitespace()
                    .filter(|t| t.starts_with('@'))
                    .map(|t| t.trim_start_matches('@').to_string()),
            );
            continue;
        }
        if let Some(name) = after_keyword(line, "Feature:") {
            feature.name = name;
            feature.tags = std::mem::take(&mut pending_tags);
            in_scenario = false;
            continue;
        }
        // "Scenario Outline:" must be tried first — "Scenario:" is its prefix.
        let scenario = after_keyword(line, "Scenario Outline:")
            .or_else(|| after_keyword(line, "Scenario Template:"))
            .map(|n| (n, true))
            .or_else(|| {
                after_keyword(line, "Scenario:")
                    .or_else(|| after_keyword(line, "Example:"))
                    .map(|n| (n, false))
            });
        if let Some((name, outline)) = scenario {
            feature.scenarios.push(Scenario {
                name,
                line: idx + 1,
                tags: std::mem::take(&mut pending_tags),
                steps: Vec::new(),
                outline,
            });
            in_scenario = true;
            continue;
        }
        // Background steps belong to every scenario, so they are not attached
        // to any one of them; the runner applies them.
        if after_keyword(line, "Background:").is_some()
            || after_keyword(line, "Rule:").is_some()
            || after_keyword(line, "Examples:").is_some()
            || after_keyword(line, "Scenarios:").is_some()
        {
            in_scenario = false;
            continue;
        }
        if in_scenario
            && STEP_KEYWORDS.iter().any(|k| line.starts_with(k))
            && let Some(last) = feature.scenarios.last_mut()
        {
            last.steps.push(line.to_string());
        }
    }
    feature
}

/// The text after `keyword`, when the line starts with it.
fn after_keyword(line: &str, keyword: &str) -> Option<String> {
    line.strip_prefix(keyword)
        .map(|rest| rest.trim().to_string())
}

/// Total scenarios across a set of features.
pub fn total_scenarios(features: &[Feature]) -> usize {
    features.iter().map(|f| f.scenarios.len()).sum()
}

/// A compact index of every scenario, for the planning prompt.
///
/// This is what makes the spec an *input* rather than decoration: the planner
/// sees the scenarios it has to satisfy before it designs anything.
pub fn scenario_index(features: &[Feature], max_chars: usize) -> String {
    let mut out = String::new();
    for feature in features {
        if feature.scenarios.is_empty() {
            continue;
        }
        out.push_str(&format!("- {} ({})\n", feature.name, feature.path));
        for scenario in &feature.scenarios {
            out.push_str(&format!(
                "  - {}{}\n",
                scenario.name,
                if scenario.outline { " [outline]" } else { "" }
            ));
        }
    }
    super::super::truncate_chars(&out, max_chars)
}

#[cfg(test)]
mod gherkin_tests {
    use super::*;

    const FEATURE: &str = r#"
# a comment, ignored
@billing @slow
Feature: Member discounts
  As a shop owner
  I want members to get a discount

  Background:
    Given the shop is open

  @happy
  Scenario: A member gets ten percent off
    Given a member with a basket of 100
    When the total is calculated
    Then the total is 90

  Scenario Outline: Discounts by tier
    Given a <tier> member
    Then the discount is <pct>

    Examples:
      | tier   | pct |
      | gold   | 20  |

  Scenario: A guest pays full price
    Given a guest with a basket of 100
    Then the total is 100
"#;

    #[test]
    fn reads_the_feature_name_and_tags() {
        let f = parse_feature("features/discount.feature", FEATURE);
        assert_eq!(f.name, "Member discounts");
        assert_eq!(f.tags, vec!["billing", "slow"]);
        assert_eq!(f.path, "features/discount.feature");
    }

    #[test]
    fn reads_every_scenario_in_order() {
        let f = parse_feature("x.feature", FEATURE);
        let names: Vec<&str> = f.scenarios.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "A member gets ten percent off",
                "Discounts by tier",
                "A guest pays full price"
            ]
        );
    }

    #[test]
    fn an_outline_is_marked_as_one() {
        // "Scenario:" is a prefix of "Scenario Outline:", so getting this
        // wrong silently mislabels every outline in the project.
        let f = parse_feature("x.feature", FEATURE);
        assert!(!f.scenarios[0].outline);
        assert!(
            f.scenarios[1].outline,
            "outline must not parse as a plain scenario"
        );
        assert_eq!(f.scenarios[1].name, "Discounts by tier");
    }

    #[test]
    fn tags_attach_to_the_scenario_that_follows_them() {
        let f = parse_feature("x.feature", FEATURE);
        assert_eq!(f.scenarios[0].tags, vec!["happy"]);
        assert!(f.scenarios[1].tags.is_empty(), "tags must not leak forward");
    }

    #[test]
    fn steps_are_captured_but_background_and_examples_are_not() {
        let f = parse_feature("x.feature", FEATURE);
        assert_eq!(f.scenarios[0].steps.len(), 3);
        assert!(f.scenarios[0].steps[0].starts_with("Given a member"));
        // "Given the shop is open" is Background — it belongs to the runner,
        // not to any one scenario.
        assert!(
            !f.scenarios[0]
                .steps
                .iter()
                .any(|s| s.contains("shop is open")),
            "{:?}",
            f.scenarios[0].steps
        );
        // The Examples table rows are not steps.
        assert!(f.scenarios[1].steps.iter().all(|s| !s.contains('|')));
    }

    #[test]
    fn scenarios_carry_their_line_number() {
        let f = parse_feature("x.feature", FEATURE);
        let line = f.scenarios[0].line;
        assert_eq!(
            FEATURE.lines().nth(line - 1).unwrap().trim(),
            "Scenario: A member gets ten percent off"
        );
    }

    #[test]
    fn a_file_with_no_scenarios_parses_to_an_empty_list() {
        let f = parse_feature("x.feature", "Feature: Just a title\n");
        assert_eq!(f.name, "Just a title");
        assert!(f.scenarios.is_empty());
    }

    #[test]
    fn a_malformed_file_yields_what_could_be_read_rather_than_nothing() {
        // A typo in a spec must not silently remove it from the index.
        let f = parse_feature(
            "x.feature",
            "Scenario: orphan with no feature header\n  Then it still counts\n",
        );
        assert_eq!(f.name, "");
        assert_eq!(f.scenarios.len(), 1);
        assert_eq!(f.scenarios[0].steps.len(), 1);
    }

    #[test]
    fn counts_and_indexes_across_files() {
        let a = parse_feature("a.feature", FEATURE);
        let b = parse_feature(
            "b.feature",
            "Feature: Refunds\n  Scenario: Late refund\n    Then it is refused\n",
        );
        assert_eq!(total_scenarios(&[a.clone(), b.clone()]), 4);

        let index = scenario_index(&[a, b], 4_000);
        assert!(index.contains("Member discounts (a.feature)"));
        assert!(index.contains("A guest pays full price"));
        assert!(index.contains("Discounts by tier [outline]"));
        assert!(index.contains("Refunds (b.feature)"));
    }

    #[test]
    fn the_index_is_bounded() {
        let many: Vec<Feature> = (0..500)
            .map(|i| parse_feature(&format!("f{i}.feature"), FEATURE))
            .collect();
        let index = scenario_index(&many, 500);
        assert!(index.len() < 700, "prompt budget must stay bounded");
        assert!(index.contains("truncated"));
    }
}
