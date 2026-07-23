use crate::procutil;
use serde::Serialize;
use serde_json::Value;
use std::io::{BufRead, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct LspClient {
    process: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    next_id: AtomicU64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

#[derive(Debug, Clone, Serialize)]
pub struct Range {
    pub start_line: u64,
    pub start_char: u64,
    pub end_line: u64,
    pub end_char: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HoverResult {
    pub contents: String,
    pub range: Option<Range>,
}

impl LspClient {
    pub fn spawn(server_path: &str, workspace_uri: &str) -> Result<Self, String> {
        let mut cmd = Command::new(server_path);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        procutil::no_window(&mut cmd);
        let mut process = cmd
            .spawn()
            .map_err(|e| format!("spawn {server_path}: {e}"))?;

        let stdin = process.stdin.take().ok_or("no stdin")?;
        let stdout = process.stdout.take().ok_or("no stdout")?;

        let mut client = LspClient {
            process,
            stdin,
            stdout,
            next_id: AtomicU64::new(1),
        };

        client.initialize(workspace_uri)?;
        client.initialized()?;

        Ok(client)
    }

    fn send_request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_message(&msg)?;
        self.read_response(id)
    }

    fn send_notification(&mut self, method: &str, params: Value) -> Result<(), String> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_message(&msg)
    }

    fn write_message(&mut self, msg: &Value) -> Result<(), String> {
        let json = serde_json::to_string(msg).map_err(|e| format!("serialize: {e}"))?;
        let header = format!("Content-Length: {}\r\n\r\n", json.len());
        self.stdin
            .write_all(header.as_bytes())
            .map_err(|e| format!("write header: {e}"))?;
        self.stdin
            .write_all(json.as_bytes())
            .map_err(|e| format!("write body: {e}"))?;
        self.stdin.flush().map_err(|e| format!("flush: {e}"))
    }

    fn read_response(&mut self, expected_id: u64) -> Result<Value, String> {
        let mut reader = std::io::BufReader::new(&mut self.stdout);
        let mut content_length: Option<usize> = None;

        loop {
            let mut line = String::new();
            let bytes_read = reader
                .read_line(&mut line)
                .map_err(|e| format!("read header: {e}"))?;

            if bytes_read == 0 {
                return Err("missing Content-Length (server closed connection)".into());
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                if content_length.is_some() {
                    break;
                }
                continue;
            }
            if let Some(len_str) = trimmed.strip_prefix("Content-Length: ") {
                content_length = Some(
                    len_str
                        .parse::<usize>()
                        .map_err(|e| format!("parse Content-Length: {e}"))?,
                );
            }
        }

        let len = content_length.ok_or("missing Content-Length")?;
        let mut buf = vec![0u8; len];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("read body: {e}"))?;

        let response: Value =
            serde_json::from_slice(&buf).map_err(|e| format!("parse response: {e}"))?;

        if let Some(err) = response.get("error") {
            return Err(format!("LSP error: {err}"));
        }

        if let Some(id) = response.get("id").and_then(|v| v.as_u64()) {
            if id == expected_id {
                Ok(response.get("result").cloned().unwrap_or(Value::Null))
            } else {
                Err(format!(
                    "unexpected response id {id}, expected {expected_id}"
                ))
            }
        } else {
            Err("response missing id".into())
        }
    }

    fn initialize(&mut self, workspace_uri: &str) -> Result<(), String> {
        let params = serde_json::json!({
            "processId": null,
            "capabilities": {
                "textDocument": {
                    "definition": { "dynamicRegistration": false },
                    "references": { "dynamicRegistration": false },
                    "hover": { "dynamicRegistration": false },
                    "synchronization": {
                        "didOpen": true,
                        "didChange": true,
                        "willSave": false,
                        "willSaveWaitUntil": false,
                        "didClose": true
                    }
                },
                "workspace": {
                    "workspaceFolders": true
                }
            },
            "workspaceFolders": [
                { "uri": workspace_uri, "name": "root" }
            ]
        });
        self.send_request("initialize", params)?;
        Ok(())
    }

    fn initialized(&mut self) -> Result<(), String> {
        self.send_notification("initialized", serde_json::json!({}))
    }

    pub fn goto_definition(
        &mut self,
        uri: &str,
        line: u64,
        character: u64,
    ) -> Result<Vec<Location>, String> {
        let result = self.send_request(
            "textDocument/definition",
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }),
        )?;

        let locations = parse_locations(result);
        Ok(locations)
    }

    pub fn find_references(
        &mut self,
        uri: &str,
        line: u64,
        character: u64,
    ) -> Result<Vec<Location>, String> {
        let result = self.send_request(
            "textDocument/references",
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
                "context": { "includeDeclaration": true }
            }),
        )?;

        let locations = parse_locations(result);
        Ok(locations)
    }

    pub fn hover(
        &mut self,
        uri: &str,
        line: u64,
        character: u64,
    ) -> Result<Option<HoverResult>, String> {
        let result = self.send_request(
            "textDocument/hover",
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }),
        )?;

        if result.is_null() {
            return Ok(None);
        }

        let contents = extract_hover_contents(&result)?;
        let range = result.get("range").map(|r| Range {
            start_line: r["start"]["line"].as_u64().unwrap_or(0),
            start_char: r["start"]["character"].as_u64().unwrap_or(0),
            end_line: r["end"]["line"].as_u64().unwrap_or(0),
            end_char: r["end"]["character"].as_u64().unwrap_or(0),
        });

        Ok(Some(HoverResult { contents, range }))
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

fn parse_locations(result: Value) -> Vec<Location> {
    let locations = match result {
        Value::Array(arr) => arr,
        Value::Object(_) => vec![result],
        _ => return vec![],
    };

    locations
        .into_iter()
        .filter_map(|loc| {
            let uri = loc.get("uri").and_then(|u| u.as_str())?.to_string();
            let range = loc.get("range")?;
            Some(Location {
                uri,
                range: Range {
                    start_line: range["start"]["line"].as_u64().unwrap_or(0),
                    start_char: range["start"]["character"].as_u64().unwrap_or(0),
                    end_line: range["end"]["line"].as_u64().unwrap_or(0),
                    end_char: range["end"]["character"].as_u64().unwrap_or(0),
                },
            })
        })
        .collect()
}

fn extract_hover_contents(result: &Value) -> Result<String, String> {
    let contents = result.get("contents").ok_or("missing hover contents")?;
    match contents {
        Value::String(s) => Ok(s.clone()),
        Value::Array(arr) => {
            let mut parts = Vec::new();
            for item in arr {
                let text = match item {
                    Value::String(s) => s.clone(),
                    Value::Object(m) => m
                        .get("value")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    _ => continue,
                };
                parts.push(text);
            }
            Ok(parts.join("\n\n"))
        }
        Value::Object(m) => {
            if let Some(value) = m.get("value").and_then(|v| v.as_str()) {
                Ok(value.to_string())
            } else if let Some(kind) = m.get("kind").and_then(|v| v.as_str()) {
                let value = m.get("value").and_then(|v| v.as_str()).unwrap_or("");
                Ok(format!("```{kind}\n{value}\n```"))
            } else {
                Ok(serde_json::to_string(contents).unwrap_or_default())
            }
        }
        _ => Ok(serde_json::to_string(contents).unwrap_or_default()),
    }
}
