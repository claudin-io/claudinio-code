# Context

O usuário quer modificar o Settings para:
1. Remover os campos de **Base URL** e **Model** (text input único)
2. Adicionar **dois selectores de modelo**: um para Brain mode e outro para Builder mode
3. Default de ambos: **"claudinio"**
4. Opções obtidas via chamada `GET {base_url}/v1/models` — data-driven, com fallback para `["claudinio", "claudius"]`
5. Base URL fixa em `https://api.claudin.io` (editável apenas manualmente em config.json)

## Estado atual
- `AgentConfig` em `provider.rs` tem `model: String` (único, default "claudinio")
- Settings em `App.tsx` renderiza inputs text para Base URL e Model
- `get_config`/`set_config` em `commands/agent.rs` lidam com único `model` e `base_url`
- `run_workflow` em `session.rs` usa `config.model` para chamar `stream_message`
- Subagents também usam `config.model`

## Decisões confirmadas
- ✅ Base URL fixa (`https://api.claudin.io`) — não editável no UI, só manualmente em config.json
- ✅ Dois campos separados: `brain_model` e `builder_model` no config.json
- ✅ Data-driven: fetch `/v1/models` com fallback para `["claudinio", "claudius"]`

# Solution Design

## 1. Rust: `provider.rs` — AgentConfig changes

### Adicionar campos
```rust
pub brain_model: String,   // default: "claudinio"
pub builder_model: String, // default: "claudinio"
```

### Adicionar método helper
```rust
impl AgentConfig {
    pub fn model_for_mode(&self, mode: SessionMode) -> &str {
        match mode {
            SessionMode::Brain => &self.brain_model,
            SessionMode::Builder => &self.builder_model,
        }
    }
}
```

### Modificar `stream_message`
- Aceitar `model: &str` como parâmetro em vez de ler de `config.model`
- O RequestBody usa `model: model.to_string()`

**ATENÇÃO:** Manter o campo `model` existente com `#[serde(default)]` para backward compat de configs existentes. Mas o UI não vai mais escrevê-lo. Podemos remover depois de uma migração, mas para já é mais seguro manter.

## 2. Rust: `session.rs` — Runtime resolution

### Em `run_workflow`
- Onde `stream_message` é chamado, passar o modelo resolvido:
```rust
let current_mode = mode_ctl.get().0;
let resolved_model = config.model_for_mode(current_mode);
let output = provider::stream_message(
    config, &resolved_model, &history, ...
).await?;
```

### Modificar assinatura de `run_workflow`
- Não precisa mudar — `config` já tem o método `model_for_mode`

## 3. Rust: `subagent.rs` — Subagent model

### Subagents usam o modelo do Builder (execução/build mode)
- Passar `config.builder_model` para `stream_message` nas subagents
- Ou receber o modelo resolvido como parâmetro

## 4. Rust: `commands/agent.rs` — IPC commands

### `SetConfigArgs`
- Substituir `model: Option<String>` por:
  - `brain_model: Option<String>`
  - `builder_model: Option<String>`
- Manter `base_url` mas ignorar no UI (só settável programaticamente)

### `set_config`
- Aplicar `brain_model` e `builder_model` ao `AgentConfig`
- Manter lógica de `base_url` para compat

### `get_config`
- Retornar `brainModel` e `builderModel` no JSON em vez de `model`

### Novo comando: `list_models`
```rust
#[tauri::command]
pub async fn list_models(state: State<'_, AppState>) -> Result<Vec<String>, String>
```
- Faz GET `{base_url}/v1/models` com API key
- Tenta parsear formatos: `{data: [{id: "..."}]}`, array simples, `{models: [...]}`
- Fallback: `["claudinio", "claudius"]`

## 5. Frontend: `src/lib/ipc.ts` — TypeScript types

### `AgentConfig` interface
- Replace `model: string` → `brainModel: string; builderModel: string`
- Keep `baseUrl` (retornado mas não exibido)

### `SetConfigArgs` interface
- Replace `model?: string` → `brainModel?: string; builderModel?: string`

### Nova função
```typescript
export function listModels(): Promise<string[]>
```

## 6. Frontend: `src/App.tsx` — Settings modal

### Remover
- Campo Base URL input
- Campo Model input

### Adicionar dois selectores
```tsx
// Brain Model selector
<label>Brain Model</label>
<select value={configBrainModel()} onChange={...}>
  <For each={availableModels()}>
    {(m) => <option value={m}>{m}</option>}
  </For>
</select>

// Builder Model selector
<label>Builder Model</label>
<select value={configBuilderModel()} onChange={...}>
  <For each={availableModels()}>
    {(m) => <option value={m}>{m}</option>}
  </For>
</select>
```

### Signals
- `configBrainModel` — signal string (default "claudinio")
- `configBuilderModel` — signal string (default "claudinio")  
- `availableModels` — signal string[] (populated from `listModels()` on open)
- Remover `configBaseUrl` e `configModel`

### openConfig
- Chamar `getConfig()` + `listModels()` em paralelo
- Popular `configBrainModel(cfg.brainModel)` e `configBuilderModel(cfg.builderModel)`
- Popular `availableModels(models)`

### saveConfig
- Enviar `brainModel` e `builderModel` em vez de `model`
- Não enviar `baseUrl`

## 7. Locale files

### en-US.ts / pt-BR.ts
- Substituir `app.config.model` → `app.config.brainModel`, `app.config.builderModel`
- Remover `app.config.baseUrl`
- Adicionar hint de fallback se necessário

# Risks

1. **Backward compat de config.json existente**: Usuários com `config.json` contendo `model` vão perder a configuração ao atualizar (model será ignorado). Como default é "claudinio" para ambos, é aceitável — o usuário explicitamente pediu a mudança.

2. **Subagent model**: Decidir se subagents usam brain_model ou builder_model. Proposta: subagents (sempre spawned durante execução) usam **builder_model**.

3. **API `/v1/models` não disponível**: Fallback seguro para `["claudinio", "claudius"]`, que são os dois únicos modelos que existem. Zero risco.

4. **Base URL customizada**: Usuários que apontavam para URL diferente vão perder essa configuração no UI. Ainda podem editar manualmente o `config.json`. Aceitável.

# Tasks Summary

| # | Task | Area |
|---|------|------|
| 1 | Rust: Add `brain_model`/`builder_model` to `AgentConfig` + `model_for_mode()` | provider.rs |
| 2 | Rust: Update `stream_message` to accept resolved model param | provider.rs |
| 3 | Rust: Add `list_models` Tauri command | commands/agent.rs |
| 4 | Rust: Update `SetConfigArgs`/`set_config`/`get_config` for new fields | commands/agent.rs |
| 5 | Rust: Update `run_workflow` to resolve model from mode | session.rs |
| 6 | Rust: Update subagent.rs to pass correct model | subagent.rs |
| 7 | Frontend: Update IPC types + add `listModels` | ipc.ts |
| 8 | Frontend: Redesign Settings modal (two selectors, no baseUrl) | App.tsx |
| 9 | Locale: Update en-US and pt-BR strings | locales/ |
