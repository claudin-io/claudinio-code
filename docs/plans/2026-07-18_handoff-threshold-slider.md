# Session Handoff Threshold Slider — Match Max Parallel Agents Pattern

## Context

O slider de **Session Handoff Threshold** em `App.tsx` tem um visual inconsistente com o resto das settings. Enquanto o slider de **Max Parallel Agents** tem track grossa, handle customizado, labels nas pontas, badge de workspace/local, e estado disabled, o slider de handoff threshold está "pelado": sem labels, sem badge, dentro de um grid 2-colunas, e com classes Tailwind faltando (`appearance-none`, `h-2`, `rounded-lg`, `cursor-pointer`).

Além de igualar o padrão visual, o usuário quer **educar** sobre o risco de "context rot": labels `120k` / `256k` nas pontas e um gradiente verde→vermelho na track indicando que à esquerda (handoff mais cedo) é mais seguro e à direita (handoff mais tarde) há maior risco de degradação do contexto.

**Imagem de referência (o que está quebrado):** slider de Handoff Threshold com track fina cinza e handle branco circular, sem labels nas pontas, dentro de grid 2-colunas.

**Imagem de referência (target):** slider de Max Parallel Agents com track grossa, handle marrom com borda, labels "slower" / "faster" nas pontas, badge de workspace, hint text abaixo.

## Solution Design

### O que muda

1. **Extrair o Session Handoff Threshold do grid 2-colunas** e torná-lo standalone full-width, idêntico ao layout do Max Parallel Agents.
2. **Labels nas pontas**: esquerda = "120k", direita = "256k" (valores min/max do range).
3. **Gradiente verde→vermelho na track**: substitui o fundo `--border-subtle` por `linear-gradient(to right, #22c55e, #ef4444)` via CSS scoped, indicando risco de context rot.
4. **Barra de risco abaixo do slider**: substitui o hint text atual por uma barra fina com gradiente verde→vermelho + labels "lower risk" / "higher risk".
5. **Badge workspace/local**: igual aos outros campos, `handoff_context_tokens` é workspace-configurable (confirmado em `provider.rs:374-375`).
6. **Disabled state**: quando sobrescrito pelo workspace, slider fica disabled com opacidade reduzida.
7. **Classes Tailwind**: `h-2 rounded-lg appearance-none cursor-pointer` (sem `accent-accent` pois a track tem gradiente customizado).
8. **Formato do valor**: manter `Nk tokens` inline no label.

### Layout final

```
[Session handoff threshold    120k tokens]    [Workspace] / [Local]
 120k  [=====slider com gradiente verde→vermelho=====]  256k

[ ████████████████████████████████████████████ ]  ← barra fina de gradiente
lower risk                              higher risk
```

### Locale keys

| Key | en-US | pt-BR |
|-----|-------|-------|
| `app.config.lowerRisk` | lower risk | menor risco |
| `app.config.higherRisk` | higher risk | maior risco |

Keys **removidas** (substituídas pela barra de risco):
- `settings.handoffThresholdHint` (en-US: "Context size at which the session writes a handoff document...")
- `settings.handoffThresholdHint` (pt-BR: "Tamanho de contexto em que a sessão escreve um documento de handoff...")

### Não muda

- Range: 120000 a 256000, step 8000
- CSS global para `input[type="range"]` (thumb, disabled) continua igual — só adicionamos regra scoped para o gradiente
- Max Golden Stalls continua no grid 2-colunas
- Config signal `configHandoffTokens` e save/load — sem alteração
- Backend: sem alteração

## Risks

- **Baixo**: mudança puramente cosmética no frontend. CSS scoped via classe `.handoff-slider` não afeta outros sliders.
- O gradiente na track usa `background: linear-gradient(...)` que funciona bem em WebKit e Firefox via pseudo-elementos.

## Non-goals

- Não alterar range, step, ou comportamento do backend
- Não alterar Max Golden Stalls
- Não criar componente Slider reutilizável
- Não alterar o CSS global de `input[type="range"]` (só adicionar regra scoped)

---

## Low-Level Design

### Arquivos a modificar

| Arquivo | Tipo de mudança |
|---------|-----------------|
| `src/App.tsx` (~linhas 1086–1103) | Reestruturar o bloco do slider handoff |
| `src/App.css` (~linha 1098) | Adicionar regras CSS scoped para gradiente |
| `src/lib/locales/en-US.ts` (~linhas 362) | Adicionar `lowerRisk`/`higherRisk`, remover `handoffThresholdHint` |
| `src/lib/locales/pt-BR.ts` (~linhas 362) | Adicionar `lowerRisk`/`higherRisk`, remover `handoffThresholdHint` |

### Detalhamento

#### 1. `src/App.tsx` — Bloco atual (linhas ~1086–1103)

```tsx
// ATUAL — dentro do grid 2-colunas, sem labels, sem badge
<div>
<label class="mb-1 block text-xs text-ink-muted">
  {t("settings.handoffThreshold")}
  <span class="ml-2 font-mono text-[11px] text-ink-faint">{Math.round(configHandoffTokens() / 1000)}k tokens</span>
</label>
<input
  type="range"
  min="120000"
  max="256000"
  step="8000"
  value={configHandoffTokens()}
  onInput={(e) => setConfigHandoffTokens(parseInt(e.currentTarget.value, 10))}
  class="mb-1 w-full accent-accent"
/>
<p class="mb-0 text-[11px] text-ink-faint">{t("settings.handoffThresholdHint")}</p>
</div>
```

**Novo bloco — standalone full-width, idêntico ao padrão Max Parallel Agents:**

```tsx
{/* Session Handoff Threshold slider — full width */}
<div class="mb-4">
  <div class="flex items-center gap-2 mb-1">
    <label class="block text-xs text-ink-muted">
      {t("settings.handoffThreshold")}
      <span class="ml-2 font-mono text-[11px] text-ink-faint">{Math.round(configHandoffTokens() / 1000)}k tokens</span>
    </label>
    <Show when={workspaceConfigFields().has("handoff_context_tokens")}>
      <span class="rounded border border-accent/40 bg-accent/10 px-1.5 py-px text-[10px] font-medium text-accent">{t("app.config.sourceWorkspace")}</span>
    </Show>
    <Show when={!workspaceConfigFields().has("handoff_context_tokens")}>
      <span class="rounded border border-border-subtle bg-surface-2 px-1.5 py-px text-[10px] text-ink-faint">{t("app.config.sourceLocal")}</span>
    </Show>
  </div>

  <div class="flex items-center gap-2">
    <span class="text-[10px] text-ink-faint w-10 text-right">120k</span>
    <input
      type="range"
      min="120000"
      max="256000"
      step="8000"
      value={configHandoffTokens()}
      onInput={(e) => setConfigHandoffTokens(parseInt(e.currentTarget.value, 10))}
      disabled={workspaceConfigFields().has("handoff_context_tokens")}
      class="flex-1 h-2 rounded-lg appearance-none cursor-pointer handoff-slider"
      classList={{
        "opacity-50 cursor-not-allowed": workspaceConfigFields().has("handoff_context_tokens"),
      }}
    />
    <span class="text-[10px] text-ink-faint w-10">256k</span>
  </div>

  {/* Context rot risk indicator bar */}
  <div class="mt-1 flex items-center gap-2">
    <span class="text-[10px] text-ink-faint">{t("app.config.lowerRisk")}</span>
    <div class="flex-1 h-1 rounded-full handoff-risk-bar"></div>
    <span class="text-[10px] text-ink-faint">{t("app.config.higherRisk")}</span>
  </div>
</div>
```

**Notas sobre o bloco:**
- `handoff-slider` é a classe CSS scoped para o gradiente na track
- `handoff-risk-bar` é a classe CSS para a barra de gradiente fina abaixo
- `workspaceConfigFields().has("handoff_context_tokens")` — o backend envia esse campo no `workspaceConfig` via `getConfig` (confirmado: `provider.rs:374-375` e `agent.rs:797`)

#### 2. `src/App.css` — Adicionar após linha ~1098

```css
/* ── Handoff threshold slider — green→red gradient track ── */
input.handoff-slider::-webkit-slider-runnable-track {
  background: linear-gradient(to right, #22c55e 0%, #fbbf24 50%, #ef4444 100%);
}
input.handoff-slider::-moz-range-track {
  background: linear-gradient(to right, #22c55e 0%, #fbbf24 50%, #ef4444 100%);
}

/* Context rot risk indicator bar */
.handoff-risk-bar {
  background: linear-gradient(to right, #22c55e 0%, #fbbf24 50%, #ef4444 100%);
}
```

**Nota:** O gradiente usa 3 stops: verde → amarelo → vermelho, igual ao pedido mas com amarelo no meio para transição mais suave.

#### 3. `src/lib/locales/en-US.ts` — Mudanças

**Remover linha:**
```ts
"settings.handoffThresholdHint": "Context size at which the session writes a handoff document and continues in a fresh linked session.",
```

**Adicionar (próximo de `app.config.slower`/`faster`):**
```ts
"app.config.lowerRisk": "lower risk",
"app.config.higherRisk": "higher risk",
```

#### 4. `src/lib/locales/pt-BR.ts` — Mudanças

**Remover linha:**
```ts
"settings.handoffThresholdHint": "Tamanho de contexto em que a sessão escreve um documento de handoff e continua em uma nova sessão encadeada.",
```

**Adicionar:**
```ts
"app.config.lowerRisk": "menor risco",
"app.config.higherRisk": "maior risco",
```

### Data flow

```
getConfig() → workspaceConfig.handoff_context_tokens → workspaceConfigFields set
                                                              ↓
                                          workspaceConfigFields().has("handoff_context_tokens")
                                                              ↓
                                          ┌───────────────────┴───────────────────┐
                                          ↓ (true)                                ↓ (false)
                                    badge: "Workspace"                      badge: "Local"
                                    slider: disabled                        slider: enabled
                                    opacity-50                              normal opacity
```

### CSS specificity

As regras `input.handoff-slider::-webkit-slider-runnable-track` têm maior especificidade que `input[type="range"]::-webkit-slider-runnable-track` (classe + pseudo-elemento vs. tipo + pseudo-elemento), então sobrescrevem corretamente apenas o slider de handoff. O slider de Max Parallel Agents continua usando `--border-subtle` como fundo da track.

**Verificação de especificidade:**
- Global: `input[type="range"]::-webkit-slider-runnable-track` → (0, 0, 1, 2) — 1 type + 1 pseudo-element
- Scoped: `input.handoff-slider::-webkit-slider-runnable-track` → (0, 0, 2, 1) — 1 type + 1 class + 1 pseudo-element ✓ Maior

### Wiring checklist

- [ ] CSS class `handoff-slider` aplicada ao `<input>` em App.tsx
- [ ] CSS rules `input.handoff-slider::-webkit-slider-runnable-track` e `::-moz-range-track` em App.css
- [ ] CSS class `handoff-risk-bar` no `<div>` da barra de risco
- [ ] CSS rule `.handoff-risk-bar` em App.css
- [ ] Locale keys `app.config.lowerRisk` / `app.config.higherRisk` em ambos os arquivos
- [ ] Chave `settings.handoffThresholdHint` **removida** de ambos os locales (não mais referenciada)
- [ ] Badge `workspaceConfigFields().has("handoff_context_tokens")` — mesmo pattern dos outros campos
- [ ] Disabled state com `classList` condicional
- [ ] Remover `<div>` vazio ou ajustar grid se necessário (o handoff estava no segundo `<div>` de um grid 2-colunas com Max Golden Stalls)

---

## Tasks

### T1: Add locale keys and remove old hint key
**Files:** `src/lib/locales/en-US.ts`, `src/lib/locales/pt-BR.ts`
- Add `app.config.lowerRisk` and `app.config.higherRisk` to both files
- Remove `settings.handoffThresholdHint` from both files
- Place new keys near `app.config.slower`/`faster`

### T2: Add scoped CSS for handoff gradient track and risk bar
**File:** `src/App.css`
- Add `input.handoff-slider::-webkit-slider-runnable-track` with green→amber→red gradient
- Add `input.handoff-slider::-moz-range-track` with same gradient
- Add `.handoff-risk-bar` class with same gradient

### T3: Restructure handoff threshold slider in App.tsx
**File:** `src/App.tsx`, around lines 1086–1103
- Extract from 2-column grid, make standalone full-width (match Max Parallel Agents layout)
- Add `120k` / `256k` end labels
- Add workspace/local badge using `workspaceConfigFields().has("handoff_context_tokens")`
- Add `disabled` + `classList` for disabled state
- Update Tailwind classes: `flex-1 h-2 rounded-lg appearance-none cursor-pointer handoff-slider` (remove `accent-accent`)
- Replace `<p>` hint text with gradient risk bar + "lower risk" / "higher risk" labels
- Ensure grid layout stays correct (Max Golden Stalls should still be in the 2-col grid)

### T4: Verify visual consistency
- Run dev server and visually compare both sliders
- Verify gradient renders on handoff track
- Verify risk bar renders below slider
- Verify disabled state when workspace config overrides
- Verify badges show correctly
- Verify both locales (en-US / pt-BR)


## Implementation Log — 2026-07-18 12:09
**Summary:** Restructured Session Handoff Threshold slider to match Max Parallel Agents pattern with green→red gradient track, risk bar, end labels, workspace/local badge, and disabled state
**Changed files:** M src/App.css, M src/App.tsx, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-18_handoff-threshold-slider.md
**Commits:** _(git unavailable or none)_
**Journal:** All 3 tasks implemented cleanly with no issues.

Key decisions during implementation:
- Trailing commas on all locale entries (project convention)
- Gradient uses 3 stops (green #22c55e → amber #fbbf24 → red #ef4444) for smoother transition — matches the plan's green→red intent but with visual midpoint
- CSS scoped via class selector (.handoff-slider) has higher specificity than global input[type=range], so no interference with Max Parallel Agents slider
- Grid now holds exactly 4 items (maxRounds, subMaxRounds, maxGoldenCycles, maxGoldenStalls) in clean 2x2
- New block inserted between grid </div> and <hr>, standalone full-width, matching Max Parallel Agents layout pattern

Verified:
- handoffThresholdHint removed from both locales (grep returns 0 hits)
- All new keys/locales present and correctly placed
- All CSS rules present at correct positions
- App.tsx slider restructured with badges, labels, disabled state, risk bar
- TypeScript build: 0 errors attributable to our changes (only pre-existing test errors)

**Task journal:**
- Add locale keys for lower/higher risk, remove old hint: Added lowerRisk/higherRisk at lines 28-29 in both en-US.ts and pt-BR.ts; Removed settings.handoffThresholdHint from both locale files; Comma style matches existing convention (trailing commas on all entries)
- Add scoped CSS for handoff gradient track and risk bar: Added 3 CSS rules at lines 1101-1111 in App.css after disabled range rules; handoff-slider webkit track, handoff-slider moz track, and handoff-risk-bar all with green→amber→red gradient
- Restructure handoff threshold slider in App.tsx: Grid now has exactly 4 child divs (maxRounds, subMaxRounds, maxGoldenCycles, maxGoldenStalls) in clean 2x2; New handoff slider block placed between grid closing </div> and <hr>; No residue of handoffThresholdHint <p> tag; Workspace/local badge, 120k/256k labels, disabled state, risk bar all wired up


## Implementation Log — 2026-07-18 12:18
**Summary:** Replaced colored risk bar with neutral border-t line and centered "Context Rot Risk" label; added locale keys; removed .handoff-risk-bar CSS
**Changed files:** M src/App.css, M src/App.tsx, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-18_handoff-threshold-slider.md
**Commits:** _(git unavailable or none)_
**Journal:** Revision per user feedback: replaced the second colored gradient bar (risk indicator) with a neutral separator line + centered "Context Rot Risk" label. 

Changes:
- App.tsx: replaced the handoff-risk-bar div with two border-t border-border-subtle lines flanking a centered whitespace-nowrap label using t("app.config.contextRotRisk")
- App.css: removed .handoff-risk-bar CSS rule entirely (no longer needed)
- Locales: added app.config.contextRotRisk = "Context Rot Risk" (en-US) / "Risco de Context Rot" (pt-BR)

Verified:
- grep handoff-risk-bar across src/ returns zero hits
- All locale keys present in both files
- Gradient track on slider preserved

**Task journal:**
- Add locale keys for lower/higher risk, remove old hint: Added lowerRisk/higherRisk at lines 28-29 in both en-US.ts and pt-BR.ts; Removed settings.handoffThresholdHint from both locale files; Comma style matches existing convention (trailing commas on all entries)
- Add scoped CSS for handoff gradient track and risk bar: Added 3 CSS rules at lines 1101-1111 in App.css after disabled range rules; handoff-slider webkit track, handoff-slider moz track, and handoff-risk-bar all with green→amber→red gradient
- Restructure handoff threshold slider in App.tsx: Grid now has exactly 4 child divs (maxRounds, subMaxRounds, maxGoldenCycles, maxGoldenStalls) in clean 2x2; New handoff slider block placed between grid closing </div> and <hr>; No residue of handoffThresholdHint <p> tag; Workspace/local badge, 120k/256k labels, disabled state, risk bar all wired up
