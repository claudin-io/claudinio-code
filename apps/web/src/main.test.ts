import { describe, it, expect } from "vitest";
import { absorb } from "./main";
import type { DeviceMessage } from "./session";

/// A stand-in for the signal setter, so the folding is testable without a DOM.
function collector() {
  let records: Record<string, unknown>[] = [];
  return {
    set: (update: (previous: Record<string, unknown>[]) => Record<string, unknown>[]) => {
      records = update(records);
    },
    get: () => records,
  };
}

const message = (m: Record<string, unknown>) => m as DeviceMessage;

describe("absorb", () => {
  /// A snapshot is a batch and an event is one. Getting that backwards shows up as a
  /// transcript that is either empty or nested one level too deep, and it is the kind
  /// of thing that looks fine until a real session arrives.
  it("spreads a snapshot's records rather than nesting them", () => {
    const records = collector();
    absorb(
      message({
        kind: "snapshot",
        session_id: "s",
        seq: 2,
        records: [{ kind: "meta" }, { kind: "user", content: "hi" }],
      }),
      records.set,
    );

    expect(records.get()).toEqual([{ kind: "meta" }, { kind: "user", content: "hi" }]);
  });

  it("appends an event as one record", () => {
    const records = collector();
    absorb(message({ kind: "event", session_id: "s", seq: 3, event: { kind: "turn" } }), records.set);

    expect(records.get()).toEqual([{ kind: "turn" }]);
  });

  it("keeps what arrived before", () => {
    const records = collector();
    absorb(message({ kind: "snapshot", records: [{ kind: "meta" }] }), records.set);
    absorb(message({ kind: "event", event: { kind: "turn" } }), records.set);

    expect(records.get()).toHaveLength(2);
  });

  /// A `Gap` is the device telling the truth about what it could not deliver, and
  /// recovery is the resubscribe the session already does. Rendering it as transcript
  /// would put protocol noise in the middle of a conversation.
  it("does not render a gap as a record", () => {
    const records = collector();
    absorb(message({ kind: "gap", session_id: "s", from_seq: 4, to_seq: 9 }), records.set);

    expect(records.get()).toEqual([]);
  });

  /// The write path's messages belong to phase 4. Until then they must not appear as
  /// if they were part of the session.
  it("drops messages this build has nothing to do with", () => {
    const records = collector();
    for (const kind of ["approval_request", "policy_denied", "ack", "error", "policy"]) {
      absorb(message({ kind }), records.set);
    }

    expect(records.get()).toEqual([]);
  });

  /// A crafted frame can carry anything that decodes. Non-objects would render as
  /// `[object Object]` or crash the summariser, so they are filtered rather than
  /// trusted.
  it("filters a snapshot's non-records", () => {
    const records = collector();
    absorb(
      message({ kind: "snapshot", records: ["a string", 42, null, ["nested"], { kind: "real" }] }),
      records.set,
    );

    expect(records.get()).toEqual([{ kind: "real" }]);
  });

  it("ignores a snapshot whose records are not a list", () => {
    const records = collector();
    absorb(message({ kind: "snapshot", records: "not a list" }), records.set);

    expect(records.get()).toEqual([]);
  });

  it("ignores an event with nothing in it", () => {
    const records = collector();
    absorb(message({ kind: "event", event: null }), records.set);
    absorb(message({ kind: "event" }), records.set);

    expect(records.get()).toEqual([]);
  });

  it("carries an empty snapshot without complaint", () => {
    const records = collector();
    absorb(message({ kind: "snapshot", records: [] }), records.set);

    expect(records.get()).toEqual([]);
  });
});
