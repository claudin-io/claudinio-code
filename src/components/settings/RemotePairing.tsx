import { Component, Show, createSignal, onCleanup, type Accessor } from "solid-js";
import type { PairingCodeView } from "../../lib/ipc";

/// A pairing waiting on the word check.
export interface PendingPairing {
  peerKey: string;
  label: string;
  /// Three words, already formatted by the device.
  sas: string;
}

export type PairingOutcome = { kind: "paired"; label: string } | { kind: "refused" };

interface RemotePairingProps {
  code: Accessor<PairingCodeView | null>;
  pending: Accessor<PendingPairing | null>;
  outcome: Accessor<PairingOutcome | null>;
  busy: Accessor<boolean>;
  /// Why pairing cannot be started right now — no device key, no open session.
  /// A reason beats a disabled button with nothing to explain it.
  blocked: Accessor<string | null>;
  onStart: (label: string) => void;
  onCancel: () => void;
  onConfirm: (matched: boolean) => void;
}

/// A ticking "now", so the code's remaining life is visible rather than implied.
function createNow() {
  const [now, setNow] = createSignal(Date.now());
  const timer = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(timer));
  return now;
}

export const RemotePairing: Component<RemotePairingProps> = (props) => {
  const [label, setLabel] = createSignal("");
  const now = createNow();

  const secondsLeft = () => {
    const code = props.code();
    if (!code) return 0;
    return Math.max(0, Math.ceil((code.expiresAt - now()) / 1000));
  };

  const start = () => {
    const name = label().trim();
    // A browser named after nothing is a row of hex in the pairing list, which
    // makes the list useless for deciding what to revoke.
    if (name) props.onStart(name);
  };

  return (
    <div class="mb-4">
      <span class="mb-1 block text-[11px] uppercase tracking-wider text-ink-muted">
        {"Pair a browser"}
      </span>

      {/* The word check. First, because when it is on screen it is the only thing
          that matters and nothing is being served until it is answered. */}
      <Show when={props.pending()}>
        {(pending) => (
          <div class="rounded-md border border-accent/40 bg-accent/5 px-3 py-3">
            <p class="mb-2 text-xs text-ink">
              {"Compare these three words with the ones in the browser."}
            </p>
            <p
              class="mb-3 break-words font-mono text-lg font-medium text-ink"
              data-testid="sas"
            >
              {pending().sas}
            </p>
            <p class="mb-3 text-[11px] text-ink-faint">
              {
                "They agree only if nothing sits between the two ends. If they differ, something is relaying your pairing — refuse, and the key is revoked."
              }
            </p>
            <div class="flex flex-wrap gap-2">
              {/* Deliberately not the primary-styled button. The dangerous mistake
                  here is confirming without looking, so neither answer is made the
                  obvious one to tap. */}
              <button
                onClick={() => props.onConfirm(true)}
                disabled={props.busy()}
                class="min-h-11 flex-1 rounded-md border border-border-subtle bg-surface-2 px-3 py-2 text-sm text-ink hover:bg-surface-3 disabled:opacity-50"
              >
                {"The words match"}
              </button>
              <button
                onClick={() => props.onConfirm(false)}
                disabled={props.busy()}
                class="min-h-11 flex-1 rounded-md border border-red-500/40 bg-red-500/10 px-3 py-2 text-sm text-red-500 hover:bg-red-500/20 disabled:opacity-50"
              >
                {"They do not match"}
              </button>
            </div>
          </div>
        )}
      </Show>

      {/* The code, while there is one and nothing is waiting on an answer. */}
      <Show when={!props.pending() && props.code()}>
        {(code) => (
          <div class="rounded-md border border-border-subtle bg-surface-1 px-3 py-3">
            <p class="mb-2 text-xs text-ink">{"Scan this with the phone's camera."}</p>
            {/* A data URI in an <img>, not innerHTML. Nothing user-supplied reaches
                this markup — the QR encodes the URL as modules, not as text — and an
                <img> cannot execute script even if that ever stopped being true. */}
            <img
              src={`data:image/svg+xml;base64,${btoa(code().qrSvg)}`}
              alt="Pairing code"
              width="240"
              height="240"
              class="mb-2 h-auto w-full max-w-[240px] rounded bg-white p-2"
            />
            <p class="mb-1 text-[11px] text-ink-faint">
              {"No app to install — it opens in the browser."}
            </p>
            <code class="mb-2 block break-all rounded bg-surface-2 px-2 py-1 font-mono text-[10px] text-ink-muted">
              {code().url}
            </code>

            {/* The typed code, for when the camera is not the way in: a desktop
                browser, a phone that will not focus, a code read out over a call.
                Below the QR rather than beside it — scanning is the shorter path and
                the one that needs no account, so it stays what the eye lands on.

                Larger than the URL above it and tracked wide, because this is the one
                that gets transcribed by hand. It is also why its alphabet has no I,
                L, O or U. */}
            <Show when={code().typedCode}>
              {(typed) => (
                <div class="mb-2 border-t border-border-subtle pt-2">
                  <p class="mb-1 text-[11px] text-ink-faint">
                    {
                      "Or type this at app.claudin.io. It will ask you to sign in, so that only your account can look the code up."
                    }
                  </p>
                  <code class="block rounded bg-surface-2 px-2 py-1 font-mono text-sm tracking-[0.12em] text-ink">
                    {typed()}
                  </code>
                </div>
              )}
            </Show>

            {/* Only when there is something worth saying. Not being signed in arrives
                as no reason at all and prints nothing: the QR works, and a message
                about an account nobody mentioned is noise. */}
            <Show when={code().typedCodeError}>
              {(why) => (
                <p class="mb-2 text-[11px] text-ink-faint">
                  {`No code to type this time — ${why()}. Scanning still works.`}
                </p>
              )}
            </Show>

            <div class="flex items-center justify-between gap-2">
              <span class="text-[11px] text-ink-faint">
                <Show when={secondsLeft() > 0} fallback={"This code has lapsed."}>
                  {`Good for ${secondsLeft()}s.`}
                </Show>
              </span>
              <button
                onClick={props.onCancel}
                class="min-h-9 shrink-0 rounded-md border border-border-subtle bg-surface-2 px-3 py-1 text-xs text-ink hover:bg-surface-3"
              >
                {"Stop"}
              </button>
            </div>
          </div>
        )}
      </Show>

      {/* Idle. */}
      <Show when={!props.pending() && !props.code()}>
        <Show
          when={!props.blocked()}
          fallback={
            <p class="text-[11px] text-ink-faint">{props.blocked()}</p>
          }
        >
          <div class="rounded-md border border-border-subtle bg-surface-1 px-2 py-2">
            <Show when={props.outcome()}>
              {(outcome) => (
                <p
                  class="mb-2 text-[11px]"
                  classList={{
                    "text-accent": outcome().kind === "paired",
                    "text-red-500": outcome().kind === "refused",
                  }}
                >
                  {outcome().kind === "paired"
                    ? `Paired with ${(outcome() as { label: string }).label}.`
                    : "Refused — that key is revoked and will not be accepted again until you allow it."}
                </p>
              )}
            </Show>
            <label class="mb-1 block text-[11px] text-ink-faint" for="remote-pair-label">
              {"What should this browser be called?"}
            </label>
            <input
              id="remote-pair-label"
              type="text"
              value={label()}
              placeholder="Safari on iPhone"
              onInput={(e) => setLabel(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") start();
              }}
              class="mb-2 min-h-9 w-full rounded-md border border-border-subtle bg-surface-2 px-2 py-1 text-xs text-ink"
            />
            <button
              onClick={start}
              disabled={props.busy() || !label().trim()}
              class="min-h-9 rounded-md bg-accent px-3 py-1.5 text-xs font-medium text-accent-ink hover:bg-accent-hover disabled:opacity-50"
            >
              {"Show a pairing code"}
            </button>
            <p class="mt-1 text-[11px] text-ink-faint">
              {"The code is good for two minutes and pairs one browser."}
            </p>
          </div>
        </Show>
      </Show>
    </div>
  );
};
