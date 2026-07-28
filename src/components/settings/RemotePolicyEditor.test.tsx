import { describe, it, expect, vi, afterEach } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { RemotePolicyEditor } from "./RemotePolicyEditor";
import type { RemotePolicyView, RemoteStoredPolicy } from "../../lib/ipc";

const flush = () => new Promise((r) => setTimeout(r, 10));

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
});

const DAY = 24 * 60 * 60 * 1000;

function stored(over: Partial<RemoteStoredPolicy> = {}): RemoteStoredPolicy {
  return {
    enabled: true,
    expires_at: Date.now() + 7 * DAY,
    workspaces: ["/Users/v/work"],
    idle_disconnect_minutes: 30,
    allow: {
      send_message: true,
      steer: true,
      interrupt: true,
      set_mode: false,
      approve_edit: true,
      approve_bash: "never",
      read_attachment: false,
      export_file: false,
    },
    bash_remote_denylist_extra: [],
    max_unattended_minutes: 60,
    ...over,
  };
}

/// A policy nobody has ever configured: the switch is off and nothing is granted.
function untouched(): RemoteStoredPolicy {
  return stored({
    enabled: false,
    expires_at: null,
    workspaces: [],
    allow: {
      send_message: false,
      steer: false,
      interrupt: false,
      set_mode: false,
      approve_edit: false,
      approve_bash: "never",
      read_attachment: false,
      export_file: false,
    },
  });
}

function view(over: Partial<RemotePolicyView> = {}): RemotePolicyView {
  const s = over.stored ?? stored();
  return {
    path: "/Users/v/.config/claudinio-code/remote-policy.json",
    active: s.enabled,
    inertBecause: s.enabled ? null : "remote access is switched off",
    effective: { ...s.allow, expires_at: s.expires_at },
    workspaces: s.workspaces,
    bashDenylist: [],
    ...over,
    stored: s,
  };
}

interface Handlers {
  onSave?: (policy: RemoteStoredPolicy) => void;
  onDisable?: () => void;
}

async function mount(
  opts: {
    policy?: RemotePolicyView | null;
    running?: string[];
    busy?: boolean;
    activeWorkspace?: string | null;
  } = {},
  handlers: Handlers = {},
) {
  host = document.createElement("div");
  document.body.appendChild(host);

  const [policy] = createSignal(opts.policy === undefined ? view() : opts.policy);
  const [running] = createSignal(opts.running ?? []);
  const [busy] = createSignal(opts.busy ?? false);
  const [activeWorkspace] = createSignal(
    opts.activeWorkspace === undefined ? "/Users/v/work" : opts.activeWorkspace,
  );

  dispose = render(
    () => (
      <RemotePolicyEditor
        policy={policy}
        running={running}
        busy={busy}
        activeWorkspace={activeWorkspace}
        onSave={handlers.onSave ?? (() => {})}
        onDisable={handlers.onDisable ?? (() => {})}
      />
    ),
    host,
  );
  await flush();
  return host;
}

const button = (root: HTMLElement, label: string) =>
  Array.from(root.querySelectorAll("button")).find((b) => b.textContent?.trim() === label);

const checkboxFor = (root: HTMLElement, label: string) => {
  const row = Array.from(root.querySelectorAll("label")).find((l) =>
    l.textContent?.includes(label),
  );
  return row?.querySelector("input[type=checkbox]") as HTMLInputElement | undefined;
};

const selects = (root: HTMLElement) => Array.from(root.querySelectorAll("select"));

describe("RemotePolicyEditor", () => {
  // ── turning it on ────────────────────────────────────────────────────────

  /// The requirement that started this: activating must not mean editing a JSON
  /// file. One press, from a policy nobody has touched, has to produce something
  /// usable.
  it("turns on with useful defaults from an untouched policy", async () => {
    const onSave = vi.fn();
    const root = await mount({ policy: view({ stored: untouched() }) }, { onSave });

    button(root, "Turn on")!.click();

    expect(onSave).toHaveBeenCalledOnce();
    const saved: RemoteStoredPolicy = onSave.mock.calls[0][0];
    expect(saved.enabled).toBe(true);
    expect(saved.allow.send_message).toBe(true);
    expect(saved.allow.interrupt).toBe(true);
    expect(saved.allow.approve_edit).toBe(true);
    expect(saved.workspaces).toEqual(["/Users/v/work"]);
  });

  /// The two that would hand a browser reach beyond the transcript stay off, because
  /// neither is needed to answer "is this going the right way?".
  it("does not grant the shell or file reads by default", async () => {
    const onSave = vi.fn();
    const root = await mount({ policy: view({ stored: untouched() }) }, { onSave });

    button(root, "Turn on")!.click();

    const saved: RemoteStoredPolicy = onSave.mock.calls[0][0];
    expect(saved.allow.approve_bash).toBe("never");
    expect(saved.allow.read_attachment).toBe(false);
    expect(saved.allow.export_file).toBe(false);
  });

  it("gives a first grant an expiry rather than leaving it open-ended", async () => {
    const onSave = vi.fn();
    const root = await mount({ policy: view({ stored: untouched() }) }, { onSave });

    button(root, "Turn on")!.click();

    const saved: RemoteStoredPolicy = onSave.mock.calls[0][0];
    expect(saved.expires_at).not.toBeNull();
    expect(saved.expires_at!).toBeGreaterThan(Date.now());
  });

  /// A configured policy that was switched off must come back as it was, not be
  /// flattened to the defaults.
  it("turning back on keeps what was configured", async () => {
    const onSave = vi.fn();
    const configured = stored({ enabled: false, allow: { ...stored().allow, set_mode: true } });
    const root = await mount({ policy: view({ stored: configured }) }, { onSave });

    button(root, "Turn back on")!.click();

    const saved: RemoteStoredPolicy = onSave.mock.calls[0][0];
    expect(saved.enabled).toBe(true);
    expect(saved.allow.set_mode).toBe(true);
  });

  // ── turning it off ───────────────────────────────────────────────────────

  /// Off has to mean off. Writing the file alone would leave a browser connected
  /// until it next reconnected, which is not what someone pressing this means.
  it("turning off both stores it and drops what is live", async () => {
    const onSave = vi.fn();
    const onDisable = vi.fn();
    const root = await mount({ running: ["abab"] }, { onSave, onDisable });

    button(root, "Turn off")!.click();

    expect(onSave.mock.calls[0][0].enabled).toBe(false);
    expect(onDisable).toHaveBeenCalledOnce();
  });

  /// The warning the web side needs to mirror: whoever is holding the browser
  /// cannot get back in on their own.
  it("warns that a connected browser will need turning back on here", async () => {
    const root = await mount({ running: ["abab"] });

    expect(root.textContent).toContain("drops the browser immediately");
    expect(root.textContent).toContain("turn it back on here");
  });

  /// An empty panel with nothing granted looks broken. Saying why does not — and
  /// this is the component that can then do something about it.
  it("says why nothing is granted when the policy is inert", async () => {
    const root = await mount({
      policy: view({ stored: untouched(), inertBecause: "the grant has expired" }),
    });

    expect(root.textContent).toContain("the grant has expired");
  });

  it("says how many connections are being served", async () => {
    const root = await mount({ running: ["abab", "cdcd"] });
    expect(root.textContent).toContain("serving 2 connections");
  });

  it("says so when it is on with nobody connected", async () => {
    const root = await mount({ running: [] });
    expect(root.textContent).toContain("nothing connected");
  });

  // ── how long ─────────────────────────────────────────────────────────────

  /// Explicitly asked for: the limit must be configurable, up to never.
  it("offers never-expires among the grant lengths", async () => {
    const root = await mount();
    const grant = selects(root)[0];

    expect(Array.from(grant.options).map((o) => o.value)).toContain("Never expires");
  });

  it("choosing never-expires clears the expiry", async () => {
    const onSave = vi.fn();
    const root = await mount({}, { onSave });
    const grant = selects(root)[0];

    grant.value = "Never expires";
    grant.dispatchEvent(new Event("change", { bubbles: true }));
    await flush();
    button(root, "Save permissions")!.click();

    expect(onSave.mock.calls[0][0].expires_at).toBeNull();
  });

  /// Available, but not silently: a grant with no end is the one nobody remembers
  /// giving.
  it("says what never-expires costs", async () => {
    const root = await mount({ policy: view({ stored: stored({ expires_at: null }) }) });

    expect(root.textContent).toContain("nobody remembers giving");
    expect(root.textContent).toContain("does not expire");
  });

  it("choosing a length sets an expiry that far out", async () => {
    const onSave = vi.fn();
    const root = await mount({}, { onSave });
    const grant = selects(root)[0];

    grant.value = "1 day";
    grant.dispatchEvent(new Event("change", { bubbles: true }));
    await flush();
    button(root, "Save permissions")!.click();

    const saved: RemoteStoredPolicy = onSave.mock.calls[0][0];
    expect(saved.expires_at!).toBeGreaterThan(Date.now() + DAY - 60_000);
    expect(saved.expires_at!).toBeLessThan(Date.now() + DAY + 60_000);
  });

  /// The select must reflect what is stored. Resetting to the first option would
  /// quietly shorten a grant on the next save.
  it("shows the stored grant length rather than the first option", async () => {
    const root = await mount({ policy: view({ stored: stored({ expires_at: Date.now() + DAY }) }) });

    expect(selects(root)[0].value).toBe("1 day");
  });

  it("says a lapsed grant has lapsed", async () => {
    const root = await mount({
      policy: view({ stored: stored({ expires_at: Date.now() - DAY }) }),
    });

    expect(root.textContent).toContain("already lapsed");
  });

  /// Matches what the transport now does, so the panel is not promising something
  /// the device does not do.
  it("says an expiry drops a connected browser", async () => {
    const root = await mount();
    expect(root.textContent).toContain("dropped rather than left running");
  });

  // ── capabilities ─────────────────────────────────────────────────────────

  it("toggles a capability and saves it", async () => {
    const onSave = vi.fn();
    const root = await mount({}, { onSave });

    const box = checkboxFor(root, "Change mode")!;
    expect(box.checked).toBe(false);
    box.checked = true;
    box.dispatchEvent(new Event("change", { bubbles: true }));
    await flush();

    button(root, "Save permissions")!.click();
    expect(onSave.mock.calls[0][0].allow.set_mode).toBe(true);
  });

  it("offers the three shell-approval settings", async () => {
    const root = await mount();
    const bash = selects(root)[1];

    expect(Array.from(bash.options).map((o) => o.value)).toEqual([
      "never",
      "allowlist",
      "always",
    ]);
  });

  it("explains what the chosen shell setting means", async () => {
    const root = await mount({
      policy: view({ stored: stored({ allow: { ...stored().allow, approve_bash: "always" } }) }),
    });

    expect(root.textContent).toContain("any command this machine would run");
  });

  /// Not a toggle, at any setting — and the panel says why, or the next person adds
  /// one.
  it("shows exporting as never grantable rather than as an unchecked box", async () => {
    const root = await mount();

    expect(checkboxFor(root, "Export")).toBeUndefined();
    expect(root.textContent).toContain("never granted remotely, at any setting");
    expect(root.textContent).toContain("native dialog");
  });

  it("warns that attachment reads are not workspace-scoped", async () => {
    const root = await mount();
    expect(root.textContent).toContain("not workspace-scoped");
  });

  // ── workspaces ───────────────────────────────────────────────────────────

  /// A policy that grants everything and lists no workspace serves nothing, and
  /// looks broken rather than misconfigured.
  it("says an empty workspace list serves nothing", async () => {
    const root = await mount({ policy: view({ stored: stored({ workspaces: [] }) }) });

    expect(root.textContent).toContain("nothing can be served");
  });

  it("adds the active workspace in one press", async () => {
    const onSave = vi.fn();
    const root = await mount(
      { policy: view({ stored: stored({ workspaces: [] }) }), activeWorkspace: "/Users/v/other" },
      { onSave },
    );

    button(root, "Add this one")!.click();
    await flush();
    button(root, "Save permissions")!.click();

    expect(onSave.mock.calls[0][0].workspaces).toEqual(["/Users/v/other"]);
  });

  it("does not offer to add a workspace already listed", async () => {
    const root = await mount({ activeWorkspace: "/Users/v/work" });
    expect(button(root, "Add this one")).toBeUndefined();
  });

  it("adds a typed path", async () => {
    const onSave = vi.fn();
    const root = await mount({}, { onSave });

    const input = root.querySelector("input[type=text]") as HTMLInputElement;
    input.value = "/Users/v/second";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();
    button(root, "Save permissions")!.click();

    expect(onSave.mock.calls[0][0].workspaces).toContain("/Users/v/second");
  });

  it("removes a workspace", async () => {
    const onSave = vi.fn();
    const root = await mount({}, { onSave });

    button(root, "Remove")!.click();
    await flush();
    button(root, "Save permissions")!.click();

    expect(onSave.mock.calls[0][0].workspaces).toEqual([]);
  });

  it("says the allowlist is a hard boundary", async () => {
    const root = await mount();
    expect(root.textContent).toContain("cannot be served at all");
  });

  // ── saving ───────────────────────────────────────────────────────────────

  /// These are permissions. Autosaving on every toggle would mean a stray tap
  /// granting something, so saving is explicit and the panel says when it is not
  /// saved.
  it("does not save until asked, and says so", async () => {
    const onSave = vi.fn();
    const root = await mount({}, { onSave });

    expect(button(root, "Save permissions")).toBeUndefined();

    const box = checkboxFor(root, "Change mode")!;
    box.checked = true;
    box.dispatchEvent(new Event("change", { bubbles: true }));
    await flush();

    expect(root.textContent).toContain("Not saved yet");
    expect(onSave).not.toHaveBeenCalled();
  });

  it("discards edits on request", async () => {
    const onSave = vi.fn();
    const root = await mount({}, { onSave });

    const box = checkboxFor(root, "Change mode")!;
    box.checked = true;
    box.dispatchEvent(new Event("change", { bubbles: true }));
    await flush();

    button(root, "Discard")!.click();
    await flush();

    expect(button(root, "Save permissions")).toBeUndefined();
    expect(checkboxFor(root, "Change mode")!.checked).toBe(false);
    expect(onSave).not.toHaveBeenCalled();
  });

  it("names the file the policy lands in, and that a peer cannot widen it", async () => {
    const root = await mount();

    expect(root.textContent).toContain("remote-policy.json");
    expect(root.textContent).toContain("can never widen it");
  });

  it("does nothing while a call is in flight", async () => {
    const root = await mount({ busy: true });

    expect(checkboxFor(root, "Change mode")!.disabled).toBe(true);
    expect(button(root, "Turn off")!.disabled).toBe(true);
  });

  it("renders nothing before the policy has loaded", async () => {
    const root = await mount({ policy: null });
    expect(root.textContent).toBe("");
  });
});
