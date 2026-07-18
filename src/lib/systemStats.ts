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
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)}GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(0)}MB`;
  if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(0)}KB`;
  return `${bytes}B`;
}
