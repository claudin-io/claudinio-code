# Prompt Enhancer: project-context grounding (two-step retrieval)

## Problem

`enhance_prompt` validated the workspace but discarded the handle (`let _ws = ...`),
so the enhancer never saw the semantic index. A draft like *"Investigar se o
embedding está acontecendo a cada hora..."* came back better written but with no
real file paths or symbol names.

## Design

Two-step retrieval inside `src-tauri/src/commands/enhance.rs`, all best-effort
(any failure silently degrades to the old behavior):

1. **Query generation** — a cheap `provider::one_shot` call
   (`QUERY_GEN_SYSTEM_PROMPT`, 500 tokens) turns the draft + last 3 messages into
   1–3 **English** code-search queries (the MiniLM embedding index is
   English-only and the user writes pt-BR). Outputs `NONE` for non-code drafts.
2. **Index search** — per query: `encode_query` on the shared embedder →
   `IndexDb::search_by_embedding` (4 hits/query), lexical `search_symbols`
   fallback when the embedder isn't loaded yet. Dedupe by `symbol_id`, cap 8
   total, snippets ≤15 lines / 1000 chars each.
3. **Git state** — reuses `commands::git::{git_branch, git_status}` directly
   (they take a plain `String`, no `State`), run in `spawn_blocking`, capped at
   20 files.

New sections `=== GIT STATE ===` and `=== RELEVANT CODE (from project index) ===`
are inserted before `=== DRAFT PROMPT ===`. The `ENHANCER_SYSTEM_PROMPT` gained
one rule framing them as reference-only material — cite paths/symbols that match
the intent, but never expand scope or length because of them (preserves the
2026-07-11 verbosity fix).

No frontend/IPC/locale changes — `workspace` was already passed.

## Verification (2026-07-11, live API)

- `cargo check` clean.
- Query-gen prompt with the Portuguese embedding draft → 3 good English queries
  (`embedding scheduled task cron interval`, ...). Note: claudinio returns a
  `thinking` block before `text`; `one_shot` already filters to text blocks, but
  the token budget was raised to 500 to leave room.
- Enhancer prompt with injected code context → concise Portuguese prompt citing
  `start_periodic_reindex`, `generate_all_embeddings`, `symbols_without_embeddings`;
  no scope inflation.
- Greeting draft → query gen returns exactly `NONE` (retrieval skipped).

## Fix: role drift (same day)

In the real app the enhancer sometimes ANSWERED the draft instead of rewriting
it ("Looking at the code... Let me trace the flow" + a mimicked `/grep` call).
Rich RELEVANT CODE sections plus agent-style `[assistant]` history push the
model into agent mode. Two changes:

1. `ENHANCER_SYSTEM_PROMPT` reframed as "prompt REWRITER": explicit "You are
   NOT the agent — never answer, investigate, trace code, or suggest commands",
   plus a same-language-as-draft rule.
2. A `=== YOUR TASK ===` re-anchor block appended AFTER the draft in the user
   message, so the last thing the model reads is the rewrite instruction, not
   the retrieved context.

Verified 3/3 live runs with adversarial agent-style history stay in role and
still cite real symbols.

## Fix: draft returned unchanged (same day)

After the role-drift hardening the enhancer started returning the draft
verbatim — the "if already clear, return unchanged" rule was winning over the
grounding rule. Changes:

1. Grounding made assertive: "when RELEVANT CODE clearly matches, you SHOULD
   anchor with file paths/symbols — a draft that names no concrete code counts
   as improvable, not 'already effective'". Unchanged is allowed ONLY when
   unambiguous AND no matching retrieved code.
2. Silent-degradation paths now log `[enhance] ...` via eprintln (index empty,
   query-gen NONE/failure, generated queries, lexical fallback, result count)
   so a missing RELEVANT CODE section is diagnosable from the terminal.

Verified 3/3 live runs (no-history scenario, code context present) anchor the
Portuguese draft with `start_periodic_reindex`/`generate_all_embeddings`
citations at roughly the original length.
