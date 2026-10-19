import { createSignal, onMount, type Component } from "solid-js";
import { getConfig, setConfig } from "../../lib/ipc";

/**
 * Project memory (lean-harness Phase 9, decision D6). On by default: the
 * first run of each new session reads the workspace's AGENTS.md (or
 * CLAUDE.md) and the `.claudinio/MEMORY.md` lessons finalize_plan wrote.
 * Off sends neither; the memory file is still written.
 */
export const SettingsProjectMemory: Component = () => {
  const [on, setOn] = createSignal(true);

  onMount(async () => {
    const cfg = await getConfig();
    setOn(cfg.projectMemory ?? true);
  });

  const toggle = async (value: boolean) => {
    setOn(value);
    await setConfig({ projectMemory: value });
  };

  return (
    <div class="mt-6 border-t border-border-subtle pt-4">
      <label class="mb-1 flex cursor-pointer items-center gap-2">
        <input
          type="checkbox"
          data-project-memory
          checked={on()}
          onChange={(e) => toggle(e.currentTarget.checked)}
          class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
        />
        <span class="text-sm font-medium text-ink">{"Project memory"}</span>
      </label>
      <p class="text-[11px] text-ink-faint">
        {
          "The first message of each new session also sends the workspace's AGENTS.md (or CLAUDE.md) and the lessons in .claudinio/MEMORY.md, which finalize_plan adds to. Takes effect from the next new session."
        }
      </p>
    </div>
  );
};
