import { Channel, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

export interface DirEntry {
  name: string;
  path: string;
  isDir: boolean;
}

export function listDir(path: string): Promise<DirEntry[]> {
  return invoke<DirEntry[]>("list_dir", { path });
}

export function readFile(path: string): Promise<string> {
  return invoke<string>("read_file", { path });
}

export async function pickFolder(): Promise<string | null> {
  const selected = await open({ directory: true, multiple: false });
  return typeof selected === "string" ? selected : null;
}

export interface SessionStarted {
  sessionId: string;
}

export interface AgentConfig {
  baseUrl: string;
  model: string;
  hasApiKey: boolean;
}

export interface SetConfigArgs {
  baseUrl?: string;
  apiKey?: string;
  model?: string;
}

export interface ApproveArgs {
  sessionId: string;
  toolId: string;
}

// Replay-only: old sessions may still have "plan" | "execute" | "summary"
// phase records on disk. No new session emits these.
export type Phase = "plan" | "execute" | "summary";

export type AgentEvent =
  | { event: "TextStep"; data: { text: string } }
  | { event: "Thinking"; data: string }
  | { event: "ToolCall"; data: ToolCallData }
  | { event: "ToolResult"; data: ToolResultData }
  | { event: "AskUser"; data: AskUserData }
  | { event: "Done"; data: DoneData }
  | { event: "SteeringInjected"; data: { text: string } }
  | { event: "Error"; data: string };

export interface AskUserQuestion {
  question: string;
  options: string[];
  multi_select?: boolean;
}

export interface AskUserData {
  sessionId: string;
  toolId: string;
  questions: AskUserQuestion[];
}

export interface UserAnswer {
  question: string;
  answer: string;
}

export interface ToolCallData {
  sessionId: string;
  toolId: string;
  toolName: string;
  args: Record<string, unknown>;
  permission: string;
  editProposal?: EditProposalData | null;
}

export type ChatStep =
  | { type: "thinking"; text: string }
  | { type: "tool_call"; data: ToolCallData }
  | { type: "tool_result"; data: ToolResultData }
  | { type: "steering"; text: string };

export interface EditProposalData {
  path: string;
  oldString: string;
  newString: string;
  unifiedDiff: string;
}

export interface ToolResultData {
  toolId: string;
  toolName: string;
  output: string;
  error?: string | null;
}

export interface DoneData {
  stopReason: string;
  textOutput: string;
  inputTokens: number;
  outputTokens: number;
}

export function sendMessage(
  message: string,
  onEvent: (event: AgentEvent) => void,
): Promise<SessionStarted> {
  const channel = new Channel<AgentEvent>();
  channel.onmessage = onEvent;
  return invoke<SessionStarted>("send_message", {
    message,
    eventChannel: channel,
  });
}

export interface SessionSummary {
  sessionId: string;
  createdAt: number;
  updatedAt: number;
  title: string;
  turnCount: number;
}

// One line of a session JSONL file. `kind` discriminates the variant; extra
// fields depend on the kind (see the Rust SessionRecord enum).
export type SessionRecord = {
  kind: "meta" | "user" | "phase" | "turn" | "phase_result" | "done" | "error" | "steering";
  [key: string]: unknown;
};

export function newSession(): Promise<void> {
  return invoke<void>("new_session");
}

export function listSessions(): Promise<SessionSummary[]> {
  return invoke<SessionSummary[]>("list_sessions");
}

export function loadSession(sessionId: string): Promise<SessionRecord[]> {
  return invoke<SessionRecord[]>("load_session", { sessionId });
}

export function approveTool(sessionId: string, toolId: string): Promise<void> {
  return invoke<void>("approve_tool", { args: { sessionId, toolId } });
}

export function rejectTool(sessionId: string, toolId: string): Promise<void> {
  return invoke<void>("reject_tool", { args: { sessionId, toolId } });
}

export function submitAnswers(
  sessionId: string,
  toolId: string,
  answers: UserAnswer[],
): Promise<void> {
  return invoke<void>("submit_answers", { args: { sessionId, toolId, answers } });
}

export function queueSteering(sessionId: string, text: string): Promise<void> {
  return invoke<void>("queue_steering", { sessionId, text });
}

export function interruptSession(sessionId: string): Promise<void> {
  return invoke<void>("interrupt_session", { sessionId });
}

export function setConfig(args: SetConfigArgs): Promise<void> {
  return invoke<void>("set_config", { args });
}

export function getConfig(): Promise<AgentConfig> {
  return invoke<AgentConfig>("get_config");
}

// --- Code Intelligence ---

export interface IndexStatus {
  status: string;
  filesCount: number;
  symbolsCount: number;
}

export interface IndexProgress {
  status: string;
  filesIndexed: number;
  symbolsIndexed: number;
  totalFiles: number;
  file?: string;
}

export interface SearchResult {
  symbolId: number;
  name: string;
  kind: string;
  filePath: string;
  startLine: number;
  signature?: string | null;
}

export interface SymbolRecord {
  id: number;
  fileId: number;
  name: string;
  kind: string;
  signature?: string | null;
  startLine: number;
  startCol: number;
  endLine: number;
  endCol: number;
  filePath?: string | null;
}

export function openWorkspace(path: string, onProgress?: (p: IndexProgress) => void): Promise<IndexStatus> {
  const channel = new Channel<IndexProgress>();
  channel.onmessage = onProgress ?? (() => {});
  return invoke<IndexStatus>("open_workspace", { path, progressChannel: channel });
}

export function searchSymbols(
  query: string,
  limit?: number,
): Promise<SearchResult[]> {
  return invoke<SearchResult[]>("search_symbols", { query, limit });
}

export function symbolLookup(name: string): Promise<SearchResult[]> {
  return invoke<SearchResult[]>("symbol_lookup", { name });
}

export function fileOutline(filePath: string): Promise<SymbolRecord[]> {
  return invoke<SymbolRecord[]>("file_outline", { filePath });
}

// --- File write ---

export function writeFile(path: string, content: string): Promise<void> {
  return invoke<void>("write_file", { path, content });
}

// --- LSP ---

export interface LspLocation {
  uri: string;
  startLine: number;
  startChar: number;
  endLine: number;
  endChar: number;
}

export interface LspPositionArgs {
  filePath: string;
  line: number;
  character: number;
}

export interface HoverInfo {
  contents: string;
  startLine?: number | null;
  startChar?: number | null;
  endLine?: number | null;
  endChar?: number | null;
}

export function lspDefinition(args: LspPositionArgs): Promise<LspLocation[]> {
  return invoke<LspLocation[]>("lsp_definition", { args });
}

export function lspReferences(args: LspPositionArgs): Promise<LspLocation[]> {
  return invoke<LspLocation[]>("lsp_references", { args });
}

export function lspHover(args: LspPositionArgs): Promise<HoverInfo | null> {
  return invoke<HoverInfo | null>("lsp_hover", { args });
}
