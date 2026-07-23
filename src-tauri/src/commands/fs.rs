use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkEntry {
    pub path: String,
    pub is_dir: bool,
}

/// Read a file and return its base64-encoded content plus metadata.
/// Used by the frontend to prepare attachments before sending to the agent.
///
/// Deliberately NOT workspace-scoped, unlike `read_file`: attaching a screenshot
/// from the Desktop or a spec from Downloads is the point of the feature, and
/// those paths arrive from a native file picker or a drag-and-drop event. The
/// consequence is that this command can read any file the user can — see the
/// "What we explicitly do not defend" section of SECURITY.md.
#[tauri::command]
pub fn read_attachment(path: String) -> Result<AttachmentData, String> {
    let file_path = Path::new(&path);
    if !file_path.exists() {
        return Err(format!("File not found: {path}"));
    }
    let bytes = std::fs::read(file_path).map_err(|e| format!("Cannot read file: {e}"))?;

    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    let media_type = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "doc" | "docx" => "application/msword",
        "xls" | "xlsx" => "application/vnd.ms-excel",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "yaml" | "yml" => "application/x-yaml",
        "toml" => "application/toml",
        "rs" => "text/x-rust",
        "ts" | "tsx" => "text/typescript",
        "js" | "jsx" => "text/javascript",
        "py" => "text/x-python",
        "swift" => "text/x-swift",
        "go" => "text/x-go",
        "rb" => "text/x-ruby",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "sh" | "bash" => "text/x-sh",
        "sql" => "text/x-sql",
        "xml" => "application/xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        _ => "application/octet-stream",
    };

    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);

    Ok(AttachmentData {
        name: file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string(),
        media_type: media_type.to_string(),
        data,
        size: bytes.len(),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentData {
    pub name: String,
    pub media_type: String,
    pub data: String,
    pub size: usize,
}

/// Lists one directory level, respecting .gitignore of the enclosing repo.
/// Directories first, then files, both alphabetical.
#[tauri::command]
pub fn list_dir(path: String) -> Result<Vec<DirEntry>, String> {
    let dir = Path::new(&path);
    if !dir.is_dir() {
        return Err(format!("not a directory: {path}"));
    }

    let walker = ignore::WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .build();

    let mut entries: Vec<DirEntry> = walker
        .filter_map(|e| e.ok())
        // depth 0 is the directory itself
        .filter(|e| e.depth() == 1)
        .map(|e| DirEntry {
            name: e.file_name().to_string_lossy().into_owned(),
            path: e.path().to_string_lossy().into_owned(),
            is_dir: e.file_type().map(|t| t.is_dir()).unwrap_or(false),
        })
        .collect();

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(entries)
}

/// Recursively walks a directory tree respecting .gitignore, returning a flat
/// list of relative paths (from `root`) for files and folders. Skips hidden
/// files and directories. Used by the frontend to build a fuzzy-searchable
/// index for @-mention autocomplete.
#[tauri::command]
pub fn walk_dir(root: String) -> Result<Vec<WalkEntry>, String> {
    let dir = Path::new(&root);
    if !dir.is_dir() {
        return Err(format!("not a directory: {root}"));
    }

    let walker = ignore::WalkBuilder::new(dir)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .build();

    let mut entries: Vec<WalkEntry> = walker
        .filter_map(|e| e.ok())
        // depth 0 is the root itself, skip it
        .filter(|e| e.depth() > 0)
        .map(|e| {
            let abs_path = e.path();
            let rel = abs_path
                .strip_prefix(&root)
                .unwrap_or(abs_path)
                .to_string_lossy()
                .into_owned();
            WalkEntry {
                path: rel,
                is_dir: e.file_type().map(|t| t.is_dir()).unwrap_or(false),
            }
        })
        .collect();

    entries.sort_by(|a, b| a.is_dir.cmp(&b.is_dir).reverse().then(a.path.cmp(&b.path)));
    Ok(entries)
}

const MAX_READ_BYTES: u64 = 2 * 1024 * 1024;

/// Reject a path that is not inside one of the currently open workspaces.
///
/// The webview renders untrusted content — model output, and file contents the
/// agent quotes back into the chat. `lib/markdown.ts` sanitizes that before it
/// becomes DOM, but sanitization is one layer: any script that does run in the
/// webview can call every registered Tauri command, and these commands are the
/// ones that reach the filesystem. Without this check, `write_file` is an
/// unguarded `std::fs::write` to any absolute path — enough to append to a shell
/// rc file and get code execution on the user's next terminal.
///
/// The agent's own file tools enforce the same rule via
/// `agent::tools::validate_path`; both go through `crate::workspace_path` so the
/// two can't drift.
async fn ensure_in_open_workspace(
    path: &str,
    state: &tauri::State<'_, crate::state::AppState>,
) -> Result<(), String> {
    let roots: Vec<std::path::PathBuf> = state.workspaces.lock().await.keys().cloned().collect();
    if roots.is_empty() {
        return Err(format!(
            "cannot access '{path}': no workspace is open. File access is restricted to the open project."
        ));
    }
    if roots
        .iter()
        .any(|root| crate::workspace_path::is_within_root(Path::new(path), root))
    {
        return Ok(());
    }
    Err(format!(
        "path '{path}' is outside every open workspace. File access is restricted to the open project."
    ))
}

#[tauri::command]
pub async fn read_file(
    path: String,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<String, String> {
    ensure_in_open_workspace(&path, &state).await?;
    let file = Path::new(&path);
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("not a file: {path}"));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(format!("file too large to open ({} bytes)", meta.len()));
    }
    std::fs::read_to_string(file).map_err(|e| format!("cannot read as text: {e}"))
}

#[tauri::command]
pub async fn write_file(
    path: String,
    content: String,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<(), String> {
    ensure_in_open_workspace(&path, &state).await?;
    std::fs::write(&path, &content).map_err(|e| format!("cannot write {path}: {e}"))
}

/// Write binary content (base64-encoded) inside the workspace. Exports that
/// legitimately land outside it (a diagram saved to the Desktop) go through
/// `export_file` / `export_file_bytes`, which own the save dialog.
#[tauri::command]
pub async fn write_file_bytes(
    path: String,
    base64_data: String,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<(), String> {
    ensure_in_open_workspace(&path, &state).await?;
    let bytes = decode_base64(&base64_data)?;
    std::fs::write(&path, &bytes).map_err(|e| format!("cannot write {path}: {e}"))
}

fn decode_base64(data: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|e| format!("invalid base64: {e}"))
}

/// Ask the user where to save an export, then write it there.
///
/// The save dialog runs here rather than in the frontend on purpose: it means
/// the destination is chosen by the user in a native dialog and never travels
/// across IPC as an attacker-suppliable argument. That is what lets `write_file`
/// stay strictly workspace-scoped without losing "export diagram to Desktop".
/// Returns the chosen path, or `None` if the user cancelled.
async fn save_dialog(
    app: &tauri::AppHandle,
    default_name: &str,
    filter_name: &str,
    extension: &str,
) -> Option<std::path::PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(default_name)
        .add_filter(filter_name, &[extension])
        .save_file(move |picked| {
            let _ = tx.send(picked);
        });
    rx.await.ok().flatten().and_then(|p| p.into_path().ok())
}

#[tauri::command]
pub async fn export_file(
    app: tauri::AppHandle,
    default_name: String,
    filter_name: String,
    extension: String,
    content: String,
) -> Result<bool, String> {
    let Some(path) = save_dialog(&app, &default_name, &filter_name, &extension).await else {
        return Ok(false);
    };
    std::fs::write(&path, &content).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(true)
}

#[tauri::command]
pub async fn export_file_bytes(
    app: tauri::AppHandle,
    default_name: String,
    filter_name: String,
    extension: String,
    base64_data: String,
) -> Result<bool, String> {
    let bytes = decode_base64(&base64_data)?;
    let Some(path) = save_dialog(&app, &default_name, &filter_name, &extension).await else {
        return Ok(false);
    };
    std::fs::write(&path, &bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(true)
}
