import { createSignal, createEffect, createMemo, For, Show, type Component } from "solid-js";
import { gitStatus, gitFileDiff, type GitStatus, type ChangedFile } from "../lib/ipc";
import { Icon } from "./Icon";

const diffStyles = `
  .diff-pre { font-family: 'JetBrains Mono', monospace; font-size: 11px; line-height: 1.5; padding: 8px 16px; margin: 0; white-space: pre; overflow-x: auto; max-height: 24rem; }
  .diff-add { background-color: rgba(34, 197, 94, 0.1); color: rgb(74, 222, 128); }
  .diff-del { background-color: rgba(239, 68, 68, 0.1); color: rgb(248, 113, 113); }
  .diff-hunk { color: rgb(96, 165, 250); }
  .diff-ctx { color: inherit; }
`;

interface FileRowProps {
  file: ChangedFile;
  diff: string;
  expanded: boolean;
  onToggle: () => void;
}

function statusColor(status: string): string {
  switch (status) {
    case "M": return "text-yellow-500";
    case "A": return "text-green-500";
    case "D": return "text-red-500";
    case "?": return "text-ink-muted";
    default: return "text-ink-muted";
  }
}

function statusLabel(status: string): string {
  switch (status) {
    case "M": return "Modified";
    case "A": return "Added";
    case "D": return "Deleted";
    case "?": return "Untracked";
    default: return status;
  }
}

function renderDiff(diff: string): string {
  if (!diff) return "";
  const lines = diff.split("\n");
  let html = "";
  for (const line of lines) {
    const escaped = line
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
    if (line.startsWith("+")) {
      html += `<div class="diff-add">${escaped}</div>`;
    } else if (line.startsWith("-")) {
      html += `<div class="diff-del">${escaped}</div>`;
    } else if (line.startsWith("@@")) {
      html += `<div class="diff-hunk">${escaped}</div>`;
    } else {
      html += `<div class="diff-ctx">${escaped}</div>`;
    }
  }
  return html;
}

const FileRow: Component<FileRowProps> = (props) => {
  const additions = props.file.additions;
  const deletions = props.file.deletions;

  return (
    <div class="border-b border-border-subtle last:border-b-0">
      <button
        onClick={props.onToggle}
        class="flex w-full items-center gap-2 px-4 py-1.5 text-left hover:bg-surface-2"
      >
        <Icon
          name={props.expanded ? "chevron-down" : "chevron-right"}
          class="h-3 w-3 shrink-0 text-ink-faint"
        />
        <span class={`text-[12px] font-medium ${statusColor(props.file.status)}`}>
          {props.file.status}
        </span>
        <span class="flex-1 truncate font-mono text-[12px] text-ink">
          {props.file.path}
        </span>
        <span class="mr-2 text-[11px] font-medium tabular-nums">
          <span class="text-green-500">+{additions}</span>
          <span class="mx-0.5 text-ink-faint">/</span>
          <span class="text-red-500">−{deletions}</span>
        </span>
        <span class="text-[10px] text-ink-faint">{statusLabel(props.file.status)}</span>
      </button>
      <Show when={props.expanded && props.diff}>
        <div class="overflow-x-auto border-t border-border-subtle bg-surface-0 text-left">
          <pre
            class="diff-pre"
            innerHTML={renderDiff(props.diff)}
          />
        </div>
      </Show>
    </div>
  );
};

export const GitChangesModal: Component<{
  workspace: string;
  open: boolean;
  onClose: () => void;
  onCommitPush: () => void;
}> = (props) => {
  const [status, setStatus] = createSignal<GitStatus | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [committing, setCommitting] = createSignal(false);
  const [expandedFiles, setExpandedFiles] = createSignal<Set<string>>(new Set<string>());
  const [diffs, setDiffs] = createSignal<Record<string, string>>({});

  const fetchStatus = async () => {
    try {
      const s = await gitStatus(props.workspace);
      setStatus(s);
    } catch (e) {
      setStatus(null);
    } finally {
      setLoading(false);
    }
  };

  const loadDiff = async (path: string) => {
    if (diffs()[path]) return;
    try {
      const diff = await gitFileDiff(props.workspace, path);
      setDiffs((prev) => ({ ...prev, [path]: diff }));
    } catch (e) {
      setDiffs((prev) => ({ ...prev, [path]: "" }));
    }
  };

  createEffect(() => {
    if (props.open) {
      fetchStatus();
    }
  });

  const toggleFile = async (path: string) => {
    const current: Set<string> = expandedFiles();
    const next = new Set<string>(current);
    if (next.has(path)) {
      next.delete(path);
    } else {
      next.add(path);
      // Load diff on first expand
      loadDiff(path);
    }
    setExpandedFiles(next);
  };

  const handleRefresh = () => {
    setLoading(true);
    setExpandedFiles(new Set<string>());
    setDiffs({});
    fetchStatus();
  };

  const handleCommitPush = () => {
    setCommitting(true);
    props.onCommitPush();
  };

  const s = status;
  const committing_ = committing;

  const files = createMemo(() => s()?.files ?? []);
  const fileCount = createMemo(() => files().length);

  return (
    <Show when={props.open}>
      <style>{diffStyles}</style>
      <div
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
        onClick={(e) => {
          if (e.target === e.currentTarget) props.onClose();
        }}
      >
        <div class="flex w-[60vw] max-h-[80vh] flex-col rounded-lg border border-border-subtle bg-surface-1 shadow-xl">
          {/* Header */}
          <div class="flex items-center justify-between border-b border-border-subtle px-4 py-3">
          <div class="flex items-center gap-1.5">
              <Icon name="diff" class="h-4 w-4 text-ink-muted" />
              <h2 class="text-[13px] font-semibold text-ink">
                {"Changes"} <span class="text-ink-faint">({fileCount()})</span>
              </h2>
            </div>
            <button
              onClick={props.onClose}
              class="rounded p-1 text-ink-faint hover:bg-surface-2 hover:text-ink"
            >
              <Icon name="x" class="h-4 w-4" />
            </button>
          </div>

          {/* File list */}
          <div class="flex-1 overflow-y-auto">
            <Show
              when={!loading()}
              fallback={
                <div class="flex items-center justify-center py-8 text-[12px] text-ink-faint">
                  Loading...
                </div>
              }
            >
              <Show
                when={fileCount() > 0}
                fallback={
                  <div class="flex items-center justify-center py-8 text-[12px] text-ink-faint">
                    {"0 changes"}
                  </div>
                }
              >
                <div>
                  <For each={files()}>
                    {(f) => (
                      <FileRow
                        file={f}
                        diff={diffs()[f.path] ?? ""}
                        expanded={expandedFiles().has(f.path)}
                        onToggle={() => toggleFile(f.path)}
                      />
                    )}
                  </For>
                </div>
              </Show>
            </Show>
          </div>

          {/* Footer */}
          <div class="flex items-center justify-between border-t border-border-subtle px-4 py-2">
            <button
              onClick={handleRefresh}
              disabled={loading()}
              class="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-ink-muted hover:bg-surface-2 disabled:opacity-30"
            >
              <Icon name="refresh" class="h-3 w-3" />
              {"Refresh"}
            </button>
            <button
              onClick={handleCommitPush}
              disabled={committing_() || fileCount() === 0}
              class="flex items-center gap-1.5 rounded-md bg-accent px-3 py-1.5 text-[12px] font-medium text-accent-ink hover:bg-accent-hover disabled:opacity-30"
            >
              <Icon name="git-commit" class="h-4 w-4" stroke />
              {"Commit & Push"}
            </button>
          </div>
        </div>
      </div>
    </Show>
  );
};
