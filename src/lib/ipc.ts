import { Channel, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";

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

export async function pickFiles(): Promise<string[]> {
  const selected = await open({ multiple: true });
  if (!selected) return [];
  return Array.isArray(selected) ? selected : [selected];
}

export interface SessionStarted {
  sessionId: string;
}

export interface AttachmentInput {
  path: string;
}

export interface AttachmentData {
  name: string;
  mediaType: string;
  data: string;
  size: number;
}

export interface AgentConfig {
  baseUrl: string;
  brainModel: string;
  builderModel: string;
  hasApiKey: boolean;
  maxContextTokens: number;
  compactThreshold: number;
  maxRounds?: number | null;
  subMaxRounds?: number | null;
  yoloMode?: boolean;
  yoloBlacklist?: string[];
  accountLogin?: string | null;
  accountTier?: string | null;
  maxGoldenCycles?: number | null;
  maxGoldenStalls?: number | null;
}

export interface SetConfigArgs {
  baseUrl?: string;
  apiKey?: string;
  brainModel?: string;
  builderModel?: string;
  maxRounds?: number | null;
  subMaxRounds?: number | null;
  yoloMode?: boolean;
  yoloBlacklist?: string[];
  maxGoldenCycles?: number | null;
  maxGoldenStalls?: number | null;
}

export interface ApproveArgs {
  sessionId: string;
  toolId: string;
}

// Replay-only: old sessions may still have "plan" | "execute" | "summary"
// phase records on disk. No new session emits these.
export type Phase = "plan" | "execute" | "summary";

export interface SubagentStartedData {
  subagentId: string;
  parentToolId: string;
  name: string;
  goal: string;
  mode: string;
}

export interface SubagentDoneData {
  subagentId: string;
  status: string;
  rounds: number;
  inputTokens: number;
  outputTokens: number;
  report?: string;
}

export type SessionMode = "brain" | "builder";

/// Map a persisted mode string to the current ids. Old session JSONLs carry
/// the original names "pensador"/"constructor".
export function normalizeSessionMode(s: unknown): SessionMode {
  return s === "brain" || s === "pensador" ? "brain" : "builder";
}
export type ModeOrigin = "human" | "agent";

export interface ModeChangedData {
  mode: SessionMode;
  origin: ModeOrigin;
  reason?: string | null;
}

export interface GoldenLoopData {
  cycle: number;
  maxCycles: number;
  pending: string[];
  mode: SessionMode;
}

export type AgentEvent =
  | { event: "TextStep"; data: { text: string } }
  | { event: "ModeChanged"; data: ModeChangedData }
  | { event: "GoldenLoop"; data: GoldenLoopData }
  | { event: "Thinking"; data: string }
  | { event: "ToolCall"; data: ToolCallData }
  | { event: "ToolResult"; data: ToolResultData }
  | { event: "AskUser"; data: AskUserData }
  | { event: "Done"; data: DoneData }
  | { event: "SteeringInjected"; data: { text: string } }
  | { event: "Error"; data: string }
  | { event: "SubagentStarted"; data: SubagentStartedData }
  | { event: "SubagentDone"; data: SubagentDoneData }
  | { event: "Subagent"; data: { subagentId: string; event: AgentEvent } }
  | {
      event: "SessionStats";
      data: {
        inputTokens: number;
        outputTokens: number;
        cumulativeCost?: number;
        costInput?: number;
        costOutput?: number;
        costCacheRead?: number;
        contextTokens: number;
        maxContextTokens: number;
        compactThreshold: number;
      };
    };

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
  workspace: string,
  message: string,
  attachments: AttachmentInput[],
  onEvent: (event: AgentEvent) => void,
  mode?: SessionMode,
): Promise<SessionStarted> {
  const channel = new Channel<AgentEvent>();
  channel.onmessage = onEvent;
  return invoke<SessionStarted>("send_message", {
    workspace,
    message,
    attachments: attachments.length > 0 ? attachments : undefined,
    mode,
    eventChannel: channel,
  });
}

export function setSessionMode(workspace: string, mode: SessionMode): Promise<SessionStarted> {
  return invoke<SessionStarted>("set_session_mode", { workspace, mode });
}

export function getSessionMode(workspace: string): Promise<{ mode: SessionMode; origin: ModeOrigin }> {
  return invoke<{ mode: SessionMode; origin: ModeOrigin }>("get_session_mode", { workspace });
}

export function readAttachment(path: string): Promise<AttachmentData> {
  return invoke<AttachmentData>("read_attachment", { path });
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
  kind: "meta" | "user" | "phase" | "turn" | "phase_result" | "done" | "error" | "steering" | "compacted" | "status" | "mode" | "tasks" | "golden_cycle";
  [key: string]: unknown;
};

export function newSession(workspace: string): Promise<void> {
  return invoke<void>("new_session", { workspace });
}

export function listSessions(workspace: string): Promise<SessionSummary[]> {
  return invoke<SessionSummary[]>("list_sessions", { workspace });
}

export function loadSession(workspace: string, sessionId: string): Promise<SessionRecord[]> {
  return invoke<SessionRecord[]>("load_session", { workspace, sessionId });
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

export function compactSession(
  workspace: string,
  sessionId: string,
  onEvent: (event: AgentEvent) => void,
): Promise<string> {
  const channel = new Channel<AgentEvent>();
  channel.onmessage = onEvent;
  return invoke<string>("compact_session", { workspace, sessionId, eventChannel: channel });
}

/// Cumulative token/cost stats and current context size from the last Status
/// record in a session.
export function getSessionStats(records: SessionRecord[]): {
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCost?: number;
  costInput?: number;
  costOutput?: number;
  costCacheRead?: number;
  contextTokens?: number;
} {
  let totalInput = 0;
  let totalOutput = 0;
  let totalCost: number | undefined;
  let costInput: number | undefined;
  let costOutput: number | undefined;
  let costCacheRead: number | undefined;
  let contextTokens: number | undefined;
  for (const rec of records) {
    if (rec.kind === "status") {
      totalInput = Number(rec.total_input_tokens ?? 0);
      totalOutput = Number(rec.total_output_tokens ?? 0);
      if (rec.total_cost != null) {
        totalCost = Number(rec.total_cost);
      }
      if (rec.total_cost_input != null) {
        costInput = Number(rec.total_cost_input);
      }
      if (rec.total_cost_output != null) {
        costOutput = Number(rec.total_cost_output);
      }
      if (rec.total_cost_cache_read != null) {
        costCacheRead = Number(rec.total_cost_cache_read);
      }
      if (rec.context_tokens != null) {
        contextTokens = Number(rec.context_tokens);
      }
    }
  }
  return {
    totalInputTokens: totalInput,
    totalOutputTokens: totalOutput,
    totalCost,
    costInput,
    costOutput,
    costCacheRead,
    contextTokens,
  };
}

export function setConfig(args: SetConfigArgs): Promise<void> {
  return invoke<void>("set_config", { args });
}

export function getConfig(): Promise<AgentConfig> {
  return invoke<AgentConfig>("get_config");
}

export function listModels(): Promise<string[]> {
  return invoke<string[]>("list_models");
}

export interface LoginResult {
  login: string;
  tier?: string | null;
}

/** Opens the browser to sign in with claudin.io and links the active API key. */
export function loginWithClaudinio(): Promise<LoginResult> {
  return invoke<LoginResult>("login_with_claudinio");
}

export function logoutClaudinio(): Promise<void> {
  return invoke<void>("logout_claudinio");
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
  /** Root path of the workspace this progress event belongs to. */
  workspace: string;
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
  if (onProgress) channel.onmessage = onProgress;
  return invoke<IndexStatus>("open_workspace", { path, progressChannel: channel });
}

export function closeWorkspace(path: string): Promise<void> {
  return invoke<void>("close_workspace", { path });
}

export function searchSymbols(
  workspace: string,
  query: string,
  limit?: number,
): Promise<SearchResult[]> {
  return invoke<SearchResult[]>("search_symbols", { workspace, query, limit });
}

export function symbolLookup(workspace: string, name: string): Promise<SearchResult[]> {
  return invoke<SearchResult[]>("symbol_lookup", { workspace, name });
}

export function fileOutline(workspace: string, filePath: string): Promise<SymbolRecord[]> {
  return invoke<SymbolRecord[]>("file_outline", { workspace, filePath });
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


// --- @-mention file autocomplete ---

export interface WalkEntry {
  path: string;
  isDir: boolean;
}

export function walkDirectory(root: string): Promise<WalkEntry[]> {
  return invoke<WalkEntry[]>("walk_dir", { root });
}

// --- Tasks ---

export interface TaskItem {
  id: string;
  title: string;
  description: string;
  journal: string[];
  status: "todo" | "doing" | "done";
}

export function getTasks(workspace: string): Promise<TaskItem[]> {
  return invoke<TaskItem[]>("get_tasks", { workspace });
}

export function setTasks(workspace: string, tasks: TaskItem[]): Promise<void> {
  return invoke<void>("set_tasks", { workspace, tasks });
}

// --- Skills ---

export interface SkillEntry {
  name: string;
  description: string;
  location: string;
  scope: "builtin" | "project" | "subfolder" | "user";
  body?: string;
}

export interface SkillCatalogEntry {
  name: string;
  description: string;
  location: string;
  scope: "builtin" | "project" | "subfolder" | "user";
}

export interface SkillsResponse {
  skills: SkillEntry[];
  count: number;
}

export interface RemoteSkill {
  name: string;
  description: string;
  url: string;
  source: { type: string; [key: string]: unknown };
}

export interface InstallRemoteSkillArgs {
  name: string;
  url: string;
  description: string;
}

export function listSkills(workspace: string): Promise<SkillsResponse> {
  return invoke<SkillsResponse>("list_skills", { workspace });
}

export function getSkillCatalog(workspace: string): Promise<string[]> {
  return invoke<string[]>("get_skill_catalog", { workspace });
}

export function getSkillContent(workspace: string, name: string): Promise<SkillEntry & { body: string }> {
  return invoke("get_skill_content", { workspace, name });
}

export function rescanSkills(workspace: string): Promise<SkillsResponse> {
  return invoke<SkillsResponse>("rescan_skills", { workspace });
}

export function findRemoteSkills(query?: string): Promise<RemoteSkill[]> {
  return invoke<RemoteSkill[]>("find_remote_skills", { query: query ?? null });
}

export function previewRemoteSkill(url: string): Promise<SkillEntry> {
  return invoke<SkillEntry>("preview_remote_skill", { url });
}

export function installRemoteSkill(workspace: string, args: InstallRemoteSkillArgs): Promise<SkillEntry> {
  return invoke<SkillEntry>("install_remote_skill", { workspace, args });
}

// --- Context Warning ---

export interface SkillTokenEntry {
  name: string;
  description: string;
  estimatedTokens: number;
  location: string;
}

export interface ContextWarningData {
  agentsMdSize: number;
  agentsMdLines: number;
  agentsMdTokens: number;
  agentsMdIssues: number;
  agentsMdPath: string | null;
  skillsCount: number;
  skillsTotalTokens: number;
  skillsBreakdown: SkillTokenEntry[];
}

export function getContextWarning(workspace: string): Promise<ContextWarningData> {
  return invoke<ContextWarningData>("get_context_warning", { workspace });
}

export function lspDefinition(workspace: string, args: LspPositionArgs): Promise<LspLocation[]> {
  return invoke<LspLocation[]>("lsp_definition", { workspace, args });
}

export function lspReferences(workspace: string, args: LspPositionArgs): Promise<LspLocation[]> {
  return invoke<LspLocation[]>("lsp_references", { workspace, args });
}

export function lspHover(workspace: string, args: LspPositionArgs): Promise<HoverInfo | null> {
  return invoke<HoverInfo | null>("lsp_hover", { workspace, args });
}

export function openExternal(path: string): void {
  openPath(path).catch(() => {});
}
