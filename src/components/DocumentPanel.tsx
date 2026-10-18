import { createEffect, createSignal, For, on, Show, type Component } from "solid-js";
import { getDocument, type DocView } from "../lib/ipc";
import { publishedDocument, requestSubagent } from "../lib/documentStore";
import { createVisibilityAwareInterval } from "../lib/visibility";
import { Icon } from "./Icon";

/** The chip's words for a section's status. */
export function docStatusLabel(status: string): string {
  if (status === "done") return "Done";
  if (status === "doing") return "Doing";
  if (status === "human") return "Edited by you";
  return "Todo";
}

/** The chip's colours, the Task Panel's own, plus the accent for the user's edits. */
function chipClass(status: string): string {
  if (status === "done") return "bg-success/15 text-success";
  if (status === "doing") return "bg-amber-500/15 text-amber-500";
  if (status === "human") return "bg-accent/15 text-accent";
  return "bg-ink-faint/10 text-ink-muted";
}

/**
 * The session's living document (lean-harness Phase 4b), shown where the
 * Task Panel lives when the session has one: the title, and a row per
 * section with its status and who holds it. A row opens the document in the
 * editor at that section; a subagent's chip opens its timeline.
 *
 * Loaded from the backend on mount and every few seconds (a person may edit
 * the file in another editor between runs), and replaced by every outline a
 * run publishes (`DocumentUpdated`, through `lib/documentStore`).
 */
export const DocumentPanel: Component<{
  workspace: string;
  onOpen: (doc: DocView, line: number) => void;
  /**
   * Called with the session's document (null without one) each time it is
   * loaded or a run publishes it: App routes every save of that file through
   * `save_document` (`isLinkedDocument`) and hides the task list.
   */
  onDocumentChange?: (doc: DocView | null) => void;
}> = (props) => {
  const [doc, setDoc] = createSignal<DocView | null>(null);

  const load = async () => {
    try {
      const d = await getDocument(props.workspace);
      setDoc(d);
      props.onDocumentChange?.(d);
    } catch {
      // no workspace open or a transient error: keep what is shown
    }
  };

  createVisibilityAwareInterval(load, 5000);

  // A run's outline replaces what is shown; an invalidation (the session
  // changed) asks the backend again.
  createEffect(
    on(
      () => publishedDocument(props.workspace),
      (p) => {
        if (p.version === 0) return;
        if (p.doc) {
          setDoc(p.doc);
          props.onDocumentChange?.(p.doc);
        } else {
          void load();
        }
      },
      { defer: true },
    ),
  );

  return (
    <Show when={doc()}>
      {(d) => (
        <div data-document-panel class="flex h-full w-full flex-col bg-surface-1">
          <div class="flex items-center gap-2 border-b border-border-subtle px-3 py-2">
            <Icon name="file-text" class="h-3.5 w-3.5 shrink-0 text-ink-faint" />
            <button
              data-doc-title
              class="min-w-0 flex-1 truncate text-left text-[12px] font-medium text-ink hover:underline"
              title={d().path}
              onClick={() => props.onOpen(d(), 1)}
            >
              {d().title || d().path}
            </button>
          </div>
          <div class="flex-1 overflow-y-auto py-1">
            <For each={d().sections}>
              {(s) => (
                <div data-doc-row class="flex items-center gap-2 px-3 py-1 hover:bg-surface-2">
                  <button
                    data-doc-open
                    class="flex min-w-0 flex-1 items-center gap-2 text-left"
                    title={`${s.id} · ${s.topic} — open in the editor`}
                    onClick={() => props.onOpen(d(), s.line)}
                  >
                    <span
                      data-doc-status
                      class={`shrink-0 whitespace-nowrap rounded-full px-1.5 py-0.5 text-[10px] font-medium ${chipClass(s.status)}`}
                    >
                      {docStatusLabel(s.status)}
                    </span>
                    <span
                      class="min-w-0 flex-1 truncate text-[12px]"
                      classList={{
                        "text-ink": s.status !== "done",
                        "text-ink-muted": s.status === "done",
                      }}
                    >
                      {s.topic}
                    </span>
                  </button>
                  <Show when={s.owner}>
                    {(owner) => (
                      <Show
                        when={owner().startsWith("sub:")}
                        fallback={
                          <span data-doc-owner class="shrink-0 font-mono text-[10px] text-ink-faint">
                            {owner()}
                          </span>
                        }
                      >
                        <button
                          data-doc-owner
                          class="max-w-[6rem] shrink-0 truncate font-mono text-[10px] text-accent hover:underline"
                          title={`Open ${owner()}'s timeline`}
                          onClick={() => requestSubagent(props.workspace, owner())}
                        >
                          {owner().slice(4)}
                        </button>
                      </Show>
                    )}
                  </Show>
                </div>
              )}
            </For>
          </div>
        </div>
      )}
    </Show>
  );
};
