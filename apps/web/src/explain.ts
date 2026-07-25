/// What the user is told, and why each wording was chosen.
///
/// Separated from the page because this is the part that matters and the part worth
/// testing. A remote session fails in ways the person holding the phone cannot
/// diagnose — the relay is down, the grant lapsed, someone turned it off, the code is
/// two minutes old — and the difference between "wait" and "go and do something" is
/// the whole value of the message.
///
/// The rule throughout: never say "error" for something ordinary, and always say
/// whether the fix is here or on the machine.

import type { PairingCodeError } from "./pairing";
import type { CloseReason, SessionState } from "./session";

export interface Explanation {
  /// One line, in the imperative or the present tense. No exclamation marks.
  headline: string;
  /// What to do, when there is something. Absent when waiting is the answer.
  detail?: string;
  /// Whether this is a state the user can act on from here.
  actionable: boolean;
}

/// Why there is nothing to connect to.
export function explainCodeError(error: PairingCodeError): Explanation {
  if (error.kind === "absent") {
    return {
      // Not an error. Opening the page directly is the ordinary way to arrive at it.
      headline: "Scan a pairing code to begin",
      detail:
        "Open Claudinio Code on your machine, go to Settings → Remote, and show a pairing code.",
      actionable: true,
    };
  }
  return {
    headline: "This pairing code is not readable",
    // The field is named because a mangled code is usually a copy-paste that lost
    // part of the URL, and knowing which part is missing is the fastest fix.
    detail: `${error.why}. Show a fresh code on your machine rather than editing this link.`,
    actionable: true,
  };
}

/// The code parsed but has gone stale.
///
/// Deliberately its own message. "Corrupted" and "expired" have different fixes, and
/// a code that lapsed is the common case — two minutes is short on purpose.
export function explainStaleCode(): Explanation {
  return {
    headline: "This pairing code has expired",
    detail: "Codes last two minutes. Show a new one on your machine.",
    actionable: true,
  };
}

export function explainState(state: SessionState): Explanation {
  switch (state.kind) {
    case "connecting":
      return {
        // Not "connecting to the relay": the user does not have a relay, they have a
        // machine. And the device may simply not be listening yet, which is not a
        // failure and must not read as one.
        headline: "Looking for your machine",
        detail: "It has to be showing a pairing code for this to find it.",
        actionable: false,
      };

    case "confirming":
      return {
        headline: "Check these words match",
        detail:
          "They appear on your machine too. If they differ, something is sitting between the two — refuse, and nothing is shared.",
        actionable: true,
      };

    case "live":
      return { headline: "Connected", actionable: false };

    case "closed":
      return explainClose(state.reason);

    case "failed":
      return {
        headline: "Could not connect",
        detail: state.why,
        actionable: true,
      };
  }
}

/// Why the device stopped, and where the fix is.
///
/// Three of the four cannot be fixed from the browser. Saying so is the difference
/// between the user waiting for something that will never happen and the user getting
/// up and going to the machine.
export function explainClose(reason: CloseReason): Explanation {
  switch (reason) {
    case "peer_asked":
      return {
        headline: "You closed this session",
        detail: "Turn remote access back on in Claudinio Code to reconnect.",
        actionable: false,
      };

    case "turned_off_locally":
      return {
        headline: "Remote access was turned off on your machine",
        detail: "It has to be turned back on there. Nothing here can reconnect on its own.",
        actionable: false,
      };

    case "grant_expired":
      return {
        headline: "The grant expired",
        detail: "Extend it in Settings → Remote on your machine.",
        actionable: false,
      };

    case "revoked":
      return {
        headline: "This browser was revoked",
        // Distinct from the others: a new pairing code is needed, not a switch.
        detail: "Pair again from your machine with a new code.",
        actionable: false,
      };
  }
}

/// A one-line summary of a transcript record, for the bare view.
///
/// Not the timeline — that lands with `@claudinio/timeline-ui`. This exists so the
/// first real connection can be judged by eye: if the records are arriving and read
/// like the session, the transport works.
export function summariseRecord(record: Record<string, unknown>): string {
  const kind = typeof record.kind === "string" ? record.kind : "?";
  for (const field of ["content", "text", "summary", "title", "toolName", "stopReason"]) {
    const value = record[field];
    if (typeof value === "string" && value.trim()) {
      return `${kind}: ${ellipsis(value.trim(), 160)}`;
    }
  }
  return kind;
}

function ellipsis(text: string, max: number): string {
  const flat = text.replace(/\s+/g, " ");
  return flat.length <= max ? flat : `${flat.slice(0, max - 1)}…`;
}
