import { createSignal, For, Show, onCleanup, onMount, batch, type Component } from "solid-js";
import { getContextWarning, type ContextWarningData } from "../lib/ipc";
import { Icon } from "./Icon";

export function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function formatTokens(n: number): string {
  if (n < 1000) return `${n}`;
  return `${(n / 1000).toFixed(1)}k`;
}

export function severityClass(agentsMdTokens: number): string {
  if (agentsMdTokens > 20_000) return "text-red-400";
  if (agentsMdTokens > 8_000) return "text-amber-400";
  return "text-ink-faint";
}

export function skillSeverityClass(skillTokens: number): string {
  if (skillTokens > 5_000) return "text-red-400";
  if (skillTokens > 2_000) return "text-amber-400";
  return "text-ink-muted";
}

export function showWarning(data: ContextWarningData | null): boolean {
  if (!data) return false;
  if (data.agentsMdTokens > 4_000) return true;
  if (data.agentsMdIssues > 0) return true;
  if (data.skillsTotalTokens > 10_000) return true;
  return false;
}

const ContextWarning: Component<{
  workspace: string;
}> = (props) => {
  const [data, setData] = createSignal<ContextWarningData | null>(null);
  const [open, setOpen] = createSignal(false);
  const [loading, setLoading] = createSignal(true);

  onMount(() => {
    setLoading(true);
    getContextWarning(props.workspace)
      .then((d) => {
        batch(() => {
          setData(d);
          setLoading(false);
        });
      })
      .catch(() => {
        batch(() => {
          setData(null);
          setLoading(false);
        });
      });
  });

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
  });

  const visible = () => showWarning(data());

  const agentColor = () => severityClass(data()!.agentsMdTokens);

  return (
    <Show when={!loading() && visible()}>
      <>
        <button
          onClick={() => setOpen(true)}
          class={`flex h-7 w-7 items-center justify-center rounded-md hover:bg-surface-2 ${agentColor()}`}
          title={"Context Budget"}
        >
          <Icon name="alert-triangle" class="h-4 w-4" />
        </button>

        <Show when={open()}>
          <div
            class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-[2px]"
            onClick={(e) => { if (e.target === e.currentTarget) setOpen(false); }}
          >
            <div class="flex w-[480px] max-w-[90vw] max-h-[85vh] flex-col rounded-lg bg-surface-1 p-5 shadow-modal">
              <div class="mb-4 flex items-center justify-between">
                <div class="flex items-center gap-2">
                  <Icon name="alert-triangle" class="h-5 w-5 text-amber-400" />
                  <h2 class="text-sm font-semibold text-ink">{"Context Budget"}</h2>
                </div>
                <button
                  onClick={() => setOpen(false)}
                  class="flex h-7 w-7 items-center justify-center rounded-md text-ink-muted hover:bg-surface-2 hover:text-ink"
                >
                  <Icon name="x" class="h-4 w-4" />
                </button>
              </div>

              <Show when={data()}>
                {(d) => (
                  <div class="flex flex-col gap-4 overflow-y-auto pr-1">
                    {/* AGENTS.md section */}
                    <Show when={d().agentsMdPath}>
                      <div>
                        <h3 class="mb-2 text-xs font-semibold uppercase tracking-wider text-ink-muted">
                          {"Injected File"}
                        </h3>
                        <div class="rounded-md border border-border-subtle bg-surface-0 p-3">
                          <div class="mb-2 flex items-center gap-2">
                            <Icon name="file-text" class="h-4 w-4 text-ink-muted" />
                            <code class="font-mono text-[12px] text-ink">{d().agentsMdPath}</code>
                          </div>
                          <div class="grid grid-cols-2 gap-2 text-[12px]">
                            <div>
                              <span class="text-ink-faint">{"Size"}:</span>{" "}
                              <span class="font-mono text-ink">{formatBytes(d().agentsMdSize)}</span>
                            </div>
                            <div>
                              <span class="text-ink-faint">{"Lines"}:</span>{" "}
                              <span class="font-mono text-ink">{d().agentsMdLines}</span>
                            </div>
                            <div>
                              <span class="text-ink-faint">{"Est. tokens"}:</span>{" "}
                              <span class={`font-mono ${severityClass(d().agentsMdTokens)}`}>
                                {formatTokens(d().agentsMdTokens)}
                              </span>
                            </div>
                            <div>
                              <span class="text-ink-faint">{"Issues"}:</span>{" "}
                              <span class={`font-mono ${d().agentsMdIssues > 0 ? "text-amber-400" : "text-ink-muted"}`}>
                                {d().agentsMdIssues}
                              </span>
                            </div>
                          </div>
                          <Show when={d().agentsMdIssues > 0}>
                            <div class="mt-2 rounded bg-amber-500/10 px-2 py-1.5 text-[11px] text-amber-400">
                              <Icon name="alert-triangle" class="mr-1 inline-block h-3 w-3 align-middle" />
                              {`${String(d().agentsMdIssues)} issues found — these directives consume context tokens every turn.`}
                            </div>
                          </Show>
                        </div>
                      </div>
                    </Show>

                    {/* Skills section */}
                    <Show when={d().skillsCount > 0}>
                      <div>
                        <h3 class="mb-2 text-xs font-semibold uppercase tracking-wider text-ink-muted">
                          {"Installed Skills"}
                        </h3>
                        <div class="rounded-md border border-border-subtle bg-surface-0 p-3">
                          <div class="mb-2 flex items-center justify-between">
                            <span class="text-[12px] text-ink-faint">{"Total skills"}</span>
                            <span class="font-mono text-[13px] text-ink">{d().skillsCount}</span>
                          </div>
                          <div class="mb-3 flex items-center justify-between">
                            <span class="text-[12px] text-ink-faint">{"Combined token cost"}</span>
                            <span class={`font-mono text-[13px] ${skillSeverityClass(d().skillsTotalTokens)}`}>
                              {formatTokens(d().skillsTotalTokens)}
                            </span>
                          </div>
                          {/* Per-skill breakdown */}
                          <div class="max-h-48 space-y-1 overflow-y-auto">
                            <For each={d().skillsBreakdown}>
                              {(sk) => (
                                <div class="flex items-center justify-between rounded px-2 py-1 text-[11px] hover:bg-surface-1">
                                  <div class="flex min-w-0 flex-1 items-center gap-1.5">
                                    <span
                                      classList={{
                                        "bg-red-400": sk.estimatedTokens > 5_000,
                                        "bg-amber-400": sk.estimatedTokens > 2_000 && sk.estimatedTokens <= 5_000,
                                        "bg-ink-faint": sk.estimatedTokens <= 2_000,
                                      }}
                                      class="h-1.5 w-1.5 shrink-0 rounded-full"
                                    />
                                    <span class="truncate text-ink-muted">{sk.name}</span>
                                  </div>
                                  <span class={`shrink-0 ml-2 font-mono ${skillSeverityClass(sk.estimatedTokens)}`}>
                                    {formatTokens(sk.estimatedTokens)}
                                  </span>
                                </div>
                              )}
                            </For>
                          </div>
                        </div>
                      </div>
                    </Show>

                    {/* Recommendation */}
                    <div class="rounded-md border border-accent/20 bg-accent/5 p-3">
                      <p class="text-[11px] leading-relaxed text-ink-muted">
                        {d().agentsMdPath
                          ? "💡 The AGENTS.md/CLAUDE.md file is injected at the start of every new chat. Large files consume significant context budget. Consider trimming unnecessary sections."
                          : "💡 Skills are injected into the system prompt as XML. Skills with large SKILL.md bodies increase the base context cost. Review if all skills are still needed."}
                      </p>
                    </div>
                  </div>
                )}
              </Show>
            </div>
          </div>
        </Show>
      </>
    </Show>
  );
};

export default ContextWarning;
