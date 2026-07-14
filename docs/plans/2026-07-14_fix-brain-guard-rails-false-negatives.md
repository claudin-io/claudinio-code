# Fix: guard rails do Brain mode rejeitam conteúdo válido

## Context

A session `533b5079` mostra o agente "sofrendo" no final por causa de dois guard rails que rejeitam entradas legítimas:

1. **LLD gate falso-negativo** — o agente escreveu um plano com `## Low-Level Design` cujo corpo começa direto com sub-headings (`### Files to Change`, `### File: ...`). O validador `has_nonempty_section` (`src-tauri/src/agent/tools/write_plan.rs:24`) trata **qualquer** linha começando com `#` como fim da seção — inclusive sub-headings `###` que fazem parte dela. Resultado: `tasks_set` rejeitou 3+ vezes com "no non-empty '## Low-Level Design' section", e o agente gastou ~15 rounds (semantic_search, grep, leitura do próprio código do validador!) até descobrir que precisava de uma linha de texto corrido antes do primeiro `###`.

2. **validate_path rejeita paths relativos** — `validate_path` (`src-tauri/src/agent/tools/mod.rs:72`) nunca resolve o path relativo contra `workspace_root`. `canonicalize()` de um path relativo resolve contra o CWD do processo (que numa app Tauri GUI não é o workspace), falha, e o fallback léxico `req.starts_with(root)` nunca casa relativo-vs-absoluto. Resultado na session: `Error: path 'src-tauri/src/agent/session.rs' is outside the workspace '/Users/victortavernari/claudinio_code'` — um path que está literalmente dentro do workspace.

(Terceiro evento na session — strip do prefixo `golden-` em `tasks_set` — é comportamento correto/anti-forgery, sem mudança.)

## Solution Design

- `has_nonempty_section` deve encerrar a seção apenas em headings de nível **igual ou superior** ao heading alvo (`##` alvo → encerra em `#` ou `##`; `###`+ conta como conteúdo da seção, e por si só já a torna não-vazia).
- `validate_path` deve resolver paths relativos contra `workspace_root` antes de canonicalizar/comparar. Path relativo sem `..` que resolve para dentro do root passa; traversal continua bloqueado pelo canonicalize/checagem existente.

Abordagem: TDD — escrever os testes primeiro, ver eles falharem (vermelho), depois implementar a correção e ver passarem (verde).

## Risks

- Afrouxar `has_nonempty_section` também afeta o soft-warning de `write_plan` para `## Context`/`## Solution Design`/`## Risks` — desejável (mesma semântica).
- `validate_path` com path relativo + `..` : coberto porque o join é canonicalizado e o `starts_with(canon_root)` continua sendo a checagem final; adicionar o teste de traversal garante isso.

## Non-goals

- Não alterar `validate_read_path` (já funciona, herda o fix de `validate_path` automaticamente).
- Não alterar o comportamento do `latest_plan_path` ou `plans_dir`.
- Não alterar o gate `check_brain_lld_gate` em `tasks.rs` (só a função que ele chama muda).

## Low-Level Design

Duas correções independentes em dois arquivos, ambas seguindo TDD (testes primeiro, implementação depois).

### Fix 1: `has_nonempty_section` — heading level awareness

**File:** `src-tauri/src/agent/tools/write_plan.rs`, lines 24–43 (current function body)

**Current behavior:** Any line starting with `#` while `in_section` returns `false` (section considered empty).

**New behavior:** Track heading level. When `in_section`, a line starting with `#` ends the section ONLY if its level is `<= target_level`. Sub-headings (`###` when target is `##`) count as body content and make the section non-empty.

**Algorithm:**
1. Compute `target_level = heading.chars().take_while(|c| *c == '#').count()` — number of `#` at start of heading (2 for `## Low-Level Design`).
2. In the loop, when `trimmed.starts_with('#')` and `in_section`:
   - `line_level = trimmed.chars().take_while(|c| *c == '#').count()`
   - If `line_level <= target_level` → `return false` (end of section at same or higher level, and no body content was found)
   - Otherwise (sub-heading `###`, `####`, etc.) → `return true` (the sub-heading IS body content, section is non-empty)
3. Rest of the logic unchanged: heading detection, non-empty body line detection, EOF fallback.

**New tests to add BEFORE the fix (TDD):**

In `mod tests` block (~line 106+):
- `nonempty_section_subheading_counts_as_body`: `"## Low-Level Design\n### Files to Change\n- src/foo.rs\n"` → `true`
- `nonempty_section_ends_at_same_level`: `"## Low-Level Design\n## Risks\nnone"` → `false`
- `nonempty_section_ends_at_higher_level`: `"## Low-Level Design\n# Other\nbody"` → `false`
- `nonempty_section_subheading_only_no_text`: `"## Low-Level Design\n### Files\n"` → `true` (the sub-heading itself is non-empty content)

### Fix 2: `validate_path` — resolve relative paths against workspace_root

**File:** `src-tauri/src/agent/tools/mod.rs`, lines 72–88 (current function body)

**Current behavior:** `Path::new(requested)` is used directly. For relative paths, `canonicalize()` resolves against CWD (not workspace), which fails. The lexical fallback `req_clean.starts_with(root_clean)` fails because relative-vs-absolute prefixes never match.

**New behavior:** Before the canonicalize block, if `requested` is relative, join it with `root`:
```rust
let effective = if req_clean.is_relative() {
    root_clean.join(req_clean)
} else {
    req_clean.to_path_buf()
};
```
Then use `effective` instead of `req_clean` in the canonicalize and fallback blocks.

**Edge case:** Path with `..` traversal — e.g., `"src/../../etc/passwd"`. After join with root, this becomes an absolute path that canonicalizes outside the root. The existing `canon_req.starts_with(&canon_root)` check catches and rejects it.

**New tests to add BEFORE the fix (TDD):**

In `mod tests` block (~line 1107+):
- `test_validate_path_allows_relative_within_workspace`: create a temp dir as workspace root, create a file inside it, then call `validate_path("relative/path/to/file", &ctx)` and assert `is_ok()`
- `test_validate_path_rejects_relative_with_traversal`: `validate_path("../outside.txt", &ctx)` with workspace_root set → `is_err()`
- Existing `test_validate_path_allows_absolute_within_workspace` must continue to pass.

### Files changed (summary)

| File | Change |
|------|--------|
| `src-tauri/src/agent/tools/write_plan.rs` | Rewrite `has_nonempty_section` with level-aware logic; add 4 new tests |
| `src-tauri/src/agent/tools/mod.rs` | Add relative-path resolution in `validate_path`; add 2 new tests |

No changes to `tasks.rs`, `write_plan.rs:execute()`, `validate_read_path`, or any other file.

### Verification

1. `cargo test -p claudinio-code` — all existing + new tests pass
2. TDD order: write tests → `cargo test` (expect FAIL for new tests, PASS for existing) → implement fixes → `cargo test` (ALL pass)
3. Manual scenario: simulate the session `533b5079` content — a plan with `## Low-Level Design\n### Files to Change\n- src/foo.rs\n` — and confirm `has_nonempty_section(content, LLD_HEADING)` returns `true`
4. Manual scenario: confirm `validate_path("src-tauri/src/agent/session.rs", &ctx)` with `workspace_root = "/Users/victortavernari/claudinio_code"` returns `Ok(())`

## Tasks summary

1. **TDD: escrever testes do `has_nonempty_section`** — adicionar 4 novos testes em `write_plan.rs`, rodar `cargo test`, confirmar que falham (vermelho)
2. **Implementar correção do `has_nonempty_section`** — reescrever a função com level-awareness, rodar testes, confirmar que passam (verde)
3. **TDD: escrever testes do `validate_path`** — adicionar 2 novos testes em `mod.rs`, rodar `cargo test`, confirmar que falham (vermelho)
4. **Implementar correção do `validate_path`** — adicionar resolução de path relativo, rodar testes, confirmar que passam (verde)
5. **Verificação final** — `cargo test` completo, verificar que todos os testes (novos + existentes) passam, rodar cenários manuais descritos na verificação


## Implementation Log — 2026-07-14 12:52
**Summary:** Fix guard rails: has_nonempty_section com level-awareness + validate_path resolve paths relativos
**Changed files:** M src-tauri/src/agent/tools/mod.rs, M src-tauri/src/agent/tools/write_plan.rs, ?? docs/plans/2026-07-14_fix-brain-guard-rails-false-negatives.md
**Commits:** _(git unavailable or none)_
**Journal:** TDD foi seguido rigorosamente: escrevemos os testes primeiro (vermelho), implementamos a correção (verde).

**Bug 1 — has_nonempty_section (write_plan.rs):**
- Causa raiz: a função tratava qualquer linha começando com `#` como fim da seção, inclusive sub-headings `###` que são conteúdo legítimo do LLD.
- Fix: calcular `target_level` a partir do heading alvo (2 para `##`), e no loop comparar `line_level`. Sub-headings com `line_level > target_level` retornam `true` (conteúdo). Só encerra a seção quando `line_level <= target_level`.
- 4 novos testes: subheading_counts_as_body, ends_at_same_level, ends_at_higher_level, subheading_only_no_text.

**Bug 2 — validate_path (mod.rs):**
- Causa raiz: paths relativos eram usados diretamente; `canonicalize()` resolvia contra o CWD do processo (não o workspace), e o fallback lexical `req.starts_with(root)` nunca casava relativo-vs-absoluto.
- Fix: se o path é relativo, fazer `root_clean.join(req_clean)` para obter `req_effective`. Usar `req_effective` em vez de `req_clean` no canonicalize e no fallback.
- Fallback lexical também passou a rejeitar paths com componentes `ParentDir` (`..`) — segurança extra contra traversal quando canonicalize falha (arquivo não existe).
- 2 novos testes: allows_relative_within_workspace, rejects_relative_with_traversal.
- Nota: traversal que existe fisicamente é pego pelo canonicalize; traversal de path que não existe (ex: `../outside.txt` numa pasta vazia) é pego pelo `ParentDir` check no fallback léxico.

**Verificação:** 225 testes passam, 0 falham, 3 ignorados (live API).

**Task journal:**
- TDD: write has_nonempty_section tests (red): Added 4 tests: subheading_counts_as_body (FAIL), ends_at_same_level (PASS), ends_at_higher_level (PASS), subheading_only_no_text (FAIL). 7 existing tests passed, 2 new red.
- Implement has_nonempty_section fix (green): Implemented level-aware logic: computes target_level from heading (2 for ##), compares line_level when in_section; only returns false if line_level <= target_level; sub-headings (###+) return true. All 9 tests pass (7 existing + 2 new).
- TDD: write validate_path tests (red): Added 2 tests: allows_relative_within_workspace (FAILED - red!), rejects_relative_with_traversal (PASS). 3 existing tests passed, 1 new red.
- Implement validate_path fix (green): Added relative-path resolution: req_effective joins with root_clean if relative. Fallback lexical check also rejects paths with ParentDir components. All 5 validate_path tests pass.
- Final verification: full test suite + manual scenarios: 225 passed, 0 failed, 3 ignored (live API tests). All new tests pass: nonempty_section_subheading_counts_as_body ✅, nonempty_section_subheading_only_no_text ✅, test_validate_path_allows_relative_within_workspace ✅, test_validate_path_rejects_relative_with_traversal ✅.
