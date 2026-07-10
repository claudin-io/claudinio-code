import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";

// ─────────────────────────────────────────────────────────────
// getSessionStats
// ─────────────────────────────────────────────────────────────

describe("getSessionStats", () => {
  it("returns all zeros and undefined optionals for empty array", async () => {
    const { getSessionStats } = await import("./ipc");
    const result = getSessionStats([]);
    expect(result).toEqual({
      totalInputTokens: 0,
      totalOutputTokens: 0,
      totalCost: undefined,
      costInput: undefined,
      costOutput: undefined,
      costCacheRead: undefined,
      contextTokens: undefined,
    });
  });

  it("returns all zeros and undefined optionals when no status record exists", async () => {
    const { getSessionStats } = await import("./ipc");
    const records = [
      { kind: "meta", sessionId: "abc" },
      { kind: "user", content: "hello" },
      { kind: "turn", role: "assistant" },
      { kind: "phase", phase: "execute" },
    ] as import("./ipc").SessionRecord[];
    const result = getSessionStats(records);
    expect(result).toEqual({
      totalInputTokens: 0,
      totalOutputTokens: 0,
      totalCost: undefined,
      costInput: undefined,
      costOutput: undefined,
      costCacheRead: undefined,
      contextTokens: undefined,
    });
  });

  it("extracts all fields from a single status record", async () => {
    const { getSessionStats } = await import("./ipc");
    const records = [
      {
        kind: "status",
        total_input_tokens: 500,
        total_output_tokens: 1200,
        total_cost: 0.042,
        total_cost_input: 0.012,
        total_cost_output: 0.03,
        total_cost_cache_read: 0.005,
        context_tokens: 3200,
      },
    ] as import("./ipc").SessionRecord[];
    const result = getSessionStats(records);
    expect(result).toEqual({
      totalInputTokens: 500,
      totalOutputTokens: 1200,
      totalCost: 0.042,
      costInput: 0.012,
      costOutput: 0.03,
      costCacheRead: 0.005,
      contextTokens: 3200,
    });
  });

  it("coerces string number values to Number", async () => {
    const { getSessionStats } = await import("./ipc");
    const records = [
      {
        kind: "status",
        total_input_tokens: "1500",
        total_output_tokens: "3400",
      },
    ] as import("./ipc").SessionRecord[];
    const result = getSessionStats(records);
    expect(result.totalInputTokens).toBe(1500);
    expect(result.totalOutputTokens).toBe(3400);
  });

  it("defaults missing token fields to 0", async () => {
    const { getSessionStats } = await import("./ipc");
    const records = [
      {
        kind: "status",
      },
    ] as import("./ipc").SessionRecord[];
    const result = getSessionStats(records);
    expect(result.totalInputTokens).toBe(0);
    expect(result.totalOutputTokens).toBe(0);
  });

  it("sets optional fields to undefined when absent", async () => {
    const { getSessionStats } = await import("./ipc");
    const records = [
      {
        kind: "status",
        total_input_tokens: 100,
        total_output_tokens: 200,
      },
    ] as import("./ipc").SessionRecord[];
    const result = getSessionStats(records);
    expect(result.totalCost).toBeUndefined();
    expect(result.costInput).toBeUndefined();
    expect(result.costOutput).toBeUndefined();
    expect(result.costCacheRead).toBeUndefined();
    expect(result.contextTokens).toBeUndefined();
  });

  it("sets optional fields to undefined when null", async () => {
    const { getSessionStats } = await import("./ipc");
    const records = [
      {
        kind: "status",
        total_input_tokens: 100,
        total_output_tokens: 200,
        total_cost: null,
        total_cost_input: null,
        total_cost_output: null,
        total_cost_cache_read: null,
        context_tokens: null,
      },
    ] as import("./ipc").SessionRecord[];
    const result = getSessionStats(records);
    expect(result.totalCost).toBeUndefined();
    expect(result.costInput).toBeUndefined();
    expect(result.costOutput).toBeUndefined();
    expect(result.costCacheRead).toBeUndefined();
    expect(result.contextTokens).toBeUndefined();
  });

  it("uses the last status record (overwrites previous ones)", async () => {
    const { getSessionStats } = await import("./ipc");
    const records = [
      {
        kind: "status",
        total_input_tokens: 100,
        total_output_tokens: 200,
        total_cost: 0.01,
      },
      {
        kind: "status",
        total_input_tokens: 999,
        total_output_tokens: 888,
        total_cost: 0.99,
        total_cost_input: 0.4,
      },
    ] as import("./ipc").SessionRecord[];
    const result = getSessionStats(records);
    expect(result.totalInputTokens).toBe(999);
    expect(result.totalOutputTokens).toBe(888);
    expect(result.totalCost).toBe(0.99);
    expect(result.costInput).toBe(0.4);
  });

  it("interleaves non-status records without affecting the result", async () => {
    const { getSessionStats } = await import("./ipc");
    const records = [
      { kind: "meta", sessionId: "s1" },
      { kind: "user", content: "hi" },
      { kind: "status", total_input_tokens: 50, total_output_tokens: 100, total_cost: 0.005 },
      { kind: "turn", role: "assistant" },
      { kind: "phase", phase: "plan" },
    ] as import("./ipc").SessionRecord[];
    const result = getSessionStats(records);
    expect(result.totalInputTokens).toBe(50);
    expect(result.totalOutputTokens).toBe(100);
    expect(result.totalCost).toBe(0.005);
  });
});

// ─────────────────────────────────────────────────────────────
// openExternal
// ─────────────────────────────────────────────────────────────

describe("openExternal", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("delegates to openPath with the given path", async () => {
    const { openExternal } = await import("./ipc");
    const mockedOpenPath = vi.mocked(openPath);

    openExternal("/some/path");

    expect(mockedOpenPath).toHaveBeenCalledTimes(1);
    expect(mockedOpenPath).toHaveBeenCalledWith("/some/path");
  });

  it("does not throw when openPath rejects", async () => {
    const { openExternal } = await import("./ipc");
    vi.mocked(openPath).mockRejectedValueOnce(new Error("permission denied"));

    expect(() => openExternal("/bad/path")).not.toThrow();
  });

  it("does not throw when openPath resolves successfully", async () => {
    const { openExternal } = await import("./ipc");
    vi.mocked(openPath).mockResolvedValueOnce(undefined);

    expect(() => openExternal("/good/path")).not.toThrow();
  });
});

// ─────────────────────────────────────────────────────────────
// openExternalUrl — uses openUrl from @tauri-apps/plugin-opener
// ─────────────────────────────────────────────────────────────

describe("openExternalUrl", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("delegates to openUrl with the given url", async () => {
    const { openExternalUrl } = await import("./ipc");
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    const mockedOpenUrl = vi.mocked(openUrl);

    openExternalUrl("https://example.com");

    expect(mockedOpenUrl).toHaveBeenCalledTimes(1);
    expect(mockedOpenUrl).toHaveBeenCalledWith("https://example.com");
  });

  it("does not throw when openUrl rejects", async () => {
    const { openExternalUrl } = await import("./ipc");
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    vi.mocked(openUrl).mockRejectedValueOnce(new Error("network error"));

    expect(() => openExternalUrl("https://bad.example")).not.toThrow();
  });

  it("does not throw when openUrl resolves successfully", async () => {
    const { openExternalUrl } = await import("./ipc");
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    vi.mocked(openUrl).mockResolvedValueOnce(undefined);

    expect(() => openExternalUrl("https://good.example")).not.toThrow();
  });
});

// ─────────────────────────────────────────────────────────────
// normalizeSessionMode — edge cases beyond ipc-normalize.test.ts
// ─────────────────────────────────────────────────────────────

describe("normalizeSessionMode edge cases", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns 'brain' for string 'brain'", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode("brain")).toBe("brain");
  });

  it("returns 'brain' for string 'pensador'", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode("pensador")).toBe("brain");
  });

  it("returns 'builder' for string 'builder'", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode("builder")).toBe("builder");
  });

  it("returns 'builder' for string 'constructor'", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode("constructor")).toBe("builder");
  });

  it("returns 'builder' for number 0", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode(0)).toBe("builder");
  });

  it("returns 'builder' for number 1", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode(1)).toBe("builder");
  });

  it("returns 'builder' for boolean true", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode(true)).toBe("builder");
  });

  it("returns 'builder' for boolean false", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode(false)).toBe("builder");
  });

  it("returns 'builder' for an object", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode({})).toBe("builder");
  });

  it("returns 'builder' for an array", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode([])).toBe("builder");
  });

  it("returns 'builder' for a symbol", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode(Symbol("foo"))).toBe("builder");
  });
});

// ─────────────────────────────────────────────────────────────
// Basic Tauri invoke wrappers (simple command + args)
// ─────────────────────────────────────────────────────────────

function mockInvokeFor<const T>(value: T) {
  vi.mocked(invoke).mockResolvedValue(value);
}

describe("listDir", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with list_dir and path, returns DirEntry[]", async () => {
    const fake: import("./ipc").DirEntry[] = [
      { name: "a.ts", path: "/root/a.ts", isDir: false },
    ];
    mockInvokeFor(fake);
    const { listDir } = await import("./ipc");
    const result = await listDir("/root");
    expect(vi.mocked(invoke)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("list_dir", { path: "/root" });
    expect(result).toBe(fake);
  });
});

describe("readFile", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with read_file and path, returns content", async () => {
    mockInvokeFor("file content");
    const { readFile } = await import("./ipc");
    const result = await readFile("/path/to/file.ts");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("read_file", { path: "/path/to/file.ts" });
    expect(result).toBe("file content");
  });
});

describe("readAttachment", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with read_attachment and path, returns attachment data", async () => {
    const fake: import("./ipc").AttachmentData = {
      name: "img.png",
      mediaType: "image/png",
      data: "base64data",
      size: 1234,
    };
    mockInvokeFor(fake);
    const { readAttachment } = await import("./ipc");
    const result = await readAttachment("/path/img.png");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("read_attachment", { path: "/path/img.png" });
    expect(result).toEqual(fake);
  });
});

// ─────────────────────────────────────────────────────────────
// Session CRUD
// ─────────────────────────────────────────────────────────────

describe("newSession", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with new_session and workspace", async () => {
    mockInvokeFor(undefined);
    const { newSession } = await import("./ipc");
    await newSession("ws");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("new_session", { workspace: "ws" });
  });
});

describe("listSessions", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with list_sessions and workspace, returns summaries", async () => {
    const fake: import("./ipc").SessionSummary[] = [
      { sessionId: "s1", createdAt: 1, updatedAt: 2, title: "hi", turnCount: 3 },
    ];
    mockInvokeFor(fake);
    const { listSessions } = await import("./ipc");
    const result = await listSessions("ws");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("list_sessions", { workspace: "ws" });
    expect(result).toEqual(fake);
  });
});

describe("loadSession", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with load_session, workspace and sessionId", async () => {
    const fake: import("./ipc").SessionRecord[] = [
      { kind: "meta", sessionId: "s1" },
    ];
    mockInvokeFor(fake);
    const { loadSession } = await import("./ipc");
    const result = await loadSession("ws", "s1");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("load_session", { workspace: "ws", sessionId: "s1" });
    expect(result).toEqual(fake);
  });
});

// ─────────────────────────────────────────────────────────────
// Session mode
// ─────────────────────────────────────────────────────────────

describe("setSessionMode", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with set_session_mode, workspace and mode, returns SessionStarted", async () => {
    const fake: import("./ipc").SessionStarted = { sessionId: "s1" };
    mockInvokeFor(fake);
    const { setSessionMode } = await import("./ipc");
    const result = await setSessionMode("ws", "brain");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_session_mode", {
      workspace: "ws",
      mode: "brain",
    });
    expect(result).toEqual(fake);
  });
});

describe("getSessionMode", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with get_session_mode and workspace, returns mode + origin", async () => {
    const fake = { mode: "builder" as const, origin: "human" as const };
    mockInvokeFor(fake);
    const { getSessionMode } = await import("./ipc");
    const result = await getSessionMode("ws");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("get_session_mode", { workspace: "ws" });
    expect(result).toEqual(fake);
  });
});

// ─────────────────────────────────────────────────────────────
// sendMessage — uses Channel
// ─────────────────────────────────────────────────────────────

describe("sendMessage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with send_message, workspace, message, attachments, mode and eventChannel", async () => {
    const fake: import("./ipc").SessionStarted = { sessionId: "s1" };
    mockInvokeFor(fake);
    const onEvent = vi.fn();
    const { sendMessage } = await import("./ipc");
    const result = await sendMessage("ws", "hello", [], onEvent, "builder");
    expect(vi.mocked(invoke)).toHaveBeenCalledTimes(1);
    const callArgs = vi.mocked(invoke).mock.calls[0];
    expect(callArgs[0]).toBe("send_message");
    expect(callArgs[1]).toMatchObject({
      workspace: "ws",
      message: "hello",
      attachments: undefined,
      mode: "builder",
    });
    // Verify the channel is passed (it's a Channel instance)
    expect(callArgs[1]).toHaveProperty("eventChannel");
    expect(result).toEqual(fake);
  });

  it("passes attachments array when non-empty", async () => {
    mockInvokeFor({ sessionId: "s2" } as import("./ipc").SessionStarted);
    const { sendMessage } = await import("./ipc");
    await sendMessage("ws", "msg", [{ path: "/a.txt" }], vi.fn());
    const args = vi.mocked(invoke).mock.calls[0][1] as Record<string, unknown>;
    expect(args.attachments).toEqual([{ path: "/a.txt" }]);
  });

  it("omits mode when not provided", async () => {
    mockInvokeFor({ sessionId: "s3" } as import("./ipc").SessionStarted);
    const { sendMessage } = await import("./ipc");
    await sendMessage("ws", "msg", [], vi.fn());
    const args = vi.mocked(invoke).mock.calls[0][1] as Record<string, unknown>;
    expect(args.mode).toBeUndefined();
  });
});

// ─────────────────────────────────────────────────────────────
// compactSession — uses Channel
// ─────────────────────────────────────────────────────────────

describe("compactSession", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with compact_session, workspace, sessionId and eventChannel", async () => {
    mockInvokeFor("compacted content");
    const { compactSession } = await import("./ipc");
    const onEvent = vi.fn();
    const result = await compactSession("ws", "s1", onEvent);
    expect(vi.mocked(invoke)).toHaveBeenCalledTimes(1);
    const callArgs = vi.mocked(invoke).mock.calls[0];
    expect(callArgs[0]).toBe("compact_session");
    expect(callArgs[1]).toMatchObject({
      workspace: "ws",
      sessionId: "s1",
    });
    expect(callArgs[1]).toHaveProperty("eventChannel");
    expect(result).toBe("compacted content");
  });
});

// ─────────────────────────────────────────────────────────────
// Tool approval / rejection / answers / steering / interrupt
// ─────────────────────────────────────────────────────────────

describe("approveTool", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with approve_tool wrapping args", async () => {
    mockInvokeFor(undefined);
    const { approveTool } = await import("./ipc");
    await approveTool("s1", "t1");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("approve_tool", {
      args: { sessionId: "s1", toolId: "t1" },
    });
  });
});

describe("rejectTool", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with reject_tool wrapping args", async () => {
    mockInvokeFor(undefined);
    const { rejectTool } = await import("./ipc");
    await rejectTool("s1", "t1");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("reject_tool", {
      args: { sessionId: "s1", toolId: "t1" },
    });
  });
});

describe("submitAnswers", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with submit_answers wrapping args including answers", async () => {
    mockInvokeFor(undefined);
    const { submitAnswers } = await import("./ipc");
    const answers: import("./ipc").UserAnswer[] = [
      { question: "q1", answer: "a1" },
    ];
    await submitAnswers("s1", "t1", answers);
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("submit_answers", {
      args: { sessionId: "s1", toolId: "t1", answers },
    });
  });
});

describe("queueSteering", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with queue_steering, sessionId and text", async () => {
    mockInvokeFor(undefined);
    const { queueSteering } = await import("./ipc");
    await queueSteering("s1", "steer text");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("queue_steering", {
      sessionId: "s1",
      text: "steer text",
      attachments: null,
    });
  });

  it("calls invoke with queue_steering and attachments", async () => {
    mockInvokeFor(undefined);
    const { queueSteering } = await import("./ipc");
    await queueSteering("s1", "steer text", [{ path: "/tmp/photo.png" }]);
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("queue_steering", {
      sessionId: "s1",
      text: "steer text",
      attachments: [{ path: "/tmp/photo.png" }],
    });
  });
});

describe("interruptSession", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with interrupt_session and sessionId", async () => {
    mockInvokeFor(undefined);
    const { interruptSession } = await import("./ipc");
    await interruptSession("s1");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("interrupt_session", {
      sessionId: "s1",
    });
  });
});

// ─────────────────────────────────────────────────────────────
// Config
// ─────────────────────────────────────────────────────────────

describe("setConfig", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with set_config wrapping args", async () => {
    mockInvokeFor(undefined);
    const { setConfig } = await import("./ipc");
    const cfg: import("./ipc").SetConfigArgs = {
      baseUrl: "http://localhost",
      yoloMode: true,
    };
    await setConfig(cfg);
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_config", { args: cfg });
  });

  it("passes null values for optional fields", async () => {
    mockInvokeFor(undefined);
    const { setConfig } = await import("./ipc");
    await setConfig({ maxRounds: null, subMaxRounds: null });
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_config", {
      args: { maxRounds: null, subMaxRounds: null },
    });
  });
});

describe("getConfig", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with get_config and returns AgentConfig", async () => {
    const fake: import("./ipc").AgentConfig = {
      baseUrl: "http://localhost",
      brainModel: "gpt-4",
      builderModel: "gpt-3.5",
      hasApiKey: true,
      maxContextTokens: 100000,
      compactThreshold: 1000,
    };
    mockInvokeFor(fake);
    const { getConfig } = await import("./ipc");
    const result = await getConfig();
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("get_config", { workspace: null });
    expect(result).toEqual(fake);
  });
});

// ─────────────────────────────────────────────────────────────
// validateApiKey
// ─────────────────────────────────────────────────────────────

describe("validateApiKey", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with validate_api_key and apiKey, returns model list", async () => {
    const fake = ["gpt-4", "gpt-3.5"];
    mockInvokeFor(fake);
    const { validateApiKey } = await import("./ipc");
    const result = await validateApiKey("sk-abc123");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("validate_api_key", { apiKey: "sk-abc123" });
    expect(result).toEqual(fake);
  });

  it("throws when invoke rejects", async () => {
    vi.mocked(invoke).mockRejectedValueOnce(new Error("invalid key"));
    const { validateApiKey } = await import("./ipc");
    await expect(validateApiKey("sk-bad")).rejects.toThrow("invalid key");
  });
});

// ─────────────────────────────────────────────────────────────
// setWorkspaceConfig
// ─────────────────────────────────────────────────────────────

describe("setWorkspaceConfig", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with set_workspace_config, workspaceRoot and planSavePath", async () => {
    mockInvokeFor(undefined);
    const { setWorkspaceConfig } = await import("./ipc");
    await setWorkspaceConfig("/path/to/workspace", "/path/to/plans");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_workspace_config", {
      workspaceRoot: "/path/to/workspace",
      planSavePath: "/path/to/plans",
    });
  });

  it("calls invoke with null planSavePath", async () => {
    mockInvokeFor(undefined);
    const { setWorkspaceConfig } = await import("./ipc");
    await setWorkspaceConfig("/workspace", null);
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_workspace_config", {
      workspaceRoot: "/workspace",
      planSavePath: null,
    });
  });
});

// ─────────────────────────────────────────────────────────────
// Models
// ─────────────────────────────────────────────────────────────

describe("listModels", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with list_models and returns string[]", async () => {
    const fake = ["gpt-4", "gpt-3.5"];
    mockInvokeFor(fake);
    const { listModels } = await import("./ipc");
    const result = await listModels();
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("list_models");
    expect(result).toEqual(fake);
  });
});

// ─────────────────────────────────────────────────────────────
// Auth (loginWithClaudinio / logoutClaudinio)
// ─────────────────────────────────────────────────────────────

describe("loginWithClaudinio", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with login_with_claudinio and returns LoginResult", async () => {
    const fake: import("./ipc").LoginResult = { login: "user@example.com", tier: "pro" };
    mockInvokeFor(fake);
    const { loginWithClaudinio } = await import("./ipc");
    const result = await loginWithClaudinio();
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("login_with_claudinio");
    expect(result).toEqual(fake);
  });

  it("returns LoginResult with null tier when absent", async () => {
    mockInvokeFor({ login: "user@example.com", tier: null });
    const { loginWithClaudinio } = await import("./ipc");
    const result = await loginWithClaudinio();
    expect(result.tier).toBeNull();
  });
});

describe("logoutClaudinio", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with logout_claudinio", async () => {
    mockInvokeFor(undefined);
    const { logoutClaudinio } = await import("./ipc");
    await logoutClaudinio();
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("logout_claudinio");
  });
});

// ─────────────────────────────────────────────────────────────
// Workspace
// ─────────────────────────────────────────────────────────────

describe("openWorkspace", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with open_workspace, path and progressChannel, returns IndexStatus", async () => {
    const fake: import("./ipc").IndexStatus = { status: "ready", filesCount: 10, symbolsCount: 100 };
    mockInvokeFor(fake);
    const onProgress = vi.fn();
    const { openWorkspace } = await import("./ipc");
    const result = await openWorkspace("/ws", onProgress);
    expect(vi.mocked(invoke)).toHaveBeenCalledTimes(1);
    const callArgs = vi.mocked(invoke).mock.calls[0];
    expect(callArgs[0]).toBe("open_workspace");
    expect(callArgs[1]).toMatchObject({ path: "/ws" });
    expect(callArgs[1]).toHaveProperty("progressChannel");
    expect(result).toEqual(fake);
  });

  it("defaults onProgress to a noop when omitted", async () => {
    mockInvokeFor({ status: "indexing", filesCount: 0, symbolsCount: 0 });
    const { openWorkspace } = await import("./ipc");
    // Should not throw when no callback provided
    await expect(openWorkspace("/ws")).resolves.not.toThrow();
  });
});

describe("closeWorkspace", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with close_workspace and path", async () => {
    mockInvokeFor(undefined);
    const { closeWorkspace } = await import("./ipc");
    await closeWorkspace("/ws");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("close_workspace", { path: "/ws" });
  });
});

// ─────────────────────────────────────────────────────────────
// Symbol search / outline
// ─────────────────────────────────────────────────────────────

describe("searchSymbols", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with search_symbols, workspace, query and limit", async () => {
    const fake: import("./ipc").SearchResult[] = [
      { symbolId: 1, name: "foo", kind: "fn", filePath: "/a.ts", startLine: 1 },
    ];
    mockInvokeFor(fake);
    const { searchSymbols } = await import("./ipc");
    const result = await searchSymbols("ws", "foo", 10);
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("search_symbols", {
      workspace: "ws",
      query: "foo",
      limit: 10,
    });
    expect(result).toEqual(fake);
  });

  it("omits limit when not provided", async () => {
    mockInvokeFor([]);
    const { searchSymbols } = await import("./ipc");
    await searchSymbols("ws", "bar");
    const args = vi.mocked(invoke).mock.calls[0][1] as Record<string, unknown>;
    expect(args.limit).toBeUndefined();
  });
});

describe("symbolLookup", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with symbol_lookup, workspace and name", async () => {
    const fake: import("./ipc").SearchResult[] = [
      { symbolId: 1, name: "myFunc", kind: "fn", filePath: "/a.ts", startLine: 5 },
    ];
    mockInvokeFor(fake);
    const { symbolLookup } = await import("./ipc");
    const result = await symbolLookup("ws", "myFunc");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("symbol_lookup", {
      workspace: "ws",
      name: "myFunc",
    });
    expect(result).toEqual(fake);
  });
});

describe("fileOutline", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with file_outline, workspace and filePath", async () => {
    const fake: import("./ipc").SymbolRecord[] = [
      { id: 1, fileId: 1, name: "func", kind: "fn", startLine: 1, startCol: 0, endLine: 5, endCol: 0 },
    ];
    mockInvokeFor(fake);
    const { fileOutline } = await import("./ipc");
    const result = await fileOutline("ws", "/path/file.ts");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("file_outline", {
      workspace: "ws",
      filePath: "/path/file.ts",
    });
    expect(result).toEqual(fake);
  });
});

// ─────────────────────────────────────────────────────────────
// File write / walk
// ─────────────────────────────────────────────────────────────

describe("writeFile", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with write_file, path and content", async () => {
    mockInvokeFor(undefined);
    const { writeFile } = await import("./ipc");
    await writeFile("/path/file.ts", "content here");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("write_file", {
      path: "/path/file.ts",
      content: "content here",
    });
  });
});

describe("walkDirectory", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with walk_dir and root, returns WalkEntry[]", async () => {
    const fake: import("./ipc").WalkEntry[] = [
      { path: "/root/a", isDir: false },
      { path: "/root/sub", isDir: true },
    ];
    mockInvokeFor(fake);
    const { walkDirectory } = await import("./ipc");
    const result = await walkDirectory("/root");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("walk_dir", { root: "/root" });
    expect(result).toEqual(fake);
  });
});

// ─────────────────────────────────────────────────────────────
// Tasks
// ─────────────────────────────────────────────────────────────

describe("getTasks", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with get_tasks and workspace, returns TaskItem[]", async () => {
    const fake: import("./ipc").TaskItem[] = [
      { id: "1", title: "t1", description: "d1", journal: [], status: "todo" },
    ];
    mockInvokeFor(fake);
    const { getTasks } = await import("./ipc");
    const result = await getTasks("ws");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("get_tasks", { workspace: "ws" });
    expect(result).toEqual(fake);
  });
});

describe("setTasks", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with set_tasks, workspace and tasks", async () => {
    mockInvokeFor(undefined);
    const { setTasks } = await import("./ipc");
    const tasks: import("./ipc").TaskItem[] = [
      { id: "1", title: "t1", description: "d1", journal: ["note"], status: "done" },
    ];
    await setTasks("ws", tasks);
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_tasks", {
      workspace: "ws",
      tasks,
    });
  });
});

// ─────────────────────────────────────────────────────────────
// Skills
// ─────────────────────────────────────────────────────────────

describe("listSkills", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with list_skills and workspace, returns SkillsResponse", async () => {
    const fake: import("./ipc").SkillsResponse = {
      skills: [{ name: "test", description: "a skill", location: "/a", scope: "builtin" }],
      count: 1,
    };
    mockInvokeFor(fake);
    const { listSkills } = await import("./ipc");
    const result = await listSkills("ws");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("list_skills", { workspace: "ws" });
    expect(result).toEqual(fake);
  });
});

describe("getSkillCatalog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with get_skill_catalog and workspace, returns string[]", async () => {
    const fake = ["skill-a", "skill-b"];
    mockInvokeFor(fake);
    const { getSkillCatalog } = await import("./ipc");
    const result = await getSkillCatalog("ws");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("get_skill_catalog", { workspace: "ws" });
    expect(result).toEqual(fake);
  });
});

describe("getSkillContent", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with get_skill_content, workspace and name", async () => {
    const fake = { name: "test", description: "d", location: "/a", scope: "builtin" as const, body: "# Skill" };
    mockInvokeFor(fake);
    const { getSkillContent } = await import("./ipc");
    const result = await getSkillContent("ws", "test");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("get_skill_content", {
      workspace: "ws",
      name: "test",
    });
    expect(result).toEqual(fake);
  });
});

describe("rescanSkills", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with rescan_skills and workspace, returns SkillsResponse", async () => {
    const fake: import("./ipc").SkillsResponse = { skills: [], count: 0 };
    mockInvokeFor(fake);
    const { rescanSkills } = await import("./ipc");
    const result = await rescanSkills("ws");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("rescan_skills", { workspace: "ws" });
    expect(result).toEqual(fake);
  });
});

describe("findRemoteSkills", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with find_remote_skills and query", async () => {
    mockInvokeFor([]);
    const { findRemoteSkills } = await import("./ipc");
    await findRemoteSkills("test");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("find_remote_skills", { query: "test" });
  });

  it("passes null query when omitted", async () => {
    mockInvokeFor([]);
    const { findRemoteSkills } = await import("./ipc");
    await findRemoteSkills();
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("find_remote_skills", { query: null });
  });
});

describe("previewRemoteSkill", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with preview_remote_skill and url", async () => {
    const fake: import("./ipc").SkillEntry = {
      name: "test", description: "d", location: "/a", scope: "builtin",
    };
    mockInvokeFor(fake);
    const { previewRemoteSkill } = await import("./ipc");
    const result = await previewRemoteSkill("https://example.com/skill");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("preview_remote_skill", {
      url: "https://example.com/skill",
    });
    expect(result).toEqual(fake);
  });
});

describe("installRemoteSkill", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with install_remote_skill, workspace and args", async () => {
    const fake: import("./ipc").SkillEntry = {
      name: "test", description: "d", location: "/a", scope: "builtin",
    };
    mockInvokeFor(fake);
    const { installRemoteSkill } = await import("./ipc");
    const args: import("./ipc").InstallRemoteSkillArgs = {
      name: "test",
      url: "https://example.com/skill",
      description: "d",
    };
    const result = await installRemoteSkill("ws", args);
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("install_remote_skill", {
      workspace: "ws",
      args,
    });
    expect(result).toEqual(fake);
  });
});

// ─────────────────────────────────────────────────────────────
// Context warning
// ─────────────────────────────────────────────────────────────

describe("getContextWarning", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with get_context_warning and workspace", async () => {
    const fake: import("./ipc").ContextWarningData = {
      agentsMdSize: 0,
      agentsMdLines: 0,
      agentsMdTokens: 0,
      agentsMdIssues: 0,
      agentsMdPath: null,
      skillsCount: 0,
      skillsTotalTokens: 0,
      skillsBreakdown: [],
    };
    mockInvokeFor(fake);
    const { getContextWarning } = await import("./ipc");
    const result = await getContextWarning("ws");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("get_context_warning", { workspace: "ws" });
    expect(result).toEqual(fake);
  });
});

// ─────────────────────────────────────────────────────────────
// LSP
// ─────────────────────────────────────────────────────────────

function makeLspArgs(overrides?: Partial<import("./ipc").LspPositionArgs>): import("./ipc").LspPositionArgs {
  return { filePath: "/a.ts", line: 1, character: 0, ...overrides };
}

describe("lspDefinition", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with lsp_definition, workspace and args", async () => {
    const fake: import("./ipc").LspLocation[] = [
      { uri: "file:///a.ts", startLine: 1, startChar: 0, endLine: 5, endChar: 0 },
    ];
    mockInvokeFor(fake);
    const { lspDefinition } = await import("./ipc");
    const args = makeLspArgs();
    const result = await lspDefinition("ws", args);
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("lsp_definition", { workspace: "ws", args });
    expect(result).toEqual(fake);
  });
});

describe("lspReferences", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with lsp_references, workspace and args", async () => {
    const fake: import("./ipc").LspLocation[] = [
      { uri: "file:///a.ts", startLine: 10, startChar: 2, endLine: 10, endChar: 5 },
    ];
    mockInvokeFor(fake);
    const { lspReferences } = await import("./ipc");
    const args = makeLspArgs();
    const result = await lspReferences("ws", args);
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("lsp_references", { workspace: "ws", args });
    expect(result).toEqual(fake);
  });
});

describe("lspHover", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls invoke with lsp_hover, workspace and args, returns HoverInfo", async () => {
    const fake: import("./ipc").HoverInfo = { contents: "**doc**" };
    mockInvokeFor(fake);
    const { lspHover } = await import("./ipc");
    const args = makeLspArgs();
    const result = await lspHover("ws", args);
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("lsp_hover", { workspace: "ws", args });
    expect(result).toEqual(fake);
  });

  it("returns null when invoke returns null", async () => {
    mockInvokeFor(null);
    const { lspHover } = await import("./ipc");
    const result = await lspHover("ws", makeLspArgs());
    expect(result).toBeNull();
  });
});

// ─────────────────────────────────────────────────────────────
// pickFolder / pickFiles — use @tauri-apps/plugin-dialog::open
// ─────────────────────────────────────────────────────────────

describe("pickFolder", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls open with directory:true, multiple:false and returns the path string", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValue("/selected/folder");
    const { pickFolder } = await import("./ipc");
    const result = await pickFolder();
    expect(vi.mocked(open)).toHaveBeenCalledWith({ directory: true, multiple: false });
    expect(result).toBe("/selected/folder");
  });

  it("returns null when open returns null", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValue(null);
    const { pickFolder } = await import("./ipc");
    const result = await pickFolder();
    expect(result).toBeNull();
  });

  it("passes defaultPath to open when provided", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValue("/workspace/sub");
    const { pickFolder } = await import("./ipc");
    const result = await pickFolder("/workspace");
    expect(vi.mocked(open)).toHaveBeenCalledWith({ directory: true, multiple: false, defaultPath: "/workspace" });
    expect(result).toBe("/workspace/sub");
  });
});

describe("pickFiles", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls open with multiple:true and returns paths", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValue(["/a.txt", "/b.txt"]);
    const { pickFiles } = await import("./ipc");
    const result = await pickFiles();
    expect(vi.mocked(open)).toHaveBeenCalledWith({ multiple: true });
    expect(result).toEqual(["/a.txt", "/b.txt"]);
  });

  it("returns a single path wrapped in array when open returns a string", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValue("/single/file.ts");
    const { pickFiles } = await import("./ipc");
    const result = await pickFiles();
    expect(result).toEqual(["/single/file.ts"]);
  });

  it("returns empty array when open returns null", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValue(null);
    const { pickFiles } = await import("./ipc");
    const result = await pickFiles();
    expect(result).toEqual([]);
  });
});
