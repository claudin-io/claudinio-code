# Auto-scroll no tooltip do ThinkingBar

## Context
Quando o usuário passa o mouse sobre o "Thinking…" (ThinkingBar), um tooltip aparece mostrando o pensamento atual do agente em tempo real via efeito typewriter. Atualmente o tooltip tem `overflow-y: auto`, mas **não tem auto-scroll** — quando um texto novo chega via streaming, o scroll permanece onde estava, e o usuário precisa rolar manualmente para ver o conteúdo mais recente.

## Solution Design

### Problema
O tooltip do ThinkingBar não rola automaticamente para o final quando o texto muda durante o typewriter/streaming.

### Fluxo de interação
1. Usuário põe o mouse sobre o "Thinking…" → tooltip aparece (`opacity: 1`)
2. Texto novo chega via `props.text()` (typewriter smoothing)
3. **Comportamento desejado:** tooltip auto-scrolla para o fundo para mostrar o texto mais recente
4. **Comportamento indesejado (atual):** o scroll fica parado, texto novo fica fora da viewport

### Decisões confirmadas com o usuário
| Decisão | Escolha |
|---|---|
| Quando auto-scrollar? | **Sempre que o texto mudar**, mesmo se o usuário estiver rolando para cima (force always). O tooltip é pequeno e o conteúdo é sequencial — não faz sentido pausar o scroll para leitura seletiva dentro de 50vh. |
| Quando ativar? | **Só quando o tooltip estiver visível** (hover ativo). Não scrolla escondido. |

### Comportamento detalhado
- Quando o texto muda via `props.text()` E o tooltip está visível → scroll para o fundo (`scrollTop = scrollHeight`)
- Quando o texto muda mas o tooltip NÃO está visível → nada acontece
- Quando o tooltip fica visível (hover) pela primeira vez → scrolla para o fundo para mostrar o conteúdo já acumulado
- Usuário nunca perde o thread: todo texto novo que chega enquanto ele está vendo o tooltip é mostrado automaticamente

## Risks
- **Nenhum.** A mudança é estritamente aditiva (auto-scroll via effect) e limitada ao componente `ThinkingBar`. Sem impactos colaterais.

## Non-goals
- **Não** alterar o comportamento de scroll da lista de mensagens principal (`scrollContainerRef` / `isAtBottom`)
- **Não** adicionar animação smooth ao scroll (será instantâneo)
- **Não** adicionar um botão "scroll to bottom" dentro do tooltip
- **Não** modificar lógica de CSS ou layout além do necessário para o auto-scroll

## Low-Level Design

### Arquivo-alvo
**`src/components/ChatPanel.tsx`** — linhas ~3079–3097 (componente `ThinkingBar`)

### Abordagem
Adicionar ao componente `ThinkingBar`:
1. Um **ref** (`tooltipRef`) no `<div class="thinking-bar-tooltip">`
2. Um **sinal `hovered()`** controlado por `onMouseEnter`/`onMouseLeave` no wrapper
3. Um **`createEffect`** que observa `props.text()` e `hovered()`, e quando ambos indicam que deve scrollar, executa `tooltipRef.scrollTop = tooltipRef.scrollHeight`

### Detalhamento

#### 1. Tooltip ref
```tsx
let tooltipRef: HTMLDivElement | undefined;
```
No JSX:
```tsx
<div ref={tooltipRef} class="thinking-bar-tooltip">
  {props.text() || ""}
</div>
```

#### 2. Hover state signal
```tsx
const [hovered, setHovered] = createSignal(false);
```
No wrapper:
```tsx
<div
  class="thinking-bar-wrapper"
  onMouseEnter={() => setHovered(true)}
  onMouseLeave={() => setHovered(false)}
>
```

#### 3. Auto-scroll effect
```tsx
createEffect(() => {
  const _text = props.text();   // subscribe to text changes
  const _hovered = hovered();   // subscribe to hover state
  if (_hovered && tooltipRef) {
    tooltipRef.scrollTop = tooltipRef.scrollHeight;
  }
});
```

### Funcionamento detalhado
- `createEffect` reage automaticamente a mudanças em qualquer sinal lido dentro dele. No caso, lê `props.text()` e `hovered()`.
- **Texto muda com hover ativo:** `_text` muda, `_hovered` é `true` → scrolla.
- **Texto muda sem hover:** `_hovered` é `false` → não scrolla.
- **Hover ativa:** `_hovered` muda para `true` → scrolla para o fundo (mostra o conteúdo já acumulado).
- **Hover desativa:** `_hovered` muda para `false` → não scrolla (mas o effect roda e não faz nada).

### Por que `scrollTop = scrollHeight` e não `scrollIntoView`?
- `scrollTop = scrollHeight` é a forma mais direta e barata de scrollar um elemento overflow para o final
- Não precisa de sentinel div
- Não há necessidade de animação smooth (instantâneo é melhor para ferramenta de dev)

### Integração com o código existente
- O componente `ThinkingBar` já está em `ChatPanel.tsx`. Não precisa criar novo arquivo.
- A assinatura do componente (`props: { text: () => string }`) não muda.
- Nenhum CSS precisa ser alterado (o `overflow-y: auto` já existe).

### Fluxo de dados
```
createSmoothText (liveThinkingText) → smoothThinking.displayed → props.text() → ThinkingBar
                                                                                    ↓
                                                                         createEffect detecta mudança
                                                                                    ↓
                                                                         tooltipRef.scrollTop = scrollHeight
```

### Tasks
1. **thinking-bar-add-ref-and-hover**: Adicionar `tooltipRef` e sinal `hovered()` ao `ThinkingBar`, juntamente com handlers `onMouseEnter`/`onMouseLeave` no wrapper e `ref={tooltipRef}` no tooltip div.
2. **thinking-bar-add-auto-scroll-effect**: Adicionar `createEffect` ao `ThinkingBar` que observa `props.text()` e `hovered()`, e scrolla o tooltip para o fundo quando ambos indicarem visibilidade.


## Implementation Log — 2026-07-15 05:19
**Summary:** Adicionar auto-scroll ao tooltip do ThinkingBar quando o texto muda via typewriter streaming"
**Changed files:** M src/components/ChatPanel.tsx, ?? docs/plans/2026-07-15_thinking-bar-auto-scroll.md
**Commits:** _(git unavailable or none)_
**Journal:** Mudança simples e direta no componente ThinkingBar em ChatPanel.tsx. Adicionamos:
1. `tooltipRef` (ref para o div do tooltip) e `hovered` signal (controlado por onMouseEnter/onMouseLeave).
2. `createEffect` que observa `props.text()` e `hovered()`, scrollando `tooltipRef.scrollTop = tooltipRef.scrollHeight` quando ambos indicam visibilidade.

Funcionamento:
- Texto muda + hover ativo → auto-scroll para o fundo instantaneamente
- Hover ativa pela primeira vez → scrolla para mostrar conteúdo já acumulado
- Texto muda sem hover → nada (economiza recursos)
- Nenhuma alteração em CSS, layout, ou lógica de scroll da lista principal

Build passou sem erros (vite build).

**Task journal:**
- Adicionar ref e hover state ao ThinkingBar: Adicionado: tooltipRef (let), hovered signal, onMouseEnter/onMouseLeave no wrapper, ref={tooltipRef} no tooltip div.
- Adicionar createEffect de auto-scroll no ThinkingBar: createEffect adicionado com sucesso. Lógica: observa props.text() e hovered(), scrolla tooltipRef.scrollTop = tooltipRef.scrollHeight quando hovered=true. Build passou sem erros.


## Implementation Log — 2026-07-15 06:00
**Summary:** Adiciona code signing Apple (Developer ID Application) + notarização automática no CI release do macOS
**Changed files:** M .github/workflows/release.yml
**Commits:** _(git unavailable or none)_
**Journal:** Implementação de code signing Apple (Developer ID Application) e notarização no CI release do macOS.

**Decisões tomadas:**
- Tipo de certificado: Developer ID Application (distribuição via DMG fora da App Store)
- Notarização ativada via envs APPLE_ID + APPLE_PASSWORD (suportada nativamente pelo tauri build CLI)
- Apenas o job macOS foi modificado — Windows e Linux intactos
- Usei env vars (APPLE_SIGNING_IDENTITY, APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID) em vez de tauri-action, para manter consistência com o `pnpm tauri build` atual

**Três blocos adicionados no workflow:**
1. **Import Apple Developer Certificate** — decodifica o .p12 do secret, cria keychain temporário, importa o certificado, configura permissões de codesign
2. **Resolve Apple Signing Identity** — extrai o CN do certificado Developer ID Application importado e expõe como APPLE_SIGNING_IDENTITY no GITHUB_ENV. Tem fallback caso o nome exato não seja encontrado
3. **Build Tauri app** — recebe os novos envs: APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID, APPLE_SIGNING_IDENTITY

**Secrets necessários (já configurados pelo usuário):**
- APPLE_ID — email da Apple ID
- APPLE_PASSWORD — app-specific password
- APPLE_CERTIFICATE — .p12 em base64
- APPLE_CERTIFICATE_PASSWORD — senha do .p12
- KEYCHAIN_PASSWORD — senha temporária do keychain no CI
- APPLE_TEAM_ID — team ID de 10 caracteres

**Verificação:**
- YAML validado com python3 yaml.safe_load — sem erros sintáticos
- Documentação oficial do Tauri v2 confirmada: os env vars são suportados nativamente pelo `tauri build` CLI

**Task journal:**
- Adicionar code signing Apple no release.yml: Adicionado step 'Import Apple Developer Certificate' (if: runner.os == 'macOS') que decodifica o .p12, cria keychain e importa o certificado; Adicionado step 'Resolve Apple Signing Identity' que extrai o CN do certificado Developer ID Application e expõe como APPLE_SIGNING_IDENTITY no GITHUB_ENV; Adicionados APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID e APPLE_SIGNING_IDENTITY como envs do 'Build Tauri app' step; Verificado com a documentação oficial do Tauri v2 que os env vars são suportados nativamente pelo tauri build CLI — nenhuma dependência extra necessária
- Verificar integridade do workflow: YAML validado com python3 yaml.safe_load — sem erros; Estrutura de steps confirmada: 12 steps no total, sendo steps 8-10 os novos (import, resolve, build com signing)
