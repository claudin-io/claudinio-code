use base64::Engine;
use serde::Serialize;
use std::fs;
use std::io::Write;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteClipboardBlobResult {
    pub path: String,
    pub name: String,
    pub media_type: String,
    pub size: usize,
}

#[tauri::command]
pub fn write_clipboard_blob(
    data: String,
    name: String,
    media_type: String,
) -> Result<WriteClipboardBlobResult, String> {
    // Decode base64 data
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data)
        .map_err(|e| format!("Failed to decode base64 data: {e}"))?;

    // Determine file extension from media type
    let ext = match media_type.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "application/zip" => "zip",
        "application/gzip" => "gz",
        _ => "bin",
    };

    // Determine final filename — use name as-is if it already has the right extension
    let file_name = if name.ends_with(&format!(".{ext}")) {
        name
    } else {
        format!("{name}.{ext}")
    };

    // Write to temp file in system temp directory
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join(&file_name);

    let mut f =
        fs::File::create(&file_path).map_err(|e| format!("Failed to create temp file: {e}"))?;
    f.write_all(&bytes)
        .map_err(|e| format!("Failed to write temp file: {e}"))?;

    let size = bytes.len();
    let path_str = file_path.to_string_lossy().to_string();

    Ok(WriteClipboardBlobResult {
        path: path_str,
        name: file_name,
        media_type,
        size,
    })
}
