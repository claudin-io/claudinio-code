//! Which MLX models can run MTP speculative decoding, and with what.
//!
//! MTP is not a property of the engine, it is a property of a *pair*: a target
//! that emits its hidden state and shared K/V, and a drafter trained against
//! that exact target. In `mlx-swift-lm` today only Gemma 4 emits the state
//! (`MLXVLM/Models/Gemma4.swift`) and only one drafter type is registered
//! (`gemma4_assistant`), so this table has two rows. It grows when upstream
//! grows, not before — a pair invented here loads two models and speculates
//! with neither.
//!
//! Every repo below was verified live on the Hugging Face API.

/// A target repo and the drafter that rides along with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MtpPair {
    /// The model the user actually chose.
    pub target_repo: &'static str,
    /// The drafter repo, ~1 GB, downloaded alongside it.
    pub drafter_repo: &'static str,
}

pub const PAIRS: &[MtpPair] = &[
    MtpPair {
        target_repo: "mlx-community/gemma-4-31b-it-4bit",
        drafter_repo: "mlx-community/gemma-4-31B-it-assistant-bf16",
    },
    MtpPair {
        target_repo: "mlx-community/gemma-4-26b-a4b-it-4bit",
        drafter_repo: "mlx-community/gemma-4-26B-A4B-it-assistant-bf16",
    },
];

/// The drafter for `repo`, if it has one.
///
/// Case-insensitive because the Hub is: `gemma-4-31B-it` and `gemma-4-31b-it`
/// address the same repository, and a user who pasted the other casing should
/// still get speculation.
pub fn drafter_for(repo: &str) -> Option<&'static str> {
    PAIRS
        .iter()
        .find(|p| p.target_repo.eq_ignore_ascii_case(repo))
        .map(|p| p.drafter_repo)
}

/// Whether `repo` is one of the drafters rather than a model to chat with.
///
/// A drafter is installed like any other MLX repo, so without this it would
/// show up in the model picker — where choosing it produces a server that
/// loads and answers nothing, since a drafter has no tokenizer of its own.
pub fn is_drafter(repo: &str) -> bool {
    PAIRS
        .iter()
        .any(|p| p.drafter_repo.eq_ignore_ascii_case(repo))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pair_is_well_formed() {
        for p in PAIRS {
            assert!(
                p.target_repo.contains('/'),
                "{} is not owner/name",
                p.target_repo
            );
            assert!(
                p.drafter_repo.contains('/'),
                "{} is not owner/name",
                p.drafter_repo
            );
            assert_ne!(p.target_repo, p.drafter_repo);
        }
    }

    #[test]
    fn a_target_finds_its_drafter() {
        assert_eq!(
            drafter_for("mlx-community/gemma-4-31b-it-4bit"),
            Some("mlx-community/gemma-4-31B-it-assistant-bf16")
        );
        assert_eq!(drafter_for("mlx-community/Qwen3.5-27B-4bit"), None);
    }

    #[test]
    fn the_hubs_casing_does_not_decide_whether_speculation_happens() {
        assert!(drafter_for("MLX-Community/Gemma-4-31B-IT-4bit").is_some());
    }

    #[test]
    fn a_drafter_is_not_offered_as_a_model() {
        assert!(is_drafter("mlx-community/gemma-4-31B-it-assistant-bf16"));
        assert!(!is_drafter("mlx-community/gemma-4-31b-it-4bit"));
    }

    /// A drafter that is also somebody's target would be installed twice and
    /// listed as neither.
    #[test]
    fn no_repo_is_both_a_target_and_a_drafter() {
        for p in PAIRS {
            assert!(!is_drafter(p.target_repo), "{} is both", p.target_repo);
            assert!(
                drafter_for(p.drafter_repo).is_none(),
                "{} is both",
                p.drafter_repo
            );
        }
    }
}
