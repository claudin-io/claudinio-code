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
});
