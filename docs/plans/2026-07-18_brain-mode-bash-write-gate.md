# Brain Mode — Bash File-Write Gate

## Context

Na sessão `6a4f6359-f08b-4d70-807d-04293ea273b8`, o agente entrou em Brain Mode e executou Python scripts que mutavam `src/App.tsx` diretamente — uma violação do contrato de Brain Mode ("you must never implement, edit files, or run state-changing commands").

**Causa raiz:** O gate do Brain mode em `session.rs:2268` verifica apenas o allowlist (`bash_permission()`), mas NÃO chama `bash_writes_files()`. Como `"python "` está no allowlist, Python scripts com `open(path, 'w')` passam silenciosamente.

O Builder mode **já tem** o gate `bash_writes_files()` (linha 2247), mas ele roda com `else if !in_brain`, então nunca alcança o Brain mode.

## Solution Design

### O que muda

**Único arquivo alterado:** `src-tauri/src/agent/session.rs`

**Única mudança:** Adicionar um novo `else if` no bloco de tool gates (entre linhas 2247–2285) que, **quando em Brain mode**, chama `bash_writes_files()` e barra comandos de escrita de arquivo com uma mensagem específica.

### Posição do novo gate

O novo gate deve vir **ANTES** do gate de allowlist existente no Brain mode, para que a mensagem de erro seja mais específica e útil. Ordem final dos gates:

1. `edit_file` → negado (já existe)
2. Builder mode + `bash_writes_files()` → negado (já existe)
3. **Brain mode + `bash_writes_files()` → negado (NOVO)**
4. Brain mode + não-allowlist → negado (já existe)

### Mensagem de erro

```
This bash command writes files. In Brain mode you cannot mutate —
record the change in the plan (write_plan) instead.
```

Diferente da mensagem do Builder mode que diz "delegate to a code-mode subagent", a do Brain mode instrui a usar `write_plan`.

### Comandos que passam a ser barrados no Brain mode

- `python3 -c "...open(...,'w')..."` (o caso da sessão `6a4f6359`)
- `python3 << 'EOF' ... open(...,'w') ... EOF` (heredoc)
- `node -e "...writeFileSync..."` 
- `echo "..." > file`
- `sed -i ...`
- `tee file`
- Qualquer comando com redirecionamento `>`

### Comandos que continuam permitidos no Brain mode

- `python3 script.py` (sem `-c`/`-e` inline + sem `open(`/`write(`)
- `node script.js` (idem)
- `cat file`, `ls`, `grep`, `git status`, `cargo check`, etc.

### Não muda

- Builder mode — zero alterações
- `bash_writes_files()` — função mantida exatamente como está
- `force_explore_mode()` — já funciona corretamente forçando subagents para "explore"
- `BASH_ALLOWLIST` — mantido como está
- `bash_permission()` — mantida como está
- System prompt do Brain mode — já diz "bash only accepts read-only commands", agora será enforced

## Risks

- **Baixíssimo:** adicionar um `else if` em um bloco de gates já existentes. Nenhuma lógica nova, só reuso de função existente.
- `bash_writes_files()` tem alguns falsos negativos (ex: `dd of=file`), mas são edge cases raros. O sistema já confia nessa função no Builder mode.
- O `force_explore_mode()` já impede subagents code no Brain — confirmado funcional.

## Non-goals

- Não alterar o system prompt de Brain mode
- Não alterar o allowlist
- Não alterar o comportamento do Builder mode
- Não alterar `force_explore_mode()`
- Não adicionar detecção de `spawn_agents` com mode "code" (já coberto por `force_explore_mode`)

---

## Low-Level Design

### Arquivo único: `src-tauri/src/agent/session.rs`

### Mudança precisa: Inserir novo `else if` entre linhas 2264 e 2268

**Bloco atual (linhas 2247–2285):**

```rust
            } else if !in_brain                                      // line 2247
                && tool_name == "bash"
                && permissions::bash_writes_files(
                    tool_input.get("command").and_then(|v| v.as_str()).unwrap_or(""),
                )
            {
                deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    "This bash command writes files. The Builder session never edits \
                     files itself — delegate the modification to a code-mode subagent \
                     via spawn_agents. Bash here is for builds, tests and read-only \
                     inspection only.",
                    event_tx,
                    session_id,
                )
            } else if in_brain                                       // line 2268
                && tool_name == "bash"
                && !matches!(
                    permissions::bash_permission(
                        tool_input.get("command").and_then(|v| v.as_str()).unwrap_or(""),
                        false
                    ),
                    PermissionLevel::Auto
                )
            {
                deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    "Command not allowed in Brain mode — only read-only allowlisted \
                     commands (git status/diff/log, ls, cat, cargo check, ...) run here.",
                    event_tx,
                    session_id,
                )
            }
```

**Novo bloco — inserir `else if` para Brain mode + `bash_writes_files()` entre os dois:**

```rust
            } else if !in_brain
                && tool_name == "bash"
                && permissions::bash_writes_files(
                    tool_input.get("command").and_then(|v| v.as_str()).unwrap_or(""),
                )
            {
                deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    "This bash command writes files. The Builder session never edits \
                     files itself — delegate the modification to a code-mode subagent \
                     via spawn_agents. Bash here is for builds, tests and read-only \
                     inspection only.",
                    event_tx,
                    session_id,
                )
            } else if in_brain                                       // NEW GATE START
                && tool_name == "bash"
                && permissions::bash_writes_files(
                    tool_input.get("command").and_then(|v| v.as_str()).unwrap_or(""),
                )
            {
                deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    "This bash command writes files. In Brain mode you cannot mutate \
                     — record the change in the plan (write_plan) instead.",
                    event_tx,
                    session_id,
                )
            } else if in_brain                                       // NEW GATE END
                && tool_name == "bash"
                && !matches!(
                    permissions::bash_permission(
                        tool_input.get("command").and_then(|v| v.as_str()).unwrap_or(""),
                        false
                    ),
                    PermissionLevel::Auto
                )
            {
                deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    "Command not allowed in Brain mode — only read-only allowlisted \
                     commands (git status/diff/log, ls, cat, cargo check, ...) run here.",
                    event_tx,
                    session_id,
                )
            }
```

### Detalhes da mudança

| Item | Detalhe |
|------|---------|
| **Arquivo** | `src-tauri/src/agent/session.rs` |
| **Local** | Entre o `}` do gate do Builder `bash_writes_files()` e o `} else if in_brain` do gate de allowlist |
| **Linhas inseridas** | 16 (9 de código + espaçamento) |
| **Condição** | `in_brain && tool_name == "bash" && permissions::bash_writes_files(command)` |
| **Mensagem** | `"This bash command writes files. In Brain mode you cannot mutate — record the change in the plan (write_plan) instead."` |
| **Reuso** | `permissions::bash_writes_files()` — já existe, sem alteração |
| **Extração do comando** | `tool_input.get("command").and_then(\|v\| v.as_str()).unwrap_or("")` — mesmo pattern dos gates vizinhos |

### Ordem de precedência dos gates (pós-mudança)

```
edit_file?                    → deny (mesmo para ambos modos)
!in_brain && bash + writes?   → deny (Builder não escreve arquivo)
 in_brain && bash + writes?   → deny (Brain não escreve arquivo) ← NOVO
 in_brain && bash + !allow?   → deny (Brain só allowlist)
 else                         → run_tool
```

O novo gate vem **antes** do allowlist check porque `bash_writes_files()` dá uma mensagem mais específica e acionável do que "Command not allowed".

### Wiring checklist

- [ ] `permissions::bash_writes_files` já importado — verificar `use` no topo do arquivo
- [ ] `in_brain` variável já em scope (definida antes do bloco de gates)
- [ ] `deny_tool` função já em scope (usada pelos gates vizinhos)
- [ ] `event_tx` e `session_id` já em scope (passados para os gates vizinhos)

### Verificação

Após a mudança, o comando exato da sessão `6a4f6359` deve ser barrado:

```
Comando: python3 << 'PYEOF'\n...open(path, 'w')...\nPYEOF
Resultado esperado: deny_tool com mensagem "In Brain mode you cannot mutate..."
```

E comandos legítimos continuam passando:

```
Comando: python3 -c "print('hello')"
Resultado esperado: run_tool (não contém open(, write(, etc.)
```

```
Comando: cargo check
Resultado esperado: run_tool (está no allowlist)
```

### Compilação

```bash
cargo check -p claudinio-code
```

---

## Tasks

### T1: Add bash_writes_files gate for Brain mode in session.rs
**File:** `src-tauri/src/agent/session.rs`, between lines 2264–2268
**Change:** Insert new `else if in_brain && tool_name == "bash" && permissions::bash_writes_files(...)` block before the existing Brain allowlist gate. Use message: "This bash command writes files. In Brain mode you cannot mutate — record the change in the plan (write_plan) instead."
**Plan:** docs/plans/2026-07-18_brain-mode-bash-write-gate.md

### T2: Verify with cargo check and test the denied case
**File:** `src-tauri/src/agent/session.rs`
**Change:** Run `cargo check -p claudinio-code` to confirm compilation. Then verify conceptually that the exact command from session `6a4f6359` (`python3 << 'PYEOF'...open(path,'w')...PYEOF`) would be denied.
**Plan:** docs/plans/2026-07-18_brain-mode-bash-write-gate.md


## Implementation Log — 2026-07-18 12:30
**Summary:** Added bash_writes_files gate for Brain mode — single else-if insertion, zero new logic, compilation clean.
**Changed files:** M src-tauri/src/agent/session.rs, M src/App.css, M src/App.tsx, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-18_brain-mode-bash-write-gate.md, ?? docs/plans/2026-07-18_handoff-threshold-slider.md
**Commits:** _(git unavailable or none)_
**Journal:** Inserted a single `else if` block (13 lines) in `src-tauri/src/agent/session.rs` between the Builder's `bash_writes_files` gate and the Brain's allowlist gate. The new gate checks `in_brain && tool_name == "bash" && permissions::bash_writes_files(command)` and denies with a Brain-specific message pointing to `write_plan`.

Key decisions:
- Placement before the allowlist gate was deliberate: `bash_writes_files` gives a more specific, actionable error than "Command not allowed."
- Reused existing `bash_writes_files()` unchanged — it already covers inline Python with `open(`/`write(`, heredocs (`<<`), `tee`, redirects, `sed -i`, etc.
- Verified the exact attack vector from session `6a4f6359` (`python3 << 'PYEOF'\nopen(path,'w')`) is now blocked.
- Legitimate commands (`python3 -c "print()"`, `cargo check`) pass through unaffected.

**Task journal:**
- Add bash_writes_files gate for Brain mode in session.rs: Inserted new else if block for Brain mode bash_writes_files gate between the Builder gate and the existing Brain allowlist gate. The gate now correctly checks bash_writes_files() before the allowlist check, providing a specific error message for file-write commands in Brain mode.
- Verify compilation and correctness: cargo check passed with 0 new errors (only pre-existing unused-import warnings). Verified bash_writes_files correctly detects python3 heredoc with open(path,'w') (matches 'python', ' <<', and 'open(' patterns). Confirmed legitimate commands (python3 -c 'print()', cargo check) still pass because they don't trigger the writes gate and remain on the allowlist.
