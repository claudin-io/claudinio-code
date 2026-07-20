use serde::Deserialize;
use serde::Serialize;

#[derive(Deserialize)]
pub struct EditFileArgs {
    #[serde(alias = "file_path")]
    pub path: String,
    pub old_string: String,
    pub new_string: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditDiff {
    pub path: String,
    pub old_string: String,
    pub new_string: String,
    pub unified_diff: String,
    pub applied: bool,
}

pub fn preview(args: &EditFileArgs) -> Result<EditDiff, String> {
    let content = std::fs::read_to_string(&args.path)
        .map_err(|e| format!("cannot read {}: {e}", args.path))?;

    if !content.contains(&args.old_string) {
        return Err(format!(
            "old_string not found in {}.\nExpected:\n{}\n\nTo see the file content, use read_file first.",
            args.path, args.old_string
        ));
    }

    let unified = diffy::create_patch(
        &content,
        &content.replace(&args.old_string, &args.new_string),
    )
    .to_string();

    Ok(EditDiff {
        path: args.path.clone(),
        old_string: args.old_string.clone(),
        new_string: args.new_string.clone(),
        unified_diff: unified,
        applied: false,
    })
}

pub fn apply(diff: &EditDiff) -> Result<String, String> {
    let content = std::fs::read_to_string(&diff.path)
        .map_err(|e| format!("cannot read {}: {e}", diff.path))?;

    if !content.contains(&diff.old_string) {
        return Err(format!(
            "old_string not found in {} (file may have changed since preview)",
            diff.path
        ));
    }

    let new_content = content.replace(&diff.old_string, &diff.new_string);
    std::fs::write(&diff.path, &new_content)
        .map_err(|e| format!("cannot write {}: {e}", diff.path))?;

    Ok(format!("Applied edit to {}", diff.path))
}
