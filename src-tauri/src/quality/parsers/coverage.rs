//! lcov parsing and diff coverage.
//!
//! lcov is the one format every coverage tool we care about can emit —
//! `cargo llvm-cov --lcov`, vitest's v8 provider, jest's istanbul — so the
//! harness speaks it and stays stack-agnostic.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::quality::diff::ChangedLines;

/// Executable lines and their hit counts, per file. Only lines the coverage
/// tool considers executable appear here — which is what makes the diff
/// denominator meaningful: a changed blank line or comment is not "uncovered",
/// it is uncoverable.
#[derive(Debug, Clone, Default)]
pub struct LcovData {
    pub files: HashMap<PathBuf, HashMap<u32, u64>>,
}

/// The result of scoring changed lines against coverage data.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CoverageSummary {
    /// Changed lines that are executable and were executed.
    pub covered: u32,
    /// Changed lines that are executable (covered + uncovered).
    pub total: u32,
    /// Up to a handful of "path:line" strings for the uncovered ones.
    pub uncovered_samples: Vec<String>,
    /// How many distinct files contributed uncovered lines.
    pub uncovered_files: u32,
}

const MAX_UNCOVERED_SAMPLES: usize = 25;

impl CoverageSummary {
    /// Percentage of changed executable lines that ran.
    ///
    /// A change touching no executable line scores 100: there was nothing a
    /// test could have covered, and failing that case would block docs-only or
    /// config-only work for no benefit.
    pub fn pct(&self) -> f64 {
        if self.total == 0 {
            return 100.0;
        }
        (self.covered as f64 / self.total as f64) * 100.0
    }

    pub fn headline(&self) -> String {
        if self.total == 0 {
            return "no changed executable lines to cover".into();
        }
        format!(
            "{:.1}% of changed lines covered ({}/{})",
            self.pct(),
            self.covered,
            self.total
        )
    }
}

/// Parse an lcov report. `root` resolves the relative `SF:` paths some tools
/// emit; absolute ones are used as-is.
pub fn parse_lcov(text: &str, root: &Path) -> LcovData {
    let mut data = LcovData::default();
    let mut current: Option<PathBuf> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(path) = line.strip_prefix("SF:") {
            let p = Path::new(path);
            current = Some(if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(p)
            });
        } else if let Some(rest) = line.strip_prefix("DA:") {
            let Some(ref file) = current else { continue };
            let mut parts = rest.split(',');
            let (Some(line_no), Some(hits)) = (parts.next(), parts.next()) else {
                continue;
            };
            let (Ok(line_no), Ok(hits)) = (line_no.trim().parse::<u32>(), parse_hits(hits)) else {
                continue;
            };
            // A line can appear more than once (several branches/functions map
            // to it); the line ran if any record says so.
            data.files
                .entry(file.clone())
                .or_default()
                .entry(line_no)
                .and_modify(|h| *h = (*h).max(hits))
                .or_insert(hits);
        } else if line == "end_of_record" {
            current = None;
        }
    }
    data
}

/// lcov writes plain integers, but some producers emit a trailing checksum
/// field and gcov can write `-` for "no data".
fn parse_hits(raw: &str) -> Result<u64, std::num::ParseIntError> {
    let first = raw.split(',').next().unwrap_or(raw).trim();
    if first == "-" {
        return Ok(0);
    }
    first.parse::<u64>()
}

/// Score the changed lines against the coverage data.
///
/// Files absent from the report contribute nothing: a changed `.md` or a
/// config file has no coverage story, and counting it as uncovered would make
/// the gate meaningless.
pub fn diff_coverage(lcov: &LcovData, changed: &ChangedLines) -> CoverageSummary {
    let mut summary = CoverageSummary::default();
    // Sorted so the samples shown to the model are stable between runs.
    let mut paths: Vec<&PathBuf> = changed.keys().collect();
    paths.sort();
    for path in paths {
        let Some(file_cov) = lookup(lcov, path) else {
            continue;
        };
        let mut lines: Vec<u32> = changed[path].iter().copied().collect();
        lines.sort_unstable();
        let mut file_had_uncovered = false;
        for line in lines {
            let Some(hits) = file_cov.get(&line) else {
                continue; // not an executable line
            };
            summary.total += 1;
            if *hits > 0 {
                summary.covered += 1;
            } else {
                file_had_uncovered = true;
                if summary.uncovered_samples.len() < MAX_UNCOVERED_SAMPLES {
                    summary
                        .uncovered_samples
                        .push(format!("{}:{}", path.display(), line));
                }
            }
        }
        if file_had_uncovered {
            summary.uncovered_files += 1;
        }
    }
    summary
}

/// Match a changed file against the report, tolerating the path shape
/// differences between tools (absolute vs relative to a package root).
fn lookup<'a>(lcov: &'a LcovData, path: &Path) -> Option<&'a HashMap<u32, u64>> {
    if let Some(found) = lcov.files.get(path) {
        return Some(found);
    }
    // Fall back to a suffix match: vitest reports paths relative to the
    // package, git reports them relative to the repo root, and in a monorepo
    // those differ by a prefix.
    lcov.files
        .iter()
        .find(|(known, _)| known.ends_with(path) || path.ends_with(known.as_path()))
        .map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const LCOV: &str = "\
SF:/repo/src/a.rs
DA:1,5
DA:2,0
DA:5,1
LF:3
LH:2
end_of_record
SF:/repo/src/b.ts
DA:10,0
end_of_record
";

    fn changed(entries: &[(&str, &[u32])]) -> ChangedLines {
        entries
            .iter()
            .map(|(p, lines)| {
                (
                    PathBuf::from(*p),
                    lines.iter().copied().collect::<HashSet<u32>>(),
                )
            })
            .collect()
    }

    #[test]
    fn parses_files_and_hit_counts() {
        let d = parse_lcov(LCOV, Path::new("/repo"));
        assert_eq!(d.files.len(), 2);
        let a = &d.files[&PathBuf::from("/repo/src/a.rs")];
        assert_eq!(a[&1], 5);
        assert_eq!(a[&2], 0);
    }

    #[test]
    fn relative_paths_resolve_against_the_root() {
        let d = parse_lcov("SF:src/x.ts\nDA:1,1\nend_of_record\n", Path::new("/pkg"));
        assert!(d.files.contains_key(&PathBuf::from("/pkg/src/x.ts")));
    }

    #[test]
    fn repeated_line_records_keep_the_highest_hit_count() {
        // Several branches map to one line; if any ran, the line ran.
        let d = parse_lcov(
            "SF:/r/a.rs\nDA:7,0\nDA:7,3\nend_of_record\n",
            Path::new("/r"),
        );
        assert_eq!(d.files[&PathBuf::from("/r/a.rs")][&7], 3);
    }

    #[test]
    fn scores_only_changed_executable_lines() {
        let d = parse_lcov(LCOV, Path::new("/repo"));
        // Line 3 is not executable, so it belongs in neither side of the ratio.
        let c = changed(&[("/repo/src/a.rs", &[1, 2, 3])]);
        let s = diff_coverage(&d, &c);
        assert_eq!((s.covered, s.total), (1, 2));
        assert_eq!(s.uncovered_samples, vec!["/repo/src/a.rs:2"]);
        assert_eq!(s.pct(), 50.0);
    }

    #[test]
    fn files_missing_from_the_report_are_ignored() {
        // A changed README must not be reported as uncovered code.
        let d = parse_lcov(LCOV, Path::new("/repo"));
        let c = changed(&[("/repo/README.md", &[1, 2, 3])]);
        let s = diff_coverage(&d, &c);
        assert_eq!(s.total, 0);
        assert_eq!(s.pct(), 100.0, "nothing to cover cannot be a failure");
    }

    #[test]
    fn a_change_with_no_executable_lines_scores_full() {
        let s = CoverageSummary::default();
        assert_eq!(s.pct(), 100.0);
        assert!(s.headline().contains("no changed executable lines"));
    }

    #[test]
    fn fully_uncovered_change_scores_zero() {
        let d = parse_lcov(LCOV, Path::new("/repo"));
        let c = changed(&[("/repo/src/b.ts", &[10])]);
        let s = diff_coverage(&d, &c);
        assert_eq!((s.covered, s.total), (0, 1));
        assert_eq!(s.pct(), 0.0);
        assert_eq!(s.uncovered_files, 1);
    }

    #[test]
    fn suffix_matching_bridges_monorepo_path_shapes() {
        // git says "src-tauri/src/a.rs" from the repo root; the tool reported
        // an absolute path from the crate root.
        let d = parse_lcov(
            "SF:/repo/src-tauri/src/a.rs\nDA:1,1\nend_of_record\n",
            Path::new("/x"),
        );
        let c = changed(&[("src-tauri/src/a.rs", &[1])]);
        let s = diff_coverage(&d, &c);
        assert_eq!((s.covered, s.total), (1, 1));
    }

    #[test]
    fn gcov_dash_hit_count_reads_as_zero() {
        let d = parse_lcov("SF:/r/a.rs\nDA:4,-\nend_of_record\n", Path::new("/r"));
        assert_eq!(d.files[&PathBuf::from("/r/a.rs")][&4], 0);
    }

    #[test]
    fn uncovered_samples_are_bounded() {
        let mut lcov_text = String::from("SF:/r/big.rs\n");
        for i in 1..=200 {
            lcov_text.push_str(&format!("DA:{i},0\n"));
        }
        lcov_text.push_str("end_of_record\n");
        let d = parse_lcov(&lcov_text, Path::new("/r"));
        let lines: Vec<u32> = (1..=200).collect();
        let c = changed(&[("/r/big.rs", &lines)]);
        let s = diff_coverage(&d, &c);
        assert_eq!(s.total, 200);
        assert_eq!(s.uncovered_samples.len(), MAX_UNCOVERED_SAMPLES);
    }
}
