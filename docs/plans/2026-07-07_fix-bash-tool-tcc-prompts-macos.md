# Plan: Eliminar prompts TCC do macOS no bash tool

## Contexto / Problema

Toda vez que o bash tool executa um comando que precisa de `node`, `tsc`, `pnpm`, etc., o LLM (Claudinio) está prepending `export PATH="/Users/victortavernari/.nvm/versions/node/v22.16.0/bin:$PATH"` ao comando. No macOS, isso faz com que o TCC (Transparency, Consent, and Control) pergunte repetidamente se o app pode acessar Downloads, Desktop, Documentos, fotos, Apple Music etc.

### Por que isso acontece?

1. **O código Rust (`bash.rs`) NÃO adiciona PATH nenhum** — ele faz `Command::new("sh").arg("-c").arg(&args.command)` e pronto.
2. **O LLM aprendeu a prepender `export PATH=...nvm...`** porque sem isso, `sh` não encontra `node`, `tsc`, `pnpm` — eles só existem no PATH do `zsh` interativo via `.zshrc`/`.nvmrc`.
3. **O macOS TCC detecta o SHELL filho** — toda vez que um novo processo `sh` roda com um `$PATH` que contém diretórios protegidos (Desktop, Downloads, Documents, etc.), o sistema pede autorização.
4. Isso escala porque **cada bash tool call** cria um processo novo, e o prompt TCC aparece toda vez.

## Solução Proposta

Modificar o **Rust backend** (`bash.rs`) para **injetar automaticamente o PATH do NVM** em todo comando bash, de forma que:
- O LLM **não precise mais** prepender `export PATH=...` manualmente
- O novo processo `sh` já comece com o PATH correto
- O TCC pare de perguntar

### Abordagem 1 (Recomendada): PATH hardcoded no backend Rust

Modificar `bash.rs` para resolver o PATH do nvm uma vez (na inicialização ou na primeira execução) e prependê-lo a todo comando, similar ao que o LLM faz hoje, mas server-side.

**Problema com essa abordagem**: O PATH ainda vai conter referências a diretórios TCC-protegidos se o `$PATH` original do usuário incluir `~/Downloads`, etc. A raiz do problema não é quem adiciona o export, mas SIM o conteúdo do PATH.

### Abordagem 2 (Correta): Set env vars no Rust via `.env()` do Command

Em vez de modificar o comando string, usar o método `.env()` do `tokio::process::Command` para definir um PATH limpo que só contém os diretórios necessários.

```rust
let path = format!(
    "/Users/victortavernari/.nvm/versions/node/v22.16.0/bin:{}",
    resolve_clean_path_from_user_shell()
);
```

### Abordagem 3 (Mais elegante): Descobrir PATH do shell login e passar limpo

1. Executar `sh -l -c 'echo $PATH'` uma vez para obter o PATH real do shell de login
2. Filtrar apenas diretórios que não estão em locais TCC-protegidos
3. Setar esse PATH no `.env()` do Command

### Decisão: Abordagem 1 + 2 combinada

1. No `bash.rs`, detectar se `.nvm/versions/node/<version>/bin` existe em `$HOME`
2. Usar `Command::new(shell).env("PATH", clean_path).arg(...)` 
3. Injetar esse PATH em **todo** comando bash, evitando que o LLM precise fazer manualmente
4. O PATH limpo resolve o problema TCC porque o shell filho não expande referências a ~/Desktop etc.

## Arquivos a modificar

- `src-tauri/src/agent/tools/bash.rs` — Adicionar lógica de PATH no `execute()`

## Verificação

1. Rodar `./node_modules/.bin/tsc --noEmit` (ou qualquer comando) sem ver `export PATH=...` no JSON
2. Confirmar que `node`, `pnpm`, `tsc` funcionam **sem** o LLM precisar prepender PATH
3. Confirmar que macOS NÃO pede permissão TCC

## Tasks

1. Modificar `bash.rs` para detectar nvm PATH e setar `.env("PATH", ...)` no Command
2. Rodar testes existentes do bash tool para garantir que não quebrou nada
3. Fazer build e testar manualmente
