import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { getQualityConfig, setQualityConfig, type QualityInfo } from "../../lib/ipc";
import { SettingsQuality } from "./SettingsQuality";

vi.mock("../../lib/ipc", () => ({
  getQualityConfig: vi.fn(),
  setQualityConfig: vi.fn().mockResolvedValue(undefined),
}));

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
  vi.clearAllMocks();
});

const info = (over: Partial<QualityInfo["settings"]> = {}): QualityInfo => ({
  settings: {
    enabled: true,
    enforceOn: "goals",
    enforcedLayers: ["tests"],
    diffCoverageThreshold: 80,
    mutationScoreThreshold: 60,
    testCmd: "",
    coverageCmd: "",
    mutationCmd: "",
    featuresDir: "",
    gherkinCmd: "",
    maxComplexity: 0,
    testTimeoutSecs: 600,
    coverageTimeoutSecs: 900,
    mutationTimeoutSecs: 1800,
    ...over,
  },
  stacks: [
    {
      name: "rust",
      root: "/p/src-tauri",
      testCmd: "cargo test",
      coverageCmd: null,
      mutationCmd: "cargo mutants -o {artifact_dir} {in_diff}",
      gherkinCmd: null,
    },
  ],
});

async function mount(workspace: string | null, data: QualityInfo = info()) {
  vi.mocked(getQualityConfig).mockResolvedValue(data);
  const [ws] = createSignal(workspace);
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <SettingsQuality workspaceRoot={ws} />, host);
  // Let the load effect's promise settle.
  await Promise.resolve();
  await Promise.resolve();
  return host;
}

describe("SettingsQuality", () => {
  it("explains that the settings are per project when none is open", async () => {
    const el = await mount(null);
    expect(el.textContent).toContain("Open a project");
    expect(getQualityConfig).not.toHaveBeenCalled();
  });

  it("loads the open project's settings", async () => {
    const el = await mount("/p");
    expect(getQualityConfig).toHaveBeenCalledWith("/p");
    expect(el.textContent).toContain("Quality harness");
  });

  it("shows the detected command so the user can see what will run", async () => {
    const el = await mount("/p");
    expect(el.textContent).toContain("cargo test");
    expect(el.textContent).toContain("rust");
  });

  it("warns when nothing was detected", async () => {
    const el = await mount("/p", { ...info(), stacks: [] });
    expect(el.textContent).toContain("No test-capable project detected");
  });

  it("saves immediately when a setting changes", async () => {
    const el = await mount("/p");
    const select = el.querySelector("select") as HTMLSelectElement;
    select.value = "code_change";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await Promise.resolve();
    expect(setQualityConfig).toHaveBeenCalledWith(
      "/p",
      expect.objectContaining({ enforceOn: "code_change" }),
    );
  });

  it("describes what each trigger actually does", async () => {
    const el = await mount("/p");
    expect(el.textContent).toContain("nothing is verified unless you tag");
    const select = el.querySelector("select") as HTMLSelectElement;
    select.value = "code_change";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await Promise.resolve();
    expect(el.textContent).toContain("touched a file a test could execute");
  });

  it("warns when no layer is enforced, since that silently stops blocking", async () => {
    const el = await mount("/p", info({ enforcedLayers: [] }));
    expect(el.textContent).toContain("never block a finish");
  });

  it("only offers the coverage threshold when coverage is enforced", async () => {
    const withoutCoverage = await mount("/p", info({ enforcedLayers: ["tests"] }));
    expect(withoutCoverage.querySelector('input[type="range"]')).toBeNull();

    dispose?.();
    host?.remove();
    const withCoverage = await mount("/p", info({ enforcedLayers: ["tests", "coverage"] }));
    expect(withCoverage.querySelector('input[type="range"]')).not.toBeNull();
    expect(withCoverage.textContent).toContain("80%");
  });

  it("toggling a layer keeps the others", async () => {
    const el = await mount("/p", info({ enforcedLayers: ["tests"] }));
    const boxes = Array.from(el.querySelectorAll('input[type="checkbox"]'));
    // [0] is the master switch, [1] tests, [2] coverage, [3] mutation.
    const coverage = boxes[2] as HTMLInputElement;
    coverage.checked = true;
    coverage.dispatchEvent(new Event("change", { bubbles: true }));
    await Promise.resolve();
    expect(setQualityConfig).toHaveBeenCalledWith(
      "/p",
      expect.objectContaining({ enforcedLayers: ["tests", "coverage"] }),
    );
  });

  it("warns that mutation is slow when it is enforced", async () => {
    // The user must learn the cost before a session stalls for minutes.
    const off = await mount("/p", info({ enforcedLayers: ["tests"] }));
    expect(off.textContent).not.toContain("reruns once per mutant");

    dispose?.();
    host?.remove();
    const on = await mount("/p", info({ enforcedLayers: ["tests", "mutation"] }));
    expect(on.textContent).toContain("reruns once per mutant");
    expect(on.textContent).toContain("60%");
  });

  it("explains that specs are write-protected when the layer is on", async () => {
    const el = await mount("/p", info({ enforcedLayers: ["tests", "gherkin"] }));
    expect(el.textContent).toContain("cannot edit");
    expect(el.textContent).toContain("Specs directory");
    // Without a runner, the honest outcome must be stated up front.
    expect(el.textContent).toContain("never as passing");
  });

  it("says complexity only reports until a budget is set", async () => {
    // A heuristic that blocks by default would block for the wrong reason.
    const el = await mount("/p", info({ enforcedLayers: ["tests", "metrics"] }));
    expect(el.textContent).toContain("report only");
    expect(el.textContent).toContain("not canonical McCabe");
  });

  it("surfaces a write failure instead of pretending it saved", async () => {
    const el = await mount("/p");
    vi.mocked(setQualityConfig).mockRejectedValueOnce(new Error("read-only file system"));
    const select = el.querySelector("select") as HTMLSelectElement;
    select.value = "code_change";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await Promise.resolve();
    await Promise.resolve();
    expect(el.textContent).toContain("read-only file system");
    expect(el.textContent).not.toContain("Saved to this project");
  });

  it("hides the detail controls when the harness is switched off", async () => {
    const el = await mount("/p", info({ enabled: false }));
    expect(el.querySelector("select")).toBeNull();
    expect(el.textContent).toContain("Quality harness");
  });
});
