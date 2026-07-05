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

export type AgentEvent =
  | { event: "Thinking"; data: string }
  | { event: "ToolCall"; data: ToolCallData }
  | { event: "ToolResult"; data: ToolResultData }
  | { event: "Done"; data: DoneData }
  | { event: "Error"; data: string };

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
  | { type: "tool_result"; data: ToolResultData };

export interface EditProposalData {
  path: string;
  oldString: string;
  newString: string;
  unifiedDiff: string;
}

export interface ToolResultData {
  toolName: string;
  output: string;
  error?: string | null;
}

export interface DoneData {
  stop_reason: string;
  text_output: string;
  input_tokens: number;
  output_tokens: number;
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

export function approveTool(sessionId: string, toolId: string): Promise<void> {
  return invoke<void>("approve_tool", { args: { sessionId, toolId } });
}

export function rejectTool(sessionId: string, toolId: string): Promise<void> {
  return invoke<void>("reject_tool", { args: { sessionId, toolId } });
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

export function openWorkspace(path: string): Promise<IndexStatus> {
  return invoke<IndexStatus>("open_workspace", { path });
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
