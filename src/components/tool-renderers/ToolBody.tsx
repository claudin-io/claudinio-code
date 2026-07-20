import { createMemo, For, Match, Show, Switch, type Component } from "solid-js";
import { marked } from "marked";
import hljs from "highlight.js/lib/core";
import langTypescript from "highlight.js/lib/languages/typescript";
import langJavascript from "highlight.js/lib/languages/javascript";
import langRust from "highlight.js/lib/languages/rust";
import langPython from "highlight.js/lib/languages/python";
import langJson from "highlight.js/lib/languages/json";
import langBash from "highlight.js/lib/languages/bash";
import langXml from "highlight.js/lib/languages/xml";
import langCss from "highlight.js/lib/languages/css";
import langMarkdown from "highlight.js/lib/languages/markdown";

hljs.registerLanguage("typescript", langTypescript);
hljs.registerLanguage("javascript", langJavascript);
hljs.registerLanguage("rust", langRust);
hljs.registerLanguage("python", langPython);
hljs.registerLanguage("json", langJson);
hljs.registerLanguage("bash", langBash);
hljs.registerLanguage("xml", langXml);
hljs.registerLanguage("css", langCss);
hljs.registerLanguage("markdown", langMarkdown);
import type { ToolCallData, ToolResultData } from "../../lib/ipc";
import { Icon, type IconName } from "../Icon";
import { DiffViewer } from "../DiffViewer";
import { ProseContent } from "../ProseContent";
import { t } from "../../lib/grill-me";
import { detectLanguageFromPath } from "./toolPresentation";

function highlight(code: string, lang: string): string {
  try {
    return hljs.getLanguage(lang) ? hljs.highlight(code, { language: lang }).value : code;
  } catch {
    return code;
  }
}

const JSON_LIST_TOOLS = new Set([
  "list_dir",
  "grep",
  "code_search",
  "symbol_lookup",
  "file_outline",
  "find_references",
  "go_to_definition",
  "semantic_search",
  "web_search",
]);

// ── bash ────────────────────────────────────────────────────────────
const BashBody: Component<{ command: string; workdir?: string; result?: ToolResultData }> = (props) => (
  <div>
    <div class="overflow-hidden rounded-md border border-border-subtle bg-surface-0">
      <div class="flex items-center gap-1.5 border-b border-border-subtle px-2.5 py-1">
        <Icon name="terminal" class="h-3 w-3 text-ink-faint" />
        <span class="truncate font-mono text-[11px] text-ink-faint">{props.workdir ?? "$"}</span>
      </div>
      <pre
        class="hljs overflow-x-auto p-2.5 font-mono text-[12px] leading-relaxed text-ink"
        innerHTML={highlight(props.command, "bash")}
      />
    </div>
    <Show when={props.result}>
      <pre
        class={`mt-2 max-h-56 overflow-y-auto whitespace-pre-wrap break-all rounded-md bg-surface-0 p-2.5 font-mono text-[11px] ${
          props.result!.error ? "text-danger" : "text-ink-faint"
        }`}
      >
        {(props.result!.error ?? props.result!.output).slice(0, 8000)}
      </pre>
    </Show>
  </div>
);

// ── read_file ───────────────────────────────────────────────────────
const ReadFileBody: Component<{ path: string; startLine?: number; result?: ToolResultData }> = (props) => (
  <Show
    when={!props.result?.error}
    fallback={<pre class="whitespace-pre-wrap break-all font-mono text-[11px] text-danger">{props.result?.error}</pre>}
  >
    <Show when={props.result}>
      {(result) => {
        const content = result().output.slice(0, 20000);
        const lang = detectLanguageFromPath(props.path);
        return (
          <div>
            <pre class="hljs max-h-72 overflow-y-auto whitespace-pre font-mono text-[11px] leading-relaxed" innerHTML={highlight(content, lang)} />
            <Show when={result().output.length > 20000}>
              <div class="mt-1 text-[11px] text-ink-faint">{t("chat.timeline.truncated")}</div>
            </Show>
          </div>
        );
      }}
    </Show>
  </Show>
);

// ── ask_user ────────────────────────────────────────────────────────
// Options come from the RAW model args, so they may be plain strings or any of
// the object shapes the model reaches for ({label,description}, description-only,
// {value,…}). Coerce each to a display string so history never shows [object Object].
interface AskUserQuestion {
  question: string;
  options: unknown[];
}

function optionLabel(opt: unknown): string {
  if (typeof opt === "string") return opt;
  if (opt && typeof opt === "object") {
    const o = opt as Record<string, unknown>;
    for (const key of ["label", "value", "title", "description", "text"]) {
      const v = o[key];
      if (typeof v === "string" && v.trim()) return v;
    }
  }
  return "";
}

const AskUserBody: Component<{ questions: AskUserQuestion[]; result?: ToolResultData }> = (props) => {
  const pairs = createMemo(() => {
    const output = props.result?.output;
    if (!output) return null;
    const blocks = output
      .split("\n\n")
      .map((block) => {
        const m = block.match(/^Pergunta: ([\s\S]*)\nResposta: ([\s\S]*)$/);
        return m ? { question: m[1], answer: m[2] } : null;
      })
      .filter((x): x is { question: string; answer: string } => x !== null);
    return blocks.length > 0 ? blocks : null;
  });

  return (
    <Show
      when={pairs()}
      fallback={
        <div class="flex flex-col gap-2">
          <For each={props.questions}>
            {(q) => (
              <div>
                <div class="text-[12px] font-medium text-ink">{q.question}</div>
                <div class="mt-0.5 flex flex-wrap gap-1">
                  <For each={q.options}>
                    {(opt) => (
                      <span class="rounded border border-border-subtle bg-surface-0 px-1.5 py-0.5 text-[11px] text-ink-faint">
                        {optionLabel(opt)}
                      </span>
                    )}
                  </For>
                </div>
              </div>
            )}
          </For>
        </div>
      }
    >
      {(list) => (
        <div class="flex flex-col gap-2">
          <For each={list()}>
            {(qa) => (
              <div>
                <div class="text-[12px] font-medium text-ink">{qa.question}</div>
                <div class="mt-0.5 flex items-start gap-1.5 text-[12px] text-accent">
                  <Icon name="check" class="mt-[2px] h-3 w-3 shrink-0" />
                  <span>{qa.answer}</span>
                </div>
              </div>
            )}
          </For>
        </div>
      )}
    </Show>
  );
};

// ── tasks_set / tasks_get ──────────────────────────────────────────
interface TaskArg {
  id: string;
  title: string;
  status: "todo" | "doing" | "done";
  journal?: string[];
}

function statusIcon(status: string): IconName {
  if (status === "done") return "check-circle";
  if (status === "doing") return "loader";
  return "circle-outline";
}

function statusClass(status: string): string {
  if (status === "done") return "text-success";
  if (status === "doing") return "text-accent animate-spin-slow";
  return "text-ink-faint";
}

const TasksBody: Component<{ argsTasks?: unknown; result?: ToolResultData }> = (props) => {
  const tasks = createMemo<TaskArg[]>(() => {
    if (Array.isArray(props.argsTasks)) return props.argsTasks as TaskArg[];
    if (props.result?.output) {
      try {
        const parsed = JSON.parse(props.result.output);
        if (Array.isArray(parsed)) return parsed as TaskArg[];
      } catch {
        /* not JSON — fall through */
      }
    }
    return [];
  });

  return (
    <Show when={tasks().length > 0} fallback={<div class="text-[11px] text-ink-faint">{t("chat.timeline.noResults")}</div>}>
      <div class="flex flex-col gap-1">
        <For each={tasks()}>
          {(task) => (
            <div class="flex items-start gap-2">
              <Icon name={statusIcon(task.status)} class={`mt-[1px] h-3.5 w-3.5 shrink-0 ${statusClass(task.status)}`} />
              <div class="min-w-0 flex-1">
                <div class="truncate text-[12px] text-ink">{task.title}</div>
                <Show when={task.journal && task.journal.length > 0}>
                  <div class="truncate text-[11px] text-ink-faint">{task.journal![task.journal!.length - 1]}</div>
                </Show>
              </div>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
};

// ── write_plan / finalize_plan ─────────────────────────────────────
const PlanBody: Component<{ markdown: string; planFile?: string }> = (props) => (
  <div>
    <ProseContent class="prose-content text-[12px] leading-[1.6] text-ink-muted" html={marked.parse(props.markdown, { async: false }) as string} />
    <Show when={props.planFile}>
      <div class="mt-2 flex items-center gap-1.5 text-[11px] text-ink-faint">
        <Icon name="file-text" class="h-3 w-3" />
        {props.planFile}
      </div>
    </Show>
  </div>
);

// ── spawn_agents ────────────────────────────────────────────────────
interface AgentSpec {
  name: string;
  goal: string;
  mode: string;
}

const SpawnAgentsBody: Component<{ agents: AgentSpec[] }> = (props) => (
  <div class="flex flex-col gap-1.5">
    <For each={props.agents}>
      {(agent) => (
        <div class="rounded-md bg-surface-0 p-2">
          <div class="flex items-center gap-2">
            <span class="text-[12px] font-medium text-ink">{agent.name}</span>
            <span class="rounded bg-accent/15 px-1 text-[10px] font-medium text-accent">{agent.mode}</span>
          </div>
          <div class="mt-0.5 truncate text-[11px] text-ink-faint">{agent.goal}</div>
        </div>
      )}
    </For>
  </div>
);

// ── list_dir / grep / code_search / symbol_lookup / file_outline /
//    find_references / go_to_definition / semantic_search / web_search ──
interface ListRow {
  title: string;
  badge?: string;
  sub?: string;
  isDir?: boolean;
}

function pickRow(obj: Record<string, unknown>): ListRow {
  // Symbol result (code_search / semantic_search / symbol_lookup / file_outline):
  // lead with the symbol name, kind as badge, location as the muted tail.
  if (typeof obj.name === "string" && typeof obj.filePath === "string") {
    const line = obj.startLine ?? obj.line;
    return {
      title: obj.name,
      badge: typeof obj.kind === "string" ? obj.kind : undefined,
      sub: `${obj.filePath}${line != null ? `:${line}` : ""}`,
    };
  }
  // Grep match: file:line first, matched text as the tail.
  if (typeof obj.file === "string" && obj.line != null) {
    return { title: `${obj.file}:${obj.line}`, sub: obj.content != null ? String(obj.content).trim() : undefined };
  }
  const title = String(obj.name ?? obj.path ?? obj.filePath ?? obj.symbol ?? JSON.stringify(obj).slice(0, 60));
  const line = obj.line ?? obj.startLine ?? obj.lineNumber;
  const badge = line != null ? `L${line}` : undefined;
  const subRaw = obj.snippet ?? obj.preview ?? obj.text ?? obj.kind ?? obj.signature;
  return { title, badge, sub: subRaw != null ? String(subRaw) : undefined, isDir: obj.isDir === true };
}

/**
 * Parse a JSON array result, salvaging complete top-level objects when the
 * output was truncated mid-array (the backend caps live ToolResult payloads,
 * e.g. `[{...}, {...}, {"na...(truncated, N chars total)`).
 */
export function parseJsonList(output: string): { list: unknown[]; truncated: boolean } | null {
  const text = output.trim();
  try {
    const v = JSON.parse(text);
    if (Array.isArray(v)) return { list: v, truncated: false };
    // Envelopes ({ mode, note?, results: [...] }) from semantic_search wrap
    // the list — render the inner results.
    if (v && Array.isArray((v as Record<string, unknown>).results)) {
      return { list: (v as Record<string, unknown>).results as unknown[], truncated: false };
    }
    return null;
  } catch {
    /* fall through to salvage */
  }
  if (text.startsWith("[")) {
    const list = salvageObjects(text, 1);
    return list.length > 0 ? { list, truncated: true } : null;
  }
  if (text.startsWith("{")) {
    // Truncated envelope: locate the results array and salvage complete
    // objects from inside it.
    const marker = text.indexOf('"results"');
    const bracket = marker >= 0 ? text.indexOf("[", marker) : -1;
    if (bracket >= 0) {
      const list = salvageObjects(text, bracket + 1);
      return list.length > 0 ? { list, truncated: true } : null;
    }
  }
  return null;
}

/** Collect complete top-level `{...}` objects starting at `from`. */
function salvageObjects(text: string, from: number): unknown[] {
  const list: unknown[] = [];
  let depth = 0;
  let start = -1;
  let inString = false;
  let escaped = false;
  for (let i = from; i < text.length; i++) {
    const ch = text[i];
    if (inString) {
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') inString = true;
    else if (ch === "{") {
      if (depth === 0) start = i;
      depth++;
    } else if (ch === "}") {
      depth--;
      if (depth === 0 && start >= 0) {
        try {
          list.push(JSON.parse(text.slice(start, i + 1)));
        } catch {
          /* skip malformed element */
        }
        start = -1;
      }
    }
  }
  return list;
}

const JsonListBody: Component<{ result?: ToolResultData }> = (props) => {
  const parsed = createMemo(() => {
    const output = props.result?.output;
    if (props.result?.error || !output) return null;
    return parseJsonList(output)?.list ?? null;
  });

  return (
    <Show
      when={parsed()}
      fallback={
        <Show when={props.result?.output || props.result?.error}>
          <pre
            class={`max-h-56 overflow-y-auto whitespace-pre-wrap break-all font-mono text-[11px] ${
              props.result?.error ? "text-danger" : "text-ink-faint"
            }`}
          >
            {(props.result?.error ?? props.result?.output ?? "").slice(0, 5000)}
          </pre>
        </Show>
      }
    >
      {(list) => (
        <Show when={list().length > 0} fallback={<div class="text-[11px] text-ink-faint">{t("chat.timeline.noResults")}</div>}>
          <div class="flex flex-col gap-0.5">
            <For each={list().slice(0, 60)}>
              {(item) => {
                const row = typeof item === "string" ? { title: item } : pickRow(item as Record<string, unknown>);
                return (
                  <div class="flex items-center gap-2 truncate text-[11px]">
                    <Icon name={row.isDir ? "folder" : "file"} class="h-3 w-3 shrink-0 text-ink-faint" />
                    <span class="truncate font-mono text-ink-muted">{row.title}</span>
                    <Show when={row.badge}>
                      <span class="shrink-0 rounded bg-surface-2 px-1 text-[10px] text-ink-faint">{row.badge}</span>
                    </Show>
                    <Show when={row.sub}>
                      <span class="min-w-0 flex-1 truncate font-mono text-ink-faint">{row.sub}</span>
                    </Show>
                  </div>
                );
              }}
            </For>
            <Show when={list().length > 60}>
              <div class="text-[11px] text-ink-faint">+{list().length - 60} {t("chat.timeline.more")}</div>
            </Show>
          </div>
        </Show>
      )}
    </Show>
  );
};

// ── generic fallback (any other tool, incl. MCP) ───────────────────
const GenericBody: Component<{ args: Record<string, unknown>; result?: ToolResultData }> = (props) => (
  <div>
    <Show when={Object.keys(props.args).length > 0}>
      <div class="flex flex-col gap-1">
        <For each={Object.entries(props.args)}>
          {([key, value]) => (
            <div class="flex gap-2 text-[11px]">
              <span class="shrink-0 font-mono text-ink-faint">{key}</span>
              <span class="truncate text-ink-muted">{typeof value === "string" ? value : JSON.stringify(value)}</span>
            </div>
          )}
        </For>
      </div>
    </Show>
    <Show when={props.result}>
      <pre
        class={`mt-2 max-h-48 overflow-y-auto whitespace-pre-wrap break-all font-mono text-[11px] ${
          props.result!.error ? "text-danger" : "text-ink-faint"
        }`}
      >
        {(props.result!.error ?? props.result!.output).slice(0, 5000)}
      </pre>
    </Show>
  </div>
);

// ── dispatcher ──────────────────────────────────────────────────────
export const ToolBody: Component<{ call: ToolCallData; result?: ToolResultData }> = (props) => {
  const name = () => props.call.toolName;

  return (
    <Switch fallback={<GenericBody args={props.call.args} result={props.result} />}>
      <Match when={name() === "bash"}>
        <BashBody
          command={String(props.call.args.command ?? "")}
          workdir={props.call.args.workdir as string | undefined}
          result={props.result}
        />
      </Match>
      <Match when={name() === "edit_file"}>
        <div class="overflow-hidden rounded border border-border-subtle">
          <DiffViewer
            original={(props.call.args.old_string as string) ?? ""}
            modified={(props.call.args.new_string as string) ?? ""}
            language={detectLanguageFromPath((props.call.args.path as string) ?? "")}
            inline
          />
        </div>
      </Match>
      <Match when={name() === "read_file"}>
        <ReadFileBody
          path={(props.call.args.path as string) ?? ""}
          startLine={props.call.args.start_line as number | undefined}
          result={props.result}
        />
      </Match>
      <Match when={name() === "ask_user"}>
        <AskUserBody questions={(props.call.args.questions as AskUserQuestion[]) ?? []} result={props.result} />
      </Match>
      <Match when={name() === "tasks_set" || name() === "tasks_get"}>
        <TasksBody argsTasks={props.call.args.tasks} result={props.result} />
      </Match>
      <Match when={name() === "write_plan"}>
        <PlanBody markdown={(props.call.args.content as string) ?? ""} />
      </Match>
      <Match when={name() === "finalize_plan"}>
        <PlanBody
          markdown={(props.call.args.journal as string) ?? ""}
          planFile={props.call.args.plan_file as string | undefined}
        />
      </Match>
      <Match when={name() === "spawn_agents"}>
        <SpawnAgentsBody agents={(props.call.args.agents as AgentSpec[]) ?? []} />
      </Match>
      <Match when={JSON_LIST_TOOLS.has(name())}>
        <JsonListBody result={props.result} />
      </Match>
    </Switch>
  );
};
