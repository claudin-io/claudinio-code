use serde::Serialize;
use std::path::Path;

use regex::Regex;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedSymbol {
    pub name: String,
    pub kind: String,
    pub parent_context: Option<String>,
    pub signature: Option<String>,
    pub doc_comment: Option<String>,
    pub body_text: Option<String>,
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

/// Map a file extension or base filename to a language key.
pub fn detect_language(path: &str) -> Option<&'static str> {
    let p = Path::new(path);
    let file_name = p.file_name()?.to_str()?;

    // Match by exact filename (case-sensitive for common build files)
    match file_name {
        "CMakeLists.txt" => return Some("cmake"),
        "Makefile" | "makefile" | "GNUmakefile" => return Some("make"),
        "Dockerfile" | "Containerfile" => return Some("dockerfile"),
        "Earthfile" => return Some("earthfile"),
        "go.mod" => return Some("gomod"),
        "Kconfig" | "Kconfig.defconfig" => return Some("kconfig"),
        "nginx.conf" | "nginx.conf.template" => return Some("nginx"),
        _ => {}
    }

    let ext = match p.extension() {
        Some(e) => e.to_str()?,
        None => return None,
    };

    match ext {
        // Ada
        "ada" | "ads" | "adb" => Some("ada"),
        // Agda
        "agda" | "lagda" => Some("agda"),
        // Assembly
        "asm" | "s" | "S" => Some("asm"),
        // Bash / Shell
        "sh" | "bash" | "zsh" | "ksh" | "dash" | "bashrc" | "profile" | "env" | "aliases"
        | "zshrc" | "zprofile" | "zshenv" | "zlogin" | "zlogout" => Some("bash"),
        // Bicep
        "bicep" => Some("bicep"),
        // C
        "c" | "h" => Some("c"),
        // Clojure
        "clj" | "cljs" | "cljc" | "edn" => Some("clojure"),
        // CMake
        "cmake" => Some("cmake"),
        // Common Lisp
        "lisp" | "cl" | "lsp" => Some("commonlisp"),
        // C++
        "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" | "ixx" => Some("cpp"),
        // C#
        "cs" => Some("c-sharp"),
        // CSS
        "css" => Some("css"),
        // SCSS/SASS route to the regex fallback scanner (grammar disabled by
        // the old-API conflict below); indented .sass degrades gracefully.
        "scss" | "sass" => Some("scss"),
        // CUDA
        "cu" | "cuh" => Some("cuda"),
        // D
        "d" => Some("d"),
        // Dart
        "dart" => Some("dart"),
        // DOT / Graphviz
        "dot" | "gv" => Some("dot"),
        // Elixir
        "ex" | "exs" => Some("elixir"),
        // Elm
        "elm" => Some("elm"),
        // Embedded template (ERB, EJS)
        "erb" | "ejs" => Some("embedded-template"),
        // Erlang
        "erl" | "hrl" => Some("erlang"),
        // Fish
        "fish" => Some("fish"),
        // Fortran
        "f" | "f90" | "f95" | "f03" | "f08" | "for" => Some("fortran"),
        // F#
        "fs" | "fsx" | "fsi" => Some("fsharp"),
        // Gleam
        "gleam" => Some("gleam"),
        // GLSL
        "glsl" | "vert" | "frag" | "geom" | "comp" | "tesc" | "tese" | "rgen" | "rchit"
        | "rmiss" | "rahit" | "rint" | "call" => Some("glsl"),
        // Go
        "go" => Some("go"),
        // GraphQL
        "graphql" | "gql" => Some("graphql"),
        // Haskell
        "hs" | "lhs" => Some("haskell"),
        // HCL / Terraform
        "hcl" | "tf" | "tfvars" => Some("hcl"),
        // HEEx
        "heex" => Some("heex"),
        // HLSL
        "hlsl" | "fx" | "fxh" | "hlsli" => Some("hlsl"),
        // HTML
        "html" | "htm" => Some("html"),
        // INI / config
        "ini" | "cfg" => Some("ini"),
        // TOML (no grammar wired — handled by the fallback section scanner)
        "toml" => Some("toml"),
        // Java
        "java" => Some("java"),
        // JavaScript (also handled by typescript grammar); JSX needs the TSX
        // grammar — the plain TS grammar turns JSX into ERROR nodes and the
        // components' bodies become invisible to the index.
        "js" | "mjs" | "cjs" => Some("typescript"),
        "jsx" => Some("tsx"),
        // JSON
        "json" | "jsonc" => Some("json"),
        // Julia
        "jl" => Some("julia"),
        // Kotlin
        "kt" | "kts" | "ktm" => Some("kotlin"),
        // Less
        "less" => Some("less"),
        // LLVM IR
        "ll" => Some("llvm"),
        // Lua
        "lua" => Some("lua"),
        // Make
        "mk" | "mak" => Some("make"),
        // MATLAB
        "m" => {
            // .m can be MATLAB or Objective-C; check file content hints
            // Default to MATLAB (user can override via frontmatter later)
            Some("matlab")
        }
        // Nickel
        "ncl" => Some("nickel"),
        // Nix
        "nix" => Some("nix"),
        // Objective-C
        "mm" => Some("objc"),
        // OCaml
        "ml" | "mli" | "mly" => Some("ocaml"),
        // OCamllex
        "mll" => Some("ocamllex"),
        // Odin
        "odin" => Some("odin"),
        // Org mode
        "org" => Some("org"),
        // Perl
        "pl" | "pm" | "t" => Some("perl"),
        // PHP
        "php" | "phtml" | "php3" | "php4" | "php5" | "php7" | "phps" => Some("php"),
        // PowerShell
        "ps1" | "psm1" | "psd1" | "ps1xml" => Some("powershell"),
        // Prisma
        "prisma" => Some("prisma-io"),
        // Prolog
        "pro" | "P" => Some("prolog"),
        // Protocol Buffers
        "proto" => Some("proto"),
        // Python
        "py" | "pyw" | "pyx" | "pxd" | "pxi" => Some("python"),
        // R
        "r" | "R" | "Rmd" => Some("r"),
        // Racket
        "rkt" | "scrbl" | "rktd" => Some("racket"),
        // Ruby
        "rb" | "ruby" => Some("ruby"),
        // Rust
        "rs" => Some("rust"),
        // Scala
        "scala" | "sc" => Some("scala"),
        // Scheme
        "scm" | "ss" => Some("scheme"),
        // Slint
        "slint" => Some("slint"),
        // Solidity
        "sol" => Some("solidity"),
        // SPARQL
        "rq" | "sparql" => Some("sparql"),
        // Swift
        "swift" => Some("swift"),
        // SystemVerilog
        "sv" | "svh" => Some("systemverilog"),
        // TypeScript / TSX (separate grammars: `<T>` casts parse differently)
        "ts" => Some("typescript"),
        "tsx" => Some("tsx"),
        // Verilog
        "v" | "vh" => Some("verilog"),
        // VHDL
        "vhd" | "vhdl" => Some("vhdl"),
        // XML
        "xml" | "xsd" | "xslt" | "svg" | "plist" | "rss" | "atom" | "xaml" => Some("xml"),
        // YAML
        "yaml" | "yml" => Some("yaml"),
        // Zig
        "zig" => Some("zig"),
        // Java properties
        "properties" => Some("properties"),

        _ => None,
    }
}

/// Detect the language for documentation/text files.
pub fn detect_doc_language(path: &str) -> Option<&'static str> {
    let p = std::path::Path::new(path);
    let ext = match p.extension() {
        Some(e) => e.to_str()?,
        None => return None,
    };
    match ext {
        "md" | "mdx" => Some("markdown"),
        "txt" => Some("text"),
        _ => None,
    }
}

/// Parse a documentation file (markdown or text) into chunked sections.
/// Produces one `ParsedSymbol` per heading section (levels 1–6). Indexing H1
/// captures the document title and the preamble before the first H2 — the
/// part of a README that says what the project IS.
/// Falls back to a single symbol using the file stem if no headings are found.
pub fn parse_doc_file(path: &str, content: &str) -> Vec<ParsedSymbol> {
    const MAX_BODY: usize = 800;

    // Find all heading lines (levels 1–6: # through ######)
    let re = Regex::new(r"^(#{1,6})\s+(.+)$").unwrap();

    let headings: Vec<(usize, String)> = content
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            re.captures(line).map(|cap| {
                let heading_text = cap.get(2).unwrap().as_str().trim().to_string();
                (i, heading_text) // 0-based line index
            })
        })
        .collect();

    // No headings → single symbol with file stem as name
    if headings.is_empty() {
        let stem = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("document")
            .to_string();

        let body: String = content.chars().take(MAX_BODY).collect();

        let total_lines = content.lines().count() as i64;

        return vec![ParsedSymbol {
            name: stem,
            kind: "doc_section".into(),
            parent_context: Some(path.to_string()),
            signature: None,
            doc_comment: None,
            body_text: if body.is_empty() { None } else { Some(body) },
            start_line: 1,
            start_col: 1,
            end_line: if total_lines < 1 { 1 } else { total_lines },
            end_col: 1,
        }];
    }

    let mut symbols: Vec<ParsedSymbol> = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    for (idx, &(heading_line, ref heading_name)) in headings.iter().enumerate() {
        // Determine where this section ends: either the next heading or EOF
        let next_heading_line = headings
            .get(idx + 1)
            .map(|&(l, _)| l)
            .unwrap_or(total_lines);

        // Body is everything between heading line (exclusive) and next heading line (exclusive)
        let body_start = heading_line + 1;
        let body_end = next_heading_line;

        let body_text = if body_start < body_end {
            let raw_body = lines[body_start..body_end].join("\n");
            let trimmed: &str = raw_body.trim();
            if trimmed.is_empty() {
                None
            } else {
                let truncated: String = trimmed
                    .char_indices()
                    .nth(MAX_BODY)
                    .map(|(i, _)| trimmed[..i].to_string())
                    .unwrap_or_else(|| trimmed.to_string());
                Some(truncated)
            }
        } else {
            None
        };

        // end_line is the last line of the section body (1-based)
        let end_line = if body_end > heading_line + 1 {
            body_end as i64 // heading_line is 0-based, body_end is the exclusive end index
        } else {
            (heading_line + 1) as i64
        };

        symbols.push(ParsedSymbol {
            name: heading_name.clone(),
            kind: "doc_section".into(),
            parent_context: Some(path.to_string()),
            signature: None,
            doc_comment: None,
            body_text,
            start_line: (heading_line + 1) as i64, // 1-based
            start_col: 1,
            end_line,
            end_col: 1,
        });
    }

    symbols
}

// ---------------------------------------------------------------------------
// LanguageFn → tree_sitter::Language conversion via `.into()`
// Each grammar exposes a `LANGUAGE` (or similarly named) constant of type
// `tree_sitter_language::LanguageFn` that implements `Into<tree_sitter::Language>`.
// ---------------------------------------------------------------------------

fn get_language(lang: &str) -> Result<tree_sitter::Language, String> {
    match lang {
        // New API — LANGUAGE constant (LanguageFn), convertible via .into()
        "ada" => Ok(tree_sitter_ada::LANGUAGE.into()),
        "agda" => Ok(tree_sitter_agda::LANGUAGE.into()),
        "asm" => Ok(tree_sitter_asm::LANGUAGE.into()),
        "bash" => Ok(tree_sitter_bash::LANGUAGE.into()),
        "bicep" => Ok(tree_sitter_bicep::LANGUAGE.into()),
        "c" => Ok(tree_sitter_c::LANGUAGE.into()),
        "clojure" => Ok(tree_sitter_clojure::LANGUAGE.into()),
        "cmake" => Ok(tree_sitter_cmake::LANGUAGE.into()),
        "commonlisp" => Ok(tree_sitter_commonlisp::LANGUAGE_COMMONLISP.into()),
        "cpp" => Ok(tree_sitter_cpp::LANGUAGE.into()),
        "c-sharp" => Ok(tree_sitter_c_sharp::LANGUAGE.into()),
        "css" => Ok(tree_sitter_css::LANGUAGE.into()),
        "cuda" => Ok(tree_sitter_cuda::LANGUAGE.into()),
        "d" => Ok(tree_sitter_d::LANGUAGE.into()),
        "dafny" => Ok(tree_sitter_dafny::LANGUAGE.into()),
        "dart" => Ok(tree_sitter_dart::LANGUAGE.into()),
        "elm" => Ok(tree_sitter_elm::LANGUAGE.into()),
        "embedded-template" => Ok(tree_sitter_embedded_template::LANGUAGE.into()),
        "elixir" => Ok(tree_sitter_elixir::LANGUAGE.into()),
        "erlang" => Ok(tree_sitter_erlang::LANGUAGE.into()),
        "fsharp" => Ok(tree_sitter_fsharp::LANGUAGE_FSHARP.into()),
        "glsl" => Ok(tree_sitter_glsl::LANGUAGE_GLSL.into()),
        "go" => Ok(tree_sitter_go::LANGUAGE.into()),
        "graphql" => Ok(tree_sitter_graphql::LANGUAGE.into()),
        "haskell" => Ok(tree_sitter_haskell::LANGUAGE.into()),
        "hcl" => Ok(tree_sitter_hcl::LANGUAGE.into()),
        "heex" => Ok(tree_sitter_heex::LANGUAGE.into()),
        "hlsl" => Ok(tree_sitter_hlsl::LANGUAGE_HLSL.into()),
        "html" => Ok(tree_sitter_html::LANGUAGE.into()),
        "ini" => Ok(tree_sitter_ini::LANGUAGE.into()),
        "java" => Ok(tree_sitter_java::LANGUAGE.into()),
        "jsdoc" => Ok(tree_sitter_jsdoc::LANGUAGE.into()),
        "json" => Ok(tree_sitter_json::LANGUAGE.into()),
        "julia" => Ok(tree_sitter_julia::LANGUAGE.into()),
        "kconfig" => Ok(tree_sitter_kconfig::LANGUAGE.into()),
        "llvm" => Ok(tree_sitter_llvm::LANGUAGE.into()),
        "lua" => Ok(tree_sitter_lua::LANGUAGE.into()),
        "make" => Ok(tree_sitter_make::LANGUAGE.into()),
        "matlab" => Ok(tree_sitter_matlab::LANGUAGE.into()),
        "nginx" => Ok(tree_sitter_nginx::LANGUAGE.into()),
        "nickel" => Ok(tree_sitter_nickel::LANGUAGE.into()),
        "nix" => Ok(tree_sitter_nix::LANGUAGE.into()),
        "objc" => Ok(tree_sitter_objc::LANGUAGE.into()),
        "ocaml" => Ok(tree_sitter_ocaml::LANGUAGE_OCAML.into()),
        "ocamllex" => Ok(tree_sitter_ocamllex::LANGUAGE.into()),
        "odin" => Ok(tree_sitter_odin::LANGUAGE.into()),
        "php" => Ok(tree_sitter_php::LANGUAGE_PHP.into()),
        "powershell" => Ok(tree_sitter_powershell::LANGUAGE.into()),
        "prisma-io" => Ok(tree_sitter_prisma_io::LANGUAGE.into()),
        "prolog" => Ok(tree_sitter_prolog::LANGUAGE.into()),
        "properties" => Ok(tree_sitter_properties::LANGUAGE.into()),
        "proto" => Ok(tree_sitter_proto::LANGUAGE.into()),
        "python" => Ok(tree_sitter_python::LANGUAGE.into()),
        "r" => Ok(tree_sitter_r::LANGUAGE.into()),
        "racket" => Ok(tree_sitter_racket::LANGUAGE.into()),
        "regex" => Ok(tree_sitter_regex::LANGUAGE.into()),
        "ruby" => Ok(tree_sitter_ruby::LANGUAGE.into()),
        "rust" => Ok(tree_sitter_rust::LANGUAGE.into()),
        "scala" => Ok(tree_sitter_scala::LANGUAGE.into()),
        "scheme" => Ok(tree_sitter_scheme::LANGUAGE.into()),
        "slint" => Ok(tree_sitter_slint::LANGUAGE.into()),
        "solidity" => Ok(tree_sitter_solidity::LANGUAGE.into()),
        "sparql" => Ok(tree_sitter_sparql::LANGUAGE.into()),
        "systemverilog" => Ok(tree_sitter_systemverilog::LANGUAGE.into()),
        "swift" => Ok(tree_sitter_swift::LANGUAGE.into()),
        "tsquery" => Ok(tree_sitter_tsquery::LANGUAGE.into()),
        "typescript" => Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Ok(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "verilog" => Ok(tree_sitter_verilog::LANGUAGE.into()),
        "vhdl" => Ok(tree_sitter_vhdl::LANGUAGE.into()),
        "xml" => Ok(tree_sitter_xml::LANGUAGE_XML.into()),
        "yaml" => Ok(tree_sitter_yaml::LANGUAGE.into()),
        "fortran" => Ok(tree_sitter_fortran::LANGUAGE.into()),
        "zig" => Ok(tree_sitter_zig::LANGUAGE.into()),
        "zsh" => Ok(tree_sitter_zsh::LANGUAGE.into()),

        // ☯ Grammars que usam `pub fn language() -> tree_sitter::Language` — API antiga
        // que linka contra uma versão diferente do nativo tree-sitter, causando conflito
        // de `links = "tree-sitter"`. Impedidas de compilar junto com 0.25.x.
        // Mantemos a detecção de linguagem, mas retornamos erro no parsing.
        "dot" | "fish" | "gleam" | "kotlin" | "less" | "org" | "scss" => Err(format!(
            "{lang} grammar uses old tree-sitter API incompatible with 0.25"
        )),

        _ => Err(format!("unknown language: {lang}")),
    }
}

// ---------------------------------------------------------------------------
// AST node-kind helpers
// ---------------------------------------------------------------------------

/// Node kinds that represent a named declaration / definition.
/// These are symbols we want to index and display to users.
const DECLARATION_KINDS: &[&str] = &[
    // Rust / general C-like
    "function_item",
    "function_declaration",
    "method_definition",
    "struct_item",
    "struct_declaration",
    "enum_item",
    "enum_declaration",
    "trait_item",
    "impl_item",
    "impl_declaration",
    "type_item",
    "type_alias_declaration",
    "const_item",
    "static_item",
    "macro_definition",
    "macro_invocation",
    "mod_item",
    "use_declaration",
    // TypeScript / JavaScript
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "method_signature",
    "property_signature",
    "call_signature",
    "construct_signature",
    "index_signature",
    "abstract_class_declaration",
    "module",
    // Python
    "class_definition",
    "function_definition",
    // Go
    "type_declaration",
    "type_spec",
    "var_declaration",
    "const_declaration",
    "method_declaration",
    // C / C++
    "class_specifier",
    "struct_specifier",
    "enum_specifier",
    "field_declaration",
    // Java
    "record_declaration",
    "annotation_type_declaration",
    "annotation_declaration",
    // Ruby
    "class",
    "module",
    "method",
    "singleton_method",
    "singleton_class",
    // PHP
    "trait_declaration",
    "namespace_definition",
    "namespace_use_declaration",
    "global_declaration",
    "function_static_declaration",
    // Kotlin
    "object_declaration",
    "companion_object",
    "property_declaration",
    "secondary_constructor",
    // Scala
    "object_definition",
    "trait_definition",
    "val_definition",
    "var_definition",
    "val_declaration",
    "var_declaration",
    "given_definition",
    "export_declaration",
    "extension_definition",
    "enum_definition",
    // C#
    "constructor_declaration",
    "destructor_declaration",
    "property_declaration",
    "event_declaration",
    "event_field_declaration",
    "field_declaration",
    "operator_declaration",
    "conversion_operator_declaration",
    "delegate_declaration",
    "namespace_declaration",
    "local_function_statement",
    "indexer_declaration",
    // CSS (rule_set's "name" is its selector list; see extract_declaration_name)
    "rule_set",
    "media_statement",
    "keyframes_statement",
    // Swift
    "protocol_declaration",
    "extension_declaration",
    "variable_declaration",
    "typealias_declaration",
    "subscript_declaration",
    "operator_declaration",
    // Haskell
    "data_type",
    "newtype",
    "type",
    "type_family",
    "type_instance",
    "type_synomym",
    "foreign_import",
    "default_types",
    "class",
    "function",
    "constructor",
    // Julia
    "abstract_definition",
    "primitive_definition",
    "macro_definition",
    "struct_definition",
    "module_definition",
    // Lua
    "local_function_declaration",
    // Elm
    "value_declaration",
    "type_alias",
    "custom_type",
    "port_annotation",
    // Go comment directives
    "expression_switch_statement",
];

/// Node kinds that represent function/method call expressions.
const CALL_EXPRESSION_KINDS: &[&str] = &["call_expression", "method_invocation"];

/// Node kinds that represent import / use / require statements.
const IMPORT_KINDS: &[&str] = &[
    "use_declaration",
    "import_statement",
    "import_from_statement",
    "import_declaration",
    "require",
    "include",
];

/// Container kinds that can provide parent context for nested symbols.
const CONTAINER_KINDS: &[&str] = &[
    // Rust
    "struct_item",
    "enum_item",
    "trait_item",
    "impl_item",
    "impl_declaration",
    "mod_item",
    // C / C++
    "struct_specifier",
    "class_specifier",
    "enum_specifier",
    // TypeScript / JS
    "class_declaration",
    "interface_declaration",
    "module",
    "enum_declaration",
    // Python
    "class_definition",
    // Go
    // Java
    "class_declaration",
    "interface_declaration",
    "record_declaration",
    "annotation_type_declaration",
    // Ruby
    "class",
    "module",
    "singleton_class",
    // PHP
    "class_declaration",
    "interface_declaration",
    "trait_declaration",
    "namespace_definition",
    // Swift
    "class_declaration",
    "struct_declaration",
    "enum_declaration",
    "protocol_declaration",
    "extension_declaration",
    // Scala
    "class_definition",
    "object_definition",
    "trait_definition",
    "enum_definition",
    // Kotlin
    "class_declaration",
    "object_declaration",
    "interface_declaration",
    // C#
    "class_declaration",
    "struct_declaration",
    "enum_declaration",
    "interface_declaration",
    "record_declaration",
    "namespace_declaration",
    // Haskell
    "class",
    "data_type",
    "newtype",
    "module",
    // Julia
    "module_definition",
    "struct_definition",
    // Common Lisp
    "namespace_definition",
    "protocol_declaration",
];

// ---------------------------------------------------------------------------
// Extracting helpers
// ---------------------------------------------------------------------------

fn extract_doc_comment(content: &str, start_line: i64) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut doc_lines: Vec<&str> = Vec::new();
    if start_line <= 1 {
        return None;
    }
    let mut line = (start_line - 2) as usize;
    loop {
        let trimmed = match lines.get(line) {
            Some(l) => l.trim(),
            None => break,
        };
        if trimmed.starts_with("///") || trimmed.starts_with("//!") {
            doc_lines.push(
                trimmed
                    .trim_start_matches("///")
                    .trim_start_matches("//!")
                    .trim(),
            );
            if line == 0 {
                break;
            }
            line = line.wrapping_sub(1);
        } else if trimmed.starts_with("/**")
            || trimmed.starts_with("/*!")
            || trimmed.starts_with("* ")
        {
            doc_lines.push(
                trimmed
                    .trim_start_matches("/**")
                    .trim_start_matches("/*!")
                    .trim_start_matches('*')
                    .trim(),
            );
            if trimmed.contains("*/") {
                break;
            }
            if line == 0 {
                break;
            }
            line = line.wrapping_sub(1);
        } else if trimmed == "*/" || trimmed == "**/" {
            if line == 0 {
                break;
            }
            line = line.wrapping_sub(1);
        } else if trimmed.starts_with("# ") || trimmed.starts_with("#'") {
            // Python/Ruby style doc comments
            doc_lines.push(
                trimmed
                    .trim_start_matches("# ")
                    .trim_start_matches("#'")
                    .trim(),
            );
            if line == 0 {
                break;
            }
            line = line.wrapping_sub(1);
        } else {
            break;
        }
    }
    if doc_lines.is_empty() {
        return None;
    }
    doc_lines.reverse();
    Some(doc_lines.join(" "))
}

fn extract_body_text(content: &str, start_line: i64, end_line: i64) -> Option<String> {
    if start_line <= 0 || end_line < start_line {
        return None;
    }
    let lines: Vec<&str> = content.lines().collect();
    let start = (start_line - 1) as usize;
    let end = (end_line as usize).min(lines.len());
    if start >= end || start >= lines.len() {
        return None;
    }
    Some(lines[start..end].join("\n"))
}

fn get_node_text<'a>(content: &'a str, node: &tree_sitter::Node, max_len: usize) -> String {
    let text = node.utf8_text(content.as_bytes()).unwrap_or("");
    if text.len() <= max_len {
        text.to_string()
    } else {
        let end = text
            .char_indices()
            .nth(max_len)
            .map(|(i, _)| i)
            .unwrap_or(text.len());
        format!("{}...", &text[..end])
    }
}

fn extract_declaration_name(node: &tree_sitter::Node, kind: &str, content: &str) -> Option<String> {
    // First try the standard "name" field
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = name_node.utf8_text(content.as_bytes()).ok()?;
        let name = name.trim();
        if !name.is_empty() && name != "?" {
            return Some(name.to_string());
        }
    }

    // For some node kinds, the name might be in a different field
    if let Some(alias) = node.child_by_field_name("alias") {
        let name = alias.utf8_text(content.as_bytes()).ok()?;
        return Some(name.to_string());
    }

    // CSS: no name fields in the grammar — the identity of a rule is its
    // selector list, of a keyframes block its name, of a media rule its query.
    if kind == "rule_set" || kind == "keyframes_statement" || kind == "media_statement" {
        let wanted_child = match kind {
            "rule_set" => Some("selectors"),
            "keyframes_statement" => Some("keyframes_name"),
            _ => None,
        };
        if let Some(wanted) = wanted_child {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == wanted {
                    let name = get_node_text(content, &child, 80);
                    let name = name.trim();
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
        // media_statement (or fallback): first line of the node up to the block.
        let text = get_node_text(content, node, 120);
        let head = text.split('{').next().unwrap_or("").trim();
        if !head.is_empty() {
            return Some(head.to_string());
        }
        return None;
    }

    // For struct_specifier / class_specifier (C/C++), name is in the "name" child
    // but might be a type_identifier — extract text directly
    if kind == "struct_specifier" || kind == "class_specifier" || kind == "enum_specifier" {
        // Walk children looking for an identifier node
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let child_kind = child.kind();
            if child_kind == "type_identifier" || child_kind == "identifier" || child_kind == "name"
            {
                if let Ok(text) = child.utf8_text(content.as_bytes()) {
                    if !text.trim().is_empty() {
                        return Some(text.trim().to_string());
                    }
                }
            }
        }
        return None;
    }

    // Last resort: for simple declarations, the first word after keyword might be the name
    // Extracting from raw text as fallback
    if let Ok(text) = node.utf8_text(content.as_bytes()) {
        let trimmed = text.trim();
        // Skip keywords like "fn ", "def ", "func ", "function ", "class ", "struct ", etc.
        let after_keyword = trimmed
            .strip_prefix("fn ")
            .or_else(|| trimmed.strip_prefix("def "))
            .or_else(|| trimmed.strip_prefix("func "))
            .or_else(|| trimmed.strip_prefix("function "))
            .or_else(|| trimmed.strip_prefix("class "))
            .or_else(|| trimmed.strip_prefix("struct "))
            .or_else(|| trimmed.strip_prefix("enum "))
            .or_else(|| trimmed.strip_prefix("trait "))
            .or_else(|| trimmed.strip_prefix("impl "))
            .or_else(|| trimmed.strip_prefix("type "))
            .or_else(|| trimmed.strip_prefix("let "))
            .or_else(|| trimmed.strip_prefix("var "))
            .or_else(|| trimmed.strip_prefix("val "))
            .or_else(|| trimmed.strip_prefix("const "))
            .or_else(|| trimmed.strip_prefix("pub "))
            .or_else(|| trimmed.strip_prefix("private "))
            .or_else(|| trimmed.strip_prefix("internal "))
            .or_else(|| trimmed.strip_prefix("open "))
            .or_else(|| trimmed.strip_prefix("static "))
            .or_else(|| trimmed.strip_prefix("override "));
        if let Some(after) = after_keyword {
            // Take the first word (or until '(' if it's a function)
            let name = after
                .split(&[' ', '(', '<', '{', ':', '\n', '\t'][..])
                .next()
                .unwrap_or("?")
                .trim();
            if !name.is_empty() && !name.starts_with('"') && name != "?" {
                return Some(name.to_string());
            }
        }
    }

    None
}

fn extract_import_name(node: &tree_sitter::Node, kind: &str, content: &str) -> String {
    let text = node.utf8_text(content.as_bytes()).unwrap_or("?");

    match kind {
        "use_declaration" => {
            // Rust `use foo::bar::Baz;` → "Baz" or last segment
            text.rsplit("::")
                .next()
                .unwrap_or(text)
                .trim_end_matches(';')
                .trim()
                .to_string()
                .trim()
                .to_string()
        }
        "import_statement" | "import_from_statement" | "import_declaration" => {
            // `import x from "y"` or `import x` → get the imported name
            let parts: Vec<&str> = text.split_whitespace().collect();
            if parts.len() >= 2 {
                parts[1].to_string()
            } else {
                text.to_string()
            }
        }
        _ => text.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Main parse entry point
// ---------------------------------------------------------------------------

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
        Err(_) => {
            // Fallback: sem tree-sitter, usamos chunkers inteligentes baseados
            // em padrões de cada linguagem (Kotlin, Gleam, Fish, SCSS, Less, DOT, Org).
            let symbols = crate::code_intel::fallback::chunk_fallback(lang, path, content);
            return ParseResult {
                language: lang.into(),
                symbols,
                calls: vec![],
                error: None,
            };
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

        // --- Declarations ---
        if DECLARATION_KINDS.contains(&kind) {
            if let Some(name) = extract_declaration_name(&node, kind, content) {
                let start = node.start_position();
                let end = node.end_position();
                let sig_text = get_node_text(content, &node, 80);
                let sl = start.row as i64 + 1;
                let el = end.row as i64 + 1;

                symbols.push(ParsedSymbol {
                    name,
                    kind: kind.into(),
                    parent_context: collect_parent_context(&node, content),
                    signature: Some(sig_text),
                    doc_comment: extract_doc_comment(content, sl),
                    body_text: extract_body_text(content, sl, el),
                    start_line: sl,
                    start_col: start.column as i64 + 1,
                    end_line: el,
                    end_col: end.column as i64 + 1,
                });
            }
        }

        // --- Function-valued variable declarations (JS/TS/etc.) ---
        // `const OnboardingWizard = (props) => {...}` is a variable_declarator,
        // not a function_declaration — but it IS the component/function. Without
        // this, arrow-function components produce no symbol at all and their
        // bodies are invisible to the embedding index.
        if kind == "variable_declarator" {
            let value_is_function = node
                .child_by_field_name("value")
                .map(|v| {
                    matches!(
                        v.kind(),
                        "arrow_function"
                            | "function_expression"
                            | "function"
                            | "generator_function"
                    )
                })
                .unwrap_or(false);
            if value_is_function {
                if let Some(name_node) = node.child_by_field_name("name") {
                    if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                        let start = node.start_position();
                        let end = node.end_position();
                        let sl = start.row as i64 + 1;
                        let el = end.row as i64 + 1;
                        symbols.push(ParsedSymbol {
                            name: name.trim().to_string(),
                            kind: "function_declaration".into(),
                            parent_context: collect_parent_context(&node, content),
                            signature: Some(get_node_text(content, &node, 80)),
                            doc_comment: extract_doc_comment(content, sl),
                            body_text: extract_body_text(content, sl, el),
                            start_line: sl,
                            start_col: start.column as i64 + 1,
                            end_line: el,
                            end_col: end.column as i64 + 1,
                        });
                    }
                }
            }
        }

        // --- Call expressions ---
        if CALL_EXPRESSION_KINDS.contains(&kind) {
            let func_node = node
                .child_by_field_name("function")
                .or_else(|| node.child(0));

            if let Some(func) = func_node {
                if let Ok(func_text) = func.utf8_text(content.as_bytes()) {
                    let called_name = func_text.trim();
                    if !called_name.starts_with('"')
                        && !called_name.starts_with('\'')
                        && !called_name.starts_with('`')
                    {
                        let start = node.start_position();
                        let containing =
                            find_containing_function_name(&node, content).unwrap_or_default();
                        if !containing.is_empty() && called_name != containing {
                            calls.push(ParsedCall {
                                from_name: containing,
                                to_name: called_name.to_string(),
                                from_line: start.row as i64 + 1,
                            });
                        }
                    }
                }
            }
        }

        // --- Import statements ---
        if IMPORT_KINDS.contains(&kind) {
            let start = node.start_position();
            let end = node.end_position();
            let sig_text = get_node_text(content, &node, 100);
            let name = extract_import_name(&node, kind, content);

            let sl = start.row as i64 + 1;
            let el = end.row as i64 + 1;

            // Avoid duplicate imports that are already handled as declarations
            let is_already_symbol = symbols.iter().any(|s| s.start_line == sl);

            if !is_already_symbol {
                symbols.push(ParsedSymbol {
                    name,
                    kind: "import".into(),
                    parent_context: None,
                    signature: Some(sig_text),
                    doc_comment: extract_doc_comment(content, sl),
                    body_text: extract_body_text(content, sl, el),
                    start_line: sl,
                    start_col: start.column as i64 + 1,
                    end_line: el,
                    end_col: end.column as i64 + 1,
                });
            }
        }

        // --- Walk ---
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

/// Walk up the AST from `node` collecting container kinds+names that enclose it
/// (e.g., a method inside a class inside a module). Stops at the file root.
fn collect_parent_context(node: &tree_sitter::Node, content: &str) -> Option<String> {
    let mut ctx_parts: Vec<String> = Vec::new();
    let mut current = node.parent()?;

    loop {
        let k = current.kind();

        if CONTAINER_KINDS.contains(&k) {
            // Try "name" field first
            let name = current
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                .map(|s| s.to_string())
                .or_else(|| {
                    // For containers like trait/impl blocks, try the type name
                    current
                        .child_by_field_name("trait")
                        .or_else(|| current.child_by_field_name("type"))
                        .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                        .map(|s| s.to_string())
                });

            if let Some(n) = name {
                let short_kind = match k {
                    "class_declaration" | "class_definition" | "class_specifier" | "class" => {
                        "class"
                    }
                    "struct_item" | "struct_declaration" | "struct_specifier"
                    | "struct_definition" => "struct",
                    "enum_item" | "enum_declaration" | "enum_specifier" | "enum_definition" => {
                        "enum"
                    }
                    "trait_item" | "trait_definition" | "trait_declaration" => "trait",
                    "impl_item" | "impl_declaration" => "impl",
                    "interface_declaration" => "interface",
                    "protocol_declaration" => "protocol",
                    "module"
                    | "mod_item"
                    | "module_definition"
                    | "namespace_definition"
                    | "namespace_declaration" => "module",
                    "object_definition" | "object_declaration" => "object",
                    "extension_declaration" => "extension",
                    "record_declaration" => "record",
                    "data_type" | "newtype" => "type",
                    _ => &k,
                };
                ctx_parts.push(format!("{short_kind}:{n}"));
            }
        }

        match current.parent() {
            Some(p) => current = p,
            None => break,
        }
    }

    if ctx_parts.is_empty() {
        None
    } else {
        ctx_parts.reverse(); // outermost first: "class:Database > method:connect"
        Some(ctx_parts.join(" > "))
    }
}

fn find_containing_function_name(node: &tree_sitter::Node, content: &str) -> Option<String> {
    let mut current = node.parent()?;
    loop {
        let k = current.kind();
        // Match any kind that represents a function/method definition
        if DECLARATION_KINDS.contains(&k) {
            if let Some(n) = current.child_by_field_name("name") {
                if let Ok(name) = n.utf8_text(content.as_bytes()) {
                    let name = name.trim();
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
        current = current.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_doc_language_markdown() {
        assert_eq!(detect_doc_language("README.md"), Some("markdown"));
        assert_eq!(detect_doc_language("docs/guide.mdx"), Some("markdown"));
        assert_eq!(detect_doc_language("ARCHITECTURE.md"), Some("markdown"));
    }

    #[test]
    fn detect_doc_language_text() {
        assert_eq!(detect_doc_language("notes.txt"), Some("text"));
        assert_eq!(detect_doc_language("docs/CHANGELOG.txt"), Some("text"));
    }

    #[test]
    fn detect_doc_language_unknown() {
        assert_eq!(detect_doc_language("main.rs"), None);
        assert_eq!(detect_doc_language("index.js"), None);
        assert_eq!(detect_doc_language("Makefile"), None);
    }

    #[test]
    fn parse_doc_file_with_headings() {
        let content = "\
# Title
Some intro text that should not be a symbol.

## Installation
Run `npm install`.

## Usage
Call the function with options.

### Advanced
Deep details here.
";
        let symbols = parse_doc_file("test.md", content);
        assert_eq!(
            symbols.len(),
            4,
            "should create 4 symbols for #, ##, ## and ###"
        );

        // H1 title captures the preamble before the first H2.
        assert_eq!(symbols[0].name, "Title");
        assert_eq!(symbols[0].kind, "doc_section");
        assert!(symbols[0]
            .body_text
            .as_ref()
            .unwrap()
            .contains("intro text"));

        assert_eq!(symbols[1].name, "Installation");
        assert_eq!(symbols[1].kind, "doc_section");
        assert!(symbols[1]
            .body_text
            .as_ref()
            .unwrap()
            .contains("npm install"));
        assert_eq!(symbols[1].start_line, 4);
        assert_eq!(symbols[1].parent_context.as_deref(), Some("test.md"));

        assert_eq!(symbols[2].name, "Usage");
        assert!(symbols[2]
            .body_text
            .as_ref()
            .unwrap()
            .contains("Call the function"));

        assert_eq!(symbols[3].name, "Advanced");
        assert!(symbols[3]
            .body_text
            .as_ref()
            .unwrap()
            .contains("Deep details"));
    }

    #[test]
    fn parse_doc_file_no_headings() {
        let content = "Just a plain text file with no markdown headings.\nSecond line.";
        let symbols = parse_doc_file("README.txt", content);
        assert_eq!(
            symbols.len(),
            1,
            "should create 1 symbol when no headings found"
        );
        assert_eq!(symbols[0].name, "README");
        assert_eq!(symbols[0].kind, "doc_section");
        assert!(symbols[0]
            .body_text
            .as_ref()
            .unwrap()
            .contains("plain text"));
        assert_eq!(symbols[0].start_line, 1);
        assert_eq!(symbols[0].parent_context.as_deref(), Some("README.txt"));
    }

    #[test]
    fn parse_doc_file_truncates_long_body() {
        let long_chunk = "x".repeat(500);
        let medium_chunk = "y".repeat(500);
        let content = format!(
            "\
## Section One
{}

## Section Two
{}",
            long_chunk, medium_chunk
        );
        let symbols = parse_doc_file("doc.md", &content);
        // Each body should be truncated to MAX_BODY (800)
        for sym in &symbols {
            if let Some(ref body) = sym.body_text {
                assert!(body.len() <= 800, "body too long: {}", body.len());
            }
        }
        // But we should still have 2 distinct symbols
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "Section One");
        assert_eq!(symbols[1].name, "Section Two");
    }

    #[test]
    fn parse_doc_file_indexes_h1_title_and_preamble() {
        let content = "\
# Project Title
The preamble that says what the project is.

## Actual section
Content here.
";
        let symbols = parse_doc_file("test.md", content);
        assert_eq!(symbols.len(), 2, "H1 becomes a section too");
        assert_eq!(symbols[0].name, "Project Title");
        assert!(symbols[0].body_text.as_ref().unwrap().contains("preamble"));
        assert_eq!(symbols[1].name, "Actual section");
    }

    #[test]
    fn toml_sections_become_symbols() {
        let content = "\
title = \"top\"

[package]
name = \"claudinio\"
version = \"0.1.0\"

[dependencies.serde]
features = [\"derive\"]

[[bin]]
name = \"main\"
";
        let result = parse_file("Cargo.toml", content);
        assert!(result.error.is_none());
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"package"), "sections found: {names:?}");
        assert!(names.contains(&"dependencies.serde"));
        assert!(names.contains(&"bin"));
        assert!(result.symbols.iter().all(|s| s.kind == "table"));
    }

    #[test]
    fn scss_rules_are_indexed_via_fallback() {
        let content = "\
@mixin theme($color) {
  color: $color;
}
.button-primary {
  @include theme(blue);
  padding: 4px;
}
";
        let result = parse_file("styles/app.scss", content);
        assert!(result.error.is_none());
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&".button-primary"), "rule found: {names:?}");
        assert!(
            result.symbols.len() >= 2,
            "mixin + rule expected: {names:?}"
        );
        // .sass routes to the same scanner instead of being invisible.
        assert_eq!(detect_language("styles/app.sass"), Some("scss"));
    }

    #[test]
    fn parse_doc_file_empty_body_after_heading() {
        let content = "\
## Empty Section

## Next Section
Has content.
";
        let symbols = parse_doc_file("test.md", content);
        assert_eq!(symbols.len(), 2);
        // Empty section should have no body
        assert!(
            symbols[0].body_text.is_none() || symbols[0].body_text.as_ref().unwrap().is_empty()
        );
        // Next section should have content
        assert!(
            symbols[1].body_text.is_some() && !symbols[1].body_text.as_ref().unwrap().is_empty()
        );
    }
}
