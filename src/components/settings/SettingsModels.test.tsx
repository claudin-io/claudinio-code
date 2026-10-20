import { describe, it, expect, afterEach } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { SettingsModels } from "./SettingsModels";

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
});

function mount(opts: { workspaceFields?: string[]; easterEgg?: boolean } = {}) {
  const [model, setModel] = createSignal("claudinio");
  const num = () => createSignal<number | null>(null);
  const [maxRounds, setMaxRounds] = num();
  const [subMaxRounds, setSubMaxRounds] = num();
  const [cycles, setCycles] = num();
  const [stalls, setStalls] = num();
  const [parallel, setParallel] = createSignal(4);
  const [handoff, setHandoff] = createSignal(180_000);
  const [url, setUrl] = createSignal("");
  const [key, setKey] = createSignal("");
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <SettingsModels
        model={model}
        setModel={setModel}
        maxParallelAgents={parallel}
        setMaxParallelAgents={setParallel}
        maxRounds={maxRounds}
        setMaxRounds={setMaxRounds}
        subMaxRounds={subMaxRounds}
        setSubMaxRounds={setSubMaxRounds}
        maxGoldenCycles={cycles}
        setMaxGoldenCycles={setCycles}
        maxGoldenStalls={stalls}
        setMaxGoldenStalls={setStalls}
        handoffTokens={handoff}
        setHandoffTokens={setHandoff}
        workspaceConfigFields={() => new Set(opts.workspaceFields ?? [])}
        easterEggActive={() => opts.easterEgg ?? false}
        overrideBaseUrl={url}
        setOverrideBaseUrl={setUrl}
        overrideApiKey={key}
        setOverrideApiKey={setKey}
        modelGroups={() => []}
      />
    ),
    host,
  );
  return { el: host, model };
}

// One timeline (single-timeline plan): a session runs on one model, so
// Settings offers one model, with nothing named after the modes that left.
describe("SettingsModels", () => {
  it("offers one model and no Brain or Builder model", () => {
    const { el } = mount();
    const labels = Array.from(el.querySelectorAll("label")).map((l) => l.textContent);
    expect(labels).toContain("Model");
    expect(el.textContent).not.toMatch(/Brain|Builder/);
  });

  it("edits the session model by hand when typed", () => {
    const { el, model } = mount({ easterEgg: true });
    const input = el.querySelector<HTMLInputElement>("input[data-model-input]")!;
    input.value = "stub/m";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    expect(model()).toBe("stub/m");
  });

  it("marks the model as set by the workspace when its .claudinio.json does", () => {
    const { el } = mount({ workspaceFields: ["model"] });
    expect(el.textContent).toContain("Workspace");
  });
});
