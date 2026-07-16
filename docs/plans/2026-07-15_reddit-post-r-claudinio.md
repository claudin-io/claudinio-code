# Context

The user (Victor Tavernari) is writing a Reddit post for r/claudinio about **Claudinio Code**, a native desktop harness he built specifically for Claudin.io. He has a rough draft (a few sentences) and wants it improved and completed.

The audience is r/claudinio — they already know Claudin.io and have likely tried other coding assistants (Claude Code, opencode, pi.dev, etc.). The post doesn't need to explain what Claudin.io is; it needs to show why this harness unlocks more from it.

## Solution Design

### Tone & Voice
- Personal dev-story style: raw, honest, relatable
- First-person, conversational
- Not a polished product launch — a dev sharing their side project with peers

### Core Narrative Arc
1. **Frustration**: Testing existing coding assistants (opencode, pi.dev, Claude Code, etc.) — they were good, but felt like they weren't extracting Claudinio's full potential
2. **Discovery**: Realized Claudin.io could do much more with the right harness
3. **Building**: Started building Claudinio Code — a native desktop app specifically designed as a harness for Claudinio
4. **The addiction**: "I am very addicted coding with this harness" — it became his daily driver
5. **Video demo**: Show it working in action `[video demo]`
6. **What makes it special**: Key differentiators that matter to Claudin.io users

### Key Features to Highlight (tailored for r/claudinio audience)
- **Brain/Builder dual-mode**: Planning vs execution — mirrors how devs actually work
- **Native desktop** (Tauri v2 + Rust): Not a terminal, not a web app — a real desktop app with native performance
- **Code intelligence that actually understands your code**: Semantic search (ONNX embeddings running locally), FTS5, LSP, tree-sitter for 100+ languages — the agent doesn't guess, it reads your codebase
- **13+ built-in tools**: read_file, edit_file, bash, grep, semantic_search, code_search, spawn_agents, ask_user, tasks, plans, and more — all with a tiered permission system
- **Parallel subagents**: Delegate to up to 4 subagents simultaneously, each with independent context
- **Visual timeline**: Every thought, tool call, and subagent action is visible and collapsible — full transparency into what the agent is doing
- **Golden goals**: `<goal>` tags create mandatory tasks the agent MUST complete — enforced by the harness, not just suggested
- **Steering**: Send guidance mid-thought; the agent adapts in real-time
- **15 themes** with an OKLCH design system
- **Cross-platform**: Windows, macOS (ARM), Linux — with auto-updater

### Non-goals
- Not a comparison chart or benchmark post
- Not a "Claudinio vs X" post
- Not a technical architecture deep-dive (save that for a follow-up)
- Not a call for contributors (though open-source)

## Risks

- Low: If the video isn't ready, the placeholder might feel incomplete. Mitigation: the text should stand on its own.
- Low: r/claudinio audience might want more technical detail. Mitigation: include enough specific features to satisfy, and offer to do a technical deep-dive in comments.

## Non-goals

- No code changes required
- No screenshots needed (video will cover visuals)
- Not changing any project files — this is purely a writing deliverable

## Low-Level Design

This is a writing-only deliverable. No code changes. No files to touch.

**Deliverable**: A complete Reddit post body (Markdown) as agreed in the Solution Design, plus a saved `.md` file.

**Structure**:
1. Title: "A dedicated harness for Claudinio — Claudinio Code!"
2. Body: follows the narrative arc (frustration → discovery → building → addiction → video → features)
3. Natural, first-person voice
4. Features section that highlights what makes Claudinio Code special for r/claudinio readers
5. Link/mention of where to get it (GitHub releases)

**Saved file**: `/Users/victortavernari/claudinio_code/docs/2026-07-15_reddit-post.md`

**Constraints**:
- Must preserve the user's original voice and phrases ("I am very addicted coding with this harness", "claudinio is running smoothly")
- Must include `[video demo]` as a placeholder
- Must be self-contained — readable even without the video
- ~400-700 words (substantial but not a novel for Reddit)

## Tasks

- [x] Draft the complete Reddit post and save it as `docs/2026-07-15_reddit-post.md`


## Implementation Log — 2026-07-15 22:13
**Summary:** Draft and save Reddit post about Claudinio Code
**Changed files:** ?? docs/2026-07-15_reddit-post.md, ?? docs/plans/2026-07-15_reddit-post-r-claudinio.md
**Commits:** _(git unavailable or none)_
**Journal:** The post was drafted in a personal dev-story style as requested. I conducted a thorough exploration of the codebase (Rust backend, SolidJS frontend, build/CI system, and plans docs) to ground the post in concrete technical details — not generic AI hype. The post highlights features specific to Claudinio: Brain/Builder dual-mode, semantic code intelligence with ONNX embeddings, parallel subagents, the visual timeline, steering, golden goals, and plan lifecycle. The post is saved at docs/2026-07-15_reddit-post.md. Key decisions: kept user's original phrases ("I am very addicted coding with this harness", "claudinio is running smoothly") verbatim; used [video demo] placeholder since the video isn't uploaded yet; targeted r/claudinio audience who already know Claudin.io so didn't need to explain what it is. The post is ~600 words — substantial but Reddit-readable.

**Task journal:**
- Draft and save the Reddit post: Drafted complete post with title and body (~600 words). Saved to docs/2026-07-15_reddit-post.md. Verified file exists and content is intact.
