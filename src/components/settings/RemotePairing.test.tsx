import { describe, it, expect, vi, afterEach } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { RemotePairing, type PairingOutcome, type PendingPairing } from "./RemotePairing";
import type { PairingCodeView } from "../../lib/ipc";

const flush = () => new Promise((r) => setTimeout(r, 10));

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
  vi.useRealTimers();
});

const code = (over: Partial<PairingCodeView> = {}): PairingCodeView => ({
  url: "https://app.claudin.io/#c=abab&k=cdcd&r=wss://relay.claudin.io/ws&e=1",
  channel: "ab".repeat(16),
  deviceKey: "cd".repeat(32),
  expiresAt: Date.now() + 120_000,
  qrSvg: '<svg xmlns="http://www.w3.org/2000/svg"><rect width="1" height="1"/></svg>',
  // Absent by default, because absent is the ordinary case: a machine that is not
  // signed in pairs by QR exactly as well.
  typedCode: null,
  typedCodeError: null,
  ...over,
});

interface Handlers {
  onStart?: (label: string) => void;
  onCancel?: () => void;
  onConfirm?: (matched: boolean) => void;
}

async function mount(
  opts: {
    code?: PairingCodeView | null;
    pending?: PendingPairing | null;
    outcome?: PairingOutcome | null;
    busy?: boolean;
    blocked?: string | null;
  } = {},
  handlers: Handlers = {},
) {
  host = document.createElement("div");
  document.body.appendChild(host);

  const [code] = createSignal(opts.code ?? null);
  const [pending] = createSignal(opts.pending ?? null);
  const [outcome] = createSignal(opts.outcome ?? null);
  const [busy] = createSignal(opts.busy ?? false);
  const [blocked] = createSignal(opts.blocked ?? null);

  dispose = render(
    () => (
      <RemotePairing
        code={code}
        pending={pending}
        outcome={outcome}
        busy={busy}
        blocked={blocked}
        onStart={handlers.onStart ?? (() => {})}
        onCancel={handlers.onCancel ?? (() => {})}
        onConfirm={handlers.onConfirm ?? (() => {})}
      />
    ),
    host,
  );
  await flush();
  return host;
}

const button = (root: HTMLElement, label: string) =>
  Array.from(root.querySelectorAll("button")).find((b) => b.textContent?.includes(label));

describe("RemotePairing", () => {
  it("asks for a name before it will show a code", async () => {
    const onStart = vi.fn();
    const root = await mount({}, { onStart });

    const show = button(root, "Show a pairing code")!;
    expect(show.disabled).toBe(true);

    const input = root.querySelector("input")!;
    input.value = "Safari on iPhone";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();

    expect(show.disabled).toBe(false);
    show.click();
    expect(onStart).toHaveBeenCalledWith("Safari on iPhone");
  });

  /// A row of hex in the pairing list is useless for deciding what to revoke.
  it("will not start with a blank name", async () => {
    const onStart = vi.fn();
    const root = await mount({}, { onStart });

    const input = root.querySelector("input")!;
    input.value = "   ";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();

    expect(onStart).not.toHaveBeenCalled();
  });

  /// A disabled button with no explanation reads as a bug.
  it("says why it cannot pair instead of just refusing to", async () => {
    const root = await mount({ blocked: "open a session first" });

    expect(root.textContent).toContain("open a session first");
    expect(button(root, "Show a pairing code")).toBeUndefined();
  });

  /// The QR goes through an <img> and a data URI, never innerHTML.
  it("renders the code as an image rather than injecting markup", async () => {
    const svg = '<svg xmlns="http://www.w3.org/2000/svg"><rect width="9" height="9"/></svg>';
    const root = await mount({ code: code({ qrSvg: svg }) });

    const img = root.querySelector("img")!;
    expect(img.getAttribute("src")).toBe(`data:image/svg+xml;base64,${btoa(svg)}`);
    // The SVG must not have become part of the document.
    expect(root.querySelector("svg")).toBeNull();
  });

  /// Browser-direct has to work: a PWA is wanted for push, never as the way in.
  it("says no app is needed, and shows the URL for a camera that will not scan", async () => {
    const root = await mount({ code: code() });

    expect(root.textContent).toContain("No app to install");
    expect(root.textContent).toContain("app.claudin.io/#c=");
  });

  it("counts the code down and then says it lapsed", async () => {
    const root = await mount({ code: code({ expiresAt: Date.now() + 5_000 }) });

    expect(root.textContent).toMatch(/Good for [45]s/);

    dispose?.();
    host?.remove();
    const lapsed = await mount({ code: code({ expiresAt: Date.now() - 1_000 }) });
    expect(lapsed.textContent).toContain("lapsed");
  });

  it("stops a pairing window on request", async () => {
    const onCancel = vi.fn();
    const root = await mount({ code: code() }, { onCancel });

    button(root, "Stop")!.click();
    expect(onCancel).toHaveBeenCalledOnce();
  });

  /// The words are the security boundary, so when they are on screen they are the
  /// only thing on screen — the QR must not still be there to look at instead.
  it("shows the words alone, not alongside the code", async () => {
    const root = await mount({
      code: code(),
      pending: { peerKey: "11".repeat(32), label: "iPhone", sas: "basalt · dahlia · fathom" },
    });

    expect(root.querySelector("[data-testid=sas]")?.textContent).toBe(
      "basalt · dahlia · fathom",
    );
    expect(root.querySelector("img")).toBeNull();
    expect(root.textContent).not.toContain("Scan this");
  });

  /// The words mean nothing to someone who does not know what a mismatch implies.
  it("explains what a mismatch would mean", async () => {
    const root = await mount({
      pending: { peerKey: "11".repeat(32), label: "iPhone", sas: "a · b · c" },
    });

    expect(root.textContent).toContain("something is relaying your pairing");
    expect(root.textContent).toContain("revoked");
  });

  it("passes on both answers to the word check", async () => {
    const onConfirm = vi.fn();
    const pending = { peerKey: "11".repeat(32), label: "iPhone", sas: "a · b · c" };

    let root = await mount({ pending }, { onConfirm });
    button(root, "The words match")!.click();
    expect(onConfirm).toHaveBeenCalledWith(true);

    dispose?.();
    host?.remove();
    root = await mount({ pending }, { onConfirm });
    button(root, "They do not match")!.click();
    expect(onConfirm).toHaveBeenCalledWith(false);
  });

  /// Confirming without looking is the dangerous mistake, so "they match" must not
  /// be the button a stray Enter or a habitual tap lands on.
  it("does not make confirming the default action", async () => {
    const root = await mount({
      pending: { peerKey: "11".repeat(32), label: "iPhone", sas: "a · b · c" },
    });

    const match = button(root, "The words match")!;
    expect(match.getAttribute("autofocus")).toBeNull();
    expect(match.getAttribute("type")).not.toBe("submit");
    expect(document.activeElement).not.toBe(match);
  });

  it("reports a pairing that went through", async () => {
    const root = await mount({ outcome: { kind: "paired", label: "Safari on iPhone" } });

    expect(root.textContent).toContain("Paired with Safari on iPhone");
  });

  /// Refusal has a consequence the user has to know about, or they will wonder why
  /// scanning again does nothing.
  it("explains that a refused key stays refused", async () => {
    const root = await mount({ outcome: { kind: "refused" } });

    expect(root.textContent).toContain("revoked");
    expect(root.textContent).toContain("until you allow it");
  });

  it("does not act while a call is in flight", async () => {
    const root = await mount({
      busy: true,
      pending: { peerKey: "11".repeat(32), label: "iPhone", sas: "a · b · c" },
    });

    expect(button(root, "The words match")!.disabled).toBe(true);
    expect(button(root, "They do not match")!.disabled).toBe(true);
  });
});

// ── the typed code ─────────────────────────────────────────────────────────

describe("the typed code", () => {
  /// The QR needs no account and is the shorter path, so it stays what the eye lands
  /// on. The typed code is the fallback for a camera that is not an option.
  it("shows the code to type, under the QR", async () => {
    const root = await mount({ code: code({ typedCode: "A1B2C-D3E4F" }) });

    const codes = [...root.querySelectorAll("code")].map((el) => el.textContent);
    expect(codes).toContain("A1B2C-D3E4F");
    expect(codes.indexOf("A1B2C-D3E4F")).toBeGreaterThan(0);
    expect(root.textContent).toContain("app.claudin.io");
  });

  /// The sign-in is the reason the code is safe to be ten characters, so the panel
  /// says so where the code is rather than leaving the browser to explain it.
  it("says why typing needs a sign-in", async () => {
    const root = await mount({ code: code({ typedCode: "A1B2C-D3E4F" }) });

    expect(root.textContent).toMatch(/only your account can look the code up/i);
  });

  /// A machine that is not signed in — or is pointed at a self-hosted setup with no
  /// account server — pairs by QR exactly as well. §1.1: claudin.io is never a hard
  /// dependency, and the panel must not report an absence as a problem.
  it("says nothing at all when there is no code and no reason", async () => {
    const root = await mount({ code: code() });

    expect(root.textContent).not.toMatch(/type this|no code to type/i);
    expect(root.querySelector("img")).toBeTruthy();
  });

  /// When there *is* a reason, it comes with the reassurance that the QR is unaffected
  /// — otherwise a failed round trip to an account server reads as pairing being
  /// broken.
  it("keeps the QR when the account server refused", async () => {
    const root = await mount({ code: code({ typedCodeError: "rate limit exceeded" }) });

    expect(root.textContent).toContain("rate limit exceeded");
    expect(root.textContent).toMatch(/scanning still works/i);
    expect(root.querySelector("img")).toBeTruthy();
  });
});
