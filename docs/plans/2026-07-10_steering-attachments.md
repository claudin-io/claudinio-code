# Steering Attachments

## Context / Problem Statement
Steering messages (sent while the agent is thinking) are **text-only**. The frontend `send()` function returns early before processing attachments, and the backend `queue_steering` command accepts only `text: String`. The `SteeringCtl` stores a `Vec<String>` with no attachment data type whatsoever. Normal messages have full attachment support (images compressed→base64 `ContentBlock::image`, text files inlined as code blocks, binary files as name/size reference), but none of this is available during steering.

## Goal (Definition of Done)
Users can attach files (images, text files, binary files — same types as normal messages) when sending steering messages. Attachments appear as pills below the steering pill in the timeline, are shown in the queued-steering bar above the input, and persist as lightweight metadata in the JSONL so they survive page reload.

## Key Findings (Prova Real)
1. **`SteeringCtl.queue`** (`session.rs:226`): `StdMutex<Vec<String>>` — only strings, no attachment support.
2. **`queue_steering` command** (`agent.rs:610-620`): takes only `session_id` and `text`, returns early if session not running.
3. **`inject_steering`** (`session.rs:637-657`): creates only `ContentBlock::text(text)` — no image/content blocks for attachments.
4. **`SessionRecord::Steering`** (`persist.rs:49`): `{ text: String, ts: u64 }` — no attachment metadata.
5. **`AgentEvent::SteeringInjected`** (`session.rs:482-484`): `{ text: String }` — no attachment data for the frontend.
6. **`ChatPanel.send()`** (`ChatPanel.tsx:1245-1255`): early `return` when `status() === "thinking"` — attachments signal is never read.
7. **`queueSteering` IPC** (`ipc.ts:292-295`): `(sessionId, text)` — no attachments parameter.
8. **`TimelineItem.steering`** (`ChatPanel.tsx:106`): `{ text: string }` — no attachment metadata.
9. **`recordsToMessages`** (`ChatPanel.tsx:383-384`): reads only `rec.text` from steering records — no attachment metadata.
10. **Timeline steering render** (`ChatPanel.tsx:2301-2309`): only shows a text pill (50-char truncation) with "steering" label.
11. **Queued steering bar** (`ChatPanel.tsx:1660-1671`): renders from `queuedSteering()` signal (`string[]`) — text pills only.
12. **`fold_into_history`** (`persist.rs:288-310`): steering records create only `ContentBlock::text` — no attachment blocks.
13. **Attachment processing** (`agent.rs:218-300`): the reading/compression/encoding logic lives inline in `send_message` — needs extraction for reuse.

## Authoritative Inputs
- **File types for full parity** (per user): images (png, jpg, jpeg, gif, webp, bmp), text files (txt, md, csv, json, yaml, yml, toml, rs, ts, tsx, js, jsx, py, swift, go, rb, html, htm, css, sh, bash, sql, xml, log), binary files (pdf, audio, video, office docs, etc.).
- **Attachment pill display** (per user): show attachment pills (icon + name + size) below the steering pill in the timeline, same as normal messages.
- **Persistence** (per user): persist lightweight metadata (name, mediaType, size) in `SessionRecord::Steering` — enough to show pills on reload, no base64 content.
- **Queued steering bar** (per user): show attachment pills alongside text pills in the bar above the input.

## Changes (Steps)

### Phase 1: Rust Backend — Data Structures

1. **Extract attachment processing into a shared function** (`src-tauri/src/commands/agent.rs`)
   - Target: `agent.rs` lines 218-300
   - Mutation: Extract the `for att in atts { ... }` loop into a new `pub fn process_attachments(atts: &[AttachmentInput]) -> Vec<(ContentBlock, AttachmentMeta)>` that returns both content blocks and metadata.
   - Why: Both `send_message` and `queue_steering` need to process attachments.
   - Constraints: Return a tuple `(ContentBlock, AttachmentMeta)` where `AttachmentMeta` has `{ name: String, media_type: String, size: u64 }`.

2. **Add `AttachmentMeta` struct** (`src-tauri/src/agent/persist.rs`)
   - Mutation: New struct: `pub struct AttachmentMeta { pub name: String, pub media_type: String, pub size: u64 }` with Serialize/Deserialize.
   - Why: Lightweight metadata for persistence and frontend events.

3. **Update `SessionRecord::Steering`** (`src-tauri/src/agent/persist.rs` line 49/288)
   - Mutation: Add `attachments: Option<Vec<AttachmentMeta>>` field.
   - Why: Persist attachment metadata so pills reappear on reload.

4. **Update `AgentEvent::SteeringInjected`** (`src-tauri/src/agent/session.rs` lines 482-484)
   - Mutation: Add `attachments: Option<Vec<AttachmentMeta>>` field to the variant.
   - Why: Frontend needs metadata to show pills in the timeline.

5. **Add `SteeringEntry` struct and update `SteeringCtl`** (`src-tauri/src/agent/session.rs` lines 225-248)
   - Mutation: Define `pub struct SteeringEntry { pub text: String, pub attachments: Vec<(ContentBlock, AttachmentMeta)> }`. Change `SteeringCtl.queue` from `StdMutex<Vec<String>>` to `StdMutex<Vec<SteeringEntry>>`. Update `drain()`, `push()`, `clear()` accordingly.
   - Why: Store pre-processed attachment content blocks alongside text so `inject_steering` can use them.

### Phase 2: Rust Backend — Command & Injection

6. **Update `queue_steering` command** (`src-tauri/src/commands/agent.rs` lines 610-620)
   - Mutation: Add `attachments: Option<Vec<AttachmentInput>>` parameter. Call `process_attachments` to pre-process files. Push a `SteeringEntry` with both text and processed attachment data.
   - Why: Accept and process attachments when queuing steering.

7. **Update `inject_steering`** (`src-tauri/src/agent/session.rs` lines 637-657)
   - Mutation: For each `SteeringEntry`, create `ContentBlock::text(text)` + all attachment content blocks, persist `SessionRecord::Steering` with attachment metadata, emit `SteeringInjected` with attachment metadata.
   - Why: Inject attachments as proper content blocks, persist metadata, and notify frontend.

8. **Update `fold_into_history`** (`src-tauri/src/agent/persist.rs` lines 288-310)
   - Mutation: When processing `SessionRecord::Steering` with attachments, create text block + additional text blocks referencing each attachment (name, type, size) since we don't persist base64 — same as binary file references in normal messages.
   - Why: On reload, steering attachments are referenced by metadata; actual file content is not re-encoded.

9. **Update `send_message` residual steering drain** (`src-tauri/src/commands/agent.rs` lines ~160-180)
   - Mutation: When draining residual steering from a previous run, also collect attachment blocks from any residual `SteeringEntry`s and prepend them to the new message's `attachment_blocks`.
   - Why: If a user queued steering with attachments before the run ended, those attachments should carry over to the new message.

### Phase 3: TypeScript Frontend — IPC & Signals

10. **Update `queueSteering` IPC** (`src/lib/ipc.ts` line 292-295)
    - Mutation: Add `attachments?: AttachmentInput[]` parameter, pass it in `invoke("queue_steering", { sessionId, text, attachments })`.
    - Why: Frontend needs to pass attachment paths for steering.

11. **Update `TimelineItem` interface** (`src/components/ChatPanel.tsx` line 98-120)
    - Mutation: Change `steering?: { text: string }` to `steering?: { text: string; attachments?: Array<{ name: string; mediaType: string; size: number }> }`.
    - Why: Store attachment metadata in timeline items for rendering.

12. **Update `AgentEvent` type** (`src/lib/ipc.ts` line 148)
    - Mutation: Change `SteeringInjected` data from `{ text: string }` to `{ text: string; attachments?: Array<{ name: string; mediaType: string; size: number }> }`.
    - Why: Receive attachment metadata from the backend event.

13. **Update `ChatPanel.send()`** (`src/components/ChatPanel.tsx` lines 1245-1255)
    - Mutation: Before the steering early-return, capture `attachments()` into the `queueSteering` call and the `queuedSteering` signal. Pass `attachments` to `queueSteering(sid, text, atts.map(a => ({ path: a.path })))`. Update `setQueuedSteering` to store objects with both text and attachment metadata. Clear `setAttachments([])` after queuing.
    - Why: Don't discard attachments when sending steering.

14. **Update queued steering signal and bar** (`src/components/ChatPanel.tsx` line 440, 1660-1671)
    - Mutation: Change `createSignal<string[]>` to `createSignal<QueuedSteeringEntry[]>`. In the bar UI, for each entry show the text pill AND attachment pills (icon + name, same style as the existing attachment pills bar at line 1692).
    - Why: Show attachment pills in the queued-steering bar.

### Phase 4: TypeScript Frontend — Rendering

15. **Update `recordsToMessages`** (`src/components/ChatPanel.tsx` line 383-384)
    - Mutation: When `kind === "steering"`, also read `rec.attachments` and pass it to the timeline item: `steering: { text: String(rec.text ?? ""), attachments: rec.attachments }`.
    - Why: Rebuild steering timeline items with attachment metadata on reload.

16. **Update `SteeringInjected` event handler** (`src/components/ChatPanel.tsx` lines 1077-1082)
    - Mutation: Pass `event.data.attachments` through to the timeline item's `steering.attachments`.
    - Why: Live steering events include attachment metadata.

17. **Update steering timeline rendering** (`src/components/ChatPanel.tsx` lines 2301-2309)
    - Mutation: Below the existing steering text pill, add a `Show when={step.steering!.attachments?.length}` block that renders attachment pills (icon + name + size) in the same style as message attachment pills.
    - Why: Show attachment pills below the steering pill in the timeline.

### Phase 5: Tests

18. **Update IPC test** (`src/lib/ipc.test.ts` lines 588-593)
    - Mutation: Update the `queueSteering` test to verify attachments are passed to `invoke`.
    - Why: Keep test coverage current.

19. **Update persist tests** (`src-tauri/src/agent/persist.rs` lines 584-650)
    - Mutation: Update `history_from_records_with_steering_merges_into_last_user` and `history_from_records_steering_merges_into_existing_user` to also test steering with attachment metadata.
    - Why: Keep test coverage current.

## Verification Plan

1. **Dry-run: cargo check** — verify all Rust changes compile.
2. **Dry-run: cargo test** — run existing tests, confirm no regressions.
3. **Dry-run: TypeScript typecheck** — `npx tsc --noEmit` or equivalent, confirm no type errors.
4. **Apply: Build** — `cargo build` to verify full compilation.
5. **End-to-end**: Start the app, start an agent run that takes time (e.g., ask it to list a large directory), attach a file, send a steering message → verify:
   - Attachment pills appear in the queued-steering bar
   - After injection, attachment pills appear below the steering pill in the timeline
   - The agent receives and processes the attachment (visible in tool calls or responses)
6. **Reload persistence**: After steering with attachment is injected, reload the page → verify attachment pills still appear in the timeline.
7. **Regression**: Send a normal (non-steering) message with attachments when the agent is idle → verify unchanged behavior.
8. **Edge case — empty text**: Queue a steering with only attachments and empty text → verify it's handled gracefully (or rejected appropriately).

## Risks
- **Module visibility**: `process_attachments` needs to be `pub` and importable from `session.rs`. The `compress_image` function is in `commands/agent.rs` and may need to move to a shared location or be made `pub`.
- **ContentBlock import**: `SteeringCtl` lives in `session.rs`; `ContentBlock` is in `provider.rs`. Already imported — no issue.
- **Attachment processing in queue_steering vs send_message duplication**: We're extracting a shared function, which is the right solution.


## Implementation Log — 2026-07-10 23:46
**Summary:** Adiciona suporte a anexos no steering: usuários podem enviar arquivos (imagens, texto, binários) enquanto o agente está pensando, com pills de anexo na barra de queued steering e na timeline, persistindo metadados leves no JSONL.
**Changed files:** M src-tauri/src/agent/persist.rs, M src-tauri/src/agent/session.rs, M src-tauri/src/commands/agent.rs, M src/components/ChatPanel.tsx, M src/lib/ipc.test.ts, M src/lib/ipc.ts, ?? docs/plans/2026-07-09_deploy-tag-0-1-1.md, ?? docs/plans/2026-07-10_steering-attachments.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Implementation Journal: Steering Attachments

### Key decisions
- **AttachmentMeta struct**: Created in persist.rs with name/media_type/size — full MIME types (image/png, text/plain) so the frontend's `startsWith("image/")` icon logic works correctly during queued steering bar rendering.
- **process_attachments extracted**: The inline processing loop in send_message was extracted into a shared pub function that returns `Vec<(ContentBlock, AttachmentMeta)>`. Both send_message and queue_steering use it now.
- **SteeringEntry**: Replaced raw `Vec<String>` in SteeringCtl with `Vec<SteeringEntry>` where each entry carries text + pre-processed attachment data. The pre-processing happens at queue time (during queue_steering), not at injection time, so the files are read immediately.
- **Persist only metadata**: SessionRecord::Steering stores `Option<Vec<AttachmentMeta>>` — just name/media_type/size, no base64 body. On history rebuild (fold_into_history), attachments generate text reference blocks like `[Anexo do steering: \`name\` (1.2 KB) — tipo: image/png]`.
- **Tauri Option serialization**: Used `attachments ?? null` in TypeScript IPC so Tauri receives `null` (not `undefined`), which correctly deserializes as Rust `None`.

### Gotchas
- The `media_type` field in AttachmentMeta was initially set to the file extension (e.g. "png") but the frontend Icon component checks `startsWith("image/")` — so it had to be changed to full MIME types in the process_attachments function.
- The SteeringInjected event handler in ChatPanel.tsx filtered queued steering by `s !== event.data.text` (literal string comparison). Since queuedSteering changed from `string[]` to `QueuedSteeringEntry[]`, this needed to be `s.text !== event.data.text`.
- The `compress_image` function is private in commands/agent.rs, but `process_attachments` lives in the same file so no export needed.
- Tauri commands use `attachments: attachments ?? null` to pass `null` instead of `undefined` — Tauri/Rust serde expects `null` for `Option::None`.

### Files changed
- src-tauri/src/agent/persist.rs — AttachmentMeta, updated SessionRecord::Steering, updated fold_into_history
- src-tauri/src/agent/session.rs — SteeringEntry, updated SteeringCtl, updated SteeringInjected event, updated inject_steering
- src-tauri/src/commands/agent.rs — extracted process_attachments, updated send_message and queue_steering
- src/components/ChatPanel.tsx — TimelineItem type, QueuedSteeringEntry, send(), recordsToMessages, event handler, queued bar render, timeline render
- src/lib/ipc.ts — queueSteering signature, AgentEvent type
- src/lib/ipc.test.ts — queueSteering tests

**Task journal:**
- Add AttachmentMeta struct to persist.rs: Added AttachmentMeta struct with Serialize, Deserialize, Debug, Clone derives.
- Add SteeringEntry struct and update SteeringCtl: Added SteeringEntry struct, updated SteeringCtl queue type, drain/push signatures.
- Extract attachment processing into shared function: Extracted process_attachments function that returns Vec<(ContentBlock, AttachmentMeta)>. Fixed media_type to use proper MIME types.
- Update queue_steering command to accept attachments: Added attachments: Option<Vec<AttachmentInput>> parameter. Pre-processes files and pushes SteeringEntry with attachment data.
- Update SteeringInjected event with attachment metadata: Added attachments field to SteeringInjected event variant.
- Update SessionRecord::Steering with attachments: Updated Steering variant in SessionRecord enum. Updated fold_into_history to generate text blocks for each attachment.
- Update inject_steering to use SteeringEntry: Updated inject_steering to iterate SteeringEntry items, build content blocks and metadata, persist and emit events.
- Update fold_into_history for steering attachments: Updated fold_into_history to generate text blocks for each attachment from metadata only (no base64).
- Update residual steering drain in send_message: Added loop after residual drain to collect attachment blocks from SteeringEntry items.
- Update queueSteering IPC function: Updated queueSteering signature to accept attachments param.
- Update AgentEvent SteeringInjected type: Extended SteeringInjected type with attachments field.
- Update TimelineItem steering interface: Updated TimelineItem.steering type with attachments field.
- Update ChatPanel.send() to pass attachments in steering: Updated send() to capture attachments() before steering early-return, pass to queueSteering and setQueuedSteering, then clear attachments.
- Update queued steering signal and bar UI: Changed createSignal<string[]> to createSignal<QueuedSteeringEntry[]>. Added QueuedSteeringEntry interface. Bar now renders attachment pills alongside text pills.
- Update recordsToMessages for steering attachments: Updated recordsToMessages to pass rec.attachments through to timeline item.
- Update SteeringInjected live event handler: Updated handler to filter by s.text and forward event.data.attachments.
- Update steering timeline rendering with attachment pills: Added attachment pills rendering below the steering text pill in the timeline.
- Update IPC test for queueSteering with attachments: Updated existing test to expect attachments: null. Added new test for passing attachments.
- Update persist tests for steering with attachments: Updated both persist tests to include attachments field. The steering-with-attachment test checks 3 content blocks (2 text + 1 attachment reference).
- Build, test, and verify everything compiles: cargo check: clean. cargo test: 161 passed, 1 pre-existing unrelated failure (test_read_file_large_truncated). npx tsc --noEmit: no errors in our files (all pre-existing). vitest run ipc.test.ts: 85/85 passed.
