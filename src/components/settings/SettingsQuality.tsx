import { Component, createEffect, createSignal, For, Show, type Accessor } from "solid-js";
import {
  getQualityConfig,
  setQualityConfig,
  type DetectedStack,
  type EnforceOn,
  type QualitySettings,
} from "../../lib/ipc";

interface SettingsQualityProps {
  workspaceRoot: Accessor<string | null>;
}

/**
 * The quality harness settings.
 *
 * Unlike the other tabs, these live in the *workspace's* `.claudinio.json`
 * rather than the global config, because the commands and thresholds belong to
 * the project. That is also why this tab saves on change instead of waiting for
 * the panel's Save button, which writes the global file — one button appearing
 * to cover two different files is worse than two obvious behaviours.
 */
export const SettingsQuality: Component<SettingsQualityProps> = (props) => {
  const [settings, setSettings] = createSignal<QualitySettings | null>(null);
  const [stacks, setStacks] = createSignal<DetectedStack[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [savedAt, setSavedAt] = createSignal<number | null>(null);

  createEffect(() => {
    const ws = props.workspaceRoot();
    if (!ws) {
      setSettings(null);
      setStacks([]);
      return;
    }
    void (async () => {
      try {
        const info = await getQualityConfig(ws);
        setSettings(info.settings);
        setStacks(info.stacks);
        setError(null);
      } catch (e) {
        setError(String(e));
      }
    })();
  });

  const persist = (next: QualitySettings) => {
    setSettings(next);
    const ws = props.workspaceRoot();
    if (!ws) return;
    void (async () => {
      try {
        await setQualityConfig(ws, next);
        setError(null);
        setSavedAt(Date.now());
      } catch (e) {
        setError(String(e));
      }
    })();
  };

  const patch = (fields: Partial<QualitySettings>) => {
    const current = settings();
    if (!current) return;
    persist({ ...current, ...fields });
  };

  const enforces = (layer: string) => settings()?.enforcedLayers.includes(layer) ?? false;

  const toggleLayer = (layer: string, on: boolean) => {
    const current = settings();
    if (!current) return;
    const next = on
      ? [...current.enforcedLayers, layer]
      : current.enforcedLayers.filter((l) => l !== layer);
    patch({ enforcedLayers: next });
  };

  return (
    <>
      <Show
        when={props.workspaceRoot()}
        fallback={
          <p class="text-sm text-ink-muted">
            {"Open a project to configure its quality harness — these settings are stored in the project's .claudinio.json, not globally."}
          </p>
        }
      >
        <Show when={settings()} fallback={<p class="text-sm text-ink-muted">{"Loading…"}</p>}>
          {(cfg) => (
            <>
              <label class="mb-1 flex cursor-pointer items-center gap-2">
                <input
                  type="checkbox"
                  checked={cfg().enabled}
                  onChange={(e) => patch({ enabled: e.currentTarget.checked })}
                  class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
                />
                <span class="text-sm font-medium text-ink">{"Quality harness"}</span>
                <span class="rounded border border-accent/40 bg-accent/10 px-1.5 py-px text-[10px] font-medium text-accent">
                  {"Workspace"}
                </span>
              </label>
              <p class="mb-4 text-[11px] text-ink-faint">
                {"Runs this project's own tests and scores them mechanically. A goal cannot be marked done on the agent's word — only on a passing run that still matches the files on disk."}
              </p>

              <Show when={cfg().enabled}>
                <label class="mb-1 block text-xs text-ink-muted">{"Verify at the end of"}</label>
                <select
                  value={cfg().enforceOn}
                  onChange={(e) => patch({ enforceOn: e.currentTarget.value as EnforceOn })}
                  class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
                >
                  <option value="code_change">{"Any run that changed code"}</option>
                  <option value="goals">{"Runs with a tagged <goal> only"}</option>
                </select>
                <p class="mb-4 text-[11px] text-ink-faint">
                  {cfg().enforceOn === "goals"
                    ? "Narrow: nothing is verified unless you tag a <goal>. Everything else finishes unchecked — use this when the suite is too slow to sit through on every change."
                    : "Any session that touched a file a test could execute gets verified once, at the end. Read-only, prose and asset changes cost nothing."}
                </p>

                <label class="mb-1 block text-xs text-ink-muted">{"Layers that block a finish"}</label>
                <label class="mb-1 flex cursor-pointer items-center gap-2">
                  <input
                    type="checkbox"
                    checked={enforces("tests")}
                    onChange={(e) => toggleLayer("tests", e.currentTarget.checked)}
                    class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
                  />
                  <span class="text-sm text-ink">{"Tests"}</span>
                  <span class="text-[11px] text-ink-faint">{"The suite must pass."}</span>
                </label>
                <label class="mb-1 flex cursor-pointer items-center gap-2">
                  <input
                    type="checkbox"
                    checked={enforces("coverage")}
                    onChange={(e) => toggleLayer("coverage", e.currentTarget.checked)}
                    class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
                  />
                  <span class="text-sm text-ink">{"Changed-line coverage"}</span>
                  <span class="text-[11px] text-ink-faint">{"Catches code nothing tests."}</span>
                </label>
                <label class="mb-1 flex cursor-pointer items-center gap-2">
                  <input
                    type="checkbox"
                    checked={enforces("mutation")}
                    onChange={(e) => toggleLayer("mutation", e.currentTarget.checked)}
                    class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
                  />
                  <span class="text-sm text-ink">{"Mutation"}</span>
                  <span class="text-[11px] text-ink-faint">{"Catches tests that assert nothing."}</span>
                </label>
                <Show when={enforces("mutation")}>
                  <p class="mb-1 text-[11px] text-amber-500">
                    {"Slow by design: the suite reruns once per mutant. It is scoped to your changed lines and only runs at the end of a session, never mid-task."}
                  </p>
                </Show>
                <label class="mb-1 flex cursor-pointer items-center gap-2">
                  <input
                    type="checkbox"
                    checked={enforces("gherkin")}
                    onChange={(e) => toggleLayer("gherkin", e.currentTarget.checked)}
                    class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
                  />
                  <span class="text-sm text-ink">{"Specification (Gherkin)"}</span>
                  <span class="text-[11px] text-ink-faint">{"Catches building the wrong thing."}</span>
                </label>
                <Show when={enforces("gherkin")}>
                  <p class="mb-1 text-[11px] text-ink-faint">
                    {"Runs your .feature files. They are the one input the agent cannot edit — it must ask you to change a scenario rather than rewrite it."}
                  </p>
                </Show>
                <label class="mb-1 flex cursor-pointer items-center gap-2">
                  <input
                    type="checkbox"
                    checked={enforces("metrics")}
                    onChange={(e) => toggleLayer("metrics", e.currentTarget.checked)}
                    class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
                  />
                  <span class="text-sm text-ink">{"Complexity"}</span>
                  <span class="text-[11px] text-ink-faint">{"Catches the codebase rotting."}</span>
                </label>
                <Show when={enforces("metrics")}>
                  <label class="mb-1 mt-2 block text-xs text-ink-muted">
                    {"Complexity budget per changed function"}
                  </label>
                  <div class="mb-1 flex items-center gap-3">
                    <input
                      type="range"
                      min="0"
                      max="40"
                      step="1"
                      value={cfg().maxComplexity}
                      onInput={(e) => patch({ maxComplexity: Number(e.currentTarget.value) })}
                      class="flex-1 accent-accent"
                    />
                    <span class="w-20 text-right font-mono text-sm text-ink">
                      {cfg().maxComplexity === 0 ? "report only" : String(cfg().maxComplexity)}
                    </span>
                  </div>
                  <p class="mb-4 text-[11px] text-ink-faint">
                    {"A consistent heuristic, not canonical McCabe — good for comparing a function against itself over time. Leave at 0 to record the trend without ever blocking."}
                  </p>
                </Show>
                <Show when={cfg().enforcedLayers.length === 0}>
                  <p class="mb-4 text-[11px] text-amber-500">
                    {"Nothing is enforced: the harness will report results but never block a finish."}
                  </p>
                </Show>

                <Show when={enforces("coverage")}>
                  <label class="mb-1 mt-3 block text-xs text-ink-muted">
                    {"Minimum coverage of changed lines"}
                  </label>
                  <div class="mb-1 flex items-center gap-3">
                    <input
                      type="range"
                      min="0"
                      max="100"
                      step="5"
                      value={cfg().diffCoverageThreshold}
                      onInput={(e) =>
                        patch({ diffCoverageThreshold: Number(e.currentTarget.value) })
                      }
                      class="flex-1 accent-accent"
                    />
                    <span class="w-12 text-right font-mono text-sm text-ink">
                      {`${String(cfg().diffCoverageThreshold)}%`}
                    </span>
                  </div>
                  <p class="mb-4 text-[11px] text-ink-faint">
                    {"Measured only on the lines this run changed, so a legacy project's history never blocks a small fix. Requires cargo-llvm-cov for Rust; vitest and jest need no extra tooling."}
                  </p>
                </Show>

                <Show when={enforces("mutation")}>
                  <label class="mb-1 mt-3 block text-xs text-ink-muted">
                    {"Minimum mutants caught"}
                  </label>
                  <div class="mb-1 flex items-center gap-3">
                    <input
                      type="range"
                      min="0"
                      max="100"
                      step="5"
                      value={cfg().mutationScoreThreshold}
                      onInput={(e) =>
                        patch({ mutationScoreThreshold: Number(e.currentTarget.value) })
                      }
                      class="flex-1 accent-accent"
                    />
                    <span class="w-12 text-right font-mono text-sm text-ink">
                      {`${String(cfg().mutationScoreThreshold)}%`}
                    </span>
                  </div>
                  <p class="mb-4 text-[11px] text-ink-faint">
                    {"Requires cargo-mutants for Rust. Other stacks need a mutation command below."}
                  </p>
                </Show>

                <label class="mb-1 mt-3 block text-xs text-ink-muted">{"Test command override"}</label>
                <input
                  type="text"
                  value={cfg().testCmd}
                  onChange={(e) => patch({ testCmd: e.currentTarget.value })}
                  placeholder={stacks()[0]?.testCmd ?? "detected automatically"}
                  class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 font-mono text-xs text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
                />
                <p class="mb-4 text-[11px] text-ink-faint">
                  {"Leave blank to use what was detected. Setting this replaces detection entirely, so it must run every suite you care about."}
                </p>

                <label class="mb-1 block text-xs text-ink-muted">{"Coverage command override"}</label>
                <input
                  type="text"
                  value={cfg().coverageCmd}
                  onChange={(e) => patch({ coverageCmd: e.currentTarget.value })}
                  placeholder={stacks()[0]?.coverageCmd ?? "detected automatically"}
                  class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 font-mono text-xs text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
                />
                <p class="mb-4 text-[11px] text-ink-faint">
                  {"Must write an lcov report to {artifact_dir}/lcov.info."}
                </p>

                <label class="mb-1 block text-xs text-ink-muted">{"Mutation command override"}</label>
                <input
                  type="text"
                  value={cfg().mutationCmd}
                  onChange={(e) => patch({ mutationCmd: e.currentTarget.value })}
                  placeholder={stacks()[0]?.mutationCmd ?? "detected automatically"}
                  class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 font-mono text-xs text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
                />
                <p class="mb-4 text-[11px] text-ink-faint">
                  {"Takes {artifact_dir} and {in_diff}. Must write a mutants.out directory into {artifact_dir}."}
                </p>
              </Show>

                <Show when={enforces("gherkin")}>
                  <label class="mb-1 block text-xs text-ink-muted">{"Specs directory"}</label>
                  <input
                    type="text"
                    value={cfg().featuresDir}
                    onChange={(e) => patch({ featuresDir: e.currentTarget.value })}
                    placeholder="features"
                    class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 font-mono text-xs text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
                  />
                  <p class="mb-4 text-[11px] text-ink-faint">
                    {"Everything under it is write-protected from the agent."}
                  </p>

                  <label class="mb-1 block text-xs text-ink-muted">{"BDD runner command"}</label>
                  <input
                    type="text"
                    value={cfg().gherkinCmd}
                    onChange={(e) => patch({ gherkinCmd: e.currentTarget.value })}
                    placeholder={stacks()[0]?.gherkinCmd ?? "cucumber-js is detected; cucumber-rs runs under cargo test"}
                    class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 font-mono text-xs text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
                  />
                  <p class="mb-4 text-[11px] text-ink-faint">
                    {"Without a runner the scenarios are reported as unmeasured — never as passing."}
                  </p>
                </Show>

              <div class="mt-4 border-t border-border-subtle pt-3">
                <p class="mb-2 text-xs text-ink-muted">{"Detected in this project"}</p>
                <Show
                  when={stacks().length > 0}
                  fallback={
                    <p class="text-[11px] text-amber-500">
                      {"No test-capable project detected. Set a test command above, or the harness will report that it could not verify anything."}
                    </p>
                  }
                >
                  <For each={stacks()}>
                    {(stack) => (
                      <div class="mb-2">
                        <p class="text-[11px] font-medium text-ink">{stack.name}</p>
                        <p class="font-mono text-[10px] break-all text-ink-faint">
                          {stack.testCmd}
                        </p>
                      </div>
                    )}
                  </For>
                </Show>
              </div>

              <Show when={error()}>
                <p class="mt-3 text-[11px] text-red-500">{error()}</p>
              </Show>
              <Show when={savedAt() && !error()}>
                <p class="mt-3 text-[11px] text-ink-faint">
                  {"Saved to this project's .claudinio.json — changes apply to the next run."}
                </p>
              </Show>
            </>
          )}
        </Show>
      </Show>
    </>
  );
};
