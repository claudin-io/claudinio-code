import { describe, it, expect } from "vitest";
import {
  workspaceStatus,
  setWorkspaceStatus,
  clearWorkspaceStatus,
  isBusy,
} from "./workspaceStatus";
import type { WsStatus } from "./workspaceStatus";

// ── workspaceStatus store ──────────────────────────────────────────

describe("workspaceStatus", () => {
  it("starts as an empty object", () => {
    expect(Object.keys(workspaceStatus)).toHaveLength(0);
  });
});

// ── setWorkspaceStatus ─────────────────────────────────────────────

describe("setWorkspaceStatus", () => {
  it("sets a workspace to a given status", () => {
    setWorkspaceStatus("ws:test-1", "thinking");
    expect(workspaceStatus["ws:test-1"]).toBe("thinking");
  });

  it("overwrites an existing status", () => {
    setWorkspaceStatus("ws:test-overwrite", "idle");
    expect(workspaceStatus["ws:test-overwrite"]).toBe("idle");

    setWorkspaceStatus("ws:test-overwrite", "error");
    expect(workspaceStatus["ws:test-overwrite"]).toBe("error");
  });

  it("handles multiple workspaces independently", () => {
    setWorkspaceStatus("ws:multi-a", "done");
    setWorkspaceStatus("ws:multi-b", "awaiting_approval");

    expect(workspaceStatus["ws:multi-a"]).toBe("done");
    expect(workspaceStatus["ws:multi-b"]).toBe("awaiting_approval");
  });

  it("accepts all valid WsStatus values", () => {
    const statuses: WsStatus[] = [
      "idle",
      "thinking",
      "awaiting_approval",
      "awaiting_input",
      "done",
      "error",
    ];

    statuses.forEach((s, i) => {
      const key = `ws:all-statuses-${i}`;
      setWorkspaceStatus(key, s);
      expect(workspaceStatus[key]).toBe(s);
    });
  });
});

// ── clearWorkspaceStatus ───────────────────────────────────────────

describe("clearWorkspaceStatus", () => {
  it("sets the workspace status to undefined", () => {
    setWorkspaceStatus("ws:to-clear", "thinking");
    expect(workspaceStatus["ws:to-clear"]).toBe("thinking");

    clearWorkspaceStatus("ws:to-clear");
    expect(workspaceStatus["ws:to-clear"]).toBeUndefined();
  });

  it("is a no-op when the workspace does not exist", () => {
    // Should not throw
    expect(() => clearWorkspaceStatus("ws:nonexistent")).not.toThrow();
    expect(workspaceStatus["ws:nonexistent"]).toBeUndefined();
  });
});

// ── isBusy ─────────────────────────────────────────────────────────

describe("isBusy", () => {
  it("returns true for 'thinking'", () => {
    expect(isBusy("thinking")).toBe(true);
  });

  it("returns true for 'awaiting_approval'", () => {
    expect(isBusy("awaiting_approval")).toBe(true);
  });

  it("returns true for 'awaiting_input'", () => {
    expect(isBusy("awaiting_input")).toBe(true);
  });

  it("returns false for 'idle'", () => {
    expect(isBusy("idle")).toBe(false);
  });

  it("returns false for 'done'", () => {
    expect(isBusy("done")).toBe(false);
  });

  it("returns false for 'error'", () => {
    expect(isBusy("error")).toBe(false);
  });

  it("returns false for undefined", () => {
    expect(isBusy(undefined)).toBe(false);
  });

  it("distinguishes busy from non-busy statuses exhaustively", () => {
    const busy: WsStatus[] = ["thinking", "awaiting_approval", "awaiting_input"];
    const notBusy: WsStatus[] = ["idle", "done", "error"];

    busy.forEach((s) => expect(isBusy(s)).toBe(true));
    notBusy.forEach((s) => expect(isBusy(s)).toBe(false));
  });
});
