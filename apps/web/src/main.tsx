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
import { DiffView } from "@claudinio/timeline-ui/DiffView";
import { Prose } from "@claudinio/timeline-ui/Prose";
import { diffLines } from "@claudinio/timeline-ui/diff";
import { recordsToMessages, type ChatMessage } from "@claudinio/timeline-ui/chatRecords";
import { renderMarkdown } from "@claudinio/timeline-ui/markdown";
import type { SessionRecord } from "@claudinio/timeline-ui/records";
import "@claudinio/timeline-ui/diff.css";
import "@claudinio/timeline-ui/prose.css";
import {
  claimTypedCode,
  listDevices,
  parseAuthorization,
  redeemAuthorization,
  releaseToken,
  startAuthorization,
  type Device,
} from "./auth";
import { editsInMessage } from "./edits";
import {
  explainAuthFailure,
  explainAuthorizing,
  explainClaiming,
  explainCodeError,
  explainStaleCode,
  explainState,
  explainTypedReady,
  type Explanation,
} from "./explain";
import {
  dismissInstallOffer,
  installOffer,
  registerServiceWorker,
  type InstallOffer,
} from "./install";
import {
  forgetPairingCode,
  isStale,
  parsePairingCode,
  type PairingCode,
  type PairingCodeResult,
} from "./pairing";
import { Session, type DeviceMessage, type SessionState } from "./session";

/// Which session to follow.
///
/// The device serves the workspace's active session and refuses anything else, so
/// there is nothing to choose here yet — `ListSessions` and a picker are write-path
/// work. The device ignores this string for now beyond matching what it is serving.
const SESSION_ID = "active";

/// Where the typed-code path has got to.
///
/// A second way in, for when the camera is not an option. `off` is not "unavailable":
/// it is the ordinary state, because the QR path never enters any of this.
type Typed =
  | { kind: "off" }
  | { kind: "authorizing" }
  | { kind: "ready"; token: string; login: string; devices: Device[] }
  | { kind: "claiming" }
  | { kind: "failed"; why: string };

/// Exported for the tests below it, which render it rather than reasoning about it.
/// The composition is the part unit tests cannot see: which of these blocks is on
/// screen at once is a decision about attention, and getting it wrong puts a second
/// way to start next to three words someone is meant to be comparing.
export const App: Component = () => {
  const arrival = window.location.hash;
  /// A returned authorisation and a pairing code arrive at the same URL, in the same
  /// fragment, and are told apart by which parameters are there. Read once, before
  /// anything clears the address bar.
  const returned = parseAuthorization(arrival);
  const [code] = createSignal<PairingCodeResult>(parsePairingCode(arrival));
  const [state, setState] = createSignal<SessionState>({ kind: "connecting" });
  const [records, setRecords] = createSignal<Record<string, unknown>[]>([]);
  const [offer, setOffer] = createSignal<InstallOffer | null>(null);
  const [typed, setTyped] = createSignal<Typed>({ kind: "off" });
  const [dialled, setDialled] = createSignal(false);
  let session: Session | undefined;

  const connect = (pairing: PairingCode) => {
    if (isStale(pairing)) return;
    setDialled(true);
    session = new Session(pairing, {
      onState: setState,
      onMessage: (message) => absorb(message, setRecords),
    });
    session.start();
    session.subscribe(SESSION_ID, 0);
  };

  /// Come back from the account server, and turn what came back into a token.
  const finishAuthorization = async (from: { code: string; state: string }) => {
    const redeemed = await redeemAuthorization(from);
    if (!redeemed.ok) {
      setTyped({ kind: "failed", why: redeemed.why });
      return;
    }
    setTyped({ kind: "ready", token: redeemed.token, login: redeemed.login, devices: [] });
    // The machine names, as a convenience and never as a dependency: pairing works
    // with an empty list, so this is left to arrive on its own.
    const devices = await listDevices(redeemed.token);
    setTyped((current) => (current.kind === "ready" ? { ...current, devices } : current));
  };

  const submitTypedCode = async (entered: string) => {
    const current = typed();
    if (current.kind !== "ready" || !entered.trim()) return;
    setTyped({ kind: "claiming" });

    const claimed = await claimTypedCode(entered, current.token);
    // Released the moment it has done its only job. The token resolves a code; the
    // session that follows runs end to end and the account server has no part in it,
    // so keeping the credential alive for the duration would be keeping it for nothing.
    releaseToken(current.token);

    if (!claimed.ok) {
      setTyped({ kind: "failed", why: claimed.why });
      return;
    }
    setTyped({ kind: "off" });
    connect(claimed.code);
  };

  onMount(() => {
    setOffer(installOffer(window));

    // Out of the address bar before anything else, whichever kind of code it was. Both
    // are single-use and short-lived, and the URL outlives both — history, screenshots,
    // whatever gets shared when someone asks for help.
    forgetPairingCode();

    if (returned.kind === "returned") {
      setTyped({ kind: "authorizing" });
      void finishAuthorization(returned);
    } else {
      const parsed = code();
      if (parsed.ok) connect(parsed.code);
    }

    // Last, and on every arrival including one with no code — which is how someone who
    // has installed it arrives. After the pairing rather than before, because the worker
    // is an optimisation and a prerequisite for phase 4's notifications, and pairing is
    // neither: it has two minutes and must not queue behind a registration.
    registerServiceWorker();
  });

  onCleanup(() => session?.stop());

  /// What the page says, and it is the typed path's turn only while it is in one.
  ///
  /// Ordered so the session wins once there is one: after a claim succeeds the typed
  /// state goes back to `off` and this reads the transport again, which is why a
  /// pairing that came from a typed code shows the same three words as one from a QR.
  const explanation = (): Explanation => {
    const current = typed();
    if (current.kind === "authorizing") return explainAuthorizing();
    if (current.kind === "claiming") return explainClaiming();
    if (current.kind === "failed") return explainAuthFailure(current.why);
    if (current.kind === "ready") return explainTypedReady(current.login);

    if (dialled()) return explainState(state());

    const parsed = code();
    if (!parsed.ok) return explainCodeError(parsed.error);
    // Readable and not dialled: `connect` declines for exactly one reason.
    return explainStaleCode();
  };

  /// The transcript as the timeline model sees it.
  ///
  /// Through the same `recordsToMessages` the desktop uses, rather than a second reading
  /// of the JSONL. Compaction, archived blocks, substantial-text promotion and the tool
  /// steps are decisions that were already made once; making them again here is how two
  /// surfaces start disagreeing about what a session contained.
  const messages = () => recordsToMessages(records() as unknown as SessionRecord[]);

  const ready = () => {
    const current = typed();
    return current.kind === "ready" ? current : null;
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

      {/* The way in when the camera is not an option. Offered whenever nothing is
          being dialled — which includes a code that has expired, since that is
          precisely when someone wants the other route, and excludes a session in
          progress, where three words waiting to be compared matter more than a second
          way to start. */}
      <Show when={typed().kind === "off" && !dialled()}>
        <div class="actions">
          <button onClick={() => void startAuthorization()}>{"Type a code instead"}</button>
        </div>
        <p class="faint">
          {
            "Typing a code needs your Claudinio account, because only the account that made the code can look it up. Scanning needs no account at all."
          }
        </p>
      </Show>

      <Show when={ready()}>
        {(entering) => (
          <>
                <form
                  class="actions"
                  onSubmit={(event) => {
                    event.preventDefault();
                    const input = event.currentTarget.elements.namedItem("code");
                    if (input instanceof HTMLInputElement) void submitTypedCode(input.value);
                  }}
                >
                  <input
                    name="code"
                    class="typed-code"
                    // Ten characters over Crockford's base32, which has no I, L, O or
                    // U — so autocorrect has nothing useful to contribute and every
                    // one of its guesses would be wrong.
                    autocomplete="off"
                    autocapitalize="characters"
                    autocorrect="off"
                    spellcheck={false}
                    inputmode="text"
                    maxlength={12}
                    placeholder="A1B2C-D3E4F"
                    aria-label="Pairing code"
                  />
                  <button type="submit">{"Connect"}</button>
                </form>
            <Show when={entering().devices.length > 0}>
              <p class="faint">
                {`This account has ${entering().devices.length === 1 ? "one machine" : `${entering().devices.length} machines`}: ${entering().devices.map((device) => device.label).join(", ")}.`}
              </p>
            </Show>
          </>
        )}
      </Show>

      <Show when={typed().kind === "failed"}>
        <div class="actions">
          <button onClick={() => void startAuthorization()}>{"Try signing in again"}</button>
        </div>
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

        {/* Only here, and only on iOS — see `installOffer`. Not while the words are
            being checked: that is the one moment the user is doing security work, and
            anything else on the screen is competing with it. */}
        <Show when={offer()}>
          {(hint) => (
            <aside class="offer">
              <p class="offer-headline">{hint().headline}</p>
              <p class="faint">{hint().detail}</p>
              <button
                class="quiet"
                onClick={() => {
                  dismissInstallOffer(window);
                  setOffer(null);
                }}
              >
                {/* Not "Not now": it does not come back. */}
                {"Hide this"}
              </button>
            </aside>
          )}
        </Show>

        <div class="records">
          <For each={messages()}>{(message) => <MessageRow message={message} />}</For>
          <div class="count">
            {records().length === 0
              ? "Waiting for the transcript…"
              : `${messages().length} message${messages().length === 1 ? "" : "s"} · ${records().length} record${records().length === 1 ? "" : "s"}`}
          </div>
        </div>
      </Show>
    </main>
  );
};

/// One message: who said it, what they said, and any change it is about to make.
///
/// The diff is the reason this is not a line of text. §7 of the threat model rests on a
/// human reading a change before approving it, and the transcript carries the before and
/// after rather than a diff — so it is computed here, from what the tool call said it
/// would do.
const MessageRow: Component<{ message: ChatMessage }> = (props) => {
  const edits = () => editsInMessage(props.message);
  /// The tools that ran, named. Not their output: this is read-only and a phone, and a
  /// wall of command output between the change and the approve button is how the change
  /// stops being read.
  const tools = () =>
    (props.message.steps ?? [])
      .map((step) => step.tool?.call.toolName)
      .filter((name): name is string => !!name);

  return (
    <div class="record" classList={{ mine: props.message.role === "user" }}>
      <div class="who">{props.message.role}</div>
      <Show when={props.message.text.trim()}>
        {/* Through the package's renderer, which is where DOMPurify runs. Model output
            reaching an origin that holds the key to someone's machine is what §10 names,
            and a second sanitizer is a second thing to get wrong. */}
        <Prose html={renderMarkdown(props.message.text)} />
      </Show>
      <Show when={tools().length > 0}>
        <div class="tools">{tools().join(" · ")}</div>
      </Show>
      <For each={edits()}>
        {(edit) => <DiffView path={edit.path} diff={diffLines(edit.oldText, edit.newText)} />}
      </For>
    </div>
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
