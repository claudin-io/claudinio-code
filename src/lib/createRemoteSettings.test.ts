import { describe, it, expect, vi, beforeEach } from "vitest";
import { createRoot } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { createRemoteSettings, type RemoteSettings } from "./createRemoteSettings";

const mocked = vi.mocked(invoke);

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
async function withSettings(body: (s: RemoteSettings) => Promise<void>) {
  let dispose = () => {};
  const settings = createRoot((d) => {
    dispose = d;
    return createRemoteSettings();
  });
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
});
