import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { getDocument, type DocView } from "../lib/ipc";
import { publishDocument, pendingSubagentRequest, clearSubagentRequest } from "../lib/documentStore";
import { DocumentPanel } from "./DocumentPanel";

vi.mock("../lib/ipc", () => ({
  getDocument: vi.fn(),
}));

const DOC: DocView = {
  path: ".claudinio/docs/pudim-de-leite.md",
  title: "Pudim de leite",
  sections: [
    { id: "s1", topic: "origem do pudim", status: "done", owner: "main", line: 3 },
    { id: "s2", topic: "ingredientes originais", status: "doing", owner: "sub:ingredientes", line: 7 },
    { id: "s3", topic: "Como servir", status: "human", owner: null, line: 9 },
    { id: "s4", topic: "Como armazenar", status: "todo", owner: null, line: 12 },
  ],
};

const flush = () => new Promise((r) => setTimeout(r, 0));

async function mount(doc: DocView | null, workspace = "/ws") {
  vi.mocked(getDocument).mockResolvedValue(doc);
  const onOpen = vi.fn();
  const onDocumentChange = vi.fn();
  const dispose = render(
    () => <DocumentPanel workspace={workspace} onOpen={onOpen} onDocumentChange={onDocumentChange} />,
    document.body,
  );
  await flush();
  return { dispose, onOpen, onDocumentChange };
}

const rows = () => Array.from(document.body.querySelectorAll("[data-doc-row]"));

describe("DocumentPanel", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
    clearSubagentRequest();
  });

  it("shows nothing, and says so, for a session without a document", async () => {
    const { dispose, onDocumentChange } = await mount(null);
    expect(getDocument).toHaveBeenCalledWith("/ws");
    expect(document.body.querySelector("[data-document-panel]")).toBeNull();
    expect(onDocumentChange).toHaveBeenLastCalledWith(null);
    dispose();
  });

  it("shows the title and one row per section with its status and owner", async () => {
    const { dispose, onDocumentChange } = await mount(DOC);
    expect(onDocumentChange).toHaveBeenLastCalledWith(DOC);
    expect(document.body.textContent).toContain("Pudim de leite");
    expect(rows().length).toBe(4);
    expect(rows()[0].textContent).toContain("origem do pudim");
    expect(rows()[0].querySelector("[data-doc-status]")?.textContent).toBe("Done");
    expect(rows()[1].querySelector("[data-doc-status]")?.textContent).toBe("Doing");
    expect(rows()[1].textContent).toContain("ingredientes");
    expect(rows()[2].querySelector("[data-doc-status]")?.textContent).toBe("Edited by you");
    expect(rows()[3].querySelector("[data-doc-status]")?.textContent).toBe("Todo");
    dispose();
  });

  it("opens the document at the section's line when a row is clicked", async () => {
    const { dispose, onOpen } = await mount(DOC);
    (rows()[1].querySelector("[data-doc-open]") as HTMLElement).click();
    expect(onOpen).toHaveBeenCalledWith(DOC, 7);
    dispose();
  });

  it("asks the chat to open a subagent's timeline from its owner chip", async () => {
    const { dispose, onOpen } = await mount(DOC);
    (rows()[1].querySelector("[data-doc-owner]") as HTMLElement).click();
    expect(pendingSubagentRequest()).toMatchObject({ workspace: "/ws", owner: "sub:ingredientes" });
    expect(onOpen).not.toHaveBeenCalled();
    expect(rows()[0].querySelector("button[data-doc-owner]")).toBeNull();
    dispose();
  });

  it("follows the outline a run publishes, without asking the backend again", async () => {
    const { dispose, onDocumentChange } = await mount(DOC);
    const calls = vi.mocked(getDocument).mock.calls.length;
    const updated = {
      ...DOC,
      sections: DOC.sections.map((s) => (s.id === "s2" ? { ...s, status: "done" } : s)),
    };
    publishDocument("/ws", updated);
    await flush();
    expect(rows()[1].querySelector("[data-doc-status]")?.textContent).toBe("Done");
    // App learns the document's path from it, to route its saves.
    expect(onDocumentChange).toHaveBeenLastCalledWith(updated);
    publishDocument("/other", { ...DOC, title: "Not this one" });
    await flush();
    expect(document.body.textContent).not.toContain("Not this one");
    expect(vi.mocked(getDocument).mock.calls.length).toBe(calls);
    dispose();
  });
});
