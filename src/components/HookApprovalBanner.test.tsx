import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { approveHooks, type HooksAwaitingApprovalData } from "../lib/ipc";
import HookApprovalBanner from "./HookApprovalBanner";

vi.mock("../lib/ipc", () => ({ approveHooks: vi.fn() }));
vi.mock("./Icon", () => ({ Icon: () => null }));

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
  vi.clearAllMocks();
});

const data: HooksAwaitingApprovalData = {
  workspace: "/ws",
  hash: "sha256:abc",
  count: 2,
  commands: ["/ws/.claude/guard.sh", "/plugins/brain/hooks/run.sh recall"],
};

function mount(onApproved = vi.fn(), onDismiss = vi.fn()): HTMLDivElement {
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => <HookApprovalBanner data={data} onApproved={onApproved} onDismiss={onDismiss} />,
    host,
  );
  return host;
}

describe("HookApprovalBanner", () => {
  it("lists every command verbatim", async () => {
    const el = mount();
    for (const cmd of data.commands) {
      expect(el.textContent).toContain(cmd);
    }
  });

  it("says the hooks have not run", async () => {
    const el = mount();
    expect(el.textContent).toContain("not run");
    expect(el.textContent).toContain("2 lifecycle hooks");
  });

  it("approves with the workspace and hash it was given", async () => {
    vi.mocked(approveHooks).mockResolvedValue({
      enabled: true,
      workspace: "/ws",
      trust: "trusted",
      fingerprint: "sha256:abc",
      approvedCommands: [],
      hooks: [],
      diagnostics: [],
    });
    const onApproved = vi.fn();
    const el = mount(onApproved);
    const allow = [...el.querySelectorAll("button")].find((b) =>
      b.textContent?.includes("Allow"),
    );
    allow?.click();
    await Promise.resolve();
    await Promise.resolve();
    expect(vi.mocked(approveHooks)).toHaveBeenCalledWith("/ws", "sha256:abc");
    expect(onApproved).toHaveBeenCalled();
  });

  it("dismissing does not approve anything", async () => {
    const onDismiss = vi.fn();
    const el = mount(vi.fn(), onDismiss);
    const not = [...el.querySelectorAll("button")].find((b) =>
      b.textContent?.includes("Not now"),
    );
    not?.click();
    expect(onDismiss).toHaveBeenCalled();
    expect(vi.mocked(approveHooks)).not.toHaveBeenCalled();
  });

  it("shows the failure rather than swallowing it", async () => {
    vi.mocked(approveHooks).mockRejectedValue(new Error("these hooks changed"));
    const el = mount();
    const allow = [...el.querySelectorAll("button")].find((b) =>
      b.textContent?.includes("Allow"),
    );
    allow?.click();
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    expect(el.textContent).toContain("these hooks changed");
  });
});
