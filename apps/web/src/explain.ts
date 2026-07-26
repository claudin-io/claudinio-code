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

// ── the typed-code path ────────────────────────────────────────────────────
//
// A second way in, for when the camera is not an option: a desktop browser, a phone
// that will not focus, a code read out over a call. It costs a sign-in, because a ten
// character code is only acceptable if the account that minted it is the only account
// that can resolve it — so every message here has to make clear that the sign-in is
// about *this* code and not about remote access needing an account. It does not: the
// QR path never touches any of this.

/// Waiting on the round trip to the account server.
export function explainAuthorizing(): Explanation {
  return {
    headline: "Checking your account",
    // Named, because the user is coming back from somewhere else and a page that
    // simply says "loading" leaves them wondering whether the trip worked.
    detail: "Just the sign-in — your session and your files are not involved.",
    actionable: false,
  };
}

/// The sign-in did not complete.
export function explainAuthFailure(why: string): Explanation {
  return {
    headline: "That sign-in did not complete",
    // The server's own sentence when it had one: "expired" and "belongs to another
    // account" have different fixes, and the second one the user cannot fix at all.
    detail: `${why}. You can try again, or pair by scanning a code instead — that needs no account.`,
    actionable: true,
  };
}

/// Signed in, waiting for the code to be typed.
export function explainTypedReady(login: string): Explanation {
  return {
    // The account is named, because someone with two accounts will otherwise type a
    // code that cannot possibly resolve and be told the code is wrong.
    headline: login ? `Signed in as ${login}` : "Signed in",
    detail: "Type the code shown in Claudinio Code on your machine.",
    actionable: true,
  };
}

/// Resolving the code.
export function explainClaiming(): Explanation {
  return {
    headline: "Looking up that code",
    actionable: false,
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
