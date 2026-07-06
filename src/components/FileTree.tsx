import { createResource, createSignal, For, Show, type Component } from "solid-js";
import { listDir, type DirEntry } from "../lib/ipc";

const TreeNode: Component<{
  entry: DirEntry;
  depth: number;
  onOpenFile: (path: string) => void;
  onOpenExternal: (path: string) => void;
  selectedPath: () => string | null;
}> = (props) => {
  const [expanded, setExpanded] = createSignal(false);
  const [children] = createResource(
    () => (props.entry.isDir && expanded() ? props.entry.path : null),
    (path) => listDir(path),
  );

  const handleClick = () => {
    if (props.entry.isDir) setExpanded(!expanded());
    else props.onOpenFile(props.entry.path);
  };

  const handleDblClick = () => {
    if (!props.entry.isDir) props.onOpenExternal(props.entry.path);
  };

  return (
    <>
      <button
        class="flex w-full items-center gap-1.5 truncate px-2 py-0.5 text-left text-sm hover:bg-surface-2"
        classList={{ "bg-surface-2 text-accent": props.selectedPath() === props.entry.path }}
        style={{ "padding-left": `${8 + props.depth * 14}px` }}
        onClick={handleClick}
        onDblClick={handleDblClick}
      >
        <span class="w-3 shrink-0 text-ink-muted">
          {props.entry.isDir ? (expanded() ? "▾" : "▸") : ""}
        </span>
        <span class="truncate">{props.entry.name}</span>
      </button>
      <Show when={expanded() && children()}>
        <For each={children()}>
          {(child) => (
            <TreeNode
              entry={child}
              depth={props.depth + 1}
              onOpenFile={props.onOpenFile}
              onOpenExternal={props.onOpenExternal}
              selectedPath={props.selectedPath}
            />
          )}
        </For>
      </Show>
    </>
  );
};

export const FileTree: Component<{
  root: string;
  onOpenFile: (path: string) => void;
  onOpenExternal: (path: string) => void;
  selectedPath: () => string | null;
}> = (props) => {
  const [entries] = createResource(() => props.root, listDir);

  return (
    <div class="h-full overflow-y-auto py-1">
      <div class="truncate px-2 pb-1 text-xs font-semibold uppercase tracking-wide text-ink-muted">
        {props.root.split("/").pop()}
      </div>
      <For each={entries()}>
        {(entry) => (
          <TreeNode
            entry={entry}
            depth={0}
            onOpenFile={props.onOpenFile}
            onOpenExternal={props.onOpenExternal}
            selectedPath={props.selectedPath}
          />
        )}
      </For>
    </div>
  );
};
