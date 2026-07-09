# Golden Goals System — Solution Design

## Context

O sistema atual permite que o agente receba instruções e crie tasks via `tasks_get`/`tasks_set`. Porém não há um mecanismo nativo de **metas obrigatórias** que o agente deve cumprir em loop até atingir — o agente executa uma vez e termina independente do resultado.

A funcionalidade proposta adiciona um novo parser de tags `<goal>` no input do usuário que:

1. Extrai goals obrigatórios do texto
2. Cria **golden tasks** automaticamente (tasks com prefixo `golden-` no ID)
3. Modifica o system prompt para instruir o agente sobre golden tasks
4. Adiciona um **pós-loop de verificação** no `run_workflow` que detecta se as golden tasks foram concluídas
5. Se não concluídas, alterna automaticamente entre Brain e Builder até atingir os goals
6. Suporta múltiplos goals na mesma instrução (`<goal>a</goal> <goal>b</goal>`)

## Solution Design

### 1. Parser de `<goal>` no Backend Rust

**Localização**: `src-tauri/src/agent/session.rs`, função `parse_goals()` — nova função pura.

```rust
fn parse_goals(text: &str) -> Vec<String> {
    // Regex: <goal>(.*?)</goal>
    // Extrai conteúdo, retorna lista de strings
}
```

**Comportamento**:
- Extrai TODOS os `<goal>...</goal>` do texto bruto
- Remove as tags do texto enviado ao LLM (substitui por string vazia)
- Retorna (cleaned_text, Vec<String>)

### 2. Golden Task Creation

**Localização**: `src-tauri/src/agent/tools/tasks.rs` — nova função `create_golden_tasks()`

```rust
pub fn create_golden_tasks(goals: &[String]) -> Vec<TaskItem> {
    // Para cada goal, gera 2 tasks placeholder:
    //   golden-<slug>-0: "Planejar: <goal>" 
    //   golden-<slug>-1: "Executar: <goal>"
    // prefixo "golden-" identifica que é obrigatória
}
```

**Slug**: Remove acentos, lowercase, substitui espaços por hífens, trunca em 40 chars.

Exemplo: `<goal>code coverage in 80%</goal>` → 
- `golden-code-coverage-in-80-0`: "Planejar como atingir code coverage in 80%"
- `golden-code-coverage-in-80-1`: "Executar o plano para code coverage in 80%"

### 3. Modificação no System Prompt

**Localização**: `src-tauri/src/agent/session.rs` — variáveis `BRAIN_PROMPT` e `BUILDER_PROMPT`

Adicionar seção GOLDEN TASKS em ambos os prompts:

```
## GOLDEN TASKS (OBRIGATÓRIAS)

Tasks com ID prefixado `golden-` são metas obrigatórias:
- Você NÃO pode encerrar o turn enquanto golden tasks estiverem com status diferente de 'done'
- Depois de completar todas as golden tasks, use o tool `tasks_get` para verificar se ainda
  existem golden tasks pendentes
- Golden tasks normais (sem prefixo `golden-`) não têm esta restrição
```

### 4. Post-Done Verification Loop

**Localização**: `src-tauri/src/agent/session.rs` — modificação no `run_workflow()`

O loop atual termina em 3 lugares (end_turn, interrupted, max_rounds) emitindo `AgentEvent::Done`. 
A nova lógica é:

**ANTES de emitir `AgentEvent::Done`**:
1. Carregar tasks atuais do `session_store`
2. Filtrar tasks com prefixo `golden-`
3. Se golden tasks existirem E alguma NÃO estiver 'done':
   a. Se o modo atual é Builder → alternar para Brain (auto-mode-switch)
   b. Se o modo atual é Brain → alternar para Builder (auto-mode-switch)
   c. Persistir Mode change
   d. Resetar steering/interrupt flags
   e. **NÃO emitir Done** — injetar mensagem de steering: 
      "O sistema detectou que golden tasks ainda não foram concluídas: [lista].
       Retome o trabalho no modo [Brain/Builder] para concluí-las."
   f. Dar `continue` no loop principal
4. Se todas as golden tasks estão 'done' OU não existem golden tasks → emitir Done normalmente

**Proteção contra loop infinito**:
- Usar `config.max_golden_cycles` (nova config field, padrão: 5)
- Contador de ciclos golden (persistido como `SessionRecord::GoldenCycle` opcional)
- Se exceder limite, emitir Done com stop_reason "max_golden_cycles"
- Se golden tasks não progredirem (mesmo conjunto de tasks) por 2 ciclos consecutivos,
  emitir Done com stop_reason "golden_stalled"

### 5. Novos Tipos e Constantes

**SessionRecord** — novo variant:
```rust
GoldenCycle { cycle: u32, mode: String, goals: Vec<String>, ts: u64 }
```

**AgentConfig** — novo field:
```rust
pub max_golden_cycles: Option<usize>,  // default: 5
pub max_golden_stalls: Option<usize>,  // default: 2
```

**AgentEvent** — novo `stop_reason`:
- `"max_golden_cycles"` — excedeu ciclos máximos de golden loop
- `"golden_stalled"` — golden tasks não progrediram

**Constantes em session.rs**:
```rust
const GOLDEN_TASK_PREFIX: &str = "golden-";
const DEFAULT_MAX_GOLDEN_CYCLES: usize = 5;
const DEFAULT_MAX_GOLDEN_STALLS: usize = 2;
```

### 6. Integração no Fluxo de Mensagens

**`commands/agent.rs` — `send_message()`**: 
Antes de chamar `session::run_workflow()`:
1. Chamar `parse_goals()` no texto da mensagem
2. Se goals extraídos:
   a. Criar golden tasks via `create_golden_tasks()`
   b. Salvar no session store via `append_tasks()`
   c. Passar texto limpo (sem tags) para `run_workflow()`
   d. Passar goals extraídos para o workflow

**`session.rs` — `run_workflow()`**:
Novo parâmetro opcional: `golden_goals: Vec<String>`

### 7. Frontend: Indicador Visual de Golden Tasks

**TasksPanel.tsx**: 
- Task dots com prefixo `golden-` recebem um ícone de estrela/ouro adicional
- Tooltip mostra "(Golden — obrigatória)" no título
- Popover tem borda dourada

**ChatPanel.tsx**:
- Novo evento `GoldenLoop` do backend indica início/fim do golden loop
- Indicador visual no timeline: "🎯 Golden loop: ciclo X de Y"

### 8. Configuração no Settings

Novo campo no config modal (`App.tsx`):
- "Max golden cycles" (input number, default 5, 0 = infinite)
- "Max golden stalls" (input number, default 2)

## Files to Change

| File | Change |
|------|--------|
| `src-tauri/src/agent/session.rs` | Adicionar `parse_goals()`, modificar `run_workflow()` com post-Done verification loop, modificar BRAIN_PROMPT e BUILDER_PROMPT, adicionar constantes golden |
| `src-tauri/src/commands/agent.rs` | `send_message()`: parse goals antes de chamar `run_workflow()`, criar golden tasks |
| `src-tauri/src/agent/tools/tasks.rs` | Adicionar `create_golden_tasks()`, `is_golden()`, `golden_tasks_remaining()` |
| `src-tauri/src/agent/persist.rs` | Adicionar `SessionRecord::GoldenCycle` |
| `src-tauri/src/agent/provider.rs` | Adicionar `max_golden_cycles` e `max_golden_stalls` ao `AgentConfig` |
| `src-tauri/src/state.rs` | Nenhuma mudança (reusa SteeringCtl existente) |
| `src/components/ChatPanel.tsx` | Suporte a evento GoldenLoop, indicador visual |
| `src/components/TasksPanel.tsx` | Estilo especial para golden tasks (borda dourada, ícone) |
| `src/lib/ipc.ts` | Tipos para GoldenLoop event |

## Risks

1. **Loop infinito**: Mitigado por `max_golden_cycles` e detecção de stall
2. **Custo de tokens não esperado**: Cada ciclo Brain+Builder consome tokens — o usuário vê o custo acumulado no SessionStats
3. **Agente não entende as golden tasks**: Mitigado pela instrução no system prompt + o backend forçar o modo correto automaticamente
4. **Race condition com steering do usuário**: O golden loop respeita interrupt flag — usuário pode parar manualmente
5. **Golden tasks antigas conflitam com novas**: Tasks_set é full replace — ao criar novos goals, tasks anteriores sem prefixo golden são preservadas, golden são recriadas

## Tasks Summary

Cada task é auto-contida para um Builder subagent executar:

1. **Parse goals**: Criar `parse_goals()` em session.rs + regex extraction + test
2. **Golden tasks helpers**: `create_golden_tasks()`, `is_golden()`, `golden_tasks_remaining()` em tools/tasks.rs
3. **Persist golden cycle**: `SessionRecord::GoldenCycle` em persist.rs + serialization test
4. **Config fields**: `max_golden_cycles`, `max_golden_stalls` em provider.rs AgentConfig
5. **System prompt**: Modificar BRAIN_PROMPT e BUILDER_PROMPT com seção GOLDEN TASKS
6. **Post-Done verification**: Modificar `run_workflow()` em session.rs com golden loop
7. **Integration in send_message**: Commands/agent.rs parse goals, create tasks, pass to workflow
8. **Frontend TasksPanel**: Estilo dourado para golden tasks
9. **Frontend ChatPanel**: Evento GoldenLoop + indicador visual
10. **Frontend config**: Campos max_golden_cycles e max_golden_stalls no settings
