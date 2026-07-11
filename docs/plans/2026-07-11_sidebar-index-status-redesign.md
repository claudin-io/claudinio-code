# Sidebar Index Status Panel — Redesign

## Context / Problem

O painel de status do índice na sidebar (`App.tsx` ~L954-959) mostra uma única linha de texto monoespaçado com as contagens de files e symbols:

```
10 arquivos, 150 símbolos
```

O usuário quer **3 linhas empilhadas com ícones** (file, layers, brain) — uma para cada métrica — mantendo o estilo minimalista atual (`font-mono text-[10px] text-ink-faint`). A contagem de embeddings não é exposta hoje (nem no backend, nem no frontend).

## Solution Design

### Layout final

Três linhas empilhadas no mesmo container `border-t border-border-subtle px-3 py-2`:

```
📄  10 arquivos
📑  150 símbolos
🧠  95 embeddings
```

Cada linha: `<Icon name="file">` + espaço + label localizado com número.

### Mudanças

#### 1. Backend: `src-tauri/src/code_intel/db.rs` — `index_stats()`

- Mudar de `Result<(i64, i64), String>` para `Result<(i64, i64, i64), String>`
- Adicionar query: `SELECT count(*) FROM symbol_embeddings`

#### 2. Backend: `src-tauri/src/commands/code_intel.rs` — `IndexStatus` struct

- Adicionar campo `pub embeddings_count: i64`

#### 3. Backend: `src-tauri/src/commands/code_intel.rs` — `open_workspace()`

- Desestruturar 3 valores de `index_stats()`:
  ```rust
  let (files_count, symbols_count, embeddings_count) = ws.index_db.index_stats().unwrap_or((0, 0, 0));
  ```
- Incluir `embeddings_count` na construção do `IndexStatus` (tanto no caminho "already open" quanto no "new workspace")

#### 4. Frontend: `src/lib/ipc.ts` — `IndexStatus` interface

- Adicionar campo `embeddingsCount: number`

#### 5. Frontend: `src/App.tsx` — refatorar `indexStatusMap` + renderização

- **`indexStatusMap`**: mudar de `Record<string, string>` para `Record<string, IndexStatus | null>`
- **`indexStatus()`**: retornar o objeto `IndexStatus | null` ao invés de string
- **`setWsIndexStatus` / `setIndexStatusMap`**: adaptar para receber o objeto
- **No `indexProject()`**: passar o `IndexStatus` completo para `setWsIndexStatus`:
  ```ts
  setWsIndexStatus(folder, s); // em vez de formatar string aqui
  ```
- **Renderização** (L954-959): substituir o `{indexStatus()}` por 3 linhas com `<Icon>`:
  ```tsx
  <div class="flex flex-col gap-0.5">
    <div class="flex items-center gap-1 font-mono text-[10px] text-ink-faint">
      <Icon name="file" class="w-3 h-3" />
      <span>{t("app.index.filesLabel", indexStatus()!.filesCount)}</span>
    </div>
    <div class="flex items-center gap-1 font-mono text-[10px] text-ink-faint">
      <Icon name="layers" class="w-3 h-3" />
      <span>{t("app.index.symbolsLabel", indexStatus()!.symbolsCount)}</span>
    </div>
    <div class="flex items-center gap-1 font-mono text-[10px] text-ink-faint">
      <Icon name="brain" class="w-3 h-3" />
      <span>{t("app.index.embeddingsLabel", indexStatus()!.embeddingsCount)}</span>
    </div>
  </div>
  ```
- **Watcher warning**: se `watcherWarning` presente, mostrar como um alerta extra (4ª linha com ícone `alert-triangle`)

#### 6. Locales: `src/lib/locales/en-US.ts` e `pt-BR.ts`

- Adicionar 3 novas chaves:
  - `"app.index.filesLabel": "{0} files"` / `"{0} arquivos"`
  - `"app.index.symbolsLabel": "{0} symbols"` / `"{0} símbolos"`
  - `"app.index.embeddingsLabel": "{0} embeddings"` / `"{0} embeddings"`
- Manter `"app.index.filesCount"` (ainda pode ser usado em outros lugares)

#### 7. Testes: `src/lib/ipc.test.ts`

- Atualizar os objetos fake `IndexStatus` para incluir `embeddingsCount`

### O que NÃO muda

- O container pai (`border-t border-border-subtle px-3 py-2`) e a lógica de visibilidade (`Show when`)
- A barra de progresso (indexação em andamento) — os Match/Switch permanecem iguais
- O `IndexProgress` interface — continua sem embeddings count (não é necessário no progresso)

## Risks

- **Baixo**: `index_stats()` é `#[allow(dead_code)]` — verificar se é usado em outros lugares antes de mudar a assinatura
- **Baixo**: a mudança de `indexStatusMap` de `string` para `IndexStatus | null` pode quebrar referências existentes — revisar todos os usos de `setWsIndexStatus` e `indexStatusMap`
- **Nenhum**: a tabela `symbol_embeddings` já existe no schema, então a query é segura

## Verification

1. `cargo build` no `src-tauri` — compila sem erros
2. `pnpm test` — todos os testes passam
3. `pnpm tsc --noEmit` — sem erros de tipo
4. Abrir o app, carregar um workspace → ver 3 linhas com ícones na sidebar


## Implementation Log — 2026-07-11 00:39
**Summary:** Sidebar index status panel redesigned: 3 lines with icons (file, layers, brain) for Files, Symbols, and Embeddings counts
**Changed files:** M src-tauri/examples/semantic_eval_queries.json, M src-tauri/src/code_intel/db.rs, M src-tauri/src/code_intel/embeddings.rs, M src-tauri/src/code_intel/indexer.rs, M src-tauri/src/commands/code_intel.rs, M src/App.tsx, M src/lib/ipc.test.ts, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-09_deploy-tag-0-1-1.md, ?? docs/plans/2026-07-10_steering-attachments.md, ?? docs/plans/2026-07-11_sidebar-index-status-redesign.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Key decisions

1. **Backend**: `index_stats()` agora retorna `(i64, i64, i64)` — files, symbols, embeddings. Query `SELECT count(*) FROM symbol_embeddings` adicionada.

2. **IndexStatus struct**: campo `embeddings_count` serializado como `embeddingsCount` (camelCase). Os dois caminhos em `open_workspace()` (workspace já aberto e novo) agora incluem o valor.

3. **Frontend state**: `indexStatusMap` mudou de `Record<string, string>` para `Record<string, IndexStatus | string | null>`. String preservada para mensagens transitórias (erros, "indexing...") enquanto o workspace abre, e `IndexStatus` objeto armazenado quando o workspace está pronto.

4. **Listener de embeddings_done**: atualiza o `embeddingsCount` no objeto `IndexStatus` preservado quando a fase de embedding termina.

5. **Renderização**: 3 linhas empilhadas com `<Icon name="file/layers/brain">` + label localizado. Watcher warning aparece como 4ª linha opcional com ícone `alert-triangle` em amarelo.

## Gotchas

- O `Show` condition `activeWorkspace() && !showTree() && (progress() || indexStatus())` funciona porque `indexStatus()` retorna `null` quando não há status, e `null` é falsy. Objetos IndexStatus são truthy então passam.
- Precisamos fazer type assertion `(indexStatus() as IndexStatus)` no JSX porque o tipo é `IndexStatus | string | null`. Um `as` é aceitável aqui porque o `Show` garante que só chegamos na renderização objeto quando o valor não é string.

**Task journal:**
- Backend: index_stats() — adicionar embeddings_count: Adicionado `SELECT count(*) FROM symbol_embeddings` em `db.rs:index_stats()`. Assinatura mudou para `Result<(i64,i64,i64), String>`.
- Backend: IndexStatus struct — adicionar embeddings_count: Adicionado campo `embeddings_count: i64` ao struct IndexStatus em commands/code_intel.rs.
- Backend: open_workspace() — consumir embeddings_count: Desestruturado `(files_count, symbols_count, embeddings_count)` no already-open e new path. No new path, query adicional após scan para pegar embeddings persistidos de sessão anterior.
- Frontend: App.tsx — refatorar indexStatusMap e indexProject: indexStatusMap agora aceita IndexStatus | string | null. `indexProject()` e listener de eventos atualizados para armazenar objeto IndexStatus completo.
- Frontend: App.tsx — renderizar 3 linhas com ícones: Renderização substituída. Fallback para string preservado para mensagens de erro/indexing.
- Locales: adicionar novas chaves de label: en-US: '{0} files', '{0} symbols', '{0} embeddings'. pt-BR: '{0} arquivos', '{0} símbolos', '{0} embeddings'.
- Testes: ipc.test.ts — adicionar embeddingsCount: Fakes atualizados em ipc.test.ts: openWorkspace e mockInvokeFor.
- Verificação: build + testes + typecheck: 384/384 testes passam. Rust build compila. TypeScript check mostra apenas erros pré-existentes (vi globals, etc).
