/// The bare page.
///
/// Enough to prove the transport works against the real device and nothing more: the
/// words to compare, the two answers, and the transcript records as they arrive. The
/// timeline proper arrives with `@claudinio/timeline-ui`; building it on top of an
/// unproven transport would bury the first real bug under a rendering layer.
///
/// Read-only, per §8 phase 3. There is no composer and no approval button, and the
/// session can only send `Subscribe`.

import { For, Show, createSignal, onCleanup, onMount, type Component } from "solid-js";
import { render } from "solid-js/web";
import {
  explainCodeError,
  explainStaleCode,
  explainState,
  summariseRecord,
  type Explanation,
} from "./explain";
import { forgetPairingCode, isStale, parsePairingCode, type PairingCodeResult } from "./pairing";
import { Session, type DeviceMessage, type SessionState } from "./session";

/// Which session to follow.
///
/// The device serves the workspace's active session and refuses anything else, so
/// there is nothing to choose here yet — `ListSessions` and a picker are write-path
/// work. The device ignores this string for now beyond matching what it is serving.
const SESSION_ID = "active";

const App: Component = () => {
  const [code] = createSignal<PairingCodeResult>(parsePairingCode(window.location.hash));
  const [state, setState] = createSignal<SessionState>({ kind: "connecting" });
  const [records, setRecords] = createSignal<Record<string, unknown>[]>([]);
  let session: Session | undefined;

  onMount(() => {
    const parsed = code();
    if (!parsed.ok) return;

    // Out of the address bar before anything else. It is single-use and lasts two
    // minutes, but the URL outlives both — history, screenshots, support requests.
    forgetPairingCode();
    if (isStale(parsed.code)) return;

    session = new Session(parsed.code, {
      onState: setState,
      onMessage: (message) => absorb(message, setRecords),
    });
    session.start();
    session.subscribe(SESSION_ID, 0);
  });

  onCleanup(() => session?.stop());

  const explanation = (): Explanation => {
    const parsed = code();
    if (!parsed.ok) return explainCodeError(parsed.error);
    if (isStale(parsed.code)) return explainStaleCode();
    return explainState(state());
  };

  const live = () => state().kind === "live";
  const confirming = () => state().kind === "confirming";
  const sas = () => {
    const current = state();
    return current.kind === "confirming" || current.kind === "live" ? current.sas : "";
  };

  return (
    <main class="page">
      <h1>{explanation().headline}</h1>
      <Show when={explanation().detail}>
        {(detail) => <p classList={{ danger: state().kind === "failed" }}>{detail()}</p>}
      </Show>

      <Show when={confirming()}>
        <div class="sas">{sas()}</div>
        <div class="actions">
          <button onClick={() => session?.confirm()}>{"The words match"}</button>
          <button class="refuse" onClick={() => session?.stop()}>
            {"They do not match"}
          </button>
        </div>
        <p class="faint">
          {
            "Refusing shares nothing and simply disconnects. Nothing has been requested from your machine yet."
          }
        </p>
      </Show>

      <Show when={live()}>
        <p class="faint">{`Following this session, read-only. ${sas()}`}</p>
        <div class="records">
          <For each={records()}>
            {(record) => <div class="record">{summariseRecord(record)}</div>}
          </For>
          <div class="count">
            {records().length === 0
              ? "Waiting for the transcript…"
              : `${records().length} record${records().length === 1 ? "" : "s"}`}
          </div>
        </div>
      </Show>
    </main>
  );
};

/// Fold a device message into the records on screen.
///
/// Exported so the shape of this is testable without a DOM: a `Snapshot` is a batch
/// and an `Event` is one, and getting that backwards shows up as a transcript that is
/// either empty or nested.
export function absorb(
  message: DeviceMessage,
  setRecords: (update: (previous: Record<string, unknown>[]) => Record<string, unknown>[]) => void,
): void {
  if (message.kind === "snapshot" && Array.isArray(message.records)) {
    const batch = (message.records as unknown[]).filter(isRecord);
    setRecords((previous) => [...previous, ...batch]);
    return;
  }
  if (message.kind === "event" && isRecord(message.event)) {
    setRecords((previous) => [...previous, message.event as Record<string, unknown>]);
    return;
  }
  // A `Gap` means the device is telling the truth about what it could not deliver, and
  // recovery is the resubscribe the session already does on reconnect. Anything else —
  // an approval request, a policy denial — belongs to the write path, which this build
  // does not have. Both are dropped rather than rendered as if they were transcript.
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

const root = document.getElementById("root");
if (root) render(() => <App />, root);
