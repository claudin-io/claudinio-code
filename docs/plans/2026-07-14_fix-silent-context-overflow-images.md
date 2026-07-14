# Fix: estouro de contexto silencioso com imagens anexadas

## Context

Na sessão `57d770c6`, 3 imagens coladas no primeiro turno estouraram o contexto (223k tokens estimados) e a chamada morreu sem resposta nem erro (`input_tokens: 0`). A investigação revelou dois problemas independentes:

1. **Erro engolido**: o proxy (claudin.io/LiteLLM) aceita a requisição com HTTP 200 e emite o erro de contexto como evento SSE `error` mid-stream. O parser em `process_line` não tem case para `"error"` — o frame cai no `_ => {}` e o stream termina "vazio com sucesso". Nenhum `AgentEvent::Error` chega à UI.
2. **Estouro evitável**: imagens não são normalizadas por custo de token. `compress_image` só age em arquivos >200KB (critério de bytes) e redimensiona até 2048px; a estimativa de tokens usa `base64.len()/3` (errado para imagens, que custam ~pixels/750); e o teto local é 256k quando o real é 200k.

Decisão de design: **não** usar subagents por imagem — com resize adequado (≤1568px, teto do custo da Anthropic ≈1.600 tokens/imagem), 3 imagens custam ~5k tokens e o raciocínio conjunto imagem+código é preservado.

## Changes

### 1. Surfar erros SSE mid-stream (root cause do silêncio)
`src-tauri/src/agent/provider.rs`, `process_line` (~linha 961-1056):
- Adicionar arm para evento `"error"`: parsear `{"type":"error","error":{"type","message"}}` e retornar `Err` com a mensagem da API.
- O Err propaga por `stream_message_with_retry` → `AgentEvent::Error` (agent.rs:352) → barra de erro existente na UI (ChatPanel.tsx:1262-1275). Nada novo na UI.
- Em `is_retryable_error` (session.rs:933-946), garantir que erros contendo "context" / "prompt is too long" NÃO sejam retryable (já é o default para 400, só confirmar que a nova mensagem não casa com padrões retryable).

### 2. Redimensionar imagens pelo custo real de token
`src-tauri/src/commands/agent.rs`, `compress_image` (linhas 20-64):
- Trocar o gate de bytes (<200KB skip) por gate de dimensão: decodificar sempre; se o lado maior > **1568px**, redimensionar para 1568px (limite a partir do qual a Anthropic redimensiona server-side sem ganho de qualidade).
- Manter re-encode JPEG q80 (e PNG→JPEG para fotos) como hoje.
- Retornar também as dimensões finais `(w, h)` para o passo 3.

### 3. Corrigir estimativa de tokens para imagens
`src-tauri/src/agent/session.rs`, `estimate_message_tokens` (46-49) / `estimate_tokens` (51-62):
- Para blocos `ContentBlock::Image`, estimar `tokens = (w*h)/750` — ou, se as dimensões não estiverem disponíveis no ponto da estimativa, usar constante conservadora **1.600 tokens/imagem** (custo máximo pós-resize), em vez de `base64.len()/3` que superestima ~50x.
- Demais blocos continuam com `len/3`.

### 4. Alinhar teto de contexto ao real
`src-tauri/src/agent/session.rs` linha 16: `MAX_CONTEXT_TOKENS: 256_000 → 200_000`. `COMPACT_THRESHOLD` (75%) recalcula sozinho para 150k.

### 5. Guard pré-flight (proteção final)
`src-tauri/src/agent/session.rs`, no bloco de pré-compactação (1103-1152): se após a (tentativa de) compactação a estimativa ainda exceder `MAX_CONTEXT_TOKENS`, retornar erro amigável ("A mensagem excede o limite de contexto — reduza os anexos") em vez de chamar a API.

## Verification
- `cargo test` no `src-tauri` (adicionar teste unitário para o novo arm de erro em `process_line` com um frame SSE `event: error` sintético, e para `compress_image` com uma imagem >1568px verificando dimensão de saída).
- Manual: reproduzir o cenário original — colar 3 screenshots grandes no modo brain e enviar. Esperado: imagens redimensionadas, requisição passa; e, forçando um erro (ex: imagem gigante com o resize desativado), a barra de erro aparece na UI em vez de silêncio.


## Implementation Log — 2026-07-14 12:05
**Summary:** Fix silent context overflow with attached images: SSE error handling, image resize at 1568px, token estimation for images, align context window to 200k, and pre-flight guard.
**Changed files:** M src-tauri/src/agent/provider.rs, M src-tauri/src/agent/session.rs, M src-tauri/src/agent/subagent.rs, M src-tauri/src/commands/agent.rs, ?? docs/plans/2026-07-14_fix-silent-context-overflow-images.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Key decisions and findings

1. **SSE error surfacing**: The root cause of the silent failure was that SSE `error` events from the API (e.g. context overflow, model overloaded) fell into the `_ => {}` catch-all in `process_line`. The API returns HTTP 200 with the error as an SSE event mid-stream — the parser silently dropped it. Added `"error"` arm that parses `{type:"error",error:{type,message}}` and returns `Err()`. The `is_retryable_error` check in `session.rs` correctly treats this as non-retryable (the new error format is "API error: type — message" which doesn't match the retryable pattern "API error: HTTP NNN").

2. **Image resize strategy**: Changed from byte-size gate (<200KB skip) to always-decode-and-check-dimensions approach. The key insight is that the Anthropic API server-side resizes images at 1568px threshold — resizing beyond that on the client creates no quality benefit. Also reduced max_dim from 2048 to 1568. Added width/height to the return value and stored them in ImageSource (non-serialized). This feeds the token estimator.

3. **Token estimation for images**: The old `estimate_message_tokens` serialized the entire message to JSON and used `len()/3`, which for images includes the full base64 data — overestimating by ~50x. New implementation iterates content blocks: for Image blocks with known dimensions uses `w*h/750`, without dimensions uses conservative 1600 (max post-resize cost). Text/tool blocks still use `json.len()/3`.

4. **Context window alignment**: The real model limit is 200k not 256k. COMPACT_THRESHOLD auto-recalculates to 150k (75%). Two test assertions updated.

5. **Pre-flight guard**: Added in two places — before the main loop (after initial auto-compact), and inside the loop after per-round compaction. Returns friendly error in Portuguese if context still exceeds MAX_CONTEXT_TOKENS, preventing a guaranteed API failure.

6. **Bonus fix**: Fixed pre-existing test compilation error in subagent.rs — the `subagent_defs` function gained a 4th parameter (`config: &AgentConfig`) in a prior commit, but 4 test calls were not updated.

**Task journal:**
- Surfar erros SSE mid-stream: Added `error` case to process_line match in provider.rs: parses `{"type":"error","error":{"type":"...","message":"..."}}` and returns `Err()` which propagates through stream_message_with_retry into AgentEvent::Error. The existing is_retryable_error in session.rs already treats non-budget API errors with HTTP codes 500+ as retryable — this new Err format starts with "API error:" which is NOT in the retryable list (only "API error: HTTP " with numeric code is), so context errors won't loop infinitely.
- Redimensionar imagens pelo custo real de token: Removed byte-size gate (was <200KB skip). compress_image now always decodes. Reduced max_dim from 2048 to 1568 (Anthropic server-side threshold). Added width/height to return tuple and to ImageSource struct (non-serialized). Updated ContentBlock::image() constructor and call site in agent.rs.
- Corrigir estimativa de tokens para imagens: Rewrote estimate_message_tokens in session.rs to iterate over ContentBlocks. For Image blocks with dimensions, uses w*h/750. For Image blocks without dimensions, uses 1600 (max post-resize conservative). Other blocks use json.len()/3 as before.
- Alinhar teto de contexto ao real (200k): Changed MAX_CONTEXT_TOKENS from 256_000 to 200_000 in session.rs line 16. COMPACT_THRESHOLD recomputes automatically via const arithmetic to 150_000 (75%). Updated two test assertions that hardcoded the old values.
- Guard pré-flight antes da chamada à API: Added pre-flight guard in two places: (1) after initial auto-compact before the main loop, and (2) after per-round compaction inside the loop. Both estimate context and return an Err if still >= MAX_CONTEXT_TOKENS.
