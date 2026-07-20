//! Construção de anexos (imagem/texto/documento) → `ContentBlock` + metadata.
//! Lógica compartilhada entre o app Tauri e o CLI/TUI (paridade real): dado um
//! caminho de arquivo, produz o bloco de conteúdo que vai no prompt do LLM e a
//! metadata leve (nome/tipo/tamanho) para exibição. Antes vivia em
//! `src-tauri/commands/agent.rs`; movida para o core Tauri-free.

use super::persist::AttachmentMeta;
use super::provider::ContentBlock;
use base64::Engine;
use std::io::Cursor;
use std::path::Path;

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const TEXT_EXTS: &[&str] = &[
    "txt", "md", "csv", "json", "yaml", "yml", "toml", "rs", "ts", "tsx", "js", "jsx", "py",
    "swift", "go", "rb", "html", "htm", "css", "sh", "bash", "sql", "xml", "log",
];

fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default()
}

fn media_type_for(ext: &str) -> String {
    match ext {
        "png" => "image/png".into(),
        "jpg" | "jpeg" => "image/jpeg".into(),
        "gif" => "image/gif".into(),
        "webp" => "image/webp".into(),
        "bmp" => "image/bmp".into(),
        e if TEXT_EXTS.contains(&e) => "text/plain".into(),
        e => format!("application/{e}"),
    }
}

/// Um caminho é anexável se existe (extensão qualquer é aceita como documento).
pub fn is_attachable(path: &str) -> bool {
    Path::new(path).is_file()
}

/// Metadata leve para exibição (pílula), sem ler/encodar o arquivo.
pub fn describe(path: &str) -> Option<AttachmentMeta> {
    let p = Path::new(path);
    if !p.is_file() {
        return None;
    }
    let name = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let size = p.metadata().map(|m| m.len()).unwrap_or(0);
    Some(AttachmentMeta {
        name,
        media_type: media_type_for(&ext_of(p)),
        size,
    })
}

/// Comprime uma imagem para reduzir o custo em tokens antes do base64.
///
/// Regras: decodifica; se a maior aresta passa de 1568px (limite de resize
/// server-side da Anthropic), reduz para 1568; re-encoda JPEG q80 (PNG/BMP
/// viram JPEG). Em qualquer erro, cai de volta pros bytes originais.
/// Retorna (bytes, media_type, largura, altura).
fn compress_image(bytes: &[u8], media_type: &str, ext: &str) -> (Vec<u8>, String, u32, u32) {
    use image::GenericImageView;
    let img = match image::load_from_memory(bytes) {
        Ok(img) => img,
        Err(_) => return (bytes.to_vec(), media_type.to_string(), 0, 0),
    };
    let (w, h) = img.dimensions();
    let max_dim = 1568u32;
    let (new_w, new_h) = if w > max_dim || h > max_dim {
        let ratio = (w as f64).max(h as f64) / max_dim as f64;
        ((w as f64 / ratio).round() as u32, (h as f64 / ratio).round() as u32)
    } else {
        (w, h)
    };
    let resized = if (new_w, new_h) != (w, h) {
        img.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let final_w = resized.width();
    let final_h = resized.height();
    let encode_as_jpeg = ext == "png" || ext == "bmp";
    let out_type = if encode_as_jpeg { "image/jpeg" } else { media_type };
    let mut out = Vec::new();
    let result = if out_type == "image/jpeg" {
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80);
        enc.encode(&resized.to_rgb8(), resized.width(), resized.height(), image::ColorType::Rgb8.into())
    } else if out_type == "image/webp" {
        let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
        enc.encode(&resized.to_rgba8(), resized.width(), resized.height(), image::ColorType::Rgba8.into())
    } else {
        resized.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
    };
    match result {
        Ok(_) => (out, out_type.to_string(), final_w, final_h),
        _ => (bytes.to_vec(), media_type.to_string(), w, h),
    }
}

/// Processa caminhos de arquivo em blocos de conteúdo + metadata. Imagens são
/// comprimidas e base64; arquivos de texto viram um bloco de texto cercado;
/// outros viram um marcador `[Arquivo anexado: nome (tamanho) — tipo]`.
/// Caminhos inexistentes são ignorados.
pub fn process_attachments(paths: &[String]) -> Vec<(ContentBlock, AttachmentMeta)> {
    let mut results = Vec::new();
    for path in paths {
        let file_path = Path::new(path);
        if !file_path.exists() {
            continue;
        }
        let ext = ext_of(file_path);
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let file_size = file_path.metadata().map(|m| m.len()).unwrap_or(0);

        let is_image = IMAGE_EXTS.contains(&ext.as_str());
        let is_text = TEXT_EXTS.contains(&ext.as_str());

        let meta = AttachmentMeta {
            name: file_name.clone(),
            media_type: media_type_for(&ext),
            size: file_size,
        };

        if is_image {
            let bytes = match std::fs::read(file_path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let media_type = media_type_for(&ext);
            let (compressed_bytes, final_media_type, img_w, img_h) =
                compress_image(&bytes, &media_type, &ext);
            let data = base64::engine::general_purpose::STANDARD.encode(&compressed_bytes);
            results.push((ContentBlock::image(&final_media_type, &data, img_w, img_h), meta));
        } else if is_text {
            let text = match std::fs::read_to_string(file_path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let block_text = format!("[Arquivo anexado: `{file_name}`]\n```\n{text}\n```");
            results.push((ContentBlock::text(block_text), meta));
        } else {
            let size_str = human_size(file_size);
            let block_text =
                format!("[Arquivo anexado: `{file_name}` ({size_str}) — tipo: {ext}]");
            results.push((ContentBlock::text(block_text), meta));
        }
    }
    results
}

pub fn human_size(bytes: u64) -> String {
    if bytes > 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes > 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_and_process_text_file() {
        let dir = std::env::temp_dir().join(format!("att_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("nota.md");
        std::fs::write(&f, "# oi\nconteúdo").unwrap();
        let path = f.to_string_lossy().to_string();

        assert!(is_attachable(&path));
        let meta = describe(&path).unwrap();
        assert_eq!(meta.name, "nota.md");
        assert_eq!(meta.media_type, "text/plain");

        let blocks = process_attachments(&[path]);
        assert_eq!(blocks.len(), 1);
        match &blocks[0].0 {
            ContentBlock::Text { text, .. } => {
                assert!(text.contains("nota.md"));
                assert!(text.contains("conteúdo"));
            }
            _ => panic!("esperava bloco de texto"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn nonexistent_path_ignored() {
        assert!(!is_attachable("/no/such/file.png"));
        assert!(describe("/no/such/file.png").is_none());
        assert!(process_attachments(&["/no/such/file.png".into()]).is_empty());
    }

    #[test]
    fn human_size_units() {
        assert_eq!(human_size(500), "500 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
    }
}
