import { describe, it, expect } from "vitest";
import { findEdits } from "./edits";

/// A `turn` record shaped like the ones in a real transcript.
const turn = (...blocks: unknown[]) => ({
  kind: "turn",
  role: "assistant",
  content: blocks,
  ts: 1_800_000_000_000,
});

const toolUse = (name: string, input: unknown, id = "toolu_1") => ({
  type: "tool_use",
  id,
  name,
  input,
});

describe("findEdits", () => {
  /// The exact shape a real session file contains, checked against one rather than
  /// guessed: `edit_file` with `old_string`, `new_string` and `path`.
  it("finds an edit_file call as a real transcript writes it", () => {
    const edits = findEdits(
      turn(
        toolUse("edit_file", {
          path: "src/main.rs",
          old_string: "let x = 1;",
          new_string: "let x = 2;",
        }),
      ),
    );

    expect(edits).toEqual([
      { id: "toolu_1", path: "src/main.rs", oldText: "let x = 1;", newText: "let x = 2;" },
    ]);
  });

  it("finds several edits in one turn", () => {
    const edits = findEdits(
      turn(
        toolUse("edit_file", { path: "a", old_string: "1", new_string: "2" }, "t1"),
        { type: "text", text: "and then" },
        toolUse("edit_file", { path: "b", old_string: "3", new_string: "4" }, "t2"),
      ),
    );

    expect(edits.map((e) => e.path)).toEqual(["a", "b"]);
    expect(edits.map((e) => e.id)).toEqual(["t1", "t2"]);
  });

  /// The tool surface has been renamed before. A change going undisplayed because a
  /// name moved is the failure this tolerance exists to avoid.
  it("accepts the other names this tool has had", () => {
    for (const name of ["Edit", "write_file", "Write", "str_replace"]) {
      const edits = findEdits(turn(toolUse(name, { path: "p", old_string: "a", new_string: "b" })));
      expect(edits, name).toHaveLength(1);
    }
  });

  it("accepts the argument names this tool has had", () => {
    const edits = findEdits(
      turn(toolUse("edit_file", { filePath: "p", oldString: "a", newString: "b" })),
    );

    expect(edits[0]).toMatchObject({ path: "p", oldText: "a", newText: "b" });
  });

  /// A write with no old text is a new file — a diff against nothing, not something to
  /// skip. Skipping it would hide whole files being created.
  it("treats a write with no old text as a new file", () => {
    const edits = findEdits(turn(toolUse("write_file", { path: "new.ts", content: "hello\n" })));

    expect(edits).toHaveLength(1);
    expect(edits[0]).toMatchObject({ oldText: "", newText: "hello\n" });
  });

  /// Both empty means the call carried nothing. An empty diff there would claim a
  /// change was inspected when there was nothing to inspect.
  it("ignores a call with nothing in it", () => {
    expect(findEdits(turn(toolUse("edit_file", { path: "p" })))).toEqual([]);
    expect(findEdits(turn(toolUse("edit_file", { path: "p", old_string: "", new_string: "" })))).toEqual(
      [],
    );
  });

  // ── everything that is not an edit ───────────────────────────────────────

  it("ignores the other tools", () => {
    const edits = findEdits(
      turn(
        toolUse("bash", { command: "ls" }),
        toolUse("read_file", { path: "a", start_line: 1, end_line: 9 }),
        toolUse("grep", { pattern: "x" }),
      ),
    );

    expect(edits).toEqual([]);
  });

  /// Called on every record, so most calls are on something else entirely and must
  /// return quietly.
  it("returns nothing for the records that are not turns", () => {
    for (const record of [
      { kind: "meta", session_id: "s", created_at: 1, workspace: null },
      { kind: "user", text: "hello", ts: 1 },
      { kind: "done", input_tokens: 1, output_tokens: 2, ts: 3 },
      { kind: "turn", role: "user", content: "a plain string", ts: 1 },
    ]) {
      expect(findEdits(record), record.kind).toEqual([]);
    }
  });

  it("survives anything at all", () => {
    for (const junk of [null, undefined, 42, "text", [], {}, { kind: "turn" }]) {
      expect(findEdits(junk)).toEqual([]);
    }
  });

  /// A crafted frame can carry whatever decodes. A block whose `input` is a string, or
  /// whose `name` is a number, must not throw in the middle of rendering a transcript.
  it("survives a malformed tool call", () => {
    expect(findEdits(turn({ type: "tool_use", name: 7, input: "nope" }))).toEqual([]);
    expect(findEdits(turn({ type: "tool_use", name: "edit_file", input: "nope" }))).toEqual([]);
    expect(findEdits(turn(null, 42, "text"))).toEqual([]);
  });

  /// Without an id, two edits to the same path in one turn would collide as keys and
  /// one would not render.
  it("makes an id up when the block has none", () => {
    const edits = findEdits(
      turn(
        { type: "tool_use", name: "edit_file", input: { path: "p", old_string: "a", new_string: "b" } },
        { type: "tool_use", name: "edit_file", input: { path: "p", old_string: "c", new_string: "d" } },
      ),
    );

    expect(edits).toHaveLength(2);
    expect(edits[0].id).not.toBe(edits[1].id);
  });
});
