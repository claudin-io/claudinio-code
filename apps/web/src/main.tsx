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
import { editsInMessage } from "./edits";
import {
  explainCodeError,
  explainStaleCode,
  explainState,
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

  /// The transcript as the timeline model sees it.
  ///
  /// Through the same `recordsToMessages` the desktop uses, rather than a second reading
  /// of the JSONL. Compaction, archived blocks, substantial-text promotion and the tool
  /// steps are decisions that were already made once; making them again here is how two
  /// surfaces start disagreeing about what a session contained.
  const messages = () => recordsToMessages(records() as unknown as SessionRecord[]);

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
