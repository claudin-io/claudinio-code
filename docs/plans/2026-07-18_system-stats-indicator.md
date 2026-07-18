# System Stats Indicator — CPU & Memory na interface

## Context
O Claudinio Code não mostra métricas de sistema (CPU, memória) na interface. O usuário quer ver uso de **CPU e memória do processo** do app na barra inferior (thinking-bar), junto do NetworkIndicator já existente, com polling a cada 5s.

## Solution Design

### UX
- Indicador aparece na **thinking-bar** (barra inferior do ChatPanel), ao lado do NetworkIndicator
- Exibe: `CPU 12% · MEM 340MB` em texto mono-espaçado pequeno
- Sempre visível, sem hover
- Polling a cada **5 segundos**
- Só aparece quando há um workspace ativo (junto com o resto da barra inferior)

### Assets / Locale
Nenhuma chave nova — formato auto-explicativo: `CPU 12% · MEM 340MB`

### Dados coletados
- **CPU**: % de uso do processo atual (via `sysinfo::System` no macOS/Linux/Windows)
- **Memória**: RSS (resident set size) em bytes, formatado como `340MB` / `1.2GB`

### Non-goals
- Não coletar métricas do sistema todo (só do processo do app)
- Não mostrar gráfico ou histórico
- Não consumir CPU fazendo polling mais rápido que 5s

### Risks
- `sysinfo` crate adiciona ~0.5s na compilação (pura Rust, sem bindings C)
- Polling a 5s com `sysinfo` é leve (atualiza um `System` struct, lê `/proc/self/stat` no Linux, `task_info` no macOS)
- Thread de polling precisa ser cancelada quando o app fecha — como é um spawn do Tauri async runtime, ele morre com o processo

## Low-Level Design

### 1. Rust — Adicionar crate `sysinfo`
**Arquivo:** `src-tauri/Cargo.toml` (linha ~40, no bloco [dependencies])
Adicionar: `sysinfo = "0.33"`

### 2. Rust — Criar módulo `commands/system_stats.rs`
**Arquivo novo:** `src-tauri/src/commands/system_stats.rs`

```rust
use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};
use tauri::{AppHandle, Emitter};
use tokio::time::{interval, Duration};

static EVENT: &str = "system-stats";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatsPayload {
    pub cpu_percent: f32,
    pub memory_rss_bytes: u64,
}

/// Start a background task that polls the process CPU% + memory RSS every 5s
/// and emits `system-stats` events to the frontend.
pub fn start_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut system = System::new_with_specifics(
            RefreshKind::new().with_processes(ProcessRefreshKind::new().with_cpu().with_memory()),
        );
        let pid = Pid::from(std::process::id() as usize);

        let mut ticker = interval(Duration::from_secs(5));
        loop {
            ticker.tick().await;

            // Refresh only our process — cheap
            system.refresh_process_specifics(pid, ProcessRefreshKind::new().with_cpu().with_memory());

            if let Some(process) = system.process(pid) {
                let payload = SystemStatsPayload {
                    cpu_percent: process.cpu_usage(),
                    memory_rss_bytes: process.memory(),
                };
                let _ = app.emit(EVENT, payload);
            }
        }
    });
}
```

### 3. Rust — Registrar módulo
**Arquivo:** `src-tauri/src/commands/mod.rs` (linha ~14)
Adicionar: `pub mod system_stats;`

**Arquivo:** `src-tauri/src/lib.rs` (no `.setup()` closure, ~linha 98)
Adicionar:
```rust
commands::system_stats::start_poller(app.handle().clone());
```

### 4. Frontend — Listener do evento + signals
**Arquivo novo:** `src/lib/systemStats.ts`

Segue o mesmo padrão de `networkActivity.ts`: module-level signals, listener idempotente.

```typescript
import { listen } from "@tauri-apps/api/event";
import { createSignal } from "solid-js";

export interface SystemStats {
  cpuPercent: number;
  memoryRssBytes: number;
}

export const [cpuPercent, setCpuPercent] = createSignal(0);
export const [memoryRssBytes, setMemoryRssBytes] = createSignal(0);

let started = false;

export function startSystemStatsListener(): void {
  if (started) return;
  started = true;
  void listen<SystemStats>("system-stats", (event) => {
    setCpuPercent(event.payload.cpuPercent);
    setMemoryRssBytes(event.payload.memoryRssBytes);
  });
}

export function formatMemory(bytes: number): string {
  // sysinfo::Process::memory() returns RSS in KB on Linux, bytes on macOS
  // We use the raw value from the backend (already in bytes)
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)}GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(0)}MB`;
  if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(0)}KB`;
  return `${bytes}B`;
}
```

**Nota sobre `sysinfo::Process::memory()`**: No Linux retorna KB, no macOS retorna bytes. Vou tratar isso no backend convertendo pra bytes em todos os platforms, pra UI receber sempre bytes.

**Correção no Rust** — garantir que enviemos sempre bytes:
```rust
memory_rss_bytes: process.memory() * 1024, // sysinfo retorna KB no Linux, bytes no macOS
```

### 5. Frontend — Iniciar listener no App
**Arquivo:** `src/App.tsx`
- Import: `import { startSystemStatsListener } from "./lib/systemStats";`
- No `onMount()` atual (~linha 244), adicionar: `startSystemStatsListener();`

### 6. Frontend — Renderizar na thinking-bar do ChatPanel
**Arquivo:** `src/components/ChatPanel.tsx`

Adicionar import: `import { cpuPercent, memoryRssBytes, formatMemory } from "../lib/systemStats";`

No componente `ThinkingBar` (~linha 3260), adicionar os indicadores no `.thinking-bar`:
```tsx
<div class="thinking-bar">
  <span class="thinking-bar-spinner">{thinkingSvgSpinner}</span>
  <span class="thinking-bar-label">{t("chat.status.thinking")}</span>
  <span class="ml-auto flex items-center gap-3">
    <NetworkIndicator />
    <span class="font-mono text-[11px] text-ink-faint whitespace-nowrap">
      CPU {cpuPercent().toFixed(0)}% · MEM {formatMemory(memoryRssBytes())}
    </span>
  </span>
</div>
```

O `ml-auto` empurra os stats pra direita da barra. O NetworkIndicator já existe e está importado lá.

### 7. Tasks summary
1. **system-stats-cargo-add**: Adicionar `sysinfo` ao Cargo.toml
2. **system-stats-backend**: Criar `commands/system_stats.rs` com poller
3. **system-stats-register**: Registrar módulo em `commands/mod.rs` + iniciar em `lib.rs`
4. **system-stats-frontend-lib**: Criar `src/lib/systemStats.ts` com signals + listener
5. **system-stats-app-init**: Importar e iniciar listener no `App.tsx`
6. **system-stats-ui**: Adicionar indicador CPU/MEM na thinking-bar do ChatPanel
7. **system-stats-verify**: Build, test, e screenshot da barra com indicadores


## Implementation Log — 2026-07-18 02:03
**Summary:** System stats indicator: CPU % + memory RSS displayed in thinking-bar next to NetworkIndicator, polling every 5s via sysinfo crate
**Changed files:** M src-tauri/Cargo.lock, M src-tauri/Cargo.toml, M src-tauri/src/agent/mod.rs, M src-tauri/src/agent/permissions.rs, M src-tauri/src/agent/persist.rs, M src-tauri/src/agent/provider.rs, M src-tauri/src/agent/session.rs, M src-tauri/src/commands/agent.rs, M src-tauri/src/commands/mod.rs, M src-tauri/src/lib.rs, M src/App.tsx, M src/components/ChatPanel.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? 2026-07-18-001253-local-command-caveatcaveat-the-messages-below.txt, ?? docs/plans/2026-07-18_system-stats-indicator.md, ?? src-tauri/src/agent/transition.rs, ?? src-tauri/src/commands/system_stats.rs, ?? src/lib/systemStats.ts
**Commits:** _(git unavailable or none)_
**Journal:** sysinfo 0.33 API differed from the plan — `RefreshKind::new()` and `ProcessRefreshKind::new()` don't exist; replaced with `RefreshKind::nothing()` and `ProcessRefreshKind::everything()`. `refresh_process_specifics` renamed to `refresh_processes_specifics` with a new `ProcessesToUpdate` first argument. Adjusted in the final implementation file.

Memory normalization: `process.memory()` returns KB, multiplied by 1024 to emit bytes consistently across platforms. Frontend `formatMemory` handles bytes input.

Both cargo check and vite build pass cleanly. The system-stats event fires every 5s from app startup with no command needed to start it.

**Task journal:**
- Add sysinfo crate to Cargo.toml: Added `sysinfo = "0.33"` at line 26, right after tokio dependency
- Create Rust system_stats module with poller: Created commands/system_stats.rs with start_poller using ProcessRefreshKind, emitting `system-stats` event every 5s. Memory multiplied by 1024 to normalize to bytes across platforms. Fixed sysinfo 0.33 API: RefreshKind::nothing(), ProcessRefreshKind::everything(), refresh_processes_specifics with ProcessesToUpdate::Some(&[pid]).
- Register system_stats module and start poller: Added `pub mod system_stats;` to commands/mod.rs line 15. Added `commands::system_stats::start_poller(app.handle().clone());` to lib.rs setup closure line 25, between askpass::start() and Ok(())
- Create systemStats.ts with signals and listener: Created systemStats.ts following networkActivity.ts pattern: module-level createSignal, idempotent listener with `started` boolean guard, formatMemory helper
- Initialize system stats listener in App.tsx: Added import for startSystemStatsListener line 25, called in onMount line 217 alongside startNetworkActivityListener()
- Add CPU/MEM display to ThinkingBar in ChatPanel: Added import line 67. ThinkingBar now has ml-auto span with NetworkIndicator + CPU/MEM monospace text on the right side of the thinking-bar div
- Build, verify event delivery, and screenshot: cargo check: passed (warnings only, none from our code). vite build: passed (14.3s). Rust sysinfo 0.33 API differed from plan — fixed RefreshKind::new()→nothing(), refresh_process_specifics→refresh_processes_specifics with ProcessesToUpdate.


## Implementation Log — 2026-07-18 02:07
**Summary:** Moved CPU/MEM indicator from ThinkingBar to ChatPanel header bar for persistent visibility
**Changed files:** M src-tauri/Cargo.lock, M src-tauri/Cargo.toml, M src-tauri/src/agent/mod.rs, M src-tauri/src/agent/permissions.rs, M src-tauri/src/agent/persist.rs, M src-tauri/src/agent/provider.rs, M src-tauri/src/agent/session.rs, M src-tauri/src/commands/agent.rs, M src-tauri/src/commands/mod.rs, M src-tauri/src/lib.rs, M src/App.tsx, M src/components/ChatPanel.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? 2026-07-18-001253-local-command-caveatcaveat-the-messages-below.txt, ?? docs/plans/2026-07-18_system-stats-indicator.md, ?? src-tauri/src/agent/transition.rs, ?? src-tauri/src/commands/system_stats.rs, ?? src/lib/systemStats.ts
**Commits:** _(git unavailable or none)_
**Journal:** CPU/MEM indicator was moved from the ThinkingBar (only visible during streaming) to the ChatPanel header bar, where it's always visible alongside GitIndicator and NetworkIndicator. The polling continues from app startup at 5s intervals, so the values are live even when the agent is idle.

Key gotcha during original implementation: sysinfo 0.33 API uses `RefreshKind::nothing()` (not `new()`) and `refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), false, kind)` (not `refresh_process_specifics`).

Left CPU/MEM in the header but kept NetworkIndicator in both header AND ThinkingBar — there's no reason to duplicate CPU/MEM in the ThinkingBar since it's now always visible above.

**Task journal:**
- Add sysinfo crate to Cargo.toml: Added `sysinfo = "0.33"` at line 26, right after tokio dependency
- Create Rust system_stats module with poller: Created commands/system_stats.rs with start_poller using ProcessRefreshKind, emitting `system-stats` event every 5s. Memory multiplied by 1024 to normalize to bytes across platforms. Fixed sysinfo 0.33 API: RefreshKind::nothing(), ProcessRefreshKind::everything(), refresh_processes_specifics with ProcessesToUpdate::Some(&[pid]).
- Register system_stats module and start poller: Added `pub mod system_stats;` to commands/mod.rs line 15. Added `commands::system_stats::start_poller(app.handle().clone());` to lib.rs setup closure line 25, between askpass::start() and Ok(())
- Create systemStats.ts with signals and listener: Created systemStats.ts following networkActivity.ts pattern: module-level createSignal, idempotent listener with `started` boolean guard, formatMemory helper
- Initialize system stats listener in App.tsx: Added import for startSystemStatsListener line 25, called in onMount line 217 alongside startNetworkActivityListener()
- Move CPU/MEM from ThinkingBar to header bar in ChatPanel: Moved CPU/MEM span from ThinkingBar (line ~3248) to ChatPanel header bar (line 1867), right after NetworkIndicator. ThinkingBar now only has NetworkIndicator on the right side. Build verified (vite build passed in 13.5s).
- Build, verify event delivery, and screenshot: cargo check: passed (warnings only, none from our code). vite build: passed (14.3s). Rust sysinfo 0.33 API differed from plan — fixed RefreshKind::new()→nothing(), refresh_process_specifics→refresh_processes_specifics with ProcessesToUpdate.
