//! What each model actually costs on this machine.
//!
//! Numbers published for a model describe someone else's hardware. The useful
//! comparison is between two models on the machine in front of you, and the
//! three costs that decide whether a model is pleasant to work with are not the
//! same number: how long it takes to load, how long before the first token
//! appears, and how fast it then generates. A model can win on one and lose
//! badly on another — a 30B MoE loads slowly and then generates faster than a
//! 7B dense.
//!
//! Stored as running averages rather than a log: the question is "which of my
//! models is quicker", not "what happened at 14:32".

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelBenchmark {
    pub model_key: String,
    /// Cold start: process spawn until the server reports ready. Dominated by
    /// reading the weights off disk, so the first run after a reboot is much
    /// slower than the next.
    pub load_seconds: f64,
    pub load_samples: u32,
    /// Request sent until the first token comes back. The number that decides
    /// whether the app feels stuck.
    pub first_token_seconds: f64,
    /// Sustained rate once tokens are flowing.
    pub tokens_per_second: f64,
    pub prompt_tokens_per_second: f64,
    pub generation_samples: u32,
    /// Prompt size of the last measured run, because time-to-first-token is
    /// meaningless without it — 200 tokens and 100k tokens are different
    /// questions.
    pub last_prompt_tokens: u32,
    pub last_run_at: String,
}

impl ModelBenchmark {
    /// Fold a new sample into the running average.
    ///
    /// A plain mean, not a decayed one: these are measurements of the same
    /// hardware doing the same thing, so an old sample is as valid as a new
    /// one — until the model or machine changes, which resets the file anyway.
    fn fold(current: f64, samples: u32, sample: f64) -> f64 {
        if samples == 0 {
            return sample;
        }
        (current * samples as f64 + sample) / (samples as f64 + 1.0)
    }

    pub fn record_load(&mut self, seconds: f64) {
        self.load_seconds = Self::fold(self.load_seconds, self.load_samples, seconds);
        self.load_samples += 1;
    }

    pub fn record_generation(
        &mut self,
        first_token_seconds: f64,
        tokens_per_second: f64,
        prompt_tokens_per_second: f64,
        prompt_tokens: u32,
    ) {
        let n = self.generation_samples;
        self.first_token_seconds = Self::fold(self.first_token_seconds, n, first_token_seconds);
        self.tokens_per_second = Self::fold(self.tokens_per_second, n, tokens_per_second);
        self.prompt_tokens_per_second =
            Self::fold(self.prompt_tokens_per_second, n, prompt_tokens_per_second);
        self.generation_samples = n + 1;
        self.last_prompt_tokens = prompt_tokens;
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchmarkStore {
    #[serde(default)]
    pub entries: HashMap<String, ModelBenchmark>,
}

fn store_path() -> Result<PathBuf, String> {
    Ok(super::catalog::catalog_dir()?.join("benchmarks.json"))
}

pub fn load() -> BenchmarkStore {
    store_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Apply `edit` to one model's entry and persist.
///
/// Failures are swallowed: a benchmark is an observation about the run, and
/// losing one must never fail the request that produced it.
pub fn update(model_key: &str, edit: impl FnOnce(&mut ModelBenchmark)) {
    let mut store = load();
    let entry = store.entries.entry(model_key.to_string()).or_default();
    entry.model_key = model_key.to_string();
    edit(entry);
    entry.last_run_at = chrono::Utc::now().to_rfc3339();

    let Ok(path) = store_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(&store) {
        let _ = std::fs::write(path, text);
    }
}

pub fn forget(model_key: &str) {
    let mut store = load();
    if store.entries.remove(model_key).is_none() {
        return;
    }
    let Ok(path) = store_path() else { return };
    if let Ok(text) = serde_json::to_string_pretty(&store) {
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_sample_is_the_average() {
        let mut b = ModelBenchmark::default();
        b.record_load(12.0);
        assert_eq!(b.load_seconds, 12.0);
        assert_eq!(b.load_samples, 1);
    }

    #[test]
    fn later_samples_average_in() {
        let mut b = ModelBenchmark::default();
        b.record_load(10.0);
        b.record_load(20.0);
        assert_eq!(b.load_seconds, 15.0);
        assert_eq!(b.load_samples, 2);
    }

    /// The three costs move independently: a model can load slowly and then
    /// generate quickly, which is exactly what a mixture-of-experts does.
    #[test]
    fn generation_and_load_are_tracked_apart() {
        let mut b = ModelBenchmark::default();
        b.record_load(40.0);
        b.record_generation(2.0, 60.0, 900.0, 1200);
        assert_eq!(b.load_seconds, 40.0);
        assert_eq!(b.load_samples, 1);
        assert_eq!(b.first_token_seconds, 2.0);
        assert_eq!(b.tokens_per_second, 60.0);
        assert_eq!(b.generation_samples, 1);
        // Time-to-first-token means nothing without the prompt it measured.
        assert_eq!(b.last_prompt_tokens, 1200);
    }

    #[test]
    fn a_store_round_trips() {
        let mut store = BenchmarkStore::default();
        let mut b = ModelBenchmark::default();
        b.record_generation(1.5, 30.0, 400.0, 800);
        store.entries.insert("abc".into(), b);
        let text = serde_json::to_string(&store).unwrap();
        let back: BenchmarkStore = serde_json::from_str(&text).unwrap();
        assert_eq!(back.entries["abc"].tokens_per_second, 30.0);
    }

    #[test]
    fn an_old_store_without_entries_still_loads() {
        let store: BenchmarkStore = serde_json::from_str("{}").unwrap();
        assert!(store.entries.is_empty());
    }
}
