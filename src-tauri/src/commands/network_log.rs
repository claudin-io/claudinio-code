use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub workspace: String,
    pub timestamp: String,
    pub source: String,
    pub detail: String,
    pub duration_ms: u64,
    pub bytes: u64,
    pub status_code: Option<u16>,
}

#[tauri::command]
pub fn get_network_log(workspace: String) -> Result<Vec<LogEntry>, String> {
    let path = crate::net_activity::csv_path();
    if !path.exists() {
        return Ok(vec![]);
    }
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(&path)
        .map_err(|e| e.to_string())?;

    let mut entries: Vec<LogEntry> = Vec::new();
    for result in rdr.records() {
        let record = result.map_err(|e| e.to_string())?;
        if record.get(0).unwrap_or("") != workspace {
            continue;
        }
        entries.push(LogEntry {
            workspace: record.get(0).unwrap_or("").to_string(),
            timestamp: record.get(1).unwrap_or("").to_string(),
            source: record.get(2).unwrap_or("").to_string(),
            detail: record.get(3).unwrap_or("").to_string(),
            duration_ms: record.get(4).unwrap_or("0").parse().unwrap_or(0),
            bytes: record.get(5).unwrap_or("0").parse().unwrap_or(0),
            status_code: record.get(6).and_then(|s| s.parse().ok()),
        });
    }
    // Return last 100, newest first
    entries.reverse();
    entries.truncate(100);
    Ok(entries)
}
