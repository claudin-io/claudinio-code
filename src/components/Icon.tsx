import type { Component } from "solid-js";

const PATHS: Record<string, string> = {
  folder:
    "M2 6a2 2 0 0 1 2-2h5l2 2h5a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6Z",
  "folder-open":
    "M2 6a2 2 0 0 1 2-2h5l2 2h6a2 2 0 0 1 2 2v0a2 2 0 0 1-1.22 1.84l-5.8 2.32A5.5 5.5 0 0 1 6.96 13H4a2 2 0 0 1-2-2V6Z",
  file: "M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2zM6 20V4h7v5h5v11H6Z",
  "chevron-right": "M9 6l6 6-6 6",
  "chevron-down": "M6 9l6 6 6-6",
  settings:
    "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2zM12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z",
  send: "M3.4 2.1A2 2 0 0 0 1.7 4.9l4.2 8.4a1 1 0 0 0 .9.55H18a1 1 0 0 0 .9-1.45l-6-12a1 1 0 0 0-1.8 0l-2.4 4.8L4.7 3.3a2 2 0 0 0-1.3-1.2Z",
  check: "M20 6L9 17l-5-5",
  x: "M18 6L6 18M6 6l12 12",
  search:
    "M10 3a7 7 0 1 0 4.95 11.95l4.25 4.25a1 1 0 0 0 1.42-1.42l-4.25-4.25A7 7 0 0 0 10 3z",
  terminal:
    "M4 20h16a2 2 0 0 0 2-2V6a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2ZM8 10l-2 2 2 2m4 0h4",
  pencil:
    "M17 3a2.85 2.85 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3Z",
  brain:
    "M9.5 2A2.5 2.5 0 0 1 12 4.5v1.56a3.5 3.5 0 0 1 4.64 4.28 3.5 3.5 0 0 1 1.6 5.1 3.5 3.5 0 0 1-1.13 5.09A3.5 3.5 0 0 1 8.5 18.5a3.5 3.5 0 0 1-2.1-6.34 3.5 3.5 0 0 1 .64-6.37A2.5 2.5 0 0 1 9.5 2Z",
  loader:
    "M21 12a9 9 0 1 1-6.22-8.55",
  "arrow-left": "M19 12H5m7 7l-7-7 7-7",
  "alert-circle":
    "M12 22c5.523 0 10-4.477 10-10S17.523 2 12 2 2 6.477 2 12s4.477 10 10 10zm0-14v4m0 4h.01",
  plus: "M12 5v14m-7-7h14",
  clock: "M12 22c5.523 0 10-4.477 10-10S17.523 2 12 2 2 6.477 2 12s4.477 10 10 10zM12 6v6l4 2",
  layers: "M12 2 2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5",
  "external-link": "M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6m4-3h6v6m-5 8L21 3",
};

export type IconName = keyof typeof PATHS;

export const Icon: Component<{ name: IconName; class?: string }> = (props) => {
  const path = PATHS[props.name];
  if (!path) return null;
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.75"
      stroke-linecap="round"
      stroke-linejoin="round"
      class={props.class}
    >
      <path d={path} />
    </svg>
  );
};

export function toolIcon(name: string): IconName {
  if (name === "read_file") return "file";
  if (name === "grep") return "search";
  if (name === "edit_file") return "pencil";
  if (name === "list_dir") return "folder";
  return "terminal";
}
