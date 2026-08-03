# HTML File Outline — Suporte Completo a Extração de Símbolos

## Context

O `file_outline` atualmente retorna **zero símbolos** para arquivos `.html`. O parser `tree-sitter-html` (v0.23) faz o parse corretamente, mas `DECLARATION_KINDS` não contém nenhum nó HTML (`element`, `script_element`, `style_element`), e não há sub-parsing do JS/CSS embutido. O arquivo do Bomberman (`/Users/victortavernari/bomberman/index.html`) exemplifica bem: tem `const`, `function`, comentários de seção, `<canvas id="game">` — tudo invisível ao outline.

O usuário quer extração **completa**: elementos HTML com `id`/`class`, comentários HTML como seções, JS de `<script>` e CSS de `<style>`, com parent context visível (`script / generateMap()`, `style / body`).

## Solution Design

### Escopo

- **Parser (`parser.rs`)**: nova função `parse_html_file` que é chamada do `parse_file` quando `lang == "html"`. Esta função faz uma walk no AST do `tree-sitter-html` e extrai:
  1. **Elementos HTML com `id`**: nome = valor do atributo `id`, kind = `"element"`, signature = texto da tag + atributos (truncado a 80 chars). Ex: `<canvas id="game">` → nome `"game"`, kind `"element"`.
  2. **Elementos HTML com `class`**: nome = `tag.classname`, kind = `"element"`. Ex: `<div class="container main">` → nome `"div.container main"`, kind `"element"`.
  3. **Comentários HTML**: nome = texto do comentário sem `<!-- -->` (truncado a 80 chars), kind = `"section"`. Ex: `<!-- 3. JS: Constants -->` → nome `"3. JS: Constants"`, kind `"section"`.
  4. **JS dentro de `<script>`**: re-parse do `raw_text` do `script_element` com `tree-sitter-typescript`. Símbolos extraídos com `parent_context = Some("script".into())`. Inclui `function_declaration`, `variable_declarator` com arrow functions, `class_declaration`, `lexical_declaration` (para `const`/`let`), etc.
  5. **CSS dentro de `<style>`**: re-parse do `raw_text` do `style_element` com `tree-sitter-css`. Símbolos extraídos com `parent_context = Some("style".into())`. Inclui `rule_set`, `media_statement`, `keyframes_statement`.

- **Parent context**: visível no outline. JS symbols ganham `parent_context: "script"`, CSS symbols ganham `parent_context: "style"`. Elementos HTML com `id`/`class` e comentários não ganham parent context (são top-level no HTML). Comentários JS dentro do `<script>` (`// ---`) também são extraídos como seções com parent `"script"`.

- **Re-indexação**: automática. A hash do conteúdo do arquivo já é verificada em cada scan do workspace (`indexer.rs:254`). Quando o código mudar e for recompilado, a próxima scan detectará o conteúdo alterado (hash diferente) e re-indexará. Nenhum comando extra necessário.

### Sizing & Layout

O outline é uma lista de símbolos já existente no UI. Não há mudanças visuais — apenas aumento no número de símbolos retornados para `.html`. O `parent_context` já é renderizado no outline existente (ex: `class:Database > method:connect`).

### Edge Cases

- **`<script>` sem `raw_text` ou vazio**: não gera símbolos JS.
- **`<style>` sem `raw_text` ou vazio**: não gera símbolos CSS.
- **JS/CSS com erro de parse**: `tree-sitter` é tolerante a erros; símbolos parciais são extraídos mesmo com ERROR nodes. O `error` do `ParseResult` permanece `None`.
- **Múltiplos `<script>`/`<style>`**: cada um é processado independentemente.
- **HTML sem `<script>`/`<style>`**: apenas elementos e comentários são extraídos.
- **Elemento com `id` e `class`**: gera um símbolo para `id` e outro para `class` (se ambos presentes).
- **Atributo `id` vazio ou whitespace**: ignorado.
- **Comentário HTML vazio ou só whitespace**: ignorado.
- **Elemento sem `id` nem `class`**: ignorado (não polui o outline).
- **Line offset para JS/CSS**: as linhas dos símbolos extraídos do `<script>`/`<style>` são relativas ao snippet; o offset (linha inicial do `raw_text` no HTML) é somado para produzir números de linha absolutos corretos.

### User-Provided Assets

Nenhum asset visual. O arquivo de referência é `/Users/victortavernari/bomberman/index.html`.

## Risks

- **Performance**: re-parse de `<script>` e `<style>` com grammars JS/CSS adiciona overhead. Mas arquivos HTML típicos têm poucos blocos e são parseados em milissegundos. Risco baixo.
- **Regressão em outros formatos**: `parse_html_file` só é chamado quando `lang == "html"`. Zero risco para `.ts`, `.rs`, `.py`, etc.
- **Tree-sitter HTML grammar**: `tree-sitter-html` v0.23 é estável. A estrutura de nós não deve mudar em patch versions.

## Non-goals

- Tags sem `id`/`class` não são extraídas.
- `<script src="...">` externo não é seguido.
- `<link rel="stylesheet">` externo não é seguido.
- HTML-in-JSX (`.tsx`/`.jsx`) não é afetado — já funciona via `LANGUAGE_TSX`.
- SVG inline dentro de HTML não extrai símbolos próprios.

---

## Low-Level Design

### Arquivos a modificar

**Apenas 1 arquivo**: `src-tauri/src/code_intel/parser.rs`

### Mudança 1: Nova função `parse_html_file`

**Local**: após `parse_file` (~linha 1099), antes de `collect_parent_context`.

**Assinatura**:
```rust
fn parse_html_file(content: &str, root: &tree_sitter::Node) -> (Vec<ParsedSymbol>, Vec<ParsedCall>)
```

**Algoritmo**:

```
parse_html_file(content, root):
    symbols = []
    calls = []
    walk root AST:
        if node.kind == "element":
            extract_html_element(node, content, &mut symbols)
        if node.kind == "comment":
            extract_html_comment(node, content, &mut symbols)
        if node.kind == "script_element":
            extract_embedded_script(node, content, &mut symbols, &mut calls)
        if node.kind == "style_element":
            extract_embedded_style(node, content, &mut symbols)
    return (symbols, calls)
```

### Mudança 2: Helper `extract_html_element`

Extrai `id` e `class` de um nó `element`.

```
extract_html_element(node, content, symbols):
    // Encontrar o start_tag ou self_closing_tag filho
    start_tag = find_child(node, ["start_tag", "self_closing_tag"]) or return
    tag_name = find_child(start_tag, "tag_name") or return
    tag_text = tag_name.utf8_text(content)

    // Coletar atributos
    for each child of start_tag where kind == "attribute":
        attr_name_node = find_child(child, "attribute_name")
        attr_value_node = find_child(child, "attribute_value")
        if neither exists: continue
        attr_name = attr_name_node.utf8_text(content)
        attr_value = attr_value_node.utf8_text(content) sem aspas (strip '"' and "'")

        if attr_name == "id" and attr_value not empty:
            name = attr_value
            kind = "element"
            signature = tag_text + atributos (start_tag text, trunc 80)
            push symbol

        if attr_name == "class" and attr_value not empty:
            name = tag_text + "." + attr_value
            kind = "element"
            signature = tag_text + atributos (start_tag text, trunc 80)
            push symbol
```

**Tree-sitter node kinds usados**:
- `element` — nó pai, contém `start_tag` ou `self_closing_tag`
- `start_tag` — contém `tag_name` e `attribute` nodes
- `self_closing_tag` — igual acima, para `<br/>`, `<img/>`, etc.
- `tag_name` — nome da tag (`canvas`, `div`, etc.)
- `attribute` — contém `attribute_name` e opcionalmente `attribute_value`
- `attribute_name` — `id`, `class`, `width`, etc.
- `attribute_value` — valor entre aspas (inclui as aspas)

### Mudança 3: Helper `extract_html_comment`

```
extract_html_comment(node, content, symbols):
    raw = node.utf8_text(content)  // "<!-- 3. JS: Constants -->"
    text = strip "<!--" prefix and "-->" suffix, then trim
    if text is empty: return
    name = text truncado a 80 chars
    kind = "section"
    push symbol (sem parent_context)
```

### Mudança 4: Helper `extract_embedded_script`

Re-parse do conteúdo `<script>` com `tree-sitter-typescript`.

```
extract_embedded_script(node, content, symbols, calls):
    raw_text_node = find_child(node, "raw_text") or return
    js_code = raw_text_node.utf8_text(content)
    if js_code.trim() is empty: return

    // Offset: linha inicial do raw_text dentro do HTML
    line_offset = raw_text_node.start_position().row as i64

    // Re-parse com tree-sitter-typescript
    parser = new Parser()
    parser.set_language(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
    tree = parser.parse(js_code, None) or return

    // Walk similar ao parse_file genérico, mas:
    // - Adiciona line_offset a todas as linhas
    // - parent_context = Some("script")
    // - Só extrai DECLARATION_KINDS, variable_declarator com arrow functions,
    //   CALL_EXPRESSION_KINDS, IMPORT_KINDS
    //   (usando as mesmas constantes globais)

    walk tree.root_node():
        same logic as parse_file lines 958-1092
        BUT: start_line = sl + line_offset
             end_line = el + line_offset
             parent_context = Some("script".into())

    // Also extract JS line comments as sections
    // "// ---------- Constants ----------" → section with parent "script"
    extract_js_comments(node, js_code, line_offset, symbols)
```

**Detalhe importante sobre `lexical_declaration`**: no tree-sitter-typescript, `const COLS = 13` é um nó `lexical_declaration` que contém `variable_declarator` filhos. O `lexical_declaration` NÃO está em `DECLARATION_KINDS` atualmente, mas `variable_declarator` é tratado como caso especial (linha 986 do parser.rs). Para JS inline, vamos:
1. Adicionar `"lexical_declaration"` a `DECLARATION_KINDS` — isso fará `const COLS = 13` aparecer como símbolo (com nome do primeiro declarator)
2. OU manter o comportamento existente: `const COLS` só vira símbolo se for arrow function (via `variable_declarator`). Isso seria insuficiente.
3. **Decisão**: Vamos adicionar `"lexical_declaration"` a `DECLARATION_KINDS` E ajustar `extract_declaration_name` para extrair o nome do primeiro `variable_declarator` filho. Isso beneficia não só HTML mas também JS/TS standalone.

### Mudança 5: Helper `extract_embedded_style`

Similar ao script, mas com `tree-sitter-css`.

```
extract_embedded_style(node, content, symbols):
    raw_text_node = find_child(node, "raw_text") or return
    css_code = raw_text_node.utf8_text(content)
    if css_code.trim() is empty: return

    line_offset = raw_text_node.start_position().row as i64

    parser = new Parser()
    parser.set_language(tree_sitter_css::LANGUAGE.into())
    tree = parser.parse(css_code, None) or return

    walk tree.root_node():
        only DECLARATION_KINDS for CSS: "rule_set", "media_statement", "keyframes_statement"
        start_line += line_offset
        parent_context = Some("style".into())
```

### Mudança 6: Helper `extract_js_comments`

Extrai comentários `// ---` e `/* --- */` do JS como símbolos `section`.

```
extract_js_comments(js_root_node, js_code, line_offset, symbols):
    walk js_root_node:
        if node.kind == "comment":
            text = node.utf8_text(js_code)
            // Strip // or /* */ delimiters
            clean = strip_comment_delimiters(text).trim()
            if clean empty or only dashes/equals: skip (decoration only)
            name = clean trunc 80
            push symbol with kind="section", parent_context=Some("script"),
                        start_line = node.start_position().row + line_offset + 1
```

### Mudança 7: Hook no `parse_file`

Após a linha 941 (após `let root = tree.root_node();`), ANTES do loop de walk genérico:

```rust
// --- HTML: custom extraction with embedded JS/CSS support ---
if lang == "html" {
    let (html_symbols, html_calls) = parse_html_file(content, &root);
    return ParseResult {
        language: lang.into(),
        symbols: html_symbols,
        calls: html_calls,
        error: None,
    };
}
```

Isso faz com que arquivos HTML pulem completamente o walk genérico (que de qualquer forma não produziria nada) e usem o novo extrator especializado.

### Mudança 8: `lexical_declaration` em `DECLARATION_KINDS`

Adicionar `"lexical_declaration"` ao array `DECLARATION_KINDS` (~linha 445). E em `extract_declaration_name`, adicionar um branch para `lexical_declaration`:

```rust
// JS/TS: lexical_declaration (const x = ..., let y = ...)
if kind == "lexical_declaration" {
    // Name = first variable_declarator's name
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    return Some(name.trim().to_string());
                }
            }
        }
    }
    return None;
}
```

### Mudança 9: `variable_declarator` com parent_context

No walk genérico do `parse_file` (linha 986), os símbolos de `variable_declarator` já ganham `parent_context` via `collect_parent_context`. Para o walk do JS embedded, vamos passar `parent_context` fixo como `Some("script".into())`.

### Fluxo de dados completo

```
indexer.rs: index_file()
  → parser::parse_file(path, content)
    → detect_language("index.html") → "html"
    → get_language("html") → tree_sitter_html::LANGUAGE
    → tree_sitter::Parser::parse(content) → AST HTML
    → if lang == "html": parse_html_file(content, &root)
      → walk AST HTML:
        → element → extract_html_element → symbols com kind="element"
        → comment → extract_html_comment → symbols com kind="section"
        → script_element → extract_embedded_script
          → re-parse raw_text com tree-sitter-typescript
          → walk AST JS → symbols com parent_context="script"
          → extract_js_comments → symbols com kind="section", parent="script"
        → style_element → extract_embedded_style
          → re-parse raw_text com tree-sitter-css
          → walk AST CSS → symbols com parent_context="style"
      → return (symbols, calls)
  → db.insert_symbol() para cada símbolo
  → db.insert_relation() para cada call
  → collect_embedding_chunks + store_chunks + encode_and_store_batched
```

### Verificação

Após compilar e rodar, testar com o arquivo `/Users/victortavernari/bomberman/index.html`:

1. `file_outline` deve retornar dezenas de símbolos (antes: 0)
2. Deve conter: `game` (kind=element), `3. JS: Constants` (kind=section), `generateMap` (kind=function_declaration, parent="script"), `body` (kind=rule_set, parent="style"), etc.
3. `semantic_search` com query "bomberman game loop" deve achar `gameLoop` function
4. Nenhum erro de parse

---

## Tasks Summary

1. Add `"lexical_declaration"` to `DECLARATION_KINDS` and add name extraction in `extract_declaration_name`
2. Create `parse_html_file` function with the HTML AST walk dispatcher
3. Create `extract_html_element` helper for id/class attributes
4. Create `extract_html_comment` helper for HTML comments as sections
5. Create `extract_embedded_script` helper with JS re-parsing and line offset
6. Create `extract_embedded_style` helper with CSS re-parsing and line offset
7. Create `extract_js_comments` helper for JS comments as sections
8. Hook `parse_html_file` into `parse_file` when `lang == "html"`
9. Build, test with Bomberman HTML, verify symbols appear in outline


## Implementation Log — 2026-08-03 01:51
**Summary:** Full HTML file-outline support: parse_html_file dispatcher + element/comment/script/style extractors, embedded JS/CSS re-parsing with parent context and absolute line offsets, lexical_declaration support, and verified end-to-end (280 symbols from Bomberman HTML, was 0).
**Changed files:** A	docs/plans/2026-08-03_html-file-outline.md
**Commits:** f62fde1 docs(plan): html-file-outline
**Journal:** ## Findings & decisions

**Quoted HTML attributes were a real silent bug.** The original `extract_html_element` matched only direct `attribute_value` children of `attribute`. The tree-sitter-html grammar (grammar.json:350-369) parses `id="game"` as a `quoted_attribute_value` node (CHOICE of `'attr'`), with the inner content ALIASED as `attribute_value`. So every quoted id/class — the universal HTML form — was skipped, yielding zero element symbols. The focused unit test caught it; the one-line fix (`"attribute_value" | "quoted_attribute_value"` + the existing quote-stripping `trim_matches`) fixed all HTML files at once. Lesson: when relying on grammar node kinds, verify against the installed grammar source, not assumptions.

**tree-sitter 0.25.10 API gotcha.** `Parser::set_language` takes `&tree_sitter::Language`, but `From<LanguageFn>` is only implemented for the owned `Language`. `set_language(x.into())` fails to infer the reference target. Fix: bind `let l: tree_sitter::Language = x.into();` then pass `&l`. The pre-existing `parse_file` site already used this pattern, which made the error easy to spot.

**Line-offset arithmetic for embedded JS/CSS.** raw_text start row (0-based) + JS/CSS node row + 1 = absolute line. Verified empirically: `generateMap` (JS row 2, script raw_text row 15) → line 18. `extract_doc_comment(content, sl)` / `extract_body_text(content, sl, el)` correctly read the HTML file lines at absolute positions, so no separate JS-text handling was needed there.

**Parent context & reuse.** Embedded JS/CSS reuse the generic walk logic and helpers (`extract_declaration_name`, `get_node_text`, `find_containing_function_name`, etc.) with a fixed `Some("script")`/`Some("style")` parent — zero duplication of the walk. `lexical_declaration` name extraction (const/let) benefits standalone JS/TS too.

**Verification depth.** 280 symbols from the real 735-line Bomberman file (was 0), error=None, all 9 spec symbols verified with exact kind/parent/line (game=element:23, HTML comment=section:24, generateMap/COLS/BOMB_FUSE/dropBomb/handleInput/render with parent script at lines 76/29/34/209/665/441, body=rule_set parent style:9). Full lib suite: 307 passed, 0 failed. Diff is 577 pure insertions, 0 deletions.

## Gotchas
- The real Bomberman HTML comment is `<!-- 3. JS: Constants -> Map Generator -> State -> Game Loop -> Renderer -> Input -->` (76 chars, under the 80-char truncation) — the full string, not the abbreviated `3. JS: Constants` from the plan example. The parser was right; the expectation was wrong.
- `gen` is a reserved keyword in Rust 2024 edition — the test binding had to be renamed `gen_fn`.
- The e2e harness must call the lib target name `claudinio_code_lib::code_intel::parser::parse_file` (the lib target is renamed in Cargo.toml).

**Task journal:**
- Add lexical_declaration to DECLARATION_KINDS + name extraction: Added "lexical_declaration" to DECLARATION_KINDS at line 449 (after function_declaration at 448).; Added lexical_declaration branch in extract_declaration_name at lines 766-784, extracting name from first variable_declarator's name field.; cargo check --lib passes cleanly (no errors, no warnings).
- Create parse_html_file dispatcher function: parse_html_file inserted at lines 1118-1165 (doc comment 1118-1120, fn 1121-1165), immediately before the collect_parent_context doc comment.; Dispatcher uses match on kind: element/comment/script_element/style_element; skip_descend prevents descending into script/style raw text.; Verified by read-back; helpers don't exist yet (tasks 3-7), so compile deferred to task 9.
- Create extract_html_element helper (id + class): extract_html_element inserted at lines 1167-1251 (doc 1167-1169, fn 1170-1251), before collect_parent_context doc comment.; Finds start_tag/self_closing_tag child, tag_name child, iterates attribute children; id → name=value, class → name='tag.class...'; strips quotes via trim_matches on attribute_value; signature = start_tag text via get_node_text(...,80); parent_context=None.; LATER FIX (task 9): match arm now also handles "quoted_attribute_value" — tree-sitter-html grammar (grammar.json:350-369) parses quoted attribute values as quoted_attribute_value nodes containing the inner alias attribute_value. Without this, id="game" (quoted) produced no element symbol. Verified against installed grammar source.; Verified by read-back.
- Create extract_html_comment helper (HTML comments as sections): extract_html_comment inserted at lines 1253-1281 (doc 1253-1255, fn 1256-1281), between extract_html_element and the collect_parent_context doc comment.; Strips '<!--' prefix and '-->' suffix (with defensive fallback if '-->' missing), trims, skips empty; name = 80-char truncation via chars().take(80); kind='section'; parent_context=None.; Verified by read-back: placement at 1253-1281 exactly between extract_html_element (ends 1251) and collect_parent_context doc (starts 1283).
- Create extract_embedded_script helper (JS re-parsing): extract_embedded_script inserted at lines 1283-1452 (doc 1283-1285, fn 1286-1452), exactly between extract_html_comment (ends 1281) and collect_parent_context doc comment (now 1454).; Byte-for-byte diff vs task spec: IDENTICAL (diff exit 0, 170/170 lines).; git diff --numstat shows 353 insertions, 0 deletions — no existing code modified.; Helpers reused, not redefined: extract_declaration_name/get_node_text/extract_doc_comment/extract_body_text/find_containing_function_name/extract_import_name all single-definition; constants DECLARATION_KINDS/CALL_EXPRESSION_KINDS/IMPORT_KINDS single-definition.; Compile fix (task 9): set_language now binds local `let js_ts_language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();` then passes &js_ts_language — tree-sitter 0.25.10 set_language takes &Language, From<LanguageFn> only for owned Language.
- Create extract_embedded_style helper (CSS re-parsing): extract_embedded_style inserted at lines 1454-1526 (doc 1454-1456, fn 1457-1526), exactly between extract_embedded_script (ends 1452) and collect_parent_context doc comment (now 1528).; Byte-for-byte diff vs task spec: IDENTICAL (diff exit 0, 73/73 lines).; git diff --numstat shows 427 insertions, 0 deletions — no existing code modified.; Helpers reused: extract_declaration_name/get_node_text/extract_body_text + DECLARATION_KINDS (already contains rule_set/media_statement/keyframes_statement).; Compile fix (task 9): set_language now binds local `let css_ts_language: tree_sitter::Language = tree_sitter_css::LANGUAGE.into();` then passes &css_ts_language.
- Create extract_js_comments helper (JS comments as sections): extract_js_comments inserted at lines 1528-1589 (doc 1528-1530, fn 1529-1589), exactly between extract_embedded_style (ends 1525) and collect_parent_context doc comment (now 1591).; Verbatim diff vs spec: IDENTICAL (diff exit 0, 62/62 lines).; Signature (js_root: &Node, js_code: &str, line_offset: i64, symbols: &mut Vec<ParsedSymbol>) matches the call site at 1451 exactly.; grep -c '^fn extract_js_comments' = 1 — single definition; ParsedSymbol reused.; Cargo check: missing-function error GONE.
- Hook parse_html_file into parse_file for HTML: HTML branch inserted at lines 961-971 (comment 961, if 962, block 963-970, blank 971), immediately after let root = tree.root_node() (line 960) and before let mut symbols (now 972).; git diff --numstat: 501 insertions, 0 deletions; parse_file hunk contains only the 11 added lines.; grep -c '^fn parse_html_file' = 1 — reused, not redefined.; Verified by read-back.
- Build, test, and verify with Bomberman HTML: cargo check (lib + bins) passes clean: exit 0, no errors/warnings.; Fixed 2 compile errors: set_language(ts_lang.into()) → bind local Language then pass &lang (tree-sitter 0.25.10 API) at extract_embedded_script ~1313 and extract_embedded_style ~1479.; Added focused unit test parse_html_embedded_js_css_symbols (parser.rs ~1905) covering element id/class, HTML comment section, JS const+function with parent 'script' + absolute line, CSS rule_set with parent 'style', JS comment section. Full lib suite: 307 passed, 0 failed, 3 ignored.; PRODUCTION FIX discovered by test: extract_html_element now also matches "quoted_attribute_value" node kind (grammar.json proof: quoted attributes parse as quoted_attribute_value containing inner alias attribute_value). Without it, quoted id/class attributes were skipped entirely — real bug affecting all HTML files.; E2E vs REAL Bomberman file (/Users/victortavernari/bomberman/index.html, 735 lines): 280 symbols (was 0), error=None. All 9 expected symbols verified: game=element:23, '3. JS: Constants -> Map Generator -> State -> Game Loop -> Renderer -> Input'=section:24, generateMap=function_declaration script:76, body=rule_set style:9, COLS=lexical_declaration script:29, BOMB_FUSE=lexical_declaration script:34, dropBomb=function_declaration script:209, handleInput=function_declaration script:665, render=function_declaration script:441.; Temporary e2e test removed; git status clean of scratch (only parser.rs modified + pre-existing untracked examples/).; Full diff is 577 pure insertions, 0 deletions.
