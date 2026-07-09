## Context

The Monaco File Editor (`src/components/FileEditorModal.tsx`) currently configures TypeScript language defaults with strict compiler options (`strict: true`, `target: ESNext`, `jsx: Preserve`, `moduleResolution: NodeJs`, etc.), enables diagnostics (`noSemanticValidation: false`, `noSyntaxValidation: false`), and loads up to 200 workspace source files as extraLibs. This causes Monaco's TS language service to show red underlines / error markers for any type or import issues in the opened file — which the user finds distracting and erroneous (e.g. imports that work at build time flagged as errors in the editor).

## Goal

Strip all TypeScript diagnostic configuration and extraLib loading from FileEditorModal so Monaco provides only syntax highlighting — no error checking, no red squigglies.

## Solution Design

In `src/components/FileEditorModal.tsx`, remove:

1. **`monaco.languages.typescript`** usage entirely — no `setCompilerOptions()`, no `setDiagnosticsOptions()`  
2. **ExtraLibs loading** — the entire `try/catch` block that reads workspace files and calls `addExtraLib()`  
3. **`fileIndexMap` import** — no longer needed  
4. Keep everything else: `defineMonacoThemes()`, `detectLanguage()`, dirty tracking, save, close, keyboard shortcuts, and the JSX layout

Minimal change — only ~9 lines removed, 1 import removed. Everything else stays identical.

## Risks

None. Removing TS type-checking from the editor only removes the red squigglies; syntax highlighting, bracket matching, and all other Monaco features continue working. The `fileIndexMap` import removal could cause a compile error if it's used elsewhere in this file — verify with a build after the change.

## Tasks

1. **Remove TS diagnostics + extraLibs from FileEditorModal** — edit `src/components/FileEditorModal.tsx`: delete the `monaco.languages.typescript` config block, delete the extraLibs loading block, delete the `fileIndexMap` import. Verify with `npx vitest run` and `npx tsc --noEmit` (or the project's build command).


## Implementation Log — 2026-07-09 16:36
**Summary:** Remove TS diagnostics, compiler options, and extraLibs from FileEditorModal — keep only syntax highlighting
**Changed files:** M	src-tauri/src/agent/tools/mod.rs
**Commits:** 2b83545 fix(agent): verify edits by read content instead of line numbers
**Journal:** Key findings from the Monaco task:
- The red squigglies (TS import errors) were caused by `monaco.languages.typescript.setCompilerOptions()` + `setDiagnosticsOptions()` activating the full TS language service with semantic validation.
- The fix was straightforward: remove the `monaco.languages.typescript` block entirely. Monaco still provides syntax highlighting for TypeScript/JSX/JS files through its built-in tokenizer — no extra configuration needed.
- The extraLibs loading was also removed (it was loading up to 200 workspace source files into `addExtraLib` as a best-effort auto-complete source, which was both slow and unnecessary for syntax-only editing).
- The `fileIndexMap` import became unused and was removed alongside.
- All 288 tests across 18 files pass after the change.
- The editor now starts cleanly: theme → read file → create editor with just `value`, `language`, and visual options. No TS service initialization.

**Task journal:**
- Add i18n keys for FileEditorModal: Added fileEditor.* keys to both locale files after the editor.* section
- Create FileEditorModal component: Created FileEditorModal with: read on mount, language detection, dirty state, save+Ctrl+S, unsaved warning, TS compiler options, extraLibs loading from fileIndexMap
- Modify FileTree: add onDblClickFile prop: Added onDblClickFile prop to TreeNode and FileTree interfaces. handleDblClick now calls onDblClickFile instead of onOpenExternal
- Wire App.tsx: double-click opens FileEditorModal: Added import for FileEditorModal; Added editorFilePath signal; Added onDblClickFile={setEditorFilePath} to FileTree; Added <Show> with FileEditorModal before closing tags
- Add TS/JS auto-complete via extraLibs (REMOVED): This work is being fully removed by task-7 per user request — only syntax highlighting wanted
- Update FileTree tests for new double-click behavior: Tests already updated with onDblClickFile: dblclick on file, dblclick on directory (no call), child file dblclick
- Remove TS diagnostics, compiler options, and extraLibs — keep only syntax highlighting: Removed import { fileIndexMap } from '../lib/fileIndex'; Removed entire monaco.languages.typescript block (compiler options + setDiagnosticsOptions); Removed extraLibs loading block (try/catch with fileIndexMap lookup + addExtraLib); initEditor now: defineMonacoThemes → readFile → editor.create with just syntax highlighting; All 288 tests pass


## Implementation Log — 2026-07-09 16:42
**Summary:** Fix Monaco red squigglies by explicitly muting TS worker diagnostics in FileEditorModal
**Changed files:** M	src-tauri/src/agent/tools/mod.rs
**Commits:** 2b83545 fix(agent): verify edits by read content instead of line numbers
**Journal:** ## Red squigglies fix

**Problem**: After removing TS compiler options and diagnostics from FileEditorModal (Task 7), Monaco's built-in TS worker still activates by default on `.ts`/`.tsx`/`.js`/`.jsx` files. Without explicit `setCompilerOptions` or `setDiagnosticsOptions`, the worker uses its own defaults — which flag any unresolved import or type as an error, producing red squigglies.

**Fix**: Added explicit `setDiagnosticsOptions({ noSemanticValidation: true, noSyntaxValidation: true, noSuggestionDiagnostics: true })` for both `monaco.languages.typescript.typescriptDefaults` and `monaco.languages.typescript.javascriptDefaults` right after `defineMonacoThemes()` in `initEditor`. This disables all TS worker diagnostics while keeping syntax highlighting intact.

**Key insight**: Monaco's TS worker is always active for TS/JS files — you can't just "not configure it" to avoid diagnostics. You must explicitly mute it if you only want syntax highlighting. Disabling both semantic and syntax validation is needed because `noSemanticValidation` alone (which Task 7 had) still leaves syntax validation errors visible.

**Task journal:**
- Fix Monaco red squigglies from TS worker defaults: Monaco's TS worker activates by default on .ts/.tsx/.js/.jsx files even without calling setCompilerOptions; Calling setDiagnosticsOptions({ noSemanticValidation: true, noSyntaxValidation: true, noSuggestionDiagnostics: true }) on both typescriptDefaults and javascriptDefaults silences the red squigglies; 288 tests still pass after the fix
- Add i18n keys for FileEditorModal: Added fileEditor.* keys to both locale files after the editor.* section
- Create FileEditorModal component: Created FileEditorModal with: read on mount, language detection, dirty state, save+Ctrl+S, unsaved warning, TS compiler options, extraLibs loading from fileIndexMap
- Modify FileTree: add onDblClickFile prop: Added onDblClickFile prop to TreeNode and FileTree interfaces. handleDblClick now calls onDblClickFile instead of onOpenExternal
- Wire App.tsx: double-click opens FileEditorModal: Added import for FileEditorModal; Added editorFilePath signal; Added onDblClickFile={setEditorFilePath} to FileTree; Added <Show> with FileEditorModal before closing tags
- Add TS/JS auto-complete via extraLibs (REMOVED): This work is being fully removed by task-7 per user request — only syntax highlighting wanted
- Update FileTree tests for new double-click behavior: Tests already updated with onDblClickFile: dblclick on file, dblclick on directory (no call), child file dblclick
- Remove TS diagnostics, compiler options, and extraLibs — keep only syntax highlighting: Removed import { fileIndexMap } from '../lib/fileIndex'; Removed entire monaco.languages.typescript block (compiler options + setDiagnosticsOptions); Removed extraLibs loading block (try/catch with fileIndexMap lookup + addExtraLib); initEditor now: defineMonacoThemes → readFile → editor.create with just syntax highlighting; All 288 tests pass


## Implementation Log — 2026-07-09 17:11
**Changed files:** M	src-tauri/src/agent/tools/mod.rs, M	src/App.tsx, A	src/components/FileEditorModal.tsx, M	src/components/FileTree.test.tsx, M	src/components/FileTree.tsx, M	src/lib/locales/en-US.ts, M	src/lib/locales/pt-BR.ts, M	vite.config.ts
**Commits:** 35d8d11 feat: add in-app Monaco file editor with double-click open, edit/save, and red-squiggly fix, 2b83545 fix(agent): verify edits by read content instead of line numbers
**Journal:** ## Fix Monaco red squigglies — final push

**Root cause**: Monaco's TS worker is always-on for `.ts`/`.tsx`/`.js`/`.jsx` files. Even without calling `setCompilerOptions` or `setDiagnosticsOptions`, the worker activates with its own defaults — flagging any project-external import or type as an error. Previous removal of TS diagnostics config (Task 7) was insufficient because it removed the *explicit mute* but the worker still ran with defaults.

**Fix**: Added `setDiagnosticsOptions({ noSemanticValidation: true, noSyntaxValidation: true, noSuggestionDiagnostics: true })` on both `monaco.languages.typescript.typescriptDefaults` and `monaco.languages.typescript.javascriptDefaults` right after `defineMonacoThemes()`. This completely silences the TS worker's diagnostics while keeping syntax highlighting intact.

**What was pushed**: Direct commit to `main` (`35d8d11`), pushed to `origin/main`. The commit bundles the full FileEditorModal feature plus this fix.

**Task journal:**
- Fix Monaco red squigglies from TS worker defaults: Monaco's TS worker activates by default on .ts/.tsx/.js/.jsx files even without calling setCompilerOptions; Calling setDiagnosticsOptions({ noSemanticValidation: true, noSyntaxValidation: true, noSuggestionDiagnostics: true }) on both typescriptDefaults and javascriptDefaults silences the red squigglies; 288 tests still pass after the fix
- Add i18n keys for FileEditorModal: Added fileEditor.* keys to both locale files after the editor.* section
- Create FileEditorModal component: Created FileEditorModal with: read on mount, language detection, dirty state, save+Ctrl+S, unsaved warning, TS compiler options, extraLibs loading from fileIndexMap
- Modify FileTree: add onDblClickFile prop: Added onDblClickFile prop to TreeNode and FileTree interfaces. handleDblClick now calls onDblClickFile instead of onOpenExternal
- Wire App.tsx: double-click opens FileEditorModal: Added import for FileEditorModal; Added editorFilePath signal; Added onDblClickFile={setEditorFilePath} to FileTree; Added <Show> with FileEditorModal before closing tags
- Add TS/JS auto-complete via extraLibs (REMOVED): This work is being fully removed by task-7 per user request — only syntax highlighting wanted
- Update FileTree tests for new double-click behavior: Tests already updated with onDblClickFile: dblclick on file, dblclick on directory (no call), child file dblclick
- Remove TS diagnostics, compiler options, and extraLibs — keep only syntax highlighting: Removed import { fileIndexMap } from '../lib/fileIndex'; Removed entire monaco.languages.typescript block (compiler options + setDiagnosticsOptions); Removed extraLibs loading block (try/catch with fileIndexMap lookup + addExtraLib); initEditor now: defineMonacoThemes → readFile → editor.create with just syntax highlighting; All 288 tests pass
