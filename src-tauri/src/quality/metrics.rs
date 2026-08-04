//! Per-function complexity, so the codebase's direction is measurable.
//!
//! The other four layers judge a change. This one judges the *codebase*: agents
//! produce code faster than anyone reads it, so decay is silent and only
//! visible in aggregate — nobody notices the function that grew a branch a week
//! for two months.
//!
//! Two honest caveats, stated up front because a metric that overclaims is
//! worse than no metric:
//!
//! 1. This is a **consistent heuristic, not canonical McCabe.** Decision points
//!    are recognised by tree-sitter node kind across 77 grammars rather than by
//!    a hand-written rule per language, so the absolute number may differ from
//!    another tool's. That is fine for what it is used for — comparing a
//!    function against itself over time, and against a budget the user picks —
//!    because the same rule is applied every time.
//! 2. It is therefore **not enforced by default.** It reports.

use serde::{Deserialize, Serialize};

use crate::code_intel::parser;

/// One function's shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionMetric {
    /// Workspace-relative path, as given to [`analyze`].
    pub file: String,
    pub name: String,
    /// 1-based line of the declaration.
    pub line: usize,
    /// Decision points + 1.
    pub complexity: u32,
    /// Lines spanned by the function, declaration to close.
    pub loc: u32,
}

/// Node kinds that declare a function or method, across grammars.
///
/// Excludes type positions (`function_type`) and call sites, which name a
/// function without being one.
fn is_function_kind(kind: &str) -> bool {
    if kind.contains("_type") || kind.contains("call") || kind.contains("parameter") {
        return false;
    }
    kind.contains("function") || kind.contains("method") || kind.contains("constructor")
}

/// Node kinds that add a branch.
///
/// Matched by kind rather than by language rule, which is what lets one
/// implementation cover every grammar the indexer already carries. Operator
/// tokens (`&&`, `||`) are node kinds in their own right in tree-sitter, so
/// short-circuit branching is counted too.
fn is_decision(kind: &str) -> bool {
    const EXACT: &[&str] = &["&&", "||", "?", "and", "or"];
    if EXACT.contains(&kind) {
        return true;
    }
    const FRAGMENTS: &[&str] = &[
        "if_statement",
        "if_expression",
        "if_clause",
        "elif",
        "else_if",
        "while_",
        "for_statement",
        "for_expression",
        "for_in",
        "foreach",
        "loop_expression",
        "case",
        "when_",
        "match_arm",
        "catch",
        "except_clause",
        "rescue",
        "conditional_expression",
        "ternary",
        "guard",
    ];
    FRAGMENTS.iter().any(|f| kind.contains(f))
}

/// Complexity and size of every function and method in one file.
///
/// Returns empty for a language the indexer cannot parse, which is the honest
/// answer rather than a fabricated zero — the caller reports how many files
/// were measured so the gap is visible.
pub fn analyze(path: &str, content: &str) -> Vec<FunctionMetric> {
    let Some(lang_name) = parser::detect_language(path) else {
        return Vec::new();
    };
    let Ok(language) = parser::get_language(lang_name) else {
        return Vec::new();
    };
    // Reuse the indexer's symbol extraction for *what* the functions are; this
    // module only adds the branch counting. `kind` is the raw tree-sitter node
    // kind, which differs per grammar (function_item, function_declaration,
    // function_definition, method_definition, …), and `start_line` is 1-based.
    let mut symbols: Vec<_> = parser::parse_file(path, content)
        .symbols
        .into_iter()
        .filter(|s| is_function_kind(&s.kind))
        .collect();
    // Some grammars report one function through two nodes (a TS arrow function
    // is both a lexical_declaration and a function_declaration on the same
    // line); counting it twice would inflate every average.
    symbols.sort_by_key(|s| (s.start_line, s.end_line, s.name.clone()));
    symbols.dedup_by(|a, b| {
        a.name == b.name && a.start_line == b.start_line && a.end_line == b.end_line
    });
    if symbols.is_empty() {
        return Vec::new();
    }

    let mut ts = tree_sitter::Parser::new();
    if ts.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = ts.parse(content, None) else {
        return Vec::new();
    };

    // One walk over the file; each decision point is attributed to the
    // innermost function whose line range contains it, so a closure inside a
    // function does not double-count into both.
    let mut counts: Vec<u32> = vec![0; symbols.len()];
    let mut cursor = tree.walk();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if is_decision(node.kind()) {
            // tree-sitter rows are 0-based; the indexer's lines are 1-based.
            let line = node.start_position().row as i64 + 1;
            // Innermost = the smallest containing range.
            let mut best: Option<(usize, i64)> = None;
            for (idx, sym) in symbols.iter().enumerate() {
                if line >= sym.start_line && line <= sym.end_line {
                    let span = sym.end_line - sym.start_line;
                    if best.is_none_or(|(_, b)| span < b) {
                        best = Some((idx, span));
                    }
                }
            }
            if let Some((idx, _)) = best {
                counts[idx] += 1;
            }
        }
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    symbols
        .into_iter()
        .zip(counts)
        .map(|(sym, decisions)| FunctionMetric {
            file: path.to_string(),
            name: sym.name,
            line: sym.start_line as usize,
            complexity: decisions + 1,
            loc: (sym.end_line - sym.start_line + 1).max(1) as u32,
        })
        .collect()
}

/// Aggregate shape of a set of functions, for the report and the trend.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MetricsSummary {
    pub functions: u32,
    pub max_complexity: u32,
    pub total_complexity: u32,
    /// Functions over the configured budget, worst first.
    pub over_budget: Vec<FunctionMetric>,
}

impl MetricsSummary {
    pub fn mean_complexity(&self) -> f64 {
        if self.functions == 0 {
            return 0.0;
        }
        self.total_complexity as f64 / self.functions as f64
    }

    pub fn headline(&self) -> String {
        if self.functions == 0 {
            return "no functions measured in the changed files".into();
        }
        format!(
            "{} function(s), mean complexity {:.1}, worst {}",
            self.functions,
            self.mean_complexity(),
            self.max_complexity
        )
    }
}

/// How many worst offenders to name. Enough to act on in one sitting.
const MAX_OVER_BUDGET_REPORTED: usize = 10;

/// Score a set of functions against a complexity budget.
pub fn summarize(metrics: &[FunctionMetric], budget: Option<u32>) -> MetricsSummary {
    let mut summary = MetricsSummary {
        functions: metrics.len() as u32,
        max_complexity: metrics.iter().map(|m| m.complexity).max().unwrap_or(0),
        total_complexity: metrics.iter().map(|m| m.complexity).sum(),
        over_budget: Vec::new(),
    };
    if let Some(budget) = budget {
        let mut over: Vec<FunctionMetric> = metrics
            .iter()
            .filter(|m| m.complexity > budget)
            .cloned()
            .collect();
        // Worst first: the biggest offender is where the reader should look.
        over.sort_by_key(|f| std::cmp::Reverse(f.complexity));
        over.truncate(MAX_OVER_BUDGET_REPORTED);
        summary.over_budget = over;
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complexity_of(path: &str, src: &str, name: &str) -> u32 {
        analyze(path, src)
            .into_iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("no function {name} in {path}"))
            .complexity
    }

    #[test]
    fn a_straight_line_function_scores_one() {
        let src = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
        assert_eq!(complexity_of("a.rs", src, "add"), 1);
    }

    #[test]
    fn each_branch_adds_one() {
        let src = "\
fn classify(n: i32) -> &'static str {
    if n < 0 {
        \"negative\"
    } else if n == 0 {
        \"zero\"
    } else {
        \"positive\"
    }
}
";
        // Two `if`s over the straight-line baseline.
        assert_eq!(complexity_of("a.rs", src, "classify"), 3);
    }

    #[test]
    fn short_circuit_operators_count_as_branches() {
        // They are real paths through the function, and they are how a
        // condition quietly grows without adding an `if`.
        let simple = "fn ok(a: bool, b: bool) -> bool {\n    a\n}\n";
        let compound = "fn ok(a: bool, b: bool) -> bool {\n    a && b\n}\n";
        assert_eq!(complexity_of("a.rs", simple, "ok"), 1);
        assert_eq!(complexity_of("a.rs", compound, "ok"), 2);
    }

    #[test]
    fn loops_and_match_arms_count() {
        let src = "\
fn scan(items: &[i32]) -> i32 {
    let mut total = 0;
    for item in items {
        match item {
            0 => total += 1,
            _ => total += 2,
        }
    }
    total
}
";
        let c = complexity_of("a.rs", src, "scan");
        assert!(c >= 4, "for + two match arms over baseline, got {c}");
    }

    #[test]
    fn it_works_across_languages_not_just_rust() {
        // The whole point of matching on node kind: one implementation, every
        // grammar the indexer already carries.
        let ts = "\
export function classify(n: number): string {
  if (n < 0) {
    return 'negative';
  }
  for (const x of []) {
    console.log(x);
  }
  return 'other';
}
";
        assert_eq!(complexity_of("a.ts", ts, "classify"), 3);

        let py = "\
def classify(n):
    if n < 0:
        return 'negative'
    while n > 10:
        n -= 1
    return 'other'
";
        assert_eq!(complexity_of("a.py", py, "classify"), 3);
    }

    #[test]
    fn a_nested_closure_does_not_double_count_into_its_parent() {
        let src = "\
fn outer(items: &[i32]) -> Vec<i32> {
    items.iter().map(|x| if *x > 0 { *x } else { 0 }).collect()
}
";
        let metrics = analyze("a.rs", src);
        let outer = metrics.iter().find(|m| m.name == "outer").unwrap();
        // The `if` belongs to the innermost function containing it. When the
        // closure is not itself reported as a function, it lands on `outer` —
        // either way it is counted exactly once.
        assert_eq!(
            metrics.iter().map(|m| m.complexity - 1).sum::<u32>(),
            1,
            "one decision point total: {metrics:?}"
        );
        assert!(outer.complexity >= 1);
    }

    #[test]
    fn functions_carry_their_file_line_and_size() {
        let src = "fn a() {}\n\nfn b() {\n    let _ = 1;\n}\n";
        let metrics = analyze("src/lib.rs", src);
        let b = metrics.iter().find(|m| m.name == "b").unwrap();
        assert_eq!(b.file, "src/lib.rs");
        assert_eq!(b.line, 3, "1-based, so it points where a human would look");
        assert_eq!(b.loc, 3);
    }

    #[test]
    fn an_unparseable_language_measures_nothing_rather_than_zero() {
        // A fabricated zero would read as "perfectly simple".
        assert!(analyze("notes.unknownext", "whatever").is_empty());
        assert!(analyze("a.rs", "").is_empty());
    }

    #[test]
    fn summarize_reports_the_worst_offenders_first() {
        let m = |name: &str, complexity: u32| FunctionMetric {
            file: "a.rs".into(),
            name: name.into(),
            line: 1,
            complexity,
            loc: 10,
        };
        let metrics = vec![m("small", 2), m("huge", 30), m("medium", 12)];
        let summary = summarize(&metrics, Some(10));
        assert_eq!(summary.functions, 3);
        assert_eq!(summary.max_complexity, 30);
        assert_eq!(summary.mean_complexity(), 44.0 / 3.0);
        let names: Vec<&str> = summary
            .over_budget
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(names, vec!["huge", "medium"]);
    }

    #[test]
    fn without_a_budget_nothing_is_over_it() {
        let metrics = vec![FunctionMetric {
            file: "a.rs".into(),
            name: "huge".into(),
            line: 1,
            complexity: 99,
            loc: 10,
        }];
        assert!(summarize(&metrics, None).over_budget.is_empty());
    }

    #[test]
    fn an_empty_set_reports_that_nothing_was_measured() {
        let s = summarize(&[], Some(10));
        assert_eq!(s.mean_complexity(), 0.0);
        assert!(s.headline().contains("no functions measured"));
    }
}
