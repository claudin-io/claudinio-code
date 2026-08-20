//! What this machine can actually run.
//!
//! Every number here is advisory except one: `usable_model_bytes` feeds the
//! admission check that stops the app from loading weights the machine cannot
//! hold. That failure mode is not an error dialog — it is the desktop freezing
//! while the kernel swaps a 30 GB mapping.

use crate::llama::hf::QuantOption;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfile {
    pub total_ram_bytes: u64,
    pub available_ram_bytes: u64,
    pub vram_bytes: Option<u64>,
    /// Apple Silicon: the GPU reads the same memory as the CPU, so there is no
    /// separate VRAM budget to fit inside.
    pub unified_memory: bool,
    pub logical_cores: usize,
    pub gpu_name: Option<String>,
}

/// How well a model of a given size fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Fit {
    Comfortable,
    Tight,
    WontFit,
}

/// Quantizations from best to worst quality. Anything not listed sorts last,
/// so an exotic quant is offered but never auto-recommended.
const QUALITY_ORDER: &[&str] = &[
    "F16", "BF16", "Q8_0", "Q6_K", "Q5_K_M", "Q5_K_S", "Q4_K_M", "Q4_K_S", "IQ4_XS", "IQ4_NL",
    "Q3_K_M", "IQ3_M", "IQ3_XS", "Q2_K", "IQ2_M",
];

/// Rough KV cache cost per token, summed over layers. Deliberately coarse and
/// on the generous side: it exists so the recommendation leaves headroom for
/// the context, not to predict allocation exactly. At 32k tokens this reserves
/// ~2 GB, which is the right order for a 7-14B model.
const KV_BYTES_PER_TOKEN: u64 = 64 * 1024;

pub fn detect() -> HardwareProfile {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};
    let sys = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    let unified = std::env::consts::OS == "macos" && std::env::consts::ARCH == "aarch64";
    let (gpu_name, vram_bytes) = if unified {
        (None, None)
    } else {
        detect_nvidia()
    };

    HardwareProfile {
        total_ram_bytes: sys.total_memory(),
        available_ram_bytes: sys.available_memory(),
        vram_bytes,
        unified_memory: unified,
        logical_cores: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        gpu_name,
    }
}

/// Best-effort NVIDIA probe. Never blocking for long and never fatal: when it
/// finds nothing, `-ngl auto` lets llama.cpp make the call instead.
fn detect_nvidia() -> (Option<String>, Option<u64>) {
    let mut cmd = std::process::Command::new("nvidia-smi");
    cmd.args([
        "--query-gpu=name,memory.total",
        "--format=csv,noheader,nounits",
    ]);
    crate::procutil::no_window(&mut cmd);
    let Ok(out) = cmd.output() else {
        return (None, None);
    };
    if !out.status.success() {
        return (None, None);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(first) = text.lines().next() else {
        return (None, None);
    };
    let Some((name, mib)) = first.split_once(',') else {
        return (None, None);
    };
    let vram = mib.trim().parse::<u64>().ok().map(|m| m * 1024 * 1024);
    (Some(name.trim().to_string()), vram)
}

/// Bytes a model may occupy.
///
/// Unified memory can spend most of RAM on weights; a discrete GPU is bounded
/// by VRAM but spills to RAM rather than failing, so the larger of the two is
/// the honest ceiling. 4 GB is held back for the OS and this app.
pub fn usable_model_bytes(hw: &HardwareProfile) -> u64 {
    const RESERVED: u64 = 4 * 1024 * 1024 * 1024;
    let ram_budget = (hw.total_ram_bytes as f64 * 0.70) as u64;
    let budget = ram_budget.max(hw.vram_bytes.unwrap_or(0));
    budget.min(hw.total_ram_bytes.saturating_sub(RESERVED))
}

fn quality_rank(quant: &str) -> usize {
    QUALITY_ORDER
        .iter()
        .position(|q| q.eq_ignore_ascii_case(quant))
        .unwrap_or(QUALITY_ORDER.len())
}

/// The highest-quality quantization whose weights plus KV cache fit the budget.
pub fn recommend_quant(
    budget_bytes: u64,
    ctx_tokens: u32,
    options: &[QuantOption],
) -> Option<&QuantOption> {
    let kv = KV_BYTES_PER_TOKEN.saturating_mul(u64::from(ctx_tokens));
    options
        .iter()
        .filter(|o| o.total_bytes.saturating_add(kv) <= budget_bytes)
        .min_by_key(|o| (quality_rank(&o.quant), std::cmp::Reverse(o.total_bytes)))
}

pub fn fit_verdict(total_bytes: u64, hw: &HardwareProfile) -> Fit {
    let budget = usable_model_bytes(hw);
    if budget == 0 {
        return Fit::WontFit;
    }
    let ratio = total_bytes as f64 / budget as f64;
    if ratio <= 0.6 {
        Fit::Comfortable
    } else if ratio <= 1.0 {
        Fit::Tight
    } else {
        Fit::WontFit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(quant: &str, gb: f64) -> QuantOption {
        QuantOption {
            quant: quant.into(),
            files: Vec::new(),
            total_bytes: (gb * 1e9) as u64,
            shards: 1,
        }
    }

    fn ladder() -> Vec<QuantOption> {
        vec![
            opt("Q8_0", 8.1),
            opt("Q6_K", 6.3),
            opt("Q5_K_M", 5.4),
            opt("Q4_K_M", 4.7),
            opt("IQ4_XS", 4.2),
            opt("Q3_K_M", 3.8),
        ]
    }

    #[test]
    fn recommend_quant_picks_the_best_that_fits() {
        let opts = ladder();
        let picked = recommend_quant(5_000_000_000, 0, &opts).unwrap();
        assert_eq!(picked.quant, "Q4_K_M");
        let picked = recommend_quant(64_000_000_000, 0, &opts).unwrap();
        assert_eq!(picked.quant, "Q8_0");
    }

    #[test]
    fn recommend_quant_returns_nothing_when_nothing_fits() {
        assert!(recommend_quant(1_000_000_000, 0, &ladder()).is_none());
    }

    #[test]
    fn recommend_quant_never_returns_something_over_budget() {
        let opts = ladder();
        for gb in 1..=64u64 {
            let budget = gb * 1_000_000_000;
            if let Some(p) = recommend_quant(budget, 8192, &opts) {
                assert!(
                    p.total_bytes <= budget,
                    "{} bytes recommended for a {gb} GB budget",
                    p.total_bytes
                );
            }
        }
    }

    #[test]
    fn kv_cache_overhead_shrinks_the_recommendation() {
        let opts = ladder();
        let no_ctx = recommend_quant(8_000_000_000, 0, &opts)
            .unwrap()
            .quant
            .clone();
        let big_ctx = recommend_quant(8_000_000_000, 32_768, &opts)
            .unwrap()
            .quant
            .clone();
        assert_eq!(no_ctx, "Q6_K");
        assert_ne!(big_ctx, no_ctx, "a large context must cost something");
        assert_eq!(big_ctx, "Q5_K_M");
    }

    #[test]
    fn fit_verdict_thresholds() {
        let hw = HardwareProfile {
            total_ram_bytes: 32_000_000_000,
            available_ram_bytes: 16_000_000_000,
            vram_bytes: None,
            unified_memory: true,
            logical_cores: 10,
            gpu_name: None,
        };
        let budget = usable_model_bytes(&hw);
        assert_eq!(fit_verdict(budget / 4, &hw), Fit::Comfortable);
        assert_eq!(fit_verdict(budget * 9 / 10, &hw), Fit::Tight);
        assert_eq!(fit_verdict(budget * 2, &hw), Fit::WontFit);
    }

    #[test]
    fn usable_model_bytes_leaves_headroom() {
        let hw = HardwareProfile {
            total_ram_bytes: 16_000_000_000,
            available_ram_bytes: 8_000_000_000,
            vram_bytes: None,
            unified_memory: false,
            logical_cores: 8,
            gpu_name: None,
        };
        assert!(usable_model_bytes(&hw) < hw.total_ram_bytes);
    }
}
