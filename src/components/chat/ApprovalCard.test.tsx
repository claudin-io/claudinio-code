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

import { ApprovalCard } from "./TimelineRows";
import type { ToolCallData } from "../../lib/ipc";

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

function editCall(oldString: string, newString: string): ToolCallData {
  return {
    sessionId: "s",
    toolId: "t",
    toolName: "edit_file",
    args: { path: "/w/docs/NEW.md", old_string: oldString, new_string: newString },
    permission: "requires_approval",
    editProposal: {
      path: "/w/docs/NEW.md",
      oldString,
      newString,
      unifiedDiff: "",
    },
  };
}

function mount(call: ToolCallData): HTMLElement {
  dispose = render(
    () => <ApprovalCard toolCall={call} isActive={() => false} onApprove={() => {}} onReject={() => {}} />,
    document.body,
  );
  return document.body;
}

describe("ApprovalCard", () => {
  // edit_file with an empty old_string creates the file (lean-harness Phase
  // 3b); the person approving it is told it is a new file, and the diff runs
  // from nothing, so every line shows as added.
  it("labels a creation as a new file with every line added", () => {
    const el = mount(editCall("", "# Notes\nfinal\n"));
    expect(el.textContent).toContain("New file");
    expect(el.textContent).not.toContain("Proposed edit");
    expect(el.querySelector('[data-testid="diff-viewer"]')?.textContent).toBe("|# Notes\nfinal\n");
  });

  // The diff shows one old-to-new snippet; with replace_all the edit rewrites
  // every occurrence, so the person approving it is told how many.
  it("says how many places a replace_all edit rewrites", () => {
    const call = editCall("retry()", "go()");
    call.editProposal = { ...call.editProposal!, replaceAll: true, occurrences: 3 };
    const el = mount(call);
    expect(el.textContent).toContain("Replaces all 3 occurrences");
  });

  // replace_all accepts a text that occurs once; "Replaces all 1 occurrences"
  // read as a bug in the card.
  it("says a replace_all edit that matches once replaces one occurrence", () => {
    const call = editCall("retry()", "go()");
    call.editProposal = { ...call.editProposal!, replaceAll: true, occurrences: 1 };
    const el = mount(call);
    expect(el.textContent).toContain("Replaces 1 occurrence");
    expect(el.textContent).not.toContain("occurrences");
  });

  it("says nothing about occurrences for a single-place edit", () => {
    const call = editCall("retry()", "go()");
    call.editProposal = { ...call.editProposal!, replaceAll: false, occurrences: 1 };
    const el = mount(call);
    expect(el.textContent).not.toContain("occurrence");
  });

  it("keeps the proposed-edit label for a change to an existing file", () => {
    const el = mount(editCall("draft", "final"));
    expect(el.textContent).toContain("Proposed edit");
    expect(el.textContent).not.toContain("New file");
  });
});
