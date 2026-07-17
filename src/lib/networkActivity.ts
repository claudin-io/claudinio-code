import { createSignal } from "solid-js";
import { listen } from "@tauri-apps/api/event";

// Mirrors NetOpView in src-tauri/src/net_activity.rs.
export interface NetOp {
  id: number;
  source:
    | "llm_stream"
    | "llm_classify"
    | "llm_one_shot"
    | "list_models"
    | "auth"
    | "skills_index"
    | "skill_fetch"
    | "embedding_model_download"
    | "web_search"
    | "mcp";
  detail: string;
  elapsedMs: number;
  bytes: number;
}

const [activeOps, setActiveOps] = createSignal<NetOp[]>([]);
const [lastFinishedOp, setLastFinishedOp] = createSignal<NetOp | null>(null);

let started = false;

/** Subscribe to the backend's network-activity events. Idempotent — the app
    calls this once; every NetworkIndicator instance reads the shared signals. */
export function startNetworkActivityListener() {
  if (started) return;
  started = true;
  void listen<NetOp[]>("network-activity", (event) => {
    const ops = event.payload;
    // Anything that vanished from the active list just finished — keep the
    // most recent one so the idle tooltip can say what ran last.
    for (const prev of activeOps()) {
      if (!ops.some((o) => o.id === prev.id)) setLastFinishedOp(prev);
    }
    setActiveOps(ops);
  });
}

export { activeOps, lastFinishedOp };

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m${s % 60}s`;
}
