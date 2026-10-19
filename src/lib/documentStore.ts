import { createSignal } from "solid-js";
import type { AgentEvent, DocView } from "./ipc";

// The living document's live state per workspace (lean-harness Phase 4b).
//
// The run's events reach ChatPanel; the panel lives beside it in App. Rather
// than thread a callback through App for one event type, ChatPanel publishes
// here and DocumentPanel reads here. A module-level signal, like the theme:
// SolidJS has no store library in this app (docs/ARCHITECTURE.md).

interface Published {
  /** The latest outline a run sent; absent after an invalidation. */
  doc?: DocView;
  /** Bumped whenever the panel should ask the backend again. */
  version: number;
}

const [published, setPublished] = createSignal<Record<string, Published>>({});

/** The document a `DocumentUpdated` event carries, also from inside a subagent's events. */
export function documentFromEvent(event: AgentEvent): DocView | null {
  if (event.event === "DocumentUpdated") return event.data.document;
  if (event.event === "Subagent") return documentFromEvent(event.data.event);
  return null;
}

export function publishDocument(workspace: string, doc: DocView): void {
  setPublished((m) => ({ ...m, [workspace]: { doc, version: (m[workspace]?.version ?? 0) + 1 } }));
}

/** The session changed (a handoff, a new session): forget the outline and ask again. */
export function invalidateDocument(workspace: string): void {
  setPublished((m) => ({ ...m, [workspace]: { version: (m[workspace]?.version ?? 0) + 1 } }));
}

/** Reactive: what was last published for `workspace`. */
export function publishedDocument(workspace: string): Published {
  return published()[workspace] ?? { version: 0 };
}

// ── Opening a subagent's timeline from the panel ───────────────────

interface SubagentRequest {
  workspace: string;
  owner: string;
  at: number;
}

const [subagentRequest, setSubagentRequest] = createSignal<SubagentRequest | null>(null);

/** The panel asks the chat to open the timeline of the subagent `owner` names. */
export function requestSubagent(workspace: string, owner: string): void {
  setSubagentRequest({ workspace, owner, at: Date.now() });
}

export function pendingSubagentRequest(): SubagentRequest | null {
  return subagentRequest();
}

export function clearSubagentRequest(): void {
  setSubagentRequest(null);
}

/**
 * The id of the subagent an owner (`sub:<name>`) names, among `candidates`
 * in timeline order — the latest one when a name was used twice. Null for
 * `main` or a name no subagent had. The name is cleaned as the backend's
 * `subagent_owner` cleans it: spaces as dashes, only letters, digits, `_`
 * and `-` kept, `agent` when nothing is left.
 */
export function subagentForOwner(owner: string, candidates: { id: string; name: string }[]): string | null {
  if (!owner.startsWith("sub:")) return null;
  const want = owner.slice(4);
  const normal = (name: string) =>
    name
      .trim()
      .split(/\s+/)
      .map((w) => w.replace(/[^\p{L}\p{N}_-]/gu, ""))
      .filter((w) => w !== "")
      .join("-") || "agent";
  for (let i = candidates.length - 1; i >= 0; i--) {
    if (normal(candidates[i].name) === want) return candidates[i].id;
  }
  return null;
}

// ── Which file is the session's document ───────────────────────────

/**
 * Whether the editor's `filePath` is `doc`, the session's living document in
 * `workspace` — however the editor was opened on it (the panel, the file
 * tree, a chat link). App routes every save of that file through
 * `save_document`, which merges with what agents wrote while the editor was
 * open and marks the person's sections `human`; a plain `write_file` would
 * roll the agents' writes back and attribute them to the person.
 */
export function isLinkedDocument(filePath: string, workspace: string, doc: DocView | null): boolean {
  if (!doc) return false;
  // Forward slashes, no `.` segments, no trailing slash: the tree, the panel
  // and Windows paths spell the same file differently.
  const normal = (p: string) =>
    p
      .replace(/\\/g, "/")
      .split("/")
      .filter((seg, i) => seg !== "." && (seg !== "" || i === 0))
      .join("/");
  return normal(filePath) === normal(`${workspace}/${doc.path}`);
}
