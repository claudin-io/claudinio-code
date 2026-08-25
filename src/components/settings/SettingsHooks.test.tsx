import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { approveHooks, listHooks, type HookInfo, type HooksInfo } from "../../lib/ipc";
import { SettingsHooks } from "./SettingsHooks";

vi.mock("../../lib/ipc", () => ({
  listHooks: vi.fn(),
  approveHooks: vi.fn(),
  revokeHooks: vi.fn(),
  reloadHooks: vi.fn(),
  setHooksEnabled: vi.fn(),
  testHook: vi.fn(),
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

const hook = (over: Partial<HookInfo> = {}): HookInfo => ({
  event: "PreToolUse",
  matcher: "Edit|Write",
  command: "/plugins/brain/hooks/run.sh",
  args: ["recall"],
  display: "/plugins/brain/hooks/run.sh recall",
  timeoutSecs: 10,
  statusMessage: null,
  source: "plugin: claudinio-brain",
  sourceKind: "plugin",
  alsoFrom: [],
  hits: ["edit_file"],
  matcherValid: true,
  ...over,
});

const info = (over: Partial<HooksInfo> = {}): HooksInfo => ({
  enabled: true,
  workspace: "/ws",
  trust: "pending",
  fingerprint: "sha256:abc",
  approvedCommands: [],
  hooks: [hook()],
  diagnostics: [],
  ...over,
});

async function mount(data: HooksInfo, workspace: string | null = "/ws"): Promise<HTMLDivElement> {
  vi.mocked(listHooks).mockResolvedValue(data);
  host = document.createElement("div");
  document.body.appendChild(host);
  const [ws] = createSignal<string | null>(workspace);
  dispose = render(() => <SettingsHooks workspaceRoot={ws} />, host);
  await Promise.resolve();
  await Promise.resolve();
  return host;
}

describe("SettingsHooks", () => {
  it("shows the resolved command, not the placeholder that produced it", async () => {
    // Approving `${CLAUDE_PLUGIN_ROOT}/run.sh` without seeing where it resolves
    // to is not consent.
    const el = await mount(info());
    expect(el.textContent).toContain("/plugins/brain/hooks/run.sh recall");
    expect(el.textContent).not.toContain("${CLAUDE_PLUGIN_ROOT}");
  });

  it("says nothing has run while approval is pending, and offers to approve", async () => {
    const el = await mount(info());
    expect(el.textContent).toContain("Waiting for your approval");
    const button = [...el.querySelectorAll("button")].find((b) =>
      b.textContent?.includes("Allow these"),
    );
    expect(button).toBeTruthy();
  });

  it("approves with the fingerprint it displayed", async () => {
    vi.mocked(approveHooks).mockResolvedValue(info({ trust: "trusted" }));
    const el = await mount(info());
    const button = [...el.querySelectorAll("button")].find((b) =>
      b.textContent?.includes("Allow these"),
    );
    button?.click();
    await Promise.resolve();
    expect(vi.mocked(approveHooks)).toHaveBeenCalledWith("/ws", "sha256:abc");
  });

  it("names a matcher that will never fire", async () => {
    // The characteristic hook bug: it installs cleanly and matches nothing.
    const el = await mount(info({ hooks: [hook({ hits: [], matcher: "Nonexistent" })] }));
    expect(el.textContent).toContain("selects nothing");
  });

  it("names a matcher that is not a valid regex", async () => {
    const el = await mount(info({ hooks: [hook({ matcherValid: false, matcher: "Edit(" })] }));
    expect(el.textContent).toContain("not a valid regex");
  });

  it("says which tools a working matcher will hit", async () => {
    const el = await mount(info());
    expect(el.textContent).toContain("fires on: edit_file");
  });

  it("says a duplicate declaration runs once", async () => {
    const el = await mount(
      info({ hooks: [hook({ alsoFrom: ["/ws/.claudinio.json"] })] }),
    );
    expect(el.textContent).toContain("runs once");
  });

  it("distinguishes a changed set from an unapproved one", async () => {
    const el = await mount(info({ trust: "changed" }));
    expect(el.textContent).toContain("changed since you approved");
  });

  it("surfaces configuration diagnostics", async () => {
    const el = await mount(
      info({ diagnostics: [{ source: "~/.claude/settings.json", message: "not valid JSON" }] }),
    );
    expect(el.textContent).toContain("not valid JSON");
    expect(el.textContent).toContain("~/.claude/settings.json");
  });

  it("asks for a project rather than listing nothing", async () => {
    const el = await mount(info(), null);
    expect(el.textContent).toContain("Open a project");
  });
});
