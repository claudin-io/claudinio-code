import { createStore } from "solid-js/store";
import { walkDirectory } from "./ipc";

const [fileIndexMap, setFileIndexMap] = createStore<Record<string, string[]>>({});

export { fileIndexMap };

export async function loadFileIndex(workspacePath: string): Promise<void> {
  try {
    const entries = await walkDirectory(workspacePath);
    const paths = entries
      .filter((e) => e.path.length > 0)
      .map((e) => e.path);
    setFileIndexMap(workspacePath, paths);
  } catch {
    setFileIndexMap(workspacePath, []);
  }
}
