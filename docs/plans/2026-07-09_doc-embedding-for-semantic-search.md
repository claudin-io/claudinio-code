# Solution Design: Embedding de Documentação Standalone para Busca Semântica

## 1. Context / Problem Statement

Atualmente o pipeline de embedding do Claudinio Code indexa apenas símbolos de código-fonte (funções, structs, etc.) — arquivos de documentação standalone como `.md`, `.mdx` e `.txt` NÃO são indexados. Quando o AI faz uma `semantic_search`, conceitos documentados fora do código-fonte (arquitetura, decisões de design, README) nunca aparecem nos resultados.

**O que o usuário CONFIRMOU:**
- Indexar `.md`, `.mdx`, `.txt` automaticamente junto com o scan normal do workspace
- Chunking por heading markdown (`##`, `###`) — cada seção com o título como contexto
- Mesmas regras de exclusão do scan de código (gitignore + hidden dirs)

**O que eu INFERI da investigação:**
- Doc comments dentro do código JÁ entram no embedding (via `build_embedding_text`)
- O modelo LateOn-Code-edge funciona com texto em inglês (é um CodeBERT, mas entende linguagem natural)
- A estrutura `symbol_embeddings` com FK para `symbols.id` pode ser reutilizada com símbolos sintéticos

## 2. Goal (Definition of Done)

Arquivos `.md`, `.mdx` e `.txt` no workspace são automaticamente indexados com embeddings semânticos durante o scan, divididos por headings markdown, e aparecem nos resultados do `semantic_search` com snippet e score, sem quebrar o pipeline existente de código.

## 3. Key Findings (Prova Real)

| Finding | Evidence |
|---------|----------|
| `scan_workspace` filtra por `detect_language()` — `.md`/`.txt` retornam `None` | `indexer.rs:143-148` — `parser::detect_language()` match de extensão |
| `build_embedding_text` aceita `doc` e `body` como strings arbitrárias | `embeddings.rs:173-205` — monta `"kind: name \| context: ... \| doc \| body"` |
| `symbol_embeddings.symbol_id` é FK para `symbols.id` | `db.rs:152-155` — `FOREIGN KEY(symbol_id) REFERENCES symbols(id) ON DELETE CASCADE` |
| `search_by_embedding` faz JOIN `symbols → files` para `file_path` | `db.rs:453-468` — `JOIN files f ON f.id = s.file_id` |
| `SemanticSearchResult` tem `snippet: Option<String>` e `score: f32` | `db.rs:49-60` |
| `upsert_embedding` aceita qualquer `symbol_id` + `&[f32]` | `db.rs:424-436` — `INSERT OR REPLACE INTO symbol_embeddings` |
| `insert_symbol` aceita `doc_comment: Option<&str>` | `db.rs:280-301` — campo `doc_comment TEXT` |
| `encode_and_store_batched` faz batch de 16 textos e chama `upsert_embedding` | `indexer.rs:107-121` |
| Watcher usa `is_ignored_path` com gitignore + hidden dirs | `watcher.rs:27-43` |
| Modelo é `lightonai/LateOn-Code-edge` com MAX_LENGTH=512 | `embeddings.rs:5,14` |

## 4. Authoritative Inputs

| Input | Source | Value |
|-------|--------|-------|
| Extensões de doc a indexar | Usuário | `.md`, `.mdx`, `.txt` |
| Estratégia de chunking | Usuário | Por heading markdown (`##`, `###`), título como contexto |
| Momento da indexação | Usuário | Junto com o scan normal do workspace |
| Regras de exclusão | Usuário | Mesmas do scan de código (gitignore + hidden dirs) |
| MAX_BODY_CHARS (limite de corpo no embedding) | Código | 800 caracteres |
| MAX_LENGTH (limite de tokens do modelo) | Código | 512 tokens |
| EMBED_BATCH_SIZE | Código | 16 |

## 5. Changes (Steps)

### 5.1 — `parser.rs`: Adicionar detecção de arquivos de documentação

**Target:** `src-tauri/src/code_intel/parser.rs`
**Mutation:** Adicionar função `detect_doc_language(path: &str) -> Option<&'static str>` que retorna `"markdown"` para `.md`/`.mdx` e `"text"` para `.txt`.
**Why:** Separar a detecção de docs do `detect_language` existente que é só para código. Evita poluir a lógica de linguagens de código.
**Constraints:** Não alterar `detect_language` existente.

### 5.2 — `parser.rs`: Adicionar parser de markdown para chunking por heading

**Target:** `src-tauri/src/code_intel/parser.rs`
**Mutation:** Adicionar função `parse_doc_file(path: &str, content: &str) -> Vec<ParsedSymbol>` que:
1. Detecta headings markdown (`## `, `### `, etc.) com regex simples (sem dependência externa — `regex` já é transitiva via outras deps)
2. Para cada heading, produz um `ParsedSymbol` com:
   - `kind`: `"doc_section"`
   - `name`: texto do heading (ex: `"## Installation"` → name = `"Installation"`)
   - `parent_context`: `Some(file_path)` — o caminho do arquivo como contexto
   - `doc_comment`: `None`
   - `body_text`: texto da seção (truncado a MAX_BODY_CHARS=800)
   - `start_line`/`end_line`: linha do heading e fim da seção
3. Se o arquivo não tem headings, cria UM símbolo com o título do arquivo (nome do arquivo sem extensão) e corpo = primeiros 800 chars
**Why:** Reutilizar `ParsedSymbol` permite que `build_embedding_text` e `encode_and_store_batched` funcionem sem modificação. Heading como `name` + corpo como `body_text` é otimizado para busca semântica.
**Constraints:** Nenhuma dependência externa nova. Regex simples com a crate `regex` (já é transitiva).

### 5.3 — `indexer.rs`: Adicionar `index_doc_file`

**Target:** `src-tauri/src/code_intel/indexer.rs`
**Mutation:** Nova função `index_doc_file(db: &IndexDb, path: &str, content: &str, embedder: Option<&mut CodeEmbedder>) -> Result<(), String>` que:
1. Chama `detect_doc_language` para identificar o tipo
2. Chama `parse_doc_file` para chunkar
3. Insere um `file` row via `db.upsert_file` com language = doc type
4. Insere cada chunk como `symbol` via `db.insert_symbol` (passando `None` para `doc_comment`)
5. Se `embedder.is_some()`, chama `encode_and_store_batched` com os mesmos chunks
**Why:** Espelha `index_file` mas sem dependência de tree-sitter e sem parsing de relações/calls.
**Constraints:** Idempotente — deleta símbolos antigos do arquivo antes de reinserir (igual `index_file`).

### 5.4 — `indexer.rs`: Modificar `scan_workspace` para incluir docs

**Target:** `src-tauri/src/code_intel/indexer.rs`, função `scan_workspace`
**Mutation:** Após o loop atual que processa arquivos de código, adicionar um segundo loop que:
1. Coleta arquivos com extensões `.md`, `.mdx`, `.txt` do mesmo walker
2. Para cada um, chama `index_doc_file`
3. Atualiza os contadores de progresso
**Why:** Manter o scan de código intacto e adicionar docs como fase separada, sem misturar lógicas.
**Constraints:** As mesmas regras de exclusão (gitignore, hidden dirs) já são aplicadas pelo `ignore::WalkBuilder`.

### 5.5 — `indexer.rs`: Modificar `generate_all_embeddings` para cobrir docs

**Target:** `src-tauri/src/code_intel/indexer.rs`, função `generate_all_embeddings`
**Mutation:** O loop atual itera sobre `db.all_files()` e chama `parser::parse_file`. Para arquivos com language = `"markdown"` ou `"text"`, chamar `parse_doc_file` em vez de `parse_file`.
**Why:** `generate_all_embeddings` é chamado quando o modelo de embedding carrega após o scan inicial — docs precisam ser cobertos também.
**Constraints:** Não quebrar o fluxo existente para arquivos de código.

### 5.6 — `watcher.rs`: Garantir que docs também são observados

**Target:** `src-tauri/src/code_intel/watcher.rs`
**Mutation:** Verificar se o file watcher atual já processa `.md`/`.txt` — se sim, apenas garantir que `reindex_file` lida com docs (chamar `index_doc_file` em vez de `index_file` quando `detect_language` retorna `None` mas `detect_doc_language` retorna `Some`).
**Why:** Mudanças em docs precisam atualizar os embeddings em tempo real.
**Constraints:** Alteração mínima — apenas adicionar fallback para docs no `reindex_file`.

### 5.7 — `db.rs`: Sem alterações

**Target:** Nenhum
**Mutation:** Nenhuma.
**Why:** O schema atual já suporta os campos que precisamos. `symbols` já tem `doc_comment TEXT`, `symbol_embeddings` já aceita qualquer `symbol_id`, e `files` já tem `language TEXT`. Nenhuma migration necessária.

### 5.8 — `embeddings.rs`: Sem alterações

**Target:** Nenhum
**Mutation:** Nenhuma.
**Why:** `build_embedding_text` já aceita strings arbitrárias e monta o texto de embedding corretamente para docs. O modelo LateOn-Code-edge funciona com texto natural.

### 5.9 — `tools/mod.rs`: Atualizar descrição do `semantic_search`

**Target:** `src-tauri/src/agent/tools/mod.rs`, linha ~194-203
**Mutation:** Atualizar a descrição da tool `semantic_search` para mencionar que docs também são indexados.
**Why:** O AI precisa saber que docs estão disponíveis para usar a tool corretamente.

### 5.10 — `session.rs` e `subagent.rs`: Atualizar system prompts

**Target:** `src-tauri/src/agent/session.rs` e `src-tauri/src/agent/subagent.rs`
**Mutation:** Atualizar menções à `semantic_search` para incluir que ela cobre documentação também.
**Why:** Coerência — se a tool cobre docs, o prompt do sistema deve refletir isso.

## 6. Verification Plan

### 6.1 — Compilação
```bash
cd src-tauri && cargo check 2>&1
```
**Expected:** Zero erros de compilação. Nenhum warning novo.

### 6.2 — Teste unitário: `parse_doc_file`
Criar teste em `parser.rs` (módulo `tests` existente) que:
- Passa um markdown com 3 headings → verifica que 3 símbolos são gerados
- Passa um markdown sem headings → verifica que 1 símbolo é gerado com nome do arquivo
- Passa uma seção com >800 chars → verifica que `body_text` é truncado

### 6.3 — Teste de integração: scan com docs
Rodar o app contra um workspace de teste com um arquivo `docs/test.md` e verificar via `index_stats` que:
- `files` count inclui o doc
- `symbols` count inclui os chunks
- `symbol_embeddings` tem rows para os chunks

### 6.4 — Teste de busca semântica
Com o workspace de teste indexado, chamar `semantic_search` com uma query que corresponde ao conteúdo do doc e verificar que:
- Resultados incluem `SemanticSearchResult` com `kind: "doc_section"`
- `file_path` aponta para o doc correto
- `snippet` está populado
- `score` > 0

### 6.5 — Regressão: scan de código intacto
Rodar o scan em um workspace só com código (sem docs) e verificar que:
- Número de symbols e embeddings é idêntico ao de antes da feature

### 6.6 — Watcher: mudança em doc dispara reindex
Modificar um `.md` no workspace de teste e verificar que:
- O arquivo é reindexado (hash muda no DB)
- Embeddings são atualizados

### 6.7 — Idempotência
Rodar o scan duas vezes no mesmo workspace e verificar que:
- Número de symbols/docs não duplica

## 7. Risks

| Risk | Mitigation |
|------|------------|
| Modelo CodeBERT pode ter performance pior em texto puro vs código | Testar com queries reais em docs do próprio repo; se qualidade for ruim, considerar modelo diferente no futuro |
| Docs muito grandes (ex: 1000+ headings) podem gerar muitos embeddings | Limitar a 200 chunks por arquivo (flag `MAX_DOC_CHUNKS`). Docs muito grandes provavelmente são gerados e não deveriam estar no workspace |
| Performance do scan pode degradar com muitos docs | Doc parsing é muito mais leve que tree-sitter (só regex); impacto deve ser insignificante |
| Arquivos `.txt` sem estrutura podem gerar embeddings de baixa qualidade | Apenas 1 chunk por arquivo `.txt` sem headings — é melhor que nada |

## 8. Tasks Summary

1. `parser.rs`: Adicionar `detect_doc_language` e `parse_doc_file`
2. `indexer.rs`: Adicionar `index_doc_file`
3. `indexer.rs`: Modificar `scan_workspace` para incluir docs
4. `indexer.rs`: Modificar `generate_all_embeddings` para docs
5. `watcher.rs`: Garantir que docs são reindexados em mudanças
6. `tools/mod.rs`: Atualizar descrição do `semantic_search`
7. `session.rs` / `subagent.rs`: Atualizar system prompts
8. Testes unitários e de integração
