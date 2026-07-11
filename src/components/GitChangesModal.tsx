import { createSignal, createEffect, createMemo, For, Show, type Component } from "solid-js";
import {
  gitStatus, gitFileDiff, gitStageFile, gitUnstageFile, gitDiscardFile,
  gitStageHunk, gitUnstageHunk, gitStageAll, gitUnstageAll,
  type GitStatus, type ChangedFile
} from "../lib/ipc";
import { Icon } from "./Icon";
import { t } from "../lib/grill-me";
import FileEditorModal, { detectLanguage, getBasename, getRelativePath } from "./FileEditorModal";
import CommitPushModal from "./CommitPushModal";

const diffStyles = `
  .diff-pre { font-family: 'JetBrains Mono', monospace; font-size: 11px; line-height: 1.5; padding: 8px 16px; margin: 0; white-space: pre; overflow-x: auto; max-height: 24rem; }
  .diff-add { background-color: rgba(34, 197, 94, 0.1); color: rgb(74, 222, 128); }
  .diff-del { background-color: rgba(239, 68, 68, 0.1); color: rgb(248, 113, 113); }
  .diff-hunk { color: rgb(96, 165, 250); }
  .diff-ctx { color: inherit; }
`;

interface Hunk {
  header: string;
  lines: string[];
}

function parseHunks(diff: string): Hunk[] {
  if (!diff) return [];
  const lines = diff.split('\n');
  const hunks: Hunk[] = [];
  let current: string[] | null = null;
  for (const line of lines) {
    if (line.startsWith('@@')) {
      if (current && current.length > 0) {
        hunks.push({ header: current[0], lines: current });
      }
      current = [line];
    } else if (current) {
      current.push(line);
    }
  }
  if (current && current.length > 0) {
    hunks.push({ header: current[0], lines: current });
  }
  return hunks;
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

function joinPath(root: string, rel: string): string {
  const normRoot = root.replace(/\/+$/, '');
  const normRel = rel.replace(/^\/+/, '');
  return normRoot + '/' + normRel;
}

function statusLabel(status: string): string {
  switch (status) {
    case "M": return t("git.modified");
    case "A": return t("git.added");
    case "D": return t("git.deleted");
    case "?": return t("git.untracked");
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

interface FileRowProps {
  file: ChangedFile;
  diff: string;
  expanded: boolean;
  onToggle: () => void;
  onStage: () => void;
  onUnstage: () => void;
  onDiscard: () => void;
  onEdit: () => void;
  hunks: Hunk[];
  onStageHunk: (hunkText: string) => void;
  onUnstageHunk: (hunkText: string) => void;
  stagedHunks: Set<string>;
}

const FileRow: Component<FileRowProps> = (props) => {
  const additions = props.file.additions;
  const deletions = props.file.deletions;

  return (
    <div class="border-b border-border-subtle last:border-b-0">
      <div class="flex items-center gap-1 px-3 py-1 hover:bg-surface-2">
        <button
          onClick={props.onToggle}
          class="flex items-center gap-1 min-w-0 flex-1 text-left"
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
          <span class="mr-2 shrink-0 text-[11px] font-medium tabular-nums">
            <span class="text-green-500">+{additions}</span>
            <span class="mx-0.5 text-ink-faint">/</span>
            <span class="text-red-500">−{deletions}</span>
          </span>
        </button>
        <button
          onClick={props.file.staged ? props.onUnstage : props.onStage}
          class="flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 text-[10px] text-ink-faint hover:bg-surface-3"
        >
          <Icon name="git-commit" class="h-3 w-3" stroke />
          {props.file.staged ? t("git.unstageFile") || "Unstage" : t("git.stageFile") || "Stage"}
        </button>
        <button
          onClick={props.onDiscard}
          class="flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 text-[10px] text-ink-faint hover:bg-surface-3"
        >
          <Icon name="trash" class="h-3 w-3" />
          {t("git.discard") || "Discard"}
        </button>
        <button
          onClick={props.onEdit}
          class="flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 text-[10px] text-ink-faint hover:bg-surface-3"
        >
          <Icon name="edit" class="h-3 w-3" />
          {t("git.edit") || "Edit"}
        </button>
      </div>
      <Show when={props.expanded && props.diff}>
        <div class="border-t border-border-subtle bg-surface-0">
          <For each={props.hunks}>
            {(hunk) => {
              const hunkKey = `${props.file.path}:${hunk.header}`;
              const isStaged = props.stagedHunks.has(hunkKey);
              return (
                <div>
                  <div class="flex items-center gap-2 px-4 py-0.5">
                    <span class="text-[10px] font-mono text-blue-400">{hunk.header}</span>
                    <button
                      onClick={() => isStaged ? props.onUnstageHunk(hunk.lines.join("\n")) : props.onStageHunk(hunk.lines.join("\n"))}
                      class="ml-auto rounded px-1.5 py-0.5 text-[10px] text-ink-faint hover:bg-surface-3"
                    >
                      {isStaged ? t("git.unstageHunk") || "Unstage Hunk" : t("git.stageHunk") || "Stage Hunk"}
                    </button>
                  </div>
                  <pre class="diff-pre" innerHTML={renderDiff(hunk.lines.join("\n"))} />
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
};

export const GitChangesModal: Component<{
  workspace: string;
  open: boolean;
  onClose: () => void;
  onCommitPush: (stagedOnly: boolean) => void;
}> = (props) => {
  const [status, setStatus] = createSignal<GitStatus | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [expandedFiles, setExpandedFiles] = createSignal<Set<string>>(new Set<string>());
  const [diffs, setDiffs] = createSignal<Record<string, string>>({});
  const [committing, setCommitting] = createSignal(false);
  const [editorFile, setEditorFile] = createSignal<string | null>(null);
  const [stagedHunks, setStagedHunks] = createSignal<Set<string>>(new Set<string>());

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

  const loadDiff = async (path: string, staged?: boolean) => {
    const key = `${path}:${staged ? "staged" : "unstaged"}`;
    if (diffs()[key]) return;
    try {
      const diff = await gitFileDiff(props.workspace, path, staged);
      setDiffs((prev) => ({ ...prev, [key]: diff }));
    } catch (e) {
      setDiffs((prev) => ({ ...prev, [key]: "" }));
    }
  };

  const handleStageFile = async (path: string) => {
    try {
      await gitStageFile(props.workspace, path);
      await fetchStatus();
    } catch (e) {
      // ignore
    }
  };

  const handleUnstageFile = async (path: string) => {
    try {
      await gitUnstageFile(props.workspace, path);
      await fetchStatus();
    } catch (e) {
      // ignore
    }
  };

  const handleDiscardFile = async (path: string) => {
    if (!window.confirm(t("git.discardConfirm", path))) return;
    try {
      await gitDiscardFile(props.workspace, path);
      await fetchStatus();
    } catch (e) {
      // ignore
    }
  };

  const handleStageAll = async () => {
    try {
      await gitStageAll(props.workspace);
      await fetchStatus();
    } catch (e) {
      // ignore
    }
  };

  const handleUnstageAll = async () => {
    try {
      await gitUnstageAll(props.workspace);
      await fetchStatus();
    } catch (e) {
      // ignore
    }
  };

  const handleStageHunk = async (path: string, hunkText: string) => {
    try {
      await gitStageHunk(props.workspace, path, hunkText);
      const header = hunkText.split("\n")[0];
      setStagedHunks((prev) => new Set(prev).add(`${path}:${header}`));
      await fetchStatus();
    } catch (e) {
      // ignore
    }
  };

  const handleUnstageHunk = async (path: string, hunkText: string) => {
    try {
      await gitUnstageHunk(props.workspace, path, hunkText);
      const header = hunkText.split("\n")[0];
      setStagedHunks((prev) => {
        const next = new Set(prev);
        next.delete(`${path}:${header}`);
        return next;
      });
      await fetchStatus();
    } catch (e) {
      // ignore
    }
  };

  createEffect(() => {
    if (props.open) {
      setLoading(true);
      setExpandedFiles(new Set<string>());
      setDiffs({});
      setStagedHunks(new Set<string>());
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
      // Expand immediately, then load diffs in background
      loadDiff(path, false);
      loadDiff(path, true);
    }
    setExpandedFiles(next);
  };

  const handleRefresh = () => {
    setLoading(true);
    setExpandedFiles(new Set<string>());
    setDiffs({});
    setStagedHunks(new Set<string>());
    fetchStatus();
  };

  const s = status;

  const files = createMemo(() => s()?.files ?? []);
  const stagedFiles = createMemo(() => files().filter((f) => f.staged));
  const unstagedFiles = createMemo(() => files().filter((f) => !f.staged));
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
        <div class="flex w-[75vw] max-h-[85vh] flex-col rounded-lg border border-border-subtle bg-surface-1 shadow-xl">
          {/* Header */}
          <div class="flex items-center justify-between border-b border-border-subtle px-4 py-3 shrink-0">
            <div class="flex items-center gap-1.5">
              <Icon name="diff" class="h-4 w-4 text-ink-muted" />
              <h2 class="text-[13px] font-semibold text-ink">
                {t("git.modalTitle")} <span class="text-ink-faint">({fileCount()})</span>
              </h2>
            </div>
            <button
              onClick={props.onClose}
              class="rounded p-1 text-ink-faint hover:bg-surface-2 hover:text-ink"
            >
              <Icon name="x" class="h-4 w-4" />
            </button>
          </div>

          {/* Body: two panels */}
          <div class="flex flex-1 min-h-0 overflow-hidden">
            {/* LEFT PANEL */}
            <div class="flex w-[55%] flex-col border-r border-border-subtle">
              {/* Stage/Unstage All bar */}
              <div class="flex items-center gap-1 px-3 py-1.5 border-b border-border-subtle shrink-0">
                <button
                  onClick={handleStageAll}
                  class="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-ink-muted hover:bg-surface-2"
                >
                  <Icon name="git-commit" class="h-3 w-3" stroke /> {t("git.stageAll")}
                </button>
                <button
                  onClick={handleUnstageAll}
                  class="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-ink-muted hover:bg-surface-2"
                >
                  <Icon name="git-commit" class="h-3 w-3" stroke /> {t("git.unstageAll")}
                </button>
              </div>
              {/* File list */}
              <div class="flex-1 overflow-y-auto">
                {/* Staged files section */}
                <Show when={stagedFiles().length > 0}>
                  <div class="px-3 py-1 text-[10px] font-semibold uppercase tracking-wider text-green-500 bg-surface-0/50">
                    {t("git.staged")}
                  </div>
                  <For each={stagedFiles()}>
                    {(f) => {
                      const unstagedKey = `${f.path}:unstaged`;
                      const stagedKey = `${f.path}:staged`;
                      const combinedDiff = [diffs()[unstagedKey], diffs()[stagedKey]]
                        .filter(Boolean)
                        .join("\n");
                      const hunks = parseHunks(combinedDiff);
                      return (
                        <FileRow
                          file={f}
                          diff={combinedDiff}
                          expanded={expandedFiles().has(f.path)}
                          onToggle={() => toggleFile(f.path)}
                          onStage={() => handleStageFile(f.path)}
                          onUnstage={() => handleUnstageFile(f.path)}
                          onDiscard={() => handleDiscardFile(f.path)}
                          onEdit={() => setEditorFile(joinPath(props.workspace, f.path))}
                          hunks={hunks}
                          onStageHunk={(hunkText: string) => handleStageHunk(f.path, hunkText)}
                          onUnstageHunk={(hunkText: string) => handleUnstageHunk(f.path, hunkText)}
                          stagedHunks={stagedHunks()}
                        />
                      );
                    }}
                  </For>
                </Show>
                {/* Unstaged files section */}
                <Show when={unstagedFiles().length > 0}>
                  <div class="px-3 py-1 text-[10px] font-semibold uppercase tracking-wider text-ink-faint bg-surface-0/50">
                    {t("git.unstaged")}
                  </div>
                  <For each={unstagedFiles()}>
                    {(f) => {
                      const unstagedKey = `${f.path}:unstaged`;
                      const stagedKey = `${f.path}:staged`;
                      const combinedDiff = [diffs()[unstagedKey], diffs()[stagedKey]]
                        .filter(Boolean)
                        .join("\n");
                      const hunks = parseHunks(combinedDiff);
                      return (
                        <FileRow
                          file={f}
                          diff={combinedDiff}
                          expanded={expandedFiles().has(f.path)}
                          onToggle={() => toggleFile(f.path)}
                          onStage={() => handleStageFile(f.path)}
                          onUnstage={() => handleUnstageFile(f.path)}
                          onDiscard={() => handleDiscardFile(f.path)}
                          onEdit={() => setEditorFile(joinPath(props.workspace, f.path))}
                          hunks={hunks}
                          onStageHunk={(hunkText: string) => handleStageHunk(f.path, hunkText)}
                          onUnstageHunk={(hunkText: string) => handleUnstageHunk(f.path, hunkText)}
                          stagedHunks={stagedHunks()}
                        />
                      );
                    }}
                  </For>
                </Show>
                <Show when={fileCount() === 0 && !loading()}>
                  <div class="flex items-center justify-center py-8 text-[12px] text-ink-faint">
                    {t("git.noChanges")}
                  </div>
                </Show>
                <Show when={loading()}>
                  <div class="flex items-center justify-center py-8 text-[12px] text-ink-faint">
                    Loading...
                  </div>
                </Show>
              </div>
            </div>

            {/* RIGHT PANEL */}
            <div class="flex w-[45%] flex-col">
              <div class="flex items-center gap-2 px-4 py-3 border-b border-border-subtle shrink-0">
                <Icon name="git-commit" class="h-4 w-4 text-ink-muted" stroke />
                <span class="text-[13px] font-semibold text-ink">{t("git.commit") || "Commit"}</span>
              </div>
              <div class="flex-1 overflow-y-auto p-4">
                <div class="space-y-4">
                  <div>
                    <div class="text-[12px] font-medium text-ink">
                      {t("git.stagedCount", stagedFiles().length)}
                    </div>
                    <div class="mt-2 space-y-1">
                      <For each={stagedFiles()}>
                        {(f) => (
                          <div class="truncate font-mono text-[11px] text-ink-muted">{f.path}</div>
                        )}
                      </For>
                    </div>
                    <Show when={stagedFiles().length === 0}>
                      <p class="text-[11px] text-ink-faint italic mt-2">{t("git.noStaged")}</p>
                    </Show>
                  </div>
                  <div class="flex flex-col gap-2">
                    <button
                      onClick={() => props.onCommitPush(true)}
                      disabled={stagedFiles().length === 0}
                      class="flex items-center justify-center gap-1.5 rounded-md bg-accent px-3 py-2 text-[12px] font-medium text-accent-ink hover:bg-accent-hover disabled:opacity-30"
                    >
                      <Icon name="git-commit" class="h-4 w-4" stroke />
                      {t("git.commitStaged")}
                    </button>
                    <button
                      onClick={() => props.onCommitPush(false)}
                      class="flex items-center justify-center gap-1.5 rounded-md border border-accent/30 px-3 py-2 text-[12px] font-medium text-accent hover:bg-accent/10"
                    >
                      <Icon name="git-commit" class="h-4 w-4" stroke />
                      {t("git.commitAll")}
                    </button>
                  </div>
                </div>
              </div>
              <div class="flex items-center justify-between border-t border-border-subtle px-4 py-2 shrink-0">
                <button
                  onClick={handleRefresh}
                  disabled={loading()}
                  class="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-ink-muted hover:bg-surface-2 disabled:opacity-30"
                >
                  <Icon name="refresh" class="h-3 w-3" />
                  {t("git.refresh")}
                </button>
              </div>
            </div>
          </div>
        </div>

        {/* FileEditorModal */}
        <Show when={editorFile() !== null && props.open}>
          <FileEditorModal
            filePath={editorFile()!}
            rootPath={props.workspace}
            onClose={() => setEditorFile(null)}
          />
        </Show>
      </div>
    </Show>
  );
};
