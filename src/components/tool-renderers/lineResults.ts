/**
 * List results as the backend writes them since they stopped being JSON
 * (`src-tauri/src/agent/tools/lines.rs`): one row of text per entry, in the
 * shapes a shell would print. Sessions recorded before that still hold JSON
 * and are read by `parseJsonList`; this is the other half.
 *
 * The two files describe the same formats. Change one and the other follows.
 */

/** One row of a rendered list result. */
export interface ListRow {
  title: string;
  badge?: string;
  sub?: string;
  isDir?: boolean;
}

/** What the live result event appends when it cuts an output short. */
const TRUNCATED = /\.\.\.\(truncated, \d+ chars total\)$/;

/** How the history copy of a repeated `read_file` starts. */
export const UNCHANGED_MARK = "[unchanged: ";

/** The output's lines, without a last line the event cut in the middle. */
function wholeLines(output: string): string[] {
  const text = output.trimEnd();
  if (!TRUNCATED.test(text)) return text.split("\n");
  const lines = text.replace(TRUNCATED, "").split("\n");
  lines.pop();
  return lines;
}

function listDir(lines: string[]): ListRow[] | null {
  if (!lines[0]?.endsWith("/")) return null;
  return lines
    .slice(1)
    .filter((l) => l !== "" && l !== "(empty)")
    .map((l) => (l.endsWith("/") ? { title: l.slice(0, -1), isDir: true } : { title: l }));
}

function grep(lines: string[]): ListRow[] | null {
  if (lines[0] === "No matches.") return [];
  if (!/^\d+ match(es)? in \d+ files?$/.test(lines[0] ?? "")) return null;
  const rows: ListRow[] = [];
  let file: string | null = null;
  let expectFile = false;
  for (const line of lines.slice(1)) {
    if (line === "") {
      expectFile = true;
      continue;
    }
    if (expectFile) {
      file = line;
      expectFile = false;
      continue;
    }
    const m = line.match(/^(\d+): (.*)$/);
    if (m && file !== null) rows.push({ title: `${file}:${m[1]}`, sub: m[2] });
  }
  return rows;
}

/** `path:line kind signature`, optionally `path:start-end` and a `[match score]` tail. */
const SYMBOL_ROW = /^(.+?):(\d+(?:-\d+)?) (\S+) (.*?)(?: \[\w+ \d(?:\.\d+)?\])?$/;

function symbolRow(line: string): ListRow | null {
  const m = line.match(SYMBOL_ROW);
  return m ? { title: m[4], badge: m[3], sub: `${m[1]}:${m[2]}` } : null;
}

function symbols(lines: string[]): ListRow[] | null {
  if (lines[0] === "No symbols found.") return [];
  const rows = lines.filter((l) => l !== "").map(symbolRow);
  return rows.length > 0 && rows.every((r) => r !== null) ? (rows as ListRow[]) : null;
}

function outline(lines: string[]): ListRow[] | null {
  const head = lines[0] ?? "";
  if (head.endsWith(" (no symbols indexed)")) return [];
  if (!/ \(\d+ symbols?\)$/.test(head)) return null;
  const rows: ListRow[] = [];
  for (const line of lines.slice(1)) {
    const m = line.match(/^(\d+(?:-\d+)?) (\S+) (.*)$/);
    if (m) rows.push({ title: m[3], badge: m[2], sub: `L${m[1]}` });
  }
  return rows;
}

function semantic(lines: string[]): ListRow[] | null {
  if (!lines[0]?.startsWith("mode: ")) return null;
  const rows: ListRow[] = [];
  let fence: string | null = null;
  for (const line of lines.slice(1)) {
    if (fence !== null) {
      if (line === fence) fence = null;
      continue;
    }
    if (line === "```" || line === "~~~~") {
      fence = line;
      continue;
    }
    const row = symbolRow(line);
    if (row) rows.push(row);
  }
  return rows;
}

const PARSERS: Record<string, (lines: string[]) => ListRow[] | null> = {
  list_dir: listDir,
  grep,
  code_search: symbols,
  symbol_lookup: symbols,
  file_outline: outline,
  semantic_search: semantic,
};

/**
 * The rows of a list result written as lines, or null when `output` is not in
 * the line format `toolName` writes — an error text, or something else to show
 * as it is.
 */
export function parseLineList(toolName: string, output: string): ListRow[] | null {
  const parse = PARSERS[toolName];
  if (!parse) return null;
  const lines = wholeLines(output);
  return lines.length > 0 ? parse(lines) : null;
}
