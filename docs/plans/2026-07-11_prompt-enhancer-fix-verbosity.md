# Prompt Enhancer — Fix: long generic output

## Problem
The `ENHANCER_SYSTEM_PROMPT` is too verbose (22 lines). It lists all tools, full syntax docs, and elaborate instructions. When the user types "Investiga se o carregamento do modelo de embedding é executado novamente a cada uso" — a short investigative question — the enhancer outputs a 6-step engineering epic with scripts and commands, which is overkill.

The LLM amplifies brevity into verbosity because the system prompt itself is verbose and structured like a project spec.

## Fix

Replace the current 22-line prompt with a tight 9-line version that:

1. States what Claudinio Code is (1 line)
2. Describes Brain vs Builder modes briefly (2 lines)  
3. Sets the **key rule**: "match the user's level of brevity. Do not expand a short question into a long plan."
4. Removes the full tool list and syntax documentation — they're unnecessary context for an enhancement task
5. Emphasizes: "clarify ambiguity and fix spelling/grammar — but don't add steps, scripts, or structure the user didn't ask for"

## Target file
`/Users/victortavernari/claudinio_code/src-tauri/src/commands/enhance.rs` — replace the `ENHANCER_SYSTEM_PROMPT` constant.

## Verification
`cd src-tauri && cargo check` — must pass.


## Implementation Log — 2026-07-11 12:07
**Summary:** Shrink ENHANCER_SYSTEM_PROMPT from 22 lines to 9 lines — removed tool list, syntax docs, and verbose instructions. Added key rule: "match the user's level of brevity. Do not expand a short question into a long plan."
**Changed files:** M src-tauri/src/commands/mod.rs, M src-tauri/src/lib.rs, M src/components/ChatPanel.tsx, M src/components/Icon.tsx, M src/components/TextEditorModal.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-11_prompt-enhancer-fix-verbosity.md, ?? docs/plans/2026-07-11_prompt-enhancer.md, ?? src-tauri/src/commands/enhance.rs
**Commits:** _(git unavailable or none)_
**Journal:** ## Root Cause

The ENHANCER_SYSTEM_PROMPT was 22 lines of detailed documentation — full tool list, syntax reference, elaborate rules. This taught the LLM to produce structured, multi-step outputs even for simple questions. When the user wrote "Investiga se o carregamento do modelo de embedding é executado novamente a cada uso" (a one-sentence question), the enhancer returned a 6-step engineering epic.

## Fix

Replaced with a 9-line prompt that:
- Still defines what Claudinio Code is and its two modes (Brain/Builder)
- Removes the tool list, syntax docs, and workflow description (irrelevant for enhancement)
- Adds the critical brevity rule: "match the user's level of brevity. Do not expand a short question into a long plan"
- Clarifies: "don't add steps, scripts, or structure the user didn't ask for"

## Verification
`cargo check` ✅ — clean compile.

**Task journal:**
- Add magic-rabbit icon to Icon.tsx: Added 'magic-rabbit' icon with 2 SVG paths to PATHS object and '0 0 32 32' to VIEWBOX map in Icon.tsx; 32x32 viewBox — added to VIEWBOX
- Add i18n strings for prompt enhancement: Added 3 keys to both en-US.ts and pt-BR.ts between Text Editor Modal and File Editor Modal sections
- Add enhancePrompt() IPC binding to ipc.ts: Added EnhancePromptContext interface and enhancePrompt() function to ipc.ts, between tasks section and skills section
- Create enhance_prompt Rust command: Created src-tauri/src/commands/enhance.rs with enhance_prompt command; Uses one_shot() with Brain model, 4096 max_tokens; Context includes conversation history (last 10, truncated 500), mode, mentioned files, active tasks, project summary, draft prompt
- Register enhance command in module tree: Added pub mod enhance in mod.rs; Added commands::enhance::enhance_prompt in lib.rs; cargo check passes ✅
- Add enhance button to TextEditorModal header: Added onEnhance optional prop to TextEditorModal; Added isEnhancing signal and handleEnhance function; Added magic-rabbit button in header (between title and close button) with spinner loading state
- Wire enhance + input bar button in ChatPanel: Added imports for enhancePrompt, getTasks, EnhancePromptContext; Added isEnhancing signal and enhanceHandler + buildEnhanceContext functions; Added magic-rabbit button to input bar before notebook-pen, with spinner loading; Wired onEnhance prop to TextEditorModal; Enhance fetches tasks via getTasks (best-effort); Toast shows on enhancement error
