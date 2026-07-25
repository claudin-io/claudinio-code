import { describe, it, expect, vi, beforeEach } from "vitest";
import { createRoot, type Accessor } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createRemoteSettings, type RemoteSettings } from "./createRemoteSettings";
import type { RemoteNotice } from "./ipc";

const mocked = vi.mocked(invoke);

/// Captures the notice handler the hook registers, so a test can play the device's
/// part and emit one.
function captureNotices(): { emit: (notice: RemoteNotice) => void } {
  let handler: ((event: { payload: RemoteNotice }) => void) | undefined;
  vi.mocked(listen).mockImplementation(async (_event, cb) => {
    handler = cb as (event: { payload: RemoteNotice }) => void;
    return () => {};
  });
  return {
    emit: (notice) => {
      if (!handler) throw new Error("the hook did not subscribe to notices");
      handler({ payload: notice });
    },
  };
}

/// Answers keyed by command name. A command with no answer here rejects, which is
/// how the "not compiled in" case is expressed.
function respond(answers: Record<string, unknown | (() => unknown)>) {
  mocked.mockImplementation(async (cmd: string) => {
    if (!(cmd in answers)) throw new Error(`Command ${cmd} not found`);
    const answer = answers[cmd];
    return typeof answer === "function" ? (answer as () => unknown)() : answer;
  });
}

const status = (over: Record<string, unknown> = {}) => ({
  enabled: false,
  deviceKey: "aa".repeat(32),
  error: null,
  ...over,
});

const policy = () => ({
  path: "/tmp/remote-policy.json",
  active: true,
  inertBecause: null,
  effective: {
    send_message: true,
    steer: true,
    interrupt: true,
    set_mode: false,
    approve_edit: true,
    approve_bash: "allowlist",
    read_attachment: false,
    export_file: false,
  },
  workspaces: ["/Users/v/work"],
  bashDenylist: [],
});

const pairing = (over: Record<string, unknown> = {}) => ({
  peer_key: "11".repeat(32),
  label: "Safari on iPhone",
  paired_at: 1_800_000_000_000,
  expires_at: null,
  ...over,
});

/// Runs `body` inside a root so signals have an owner, and disposes it after.
async function withSettings(
  body: (s: RemoteSettings) => Promise<void>,
  workspace: Accessor<string | null> = () => "/Users/v/work",
) {
  let dispose = () => {};
  const settings = createRoot((d) => {
    dispose = d;
    return createRemoteSettings(workspace);
  });
  // The notice subscription is a promise the hook does not expose, so give it a
  // turn to land before a test plays the device's part.
  await settle();
  try {
    await body(settings);
  } finally {
    dispose();
  }
}

/// Mutations are fire-and-forget by design (the UI must not await a click), so
/// tests need a turn or two of the microtask queue to settle.
const settle = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  mocked.mockReset();
  vi.mocked(listen).mockReset().mockResolvedValue(() => {});
});

describe("createRemoteSettings", () => {
  it("knows nothing before it has asked", async () => {
    respond({});
    await withSettings(async (s) => {
      expect(s.available()).toBeNull();
    });
  });

  /// A build without `--features remote` has no `remote_status` command at all.
  /// The panel must disappear rather than offer buttons that reject.
  it("reports unavailable when the device has no remote commands", async () => {
    respond({});
    await withSettings(async (s) => {
      await s.probe();
      expect(s.available()).toBe(false);
      expect(s.deviceKey()).toBeNull();
      expect(s.policy()).toBeNull();
    });
  });

  /// And it must not then go on to call the rest, which would only produce noise
  /// in the log for a build that was never going to answer.
  it("stops asking once it knows remote access is not there", async () => {
    respond({});
    await withSettings(async (s) => {
      await s.probe();
      const asked = mocked.mock.calls.map((c) => c[0]);
      expect(asked).toEqual(["remote_status"]);
    });
  });

  it("loads the key, the policy and the pairings when remote access is there", async () => {
    respond({
      remote_status: status(),
      remote_policy: policy(),
      remote_pairings: [pairing()],
      remote_revoked: ["22".repeat(32)],
    });
    await withSettings(async (s) => {
      await s.probe();
      expect(s.available()).toBe(true);
      expect(s.deviceKey()).toBe("aa".repeat(32));
      expect(s.policy()?.path).toBe("/tmp/remote-policy.json");
      expect(s.pairings()).toHaveLength(1);
      expect(s.revoked()).toEqual(["22".repeat(32)]);
      expect(s.error()).toBeNull();
    });
  });

  /// `remote_status` answers instead of failing, so a broken identity arrives as
  /// a value. It has to reach the screen — an empty panel with no explanation is
  /// the failure mode this field exists to prevent.
  it("surfaces an identity that could not be read", async () => {
    respond({
      remote_status: status({ deviceKey: null, error: "device.key is not a device identity" }),
      remote_policy: policy(),
      remote_pairings: [],
      remote_revoked: [],
    });
    await withSettings(async (s) => {
      await s.probe();
      expect(s.available()).toBe(true);
      expect(s.error()).toContain("not a device identity");
    });
  });

  /// A damaged pairing book must not take the policy down with it. The user came
  /// to read what a browser is allowed to do; that answer is still available.
  it("keeps what loaded when one of the three fails", async () => {
    respond({
      remote_status: status(),
      remote_policy: policy(),
      remote_revoked: [],
      remote_pairings: () => {
        throw new Error("pairings.json is not valid JSON");
      },
    });
    await withSettings(async (s) => {
      await s.probe();
      expect(s.policy()).not.toBeNull();
      expect(s.error()).toContain("not valid JSON");
    });
  });

  it("creates a device key on request and shows it", async () => {
    respond({
      remote_status: status({ deviceKey: null }),
      remote_policy: policy(),
      remote_pairings: [],
      remote_revoked: [],
      remote_create_identity: "bb".repeat(32),
    });
    await withSettings(async (s) => {
      await s.probe();
      expect(s.deviceKey()).toBeNull();

      s.createIdentity();
      await settle();

      expect(s.deviceKey()).toBe("bb".repeat(32));
      expect(s.busy()).toBe(false);
    });
  });

  /// The list is the user's evidence that the revoke happened. Leaving a revoked
  /// peer on screen is the one outcome that would make them doubt it.
  it("reloads the pairings after a revoke", async () => {
    let paired = [pairing(), pairing({ peer_key: "33".repeat(32), label: "second" })];
    respond({
      remote_status: status(),
      remote_policy: policy(),
      remote_revoked: [],
      remote_pairings: () => paired,
      remote_revoke: () => {
        paired = paired.slice(1);
        return null;
      },
    });
    await withSettings(async (s) => {
      await s.probe();
      expect(s.pairings()).toHaveLength(2);

      s.revoke("11".repeat(32));
      await settle();

      expect(s.pairings()).toHaveLength(1);
      expect(s.pairings()[0].label).toBe("second");
    });
  });

  /// A revoke that failed halfway would otherwise leave the screen asserting a
  /// state nobody checked. Reload regardless, and say what went wrong.
  it("reloads and reports when a revoke fails", async () => {
    respond({
      remote_status: status(),
      remote_policy: policy(),
      remote_revoked: [],
      remote_pairings: [pairing()],
      remote_revoke: () => {
        throw new Error("read-only file system");
      },
    });
    await withSettings(async (s) => {
      await s.probe();

      s.revoke("11".repeat(32));
      await settle();

      expect(s.error()).toContain("read-only file system");
      expect(s.pairings()).toHaveLength(1);
      expect(s.busy()).toBe(false);
    });
  });

  it("renames a pairing with the label it was given", async () => {
    respond({
      remote_status: status(),
      remote_policy: policy(),
      remote_revoked: [],
      remote_pairings: [pairing()],
      remote_rename_pairing: null,
    });
    await withSettings(async (s) => {
      await s.probe();

      s.rename("11".repeat(32), "Firefox on the laptop");
      await settle();

      expect(mocked).toHaveBeenCalledWith("remote_rename_pairing", {
        peerKey: "11".repeat(32),
        label: "Firefox on the laptop",
      });
    });
  });

  it("allows a revoked key again", async () => {
    respond({
      remote_status: status(),
      remote_policy: policy(),
      remote_revoked: [],
      remote_pairings: [],
      remote_unrevoke: null,
    });
    await withSettings(async (s) => {
      await s.probe();

      s.unrevoke("22".repeat(32));
      await settle();

      expect(mocked).toHaveBeenCalledWith("remote_unrevoke", { peerKey: "22".repeat(32) });
    });
  });

  /// Two revokes from one impatient double-click must not race the reload that
  /// follows each of them.
  it("runs one mutation at a time", async () => {
    let revokes = 0;
    respond({
      remote_status: status(),
      remote_policy: policy(),
      remote_revoked: [],
      remote_pairings: [],
      remote_revoke: () => {
        revokes += 1;
        return null;
      },
    });
    await withSettings(async (s) => {
      await s.probe();

      s.revoke("11".repeat(32));
      s.revoke("11".repeat(32));
      await settle();

      expect(revokes).toBe(1);
    });
  });

  // --- pairing -------------------------------------------------------------

  const pairingAnswers = () => ({
    remote_status: status(),
    remote_policy: policy(),
    remote_pairings: [],
    remote_revoked: [],
  });

  const code = () => ({
    url: "https://app.claudin.io/#c=abab&k=cdcd&r=wss%3A%2F%2Fr&e=1",
    channel: "ab".repeat(16),
    deviceKey: "cd".repeat(32),
    expiresAt: 1_800_000_120_000,
    qrSvg: "<svg/>",
  });

  /// Generating a key is a separate deliberate act, so the panel has to say that
  /// is what is missing rather than offering a button that cannot work.
  it("will not pair without a device key", async () => {
    respond({ ...pairingAnswers(), remote_status: status({ deviceKey: null }) });
    await withSettings(async (s) => {
      await s.probe();
      expect(s.blocked()).toContain("device key");
    });
  });

  /// A browser drives a session, and there is no session without a workspace.
  it("will not pair with no workspace open", async () => {
    respond(pairingAnswers());
    await withSettings(
      async (s) => {
        await s.probe();
        expect(s.blocked()).toContain("Open a workspace");
      },
      () => null,
    );
  });

  it("is ready to pair once there is a key and a workspace", async () => {
    respond(pairingAnswers());
    await withSettings(async (s) => {
      await s.probe();
      expect(s.blocked()).toBeNull();
    });
  });

  it("asks the device for a code and shows it", async () => {
    respond({ ...pairingAnswers(), remote_start_pairing: code() });
    await withSettings(async (s) => {
      await s.probe();

      s.startPairing("Safari on iPhone");
      await settle();

      expect(s.code()?.channel).toBe("ab".repeat(16));
      const call = mocked.mock.calls.find((c) => c[0] === "remote_start_pairing");
      expect(call?.[1]).toMatchObject({
        args: { workspace: "/Users/v/work", peerLabel: "Safari on iPhone" },
      });
    });
  });

  /// §6.3 wants every grant to expire. A pairing that outlives the reason it was
  /// made is the one nobody remembers to revoke.
  it("gives the pairing an expiry rather than leaving it open-ended", async () => {
    respond({ ...pairingAnswers(), remote_start_pairing: code() });
    await withSettings(async (s) => {
      await s.probe();
      s.startPairing("iPhone");
      await settle();

      const call = mocked.mock.calls.find((c) => c[0] === "remote_start_pairing");
      const expires = (call?.[1] as { args: { pairingExpiresAt: number } }).args
        .pairingExpiresAt;
      expect(expires).toBeGreaterThan(Date.now());
    });
  });

  /// The words replace the code. Leaving the QR on screen gives someone something
  /// else to look at while the only thing that matters goes unanswered.
  it("swaps the code for the words when the device asks for a check", async () => {
    const notices = captureNotices();
    respond({ ...pairingAnswers(), remote_start_pairing: code() });
    await withSettings(async (s) => {
      await s.probe();
      s.startPairing("iPhone");
      await settle();
      expect(s.code()).not.toBeNull();

      notices.emit({
        kind: "confirmPairing",
        peerKey: "11".repeat(32),
        label: "iPhone",
        sas: "basalt · dahlia · fathom",
      });
      await settle();

      expect(s.code()).toBeNull();
      expect(s.pending()?.sas).toBe("basalt · dahlia · fathom");
    });
  });

  it("sends the answer for the key that is waiting", async () => {
    const notices = captureNotices();
    respond({ ...pairingAnswers(), remote_confirm_pairing: null });
    await withSettings(async (s) => {
      await s.probe();
      notices.emit({
        kind: "confirmPairing",
        peerKey: "11".repeat(32),
        label: "iPhone",
        sas: "a · b · c",
      });
      await settle();

      s.confirmPairing(true);
      await settle();

      expect(mocked).toHaveBeenCalledWith("remote_confirm_pairing", {
        peerKey: "11".repeat(32),
        matched: true,
      });
    });
  });

  /// The answer is one-shot on the device. A second click would come back with
  /// "nothing is waiting for confirmation", which reads as a bug.
  it("cannot be answered twice", async () => {
    const notices = captureNotices();
    respond({ ...pairingAnswers(), remote_confirm_pairing: null });
    await withSettings(async (s) => {
      await s.probe();
      notices.emit({
        kind: "confirmPairing",
        peerKey: "11".repeat(32),
        label: "iPhone",
        sas: "a · b · c",
      });
      await settle();

      s.confirmPairing(true);
      s.confirmPairing(true);
      await settle();

      const answers = mocked.mock.calls.filter((c) => c[0] === "remote_confirm_pairing");
      expect(answers).toHaveLength(1);
      expect(s.pending()).toBeNull();
    });
  });

  it("reports a pairing that completed, and reloads the list", async () => {
    const notices = captureNotices();
    respond({
      ...pairingAnswers(),
      remote_pairings: [pairing({ label: "iPhone" })],
    });
    await withSettings(async (s) => {
      await s.probe();

      notices.emit({ kind: "paired", peerKey: "11".repeat(32), label: "iPhone" });
      await settle();

      expect(s.outcome()).toEqual({ kind: "paired", label: "iPhone" });
      expect(s.pending()).toBeNull();
      expect(s.pairings()).toHaveLength(1);
    });
  });

  /// A refusal revokes on the device, so the revoked list has to be reloaded or the
  /// panel will not show why the browser cannot simply scan again.
  it("reports a refusal and reloads the revoked list", async () => {
    const notices = captureNotices();
    respond({ ...pairingAnswers(), remote_revoked: ["11".repeat(32)] });
    await withSettings(async (s) => {
      await s.probe();

      notices.emit({ kind: "pairingRefused", peerKey: "11".repeat(32) });
      await settle();

      expect(s.outcome()).toEqual({ kind: "refused" });
      expect(s.revoked()).toEqual(["11".repeat(32)]);
    });
  });

  /// An already-paired browser reconnecting is not a pairing. Prompting for the
  /// words every time would teach the user to click through them.
  it("does not ask for a word check when a paired browser reconnects", async () => {
    const notices = captureNotices();
    respond(pairingAnswers());
    await withSettings(async (s) => {
      await s.probe();

      notices.emit({
        kind: "connected",
        peerKey: "11".repeat(32),
        label: "iPhone",
        sas: "a · b · c",
      });
      await settle();

      expect(s.pending()).toBeNull();
      expect(s.outcome()).toBeNull();
    });
  });

  it("clears a stale outcome when a new pairing starts", async () => {
    const notices = captureNotices();
    respond({ ...pairingAnswers(), remote_start_pairing: code() });
    await withSettings(async (s) => {
      await s.probe();
      notices.emit({ kind: "pairingRefused", peerKey: "11".repeat(32) });
      await settle();
      expect(s.outcome()).not.toBeNull();

      s.startPairing("second try");
      await settle();

      expect(s.outcome()).toBeNull();
    });
  });

  it("surfaces a device that refuses to start pairing", async () => {
    respond({
      ...pairingAnswers(),
      remote_start_pairing: () => {
        throw new Error("that workspace has no open session to share");
      },
    });
    await withSettings(async (s) => {
      await s.probe();

      s.startPairing("iPhone");
      await settle();

      expect(s.error()).toContain("no open session");
      expect(s.code()).toBeNull();
    });
  });
});
