import { describe, it, expect } from "vitest";
import { parseLineList } from "./lineResults";

describe("parseLineList", () => {
  it("reads a directory listing, marking directories", () => {
    expect(parseLineList("list_dir", "/ws/src/\nagent/\nmain.rs")).toEqual([
      { title: "agent", isDir: true },
      { title: "main.rs" },
    ]);
    expect(parseLineList("list_dir", "/ws/empty/\n(empty)")).toEqual([]);
  });

  it("reads grep matches under the file they belong to", () => {
    const out = "3 matches in 2 files\n\n/ws/a.rs\n12: let x = 1;\n40: let y = x;\n\n/ws/b.rs\n3: use x;";
    expect(parseLineList("grep", out)).toEqual([
      { title: "/ws/a.rs:12", sub: "let x = 1;" },
      { title: "/ws/a.rs:40", sub: "let y = x;" },
      { title: "/ws/b.rs:3", sub: "use x;" },
    ]);
    expect(parseLineList("grep", "1 match in 1 file\n\n/ws/a.rs\n1: x")).toHaveLength(1);
    expect(parseLineList("grep", "No matches.")).toEqual([]);
  });

  it("keeps a matched line that itself looks like a row", () => {
    const out = "1 match in 1 file\n\n/ws/notes.md\n7: 12: not a line number";
    expect(parseLineList("grep", out)).toEqual([{ title: "/ws/notes.md:7", sub: "12: not a line number" }]);
  });

  it("reads symbol hits as signature, kind and place", () => {
    const out =
      "/ws/src/session.rs:2128 function pub async fn run_workflow( config: &AgentConfig, )\n/ws/src/budget.rs:7 const MAX";
    expect(parseLineList("code_search", out)).toEqual([
      {
        title: "pub async fn run_workflow( config: &AgentConfig, )",
        badge: "function",
        sub: "/ws/src/session.rs:2128",
      },
      { title: "MAX", badge: "const", sub: "/ws/src/budget.rs:7" },
    ]);
    expect(parseLineList("symbol_lookup", "No symbols found.")).toEqual([]);
  });

  it("reads a Windows path without mistaking the drive for the line", () => {
    expect(parseLineList("code_search", "C:\\ws\\src\\app.rs:12 function fn run()")).toEqual([
      { title: "fn run()", badge: "function", sub: "C:\\ws\\src\\app.rs:12" },
    ]);
  });

  it("reads an outline, one row per symbol with its lines", () => {
    const out = "/ws/src/app.rs (3 symbols)\n1 import use std::fmt;\n3-9 struct pub struct App\n11-40 function run";
    expect(parseLineList("file_outline", out)).toEqual([
      { title: "use std::fmt;", badge: "import", sub: "L1" },
      { title: "pub struct App", badge: "struct", sub: "L3-9" },
      { title: "run", badge: "function", sub: "L11-40" },
    ]);
    expect(parseLineList("file_outline", "/ws/x.rs (no symbols indexed)")).toEqual([]);
  });

  it("reads semantic hits and skips the source under them", () => {
    const out = [
      "mode: hybrid",
      "/ws/src/queue.rs:10-14 function fn drain() [hybrid 0.78]",
      "```",
      "fn drain() {",
      "/ws/src/fake.rs:1 function fn not_a_hit()",
      "}",
      "```",
      "/ws/src/queue.rs:20 function fn push() [lexical 1.00]",
    ].join("\n");
    expect(parseLineList("semantic_search", out)).toEqual([
      { title: "fn drain()", badge: "function", sub: "/ws/src/queue.rs:10-14" },
      { title: "fn push()", badge: "function", sub: "/ws/src/queue.rs:20" },
    ]);
    expect(parseLineList("semantic_search", "mode: lexical-only (embedding model unavailable)\nNo results.")).toEqual(
      [],
    );
  });

  it("drops the row a live result was cut in the middle of", () => {
    const out = "/ws/src/\nagent/\nmain.rs\nlib...(truncated, 5231 chars total)";
    expect(parseLineList("list_dir", out)).toEqual([{ title: "agent", isDir: true }, { title: "main.rs" }]);
  });

  it("leaves anything that is not its format to be shown as text", () => {
    expect(parseLineList("list_dir", "Error: not a directory: /ws/x")).toBeNull();
    expect(parseLineList("grep", "rg error: regex parse error")).toBeNull();
    expect(parseLineList("code_search", "Workspace index is still being built: 3/90 files indexed")).toBeNull();
    expect(parseLineList("file_outline", "something else")).toBeNull();
    expect(parseLineList("semantic_search", "something else")).toBeNull();
    // A tool that never wrote lines.
    expect(parseLineList("web_search", "/ws/src/\nagent/")).toBeNull();
    expect(parseLineList("list_dir", "")).toBeNull();
  });
});
