# Support Icon in Settings

## Context
The user wants to add a support link inside the settings modal. The link should open `https://claudin.io/dashboard#account` (which contains a support form). The icon should use the user-provided SVG pixelart speech bubble. The link should be placed in the Account section, below the sign in/sign out area.

## Solution Design
- **Icon**: Add a new `"speech-balloon-alt"` icon entry to `PATHS` in `src/components/Icon.tsx` using the user-provided SVG path data. The path is: `M20 2H2v20h2V4h16v12H6v2H4v2h2v-2h16V2z`
- **Translation keys**: Add `"app.config.support"` in both locale files:
  - en-US: `"Support"`
  - pt-BR: `"Suporte"`
- **UI placement**: Inside settings modal, after the Account `<Show>` block (the sign in/sign out section) and before the Easter egg section, insert a support button that:
  - Uses the new `speech-balloon-alt` icon with `Icon` component
  - Shows the translated label
  - On click, calls `openExternalUrl("https://claudin.io/dashboard#account")`
  - Styled consistently with the existing settings: rounded border, subtle hover, etc.

## Risks
- Very low risk — purely additive UI change, no data flow or state mutation
- The `openExternalUrl` function already exists and is imported in `src/App.tsx` (line 5)

## Non-goals
- No changes to the support page itself — just linking to the existing URL
- No changes to the Tauri backend
- No changes to any other component

## Low-Level Design

### Files to modify

**1. `src/components/Icon.tsx`** — add a new icon entry
- Add `"speech-balloon-alt"` to the `PATHS` record with the path array: `["M20 2H2v20h2V4h16v12H6v2H4v2h2v-2h16V2z"]`
- No viewBox or stroke overrides needed (uses default 24×24, no stroke)

**2. `src/lib/locales/en-US.ts`** — add translation string
- Add `"app.config.support": "Support"` in the App config section

**3. `src/lib/locales/pt-BR.ts`** — add translation string
- Add `"app.config.support": "Suporte"` in the App config section

**4. `src/App.tsx`** — add the support button in the settings modal
- The `openExternalUrl` is already imported on line 5
- After the closing `</Show>` of the Account section (around line 727), insert:
  ```
  <!-- Support link -->
  <div class="mb-3">
    <button
      onClick={() => openExternalUrl("https://claudin.io/dashboard#account")}
      class="flex items-center gap-2 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink hover:bg-surface-2 hover:border-accent/40 transition-colors"
    >
      <Icon name="speech-balloon-alt" class="h-4 w-4 shrink-0" />
      <span>{t("app.config.support")}</span>
    </button>
  </div>
  ```

### Data flow
- Click → `openExternalUrl("https://claudin.io/dashboard#account")` → Tauri plugin `opener` → opens in default browser
- No state, no persistence, no side effects

## Tasks summary
1. Add `"speech-balloon-alt"` icon path to `src/components/Icon.tsx`
2. Add `"app.config.support"` translation to `src/lib/locales/en-US.ts` and `src/lib/locales/pt-BR.ts`
3. Add support button to settings modal in `src/App.tsx` after the Account section


## Implementation Log — 2026-07-15 18:42
**Summary:** Adicionado ícone de suporte (speech-balloon-alt) no settings modal que abre claudin.io/dashboard#account
**Changed files:** M src/App.tsx, M src/components/Icon.tsx, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-15_support-icon-settings.md
**Commits:** _(git unavailable or none)_
**Journal:** - Adicionado ícone `speech-balloon-alt` em Icon.tsx com o path do SVG fornecido pelo usuário (Pixelarticons, Gerrit Halfmann)
- openExternalUrl não estava importado em App.tsx — adicionado ao import junto com os demais
- Botão posicionado na seção Account do modal de configurações, logo após o bloco de sign in/sign out
- Estilo consistente com o resto do modal: border, bg-surface-0, hover:bg-surface-2, hover:border-accent/40
- Build verificado com sucesso (vite build)

**Task journal:**
- Add speech-balloon-alt icon to Icon.tsx: Added speech-balloon-alt icon after the existing speech-balloon entry in PATHS. Uses Pixelarticons license (Gerrit Halfmann), default 24×24 viewBox, fill mode.
- Add support translation keys (en-US / pt-BR): Added key after app.config.theme in both files
- Add support button in settings modal at Account section: openExternalUrl estava ausente no import de App.tsx — adicionado à import list; Botão inserido entre o fechamento do <Show> da Account e o comentário do Easter egg; Build verificado: npx vite build → ✓ built in 15.43s


## Implementation Log — 2026-07-15 19:58
**Summary:** ✅ v0.1.9 deploy verified — already released on claudin-io/claudinio-code-releases
**Changed files:** _(none detected)_
**Commits:** _(git unavailable or none)_
**Journal:** v0.1.9 release was already fully deployed and published. No action needed — the release exists with all 24 artifacts across 5 platform targets (macOS-arm64, Linux-x64, Linux-arm64, Windows-x64, Windows-arm64), is published (not draft), and the workflow completed successfully.

**Task journal:**
- deploy da versão v0.1.9, o goal é garantir que o deploy foi released no claudinio-code-releases: Tag v0.1.9 already exists locally and on remote origin; Release already published on claudin-io/claudinio-code-releases; 24 assets across all 5 platform targets: macOS-arm64, Linux-x64, Linux-arm64, Windows-x64, Windows-arm64; Published at 2026-07-15T12:46:09Z, not a draft, not a prerelease; GitHub Actions workflow completed successfully
- deploy da versão v0.1.9, o goal é garantir que o deploy foi released no claudinio-code-releases: Verified via GitHub API: release exists and is published (not draft); All expected platform binaries exist in the release assets; Workflow completed successfully
