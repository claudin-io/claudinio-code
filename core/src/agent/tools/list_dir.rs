use serde::Deserialize;
use serde::Serialize;

#[derive(Deserialize)]
pub struct ListDirArgs {
    #[serde(alias = "file_path")]
    pub path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntryInfo {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

pub fn execute(args: ListDirArgs) -> Result<Vec<DirEntryInfo>, String> {
    let dir = std::path::Path::new(&args.path);
    if !dir.is_dir() {
        return Err(format!("not a directory: {}", args.path));
    }

    let walker = ignore::WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .build();

    let mut entries: Vec<DirEntryInfo> = walker
        .filter_map(|e| e.ok())
        .filter(|e| e.depth() == 1)
        .map(|e| DirEntryInfo {
            name: e.file_name().to_string_lossy().into_owned(),
            path: e.path().to_string_lossy().into_owned(),
            is_dir: e.file_type().map(|t| t.is_dir()).unwrap_or(false),
        })
        .collect();

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(entries)
}
