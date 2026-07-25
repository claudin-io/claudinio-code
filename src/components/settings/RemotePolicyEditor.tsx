import { Component, For, Show, createMemo, createSignal, type Accessor } from "solid-js";
import {
  GRANT_DURATIONS,
  type BashApproval,
  type RemotePolicyView,
  type RemoteStoredPolicy,
} from "../../lib/ipc";

interface RemotePolicyEditorProps {
  policy: Accessor<RemotePolicyView | null>;
  /// The channels being served. Non-empty means a browser is connected right now,
  /// which changes what turning this off means to the person holding it.
  running: Accessor<string[]>;
  busy: Accessor<boolean>;
  /// Offered as a one-tap addition to the allowlist, because a policy that grants
  /// everything and lists no workspace serves nothing and looks broken.
  activeWorkspace: Accessor<string | null>;
  onSave: (policy: RemoteStoredPolicy) => void;
  onDisable: () => void;
}

/// What "on" means when the user has never configured anything.
///
/// Chosen so the first thing that happens is useful rather than empty: watch a run,
/// steer it, interrupt it, approve an edit. Shell approval and attachment reads stay
/// off — both hand a remote peer reach beyond the transcript, and neither is needed
/// to answer "is it going the right way?", which is why someone opens this from a
/// phone.
function safeDefaults(workspace: string | null): RemoteStoredPolicy {
  return {
    enabled: true,
    expires_at: Date.now() + 7 * 24 * 60 * 60 * 1000,
    workspaces: workspace ? [workspace] : [],
    idle_disconnect_minutes: 30,
    allow: {
      send_message: true,
      steer: true,
      interrupt: true,
      set_mode: false,
      approve_edit: true,
      approve_bash: "never",
      read_attachment: false,
      // Never, at any setting. See the note in the panel.
      export_file: false,
    },
    bash_remote_denylist_extra: [],
    max_unattended_minutes: 60,
  };
}

const CAPABILITIES: {
  key: keyof RemoteStoredPolicy["allow"];
  label: string;
  note?: string;
}[] = [
  { key: "send_message", label: "Send messages" },
  { key: "steer", label: "Steer a run mid-reasoning" },
  { key: "interrupt", label: "Interrupt" },
  { key: "set_mode", label: "Change mode" },
  { key: "approve_edit", label: "Approve edits" },
  {
    key: "read_attachment",
    label: "Read attachments",
    note: "Reads are not workspace-scoped, so this is a read of anywhere you can read.",
  },
];

const BASH_CHOICES: { value: BashApproval; label: string; note: string }[] = [
  { value: "never", label: "Never", note: "Shell approvals wait for you at the machine." },
  {
    value: "allowlist",
    label: "Allowlist only",
    note: "Commands on the local allowlist, minus anything you add to the remote denylist.",
  },
  {
    value: "always",
    label: "Anything",
    note: "A browser can approve any command this machine would run.",
  },
];

const dayMs = 24 * 60 * 60 * 1000;

export const RemotePolicyEditor: Component<RemotePolicyEditorProps> = (props) => {
  /// Edits in flight, or `null` when what is shown is what is stored. Kept separate
  /// so the panel does not appear to have saved something it has not.
  const [draft, setDraft] = createSignal<RemoteStoredPolicy | null>(null);
  const [workspaceDraft, setWorkspaceDraft] = createSignal("");

  const stored = () => props.policy()?.stored ?? null;
  const current = () => draft() ?? stored();
  const dirty = () => draft() !== null;
  const connected = () => props.running().length > 0;

  /// Has this ever been set up? A policy that grants nothing and lists no workspace
  /// has not, whatever the switch says.
  const unconfigured = createMemo(() => {
    const policy = stored();
    if (!policy) return true;
    const nothingAllowed = !Object.entries(policy.allow).some(
      ([key, value]) => key !== "approve_bash" && value === true,
    );
    return nothingAllowed && policy.workspaces.length === 0;
  });

  const edit = (change: (policy: RemoteStoredPolicy) => RemoteStoredPolicy) => {
    const base = current();
    if (!base) return;
    setDraft(change(structuredClone(base)));
  };

  const save = () => {
    const policy = draft();
    if (!policy) return;
    setDraft(null);
    props.onSave(policy);
  };

  const turnOn = () => {
    const base = unconfigured() ? safeDefaults(props.activeWorkspace()) : current();
    if (!base) return;
    setDraft(null);
    props.onSave({ ...base, enabled: true });
  };

  /// Off writes the file *and* stops what is live. Only writing would leave a
  /// browser connected until it next reconnected, which is not what "off" means to
  /// someone who just pressed it.
  const turnOff = () => {
    const base = current();
    setDraft(null);
    if (base) props.onSave({ ...base, enabled: false });
    props.onDisable();
  };

  /// Which preset the stored expiry matches, so the select reflects reality rather
  /// than resetting to the first option.
  const grantChoice = () => {
    const policy = current();
    if (!policy) return "";
    if (policy.expires_at === null) return "never";
    const remaining = policy.expires_at - Date.now();
    const closest = GRANT_DURATIONS.filter((d) => d.ms !== null).reduce((best, option) =>
      Math.abs((option.ms as number) - remaining) < Math.abs((best.ms as number) - remaining)
        ? option
        : best,
    );
    return closest.label;
  };

  const setGrant = (label: string) => {
    const option = GRANT_DURATIONS.find((d) => d.label === label);
    if (!option) return;
    edit((policy) => ({
      ...policy,
      expires_at: option.ms === null ? null : Date.now() + option.ms,
    }));
  };

  const expiryText = () => {
    const policy = current();
    if (!policy) return "";
    if (policy.expires_at === null) return "This grant does not expire.";
    const days = Math.max(0, Math.round((policy.expires_at - Date.now()) / dayMs));
    if (policy.expires_at <= Date.now()) return "This grant has already lapsed.";
    return days >= 1
      ? `Lapses in about ${days} day${days === 1 ? "" : "s"}.`
      : "Lapses within the day.";
  };

  const addWorkspace = (path: string) => {
    const trimmed = path.trim();
    if (!trimmed) return;
    edit((policy) =>
      policy.workspaces.includes(trimmed)
        ? policy
        : { ...policy, workspaces: [...policy.workspaces, trimmed] },
    );
    setWorkspaceDraft("");
  };

  const canAddActive = () => {
    const workspace = props.activeWorkspace();
    const policy = current();
    return !!workspace && !!policy && !policy.workspaces.includes(workspace);
  };

  return (
    <Show when={props.policy()}>
      {(view) => (
        <div class="mb-4">
          {/* The switch. First, and phrased as a state rather than an action, so it
              reads the same whichever way round it is. */}
          <div class="mb-2 flex items-center justify-between gap-2 rounded-md border border-border-subtle bg-surface-1 px-2 py-2">
            <div class="min-w-0">
              <div class="text-xs text-ink">{"Remote access"}</div>
              <div class="text-[11px] text-ink-faint">
                <Show when={view().active} fallback={view().inertBecause ?? "Off."}>
                  <Show when={connected()} fallback={"On, with nothing connected."}>
                    {`On, serving ${props.running().length} connection${
                      props.running().length === 1 ? "" : "s"
                    }.`}
                  </Show>
                </Show>
              </div>
            </div>
            <Show
              when={view().active}
              fallback={
                <button
                  onClick={turnOn}
                  disabled={props.busy()}
                  class="min-h-9 shrink-0 rounded-md bg-accent px-3 py-1.5 text-xs font-medium text-accent-ink hover:bg-accent-hover disabled:opacity-50"
                >
                  {unconfigured() ? "Turn on" : "Turn back on"}
                </button>
              }
            >
              <button
                onClick={turnOff}
                disabled={props.busy()}
                class="min-h-9 shrink-0 rounded-md border border-red-500/40 bg-red-500/10 px-3 py-1.5 text-xs text-red-500 hover:bg-red-500/20 disabled:opacity-50"
              >
                {"Turn off"}
              </button>
            </Show>
          </div>

          <Show when={unconfigured() && !view().active}>
            <p class="mb-3 text-[11px] text-ink-faint">
              {
                "Turning it on grants a paired browser what it needs to follow a run and approve an edit — not the shell, and not your files. You can narrow or widen it below."
              }
            </p>
          </Show>

          <Show when={connected()}>
            <p class="mb-3 rounded-md border border-border-subtle bg-surface-1 px-2 py-1.5 text-[11px] text-ink-muted">
              {
                "Turning it off drops the browser immediately and it will not reconnect on its own. Whoever is using it has to wait for you to turn it back on here."
              }
            </p>
          </Show>

          <Show when={current()}>
            {(policy) => (
              <>
                {/* How long. */}
                <span class="mb-1 block text-[11px] uppercase tracking-wider text-ink-muted">
                  {"How long this grant lasts"}
                </span>
                <select
                  value={grantChoice()}
                  onChange={(e) => setGrant(e.currentTarget.value)}
                  class="mb-1 min-h-9 w-full rounded-md border border-border-subtle bg-surface-2 px-2 py-1 text-xs text-ink"
                >
                  <For each={GRANT_DURATIONS}>
                    {(option) => (
                      <option value={option.label} selected={grantChoice() === option.label}>
                        {option.label}
                      </option>
                    )}
                  </For>
                </select>
                <p class="mb-1 text-[11px] text-ink-faint">{expiryText()}</p>
                <Show when={policy().expires_at === null}>
                  <p class="mb-3 text-[11px] text-amber-500">
                    {
                      "A grant with no end is the one nobody remembers giving. Fine if you meant it — revoking a browser still works from here at any time."
                    }
                  </p>
                </Show>
                <Show when={policy().expires_at !== null}>
                  <p class="mb-3 text-[11px] text-ink-faint">
                    {"When it lapses, a connected browser is dropped rather than left running."}
                  </p>
                </Show>

                {/* What a browser may do. */}
                <span class="mb-1 block text-[11px] uppercase tracking-wider text-ink-muted">
                  {"What a paired browser may do"}
                </span>
                <div class="mb-2 space-y-1">
                  <For each={CAPABILITIES}>
                    {(capability) => (
                      <label class="flex min-h-9 cursor-pointer items-start justify-between gap-2 rounded-md border border-border-subtle bg-surface-1 px-2 py-1.5 text-xs">
                        <span class="min-w-0">
                          <span class="text-ink">{capability.label}</span>
                          <Show when={capability.note}>
                            <span class="block text-[11px] text-ink-faint">{capability.note}</span>
                          </Show>
                        </span>
                        <input
                          type="checkbox"
                          checked={policy().allow[capability.key] === true}
                          disabled={props.busy()}
                          onChange={(e) => {
                            const on = e.currentTarget.checked;
                            edit((p) => ({ ...p, allow: { ...p.allow, [capability.key]: on } }));
                          }}
                          class="mt-0.5 shrink-0"
                        />
                      </label>
                    )}
                  </For>
                </div>

                {/* Shell approval: three states, so a select rather than a switch. */}
                <span class="mb-1 block text-[11px] uppercase tracking-wider text-ink-muted">
                  {"Approve shell commands"}
                </span>
                <select
                  value={policy().allow.approve_bash}
                  onChange={(e) =>
                    edit((p) => ({
                      ...p,
                      allow: { ...p.allow, approve_bash: e.currentTarget.value as BashApproval },
                    }))
                  }
                  class="mb-1 min-h-9 w-full rounded-md border border-border-subtle bg-surface-2 px-2 py-1 text-xs text-ink"
                >
                  <For each={BASH_CHOICES}>
                    {(choice) => (
                      <option
                        value={choice.value}
                        selected={policy().allow.approve_bash === choice.value}
                      >
                        {choice.label}
                      </option>
                    )}
                  </For>
                </select>
                <p class="mb-3 text-[11px] text-ink-faint">
                  {BASH_CHOICES.find((c) => c.value === policy().allow.approve_bash)?.note}
                </p>

                {/* Exporting has no safe remote form, so it is not a switch. */}
                <p class="mb-3 rounded-md border border-border-subtle bg-surface-1 px-2 py-1.5 text-[11px] text-ink-faint">
                  {
                    "Exporting files is never granted remotely, at any setting: it is safe locally only because you pick the destination in a native dialog, and there is no remote equivalent of standing at the machine."
                  }
                </p>

                {/* Workspaces. */}
                <span class="mb-1 block text-[11px] uppercase tracking-wider text-ink-muted">
                  {"Workspaces a browser can see"}
                </span>
                <Show
                  when={policy().workspaces.length > 0}
                  fallback={
                    <p class="mb-1 text-[11px] text-amber-500">
                      {"None listed, so nothing can be served — add one below."}
                    </p>
                  }
                >
                  <div class="mb-1 space-y-1">
                    <For each={policy().workspaces}>
                      {(path) => (
                        <div class="flex items-center gap-2 rounded-md border border-border-subtle bg-surface-1 px-2 py-1">
                          <code class="min-w-0 flex-1 truncate font-mono text-[11px] text-ink">
                            {path}
                          </code>
                          <button
                            onClick={() =>
                              edit((p) => ({
                                ...p,
                                workspaces: p.workspaces.filter((w) => w !== path),
                              }))
                            }
                            disabled={props.busy()}
                            class="shrink-0 rounded-md border border-border-subtle bg-surface-2 px-2 py-0.5 text-[11px] text-ink hover:bg-surface-3 disabled:opacity-50"
                          >
                            {"Remove"}
                          </button>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
                <div class="mb-1 flex gap-2">
                  <input
                    type="text"
                    value={workspaceDraft()}
                    placeholder="/absolute/path"
                    onInput={(e) => setWorkspaceDraft(e.currentTarget.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") addWorkspace(workspaceDraft());
                    }}
                    class="min-h-9 min-w-0 flex-1 rounded-md border border-border-subtle bg-surface-2 px-2 py-1 font-mono text-[11px] text-ink"
                  />
                  <Show when={canAddActive()}>
                    <button
                      onClick={() => addWorkspace(props.activeWorkspace()!)}
                      disabled={props.busy()}
                      class="min-h-9 shrink-0 rounded-md border border-border-subtle bg-surface-2 px-2 py-1 text-[11px] text-ink hover:bg-surface-3 disabled:opacity-50"
                    >
                      {"Add this one"}
                    </button>
                  </Show>
                </div>
                <p class="mb-3 text-[11px] text-ink-faint">
                  {"Matched exactly. A workspace that is not on this list cannot be served at all, whatever else is granted."}
                </p>

                {/* Saving. Explicit, because these are permissions. */}
                <Show when={dirty()}>
                  <div class="flex items-center gap-2">
                    <button
                      onClick={save}
                      disabled={props.busy()}
                      class="min-h-9 rounded-md bg-accent px-3 py-1.5 text-xs font-medium text-accent-ink hover:bg-accent-hover disabled:opacity-50"
                    >
                      {"Save permissions"}
                    </button>
                    <button
                      onClick={() => setDraft(null)}
                      disabled={props.busy()}
                      class="min-h-9 rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-xs text-ink hover:bg-surface-3 disabled:opacity-50"
                    >
                      {"Discard"}
                    </button>
                    <span class="text-[11px] text-ink-faint">{"Not saved yet."}</span>
                  </div>
                </Show>

                <p class="mt-2 text-[11px] text-ink-faint">
                  {"Stored on this machine, in "}
                  <code class="font-mono text-ink-muted">{view().path}</code>
                  {". A paired browser can read this and can never widen it."}
                </p>
              </>
            )}
          </Show>
        </div>
      )}
    </Show>
  );
};
