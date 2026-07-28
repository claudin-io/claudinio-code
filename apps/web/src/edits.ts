/// Finding the edits in a transcript.
///
/// The device sends transcript records, not diffs. An `edit_file` tool call carries
/// `old_string`, `new_string` and `path` inside a `tool_use` block on a `turn` record —
/// the unified diff the desktop draws is computed live and never persisted. So a peer
/// replaying a transcript has to find the calls and compute the diff itself.
///
/// Kept separate from the page and separately tested, because the shapes here come from
/// a model's tool call: the names change between versions, the arguments arrive as
/// whatever JSON was produced, and a wrong assumption shows up as a change silently not
/// being displayed rather than as an error.

/// An edit found in the transcript, ready to diff.
export interface FoundEdit {
  /// The tool-use id, so a record's edits are stable across re-renders.
  id: string;
  path: string;
  oldText: string;
  newText: string;
}

/// Tool names that mean "this file is about to change".
///
/// `edit_file` is what a real transcript contains. The others are accepted because the
/// tool surface has been renamed before — `Edit` and `Write` are the shapes an older
/// session or a different build uses — and a change that goes undisplayed because a name
/// moved is exactly the failure this list exists to avoid.
const EDIT_TOOLS = new Set(["edit_file", "edit", "write_file", "write", "str_replace"]);

/// One tool call, as an edit — or nothing, if it is not one.
///
/// The core both callers share. `findEdits` reads raw transcript records, which is what
/// the prova real checks because it is asserting on what arrived over the wire;
/// `editsInMessage` reads the timeline model, which is what the app renders. Same
/// recognition either way, so the two cannot drift into disagreeing about what an edit
/// is.
export function editFromCall(name: unknown, args: unknown, id: string): FoundEdit | null {
  const toolName = typeof name === "string" ? name.toLowerCase() : "";
  if (!EDIT_TOOLS.has(toolName)) return null;

  const input = isObject(args) ? args : {};
  const path = firstString(input, ["path", "file_path", "filePath"]) ?? "";
  const oldText = firstString(input, ["old_string", "oldString"]) ?? "";
  // A write with no old text is a new file, which is a diff against nothing rather than
  // something to skip.
  const newText = firstString(input, ["new_string", "newString", "content", "contents"]) ?? "";

  // Both empty means the call carried nothing to show. Displaying an empty diff there
  // would claim a change was inspected when nothing was.
  if (!oldText && !newText) return null;

  return { id, path, oldText, newText };
}

/// Every edit in a timeline message, from its tool calls.
///
/// What the app renders. `recordsToMessages` has already turned the JSONL into messages
/// and steps by the time this runs, so this reads the model rather than parsing content
/// blocks a second time.
export function editsInMessage(message: {
  steps?: { tool?: { call: { toolId?: string; toolName: string; args: Record<string, unknown> } } }[];
}): FoundEdit[] {
  const edits: FoundEdit[] = [];
  for (const step of message.steps ?? []) {
    const call = step.tool?.call;
    if (!call) continue;
    const edit = editFromCall(call.toolName, call.args, call.toolId ?? `${edits.length}`);
    if (edit) edits.push(edit);
  }
  return edits;
}

/// Pull every edit out of one transcript record.
///
/// Returns an empty list for anything that is not a turn with tool calls in it, which is
/// most records — so this is safe to call on all of them.
export function findEdits(record: unknown): FoundEdit[] {
  if (!isObject(record) || record.kind !== "turn") return [];
  const content = record.content;
  if (!Array.isArray(content)) return [];

  const edits: FoundEdit[] = [];
  for (const block of content) {
    if (!isObject(block) || block.type !== "tool_use") continue;
    const id = typeof block.id === "string" ? block.id : `block:${edits.length}`;
    const edit = editFromCall(block.name, block.input, id);
    if (edit) edits.push(edit);
  }
  return edits;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function firstString(source: Record<string, unknown>, keys: string[]): string | undefined {
  for (const key of keys) {
    const value = source[key];
    if (typeof value === "string") return value;
  }
  return undefined;
}
