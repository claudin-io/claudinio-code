import { For, Show, type Component } from "solid-js";
import { Icon } from "./Icon";
import { t } from "../lib/grill-me";

export const EmptyState: Component<{
  recentProjects: string[];
  openRecent: (path: string) => void;
  openFolder: () => void;
}> = (props) => {
  return (
    <div class="flex h-full flex-col items-center justify-center px-6">
      <div class="mx-auto max-w-sm text-center">
        <div class="mb-4 text-ink-faint">
          <Icon name="brain" class="mx-auto h-10 w-10" />
        </div>
        <h2 class="mb-1 text-[15px] font-semibold text-ink">{t("empty.title")}</h2>
        <p class="mb-6 text-sm text-ink-muted">
          {t("empty.subtitle")}
        </p>
        <button
          onClick={props.openFolder}
          class="inline-flex items-center gap-2 rounded-md bg-accent px-4 py-2 text-sm font-medium text-accent-ink hover:bg-accent-hover"
        >
          <Icon name="folder-open" class="h-4 w-4" />
          {t("empty.openFolder")}
        </button>
      </div>

      <Show when={props.recentProjects.length > 0}>
        <div class="mt-10 w-full max-w-sm">
          <div class="mb-3 text-[11px] font-semibold uppercase tracking-wider text-ink-faint">
            {t("empty.recent")}
          </div>
          <div class="flex flex-col gap-0.5">
            <For each={props.recentProjects.slice(0, 5)}>
              {(proj) => (
                <button
                  onClick={() => props.openRecent(proj)}
                  class="flex items-center gap-2 rounded-md px-3 py-2 text-left text-sm text-ink-muted hover:bg-surface-2 hover:text-ink"
                >
                  <Icon name="folder" class="h-4 w-4 shrink-0 text-ink-faint" />
                  <div class="min-w-0">
                    <div class="truncate text-[13px]">{proj.split("/").pop()}</div>
                    <div class="truncate font-mono text-[11px] text-ink-faint">{proj}</div>
                  </div>
                </button>
              )}
            </For>
          </div>
        </div>
      </Show>
    </div>
  );
};
