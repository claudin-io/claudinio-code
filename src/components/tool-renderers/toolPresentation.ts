import type { ToolCallData } from "../../lib/ipc";
import type { IconName } from "../Icon";
import { toolIcon } from "../Icon";

export function detectLanguageFromPath(path: string): string {
  if (path.endsWith(".ts") || path.endsWith(".tsx")) return "typescript";
  if (path.endsWith(".rs")) return "rust";
  if (path.endsWith(".py")) return "python";
  if (path.endsWith(".swift")) return "swift";
  if (path.endsWith(".js") || path.endsWith(".jsx")) return "javascript";
  if (path.endsWith(".css")) return "css";
  if (path.endsWith(".json")) return "json";
  if (path.endsWith(".html")) return "html";
  if (path.endsWith(".md")) return "markdown";
  if (path.endsWith(".sh")) return "bash";
  return "plaintext";
}

const TITLES: Record<string, string> = {
  bash: "Ran command",
  read_file: "Read file",
  edit_file: "Edited file",
  list_dir: "Listed folder",
  grep: "Searched files",
  code_search: "Searched code",
  semantic_search: "Searched code",
  symbol_lookup: "Looked up symbol",
  file_outline: "Outlined file",
  go_to_definition: "Went to definition",
  find_references: "Found references",
  web_search: "Searched the web",
  ask_user: "Asked you",
  tasks_get: "Checked tasks",
  tasks_set: "Updated tasks",
  spawn_agents: "Spawned agents",
  write_plan: "Wrote plan",
  finalize_plan: "Finalized plan",
  enter_plan_mode: "Entered plan mode",
  exit_plan_mode: "Exited plan mode",
};

export function toolTitle(name: string): string {
  if (TITLES[name]) return TITLES[name];
  if (name.startsWith("mcp__")) {
    const parts = name.slice(5).split("__");
    return parts[parts.length - 1].replace(/_/g, " ");
  }
  return name.replace(/_/g, " ");
}

const SEARCH_COMMANDS = new Set(["grep", "rg", "egrep", "fgrep", "ag", "ack"]);

/** First real command of a bash invocation, skipping leading env assignments. */
function bashLeadCommand(command: string): string {
  const tokens = command.trimStart().split(/\s+/);
  for (const token of tokens) {
    if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(token)) continue;
    return token.replace(/^.*\//, "");
  }
  return "";
}

/**
 * Header icon + title for a tool call. Mostly delegates to toolIcon/toolTitle,
 * but reclassifies bash invocations that are really searches (grep/rg/…) so
 * the timeline reads "Searched files" instead of "Ran command".
 */
export function toolHeader(call: ToolCallData): { icon: IconName; title: string } {
  if (call.toolName === "bash" && SEARCH_COMMANDS.has(bashLeadCommand(String(call.args.command ?? "")))) {
    return { icon: "search", title: TITLES.grep };
  }
  return { icon: toolIcon(call.toolName), title: toolTitle(call.toolName) };
}

/** Tools whose body is always shown — small enough to skip the click-to-expand step. */
export function alwaysShowsBody(name: string): boolean {
  return name === "edit_file" || name === "ask_user" || name === "tasks_set";
}

function truncate(s: string, n: number): string {
  const clean = s.trim().replace(/\s+/g, " ");
  return clean.length > n ? `${clean.slice(0, n)}…` : clean;
}

/** One-line, tool-specific subtitle shown next to the header — replaces the raw-args JSON preview. */
export function toolSummary(call: ToolCallData): string {
  const args = call.args;
  switch (call.toolName) {
    case "bash":
      return truncate(String(args.command ?? ""), 80);
    case "read_file":
    case "edit_file":
    case "list_dir": {
      const path = String(args.path ?? "");
      const start = args.start_line as number | undefined;
      const end = args.end_line as number | undefined;
      if (start != null) return `${path} L${start}${end != null ? `-${end}` : "+"}`;
      return path;
    }
    case "file_outline":
    case "go_to_definition":
    case "find_references":
      return String(args.file_path ?? "");
    case "grep":
      return `/${String(args.pattern ?? "")}/${args.path ? ` in ${args.path}` : ""}`;
    case "code_search":
    case "semantic_search":
    case "web_search":
      return truncate(String(args.query ?? ""), 80);
    case "symbol_lookup":
      return String(args.name ?? "");
    case "ask_user": {
      const qs = (args.questions as { question: string }[] | undefined) ?? [];
      if (qs.length === 1) return truncate(qs[0].question, 80);
      return `${qs.length} question${qs.length === 1 ? "" : "s"}`;
    }
    case "tasks_set": {
      const tasks = (args.tasks as { status: string }[] | undefined) ?? [];
      const done = tasks.filter((task) => task.status === "done").length;
      return `${done}/${tasks.length} done`;
    }
    case "spawn_agents": {
      const agents = (args.agents as { name: string }[] | undefined) ?? [];
      return `${agents.length} agent${agents.length === 1 ? "" : "s"}`;
    }
    case "write_plan":
      return String(args.name ?? "");
    case "finalize_plan":
      return truncate(String(args.summary ?? ""), 80);
    case "enter_plan_mode":
      return truncate(String(args.reason ?? ""), 80);
    default: {
      const path = args.path as string | undefined;
      if (path) return path;
      const pattern = args.pattern as string | undefined;
      if (pattern) return `/${pattern}/`;
      const content = args.content as string | undefined;
      if (content) return truncate(content, 60);
      return truncate(JSON.stringify(args), 80);
    }
  }
}
