use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedSymbol {
    pub name: String,
    pub kind: String,
    pub signature: Option<String>,
    pub start_line: i64,
    pub start_col: i64,
    pub end_line: i64,
    pub end_col: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedCall {
    pub from_name: String,
    pub to_name: String,
    pub from_line: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParseResult {
    pub language: String,
    pub symbols: Vec<ParsedSymbol>,
    pub calls: Vec<ParsedCall>,
    pub error: Option<String>,
}

pub fn detect_language(path: &str) -> Option<&'static str> {
    let ext = Path::new(path).extension()?.to_str()?;
    match ext {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some("typescript"),
        "rs" => Some("rust"),
        "py" => Some("python"),
        "swift" => Some("swift"),
        _ => None,
    }
}

fn get_language(lang: &str) -> Result<tree_sitter::Language, String> {
    match lang {
        "typescript" => Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "rust" => Ok(tree_sitter_rust::LANGUAGE.into()),
        "python" => Ok(tree_sitter_python::LANGUAGE.into()),
        "swift" => Err("swift grammar not loaded".into()),
        _ => Err(format!("unknown language: {lang}")),
    }
}

pub fn parse_file(path: &str, content: &str) -> ParseResult {
    let lang = match detect_language(path) {
        Some(l) => l,
        None => {
            return ParseResult {
                language: "unknown".into(),
                symbols: vec![],
                calls: vec![],
                error: Some("unsupported language".into()),
            }
        }
    };

    let ts_language = match get_language(lang) {
        Ok(l) => l,
        Err(e) => {
            return ParseResult {
                language: lang.into(),
                symbols: vec![],
                calls: vec![],
                error: Some(e),
            }
        }
    };

    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&ts_language).is_err() {
        return ParseResult {
            language: lang.into(),
            symbols: vec![],
            calls: vec![],
            error: Some("failed to set language".into()),
        };
    }

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => {
            return ParseResult {
                language: lang.into(),
                symbols: vec![],
                calls: vec![],
                error: Some("parse failed".into()),
            }
        }
    };
    let root = tree.root_node();

    let mut symbols: Vec<ParsedSymbol> = Vec::new();
    let mut calls: Vec<ParsedCall> = Vec::new();
    let mut cursor = root.walk();
    let mut done = false;

    loop {
        if done {
            break;
        }
        let node = cursor.node();
        let kind = node.kind();

        match kind {
            "function_declaration" | "method_definition" | "class_declaration"
            | "function_item" | "struct_item" | "enum_item"
            | "function_definition" | "class_definition"
            | "interface_declaration" | "type_alias_declaration"
            | "trait_item" | "protocol_declaration"
            | "struct_declaration" | "enum_declaration" => {
                let name_node = node.child_by_field_name("name");
                if let Some(name_node) = name_node {
                    let name = name_node.utf8_text(content.as_bytes()).unwrap_or("?").to_string();
                    let start = node.start_position();
                    let end = node.end_position();
                    let sig_text = get_node_text(content, &node, 80);

                    symbols.push(ParsedSymbol {
                        name,
                        kind: kind.into(),
                        signature: Some(sig_text),
                        start_line: start.row as i64 + 1,
                        start_col: start.column as i64 + 1,
                        end_line: end.row as i64 + 1,
                        end_col: end.column as i64 + 1,
                    });
                }
            }

            "call_expression" => {
                let func_node = node.child_by_field_name("function");
                if let Some(func) = func_node {
                    let called_name = func.utf8_text(content.as_bytes()).unwrap_or("?").to_string();
                    if !called_name.starts_with('"') && !called_name.starts_with('\'') {
                        let start = node.start_position();
                        let containing = find_containing_function_name(&node, content)
                            .unwrap_or_default();
                        if !containing.is_empty() && called_name != containing {
                            calls.push(ParsedCall {
                                from_name: containing,
                                to_name: called_name,
                                from_line: start.row as i64 + 1,
                            });
                        }
                    }
                }
            }

            "use_declaration" | "import_statement" | "import_from_statement"
            | "import_declaration" => {
                let start = node.start_position();
                let end = node.end_position();
                let sig_text = get_node_text(content, &node, 100);
                let text = node.utf8_text(content.as_bytes()).unwrap_or("");

                let name = if kind == "use_declaration" {
                    text.split("::").last().unwrap_or(text).trim().to_string()
                } else if text.starts_with("import ") {
                    text.split_whitespace().nth(1).unwrap_or("import").to_string()
                } else {
                    text.split_whitespace().nth(1).unwrap_or("import").to_string()
                };

                symbols.push(ParsedSymbol {
                    name,
                    kind: "import".into(),
                    signature: Some(sig_text),
                    start_line: start.row as i64 + 1,
                    start_col: start.column as i64 + 1,
                    end_line: end.row as i64 + 1,
                    end_col: end.column as i64 + 1,
                });
            }

            _ => {}
        }

        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                done = true;
                break;
            }
        }
    }

    ParseResult {
        language: lang.into(),
        symbols,
        calls,
        error: None,
    }
}

fn find_containing_function_name(
    node: &tree_sitter::Node,
    content: &str,
) -> Option<String> {
    let mut current = node.parent()?;
    loop {
        let k = current.kind();
        if matches!(
            k,
            "function_declaration" | "method_definition" | "function_item"
                | "function_definition" | "closure_expression"
        ) {
            if let Some(n) = current.child_by_field_name("name") {
                return n.utf8_text(content.as_bytes()).ok().map(|s| s.to_string());
            }
        }
        current = current.parent()?;
    }
}

fn get_node_text<'a>(content: &'a str, node: &tree_sitter::Node, max_len: usize) -> String {
    let text = node.utf8_text(content.as_bytes()).unwrap_or("");
    if text.len() <= max_len {
        text.to_string()
    } else {
        let end = text.char_indices().nth(max_len).map(|(i, _)| i).unwrap_or(text.len());
        format!("{}...", &text[..end])
    }
}
