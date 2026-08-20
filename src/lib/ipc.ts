import { Channel, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

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

export async function pickFolder(defaultPath?: string): Promise<string | null> {
  const selected = await open({ directory: true, multiple: false, ...(defaultPath !== undefined ? { defaultPath } : {}) });
  return typeof selected === "string" ? selected : null;
}

export async function pickFiles(): Promise<string[]> {
  const selected = await open({ multiple: true });
  if (!selected) return [];
  return Array.isArray(selected) ? selected : [selected];
}

export function openInTerminal(path: string): Promise<void> {
  return invoke<void>("open_in_terminal", { path });
}

export function detectIdes(): Promise<string[]> {
  return invoke<string[]>("detect_ides");
}

export function openInIde(path: string, ide: string, gotoLine?: number): Promise<void> {
  return invoke<void>("open_in_ide", { path, ide, gotoLine });
}

export async function copyPath(path: string): Promise<void> {
  await navigator.clipboard.writeText(path);
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

export interface WriteClipboardBlobResult {
  path: string;
  name: string;
  mediaType: string;
  size: number;
}

export type McpTransportConfig =
  | { type: "stdio"; command: string; args?: string[]; env?: Record<string, string> }
  | { type: "remote"; url: string; headers?: Record<string, string> };

export type McpServerEntry = McpTransportConfig & {
  enabled?: boolean;
};

// Keyed by server name, e.g. { "context7": { type: "remote", url: "...", headers: {...} } }
export type McpServerMap = Record<string, McpServerEntry>;

export interface McpServerStatus {
  name: string;
  connected: boolean;
  toolCount: number;
  toolNames: string[];
  error?: string | null;
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
  keepAwake?: boolean;
  accountLogin?: string | null;
  accountTier?: string | null;
  maxGoldenCycles?: number | null;
  maxGoldenStalls?: number | null;
  maxParallelAgents?: number | null;
  planSavePath?: string | null;
  overrideBaseUrl?: string | null;
  overrideApiKey?: string | null;
  mcp?: McpServerMap;
  codeIntelEnabled?: boolean;
  preferredIde?: string | null;
  handoffContextTokens?: number | null;
  autoCommitPlan?: boolean;
  thinkingEffort?: string;
  browser?: BrowserPrefs;
  local?: LocalPrefs;
  providers?: Record<string, ConnectedProviderInfo>;
  workspaceConfig?: Record<string, unknown> | null;
}

/** A connected external provider as reported by get_config — never the key. */
export interface ConnectedProviderInfo {
  connected: boolean;
  baseUrl: string;
  label?: string | null;
  protocol?: string;
  enabledModels?: string[];
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
  keepAwake?: boolean;
  maxGoldenCycles?: number | null;
  maxGoldenStalls?: number | null;
  maxParallelAgents?: number | null;
  planSavePath?: string | null;
  overrideBaseUrl?: string;
  overrideApiKey?: string;
  mcp?: McpServerMap;
  codeIntelEnabled?: boolean;
  preferredIde?: string;
  handoffContextTokens?: number | null;
  autoCommitPlan?: boolean;
  thinkingEffort?: ThinkingEffort;
  browser?: BrowserPrefs;
  local?: LocalPrefs;
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
  cost: number;
  report?: string;
}

export type SessionMode = "brain" | "builder";

export type ThinkingEffort = "low" | "medium" | "high" | "xhigh" | "max";

/// Slider order, lowest to highest — index in this array is the range value.
export const THINKING_EFFORTS: ThinkingEffort[] = ["low", "medium", "high", "xhigh", "max"];

export function normalizeThinkingEffort(s: unknown): ThinkingEffort {
  return THINKING_EFFORTS.includes(s as ThinkingEffort) ? (s as ThinkingEffort) : "medium";
}

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

/** One layer of the quality harness, as scored against one stack. */
export interface QualityLayerView {
  layer: string;
  stack: string;
  status: "pass" | "fail" | "unavailable";
  summary: string;
}

/** A verification run finished. `trigger` is "tool" when the agent asked for
 * it and "harness" when the loop enforced it before letting the run finish. */
export interface QualityVerdictData {
  pass: boolean;
  summary: string;
  layers: QualityLayerView[];
  trigger: "tool" | "harness";
}

/// Why a session handed off to a linked successor.
export type HandoffReason =
  | "plan_execution"
  | "golden_flip"
  | "context_handoff"
  | "manual_builder";

export interface SessionLinkedData {
  prevSessionId: string;
  sessionId: string;
  reason: HandoffReason;
  mode: SessionMode;
  firstMessage: string;
}

export type AgentEvent =
  | { event: "TextStep"; data: { text: string } }
  | { event: "TextDelta"; data: { text: string } }
  | { event: "ModeChanged"; data: ModeChangedData }
  | { event: "GoldenLoop"; data: GoldenLoopData }
  | { event: "QualityVerdict"; data: QualityVerdictData }
  | { event: "SessionLinked"; data: SessionLinkedData }
  | { event: "Thinking"; data: string }
  | { event: "ToolCall"; data: ToolCallData }
  | { event: "ToolResult"; data: ToolResultData }
  | { event: "ToolResultImages"; data: ToolResultImagesData }
  | { event: "AskUser"; data: AskUserData }
  | { event: "Done"; data: DoneData }
  | { event: "SteeringInjected"; data: { text: string; attachments?: Array<{ name: string; mediaType: string; size: number }> } }
  | { event: "Error"; data: string }
  | { event: "Retrying"; data: RetryingData }
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

/** Transient provider failure being retried with backoff (claudin.io
 * failover can take ~2min) — the UI shows a reconnecting banner instead of
 * dropping the run. */
export interface RetryingData {
  attempt: number;
  maxAttempts: number;
  delayMs: number;
  error: string;
}

export interface AskUserOption {
  /** Concise choice shown on the button. */
  label: string;
  /** Optional one-line explanation rendered under the label. */
  description?: string;
}

export interface AskUserQuestion {
  question: string;
  options: AskUserOption[];
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
  /** Merged in from the ToolResultImages event that follows this one. */
  images?: ToolImageData[];
}

/** An image a tool produced, already compressed and base64-encoded. */
export interface ToolImageData {
  mediaType: string;
  data: string;
  width: number;
  height: number;
}

export interface ToolResultImagesData {
  toolId: string;
  images: ToolImageData[];
}

export interface DoneData {
  stopReason: string;
  textOutput: string;
  inputTokens: number;
  outputTokens: number;
}

// --- Git ---

export interface ChangedFile {
  path: string;
  status: string;
  additions: number;
  deletions: number;
}

export interface GitStatus {
  hasChanges: boolean;
  files: ChangedFile[];
  totalAdditions: number;
  totalDeletions: number;
}

export function gitStatus(workspace: string): Promise<GitStatus> {
  return invoke<GitStatus>("git_status", { workspace });
}

export function gitFileDiff(workspace: string, path: string): Promise<string> {
  return invoke<string>("git_file_diff", { workspace, path });
}

export function gitBranch(workspace: string): Promise<string> {
  return invoke<string>("git_branch", { workspace });
}

export function checkGitAvailable(): Promise<boolean> {
  return invoke<boolean>("check_git_available");
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

export function commitAndPush(
  workspace: string,
  onEvent: (event: AgentEvent) => void,
): Promise<{ sessionId: string }> {
  const channel = new Channel<AgentEvent>();
  channel.onmessage = onEvent;
  return invoke<{ sessionId: string }>("commit_and_push", { workspace, eventChannel: channel });
}

export function setSessionMode(workspace: string, mode: SessionMode): Promise<SessionStarted> {
  return invoke<SessionStarted>("set_session_mode", { workspace, mode });
}

/// Approve the Brain's plan: creates a NEW linked Builder session whose first
/// prompt carries the plan, and starts executing it. Returns the new session id.
export function continueWithBuilderSession(
  workspace: string,
  onEvent: (event: AgentEvent) => void,
): Promise<SessionStarted> {
  const channel = new Channel<AgentEvent>();
  channel.onmessage = onEvent;
  return invoke<SessionStarted>("continue_with_builder", { workspace, eventChannel: channel });
}

export function getSessionMode(workspace: string): Promise<{ mode: SessionMode; origin: ModeOrigin }> {
  return invoke<{ mode: SessionMode; origin: ModeOrigin }>("get_session_mode", { workspace });
}

export function checkPlanExists(workspace: string): Promise<boolean> {
  return invoke<boolean>("check_plan_exists", { workspace });
}

export interface PlanEntry {
  name: string;
  path: string;
  modifiedAt: number;
}

export function listPlans(workspace: string): Promise<PlanEntry[]> {
  return invoke<PlanEntry[]>("list_plans", { workspace });
}

export function readAttachment(path: string): Promise<AttachmentData> {
  return invoke<AttachmentData>("read_attachment", { path });
}

export function writeClipboardBlob(data: string, name: string, mediaType: string): Promise<WriteClipboardBlobResult> {
  return invoke<WriteClipboardBlobResult>("write_clipboard_blob", { data, name, mediaType });
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
  kind: "meta" | "user" | "phase" | "turn" | "phase_result" | "done" | "error" | "steering" | "compacted" | "status" | "mode" | "tasks" | "golden_cycle" | "continuation_judge" | "base_commit" | "plan_finalized" | "linked_from" | "handoff_to" | "handoff";
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

export function queueSteering(sessionId: string, text: string, attachments?: AttachmentInput[]): Promise<void> {
  return invoke<void>("queue_steering", { sessionId, text, attachments: attachments ?? null });
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

// --- Browser ---

export interface BrowserPrefs {
  enabled: boolean;
  headless: boolean;
  chromePath?: string | null;
  viewportWidth: number;
  viewportHeight: number;
}

export interface BrowserStatus {
  installed: boolean;
  version: string;
  exePath?: string | null;
  downloadSize: number;
  systemChrome?: string | null;
  supported: boolean;
}

export interface BrowserInstallProgress {
  downloadedBytes: number;
  totalBytes: number;
  phase: "download" | "extract";
}

export function browserStatus(): Promise<BrowserStatus> {
  return invoke<BrowserStatus>("browser_status");
}

export function browserInstall(): Promise<BrowserStatus> {
  return invoke<BrowserStatus>("browser_install");
}

export function browserUninstall(): Promise<BrowserStatus> {
  return invoke<BrowserStatus>("browser_uninstall");
}

export function browserTest(): Promise<string> {
  return invoke<string>("browser_test");
}

export function browserClose(): Promise<void> {
  return invoke<void>("browser_close");
}

// --- Local models (llama.cpp) ---

export type LlamaBackend = "auto" | "cpu" | "vulkan";
/** llama.cpp runs everywhere; MLX is Apple Silicon only and faster there. */
export type LocalEngine = "llamacpp" | "mlx";
export type Fit = "comfortable" | "tight" | "wontFit";

export interface LocalPrefs {
  enabled: boolean;
  serverPath?: string | null;
  backend: LlamaBackend;
  engine: LocalEngine;
  ctxSize: number;
  gpuLayers: string;
  parallel: number;
  sleepIdleSeconds: number;
  maxLoadedModels: number;
}

export interface LocalStatus {
  supported: boolean;
  build: string;
  target?: string | null;
  serverInstalled: boolean;
  exePath?: string | null;
  downloadSize: number;
  systemServer?: string | null;
  /** The MLX runtime is independent of the llama.cpp one. */
  mlxSupported: boolean;
  mlxInstalled: boolean;
  mlxVersion: string;
  mlxDownloadSize: number;
  /** What will actually run, after falling back if the configured engine is
   *  not available on this machine. */
  engine: LocalEngine;
}

export interface HardwareProfile {
  totalRamBytes: number;
  availableRamBytes: number;
  vramBytes?: number | null;
  unifiedMemory: boolean;
  logicalCores: number;
  gpuName?: string | null;
}

/** A model offered as a starting point: what the Hub is trending for the
 *  preferred engine, or the built-in list when the Hub is unreachable. */
export interface SuggestedModel {
  repo: string;
  displayName: string;
  downloads: number;
  likes: number;
  /** Only the built-in entries carry one. */
  blurb?: string | null;
  /** True when the Hub could not be reached and this is the fallback list. */
  offline: boolean;
}

export interface HfModelSummary {
  repo: string;
  downloads: number;
  likes: number;
  gated: boolean;
}

export interface QuantOption {
  quant: string;
  totalBytes: number;
  shards: number;
  fit: Fit;
}

export interface RepoQuants {
  repo: string;
  gated: boolean;
  quants: QuantOption[];
  recommended?: string | null;
  contextLength?: number | null;
  hasChatTemplate: boolean;
  architecture?: string | null;
  /** Which engine can run what this repo publishes. */
  format: "gguf" | "mlx";
}

export interface LocalModel {
  key: string;
  displayName: string;
  repo: string;
  quant: string;
  totalBytes: number;
  contextLength?: number | null;
  hasChatTemplate: boolean;
  architecture?: string | null;
  format: "gguf" | "mlx";
  installedAt: string;
}

export interface LocalModelView extends LocalModel {
  running: boolean;
  complete: boolean;
  fit: Fit;
  benchmark?: ModelBenchmark | null;
}

/** Emitted on "local-model-download-progress" and
 *  "llama-server-install-progress". `overall*` spans every shard — a
 *  three-shard model whose bar resets twice reads as a stuck download. */
export interface ModelDownloadProgress {
  key: string;
  fileIndex: number;
  fileCount: number;
  downloadedBytes: number;
  totalBytes: number;
  overallDone: number;
  overallTotal: number;
  phase: "download" | "verify" | "done";
}

export function localStatus(): Promise<LocalStatus> {
  return invoke<LocalStatus>("local_status");
}

export function localHardware(): Promise<HardwareProfile> {
  return invoke<HardwareProfile>("local_hardware");
}

export function localCuratedModels(): Promise<SuggestedModel[]> {
  return invoke<SuggestedModel[]>("local_curated_models");
}

export function localInstallServer(): Promise<LocalStatus> {
  return invoke<LocalStatus>("local_install_server");
}

export function localUninstallServer(): Promise<LocalStatus> {
  return invoke<LocalStatus>("local_uninstall_server");
}

export function localInstallMlx(): Promise<LocalStatus> {
  return invoke<LocalStatus>("local_install_mlx");
}

export function localUninstallMlx(): Promise<LocalStatus> {
  return invoke<LocalStatus>("local_uninstall_mlx");
}

export function localSearchModels(query: string, limit?: number): Promise<HfModelSummary[]> {
  return invoke<HfModelSummary[]>("local_search_models", { query, limit: limit ?? null });
}

export function localRepoQuants(repo: string): Promise<RepoQuants> {
  return invoke<RepoQuants>("local_repo_quants", { repo });
}

export function localInstallModel(repo: string, quant: string): Promise<LocalModel> {
  return invoke<LocalModel>("local_install_model", { repo, quant });
}

export function localCancelInstall(key: string): Promise<void> {
  return invoke<void>("local_cancel_install", { key });
}

export function localListModels(): Promise<LocalModelView[]> {
  return invoke<LocalModelView[]>("local_list_models");
}

export function localRemoveModel(key: string): Promise<void> {
  return invoke<void>("local_remove_model", { key });
}

export function localUnloadModel(key: string): Promise<void> {
  return invoke<void>("local_unload_model", { key });
}

export function localServerLogs(key: string): Promise<string[]> {
  return invoke<string[]>("local_server_logs", { key });
}

export function localDiskUsage(): Promise<number> {
  return invoke<number>("local_disk_usage");
}

export function localTestModel(key: string): Promise<string> {
  return invoke<string>("local_test_model", { key });
}

/** What a resident model is costing and producing right now. */
/** What a local model is doing. `loading` and `readingPrompt` are the phases
 *  that produce no output, which is what reads as a hang. */
export type LocalPhase = "loading" | "readingPrompt" | "generating" | "idle" | "sleeping";

/** What a model has cost on this machine: a benchmark of your hardware, not
 *  of the model's published numbers. */
export interface ModelBenchmark {
  modelKey: string;
  loadSeconds: number;
  loadSamples: number;
  firstTokenSeconds: number;
  tokensPerSecond: number;
  promptTokensPerSecond: number;
  generationSamples: number;
  lastPromptTokens: number;
  lastRunAt: string;
}

export interface LocalModelStats {
  modelKey: string;
  displayName: string;
  engine: LocalEngine;
  phase: LocalPhase;
  /** Weights plus KV cache: the number that explains a large context. */
  memoryBytes: number;
  ctxSize: number;
  ctxUsed: number;
  tokensPerSecond: number;
  promptTokensPerSecond: number;
  tokensGenerated: number;
  busy: boolean;
  /** Weights unloaded after an idle period; the port is still there. */
  sleeping: boolean;
}

export function localRuntimeStats(): Promise<LocalModelStats[]> {
  return invoke<LocalModelStats[]>("local_runtime_stats");
}

export function getConfig(workspace?: string): Promise<AgentConfig> {
  return invoke<AgentConfig>("get_config", { workspace: workspace ?? null });
}

/** What triggers the harness's finish-line verification. */
export type EnforceOn = "goals" | "code_change";

/** Per-project quality settings, stored in the workspace's `.claudinio.json`. */
export interface QualitySettings {
  enabled: boolean;
  enforceOn: EnforceOn;
  /** Layers that block a finish: "tests" and/or "coverage". Empty = report only. */
  enforcedLayers: string[];
  diffCoverageThreshold: number;
  mutationScoreThreshold: number;
  /** Empty = use the detected command. */
  testCmd: string;
  coverageCmd: string;
  mutationCmd: string;
  /** Empty = the default "features" directory. */
  featuresDir: string;
  gherkinCmd: string;
  /** 0 = no budget; the complexity layer reports without blocking. */
  maxComplexity: number;
  testTimeoutSecs: number;
  coverageTimeoutSecs: number;
  mutationTimeoutSecs: number;
}

/** A build root the harness found, and what it would run there. */
export interface DetectedStack {
  name: string;
  root: string;
  testCmd: string;
  coverageCmd: string | null;
  mutationCmd: string | null;
  gherkinCmd: string | null;
}

export interface QualityInfo {
  settings: QualitySettings;
  stacks: DetectedStack[];
}

export function getQualityConfig(workspaceRoot: string): Promise<QualityInfo> {
  return invoke<QualityInfo>("get_quality_config", { workspaceRoot });
}

export function setQualityConfig(
  workspaceRoot: string,
  settings: QualitySettings,
): Promise<void> {
  return invoke<void>("set_quality_config", { workspaceRoot, settings });
}

export function setKeepAwake(active: boolean): Promise<void> {
  return invoke<void>("set_keep_awake", { active });
}

export function setWorkspaceConfig(workspaceRoot: string, planSavePath: string | null): Promise<void> {
  return invoke<void>("set_workspace_config", { workspaceRoot, planSavePath });
}

export function listMcpServers(workspace?: string): Promise<McpServerStatus[]> {
  return invoke<McpServerStatus[]>("mcp_list_servers", { workspace: workspace ?? null });
}

export function testMcpServer(name: string, entry: McpServerEntry, workspace?: string): Promise<McpServerStatus> {
  return invoke<McpServerStatus>("mcp_test_server", { name, entry, workspace: workspace ?? null });
}

export function reconnectMcp(workspace: string): Promise<McpServerStatus[]> {
  return invoke<McpServerStatus[]>("mcp_reconnect", { workspace });
}

export function listModels(): Promise<string[]> {
  return invoke<string[]>("list_models");
}

// --- External providers (OpenRouter + models.dev catalog) ---

export interface CatalogModel {
  id: string;
  name: string;
  costInput?: number | null;
  costOutput?: number | null;
  context?: number | null;
  outputLimit?: number | null;
  reasoning?: boolean;
  toolCall?: boolean;
}

export interface CatalogProvider {
  id: string;
  name: string;
  api: string;
  env: string[];
  doc?: string | null;
  protocol: "openai" | "anthropic";
  models: CatalogModel[];
}

/** One picker group per provider; external models are "<providerId>/<model>" qualified. */
export interface ModelGroup {
  providerId: string;
  providerName: string;
  models: string[];
  /** Display name per model id, for ids that are not readable on their own
   *  (a local model is keyed by a content hash). Absent for providers whose
   *  ids already read as names. */
  labels?: Record<string, string>;
}

/** OpenRouter OAuth PKCE connect; resolves with the live model list. */
export function openrouterLogin(): Promise<string[]> {
  return invoke<string[]>("openrouter_login");
}

/** Abort a pending openrouterLogin stuck waiting for the browser callback. */
export function openrouterLoginCancel(): Promise<void> {
  return invoke<void>("openrouter_login_cancel");
}

export function fetchProviderCatalog(force?: boolean): Promise<{ providers: CatalogProvider[] }> {
  return invoke<{ providers: CatalogProvider[] }>("fetch_provider_catalog", { force: force ?? false });
}

export function connectProvider(providerId: string, apiKey: string, baseUrl?: string): Promise<string[]> {
  return invoke<string[]>("connect_provider", { providerId, apiKey, baseUrl: baseUrl ?? null });
}

export function disconnectProvider(providerId: string): Promise<void> {
  return invoke<void>("disconnect_provider", { providerId });
}

export function listProviderModels(providerId: string): Promise<string[]> {
  return invoke<string[]>("list_provider_models", { providerId });
}

export function listAllModels(): Promise<ModelGroup[]> {
  return invoke<ModelGroup[]>("list_all_models");
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

/** Validates an API key by calling the models endpoint. Returns model list on success, throws on failure. */
export function validateApiKey(apiKey: string): Promise<string[]> {
  return invoke<string[]>("validate_api_key", { apiKey });
}

// --- Code Intelligence ---

export interface IndexStatus {
  status: string;
  filesCount: number;
  symbolsCount: number;
  embeddingsCount: number;
  watcherWarning?: string;
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

/** Write inside an open workspace. Paths outside it are rejected by the backend. */
export function writeFile(path: string, content: string): Promise<void> {
  return invoke<void>("write_file", { path, content });
}

/** Write binary content (base64-encoded) inside an open workspace. */
export function writeFileBytes(path: string, base64Data: string): Promise<void> {
  return invoke<void>("write_file_bytes", { path, base64Data });
}

// --- Export (save outside the workspace) ---
//
// The save dialog runs in Rust, so the destination never crosses IPC as an
// argument this side could be tricked into supplying. That is what keeps
// writeFile/writeFileBytes strictly workspace-scoped. Resolves false if the
// user cancelled the dialog.

export function exportFile(
  defaultName: string,
  filterName: string,
  extension: string,
  content: string,
): Promise<boolean> {
  return invoke<boolean>("export_file", { defaultName, filterName, extension, content });
}

export function exportFileBytes(
  defaultName: string,
  filterName: string,
  extension: string,
  base64Data: string,
): Promise<boolean> {
  return invoke<boolean>("export_file_bytes", { defaultName, filterName, extension, base64Data });
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

/// Drop golden tasks so a stale `<goal>` from an earlier turn stops
/// re-triggering the golden loop. Omit `taskId` to drop all golden tasks.
export function dismissGoldenTasks(workspace: string, taskId?: string): Promise<TaskItem[]> {
  return invoke<TaskItem[]>("dismiss_golden_tasks", { workspace, taskId: taskId ?? null });
}

export interface EnhancePromptContext {
  messages: Array<{ role: string; text: string }>;
  mode: string;
  mentionedFiles: string[];
  activeTaskTitles: string[];
  projectSummary: string;
}

export function enhancePrompt(
  workspace: string,
  prompt: string,
  context: EnhancePromptContext
): Promise<string> {
  return invoke("enhance_prompt", { workspace, prompt, context });
}

// --- Skills ---

export interface SkillEntry {
  name: string;
  description: string;
  location: string;
  scope: "builtin" | "project" | "subfolder" | "user" | "plugin";
  body?: string;
}

export interface SkillCatalogEntry {
  name: string;
  description: string;
  location: string;
  scope: "builtin" | "project" | "subfolder" | "user" | "plugin";
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

// --- Agent Plugins (https://agent-plugins.org) ---

export interface PluginAuthor {
  name?: string;
  email?: string;
  url?: string;
}

export interface PluginDiagnostic {
  severity: "error" | "warning";
  message: string;
}

export interface PluginSkillInfo {
  name: string;
  description: string;
  location: string;
}

export interface PluginMcpInfo {
  name: string;
  qualifiedName: string;
  transport: string;
}

export interface PluginInfo {
  name: string;
  root: string;
  scope: "project" | "user";
  enabled: boolean;
  /** False when the manifest itself was rejected — no components load. */
  valid: boolean;
  version?: string;
  description?: string;
  author?: PluginAuthor;
  homepage?: string;
  repository?: string;
  license?: string;
  keywords: string[];
  skills: PluginSkillInfo[];
  mcpServers: PluginMcpInfo[];
  diagnostics: PluginDiagnostic[];
}

export type PluginInstallScope = "user" | "project";

export interface ScaffoldSkillInput {
  name: string;
  description: string;
}

export interface ScaffoldPluginArgs {
  name: string;
  description?: string;
  version?: string;
  authorName?: string;
  authorEmail?: string;
  authorUrl?: string;
  homepage?: string;
  repository?: string;
  license?: string;
  keywords?: string[];
  skills?: ScaffoldSkillInput[];
  mcpServers?: {
    name: string;
    transport: "stdio" | "streamable-http";
    command?: string;
    args?: string[];
    url?: string;
  }[];
  dest?: string;
  scope?: PluginInstallScope;
  workspace?: string | null;
}

export interface ScaffoldPluginResult {
  root: string;
  files: string[];
  plugin: PluginInfo;
}

export function listPlugins(workspace: string | null): Promise<PluginInfo[]> {
  return invoke<PluginInfo[]>("plugins_list", { workspace });
}

/** Load a directory as a plugin without installing it, to preview diagnostics. */
export function inspectPlugin(path: string): Promise<PluginInfo> {
  return invoke<PluginInfo>("plugins_inspect", { path });
}

export function setPluginEnabled(
  name: string,
  enabled: boolean,
  workspace: string | null,
): Promise<PluginInfo[]> {
  return invoke<PluginInfo[]>("plugins_set_enabled", { name, enabled, workspace });
}

export function installPluginFromPath(args: {
  path: string;
  scope?: PluginInstallScope;
  workspace?: string | null;
}): Promise<PluginInfo> {
  return invoke<PluginInfo>("plugins_install_from_path", { args });
}

export function installPluginFromUrl(args: {
  url: string;
  gitRef?: string | null;
  subdir?: string | null;
  scope?: PluginInstallScope;
  workspace?: string | null;
}): Promise<PluginInfo> {
  return invoke<PluginInfo>("plugins_install_from_url", { args });
}

export function uninstallPlugin(name: string, workspace: string | null): Promise<PluginInfo[]> {
  return invoke<PluginInfo[]>("plugins_uninstall", { name, workspace });
}

export function scaffoldPlugin(args: ScaffoldPluginArgs): Promise<ScaffoldPluginResult> {
  return invoke<ScaffoldPluginResult>("plugins_scaffold", { args });
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

/** Abre uma URL no navegador padrão (best-effort). */
export function openExternalUrl(url: string): void {
  openUrl(url).catch(() => {});
}

// ── Auto-update (tauri-plugin-updater) ─────────────────────────────

export interface UpdateInfo {
  version: string;
  currentVersion: string;
  body: string | null;
  /** Baixa, instala e reinicia o app. Progresso em [0, 1] (ou -1 se tamanho desconhecido). */
  install: (onProgress?: (fraction: number) => void) => Promise<void>;
}

/** Retorna a atualização disponível, ou null se já está na última versão. */
export async function checkForUpdate(): Promise<UpdateInfo | null> {
  const update = await check();
  if (!update) return null;
  return {
    version: update.version,
    currentVersion: update.currentVersion,
    body: update.body ?? null,
    install: async (onProgress) => {
      let total = 0;
      let received = 0;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? 0;
        } else if (event.event === "Progress") {
          received += event.data.chunkLength;
          onProgress?.(total > 0 ? Math.min(received / total, 1) : -1);
        } else if (event.event === "Finished") {
          onProgress?.(1);
        }
      });
      // No Windows o instalador encerra o app sozinho; nos demais, relança.
      await relaunch();
    },
  };
}

// ── Network Log ────────────────────────────────────────────────────────

export interface LogEntry {
  workspace: string;
  timestamp: string;
  source: string;
  detail: string;
  durationMs: number;
  bytes: number;
  statusCode?: number;
}

export function getNetworkLog(workspace: string): Promise<LogEntry[]> {
  return invoke<LogEntry[]>("get_network_log", { workspace });
}

// ── Askpass bridge ─────────────────────────────────────────────────────
// A git/ssh credential prompt intercepted by the backend (askpass.rs) and
// surfaced as an `askpass-request` event; answer resolves the waiting command.
export interface AskpassRequest {
  id: number;
  prompt: string;
}

/** Reply to a pending askpass prompt. `secret: null` cancels it. */
export function answerAskpass(id: number, secret: string | null): Promise<void> {
  return invoke("answer_askpass", { id, secret });
}
