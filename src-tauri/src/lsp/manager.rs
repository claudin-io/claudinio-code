use crate::lsp::client::{HoverResult, Location, LspClient};
use std::collections::HashMap;
use std::path::Path;

const TSSERVER: &str = "typescript-language-server";
const RUST_ANALYZER: &str = "rust-analyzer";

pub struct LspManager {
    servers: HashMap<String, LspServerInstance>,
}

struct LspServerInstance {
    client: LspClient,
}

impl LspManager {
    pub fn new() -> Self {
        LspManager {
            servers: HashMap::new(),
        }
    }

    pub fn start_for_workspace(
        &mut self,
        workspace_root: &str,
    ) -> Result<(), String> {
        self.start_tsserver(workspace_root)?;
        self.start_rust_analyzer(workspace_root)?;
        Ok(())
    }

    fn start_tsserver(&mut self, workspace_root: &str) -> Result<(), String> {
        if self.servers.contains_key("typescript") {
            return Ok(());
        }
        let uri = path_to_uri(workspace_root);
        match LspClient::spawn(TSSERVER, &uri) {
            Ok(client) => {
                self.servers.insert(
                    "typescript".into(),
                    LspServerInstance { client },
                );
                Ok(())
            }
            Err(e) => {
                eprintln!("tsserver failed to start (optional): {e}");
                Ok(())
            }
        }
    }

    fn start_rust_analyzer(&mut self, workspace_root: &str) -> Result<(), String> {
        if self.servers.contains_key("rust") {
            return Ok(());
        }
        let uri = path_to_uri(workspace_root);
        match LspClient::spawn(RUST_ANALYZER, &uri) {
            Ok(client) => {
                self.servers.insert(
                    "rust".into(),
                    LspServerInstance { client },
                );
                Ok(())
            }
            Err(e) => {
                eprintln!("rust-analyzer failed to start (optional): {e}");
                Ok(())
            }
        }
    }

    pub fn lsp_key(file_path: &str) -> Option<&'static str> {
        let ext = Path::new(file_path).extension()?.to_str()?;
        match ext {
            "ts" | "tsx" | "js" | "jsx" => Some("typescript"),
            "rs" => Some("rust"),
            _ => None,
        }
    }

    pub fn get_uri(file_path: &str) -> String {
        path_to_uri(file_path)
    }

    pub fn goto_definition(
        &mut self,
        file_path: &str,
        line: u64,
        character: u64,
    ) -> Result<Vec<Location>, String> {
        let key = Self::lsp_key(file_path).ok_or("no LSP for this file type")?;
        let instance = self.servers.get_mut(key).ok_or("LSP server not running")?;
        let uri = Self::get_uri(file_path);
        instance.client.goto_definition(&uri, line, character)
    }

    pub fn find_references(
        &mut self,
        file_path: &str,
        line: u64,
        character: u64,
    ) -> Result<Vec<Location>, String> {
        let key = Self::lsp_key(file_path).ok_or("no LSP for this file type")?;
        let instance = self.servers.get_mut(key).ok_or("LSP server not running")?;
        let uri = Self::get_uri(file_path);
        instance.client.find_references(&uri, line, character)
    }

    pub fn hover(
        &mut self,
        file_path: &str,
        line: u64,
        character: u64,
    ) -> Result<Option<HoverResult>, String> {
        let key = Self::lsp_key(file_path).ok_or("no LSP for this file type")?;
        let instance = self.servers.get_mut(key).ok_or("LSP server not running")?;
        let uri = Self::get_uri(file_path);
        instance.client.hover(&uri, line, character)
    }
}

fn path_to_uri(path: &str) -> String {
    let abs = std::path::absolute(path)
        .unwrap_or_else(|_| std::path::PathBuf::from(path))
        .to_string_lossy()
        .replace('\\', "/");

    if abs.starts_with('/') {
        format!("file://{abs}")
    } else {
        format!("file:///{abs}")
    }
}
