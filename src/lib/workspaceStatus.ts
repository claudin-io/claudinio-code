import { createStore } from "solid-js/store";

/// Per-workspace agent status, written by each ChatPanel instance and read by
/// the sidebar so a background workspace can show a "running" indicator.
export type WsStatus =
  | "idle"
  | "thinking"
  | "awaiting_approval"
  | "awaiting_input"
  | "done"
  | "error";

const [workspaceStatus, setStatus] = createStore<Record<string, WsStatus>>({});

export { workspaceStatus };

export function setWorkspaceStatus(workspace: string, status: WsStatus) {
  setStatus(workspace, status);
}

export function clearWorkspaceStatus(workspace: string) {
  setStatus(workspace, undefined as unknown as WsStatus);
}

/// Statuses where the agent is actively working or waiting on the user.
export function isBusy(status: WsStatus | undefined): boolean {
  return status === "thinking" || status === "awaiting_approval" || status === "awaiting_input";
}
