use ort::{session::Session, value::Tensor};
use std::path::Path;
use tokenizers::Tokenizer;

const MODEL_REPO: &str = "lightonai/LateOn-Code-edge";
const MODEL_FILES: &[(&str, &str)] = &[
    ("model_int8.onnx", "model_int8.onnx"),
    ("tokenizer.json", "tokenizer.json"),
    ("config.json", "config.json"),
];
const MAX_LENGTH: usize = 2047;

pub struct CodeEmbedder {
    session: Session,
    tokenizer: Tokenizer,
    output_name: String,
}

impl CodeEmbedder {
    pub fn load(model_dir: &Path) -> Result<Self, String> {
        let model_path = model_dir.join("model_int8.onnx");
        let tokenizer_path = model_dir.join("tokenizer.json");

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
            .commit_from_file(&model_path)
            .map_err(|e| format!("ort load model: {e}"))?;

        let tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|e| format!("tokenizer load: {e}"))?;

        let output_name = session
            .outputs()
            .first()
            .map(|o| o.name().to_string())
            .ok_or("model has no outputs")?;

        Ok(CodeEmbedder { session, tokenizer, output_name })
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

        // LateOn is a late-interaction (ColBERT-style) model; mean-pooling its token
        // embeddings into one vector is a deliberate v1 simplification.
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
        let mut vecs = self.encode(&[text])?;
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

pub async fn ensure_model_downloaded(cache_dir: &Path) -> Result<(), String> {
    if cache_dir.join("model_int8.onnx").exists() {
        return Ok(());
    }

    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("create model dir: {e}"))?;

    let base_url = format!("https://huggingface.co/{MODEL_REPO}/resolve/main");

    for (filename, _) in MODEL_FILES {
        let url = format!("{base_url}/{filename}");
        let dest = cache_dir.join(filename);
        if dest.exists() {
            continue;
        }

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("download {filename}: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            return Err(format!("download {filename} failed: HTTP {status}"));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("read {filename}: {e}"))?;

        std::fs::write(&dest, &bytes).map_err(|e| format!("write {filename}: {e}"))?;
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
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models/LateOn-Code-edge");
        if !dir.join("model_int8.onnx").exists() {
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
}
