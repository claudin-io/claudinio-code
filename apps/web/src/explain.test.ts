import { describe, it, expect } from "vitest";
import {
  explainAuthFailure,
  explainAuthorizing,
  explainClaiming,
  explainClose,
  explainCodeError,
  explainStaleCode,
  explainState,
  explainTypedReady,
} from "./explain";
import type { CloseReason } from "./session";

describe("explaining a missing or broken code", () => {
  /// Arriving at the page directly is the ordinary way in, not a failure. It must not
  /// read as one.
  it("treats an absent code as an invitation, not an error", () => {
    const explanation = explainCodeError({ kind: "absent" });

    expect(explanation.headline).not.toMatch(/error|invalid|failed/i);
    expect(explanation.detail).toContain("Settings");
    expect(explanation.actionable).toBe(true);
  });

  /// A mangled code is usually a copy-paste that lost part of the URL, so naming the
  /// part that is wrong is the fastest route to a fix.
  it("names what is wrong with a malformed code", () => {
    const explanation = explainCodeError({
      kind: "malformed",
      field: "relayUrl",
      why: "the relay URL must be wss://, not ws",
    });

    expect(explanation.detail).toContain("must be wss://");
  });

  /// And it must not suggest fixing the link by hand, which is how someone talks
  /// themselves into pasting a code from a stranger.
  it("sends the user for a fresh code rather than suggesting an edit", () => {
    const explanation = explainCodeError({ kind: "malformed", field: "channel", why: "too short" });
    expect(explanation.detail).toContain("fresh code");
  });

  /// "Expired" and "corrupted" have different fixes. Two minutes is short on purpose,
  /// so a lapsed code is the common case and deserves its own message.
  it("distinguishes a lapsed code from a broken one", () => {
    const stale = explainStaleCode();
    const broken = explainCodeError({ kind: "malformed", field: "channel", why: "too short" });

    expect(stale.headline).not.toBe(broken.headline);
    expect(stale.headline).toMatch(/expired/i);
    expect(stale.detail).toContain("two minutes");
  });
});

describe("explaining a session state", () => {
  /// The device may simply not be listening yet. That is not a failure and must not
  /// read as one, or the user goes looking for a problem that does not exist.
  it("does not call waiting for the device an error", () => {
    const explanation = explainState({ kind: "connecting" });

    expect(explanation.headline).not.toMatch(/error|failed|problem/i);
    expect(explanation.actionable).toBe(false);
  });

  /// The user has no relay. They have a machine.
  it("talks about the machine rather than the relay", () => {
    const explanation = explainState({ kind: "connecting" });
    expect(explanation.headline).toContain("machine");
    expect(`${explanation.headline} ${explanation.detail}`).not.toMatch(/relay/i);
  });

  /// The words are the security boundary. Telling the user to compare them is not
  /// enough — they have to know what a mismatch would mean, or "they look about
  /// right" becomes the answer.
  it("says what a mismatch would mean", () => {
    const explanation = explainState({ kind: "confirming", sas: "a · b · c" });

    expect(explanation.detail).toContain("sitting between");
    expect(explanation.detail).toContain("nothing is shared");
    expect(explanation.actionable).toBe(true);
  });

  it("says when it is connected, briefly", () => {
    const explanation = explainState({ kind: "live", sas: "a · b · c" });
    expect(explanation.headline).toBe("Connected");
    expect(explanation.detail).toBeUndefined();
  });

  it("surfaces why a connection failed", () => {
    const explanation = explainState({
      kind: "failed",
      why: "the device did not answer with the key this code names",
    });
    expect(explanation.detail).toContain("did not answer with the key");
  });
});

describe("explaining a close", () => {
  const reasons: CloseReason[] = [
    "peer_asked",
    "turned_off_locally",
    "grant_expired",
    "revoked",
  ];

  it("has its own message for every reason", () => {
    const headlines = reasons.map((reason) => explainClose(reason).headline);
    expect(new Set(headlines).size).toBe(reasons.length);
  });

  /// The one thing every close has to convey: waiting here will not help. Three of
  /// these need someone at the machine, and the fourth needs a new pairing code.
  it("never suggests that waiting will fix it", () => {
    for (const reason of reasons) {
      const explanation = explainClose(reason);
      expect(explanation.actionable, reason).toBe(false);
      expect(`${explanation.headline} ${explanation.detail}`, reason).toMatch(
        /machine|Claudinio Code|pair again/i,
      );
    }
  });

  /// Revocation is not a switch. Distinguishing it matters: turning remote access on
  /// again will not let a revoked browser back in.
  it("says a revoked browser needs a new code, not a switch", () => {
    const explanation = explainClose("revoked");
    expect(explanation.detail).toMatch(/new code/i);
  });

  it("matches the reason the session reports", () => {
    expect(explainState({ kind: "closed", reason: "grant_expired" })).toEqual(
      explainClose("grant_expired"),
    );
  });
});

describe("explaining the typed-code path", () => {
  /// The sign-in is about the code, not about remote access. Someone who reads it as
  /// "this needs an account" will not go back and try the QR, which needs none.
  it("says the sign-in is only for the lookup", () => {
    const explanation = explainAuthorizing();

    expect(explanation.detail).toMatch(/sign-in/i);
    expect(explanation.detail).toMatch(/not involved/i);
    expect(explanation.actionable).toBe(false);
  });

  /// The account is named, because someone with two accounts will otherwise type a
  /// code that cannot possibly resolve and be told the code is wrong.
  it("names the account it signed in as", () => {
    expect(explainTypedReady("alice").headline).toContain("alice");
  });

  it("still says something useful with no name to show", () => {
    const explanation = explainTypedReady("");

    expect(explanation.headline).toBeTruthy();
    expect(explanation.detail).toMatch(/type the code/i);
  });

  /// A failed sign-in must not read as a dead end: scanning is right there and needs
  /// no account at all.
  it("points at the path that needs no account when the sign-in fails", () => {
    const explanation = explainAuthFailure("that authorisation has expired");

    expect(explanation.detail).toContain("that authorisation has expired");
    expect(explanation.detail).toMatch(/scanning|scan/i);
    expect(explanation.detail).toMatch(/no account/i);
    expect(explanation.actionable).toBe(true);
  });

  /// Waiting is the answer while a code is being looked up, so there must be nothing
  /// to act on — a button here invites a second attempt against a single-use code.
  it("offers nothing to do while a code is being looked up", () => {
    const explanation = explainClaiming();

    expect(explanation.actionable).toBe(false);
    expect(explanation.detail).toBeUndefined();
  });

  /// The rule the whole module follows, applied to the four newest messages.
  it("never calls an ordinary state an error", () => {
    for (const explanation of [
      explainAuthorizing(),
      explainClaiming(),
      explainTypedReady("alice"),
    ]) {
      expect(explanation.headline).not.toMatch(/error|invalid|failed/i);
    }
  });
});
