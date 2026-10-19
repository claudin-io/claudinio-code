import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";

vi.mock("monaco-editor", () => ({
  editor: { create: vi.fn(), createDiffEditor: vi.fn(), defineTheme: vi.fn() },
}));

vi.mock("../DiffViewer", () => ({
  DiffViewer: (props: { original: string; modified: string }) => (
    <div data-testid="diff-viewer">
      {props.original}|{props.modified}
    </div>
  ),
}));

import { ToolRow } from "./TimelineRows";
import type { ToolCallData, ToolResultData } from "../../lib/ipc";

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

const create: ToolCallData = {
  sessionId: "s",
  toolId: "t",
  toolName: "edit_file",
  args: { path: "/w/docs/NEW.md", old_string: "", new_string: "# New\n" },
  permission: "requires_approval",
};

function mount(result?: ToolResultData): HTMLElement {
  dispose = render(
    () => <ToolRow tool={{ call: create, result, status: "done" }} isExpanded={false} onToggle={() => {}} />,
    document.body,
  );
  return document.body;
}

describe("ToolRow", () => {
  // The row's title reads the result: a creation the person rejected did not
  // happen, so the timeline does not say "Created file".
  it("titles a rejected creation as not created", () => {
    const el = mount({ toolId: "t", toolName: "edit_file", output: "Edit rejected by user" });
    expect(el.textContent).toContain("File not created");
    expect(el.textContent).not.toContain("Created file");
  });

  it("titles a creation that succeeded as created", () => {
    const el = mount({ toolId: "t", toolName: "edit_file", output: "Created /w/docs/NEW.md" });
    expect(el.textContent).toContain("Created file");
  });
});
