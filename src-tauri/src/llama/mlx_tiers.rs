//! RAM-tier recommendations for the MLX engine.
//!
//! MLX only runs on Apple Silicon, where memory is unified, so a whole-machine
//! RAM tier is a good enough proxy for what fits: pick the largest tier whose
//! `min_ram_gb` the machine clears, and everything at or below it is fair game.
//!
//! Every repo below was verified live on the Hugging Face API during planning.

use crate::llama::hardware::HardwareProfile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MlxTierModel {
    pub repo: &'static str,
    pub display_name: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MlxTier {
    pub min_ram_gb: u32,
    pub label: &'static str,
    pub note: &'static str,
    pub models: &'static [MlxTierModel],
}

pub const TIERS: &[MlxTier] = &[
    MlxTier {
        min_ram_gb: 8,
        label: "8GB",
        note: "Small but capable",
        models: &[
            MlxTierModel {
                repo: "mlx-community/Qwen3.5-4B-MLX-4bit",
                display_name: "Qwen3.5-4B-4bit",
            },
            MlxTierModel {
                repo: "mlx-community/Phi-4-mini-instruct-4bit",
                display_name: "Phi-4-mini-4bit",
            },
        ],
    },
    MlxTier {
        min_ram_gb: 16,
        label: "16GB",
        note: "Great balance",
        models: &[
            MlxTierModel {
                repo: "mlx-community/Qwen3.5-9B-MLX-4bit",
                display_name: "Qwen3.5-9B-4bit",
            },
            MlxTierModel {
                repo: "mlx-community/gemma-3-12b-it-qat-4bit",
                display_name: "Gemma-3-12B-4bit",
            },
        ],
    },
    MlxTier {
        min_ram_gb: 32,
        label: "32GB",
        note: "Strong coding + reasoning",
        models: &[
            MlxTierModel {
                repo: "mlx-community/Qwen3.5-27B-4bit",
                display_name: "Qwen3.5-27B-4bit",
            },
            MlxTierModel {
                repo: "mlx-community/Devstral-Small-2-24B-Instruct-2512-OptiQ-4bit",
                display_name: "Devstral-24B-4bit",
            },
        ],
    },
    MlxTier {
        min_ram_gb: 64,
        label: "64GB+",
        note: "Near-frontier quality",
        models: &[
            MlxTierModel {
                repo: "mlx-community/Qwen3.5-122B-A10B-4bit",
                display_name: "Qwen3.5-122B-A10B-4bit",
            },
            MlxTierModel {
                repo: "mlx-community/meta-llama-Llama-4-Scout-17B-16E-4bit",
                display_name: "Llama-4-Scout-4bit",
            },
        ],
    },
];

/// The tiers this machine qualifies for, smallest first. A machine too small
/// for even the lowest tier still gets that tier — better a tight fit than an
/// empty list.
pub fn for_machine(hw: &HardwareProfile) -> Vec<&'static MlxTier> {
    let ram_gb = (hw.total_ram_bytes / 1_000_000_000) as u32;
    let qualifying: Vec<&'static MlxTier> =
        TIERS.iter().filter(|t| t.min_ram_gb <= ram_gb).collect();
    if qualifying.is_empty() {
        vec![&TIERS[0]]
    } else {
        qualifying
    }
}

/// Index into [`TIERS`] of the largest tier this machine qualifies for,
/// clamped to index 0 when it qualifies for none and to the last index when
/// it qualifies for all.
pub fn current_tier_index(hw: &HardwareProfile) -> usize {
    let ram_gb = (hw.total_ram_bytes / 1_000_000_000) as u32;
    TIERS
        .iter()
        .rposition(|t| t.min_ram_gb <= ram_gb)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(ram_gb: u64) -> HardwareProfile {
        HardwareProfile {
            total_ram_bytes: ram_gb * 1_000_000_000,
            available_ram_bytes: ram_gb * 500_000_000,
            vram_bytes: None,
            unified_memory: true,
            logical_cores: 8,
            gpu_name: None,
        }
    }

    #[test]
    fn every_tier_is_well_formed() {
        assert!(!TIERS.is_empty());
        for t in TIERS {
            assert!(!t.label.is_empty());
            assert!(!t.note.is_empty());
            assert!(!t.models.is_empty());
            for m in t.models {
                assert!(m.repo.contains('/'), "{} is not owner/name", m.repo);
                assert!(!m.display_name.is_empty());
            }
        }
    }

    #[test]
    fn repos_are_unique_across_all_tiers() {
        let mut repos: Vec<_> = TIERS
            .iter()
            .flat_map(|t| t.models.iter().map(|m| m.repo))
            .collect();
        repos.sort_unstable();
        let before = repos.len();
        repos.dedup();
        assert_eq!(before, repos.len(), "duplicate repo across tiers");
    }

    #[test]
    fn a_machine_with_no_ram_still_gets_one_tier() {
        let tiers = for_machine(&hw(0));
        assert_eq!(tiers, vec![&TIERS[0]]);
        assert_eq!(tiers[0].label, "8GB");
    }

    #[test]
    fn selection_grows_with_ram() {
        let labels = |ram: u64| -> Vec<&'static str> {
            for_machine(&hw(ram)).iter().map(|t| t.label).collect()
        };
        assert_eq!(labels(8), vec!["8GB"]);
        assert_eq!(labels(16), vec!["8GB", "16GB"]);
        assert_eq!(labels(32), vec!["8GB", "16GB", "32GB"]);
        assert_eq!(labels(64), vec!["8GB", "16GB", "32GB", "64GB+"]);
        assert_eq!(labels(128), vec!["8GB", "16GB", "32GB", "64GB+"]);
    }

    #[test]
    fn current_tier_index_tracks_the_largest_qualifying_tier() {
        assert_eq!(current_tier_index(&hw(0)), 0);
        assert_eq!(TIERS[current_tier_index(&hw(32))].label, "32GB");
        assert_eq!(current_tier_index(&hw(128)), TIERS.len() - 1);
    }

    #[test]
    fn tiers_are_ordered_by_min_ram() {
        let mins: Vec<u32> = TIERS.iter().map(|t| t.min_ram_gb).collect();
        let mut sorted = mins.clone();
        sorted.sort_unstable();
        assert_eq!(mins, sorted);
    }
}
