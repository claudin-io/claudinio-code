import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import type { ToolCallData, ToolResultData } from "../../lib/ipc";

vi.mock("../../lib/grill-me", () => ({
  t: (key: string) => key,
}));

vi.mock("../Icon", () => ({
  Icon: (props: { name: string; class?: string }) => <span data-testid={`icon-${props.name}`} class={props.class} />,
}));

vi.mock("../DiffViewer", () => ({
  DiffViewer: (props: { original: string; modified: string; language?: string }) => (
    <div data-testid="diff-viewer" data-language={props.language}>
      {props.original}|{props.modified}
    </div>
  ),
}));

import { ToolBody } from "./ToolBody";
import { toolTitle, toolSummary, alwaysShowsBody, detectLanguageFromPath } from "./toolPresentation";

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
