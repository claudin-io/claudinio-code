import type { LocaleDict } from "../grill-me";

const dict: LocaleDict = {
  // ── App ───────────────────────────────────────────────────────────
  "app.title": "Claudinio Code",
  "app.config.title": "API Configuration",
  "app.config.apiKey": "API Key",
  "app.config.baseUrl": "Base URL",
  "app.config.model": "Model",
  "app.config.maxRounds": "Max rounds (main agent)",
  "app.config.subMaxRounds": "Max rounds (subagents)",
  "app.config.unlimited": "Unlimited (default)",
  "app.config.maxRoundsHint": "Leave empty for unlimited. Sets how many tool calls the agent may make per submission.",
  "app.config.subMaxRoundsHint": "Leave empty for unlimited. Sets the limit per subagent.",
  "app.config.cancel": "Cancel",
  "app.config.save": "Save",
  "app.sidebar.projects": "Projects",
  "app.sidebar.noRecent": "No recent projects",
  "app.sidebar.openFolder": "Open folder",
  "app.sidebar.browseFiles": "Browse files",
  "app.sidebar.back": "Back",
  "app.sidebar.closeWorkspace": "Close workspace",
  "app.index.loadingModel": "Loading model…",
  "app.index.indexing": "Indexing",
  "app.index.generatingEmbeddings": "Generating embeddings",
  "app.index.embeddingsReady": "Embeddings ready",
  "app.index.embeddingFailed": "Embedding failed — semantic search unavailable",
  "app.index.symbols": "symbols",
  "app.index.filesCount": "{0} files, {1} symbols",
  "app.index.indexingStatus": "indexing…",
  "app.index.embeddingStatus": "Generating embeddings",
  "app.config.saveError": "Error saving config: {0}",
  "app.config.yoloMode": "⚡ YOLO Mode (auto-approve all)",
  "app.config.yoloModeHint": "Auto-approves tool calls except those in the blacklist below.",
  "app.config.yoloBlacklist": "YOLO Blacklist (comma-separated tool names)",
  "app.config.yoloBlacklistHint": "These tools still require manual approval even with YOLO on. Ex: edit_file, bash",

  // ── EmptyState ────────────────────────────────────────────────────
  "empty.title": "Claudinio Code",
  "empty.subtitle": "Open a project folder to start using the agent.",
  "empty.openFolder": "Open folder",
  "empty.recent": "Recent",

  // ── ChatPanel - Header ────────────────────────────────────────────
  "chat.header.agent": "Agent",
  "chat.header.newSession": "New session",
  "chat.header.new": "New",
  "chat.header.savedSessions": "Saved sessions",
  "chat.header.history": "History",
  "chat.header.noSessions": "No saved sessions.",
  "chat.header.sessionTitle": "{0} · {1} turn{2}",
  "chat.header.turns": "s",
  "chat.header.turn": "",

  // ── ChatPanel - Status ────────────────────────────────────────────
  "chat.status.thinking": "Working",
  "chat.status.awaitingApproval": "Awaiting approval",
  "chat.status.awaitingInput": "Awaiting your input",
  "chat.status.done": "Done",
  "chat.status.error": "Error",
  "chat.status.idle": "Idle",

  // ── ChatPanel - Messages ──────────────────────────────────────────
  "chat.message.you": "You",
  "chat.message.agent": "Agent",
  "chat.message.failedToSend": "Failed to send: {0}",
  "chat.message.failedToReopen": "Failed to reopen: {0}",
  "chat.message.failedToCompact": "Compaction failed: {0}",
  "chat.scrollToBottom": "Scroll to bottom",

  // ── ChatPanel - Phases ────────────────────────────────────────────
  "chat.phase.plan": "Plan",
  "chat.phase.execute": "Execute",
  "chat.phase.summary": "Summary",

  // ── ChatPanel - Timeline ──────────────────────────────────────────
  "chat.timeline.thought": "Thought",
  "chat.timeline.steering": "steering",
  "chat.timeline.waiting": "Waiting for response...",
  "chat.timeline.workedFor": "Worked for {0}",
  "chat.timeline.steps": "{0} step{1}",
  "chat.timeline.args": "Arguments",
  "chat.timeline.result": "Result",

  // ── ChatPanel - Subagent ──────────────────────────────────────────
  "chat.subagent.running": "Working",
  "chat.subagent.completed": "{0} rounds",
  "chat.subagent.failed": "Failed",
  "chat.subagent.interrupted": "Interrupted",
  "chat.subagent.maxRounds": "Max rounds",
  "chat.subagent.rounds": "rounds",
  "chat.subagent.title": "Subagent: {0}",

  // ── ChatPanel - Input ─────────────────────────────────────────────
  "chat.input.attachFile": "Attach file",
  "chat.input.compacting": "Compacting context…",
  "chat.input.approveFirst": "Approve or reject the edit first…",
  "chat.input.answerFirst": "Answer the questions above first…",
  "chat.input.steerAgent": "Type to steer the agent… (Esc to pause)",
  "chat.input.stop": "Stop",
  "chat.input.askCode": "Ask something about the code…",
  "chat.input.dropToAttach": "Drop file to attach",
  "chat.input.dropHint": "Images, PDFs, docs, code and more",

  // ── ChatPanel - Approval ──────────────────────────────────────────
  "chat.approval.proposedEdit": "Proposed edit",
  "chat.approval.bashCommand": "Bash command",
  "chat.approval.approve": "Approve",
  "chat.approval.reject": "Reject",
  "chat.approval.failed": "Approval failed: {0}",
  "chat.approval.rejectFailed": "Rejection failed: {0}",

  // ── ChatPanel - Question ──────────────────────────────────────────
  "chat.question.needsAnswer": "The agent needs your answer",
  "chat.question.other": "Other answer…",
  "chat.question.typeAnswer": "Type your answer…",
  "chat.question.submit": "Submit",
  "chat.question.answerFailed": "Failed to submit answers: {0}",

  // ── ChatPanel - Context Footer ────────────────────────────────────
  "chat.context.nextRequest": "Context for next request",
  "chat.context.sessionTokens": "Session cumulative tokens",
  "chat.context.sessionCost": "Session cumulative cost",
  "chat.context.total": "total: {0}",
  "chat.context.compact": "Compact",
  "chat.context.compacting": "Compacting…",
  "chat.context.compactLabel": "{0} / {1}",

  // ── ChatPanel - Compaction ────────────────────────────────────────
  "chat.compact.start": "Context at {0}k / {1}k — compacting…",
  "chat.compact.done": "Context compacted: ~{0}k → ~{1}k tokens.",
  "chat.compact.fail": "Compaction failed: {0} — continuing with full context.",

  // ── ChatPanel - Archived ──────────────────────────────────────────
  "chat.archived.title": "Compacted history",
  "chat.archived.messages": "{0} messages",
  "chat.archived.you": "You",
  "chat.archived.agent": "Agent",

  // ── ChatPanel - Drop overlay ──────────────────────────────────────
  "chat.drop.title": "Drop file to attach",
  "chat.drop.hint": "Images, PDFs, docs, code and more",
  // ── Tasks Panel ──────────────────────────────────────────────
  "tasks.panel.title": "Tasks",
  "tasks.panel.noTasks": "No tasks yet — ask the agent to create some",
  "tasks.panel.showDetails": "Show details",
  "tasks.panel.hideDetails": "Hide details",
  "tasks.panel.journal": "Journal",
  "tasks.panel.cycleStatus": "Cycle status",
  "tasks.panel.collapse": "Collapse",
  "tasks.panel.expand": "Show tasks",
  "tasks.panel.refresh": "Refresh",
  "tasks.status.todo": "Todo",
  "tasks.status.doing": "Doing",
  "tasks.status.done": "Done",
};

export default dict;
