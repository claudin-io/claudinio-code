import type { AgentEvent } from "./ipc";

/// Per-workspace event buffer. When a ChatPanel is inactive (or unmounted via
/// <Show>), AgentEvents from in-flight IPC calls are pushed here instead of
/// going into the panel's signals. On re-activation the buffer is drained and
/// replayed.

const MAX_BUFFER = 100;
const buffers = new Map<string, AgentEvent[]>();

/// Get or create the buffer for a workspace.
export function getBuffer(workspace: string): AgentEvent[] {
  let buf = buffers.get(workspace);
  if (!buf) {
    buf = [];
    buffers.set(workspace, buf);
  }
  return buf;
}

/// Push an event into the workspace buffer. Maintains a FIFO cap of
/// MAX_BUFFER so long-dormant workspaces don't leak memory.
export function pushEvent(workspace: string, event: AgentEvent): void {
  const buf = getBuffer(workspace);
  buf.push(event);
  if (buf.length > MAX_BUFFER) {
    buf.splice(0, buf.length - MAX_BUFFER);
  }
}

/// Drain and return all buffered events, then delete the buffer entry.
/// Returns an empty array if no events were buffered.
export function drainBuffer(workspace: string): AgentEvent[] {
  const events = buffers.get(workspace) ?? [];
  if (events.length > 0) {
    buffers.delete(workspace);
  }
  return events;
}

/// Check if a workspace has buffered events.
export function hasBufferedEvents(workspace: string): boolean {
  const buf = buffers.get(workspace);
  return buf !== undefined && buf.length > 0;
}

/// Remove all buffered events for a workspace without returning them.
export function clearBuffer(workspace: string): void {
  buffers.delete(workspace);
}
