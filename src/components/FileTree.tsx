import { createResource, createSignal, For, Show, type Component } from "solid-js";
import { listDir, openInTerminal, copyPath, type DirEntry } from "../lib/ipc";
import { openPath } from "@tauri-apps/plugin-opener";
import { ContextMenu, type ContextMenuItem } from "./ContextMenu";
import { platform } from "../lib/platform";

const TreeNode: Component<{
  entry: DirEntry;
  depth: number;
  onOpenFile: (path: string) => void;
  onDblClickFile: (path: string) => void;
  onOpenExternal: (path: string) => void;
  selectedPath: () => string | null;
  onContextMenu: (x: number, y: number, path: string, isDir: boolean) => void;
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
    if (!props.entry.isDir) props.onDblClickFile(props.entry.path);
  };

  return (
    <>
      <button
        class="flex w-full items-center gap-1.5 truncate px-2 py-0.5 text-left text-sm hover:bg-surface-2"
        classList={{ "bg-surface-2 text-accent": props.selectedPath() === props.entry.path }}
        style={{ "padding-left": `${8 + props.depth * 14}px` }}
        onClick={handleClick}
        onDblClick={handleDblClick}
        onContextMenu={(e) => {
          e.preventDefault();
          props.onContextMenu(e.clientX, e.clientY, props.entry.path, props.entry.isDir);
        }}
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
              onDblClickFile={props.onDblClickFile}
              onOpenExternal={props.onOpenExternal}
              selectedPath={props.selectedPath}
              onContextMenu={props.onContextMenu}
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
  onDblClickFile: (path: string) => void;
  onOpenExternal: (path: string) => void;
  selectedPath: () => string | null;
}> = (props) => {
  const [entries] = createResource(() => props.root, listDir);
  const [contextPos, setContextPos] = createSignal<{ x: number; y: number; path: string; isDir: boolean } | null>(null);

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
            onDblClickFile={props.onDblClickFile}
            onOpenExternal={props.onOpenExternal}
            selectedPath={props.selectedPath}
            onContextMenu={(x, y, path, isDir) => setContextPos({ x, y, path, isDir })}
          />
        )}
      </For>
      <Show when={contextPos()}>
        {(pos) => {
          const items = (): ContextMenuItem[] => {
            const revealPath = pos().isDir ? pos().path : pos().path.substring(0, pos().path.lastIndexOf("/"));
            return [
              {
                label: platform() === 'mac' ? 'Reveal in Finder' : platform() === 'win' ? 'Show in Explorer' : 'Open in File Manager',
                icon: 'external-link',
                action: () => openPath(revealPath).catch(console.error),
              },
              {
                label: 'Open in Terminal',
                icon: 'terminal',
                action: () => openInTerminal(pos().path).catch(console.error),
              },
              {
                label: 'Copy Path',
                icon: 'file-text',
                action: () => copyPath(pos().path),
              },
            ];
          };
          return (
            <ContextMenu
              x={pos().x}
              y={pos().y}
              items={items()}
              onClose={() => setContextPos(null)}
            />
          );
        }}
      </Show>
    </div>
  );
};
