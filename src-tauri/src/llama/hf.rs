//! Finding GGUF weights on the Hugging Face Hub.
//!
//! The tree API returns each LFS object's `oid`, which *is* the file's sha256.
//! That is what makes this different from every other pinned artifact in the
//! app: the integrity check comes from upstream, live, so there is no pin table
//! to generate and nothing to go stale.

use crate::net_activity::{NetGuard, NetSource};
use serde::{Deserialize, Serialize};

const API: &str = "https://huggingface.co/api/models";
const RESOLVE: &str = "https://huggingface.co";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HfModelSummary {
    pub repo: String,
    pub downloads: u64,
    pub likes: u64,
    /// Gated repos need a licence accepted on the website; downloading one
    /// without a token yields an HTML error page, not weights.
    pub gated: bool,
}

/// One file in a repo tree. `lfs` is absent for small git-stored files
/// (README, config.json) and present for every real GGUF.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfTreeFile {
    pub path: String,
    pub size: u64,
    #[serde(default)]
    pub lfs: Option<HfLfs>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfLfs {
    /// sha256 of the object, hex-encoded.
    pub oid: String,
    pub size: u64,
}

/// The `gguf` block Hugging Face parses out of the weights themselves.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GgufMeta {
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub architecture: Option<String>,
    #[serde(default, rename = "context_length")]
    pub context_length: Option<u32>,
    /// Without a chat template `--jinja` has nothing to drive tool calling
    /// with, and the model answers in prose forever.
    #[serde(default, rename = "chat_template")]
    pub chat_template: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HfRepoDetail {
    pub repo: String,
    pub gated: bool,
    pub gguf: Option<GgufMeta>,
    pub files: Vec<HfTreeFile>,
}

/// One downloadable quantization: a single GGUF, or an ordered shard set.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuantOption {
    pub quant: String,
    #[serde(skip)]
    pub files: Vec<HfTreeFile>,
    pub total_bytes: u64,
    pub shards: usize,
}

/// Files that make up an MLX model.
///
/// Unlike GGUF — one file, or a shard set, with the quantization in the
/// filename — an MLX model is the whole repository: weights plus config plus
/// tokenizer, with the quantization in the *repo* name (`…-4bit`). Everything
/// except documentation has to come down or the loader fails.
pub fn mlx_files(files: &[HfTreeFile]) -> Vec<HfTreeFile> {
    const SKIP: &[&str] = &[".gitattributes", "README.md", "LICENSE", ".gitignore"];
    files
        .iter()
        .filter(|f| {
            let name = f.path.rsplit('/').next().unwrap_or(&f.path);
            !SKIP.contains(&name) && !name.starts_with('.')
        })
        .cloned()
        .collect()
}

/// Whether a repo looks like an MLX model rather than a GGUF one.
pub fn is_mlx_repo(files: &[HfTreeFile]) -> bool {
    let has_safetensors = files.iter().any(|f| f.path.ends_with(".safetensors"));
    let has_gguf = files
        .iter()
        .any(|f| f.path.to_lowercase().ends_with(".gguf"));
    has_safetensors && !has_gguf
}

/// The quantization an MLX repo advertises in its name (`…-4bit`, `…-8bit`,
/// `…-bf16`). MLX repos are published one quantization per repo, so there is
/// nothing to choose once the repo is chosen.
pub fn mlx_quant(repo: &str) -> String {
    let name = repo.rsplit('/').next().unwrap_or(repo);
    for suffix in ["4bit", "8bit", "6bit", "3bit", "bf16", "fp16"] {
        if name.to_lowercase().ends_with(suffix) {
            return suffix.to_uppercase();
        }
    }
    "unknown".into()
}

pub fn resolve_url(repo: &str, path: &str) -> String {
    format!("{RESOLVE}/{repo}/resolve/main/{path}")
}

pub async fn search(query: &str, limit: usize) -> Result<Vec<HfModelSummary>, String> {
    #[derive(Deserialize)]
    struct Row {
        id: String,
        #[serde(default)]
        downloads: u64,
        #[serde(default)]
        likes: u64,
        #[serde(default)]
        gated: serde_json::Value,
    }

    let url = format!(
        "{API}?filter=gguf&search={}&sort=downloads&direction=-1&limit={limit}",
        urlencode(query)
    );
    let _guard = NetGuard::begin(NetSource::HuggingFaceApi, "model search");
    let rows: Vec<Row> = crate::http::default_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("hugging face search: {e}"))?
        .error_for_status()
        .map_err(|e| format!("hugging face search: {e}"))?
        .json()
        .await
        .map_err(|e| format!("hugging face search: unexpected response: {e}"))?;

    Ok(rows
        .into_iter()
        .map(|r| HfModelSummary {
            repo: r.id,
            downloads: r.downloads,
            likes: r.likes,
            // `gated` is false, "auto" or "manual" — anything but false gates.
            gated: !matches!(r.gated, serde_json::Value::Bool(false)),
        })
        .collect())
}

pub async fn repo_detail(repo: &str) -> Result<HfRepoDetail, String> {
    #[derive(Deserialize)]
    struct Info {
        #[serde(default)]
        gguf: Option<GgufMeta>,
        #[serde(default)]
        gated: serde_json::Value,
    }

    let client = crate::http::default_client();
    let _guard = NetGuard::begin(NetSource::HuggingFaceApi, repo);

    let info: Info = client
        .get(format!("{API}/{repo}"))
        .send()
        .await
        .map_err(|e| format!("hugging face {repo}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("hugging face {repo}: {e}"))?
        .json()
        .await
        .map_err(|e| format!("hugging face {repo}: unexpected response: {e}"))?;

    let files: Vec<HfTreeFile> = client
        .get(format!("{API}/{repo}/tree/main?recursive=true"))
        .send()
        .await
        .map_err(|e| format!("hugging face {repo} tree: {e}"))?
        .error_for_status()
        .map_err(|e| format!("hugging face {repo} tree: {e}"))?
        .json()
        .await
        .map_err(|e| format!("hugging face {repo} tree: unexpected response: {e}"))?;

    Ok(HfRepoDetail {
        repo: repo.to_string(),
        gated: !matches!(info.gated, serde_json::Value::Bool(false)),
        gguf: info.gguf,
        files,
    })
}

/// `…-00002-of-00003.gguf` → `(2, 3)`. Upstream always pads to five digits;
/// anything else is a filename that merely looks like a shard.
pub fn parse_shard(path: &str) -> Option<(u32, u32)> {
    let stem = path.strip_suffix(".gguf")?;
    let (rest, of) = stem.rsplit_once("-of-")?;
    let (_, index) = rest.rsplit_once('-')?;
    if index.len() != 5 || of.len() != 5 {
        return None;
    }
    Some((index.parse().ok()?, of.parse().ok()?))
}

/// The quantization token in a GGUF filename: the last dash-separated segment
/// once any shard suffix is removed.
fn quant_of(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.strip_suffix(".gguf")?;
    let stem = match parse_shard(name) {
        Some(_) => stem.rsplit_once("-of-")?.0.rsplit_once('-')?.0,
        None => stem,
    };
    let (_, quant) = stem.rsplit_once('-')?;
    if quant.is_empty() {
        return None;
    }
    Some(quant.to_uppercase())
}

/// Group a repo tree into downloadable quantizations.
///
/// Incomplete shard sets are dropped rather than offered: half a model
/// downloads happily and then fails to load, which is the worst possible place
/// to discover the problem.
pub fn group_quants(files: &[HfTreeFile]) -> Vec<QuantOption> {
    use std::collections::BTreeMap;
    let mut buckets: BTreeMap<String, Vec<HfTreeFile>> = BTreeMap::new();
    for f in files {
        if !f.path.to_lowercase().ends_with(".gguf") {
            continue;
        }
        // Only LFS files carry a sha256, and without one there is nothing to
        // verify the download against.
        if f.lfs.is_none() {
            continue;
        }
        let Some(quant) = quant_of(&f.path) else {
            continue;
        };
        buckets.entry(quant).or_default().push(f.clone());
    }

    let mut out = Vec::with_capacity(buckets.len());
    for (quant, mut group) in buckets {
        let shard_info: Vec<Option<(u32, u32)>> = group
            .iter()
            .map(|f| parse_shard(f.path.rsplit('/').next().unwrap_or(&f.path)))
            .collect();

        if shard_info.iter().all(Option::is_some) {
            let expected = shard_info[0].expect("checked").1 as usize;
            if group.len() != expected {
                continue;
            }
            let mut indexed: Vec<(u32, HfTreeFile)> = shard_info
                .iter()
                .zip(group)
                .map(|(s, f)| (s.expect("checked").0, f))
                .collect();
            indexed.sort_by_key(|(i, _)| *i);
            group = indexed.into_iter().map(|(_, f)| f).collect();
        } else if group.len() != 1 {
            // A mix of sharded and unsharded files under one quant token is
            // not something we can order or trust.
            continue;
        }

        let total_bytes = group
            .iter()
            .map(|f| f.lfs.as_ref().map_or(f.size, |l| l.size))
            .sum();
        let shards = group.len();
        out.push(QuantOption {
            quant,
            files: group,
            total_bytes,
            shards,
        });
    }
    out
}

/// Minimal percent-encoding for a query value. The search box is free text, so
/// spaces and slashes reach here routinely.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lfs_file(path: &str, size: u64) -> HfTreeFile {
        HfTreeFile {
            path: path.into(),
            size,
            lfs: Some(HfLfs {
                oid: "a".repeat(64),
                size,
            }),
        }
    }

    #[test]
    fn parse_shard_reads_the_five_digit_form() {
        assert_eq!(parse_shard("Qwen3-Q8_0-00002-of-00003.gguf"), Some((2, 3)));
        assert_eq!(parse_shard("Qwen3-Q4_K_M.gguf"), None);
        assert_eq!(parse_shard("Qwen3-1-of-3.gguf"), None);
        assert_eq!(parse_shard("not-a-model.txt"), None);
    }

    #[test]
    fn quant_is_read_from_the_filename() {
        assert_eq!(quant_of("Qwen3-8B-Q4_K_M.gguf").as_deref(), Some("Q4_K_M"));
        assert_eq!(
            quant_of("sub/dir/Qwen3-8B-Q8_0-00001-of-00003.gguf").as_deref(),
            Some("Q8_0")
        );
        assert_eq!(quant_of("model.bin"), None);
    }

    #[test]
    fn group_quants_buckets_by_quant_and_orders_shards() {
        let files = vec![
            lfs_file("Qwen3-8B-Q4_K_M.gguf", 4_700_000_000),
            lfs_file("Qwen3-8B-Q8_0-00003-of-00003.gguf", 1_000),
            lfs_file("Qwen3-8B-Q8_0-00001-of-00003.gguf", 2_000),
            lfs_file("Qwen3-8B-Q8_0-00002-of-00003.gguf", 3_000),
        ];
        let opts = group_quants(&files);
        assert_eq!(opts.len(), 2);

        let q8 = opts.iter().find(|o| o.quant == "Q8_0").unwrap();
        assert_eq!(q8.shards, 3);
        assert_eq!(q8.total_bytes, 6_000);
        let order: Vec<&str> = q8.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            order,
            vec![
                "Qwen3-8B-Q8_0-00001-of-00003.gguf",
                "Qwen3-8B-Q8_0-00002-of-00003.gguf",
                "Qwen3-8B-Q8_0-00003-of-00003.gguf",
            ]
        );

        let q4 = opts.iter().find(|o| o.quant == "Q4_K_M").unwrap();
        assert_eq!(q4.shards, 1);
    }

    #[test]
    fn group_quants_drops_an_incomplete_shard_set() {
        let files = vec![
            lfs_file("m-Q8_0-00001-of-00003.gguf", 1),
            lfs_file("m-Q8_0-00002-of-00003.gguf", 1),
        ];
        assert!(group_quants(&files).is_empty());
    }

    #[test]
    fn group_quants_ignores_files_without_a_checksum() {
        let files = vec![HfTreeFile {
            path: "m-Q4_K_M.gguf".into(),
            size: 10,
            lfs: None,
        }];
        assert!(group_quants(&files).is_empty());
    }

    /// Captured verbatim from `/api/models/unsloth/Qwen3-8B-GGUF/tree/main`.
    /// The point of the test is that `lfs.oid` is the sha256 we verify against,
    /// and that the top-level `oid` (a git blob hash) is not mistaken for it.
    #[test]
    fn tree_entry_deserializes_lfs_oid_as_the_sha256() {
        let raw = r#"{
            "type":"file",
            "oid":"6e741018b927a409553a13979af2f9590676997f",
            "size":16388044384,
            "lfs":{
                "oid":"5e416a2020fe63e76ea13c8979be35fc6070aaf3578f7876400c55c2f5c3eb30",
                "size":16388044384,
                "pointerSize":136
            },
            "path":"Qwen3-8B-BF16.gguf"
        }"#;
        let f: HfTreeFile = serde_json::from_str(raw).unwrap();
        let lfs = f.lfs.unwrap();
        assert_eq!(lfs.oid.len(), 64);
        assert_eq!(
            lfs.oid,
            "5e416a2020fe63e76ea13c8979be35fc6070aaf3578f7876400c55c2f5c3eb30"
        );
        assert_eq!(f.path, "Qwen3-8B-BF16.gguf");
    }

    #[test]
    fn tree_entry_without_lfs_deserializes() {
        let raw = r#"{"type":"file","oid":"e433","size":3083,"path":".gitattributes"}"#;
        let f: HfTreeFile = serde_json::from_str(raw).unwrap();
        assert!(f.lfs.is_none());
    }

    #[test]
    fn mlx_files_keep_everything_the_loader_needs() {
        let files = vec![
            lfs_file("model.safetensors", 4_000_000_000),
            HfTreeFile {
                path: "config.json".into(),
                size: 900,
                lfs: None,
            },
            HfTreeFile {
                path: "tokenizer.json".into(),
                size: 7_000_000,
                lfs: None,
            },
            HfTreeFile {
                path: "README.md".into(),
                size: 100,
                lfs: None,
            },
            HfTreeFile {
                path: ".gitattributes".into(),
                size: 10,
                lfs: None,
            },
        ];
        let kept: Vec<String> = mlx_files(&files).into_iter().map(|f| f.path).collect();
        let has = |name: &str| kept.iter().any(|p| p == name);
        assert!(has("config.json"), "config is not optional");
        assert!(has("tokenizer.json"));
        assert!(has("model.safetensors"));
        assert!(!has("README.md"), "docs are not weights");
        assert!(!has(".gitattributes"));
    }

    #[test]
    fn an_mlx_repo_is_told_from_a_gguf_one() {
        let mlx = vec![lfs_file("model.safetensors", 1)];
        let gguf = vec![lfs_file("m-Q4_K_M.gguf", 1)];
        assert!(is_mlx_repo(&mlx));
        assert!(!is_mlx_repo(&gguf));
        // A repo publishing both is ambiguous; GGUF wins because that is the
        // engine that exists everywhere.
        assert!(!is_mlx_repo(&[mlx[0].clone(), gguf[0].clone()]));
    }

    #[test]
    fn mlx_quant_comes_from_the_repo_name() {
        assert_eq!(mlx_quant("mlx-community/Qwen3-8B-4bit"), "4BIT");
        assert_eq!(mlx_quant("mlx-community/Qwen3-8B-bf16"), "BF16");
        assert_eq!(mlx_quant("someone/plain-model"), "unknown");
    }

    #[test]
    fn resolve_url_builds_the_hf_resolve_path() {
        assert_eq!(
            resolve_url("unsloth/Qwen3-8B-GGUF", "Qwen3-8B-Q4_K_M.gguf"),
            "https://huggingface.co/unsloth/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf"
        );
    }

    #[test]
    fn urlencode_escapes_what_a_search_box_produces() {
        assert_eq!(urlencode("qwen coder/gguf"), "qwen%20coder%2Fgguf");
    }
}
