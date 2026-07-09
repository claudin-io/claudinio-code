# Fix: Windows Network Drive — "workspace not open" 错误

## Context

用户在 Windows 上打开位于 `Z:\Prithv\Documents\GitHub\trading_new` 的 workspace — 这很可能是一个网络驱动器（映射的网络共享或 subst 驱动器）。应用在打开 workspace 时失败，随后所有消息发送均返回 `"Failed to send: workspace not open: Z:\..."`。

### 复现链路

1. 用户通过 "Open Folder" 对话框在 Windows 上选择 `Z:\...`。
2. 前端 `indexProject()` 在 `openWorkspace()` IPC 完成**之前**即调用 `addOpenWorkspace()`——UI 假定成功并显示已打开的 workspace。
3. 后端 `open_workspace` 遍历各阶段：索引数据库、模型加载、扫描、嵌入——全部成功或降级。
4. **第 201 行**：`let watcher = FileWatcher::start(&path, &db_path, app_handle.clone())?;` — `notify::recommended_watcher` 在 Windows 上使用 `ReadDirectoryChangesW`，该 API **不支持网络驱动器**。`.watch()` 调用失败，触发 `?` 传播，导致整个 `open_workspace` 在 workspace 被插入 `HashMap` **之前**即返回错误。
5. 前端 `catch` 块收到错误信息并显示，但 workspace 在前端的 `openWorkspaces` 列表中**仍然**存在（步骤 2 中添加）。
6. 用户尝试发送消息 → `send_message` 调用 `state.workspace("Z:\...")` → `"workspace not open: Z:\..."`。

### 影响范围

- 影响所有通过不支持的 FS 监视器后端的路径：Windows 网络驱动器、某些 WSL 路径、以及 `notify` 无法附加监视器的任何挂载点。
- 此问题在所有平台上都有可能发生——缺陷在于监视器失败导致整个 workspace 打开操作崩溃，而非降级。

## 解决方案设计

**策略**：让 FileWatcher 失败变为非致命性 — 即使监视器无法启动，workspace 也应正常打开。在 UI 中显示非阻塞性警告。WorkspaceState 新增一个字段用于记录警告，供 UI 后续读取。从打开到发送的整个流程必须能容错处理无监视器的情况。

### 变更

#### 1. `src-tauri/src/state.rs` — 向 WorkspaceState 添加 `watcher_warning` 字段

- **目标**：`WorkspaceState` 结构体（第 27-37 行）
- **变更**：新增字段 `pub watcher_warning: Mutex<Option<String>>`
- **原因**：workspace 需要记住为何文件监视器离线，以便当 ChatPanel 启动 session 时，UI 可以读取该状态。`Option` 表示正常（None）或警告消息（Some）。
- **约束**：`_watcher` 字段类型变更为 `Mutex<Option<FileWatcher>>`（已是如此——无需变更，仅 _watcher 在监视器失败时保持为 None 即可）。

#### 2. `src-tauri/src/commands/code_intel.rs` — 使监视器失败变为非致命性

- **目标**：`open_workspace` 函数，第 201-218 行
- **变更**：将对 `FileWatcher::start()` 的 `?` 调用替换为 `match`，记录错误、设置 warning 并继续 — 不插入监视器，workspace 仍被插入 HashMap。
- **精确变更**：将第 201 行从：
  ```rust
  let watcher = FileWatcher::start(&path, &db_path, app_handle.clone())?;
  ```
  替换为：
  ```rust
  let (watcher, watcher_warning): (Option<FileWatcher>, Option<String>) =
      match FileWatcher::start(&path, &db_path, app_handle.clone()) {
          Ok(w) => (Some(w), None),
          Err(e) => {
              eprintln!("[open_workspace] file watcher failed (workspace still usable): {e}");
              let _ = app_handle.emit("index-progress", indexer::IndexProgress {
                  status: "watcher_warning".into(),
                  files_indexed: files_count,
                  symbols_indexed: symbols_count,
                  total_files: files_count,
                  workspace: path.clone(),
              });
              (None, Some(format!("Live file watching unavailable: {e}")))
          }
      };
  ```
- **第 217 行**：将 `_watcher: tokio::sync::Mutex::new(Some(watcher))` 改为 `_watcher: tokio::sync::Mutex::new(watcher)`，并新增 `watcher_warning: tokio::sync::Mutex::new(watcher_warning)`。
- **`IndexStatus` 结构体**（第 14-19 行）：新增可选字段 `pub watcher_warning: Option<String>`。在 Ok 返回值中填充该字段。

#### 3. `src/lib/ipc.ts` — 新增 `IndexStatus.watcherWarning` 类型字段

- **目标**：TS 类型中的 `IndexStatus` 接口
- **变更**：新增 `watcherWarning?: string`
- **原因**：前端需要接收警告信息。

#### 4. `src/lib/locales/en-US.ts` + `src/lib/locales/pt-BR.ts` — i18n 键值

- **目标**：在两个文件中的现有 `app.index` 键值之后
- **变更**：新增 `"app.index.watcherWarning": "Live file watching unavailable — filesystem changes won't be re-indexed automatically."`（en-US）和 `"app.index.watcherWarning": "Monitoramento de arquivos indisponível — alterações não serão reindexadas automaticamente."`（pt-BR）。

#### 5. `src/App.tsx` — 在 UI 中显示警告

- **目标**：`indexProject` 函数（第 289-316 行）以及 workspace 状态 UI
- **变更 A**（indexProject）：在第 300 行成功调用 `openWorkspace(folder, ...)` 之后，检查 `s.watcherWarning`——如果存在，将 index 状态设置为 `⚠️ {warning}` 而非 `"{0} files, {1} symbols"`。
- **变更 B**：在 workspace 侧边栏条目中，当 `watcherWarning` 处于活动状态时显示一个小警告图标（或将 index 状态映射到 status）。

### 保持不变的内容

- `watcher.rs` 无需变更——它正确地返回 `Result`。处理层在命令层。
- `close_workspace` 无需变更——它仅移除 HashMap 条目。
- 消息发送路径和其他工具路径均无需变更——它们调用 `state.workspace()`，只要 workspace 已打开即可正常工作。代理文件操作会在下次文件系统扫描时（通过用户手动触发或编辑文件时）检测到变更，只是没有实时监视器。

## 风险

- **低**：将 FileWatcher 设为可选引入了其自身 tokio task 可能 panic 的风险。已检查：watcher 线程已正确处理 `rx.recv()` 错误并优雅退出。没有问题。
- **低**：前端在 `openWorkspace()` 调用**之前**就将 workspace 标记为打开的顺序问题（`App.tsx:289`）。仅需在 workspace 成功打开时显示警告——错误路径（第 313 行 catch）仍需处理。如果有意保留该顺序，API 使用 `activate: false` 调用 `indexProject` 的情况（`onMount` 恢复）已经能够正确处理——失败时会显示错误状态。

## 任务摘要

此修复涉及 5 处代码变更 + 2 处 i18n 新增：
1. `WorkspaceState` 新增 `watcher_warning` 字段
2. `open_workspace` 优雅处理 FileWatcher 失败
3. `IndexStatus` 新增 `watcher_warning` 可选字段
4. TS `IndexStatus` 类型新增字段
5. 两个语言文件各新增 1 行 i18n
6. 前端在 UI 中显示警告


## Implementation Log — 2026-07-09 23:02
**Changed files:** M src-tauri/src/agent/provider.rs, M src-tauri/src/agent/session.rs, M src-tauri/src/agent/subagent.rs, M src-tauri/src/agent/tools/mod.rs, M src-tauri/src/commands/code_intel.rs, M src-tauri/src/state.rs, M src/App.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-09_windows-network-drive-watcher-fix.md
**Commits:** _(git unavailable or none)_
**Journal:** Fixed the "workspace not open" crash on Windows network drives by making FileWatcher::start failure non-fatal. Key decisions:

1. **Backend**: Replaced `let watcher = FileWatcher::start(...)?` with a `match` that captures the error, logs it, emits a `watcher_warning` index-progress event, and continues with `watcher: None` + a warning string. Added `watcher_warning: Mutex<Option<String>>` to `WorkspaceState` so the warning persists across re-activations.

2. **IndexStatus**: Added `pub watcher_warning: Option<String>` to the IPC response struct (with `#[serde(skip_serializing_if = "Option::is_none")]` so it's absent when there's no warning). The early-return path (re-activating an already-open workspace) now reads the warning from the existing `WorkspaceState`.

3. **Frontend**: `IndexStatus` interface in ipc.ts updated. `indexProject()` now checks `s.watcherWarning` and appends it to the status display when present: `"{0} files, {1} symbols — ⚠️ Live file watching unavailable: ..."`.

4. **Dropped**: No need to show a separate warning icon in the sidebar — the index status text already appears next to each workspace entry. The design intent is non-blocking: the workspace works fine, the agent can still edit files, run bash, etc. — it just won't auto-detect external edits.

5. **Compiled**: Both `cargo check` and `vite build` pass with zero new errors.",
<｜｜DSML｜｜parameter name="summary" string="true">FileWatcher failure on Windows network drives (and other unsupported paths) no longer blocks workspace opening. UI shows non-blocking warning.

**Task journal:**
- Add watcher_warning field to WorkspaceState: Added `pub watcher_warning: Mutex<Option<String>>` field to WorkspaceState struct in state.rs after the `_watcher` field
- Make FileWatcher failure non-fatal in open_workspace: Made FileWatcher::start failure non-fatal in open_workspace - replaced `?` with match, added `watcher_warning` field to IndexStatus, propagates existing warning on re-activation
- Add watcherWarning to TS IndexStatus type: Added `watcherWarning?: string` to the IndexStatus interface in ipc.ts
- Add i18n keys for watcher warning: Added app.index.watcherWarning key to both en-US.ts and pt-BR.ts
- Show watcher warning in frontend UI: Added conditional logic in indexProject to append watcherWarning to the status display when present
