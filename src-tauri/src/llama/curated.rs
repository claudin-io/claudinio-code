//! A short list of models worth suggesting.
//!
//! Free-text search over the Hub is also offered, but it is a bad default: the
//! agent loop needs tool calling, and most GGUF repos say nothing about whether
//! they can do it. Every entry here was checked against the Hub for a `gguf`
//! block carrying a `chat_template` — without one, `--jinja` has nothing to
//! drive tool calls with and the model answers in prose while the agent waits.
//!
//! `min_ram_gb` is the whole-machine RAM at which the preferred quantization is
//! a reasonable idea, not a hard floor; the real gate is `hardware::fit_verdict`
//! against the actual file size, which is only known once the Hub is queried.

use crate::llama::hardware::HardwareProfile;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CuratedModel {
    pub repo: &'static str,
    pub display_name: &'static str,
    pub preferred_quant: &'static str,
    pub min_ram_gb: u32,
    pub params: &'static str,
    pub blurb: &'static str,
}

pub const CURATED: &[CuratedModel] = &[
    CuratedModel {
        repo: "unsloth/Qwen3-4B-Instruct-2507-GGUF",
        display_name: "Qwen3 4B Instruct",
        preferred_quant: "Q4_K_M",
        min_ram_gb: 8,
        params: "4B",
        blurb: "The smallest model here that still holds a tool-calling loop together. \
                Start here on a laptop with 8-16 GB.",
    },
    CuratedModel {
        repo: "unsloth/Qwen2.5-Coder-7B-Instruct-GGUF",
        display_name: "Qwen2.5 Coder 7B",
        preferred_quant: "Q4_K_M",
        min_ram_gb: 16,
        params: "7B",
        blurb: "Code-tuned and small enough to stay resident while you work. \
                32k context.",
    },
    CuratedModel {
        repo: "unsloth/Qwen3-8B-GGUF",
        display_name: "Qwen3 8B",
        preferred_quant: "Q4_K_M",
        min_ram_gb: 16,
        params: "8B",
        blurb: "General-purpose sibling of the Coder models, with a longer \
                context than Qwen2.5.",
    },
    CuratedModel {
        repo: "unsloth/Qwen2.5-Coder-14B-Instruct-GGUF",
        display_name: "Qwen2.5 Coder 14B",
        preferred_quant: "Q4_K_M",
        min_ram_gb: 24,
        params: "14B",
        blurb: "The step up that starts to feel usable for multi-file edits.",
    },
    CuratedModel {
        repo: "unsloth/gpt-oss-20b-GGUF",
        display_name: "gpt-oss 20B",
        preferred_quant: "Q4_K_M",
        min_ram_gb: 32,
        params: "20B",
        blurb: "OpenAI's open-weight model, 131k context. Strong at following \
                tool schemas.",
    },
    CuratedModel {
        repo: "bartowski/mistralai_Devstral-Small-2507-GGUF",
        display_name: "Devstral Small",
        preferred_quant: "Q4_K_M",
        min_ram_gb: 32,
        params: "24B",
        blurb: "Mistral's agentic coding model — trained for exactly this kind \
                of edit-and-run loop.",
    },
    CuratedModel {
        repo: "unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF",
        display_name: "Qwen3 Coder 30B A3B",
        preferred_quant: "Q4_K_M",
        min_ram_gb: 32,
        params: "30B (3B active)",
        blurb: "Mixture-of-experts: 30B of weights but only ~3B active per \
                token, so it runs far faster than its size suggests.",
    },
    CuratedModel {
        repo: "unsloth/Qwen2.5-Coder-32B-Instruct-GGUF",
        display_name: "Qwen2.5 Coder 32B",
        preferred_quant: "Q4_K_M",
        min_ram_gb: 48,
        params: "32B",
        blurb: "The largest of the Coder line. Wants a workstation, not a laptop.",
    },
];

/// The curated list ordered by what this machine can actually run: models that
/// fit first, largest first, then the rest. Nothing is hidden — a model the
/// machine cannot hold is still worth seeing, just not worth recommending.
pub fn for_hardware(hw: &HardwareProfile) -> Vec<&'static CuratedModel> {
    let ram_gb = (hw.total_ram_bytes / 1_000_000_000) as u32;
    let mut models: Vec<&'static CuratedModel> = CURATED.iter().collect();
    models.sort_by_key(|m| {
        let fits = m.min_ram_gb <= ram_gb;
        (!fits, std::cmp::Reverse(m.min_ram_gb))
    });
    models
}

pub fn fits(model: &CuratedModel, hw: &HardwareProfile) -> bool {
    model.min_ram_gb <= (hw.total_ram_bytes / 1_000_000_000) as u32
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
    fn every_curated_entry_is_well_formed() {
        assert!(!CURATED.is_empty());
        for m in CURATED {
            assert!(m.repo.contains('/'), "{} is not owner/name", m.repo);
            assert!(!m.display_name.is_empty());
            assert!(m.preferred_quant.starts_with('Q') || m.preferred_quant.starts_with("IQ"));
            assert!(m.min_ram_gb >= 8, "{} claims to fit in nothing", m.repo);
        }
    }

    #[test]
    fn curated_repos_are_unique() {
        let mut repos: Vec<_> = CURATED.iter().map(|m| m.repo).collect();
        repos.sort_unstable();
        let before = repos.len();
        repos.dedup();
        assert_eq!(before, repos.len(), "duplicate repo in the curated list");
    }

    #[test]
    fn for_hardware_puts_the_biggest_that_fits_first() {
        let ordered = for_hardware(&hw(32));
        assert!(fits(ordered[0], &hw(32)));
        assert_eq!(ordered[0].min_ram_gb, 32);
        assert_eq!(ordered.len(), CURATED.len(), "nothing is hidden");
        // Everything that does not fit sorts after everything that does.
        let first_misfit = ordered.iter().position(|m| !fits(m, &hw(32)));
        if let Some(i) = first_misfit {
            assert!(ordered[i..].iter().all(|m| !fits(m, &hw(32))));
        }
    }

    #[test]
    fn a_small_machine_still_gets_a_recommendation() {
        let ordered = for_hardware(&hw(8));
        assert!(
            fits(ordered[0], &hw(8)),
            "{} should fit 8 GB",
            ordered[0].repo
        );
    }
}
