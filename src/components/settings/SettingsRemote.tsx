import { Component, For, Show, type Accessor } from "solid-js";
import type { Pairing, RemotePolicyView } from "../../lib/ipc";

interface SettingsRemoteProps {
  deviceKey: Accessor<string | null>;
  policy: Accessor<RemotePolicyView | null>;
  pairings: Accessor<Pairing[]>;
  revoked: Accessor<string[]>;
  error: Accessor<string | null>;
  busy: Accessor<boolean>;
  onCreateIdentity: () => void;
  onRevoke: (peerKey: string) => void;
  onUnrevoke: (peerKey: string) => void;
  onRename: (peerKey: string) => void;
}

/// A key is 64 hex characters, which is unreadable and also the thing the user has
/// to compare when pairing by hand. Shown in fragments so it can be checked
/// against a screen without scrolling.
const shortKey = (key: string) => `${key.slice(0, 8)}…${key.slice(-8)}`;

const formatWhen = (ms: number) => new Date(ms).toLocaleString();

export const SettingsRemote: Component<SettingsRemoteProps> = (props) => {
  return (
    <>
      <div class="mb-2 flex items-center justify-between">
        <span class="text-sm font-medium text-ink">{"Remote access"}</span>
        <Show when={props.policy()}>
          {(policy) => (
            <span
              class="rounded-md px-2 py-0.5 text-[11px]"
              classList={{
                "bg-surface-2 text-ink-muted": !policy().active,
                "bg-accent/15 text-accent": policy().active,
              }}
            >
              {policy().active ? "Active" : "Off"}
            </span>
          )}
        </Show>
      </div>

      <p class="mb-3 text-[11px] text-ink-faint">
        {
          "Drive this machine's sessions from a browser. The agent, your files and the transcript stay here — what travels is an encrypted stream the relay cannot read."
        }
      </p>

      <Show when={props.error()}>
        <p class="mb-3 rounded-md border border-red-500/40 bg-red-500/10 px-2 py-1.5 text-[11px] text-red-500">
          {props.error()}
        </p>
      </Show>

      {/* Identity. Creating a long-term key is a deliberate act, so opening this
          panel does not do it. */}
      <div class="mb-4">
        <span class="mb-1 block text-[11px] uppercase tracking-wider text-ink-muted">
          {"This device"}
        </span>
        <Show
          when={props.deviceKey()}
          fallback={
            <div class="rounded-md border border-border-subtle bg-surface-1 px-2 py-2">
              <p class="mb-2 text-[11px] text-ink-faint">
                {
                  "No device key yet. It identifies this machine to the browsers you pair, and it is what makes a substituted key fail instead of going unnoticed."
                }
              </p>
              <button
                onClick={props.onCreateIdentity}
                disabled={props.busy()}
                class="rounded-md border border-border-subtle bg-surface-2 px-2 py-1 text-xs text-ink hover:bg-surface-3 disabled:opacity-50"
              >
                {"Create device key"}
              </button>
            </div>
          }
        >
          {(key) => (
            <div class="rounded-md border border-border-subtle bg-surface-1 px-2 py-1.5">
              <code class="font-mono text-xs text-ink">{shortKey(key())}</code>
              <p class="mt-1 text-[11px] text-ink-faint">
                {"Public by design — this is what a pairing code carries."}
              </p>
            </div>
          )}
        </Show>
      </div>

      {/* Policy. Read-only here: it is edited by hand, and the panel says where. */}
      <Show when={props.policy()}>
        {(policy) => (
          <div class="mb-4">
            <span class="mb-1 block text-[11px] uppercase tracking-wider text-ink-muted">
              {"What a paired browser may do"}
            </span>

            <Show when={policy().inertBecause}>
              <p class="mb-2 rounded-md border border-border-subtle bg-surface-1 px-2 py-1.5 text-[11px] text-ink-muted">
                {`Nothing is granted: ${policy().inertBecause}.`}
              </p>
            </Show>

            <div class="mb-2 space-y-1">
              <For
                each={[
                  ["Send messages", policy().effective.send_message],
                  ["Steer a run", policy().effective.steer],
                  ["Interrupt", policy().effective.interrupt],
                  ["Change mode", policy().effective.set_mode],
                  ["Approve edits", policy().effective.approve_edit],
                  ["Read attachments", policy().effective.read_attachment],
                  ["Export files", policy().effective.export_file],
                ]}
              >
                {([label, granted]) => (
                  <div class="flex items-center justify-between rounded-md border border-border-subtle bg-surface-1 px-2 py-1 text-xs">
                    <span class="text-ink">{label}</span>
                    <span
                      classList={{
                        "text-ink-faint": !granted,
                        "text-accent": !!granted,
                      }}
                    >
                      {granted ? "allowed" : "denied"}
                    </span>
                  </div>
                )}
              </For>
              <div class="flex items-center justify-between rounded-md border border-border-subtle bg-surface-1 px-2 py-1 text-xs">
                <span class="text-ink">{"Approve shell commands"}</span>
                <span class="text-ink-muted">{policy().effective.approve_bash}</span>
              </div>
            </div>

            {/* Exporting has no safe remote form, and saying so is better than
                letting someone wonder why the switch does nothing. */}
            <p class="mb-2 text-[11px] text-ink-faint">
              {
                "Exporting files is never granted remotely: it is safe locally only because you pick the destination in a native dialog, and there is no remote equivalent of standing at the machine."
              }
            </p>

            <Show when={policy().workspaces.length > 0}>
              <span class="mb-1 block text-[11px] uppercase tracking-wider text-ink-muted">
                {"Workspaces a browser can see"}
              </span>
              <div class="mb-2 space-y-1">
                <For each={policy().workspaces}>
                  {(path) => (
                    <code class="block truncate rounded-md border border-border-subtle bg-surface-1 px-2 py-1 font-mono text-[11px] text-ink">
                      {path}
                    </code>
                  )}
                </For>
              </div>
            </Show>

            <p class="text-[11px] text-ink-faint">
              {"Edited on this machine only, in "}
              <code class="font-mono text-ink-muted">{policy().path}</code>
              {". A paired browser can read this policy and can never widen it."}
            </p>
          </div>
        )}
      </Show>

      {/* Pairings. */}
      <div class="mb-4">
        <span class="mb-1 block text-[11px] uppercase tracking-wider text-ink-muted">
          {"Paired browsers"}
        </span>
        <Show
          when={props.pairings().length > 0}
          fallback={
            <p class="text-[11px] text-ink-faint">
              {"None yet. Pairing needs physical access to this machine, to read the code off this screen."}
            </p>
          }
        >
          <div class="space-y-1">
            <For each={props.pairings()}>
              {(pairing) => (
                <div class="flex items-center gap-2 rounded-md border border-border-subtle bg-surface-1 px-2 py-1.5 text-xs">
                  <div class="min-w-0 flex-1">
                    <div class="truncate text-ink">{pairing.label}</div>
                    <div class="font-mono text-[11px] text-ink-faint">
                      {shortKey(pairing.peer_key)}
                      {" · paired "}
                      {formatWhen(pairing.paired_at)}
                      <Show when={pairing.expires_at}>
                        {(at) => `${" · expires "}${formatWhen(at())}`}
                      </Show>
                    </div>
                  </div>
                  <button
                    onClick={() => props.onRename(pairing.peer_key)}
                    disabled={props.busy()}
                    class="shrink-0 rounded-md border border-border-subtle bg-surface-2 px-2 py-0.5 text-[11px] text-ink hover:bg-surface-3 disabled:opacity-50"
                  >
                    {"Rename"}
                  </button>
                  <button
                    onClick={() => props.onRevoke(pairing.peer_key)}
                    disabled={props.busy()}
                    class="shrink-0 rounded-md border border-red-500/40 bg-red-500/10 px-2 py-0.5 text-[11px] text-red-500 hover:bg-red-500/20 disabled:opacity-50"
                  >
                    {"Revoke"}
                  </button>
                </div>
              )}
            </For>
          </div>
        </Show>
        <p class="mt-1 text-[11px] text-ink-faint">
          {
            "Revoking takes effect here, immediately, without asking anyone — so it still works when the relay is down."
          }
        </p>
      </div>

      {/* Revoked keys, kept so a revoked browser does not look like a stranger. */}
      <Show when={props.revoked().length > 0}>
        <div class="mb-2">
          <span class="mb-1 block text-[11px] uppercase tracking-wider text-ink-muted">
            {"Revoked"}
          </span>
          <div class="space-y-1">
            <For each={props.revoked()}>
              {(peerKey) => (
                <div class="flex items-center gap-2 rounded-md border border-border-subtle bg-surface-1 px-2 py-1 text-xs">
                  <code class="min-w-0 flex-1 truncate font-mono text-[11px] text-ink-faint">
                    {shortKey(peerKey)}
                  </code>
                  <button
                    onClick={() => props.onUnrevoke(peerKey)}
                    disabled={props.busy()}
                    class="shrink-0 rounded-md border border-border-subtle bg-surface-2 px-2 py-0.5 text-[11px] text-ink hover:bg-surface-3 disabled:opacity-50"
                  >
                    {"Allow again"}
                  </button>
                </div>
              )}
            </For>
          </div>
          <p class="mt-1 text-[11px] text-ink-faint">
            {
              "Kept on purpose. A revoked browser stays refused until you allow it again, rather than pairing itself back in by reconnecting."
            }
          </p>
        </div>
      </Show>
    </>
  );
};
