import { describe, it, expect, vi, afterEach } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { SettingsRemote } from "./SettingsRemote";
import type { Pairing, RemotePolicyView, RemoteStoredPolicy } from "../../lib/ipc";

/// SolidJS needs a macrotask to settle in jsdom.
const flush = () => new Promise((r) => setTimeout(r, 10));

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
});

const allow: RemoteStoredPolicy["allow"] = {
  send_message: true,
  steer: false,
  interrupt: true,
  set_mode: false,
  approve_edit: true,
  approve_bash: "allowlist",
  read_attachment: false,
  export_file: false,
};

function policyView(over: Partial<RemotePolicyView> = {}): RemotePolicyView {
  return {
    path: "/Users/v/.config/claudinio-code/remote-policy.json",
    active: true,
    inertBecause: null,
    effective: { ...allow, expires_at: null },
    workspaces: ["/Users/v/work"],
    bashDenylist: ["rm -rf"],
    stored: {
      enabled: true,
      expires_at: null,
      workspaces: ["/Users/v/work"],
      idle_disconnect_minutes: 30,
      allow,
      bash_remote_denylist_extra: [],
      max_unattended_minutes: 60,
    },
    ...over,
  };
}

function pairing(over: Partial<Pairing> = {}): Pairing {
  return {
    peer_key: "aa".repeat(32),
    label: "Safari on iPhone",
    paired_at: 1_800_000_000_000,
    expires_at: null,
    ...over,
  };
}

interface Handlers {
  onCreateIdentity?: () => void;
  onRevoke?: (key: string) => void;
  onUnrevoke?: (key: string) => void;
  onRename?: (key: string, label: string) => void;
}

async function mount(
  opts: {
    deviceKey?: string | null;
    policy?: RemotePolicyView | null;
    pairings?: Pairing[];
    revoked?: string[];
    error?: string | null;
  } = {},
  handlers: Handlers = {},
) {
  host = document.createElement("div");
  document.body.appendChild(host);

  // `??` would turn an explicit null back into the default, which is exactly
  // the case one of these tests is about.
  const [deviceKey] = createSignal(
    opts.deviceKey === undefined ? "bb".repeat(32) : opts.deviceKey,
  );
  const [policy] = createSignal(opts.policy === undefined ? policyView() : opts.policy);
  const [pairings] = createSignal(opts.pairings ?? []);
  const [revoked] = createSignal(opts.revoked ?? []);
  const [error] = createSignal(opts.error ?? null);
  const [busy] = createSignal(false);

  dispose = render(
    () => (
      <SettingsRemote
        deviceKey={deviceKey}
        policy={policy}
        pairings={pairings}
        revoked={revoked}
        error={error}
        busy={busy}
        onCreateIdentity={handlers.onCreateIdentity ?? (() => {})}
        onRevoke={handlers.onRevoke ?? (() => {})}
        onUnrevoke={handlers.onUnrevoke ?? (() => {})}
        onRename={handlers.onRename ?? (() => {})}
      />
    ),
    host,
  );
  await flush();
  return host;
}

const button = (root: HTMLElement, label: string) =>
  Array.from(root.querySelectorAll("button")).find((b) => b.textContent?.includes(label));

describe("SettingsRemote", () => {
  /// Generating a long-term key is a deliberate act. Opening a settings screen
  /// must not create one.
  it("offers to create a device key rather than having made one", async () => {
    const onCreateIdentity = vi.fn();
    const root = await mount({ deviceKey: null }, { onCreateIdentity });

    const create = button(root, "Create device key");
    expect(create).toBeTruthy();

    create!.click();
    expect(onCreateIdentity).toHaveBeenCalledOnce();
  });

  /// The key is public by design, and the panel should say so — otherwise someone
  /// will treat it as a secret and be reluctant to read it aloud while pairing.
  it("shows the device key as public", async () => {
    const root = await mount({ deviceKey: "cd".repeat(32) });

    expect(root.textContent).toContain("Public by design");
    // Shown in fragments: 64 hex characters cannot be compared against a screen.
    expect(root.textContent).toContain("cdcdcdcd");
    expect(root.textContent).not.toContain("cd".repeat(32));
  });

  /// The badge is all this component says about the policy now — the reason lives
  /// in RemotePolicyEditor, which is the thing that can act on it.
  it("badges the policy as off when nothing is granted", async () => {
    const root = await mount({
      policy: policyView({ active: false, inertBecause: "the grant has expired" }),
    });

    expect(root.textContent).toContain("Off");
  });

  it("badges the policy as active when it is", async () => {
    const root = await mount();
    expect(root.textContent).toContain("Active");
  });

  /// The slots are why this file did not have to learn to write a policy or run a
  /// pairing. Worth a test, because a silently unrendered slot is a blank panel.
  it("renders what the host slots into it", async () => {
    host = document.createElement("div");
    document.body.appendChild(host);
    const [policy] = createSignal<RemotePolicyView | null>(policyView());
    const [none] = createSignal<Pairing[]>([]);
    const [empty] = createSignal<string[]>([]);
    const [nothing] = createSignal<string | null>(null);
    const [busy] = createSignal(false);
    const [key] = createSignal<string | null>("bb".repeat(32));

    dispose = render(
      () => (
        <SettingsRemote
          deviceKey={key}
          policy={policy}
          pairings={none}
          revoked={empty}
          error={nothing}
          busy={busy}
          onCreateIdentity={() => {}}
          onRevoke={() => {}}
          onUnrevoke={() => {}}
          onRename={() => {}}
          policyEditor={<span>{"THE EDITOR"}</span>}
          pairing={<span>{"THE PAIRING FLOW"}</span>}
        />
      ),
      host,
    );
    await flush();

    expect(host.textContent).toContain("THE EDITOR");
    expect(host.textContent).toContain("THE PAIRING FLOW");
  });

  it("lists a paired browser by the name it was given", async () => {
    const root = await mount({ pairings: [pairing({ label: "Firefox on the laptop" })] });

    expect(root.textContent).toContain("Firefox on the laptop");
    expect(root.textContent).toContain("aaaaaaaa");
  });

  it("revokes the pairing it was asked to revoke", async () => {
    const onRevoke = vi.fn();
    const mine = pairing({ peer_key: "11".repeat(32), label: "mine" });
    const root = await mount({ pairings: [mine] }, { onRevoke });

    button(root, "Revoke")!.click();

    expect(onRevoke).toHaveBeenCalledWith("11".repeat(32));
  });

  /// The promise that makes revocation worth having: it does not depend on the
  /// relay. The panel says so, because a user who thinks otherwise will not trust
  /// it in the moment they need it.
  it("says revocation does not need the relay", async () => {
    const root = await mount({ pairings: [pairing()] });

    expect(root.textContent).toContain("without asking anyone");
    expect(root.textContent).toContain("relay is down");
  });

  it("offers to allow a revoked key again, and says why it is still listed", async () => {
    const onUnrevoke = vi.fn();
    const root = await mount({ revoked: ["22".repeat(32)] }, { onUnrevoke });

    expect(root.textContent).toContain("Revoked");
    expect(root.textContent).toContain("pairing itself back in");

    button(root, "Allow again")!.click();
    expect(onUnrevoke).toHaveBeenCalledWith("22".repeat(32));
  });

  it("says pairing needs physical access when there is nothing paired", async () => {
    const root = await mount({ pairings: [] });

    expect(root.textContent).toContain("physical access");
  });

  /// Renaming happens in place: `window.prompt` is unstyled, blocking, and on a
  /// phone it covers the list the name is being compared against.
  it("renames in place, without a prompt dialog", async () => {
    const onRename = vi.fn();
    const mine = pairing({ peer_key: "44".repeat(32), label: "old name" });
    const root = await mount({ pairings: [mine] }, { onRename });

    button(root, "Rename")!.click();
    await flush();

    const input = root.querySelector("input") as HTMLInputElement;
    expect(input).toBeTruthy();
    expect(input.value).toBe("old name");

    input.value = "iPhone in the kitchen";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();

    expect(onRename).toHaveBeenCalledWith("44".repeat(32), "iPhone in the kitchen");
    expect(root.querySelector("input")).toBeNull();
  });

  /// A browser named with 64 hex characters and nothing else is unidentifiable,
  /// which defeats the point of the list.
  it("refuses to rename a pairing to nothing", async () => {
    const onRename = vi.fn();
    const root = await mount({ pairings: [pairing()] }, { onRename });

    button(root, "Rename")!.click();
    await flush();

    const input = root.querySelector("input") as HTMLInputElement;
    input.value = "   ";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();

    expect(onRename).not.toHaveBeenCalled();
  });

  it("abandons a rename on Escape", async () => {
    const onRename = vi.fn();
    const root = await mount({ pairings: [pairing()] }, { onRename });

    button(root, "Rename")!.click();
    await flush();

    const input = root.querySelector("input") as HTMLInputElement;
    input.value = "typed then thought better of it";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await flush();

    expect(onRename).not.toHaveBeenCalled();
    expect(root.querySelector("input")).toBeNull();
  });

  it("surfaces an error rather than swallowing it", async () => {
    const root = await mount({ error: "device.key is not a device identity" });

    expect(root.textContent).toContain("not a device identity");
  });
});
