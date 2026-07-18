use ort::{session::Session, value::Tensor};
use std::path::Path;
use tokenizers::Tokenizer;

/// Config for a single-vector bi-encoder embedding model. Swapping candidates
/// is changing `ACTIVE_MODEL` below to point at a different const — everything
/// else (download, load, encode) reads from this struct.
struct ModelConfig {
    /// HF repo id, e.g. "Xenova/bge-small-en-v1.5".
    repo: &'static str,
    /// (remote path relative to `resolve/main/`, local filename in cache_dir).
    /// Quantized ONNX exports from Xenova live under an `onnx/` subfolder in
    /// the repo, but we flatten them into cache_dir for simplicity.
    files: &'static [(&'static str, &'static str)],
    /// Local filename (from `files`) of the ONNX model, used to check
    /// presence and to load the session.
    model_filename: &'static str,
    /// Local filename (from `files`) of the tokenizer.
    tokenizer_filename: &'static str,
    /// Prefix prepended to queries only (not documents) before encoding.
    /// bge-family models are trained with an instruction prefix for queries;
    /// MiniLM has none.
    query_prefix: &'static str,
    /// Short name used to namespace the on-disk cache dir per model, so
    /// switching ACTIVE_MODEL doesn't silently reuse a stale cache.
    cache_dirname: &'static str,
}

#[allow(dead_code)]
const BGE_SMALL: ModelConfig = ModelConfig {
    repo: "Xenova/bge-small-en-v1.5",
    files: &[
        ("onnx/model_quantized.onnx", "model_quantized.onnx"),
        ("tokenizer.json", "tokenizer.json"),
        ("config.json", "config.json"),
    ],
    model_filename: "model_quantized.onnx",
    tokenizer_filename: "tokenizer.json",
    query_prefix: "Represent this sentence for searching relevant passages: ",
    cache_dirname: "bge-small-en-v1.5",
};

const MINILM_L6: ModelConfig = ModelConfig {
    repo: "Xenova/all-MiniLM-L6-v2",
    files: &[
        ("onnx/model_quantized.onnx", "model_quantized.onnx"),
        ("tokenizer.json", "tokenizer.json"),
        ("config.json", "config.json"),
    ],
    model_filename: "model_quantized.onnx",
    tokenizer_filename: "tokenizer.json",
    query_prefix: "",
    cache_dirname: "all-MiniLM-L6-v2",
};

/// Single-vector bi-encoder in active use. Change this to `BGE_SMALL` to try
/// the other candidate — nothing else in this file needs to change. MiniLM won
/// the calibration eval (see examples/semantic_eval.rs): better top-3 rank,
/// wider score spread, less noise on off-topic queries, faster indexing.
const ACTIVE_MODEL: ModelConfig = MINILM_L6;

/// Cache-dir name derived from the active model config, so callers can
/// namespace the on-disk cache per model (e.g. `models/{this}`).
pub fn model_cache_dirname() -> &'static str {
    ACTIVE_MODEL.cache_dirname
}

/// Local filename of the active model's ONNX file, so callers can check for
/// its presence without re-hardcoding the name.
pub fn model_filename() -> &'static str {
    ACTIVE_MODEL.model_filename
}

// Bounded low: attention memory grows with seq^2, and embedding texts are
// already capped to ~800 chars of body (see MAX_BODY_CHARS below), so a much
// larger window only inflates padding and peak memory without adding signal.
const MAX_LENGTH: usize = 512;

pub struct CodeEmbedder {
    session: Session,
    tokenizer: Tokenizer,
    output_name: String,
    /// Whether the loaded model's inputs include `token_type_ids`, detected
    /// at load time from `session.inputs()` rather than assumed.
    wants_token_type_ids: bool,
}

impl CodeEmbedder {
    pub fn load(model_dir: &Path) -> Result<Self, String> {
        let model_path = model_dir.join(ACTIVE_MODEL.model_filename);
        let tokenizer_path = model_dir.join(ACTIVE_MODEL.tokenizer_filename);

        if !model_path.exists() {
            return Err(format!(
                "model not found at {}. Call ensure_model_downloaded first.",
                model_path.display()
            ));
        }
        if !tokenizer_path.exists() {
            return Err(format!(
                "tokenizer not found at {}. Call ensure_model_downloaded first.",
                tokenizer_path.display()
            ));
        }

        let session = Session::builder()
            .map_err(|e| format!("ort builder: {e}"))?
            // Memory pattern pre-allocates and retains buffers sized for the
            // largest batch ever seen, so peak memory never shrinks back down.
            // We batch inputs ourselves (see EMBED_BATCH_SIZE in indexer.rs),
            // so disable it and let the allocator release memory between runs.
            .with_memory_pattern(false)
            .map_err(|e| format!("ort memory pattern: {e}"))?
            // Cap ONNX threading: the default (one intra-op thread per
            // physical core) saturates the whole machine during indexing and
            // starves the WebView UI thread — Windows then flags the window
            // as "Not responding". Embedding is background work; keep it slow
            // and polite.
            .with_intra_threads(2)
            .map_err(|e| format!("ort intra threads: {e}"))?
            .with_inter_threads(1)
            .map_err(|e| format!("ort inter threads: {e}"))?
            .commit_from_file(&model_path)
            .map_err(|e| format!("ort load model: {e}"))?;

        let tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|e| format!("tokenizer load: {e}"))?;

        let output_name = session
            .outputs()
            .first()
            .map(|o| o.name().to_string())
            .ok_or("model has no outputs")?;

        // Some BERT-family ONNX exports require a token_type_ids input in
        // addition to input_ids/attention_mask; others don't. Detect it from
        // the model itself rather than assuming either way.
        let wants_token_type_ids = session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");

        Ok(CodeEmbedder { session, tokenizer, output_name, wants_token_type_ids })
    }

    pub fn encode(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let encoding = self
            .tokenizer
            .encode_batch(
                texts
                    .iter()
                    .map(|t| tokenizers::EncodeInput::Single(t.to_string().into()))
                    .collect(),
                true,
            )
            .map_err(|e| format!("tokenize: {e}"))?;

        let batch_size = encoding.len();
        let mut padded_len = 0;
        for enc in &encoding {
            padded_len = padded_len.max(enc.len().min(MAX_LENGTH));
        }
        if padded_len == 0 {
            padded_len = 1;
        }

        let mut input_ids = vec![0i64; batch_size * padded_len];
        let mut attention_mask = vec![0i64; batch_size * padded_len];

        for (b, enc) in encoding.iter().enumerate() {
            let ids = enc.get_ids();
            let mask = enc.get_attention_mask();
            let len = ids.len().min(MAX_LENGTH);
            for i in 0..len {
                input_ids[b * padded_len + i] = ids[i] as i64;
                attention_mask[b * padded_len + i] = mask[i] as i64;
            }
        }

        let ids_tensor = Tensor::from_array((vec![batch_size as i64, padded_len as i64], input_ids.clone()))
            .map_err(|e| format!("input_ids tensor: {e}"))?;
        let mask_tensor = Tensor::from_array((vec![batch_size as i64, padded_len as i64], attention_mask.clone()))
            .map_err(|e| format!("attention_mask tensor: {e}"))?;

        let mut inputs_map: std::collections::HashMap<String, ort::value::DynValue> =
            std::collections::HashMap::new();
        inputs_map.insert("input_ids".to_string(), ids_tensor.into());
        inputs_map.insert("attention_mask".to_string(), mask_tensor.into());

        if self.wants_token_type_ids {
            let token_type_ids = vec![0i64; batch_size * padded_len];
            let type_tensor =
                Tensor::from_array((vec![batch_size as i64, padded_len as i64], token_type_ids))
                    .map_err(|e| format!("token_type_ids tensor: {e}"))?;
            inputs_map.insert("token_type_ids".to_string(), type_tensor.into());
        }

        let ort_outs = self
            .session
            .run(inputs_map)
            .map_err(|e| format!("ort run: {e}"))?;

        let (shape, flat) = ort_outs[self.output_name.as_str()]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("extract output: {e}"))?;

        // Hidden size comes from the model output, not a constant: [batch, seq, hidden].
        let hidden = shape
            .last()
            .map(|d| *d as usize)
            .filter(|d| *d > 0)
            .unwrap_or_else(|| flat.len() / (batch_size * padded_len));
        if hidden == 0 || flat.len() < batch_size * padded_len * hidden {
            return Err(format!(
                "unexpected output tensor: shape {shape:?}, len {}",
                flat.len()
            ));
        }

        // Bi-encoder: mean-pool token embeddings into a single vector per
        // text, then L2-normalize so cosine similarity is a plain dot product.
        let mut results = Vec::with_capacity(batch_size);
        for b in 0..batch_size {
            let mut sum = vec![0f32; hidden];
            let mut count: f32 = 0.0;
            for s in 0..padded_len {
                if attention_mask[b * padded_len + s] > 0 {
                    let offset = (b * padded_len + s) * hidden;
                    for d in 0..hidden {
                        sum[d] += flat[offset + d];
                    }
                    count += 1.0;
                }
            }
            if count > 0.0 {
                for d in 0..hidden {
                    sum[d] /= count;
                }
            }
            let norm: f32 = sum.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
            for d in 0..hidden {
                sum[d] /= norm;
            }
            results.push(sum);
        }

        Ok(results)
    }

    pub fn encode_query(&mut self, text: &str) -> Result<Vec<f32>, String> {
        let prefixed;
        let query = if ACTIVE_MODEL.query_prefix.is_empty() {
            text
        } else {
            prefixed = format!("{}{}", ACTIVE_MODEL.query_prefix, text);
            &prefixed
        };
        let mut vecs = self.encode(&[query])?;
        vecs.pop().ok_or("empty encode result".into())
    }
}

/// Max chars of symbol body included in the embedding text. Bounds tokenizer
/// and inference cost — beyond this, extra body adds latency, not signal.
const MAX_BODY_CHARS: usize = 800;

pub fn build_embedding_text(
    kind: &str,
    name: &str,
    parent_context: Option<&str>,
    doc: Option<&str>,
    body: Option<&str>,
) -> String {
    let mut parts = vec![format!("{kind}: {name}")];
    if let Some(ctx) = parent_context {
        let trimmed = ctx.trim();
        if !trimmed.is_empty() {
            parts.push(format!("context: {trimmed}"));
        }
    }
    if let Some(d) = doc {
        let trimmed = d.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if let Some(b) = body {
        let trimmed = b.trim();
        if !trimmed.is_empty() {
            let cut = trimmed
                .char_indices()
                .nth(MAX_BODY_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(trimmed.len());
            parts.push(trimmed[..cut].to_string());
        }
    }
    parts.join(" | ")
}

/// Lines of overlap between consecutive chunks so a match spanning a chunk
/// boundary still lands in at least one chunk.
const CHUNK_OVERLAP_LINES: usize = 2;

/// One embeddable slice of a symbol. Large symbol bodies (big components,
/// long functions) are split into several chunks so content deep inside them
/// is searchable — a single mean-pooled vector of the first MAX_BODY_CHARS
/// can't represent a 300-line component.
#[derive(Debug, Clone)]
pub struct EmbedChunk {
    pub chunk_index: i64,
    /// Absolute 1-based file lines covered by this chunk's body slice.
    pub start_line: i64,
    pub end_line: i64,
    pub text: String,
}

/// Max chars of resolved i18n copy appended to one chunk's embedding text.
const MAX_I18N_CHARS: usize = 400;

/// Resolve i18n keys referenced in a code slice into their user-visible copy.
/// Framework-agnostic: instead of parsing call syntax (`t(...)`,
/// `NSLocalizedString(...)`, `I18n.t(...)`, `$t(...)`, ...), every quoted
/// string literal in the slice is checked for exact membership in the dict —
/// translation keys are distinctive enough that membership is the filter.
fn resolve_i18n_keys(slice: &str, dict: &std::collections::HashMap<String, String>) -> String {
    let bytes = slice.as_bytes();
    let mut out: Vec<&str> = Vec::new();
    let mut total = 0usize;
    let mut i = 0usize;
    while i < bytes.len() && total < MAX_I18N_CHARS {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            if let Some(end) = slice[i + 1..].find(b as char) {
                let literal = &slice[i + 1..i + 1 + end];
                if !literal.is_empty()
                    && literal.len() <= 128
                    && literal.chars().all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))
                {
                    if let Some(value) = dict.get(literal) {
                        if !out.contains(&value.as_str()) {
                            total += value.len() + 1;
                            out.push(value);
                        }
                    }
                }
                i += 1 + end + 1;
                continue;
            }
        }
        i += 1;
    }
    out.join(" ")
}

/// Split a symbol into embedding chunks. Line-based and language-agnostic:
/// works identically for any tree-sitter grammar (and doc sections), since it
/// only sees the extracted body text. Every chunk repeats the symbol header
/// (kind/name/context/doc) so it stays anchored to its symbol. When an i18n
/// dict is given, copy referenced via `t("key")` inside a chunk is resolved
/// and appended to that chunk's text, so user-visible wording is searchable.
pub fn build_embedding_chunks(
    kind: &str,
    name: &str,
    parent_context: Option<&str>,
    doc: Option<&str>,
    body: Option<&str>,
    symbol_start_line: i64,
    symbol_end_line: i64,
    i18n: Option<&std::collections::HashMap<String, String>>,
) -> Vec<EmbedChunk> {
    let mut header_parts = vec![format!("{kind}: {name}")];
    if let Some(ctx) = parent_context {
        let trimmed = ctx.trim();
        if !trimmed.is_empty() {
            header_parts.push(format!("context: {trimmed}"));
        }
    }
    if let Some(d) = doc {
        let trimmed = d.trim();
        if !trimmed.is_empty() {
            header_parts.push(trimmed.to_string());
        }
    }
    let header = header_parts.join(" | ");

    let body = body.map(str::trim_end).unwrap_or("");
    if body.trim().is_empty() {
        return vec![EmbedChunk {
            chunk_index: 0,
            start_line: symbol_start_line,
            end_line: symbol_end_line,
            text: header,
        }];
    }

    let lines: Vec<&str> = body.lines().collect();
    // The body is the tail of the symbol's line range (for code it's the whole
    // range; for doc sections it starts after the heading), so anchor absolute
    // line numbers from the end. This holds for every language uniformly.
    let body_first_line = (symbol_end_line - lines.len() as i64 + 1).max(symbol_start_line);

    let mut chunks: Vec<EmbedChunk> = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let mut chars = 0usize;
        let mut j = i;
        while j < lines.len() {
            let line_len = lines[j].chars().count() + 1;
            if chars + line_len > MAX_BODY_CHARS && j > i {
                break;
            }
            chars += line_len;
            j += 1;
        }
        let slice = lines[i..j].join("\n");
        let slice = slice.trim();
        if !slice.is_empty() {
            let mut text = format!("{header} | {slice}");
            if let Some(dict) = i18n {
                let copy = resolve_i18n_keys(slice, dict);
                if !copy.is_empty() {
                    text.push_str(" | i18n: ");
                    text.push_str(&copy);
                }
            }
            chunks.push(EmbedChunk {
                chunk_index: chunks.len() as i64,
                start_line: body_first_line + i as i64,
                end_line: body_first_line + j as i64 - 1,
                text,
            });
        }
        if j >= lines.len() {
            break;
        }
        i = j.saturating_sub(CHUNK_OVERLAP_LINES).max(i + 1);
    }

    if chunks.is_empty() {
        chunks.push(EmbedChunk {
            chunk_index: 0,
            start_line: symbol_start_line,
            end_line: symbol_end_line,
            text: header,
        });
    }
    chunks
}

pub async fn ensure_model_downloaded(cache_dir: &Path) -> Result<(), String> {
    if cache_dir.join(ACTIVE_MODEL.model_filename).exists() {
        return Ok(());
    }

    // A repeated hit here means the cache path is not persisting between runs
    // (suspected on some Windows setups) — every index would re-download the
    // model from HuggingFace.
    eprintln!(
        "[embeddings] model not in cache, downloading {} to {}",
        ACTIVE_MODEL.repo,
        cache_dir.display()
    );

    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("create model dir: {e}"))?;

    let base_url = format!("https://huggingface.co/{}/resolve/main", ACTIVE_MODEL.repo);

    for (remote_path, local_filename) in ACTIVE_MODEL.files {
        let url = format!("{base_url}/{remote_path}");
        let dest = cache_dir.join(local_filename);
        if dest.exists() {
            continue;
        }

        let net_guard = crate::net_activity::NetGuard::begin(
            crate::net_activity::NetSource::EmbeddingModelDownload,
            *local_filename,
        );
        let client = crate::http::default_client();
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("download {local_filename}: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            return Err(format!("download {local_filename} failed: HTTP {status}"));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("read {local_filename}: {e}"))?;
        net_guard.add_bytes(bytes.len() as u64);

        std::fs::write(&dest, &bytes).map_err(|e| format!("write {local_filename}: {e}"))?;
    }

    Ok(())
}

pub type SharedEmbedder = std::sync::Arc<std::sync::Mutex<CodeEmbedder>>;

pub fn load_shared(cache_dir: &Path) -> Result<SharedEmbedder, String> {
    let embedder = CodeEmbedder::load(cache_dir)?;
    Ok(std::sync::Arc::new(std::sync::Mutex::new(embedder)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_produces_normalized_model_dim_vectors() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("models/{}", model_cache_dirname()));
        if !dir.join(ACTIVE_MODEL.model_filename).exists() {
            eprintln!("model not present, skipping");
            return;
        }
        let mut e = CodeEmbedder::load(&dir).expect("load model");
        for o in e.session.outputs() {
            eprintln!("model output: {}", o.name());
        }
        for i in e.session.inputs() {
            eprintln!("model input: {}", i.name());
        }
        let vecs = e
            .encode(&["fn hello_world() {}", "struct FileWatcher that reindexes files"])
            .expect("encode");
        assert_eq!(vecs.len(), 2);
        eprintln!("embedding dim = {}", vecs[0].len());
        assert!(vecs[0].len() >= 32, "dim too small: {}", vecs[0].len());
        assert_eq!(vecs[0].len(), vecs[1].len());
        for v in &vecs {
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-3, "not normalized: {norm}");
        }
        // Distinct texts must not collapse to the same vector.
        assert!(vecs[0].iter().zip(&vecs[1]).any(|(a, b)| (a - b).abs() > 1e-3));
    }

    #[test]
    fn small_body_yields_single_chunk_with_symbol_lines() {
        let chunks = build_embedding_chunks("function", "foo", None, None, Some("let x = 1;"), 10, 10, None);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!((chunks[0].start_line, chunks[0].end_line), (10, 10));
        assert!(chunks[0].text.contains("function: foo"));
        assert!(chunks[0].text.contains("let x = 1;"));
    }

    #[test]
    fn no_body_yields_header_only_chunk() {
        let chunks = build_embedding_chunks("struct", "Config", Some("mod db"), None, None, 5, 8, None);
        assert_eq!(chunks.len(), 1);
        assert_eq!((chunks[0].start_line, chunks[0].end_line), (5, 8));
        assert_eq!(chunks[0].text, "struct: Config | context: mod db");
    }

    #[test]
    fn large_body_splits_into_overlapping_chunks_with_absolute_lines() {
        // 100 lines of ~40 chars -> several chunks under MAX_BODY_CHARS (800).
        let body: Vec<String> = (0..100).map(|i| format!("line {i} {}", "x".repeat(32))).collect();
        let body = body.join("\n");
        // Symbol spans lines 50..149 and body covers the whole range.
        let chunks = build_embedding_chunks("function", "big", None, None, Some(&body), 50, 149, None);
        assert!(chunks.len() > 2, "expected multiple chunks, got {}", chunks.len());
        assert_eq!(chunks[0].start_line, 50);
        assert_eq!(chunks.last().unwrap().end_line, 149);
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.chunk_index, i as i64);
            assert!(c.text.starts_with("function: big | "));
            assert!(c.start_line >= 50 && c.end_line <= 149 && c.start_line <= c.end_line);
        }
        // Consecutive chunks overlap by CHUNK_OVERLAP_LINES.
        for w in chunks.windows(2) {
            assert_eq!(w[1].start_line, w[0].end_line + 1 - CHUNK_OVERLAP_LINES as i64);
        }
        // Deep content (line 90) is present in some chunk even though it is
        // far beyond the first MAX_BODY_CHARS of the body.
        assert!(chunks.iter().any(|c| c.text.contains("line 90")));
    }

    #[test]
    fn i18n_keys_resolve_into_chunk_text_framework_agnostic() {
        let mut dict = std::collections::HashMap::new();
        dict.insert("onboarding.features.agent.title".to_string(), "Agent-first coding".to_string());
        dict.insert("app.title".to_string(), "Claudinio Code".to_string());
        // t("...") (web), NSLocalizedString (iOS), I18n.t (Rails) all reduce
        // to a quoted literal that is a dict key.
        let body = concat!(
            "const a = t(\"onboarding.features.agent.title\");\n",
            "let b = NSLocalizedString('app.title', comment: '');\n",
            "let c = other(\"not.a.key\");"
        );
        let chunks = build_embedding_chunks("function", "Wizard", None, None, Some(body), 1, 3, Some(&dict));
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.contains("i18n: "));
        assert!(chunks[0].text.contains("Agent-first coding"));
        assert!(chunks[0].text.contains("Claudinio Code"));
        assert!(!chunks[0].text.contains("not.a.key\" resolved"));

        // Without a dict, text is unchanged (no i18n marker).
        let plain = build_embedding_chunks("function", "Wizard", None, None, Some(body), 1, 3, None);
        assert!(!plain[0].text.contains("i18n:"));
    }

    #[test]
    fn i18n_resolution_is_capped_and_deduped() {
        let mut dict = std::collections::HashMap::new();
        dict.insert("k".to_string(), "x".repeat(300));
        dict.insert("k2".to_string(), "y".repeat(300));
        let body = "t(\"k\") t(\"k\") t(\"k2\") t(\"k2\")";
        let out = resolve_i18n_keys(body, &dict);
        // Deduped: each value once; capped near MAX_I18N_CHARS.
        assert!(out.len() <= MAX_I18N_CHARS + 310);
        assert_eq!(out.matches(&"x".repeat(300)).count(), 1);
    }

    #[test]
    fn doc_section_body_anchors_lines_from_symbol_end() {
        // Doc sections: body starts after the heading line, so absolute lines
        // are anchored from end_line (body = last N lines of the range).
        let body = "para one\npara two\npara three";
        let chunks = build_embedding_chunks("doc_section", "Intro", None, None, Some(body), 4, 7, None);
        assert_eq!(chunks.len(), 1);
        assert_eq!((chunks[0].start_line, chunks[0].end_line), (5, 7));
    }
}
