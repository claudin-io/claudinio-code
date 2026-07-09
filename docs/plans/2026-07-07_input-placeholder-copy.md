# Solution Design: Input Placeholder Copy Update

## Context / Problem Statement

The chat input textarea placeholder currently reads:
- **PT-BR**: "Pergunte algo sobre o código…"
- **EN-US**: "Ask something about the code…"

The textarea supports three discovery/inline-mention features that users have no way of knowing about:
1. `@` — triggers `FileMentionPopover` to mention files/folders
2. `<` — triggers `TagMentionPopover` for tags
3. `<skill>` — triggers `SkillMentionPopover` for skills

The placeholder should be updated to teach users these features at a glance, following good UX copywriting principles (clarify skill).

**CONFIRMED**: Pipe-separated format, all three features, exact wording confirmed by user.

## Goal (Definition of Done)

The `chat.input.askCode` key in both locale files is updated to the new placeholder text, and the placeholder renders correctly in the textarea.

## Key Findings (Prova Real)

1. **Placeholder key**: `chat.input.askCode` — used in `ChatPanel.tsx` at the textarea's `placeholder` attribute (line ~1623).
   - **Source**: `grep` of ChatPanel.tsx, locale files `pt-BR.ts` and `en-US.ts`.
2. **Textarea features**: `@` mention popover (FileMentionPopover, line 1509-1539), `<` tag popover (TagMentionPopover, line 1554-1584), `<skill>` skill popover (SkillMentionPopover, line 1585-1610).
   - **Source**: `ChatPanel.tsx` onInput handler, lines 1503-1610.
3. **Locale system**: Uses `t()` from `grill-me.ts`, lazy-loads `pt-BR.ts` or `en-US.ts` based on `localStorage["claudinio_locale"]`.
   - **Source**: `src/lib/grill-me.ts`, lines 1-100.

## Changes (Steps)

1. **Target**: `src/lib/locales/pt-BR.ts`
   - **Mutation**: Change `"chat.input.askCode"` value from `"Pergunte algo sobre o código…"` to `"Escreva algo… | @arquivo | <tag> | <skill>"`
   - **Why**: Teaches PT-BR users about @mentions, <tag> and <skill> features
   - **Constraints**: Keep the key name unchanged (`chat.input.askCode`) — no other file changes needed.

2. **Target**: `src/lib/locales/en-US.ts`
   - **Mutation**: Change `"chat.input.askCode"` value from `"Ask something about the code…"` to `"Write anything… | @file | <tag> | <skill>"`
   - **Why**: Teaches EN-US users about the same features
   - **Constraints**: Same key, same approach.

**No other files change** — `ChatPanel.tsx` already uses `t("chat.input.askCode")` for the placeholder, no code change needed.

## Risks

- **Low risk**: Only changes locale string values, no code or logic changes. If the placeholder is too long, it may truncate in narrow viewports. The pipe-separated format is compact and should fit. If truncation occurs on mobile, we can adjust by shortening the hint portion.

## Tasks summary

- **Task 1**: Update `chat.input.askCode` in `src/lib/locales/pt-BR.ts`
- **Task 2**: Update `chat.input.askCode` in `src/lib/locales/en-US.ts`
