import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import type { ToolCallData, ToolResultData } from "../../lib/ipc";


vi.mock("../Icon", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../Icon")>()),
  Icon: (props: { name: string; class?: string }) => <span data-testid={`icon-${props.name}`} class={props.class} />,
}));

vi.mock("../DiffViewer", () => ({
  DiffViewer: (props: { original: string; modified: string; language?: string }) => (
    <div data-testid="diff-viewer" data-language={props.language}>
      {props.original}|{props.modified}
    </div>
  ),
}));

import { ToolBody, parseJsonList } from "./ToolBody";
import { toolTitle, toolSummary, toolHeader, alwaysShowsBody, detectLanguageFromPath } from "./toolPresentation";

function makeCall(toolName: string, args: Record<string, unknown>): ToolCallData {
  return { sessionId: "s", toolId: "t", toolName, args, permission: "auto" };
}

function makeResult(toolName: string, output: string, error?: string | null): ToolResultData {
  return { toolId: "t", toolName, output, error };
}

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("toolPresentation", () => {
  it("humanizes known tool names", () => {
    expect(toolTitle("bash")).toBe("Ran command");
    expect(toolTitle("edit_file")).toBe("Edited file");
    expect(toolTitle("tasks_set")).toBe("Updated tasks");
  });

  it("humanizes mcp__ tool names by stripping the prefix", () => {
    expect(toolTitle("mcp__github__list_issues")).toBe("list issues");
  });

  it("falls back to underscored-to-spaced for unknown tools", () => {
    expect(toolTitle("some_future_tool")).toBe("some future tool");
  });

  it("summarizes bash as the truncated command", () => {
    const call = makeCall("bash", { command: "git status" });
    expect(toolSummary(call)).toBe("git status");
  });

  it("drops the redundant command and flags from bash search summaries", () => {
    expect(toolSummary(makeCall("bash", { command: "grep -o 'v0\\.1\\.5' dist/index.js | head -5" }))).toBe(
      "'v0\\.1\\.5' dist/index.js | head -5",
    );
    expect(toolSummary(makeCall("bash", { command: "rg -n --hidden pattern src/" }))).toBe("pattern src/");
  });

  it("summarizes tasks_set as a done/total ratio", () => {
    const call = makeCall("tasks_set", {
      tasks: [{ status: "done" }, { status: "doing" }, { status: "done" }],
    });
    expect(toolSummary(call)).toBe("2/3 done");
  });

  it("summarizes tasks_update as the task and what changed", () => {
    expect(toolTitle("tasks_update")).toBe("Updated task");
    expect(toolSummary(makeCall("tasks_update", { id: "t3", status: "done" }))).toBe("t3 → done");
    expect(toolSummary(makeCall("tasks_update", { id: "t3", journal: ["found it"] }))).toBe("t3 · 1 note");
    expect(toolSummary(makeCall("tasks_update", { id: "t3", journal: ["a", "b"] }))).toBe("t3 · 2 notes");
    expect(toolSummary(makeCall("tasks_update", { id: "t3" }))).toBe("t3");
  });

  it("summarizes ask_user with a single question inline", () => {
    const call = makeCall("ask_user", { questions: [{ question: "Proceed?", options: ["Yes", "No"] }] });
    expect(toolSummary(call)).toBe("Proceed?");
  });

  it("marks edit_file, ask_user and tasks_set as always-shown", () => {
    expect(alwaysShowsBody("edit_file")).toBe(true);
    expect(alwaysShowsBody("ask_user")).toBe(true);
    expect(alwaysShowsBody("tasks_set")).toBe(true);
    expect(alwaysShowsBody("bash")).toBe(false);
  });

  it("detects language from file extension", () => {
    expect(detectLanguageFromPath("src/foo.rs")).toBe("rust");
    expect(detectLanguageFromPath("src/foo.unknownext")).toBe("plaintext");
  });

  it("reclassifies bash grep/rg commands as Searched files", () => {
    expect(toolHeader(makeCall("bash", { command: "grep -o 'v0' dist/index.js | head -5" }))).toEqual({
      icon: "search",
      title: "Searched files",
    });
    expect(toolHeader(makeCall("bash", { command: "rg pattern src/" })).title).toBe("Searched files");
    expect(toolHeader(makeCall("bash", { command: "FOO=1 grep pattern file" })).title).toBe("Searched files");
    expect(toolHeader(makeCall("bash", { command: "/usr/bin/grep pattern file" })).title).toBe("Searched files");
  });

  it("titles an edit_file with an empty old_string as a created file", () => {
    expect(toolHeader(makeCall("edit_file", { path: "a.md", old_string: "", new_string: "x" })).title).toBe(
      "Created file",
    );
    expect(toolHeader(makeCall("edit_file", { path: "a.md", old_string: "x", new_string: "y" })).title).toBe(
      "Edited file",
    );
  });

  // "Created" claims a file now exists on disk: a creation the person
  // rejected, or one the harness refused (the file exists, the path is
  // relative), did not happen and must not be titled as if it had.
  it("titles a creation that did not happen as not created", () => {
    const call = makeCall("edit_file", { path: "a.md", old_string: "", new_string: "x" });
    expect(toolHeader(call, makeResult("edit_file", "Created a.md")).title).toBe("Created file");
    expect(toolHeader(call, makeResult("edit_file", "Edit rejected by user")).title).toBe("File not created");
    expect(toolHeader(call, makeResult("edit_file", "", "a.md already exists, ...")).title).toBe("File not created");
    // A reloaded session carries the refusal as plain output, no error flag.
    expect(toolHeader(call, makeResult("edit_file", "Error: a.md is relative; ...")).title).toBe("File not created");
  });

  it("keeps Ran command for bash that merely mentions grep", () => {
    expect(toolHeader(makeCall("bash", { command: "cat file | grep foo" })).title).toBe("Ran command");
    expect(toolHeader(makeCall("bash", { command: "git status" })).title).toBe("Ran command");
  });
});

describe("parseJsonList", () => {
  it("parses a complete JSON array", () => {
    expect(parseJsonList('[{"a":1},{"a":2}]')).toEqual({ list: [{ a: 1 }, { a: 2 }], truncated: false });
  });

  it("returns null for non-array JSON and plain text", () => {
    expect(parseJsonList('{"a":1}')).toBeNull();
    expect(parseJsonList("plain text answer")).toBeNull();
  });

  it("salvages complete objects from a truncated array", () => {
    const out = parseJsonList('[{"a":1},{"b":"x}y"},{"c":...(truncated, 999 chars total)');
    expect(out).toEqual({ list: [{ a: 1 }, { b: "x}y" }], truncated: true });
  });

  it("returns null when nothing can be salvaged", () => {
    expect(parseJsonList('[{"a":...(truncated)')).toBeNull();
  });

  it("unwraps the semantic_search envelope", () => {
    const out = parseJsonList(
      '{"mode":"hybrid","note":"3 files still embedding","results":[{"name":"foo","matchType":"lexical"}]}'
    );
    expect(out).toEqual({ list: [{ name: "foo", matchType: "lexical" }], truncated: false });
  });

  it("salvages complete objects from a truncated envelope", () => {
    const out = parseJsonList(
      '{"mode":"hybrid","results":[{"a":1},{"b":"x}y"},{"c":...(truncated, 999 chars total)'
    );
    expect(out).toEqual({ list: [{ a: 1 }, { b: "x}y" }], truncated: true });
  });

  it("returns null for a truncated envelope with no results array", () => {
    expect(parseJsonList('{"mode":"hybrid","note":"still emb...(truncated)')).toBeNull();
  });
});

describe("ToolBody", () => {
  it("renders a bash command and its stdout", () => {
    const call = makeCall("bash", { command: "echo hi" });
    const result = makeResult("bash", "hi\n");
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.textContent).toContain("echo hi");
    expect(document.body.textContent).toContain("hi");
    dispose();
  });

  it("shows bash stderr in the error style when the command fails", () => {
    const call = makeCall("bash", { command: "false" });
    const result = makeResult("bash", "", "command failed");
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.textContent).toContain("command failed");
    dispose();
  });

  it("routes edit_file to DiffViewer with old/new strings", () => {
    const call = makeCall("edit_file", { path: "src/foo.ts", old_string: "a", new_string: "b" });
    const dispose = render(() => <ToolBody call={call} />, document.body);
    const diff = document.querySelector('[data-testid="diff-viewer"]');
    expect(diff).not.toBeNull();
    expect(diff?.getAttribute("data-language")).toBe("typescript");
    dispose();
  });

  // An empty old_string creates the file (lean-harness Phase 3b): the
  // timeline says so, and the diff runs from nothing.
  it("labels an edit_file that creates a file as a new file, every line added", () => {
    const call = makeCall("edit_file", { path: "docs/NEW.md", old_string: "", new_string: "# New\n" });
    const dispose = render(() => <ToolBody call={call} />, document.body);
    expect(document.body.textContent).toContain("New file");
    expect(document.querySelector('[data-testid="diff-viewer"]')?.textContent).toBe("|# New\n");
    dispose();
  });

  it("does not label an ordinary edit as a new file", () => {
    const call = makeCall("edit_file", { path: "src/foo.ts", old_string: "a", new_string: "b" });
    const dispose = render(() => <ToolBody call={call} />, document.body);
    expect(document.body.textContent).not.toContain("New file");
    dispose();
  });

  it("renders read_file content", () => {
    const call = makeCall("read_file", { path: "src/foo.ts" });
    const result = makeResult("read_file", "const x = 1;");
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.textContent).toContain("const x = 1;");
    dispose();
  });

  it("parses ask_user Pergunta/Resposta pairs from the result", () => {
    const call = makeCall("ask_user", {
      questions: [{ question: "Proceed?", options: ["Yes", "No"] }],
    });
    const result = makeResult("ask_user", "Pergunta: Proceed?\nResposta: Yes");
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.textContent).toContain("Proceed?");
    expect(document.body.textContent).toContain("Yes");
    dispose();
  });

  it("shows pending questions when ask_user has no result yet", () => {
    const call = makeCall("ask_user", {
      questions: [{ question: "Proceed?", options: ["Yes", "No"] }],
    });
    const dispose = render(() => <ToolBody call={call} />, document.body);
    expect(document.body.textContent).toContain("Proceed?");
    expect(document.body.textContent).toContain("Yes");
    dispose();
  });

  it("renders object-shaped ask_user options without [object Object]", () => {
    // Raw model args may carry {label, description} or description-only options.
    const call = makeCall("ask_user", {
      questions: [
        {
          question: "Proceed?",
          options: [{ label: "Ship it", description: "tag and push" }, { description: "Cancel" }],
        },
      ],
    });
    const dispose = render(() => <ToolBody call={call} />, document.body);
    expect(document.body.textContent).toContain("Ship it");
    expect(document.body.textContent).toContain("Cancel");
    expect(document.body.textContent).not.toContain("[object Object]");
    dispose();
  });

  it("renders tasks_set as a checklist with status icons", () => {
    const call = makeCall("tasks_set", {
      tasks: [
        { id: "1", title: "Do the thing", status: "done", journal: ["done note"] },
        { id: "2", title: "Do another thing", status: "doing", journal: [] },
      ],
    });
    const dispose = render(() => <ToolBody call={call} />, document.body);
    expect(document.body.textContent).toContain("Do the thing");
    expect(document.body.textContent).toContain("Do another thing");
    expect(document.querySelector('[data-testid="icon-check-circle"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="icon-loader"]')).not.toBeNull();
    dispose();
  });

  it("renders finalize_plan journal as markdown", () => {
    const call = makeCall("finalize_plan", { journal: "**bold** text", plan_file: "docs/plan.md" });
    const dispose = render(() => <ToolBody call={call} />, document.body);
    expect(document.body.innerHTML).toContain("<strong>bold</strong>");
    expect(document.body.textContent).toContain("docs/plan.md");
    dispose();
  });

  it("renders spawn_agents as mini agent cards", () => {
    const call = makeCall("spawn_agents", {
      agents: [{ name: "explorer", goal: "map the repo", mode: "explore" }],
    });
    const dispose = render(() => <ToolBody call={call} />, document.body);
    expect(document.body.textContent).toContain("explorer");
    expect(document.body.textContent).toContain("map the repo");
    expect(document.body.textContent).toContain("explore");
    dispose();
  });

  it("renders list_dir line output as a file list, folders marked", () => {
    const call = makeCall("list_dir", { path: "/ws/src" });
    const result = makeResult("list_dir", "/ws/src/\nagent/\nmain.rs");
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.textContent).toContain("agent");
    expect(document.body.textContent).toContain("main.rs");
    expect(document.querySelectorAll('[data-testid="icon-folder"]').length).toBe(1);
    expect(document.querySelectorAll('[data-testid="icon-file"]').length).toBe(1);
    expect(document.querySelector("pre")).toBeNull();
    dispose();
  });

  it("renders grep line output as file:line rows", () => {
    const call = makeCall("grep", { pattern: "x" });
    const result = makeResult("grep", "2 matches in 1 file\n\n/ws/a.rs\n12: let x = 1;\n40: let y = x;");
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.textContent).toContain("/ws/a.rs:12");
    expect(document.body.textContent).toContain("let y = x;");
    expect(document.body.textContent).not.toContain("2 matches in 1 file");
    dispose();
  });

  it("says No results for a search that found nothing", () => {
    const call = makeCall("grep", { pattern: "x" });
    const dispose = render(
      () => <ToolBody call={call} result={makeResult("grep", "No matches.")} />,
      document.body,
    );
    expect(document.body.textContent).toContain("No results");
    dispose();
  });

  it("shows a list tool's error text as it is", () => {
    const call = makeCall("list_dir", { path: "/ws/x" });
    const result = makeResult("list_dir", "Error: not a directory: /ws/x");
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.querySelector("pre")?.textContent).toContain("not a directory");
    dispose();
  });

  it("shows a repeated read as a note, not as the file's content", () => {
    const call = makeCall("read_file", { path: "src/foo.ts" });
    const result = makeResult(
      "read_file",
      "[unchanged: src/foo.ts is identical to what your earlier read_file of it returned (40 lines). That result is still in this conversation and is not repeated here. If you cannot find it, call read_file again and the content is returned.]",
    );
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.querySelector("[data-unchanged-read]")?.textContent).toContain("earlier read");
    expect(document.querySelector("pre")).toBeNull();
    dispose();
  });

  it("renders list_dir JSON output as a file list", () => {
    const call = makeCall("list_dir", { path: "src" });
    const result = makeResult(
      "list_dir",
      JSON.stringify([{ name: "foo.ts", path: "src/foo.ts", isDir: false }]),
    );
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.textContent).toContain("foo.ts");
    dispose();
  });

  it("renders code_search symbol results as name + kind + location rows", () => {
    const call = makeCall("code_search", { query: "ToolRow" });
    const result = makeResult(
      "code_search",
      JSON.stringify([
        {
          symbolId: 82943,
          name: "ToolRow",
          kind: "function_declaration",
          filePath: "/repo/src/components/ChatPanel.tsx",
          startLine: 3140,
          signature: "ToolRow: Component<...>",
          snippet: "const ToolRow = ...",
        },
      ]),
    );
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.textContent).toContain("ToolRow");
    expect(document.body.textContent).toContain("function_declaration");
    expect(document.body.textContent).toContain("/repo/src/components/ChatPanel.tsx:3140");
    // The raw snippet noise must not leak into the row
    expect(document.body.textContent).not.toContain("const ToolRow = ...");
    dispose();
  });

  it("renders grep matches as file:line + matched text", () => {
    const call = makeCall("grep", { pattern: "foo" });
    const result = makeResult(
      "grep",
      JSON.stringify([{ file: "src/a.ts", line: 12, content: "  const foo = 1;" }]),
    );
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.textContent).toContain("src/a.ts:12");
    expect(document.body.textContent).toContain("const foo = 1;");
    dispose();
  });

  it("salvages rows from a JSON array truncated mid-object", () => {
    const full = JSON.stringify([
      { name: "A", filePath: "/x/a.ts", kind: "function", startLine: 1 },
      { name: "B", filePath: "/x/b.ts", kind: "class", startLine: 2 },
    ]);
    const truncated = `${full.slice(0, full.length - 20)}...(truncated, ${full.length} chars total)`;
    const call = makeCall("semantic_search", { query: "anything" });
    const result = makeResult("semantic_search", truncated);
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    // First object is complete and must render as a row, not raw JSON
    expect(document.body.textContent).toContain("A");
    expect(document.body.textContent).toContain("/x/a.ts:1");
    expect(document.body.textContent).not.toContain("symbolId");
    expect(document.body.textContent).not.toContain('"name"');
    dispose();
  });

  it("falls back to plain text when list-tool output isn't JSON", () => {
    const call = makeCall("web_search", { query: "solidjs" });
    const result = makeResult("web_search", "Solid is a reactive JS framework.");
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.textContent).toContain("Solid is a reactive JS framework.");
    dispose();
  });

  it("falls back to a key-value list for unknown tools", () => {
    const call = makeCall("mcp__github__list_issues", { repo: "acme/widgets" });
    const dispose = render(() => <ToolBody call={call} />, document.body);
    expect(document.body.textContent).toContain("repo");
    expect(document.body.textContent).toContain("acme/widgets");
    dispose();
  });

  // Images are appended after whatever body the tool itself renders, so any
  // tool that returns them displays them without its own renderer.
  it("renders images attached to a tool result", () => {
    const call = makeCall("mcp__playwright__screenshot", {});
    const result: ToolResultData = {
      ...makeResult("mcp__playwright__screenshot", "[image 1280x800]"),
      images: [{ mediaType: "image/jpeg", data: "QUJD", width: 1280, height: 800 }],
    };
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    const img = document.body.querySelector("img");
    expect(img).not.toBeNull();
    expect(img!.getAttribute("src")).toBe("data:image/jpeg;base64,QUJD");
    dispose();
  });

  it("renders no img element when the result carries no images", () => {
    const call = makeCall("mcp__github__list_issues", { repo: "acme/widgets" });
    const result = makeResult("mcp__github__list_issues", "none");
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.querySelector("img")).toBeNull();
    dispose();
  });
});

// Phase 4 of the lean-harness plan: one `tasks_update` call may close a task
// and start the next. The row says how many tasks moved and the body lists
// each change, whichever form the call took.
describe("batched tasks_update", () => {
  const batch = makeCall("tasks_update", {
    updates: [
      { id: "t1", status: "done", journal: ["it was the cache"] },
      { id: "t2", status: "doing" },
      { id: "t3", journal: ["a", "b"] },
    ],
  });

  it("titles a batch in the plural and summarizes each change", () => {
    expect(toolHeader(batch).title).toBe("Updated tasks");
    expect(toolSummary(batch)).toBe("t1 → done, t2 → doing, t3 · 2 notes");
    // The single form keeps its own title and summary.
    const one = makeCall("tasks_update", { id: "t3", status: "done" });
    expect(toolHeader(one).title).toBe("Updated task");
    expect(toolSummary(one)).toBe("t3 → done");
  });

  it("renders one row per change with the new status", () => {
    const dispose = render(
      () => <ToolBody call={batch} result={makeResult("tasks_update", "Tasks updated — …")} />,
      document.body,
    );
    const rows = document.body.querySelectorAll("[data-task-update]");
    expect(rows.length).toBe(3);
    expect(rows[0].textContent).toContain("t1");
    expect(rows[0].textContent).toContain("done");
    expect(rows[0].textContent).toContain("it was the cache");
    expect(rows[0].querySelector('[data-testid="icon-check-circle"]')).not.toBeNull();
    expect(rows[1].querySelector('[data-testid="icon-loader"]')).not.toBeNull();
    expect(rows[2].textContent).toContain("2 notes");
    dispose();
  });

  it("renders the single form as one row too", () => {
    const one = makeCall("tasks_update", { id: "t9", status: "doing" });
    const dispose = render(() => <ToolBody call={one} />, document.body);
    const rows = document.body.querySelectorAll("[data-task-update]");
    expect(rows.length).toBe(1);
    expect(rows[0].textContent).toContain("t9");
    dispose();
  });

  it("shows why a batch was refused", () => {
    const result = makeResult(
      "tasks_update",
      "",
      "tasks_update: there is no task 't9'. The batch was refused: nothing was applied.",
    );
    const dispose = render(() => <ToolBody call={batch} result={result} />, document.body);
    expect(document.body.textContent).toContain("nothing was applied");
    dispose();
  });
});

// The two tools Phase 2 of the lean-harness plan added: a deferred-tool load
// and a skill read are bookkeeping, so each renders as compact rows rather
// than a key-value dump of its arguments.
describe("load_tools and skills", () => {
  it("titles and summarizes a load by the tools it asked for", () => {
    const call = makeCall("load_tools", { names: ["semantic_search", "mcp__github__list_issues"] });
    expect(toolHeader(call).title).toBe("Loaded tools");
    expect(toolSummary(call)).toBe("semantic_search, mcp__github__list_issues");
  });

  it("titles a skill read and a skill listing apart", () => {
    const read = makeCall("skills", { action: "read", name: "deploy" });
    expect(toolHeader(read).title).toBe("Read a skill");
    expect(toolSummary(read)).toBe("deploy");
    const list = makeCall("skills", { action: "list" });
    expect(toolHeader(list).title).toBe("Listed skills");
    expect(toolSummary(list)).toBe("");
  });

  it("renders one row per loaded tool, by its human name", () => {
    const call = makeCall("load_tools", { names: ["semantic_search", "mcp__github__list_issues"] });
    const result = makeResult("load_tools", "Loaded: semantic_search, mcp__github__list_issues.");
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    const rows = document.body.querySelectorAll("[data-loaded-tool]");
    expect(rows.length).toBe(2);
    expect(rows[0].textContent).toContain("Searched code");
    expect(rows[0].textContent).toContain("semantic_search");
    expect(rows[1].textContent).toContain("list issues");
    dispose();
  });

  it("shows why a load failed", () => {
    const call = makeCall("load_tools", { names: ["nope"] });
    const result = makeResult("load_tools", "", "Not a deferred tool here: nope.");
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.textContent).toContain("Not a deferred tool here: nope.");
    dispose();
  });

  it("renders a skill listing as rows with their scope", () => {
    const call = makeCall("skills", { action: "list" });
    const result = makeResult(
      "skills",
      "2 skills, the user's own first:\n" +
        "- deploy [project] /ws/.agents/skills/deploy/SKILL.md\n  Ship the app\n" +
        "- find-skills [builtin] <built-in>\n  Helps users discover skills\n",
    );
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    const rows = document.body.querySelectorAll("[data-skill-row]");
    expect(rows.length).toBe(2);
    expect(rows[0].textContent).toContain("deploy");
    expect(rows[0].textContent).toContain("project");
    expect(rows[0].textContent).toContain("Ship the app");
    expect(rows[1].textContent).toContain("builtin");
    dispose();
  });

  it("renders a skill read as its instructions", () => {
    const call = makeCall("skills", { action: "read", name: "deploy" });
    const result = makeResult("skills", "# Skill: deploy\nDirectory: /ws/x\n\nRun **the script**.");
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.querySelector("strong")?.textContent).toBe("the script");
    dispose();
  });
});

// Model-supplied args are not checked against the schema before they reach
// the UI (the backend answers a malformed call with an error), so a summary
// that assumed an array threw, and the error boundary replaced the whole live
// run with its fallback (final branch review).
describe("malformed tool args", () => {
  it("summarises a load_tools call whose names is a string", () => {
    expect(toolSummary(makeCall("load_tools", { names: "semantic_search" }))).toBe("semantic_search");
    expect(toolSummary(makeCall("load_tools", { names: 3 }))).toBe("");
  });

  it("renders one row for a load_tools call whose names is a string", () => {
    const call = makeCall("load_tools", { names: "semantic_search" });
    const result = makeResult("load_tools", "", "load_tools needs the names to load");
    const dispose = render(() => <ToolBody call={call} result={result} />, document.body);
    expect(document.body.querySelectorAll("[data-loaded-tool]").length).toBe(1);
    dispose();
  });

  it("skips entries of a tasks_update batch that are not objects", () => {
    expect(toolSummary(makeCall("tasks_update", { updates: [null, { id: "t1", status: "done" }, "x"] }))).toBe(
      "t1 → done",
    );
  });

  it("summarises a tasks_set call whose tasks is not an array", () => {
    expect(toolSummary(makeCall("tasks_set", { tasks: "t1" }))).toBe("0/0 done");
  });
});

// Lean-harness Phase 4b: the living document's tools. Each reads as what it
// did to the document — started it, wrote or claimed a section, read one,
// added one — and the outline the backend answers with renders as rows.
describe("living document tools", () => {
  const OUTLINE =
    "Wrote s1 (done).\n# Pudim de leite\ns1 · done · main · origem do pudim\ns2 · doing · sub:ingredientes · ingredientes originais";

  it("titles each call by what it did to the document", () => {
    expect(toolHeader(makeCall("doc_create", { title: "Pudim", sections: ["a"] })).title).toBe(
      "Started a document",
    );
    expect(toolHeader(makeCall("doc_write", { id: "s1", body: "x" })).title).toBe("Wrote a section");
    expect(toolHeader(makeCall("doc_write", { id: "s1", status: "doing" })).title).toBe("Claimed a section");
    expect(toolHeader(makeCall("doc_read", { id: "s1" })).title).toBe("Read a section");
    expect(toolHeader(makeCall("doc_read", {})).title).toBe("Read the document");
    expect(toolHeader(makeCall("doc_add", { topic: "caramelo" })).title).toBe("Added a section");
  });

  it("summarizes the document, section or topic", () => {
    expect(toolSummary(makeCall("doc_create", { title: "Pudim", sections: ["a", "b"] }))).toBe(
      "Pudim · 2 sections",
    );
    expect(toolSummary(makeCall("doc_write", { id: "s2", body: "x" }))).toBe("s2");
    expect(toolSummary(makeCall("doc_read", {}))).toBe("outline");
    expect(toolSummary(makeCall("doc_add", { topic: "caramelo", after: "s1" }))).toBe("caramelo");
  });

  it("renders the outline a doc tool answers with as one row per section", () => {
    const call = makeCall("doc_add", { topic: "x" });
    const dispose = render(
      () => <ToolBody call={call} result={makeResult("doc_add", OUTLINE)} />,
      document.body,
    );
    const rows = document.body.querySelectorAll("[data-doc-section]");
    expect(rows.length).toBe(2);
    expect(rows[0].textContent).toContain("origem do pudim");
    expect(rows[0].textContent).toContain("done");
    expect(rows[1].textContent).toContain("sub:ingredientes");
    dispose();
  });

  it("shows what a doc_write wrote, and why a write was refused", () => {
    const call = makeCall("doc_write", { id: "s1", body: "Veio de **Portugal**." });
    let dispose = render(
      () => <ToolBody call={call} result={makeResult("doc_write", OUTLINE)} />,
      document.body,
    );
    expect(document.body.querySelector("strong")?.textContent).toBe("Portugal");
    dispose();
    document.body.innerHTML = "";
    dispose = render(
      () => (
        <ToolBody
          call={call}
          result={makeResult("doc_write", "", "s1 was edited by the user since you last read it")}
        />
      ),
      document.body,
    );
    expect(document.body.textContent).toContain("edited by the user");
    dispose();
  });
});
