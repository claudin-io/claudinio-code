import { describe, it, expect } from "vitest";
import {
  documentFromEvent,
  invalidateDocument,
  isLinkedDocument,
  publishDocument,
  publishedDocument,
  subagentForOwner,
} from "./documentStore";
import type { AgentEvent, DocView } from "./ipc";

const DOC: DocView = {
  path: ".claudinio/docs/pudim.md",
  title: "Pudim",
  sections: [{ id: "s1", topic: "origem", status: "todo", owner: null, line: 3 }],
};

describe("documentStore", () => {
  it("reads the document from a DocumentUpdated event, also one a subagent sent", () => {
    const direct: AgentEvent = { event: "DocumentUpdated", data: { sessionId: "s", document: DOC } };
    expect(documentFromEvent(direct)).toEqual(DOC);
    const nested: AgentEvent = { event: "Subagent", data: { subagentId: "s:sub:t:0", event: direct } };
    expect(documentFromEvent(nested)).toEqual(DOC);
    expect(documentFromEvent({ event: "TextStep", data: { text: "x" } })).toBeNull();
  });

  it("keeps the latest outline per workspace and forgets it when the session changes", () => {
    publishDocument("/ws-a", DOC);
    expect(publishedDocument("/ws-a").doc).toEqual(DOC);
    expect(publishedDocument("/ws-b").doc).toBeUndefined();
    const before = publishedDocument("/ws-a").version;
    invalidateDocument("/ws-a");
    expect(publishedDocument("/ws-a").doc).toBeUndefined();
    expect(publishedDocument("/ws-a").version).toBeGreaterThan(before);
  });

  it("finds the subagent an owner names, the latest of a repeated name", () => {
    const subs = [
      { id: "a", name: "ingredientes originais" },
      { id: "b", name: "origem" },
      { id: "c", name: "ingredientes originais" },
    ];
    expect(subagentForOwner("sub:ingredientes-originais", subs)).toBe("c");
    expect(subagentForOwner("sub:origem", subs)).toBe("b");
    expect(subagentForOwner("main", subs)).toBeNull();
    expect(subagentForOwner("sub:nobody", subs)).toBeNull();
  });

  it("matches a name the harness had to clean for its comment", () => {
    // The backend keeps letters, digits, _ and - (subagent_owner), so a
    // name with '>' or '=' still opens its own timeline.
    const subs = [
      { id: "a", name: "a-->" },
      { id: "b", name: "x=1 <b>" },
      { id: "c", name: "<>" },
    ];
    expect(subagentForOwner("sub:a--", subs)).toBe("a");
    expect(subagentForOwner("sub:x1-b", subs)).toBe("b");
    expect(subagentForOwner("sub:agent", subs)).toBe("c");
  });

  it("knows the session's document however the editor was opened on it", () => {
    // From the panel, the file tree or a chat link: every save of the
    // linked document must go through save_document.
    expect(isLinkedDocument("/ws/.claudinio/docs/pudim.md", "/ws", DOC)).toBe(true);
    expect(isLinkedDocument("/ws/.claudinio/docs/pudim.md", "/ws/", DOC)).toBe(true);
    expect(isLinkedDocument("C:\\ws\\.claudinio\\docs\\pudim.md", "C:\\ws", DOC)).toBe(true);
    expect(isLinkedDocument("/ws/.claudinio/docs/./pudim.md", "/ws", DOC)).toBe(true);
    expect(isLinkedDocument("/ws/.claudinio/docs/other.md", "/ws", DOC)).toBe(false);
    expect(isLinkedDocument("/other/.claudinio/docs/pudim.md", "/ws", DOC)).toBe(false);
    expect(isLinkedDocument("/ws/.claudinio/docs/pudim.md", "/ws", null)).toBe(false);
  });
});
