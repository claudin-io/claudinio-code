# Plan: read_file inteligente com aviso de ficheiro grande

## 1. Context / Problem Statement

O `read_file` atual (`src-tauri/src/agent/tools/read_file.rs`) tem apenas um limite binário de 2MB. Um ficheiro de 1.5MB com ~50.000 linhas passa nesse limite mas causa estouro de contexto quando o conteúdo inteiro é injetado no prompt do LLM. Não há qualquer aviso, truncagem ou recomendação para usar `start_line`/`end_line`.

**O que o utilizador confirmou (via entrevista):**
- Comportamento: **Warning + conteúdo truncado** com nota informativa (não erro duro)
- Métrica: **Tokens** (contagem real com tokenizer)
- Threshold: **5000 tokens**
- Tokenizer: **tiktoken-rs** (encoding `cl100k_base`, compatível OpenAI)
- Nota deve informar: quantas linhas foram lidas vs. total de linhas do ficheiro
- O limite 2MB atual permanece como guarda binária (proteção contra binários)

## 2. Goal (Definition of Done)

Quando o `read_file` é chamado sem `start_line`/`end_line` e o ficheiro excede 5000 tokens no encoding `cl100k_base`:
1. O conteúdo é truncado a ~5000 tokens
2. A resposta inclui um cabeçalho de warning com: token count total, linhas exibidas, linhas totais do ficheiro, e sugestão para usar `start_line`/`end_line`
3. O limite 2MB continua ativo como erro duro

## 3. Key Findings (Prova Real)

- **F1:** `read_file.rs:execute()` — `read_file.rs:18` — Lê o conteúdo completo com `std::fs::read_to_string(p)` e retorna sem verificação de tamanho (além do 2MB).
- **F2:** `mod.rs:350-365` — O dispatch do `read_file` chama `read_file::execute(a)?` e regista no `ReadTracker`. Não há lógica de pós-processamento.
- **F3:** `Cargo.toml` — Já tem 90+ dependências incluindo `tokenizers = "0.23"`, mas **não** tem `tiktoken-rs`.
- **F4:** O `read_file` NÃO tem acesso ao `ToolContext` diretamente (a struct `ReadFileArgs` só tem `path`, `start_line`, `end_line`). Para injectar o tokenizer, ou passamos como parâmetro extra, ou fazemos a tokenização no `mod.rs` (dispatch) após o `execute()`.

## 4. Authoritative Inputs

| Input | Value | Source |
|-------|-------|--------|
| Token threshold | 5000 tokens | Per the user |
| Token encoding | `cl100k_base` (o200k_base se disponível) | Per the user (tiktoken-rs) |
| Crate a adicionar | `tiktoken-rs` | Per the user |
| Limite 2MB | Mantém-se como erro duro | Inferido (não foi pedido para remover) |

## 5. Changes (Steps)

### Step 1: Adicionar `tiktoken-rs` ao Cargo.toml
- **Target:** `src-tauri/Cargo.toml`
- **Mutation:** Adicionar `tiktoken-rs = "0.6"` (ou versão mais recente) nas dependências
- **Why:** Tokenização real compatível com OpenAI para contar tokens
- **Constraints:** Usar a versão estável mais recente disponível no crates.io

### Step 2: Criar lógica de truncagem + warning em `read_file.rs`
- **Target:** `src-tauri/src/agent/tools/read_file.rs`
- **Mutation:** 
  - Adicionar função `truncate_by_tokens(content: &str, max_tokens: usize) -> (String, usize, usize)` que:
    1. Conta total de tokens no conteúdo
    2. Se ≤ max_tokens, retorna conteúdo completo
    3. Se > max_tokens, itera linha a linha acumulando tokens até ao limite
    4. Retorna (conteúdo truncado, tokens exibidos, linhas exibidas)
  - Modificar `execute()` para:
    1. Após `read_to_string`, contar total de linhas (`content.lines().count()`)
    2. Se NÃO há range (`start_line`/`end_line` ambos None):
       - Chamar `truncate_by_tokens`
       - Se houve truncagem, prefixar conteúdo com warning
    3. Se HÁ range: manter comportamento atual (o utilizador já limitou o escopo)
  - Manter o guard 2MB existente intacto
- **Why:** Implementa o core da feature — aviso + truncagem
- **Constraints:** 
  - O tokenizer deve ser lazy-initialized (não queremos carregar o modelo em cada chamada)
  - Usar `std::sync::OnceLock` ou `lazy_static` para singleton do tokenizer BPE

### Step 3: Atualizar o dispatch no `mod.rs` (se necessário)
- **Target:** `src-tauri/src/agent/tools/mod.rs` (linhas 352-365)
- **Mutation:** Nenhuma — apenas verificar que a função `execute` do `read_file` é chamada corretamente
- **Why:** A tokenização é feita dentro de `read_file::execute()`, o dispatch não precisa mudar
- **Constraints:** No code change needed here — verify only

### Step 4: Testes
- **Target:** `src-tauri/src/agent/tools/mod.rs` (secção `tests`, a partir da linha 701)
- **Mutation:** Adicionar 2-3 testes:
  1. Ficheiro pequeno (< 5000 tokens) retorna conteúdo completo sem warning
  2. Ficheiro grande (> 5000 tokens) retorna conteúdo truncado com warning
  3. Ficheiro grande com range específico retorna apenas o range sem warning
- **Why:** Provar que a feature funciona e não quebra casos existentes

## 6. Verification Plan

1. **Cargo check:** `cargo check` passa sem erros (nova dependência compila)
2. **Testes existentes:** `cargo test` — todos os 17+ testes existentes continuam a passar
3. **Teste 1:** Ficheiro de 20 linhas → sem warning, conteúdo completo
4. **Teste 2:** Ficheiro de 5000+ linhas → warning presente, conteúdo truncado, mensagem informativa com contagens
5. **Teste 3:** Ficheiro grande + `start_line=1&end_line=20` → sem warning, apenas 20 linhas
6. **Regressão:** Limite 2MB continua a dar erro duro

## 7. Risks

- **Risco baixo:** `tiktoken-rs` pode ter problemas de compilação em algumas plataformas. Mitigação: é um crate maduro com builds pre-compiladas para as principais targets.
- **Risco baixo:** O tokenizer `cl100k_base` pode não ser o encoding exato do modelo em uso. Mitigação: é uma aproximação razoável; todos os modelos modernos usam BPE similar e a diferença de contagem é tipicamente < 5%.

## 8. Tasks Summary

| ID | Título | Descrição |
|----|--------|-----------|
| t1 | Adicionar tiktoken-rs ao Cargo.toml | Adicionar dependência `tiktoken-rs = "0.6"` |
| t2 | Implementar truncagem + warning no read_file.rs | Singleton do tokenizer + função truncate_by_tokens + warning format |
| t3 | Atualizar testes | Adicionar testes de ficheiro grande/pequeno/com range |
| t4 | Verificar regressões | Correr `cargo check && cargo test`, verificar limite 2MB intacto |
