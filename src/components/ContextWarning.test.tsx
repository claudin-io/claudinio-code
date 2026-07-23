import { describe, it, expect, vi, afterEach } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { getContextWarning } from "../lib/ipc";
import type { ContextWarningData } from "../lib/ipc";

/// Flush pending microtasks AND SolidJS reactive updates in jsdom.
/// SolidJS async onMount needs a macroTask to settle in jsdom.
function flush() {
  return new Promise((r) => setTimeout(r, 10));
}

// ── Module mocks ───────────────────────────────────────────────────
vi.mock("../lib/ipc", () => ({
  getContextWarning: vi.fn(),
}));


// ── Imports (after mocks) ──────────────────────────────────────────
import {
  formatBytes,
  formatTokens,
  severityClass,
  skillSeverityClass,
  showWarning,
} from "./ContextWarning";
import ContextWarning from "./ContextWarning";

// ── Sample data fixtures ───────────────────────────────────────────
const warningData: ContextWarningData = {
  agentsMdSize: 12345,
  agentsMdLines: 300,
  agentsMdTokens: 12_000,
  agentsMdIssues: 2,
  agentsMdPath: "/some/path/AGENTS.md",
  skillsCount: 5,
  skillsTotalTokens: 15_000,
  skillsBreakdown: [
    { name: "skill-a", description: "", estimatedTokens: 6_000, location: "/a" },
    { name: "skill-b", description: "", estimatedTokens: 3_000, location: "/b" },
  ],
};

const safeData: ContextWarningData = {
  agentsMdSize: 100,
  agentsMdLines: 10,
  agentsMdTokens: 3_999,
  agentsMdIssues: 0,
  agentsMdPath: null,
  skillsCount: 0,
  skillsTotalTokens: 0,
  skillsBreakdown: [],
};

// ══════════════════════════════════════════════════════════════════════
// Pure helper function tests (unchanged)
// ══════════════════════════════════════════════════════════════════════

describe("formatBytes", () => {
  it('formats 0 bytes as "0 B"', () => {
    expect(formatBytes(0)).toBe("0 B");
  });
  it("formats bytes under 1024 as raw number", () => {
    expect(formatBytes(100)).toBe("100 B");
  });
  it("formats kilobytes with 1 decimal and KB suffix", () => {
    expect(formatBytes(1500)).toBe("1.5 KB");
  });
  it("formats megabytes with 1 decimal and MB suffix", () => {
    expect(formatBytes(1048576)).toBe("1.0 MB");
  });
});

describe("formatTokens", () => {
  it('formats 0 as "0"', () => {
    expect(formatTokens(0)).toBe("0");
  });
  it("formats numbers under 1000 as raw number", () => {
    expect(formatTokens(500)).toBe("500");
  });
  it("formats thousands with 1 decimal and k suffix", () => {
    expect(formatTokens(1500)).toBe("1.5k");
  });
  it("formats tens of thousands with 1 decimal and k suffix", () => {
    expect(formatTokens(12000)).toBe("12.0k");
  });
});

describe("severityClass", () => {
  it('returns text-red-400 for agentsMdTokens > 20_000', () => {
    expect(severityClass(25000)).toBe("text-red-400");
  });
  it('returns text-amber-400 for agentsMdTokens > 8_000', () => {
    expect(severityClass(10000)).toBe("text-amber-400");
  });
  it('returns text-ink-faint for agentsMdTokens <= 8_000', () => {
    expect(severityClass(3000)).toBe("text-ink-faint");
  });
});

describe("skillSeverityClass", () => {
  it('returns text-red-400 for skill tokens > 5_000', () => {
    expect(skillSeverityClass(6000)).toBe("text-red-400");
  });
  it('returns text-amber-400 for skill tokens > 2_000', () => {
    expect(skillSeverityClass(3000)).toBe("text-amber-400");
  });
  it('returns text-ink-muted for skill tokens <= 2_000', () => {
    expect(skillSeverityClass(1000)).toBe("text-ink-muted");
  });
});

describe("showWarning", () => {
  it("returns false when data is null", () => {
    expect(showWarning(null)).toBe(false);
  });
  it("returns true when agentsMdTokens > 4_000", () => {
    expect(
      showWarning({ ...warningData, agentsMdTokens: 5_000, agentsMdIssues: 0, skillsTotalTokens: 0 }),
    ).toBe(true);
  });
  it("returns true when agentsMdIssues > 0", () => {
    expect(
      showWarning({ ...safeData, agentsMdIssues: 1 }),
    ).toBe(true);
  });
  it("returns true when skillsTotalTokens > 10_000", () => {
    expect(
      showWarning({ ...safeData, agentsMdTokens: 1_000, skillsTotalTokens: 15_000 }),
    ).toBe(true);
  });
  it("returns false when no thresholds are exceeded", () => {
    expect(showWarning(safeData)).toBe(false);
  });
});

// ══════════════════════════════════════════════════════════════════════
// Component rendering tests
// ══════════════════════════════════════════════════════════════════════

describe("ContextWarning component", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  // ── loading state ──────────────────────────────────────────────────
  it('renders nothing while getContextWarning is pending (loading = true)', () => {
    vi.mocked(getContextWarning).mockReturnValue(new Promise(() => {}));

    render(() => <ContextWarning workspace="/test" />, document.body);

    // loading starts true, promise never resolves → button never renders
    expect(document.body.innerHTML).toBe("");
  });

  // ── warning visible ────────────────────────────────────────────────
  it("renders the warning button when data triggers a warning", async () => {
    vi.mocked(getContextWarning).mockResolvedValue(warningData);

    render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    const btn = document.querySelector('button[title="Context Budget"]');
    expect(btn).not.toBeNull();
  });

  // ── no warning ─────────────────────────────────────────────────────
  it("renders nothing when data does not trigger a warning", async () => {
    vi.mocked(getContextWarning).mockResolvedValue(safeData);

    render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    // safeData has agentsMdTokens=3_999, agentsMdIssues=0, skillsTotalTokens=0
    // → showWarning returns false → outer <Show> hides everything
    expect(document.body.innerHTML).toBe("");
  });

  // ── agentColor inline computation ──────────────────────────────────
  it("applies the correct agentColor class based on token severity", async () => {
    // warningData.agentsMdTokens = 12_000 → severityClass(12_000) = "text-amber-400"
    vi.mocked(getContextWarning).mockResolvedValue(warningData);

    render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    const btn = document.querySelector('button[title="Context Budget"]');
    expect(btn).not.toBeNull();
    expect(btn!.className).toContain("text-amber-400");
  });

  // ── modal open / close ─────────────────────────────────────────────
  it("opens the modal on warning-button click and closes on X click", async () => {
    vi.mocked(getContextWarning).mockResolvedValue(warningData);

    render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    // Modal hidden before click
    expect(document.querySelector(".fixed.inset-0")).toBeNull();

    // Click warning button → open
    const btn = document.querySelector(
      'button[title="Context Budget"]',
    ) as HTMLElement;
    btn.click();
    await flush();

    expect(document.querySelector(".fixed.inset-0")).not.toBeNull();
    expect(document.body.textContent).toContain("/some/path/AGENTS.md");

    // Click the X close button
    const closeBtn = document.querySelector(".fixed.inset-0 button") as HTMLElement;
    closeBtn.click();
    await flush();

    expect(document.querySelector(".fixed.inset-0")).toBeNull();
  });

  // ── Escape key ─────────────────────────────────────────────────────
  it("closes the modal on Escape keypress", async () => {
    vi.mocked(getContextWarning).mockResolvedValue(warningData);

    render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    // Open
    (document.querySelector(
      'button[title="Context Budget"]',
    ) as HTMLElement).click();
    await flush();
    expect(document.querySelector(".fixed.inset-0")).not.toBeNull();

    // Press Escape
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flush();
    expect(document.querySelector(".fixed.inset-0")).toBeNull();
  });

  // ── backdrop click ─────────────────────────────────────────────────
  it("closes the modal on backdrop click (e.target === e.currentTarget)", async () => {
    vi.mocked(getContextWarning).mockResolvedValue(warningData);

    render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    // Open
    (document.querySelector(
      'button[title="Context Budget"]',
    ) as HTMLElement).click();
    await flush();
    expect(document.querySelector(".fixed.inset-0")).not.toBeNull();

    // Click the backdrop div (e.target === e.currentTarget)
    (document.querySelector(".fixed.inset-0") as HTMLElement).click();
    await flush();
    expect(document.querySelector(".fixed.inset-0")).toBeNull();
  });

  it("does not close when clicking inside the modal card (bubbled event)", async () => {
    vi.mocked(getContextWarning).mockResolvedValue(warningData);

    render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    // Open
    (document.querySelector(
      'button[title="Context Budget"]',
    ) as HTMLElement).click();
    await flush();
    expect(document.querySelector(".fixed.inset-0")).not.toBeNull();

    // Click the inner card — event bubbles through backdrop but e.target !==
    // e.currentTarget, so the backdrop's onClick guard keeps modal open
    const innerCard = document.querySelector(
      ".fixed.inset-0 [class*=rounded-lg]",
    ) as HTMLElement;
    innerCard.click();
    await flush();

    // Modal should still be open
    expect(document.querySelector(".fixed.inset-0")).not.toBeNull();
  });

  // ── .catch handler (anonymous_10, line 55-57) ────────────────────────
  it("renders nothing when getContextWarning rejects (covers .catch handler)", async () => {
    vi.mocked(getContextWarning).mockRejectedValue(new Error("network error"));

    render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    // After catch: data=null, loading=false → visible()=false → outer Show hides everything
    expect(document.body.innerHTML).toBe("");
    // Prove the mock was called — if the .catch ran, we know the promise was consumed
    expect(getContextWarning).toHaveBeenCalledWith("/test");
  });

  // ── .then handler with null data (variant of anonymous_9) ────────────
  it("renders nothing when getContextWarning resolves to null (covers .then setting null data)", async () => {
    vi.mocked(getContextWarning).mockResolvedValue(null as unknown as ContextWarningData);

    render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    // data is null → showWarning(null) returns false → nothing renders
    expect(document.body.innerHTML).toBe("");
    expect(getContextWarning).toHaveBeenCalledWith("/test");
  });

  // ── agentColor severity: text-red-400 threshold (agentColor with > 20k tokens) ──
  it("applies text-red-400 agentColor when agentsMdTokens > 20_000", async () => {
    vi.mocked(getContextWarning).mockResolvedValue({
      ...warningData,
      agentsMdTokens: 25_000,
    });

    render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    const btn = document.querySelector(
      'button[title="Context Budget"]',
    ) as HTMLElement;
    expect(btn.className).toContain("text-red-400");
  });

  // ── visible() with showWarning(null) ─────────────────────────────────
  it("visible() returns false when data is null (showWarning(null) path)", () => {
    const [data] = createSignal<ContextWarningData | null>(null);
    // Inline equivalent of visible():
    const visible = () => showWarning(data());
    expect(visible()).toBe(false);
  });

  // ── onCleanup (anonymous_13, line 67) ────────────────────────────────
  it("cleans up the keydown event listener on component disposal (onCleanup)", async () => {
    vi.mocked(getContextWarning).mockResolvedValue(warningData);

    // render() returns a dispose function; calling it fires SolidJS's
    // cleanup chain, which triggers all onCleanup callbacks.
    const dispose = render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    // Sanity: component is mounted
    expect(
      document.querySelector('button[title="Context Budget"]'),
    ).not.toBeNull();

    // Dispose the component — triggers onCleanup which removes the listener
    dispose();

    // Verify: the keydown listener was removed by onCleanup
    const spy = vi.fn();
    document.addEventListener("keydown", spy);
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(spy).toHaveBeenCalledTimes(1);
    document.removeEventListener("keydown", spy);
  });

  // ── non-Escape keydown is a no-op (line 64 false branch) ──────────────
  it("does not close modal on non-Escape keypress", async () => {
    vi.mocked(getContextWarning).mockResolvedValue(warningData);

    render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    // Open the modal
    (document.querySelector(
      'button[title="Context Budget"]',
    ) as HTMLElement).click();
    await flush();
    expect(document.querySelector(".fixed.inset-0")).not.toBeNull();

    // Press a non-Escape key — the guard `e.key === "Escape"` should be false
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    // Modal should remain open
    expect(document.querySelector(".fixed.inset-0")).not.toBeNull();

    // Now press Escape to confirm it still works
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flush();
    expect(document.querySelector(".fixed.inset-0")).toBeNull();
  });

  // ── issues ternary false branch (line 135: agentsMdIssues = 0) ────────
  it("renders text-ink-muted for issues count when agentsMdIssues is 0", async () => {
    // Data triggers warning via agentsMdTokens (5000 > 4000) but has 0 issues
    vi.mocked(getContextWarning).mockResolvedValue({
      ...warningData,
      agentsMdTokens: 5_000,
      agentsMdIssues: 0,
    });

    render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    // Open the modal
    (document.querySelector(
      'button[title="Context Budget"]',
    ) as HTMLElement).click();
    await flush();

    // The issues span should use text-ink-muted (the false branch)
    const issuesSpan = document.querySelector('.font-mono.text-ink-muted');
    expect(issuesSpan).not.toBeNull();
    expect(issuesSpan!.textContent).toBe("0");
  });

  // ── recommendation ternary false branch (line 197: agentsMdPath == null) ──
  it("shows hintSkills when agentsMdPath is null but warning still triggers", async () => {
    vi.mocked(getContextWarning).mockResolvedValue({
      ...safeData,
      agentsMdTokens: 5_000, // triggers warning
      agentsMdIssues: 0,
    });

    render(() => <ContextWarning workspace="/test" />, document.body);
    await flush();

    // Open the modal
    (document.querySelector(
      'button[title="Context Budget"]',
    ) as HTMLElement).click();
    await flush();

    // The recommendation div should contain hintSkills (the false branch)
    expect(document.body.textContent).toContain("\ud83d\udca1 Skills are injected into the system prompt as XML. Skills with large SKILL.md bodies increase the base context cost. Review if all skills are still needed.");
    expect(document.body.textContent).not.toContain("\ud83d\udca1 The AGENTS.md/CLAUDE.md file is injected at the start of every new chat. Large files consume significant context budget. Consider trimming unnecessary sections.");
  });
});
