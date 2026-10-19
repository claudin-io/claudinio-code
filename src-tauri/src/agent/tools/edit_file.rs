use serde::Deserialize;
use serde::Serialize;

#[derive(Deserialize)]
pub struct EditFileArgs {
    #[serde(alias = "file_path")]
    pub path: String,
    /// `""` creates `path`, which must not exist yet (lean-harness Phase 3b).
    pub old_string: String,
    pub new_string: String,
    /// Replace every occurrence; otherwise `old_string` must occur once.
    #[serde(default)]
    pub replace_all: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditDiff {
    pub path: String,
    pub old_string: String,
    pub new_string: String,
    pub unified_diff: String,
    pub applied: bool,
    pub replace_all: bool,
}

impl EditDiff {
    /// An empty `old_string` is a creation: an existing file is refused
    /// before a proposal is made, so a proposal with one always creates.
    pub fn creates(&self) -> bool {
        self.old_string.is_empty()
    }
}

/// The proposal for `args`, read from the disk as it is now.
///
/// A path that does not exist, with an empty `old_string`, proposes creating
/// it: the diff runs from nothing to `new_string`. Before Phase 3b nothing
/// could create a file — `read_file` errors on a missing path, the read gate
/// then refuses the edit, and this function read the file anyway — so every
/// failure of the clean baseline eval (2026-10-18) was a new file the model
/// could not write, and in the app a model fell back to a shell heredoc that
/// needs an approval and shows no diff.
pub fn preview(args: &EditFileArgs) -> Result<EditDiff, String> {
    let content = match std::fs::read_to_string(&args.path) {
        // Creating never overwrites; and `content.replace("", new)` would put
        // `new` between every character of the file.
        Ok(_) if args.old_string.is_empty() => {
            return Err(crate::agent::gate_text::create_over_existing(&args.path));
        }
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !args.old_string.is_empty() {
                return Err(crate::agent::gate_text::edit_target_missing(&args.path));
            }
            String::new()
        }
        Err(e) => return Err(format!("cannot read {}: {e}", args.path)),
    };

    let modified = if args.old_string.is_empty() {
        args.new_string.clone()
    } else {
        replace(&content, &args.path, &args.old_string, &args.new_string, args.replace_all)
            .map_err(|e| match e {
                Miss::NotFound => format!(
                    "old_string not found in {}.\nExpected:\n{}\n\nTo see the file content, use read_file first.",
                    args.path, args.old_string
                ),
                Miss::Ambiguous(message) => message,
            })?
    };

    let unified = diffy::create_patch(&content, &modified).to_string();

    Ok(EditDiff {
        path: args.path.clone(),
        old_string: args.old_string.clone(),
        new_string: args.new_string.clone(),
        unified_diff: unified,
        applied: false,
        replace_all: args.replace_all,
    })
}

enum Miss {
    NotFound,
    Ambiguous(String),
}

/// `content` with `old` replaced once — or everywhere with `replace_all` —
/// refusing an `old` that occurs more than once otherwise. `str::replace`
/// alone replaced every occurrence while the description promised one, so a
/// short `old_string` silently changed places the model never looked at.
fn replace(
    content: &str,
    path: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<String, Miss> {
    match content.matches(old).count() {
        0 => Err(Miss::NotFound),
        1 => Ok(content.replacen(old, new, 1)),
        _ if replace_all => Ok(content.replace(old, new)),
        n => Err(Miss::Ambiguous(crate::agent::gate_text::ambiguous_match(
            path, n,
        ))),
    }
}

pub fn apply(diff: &EditDiff) -> Result<String, String> {
    if diff.creates() {
        return create(diff);
    }
    let content = std::fs::read_to_string(&diff.path)
        .map_err(|e| format!("cannot read {}: {e}", diff.path))?;

    // Counted again on the file as it is now: the approval can come long
    // after the proposal.
    let new_content = replace(
        &content,
        &diff.path,
        &diff.old_string,
        &diff.new_string,
        diff.replace_all,
    )
    .map_err(|e| match e {
        Miss::NotFound => format!(
            "old_string not found in {} (file may have changed since preview)",
            diff.path
        ),
        Miss::Ambiguous(message) => message,
    })?;
    std::fs::write(&diff.path, &new_content)
        .map_err(|e| format!("cannot write {}: {e}", diff.path))?;

    Ok(format!("Applied edit to {}", diff.path))
}

/// Write a new file, making the folders it needs. `create_new` makes the
/// "never an overwrite" rule hold even when something wrote the file between
/// the proposal and the approval.
fn create(diff: &EditDiff) -> Result<String, String> {
    use std::io::Write;
    let path = std::path::Path::new(&diff.path);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create the folders for {}: {e}", diff.path))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => {
                crate::agent::gate_text::create_over_existing(&diff.path)
            }
            _ => format!("cannot create {}: {e}", diff.path),
        })?;
    file.write_all(diff.new_string.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", diff.path))?;
    Ok(format!("Created {}", diff.path))
}
