import { createSignal, onCleanup, type Accessor } from "solid-js";
import {
  DEFAULT_RELAY_URL,
  remoteStatus,
  remoteCreateIdentity,
  remotePolicy,
  remotePairings,
  remoteRevoked,
  remoteRevoke,
  remoteUnrevoke,
  remoteRenamePairing,
  remoteStartPairing,
  remoteConfirmPairing,
  onRemoteNotice,
  type Pairing,
  type PairingCodeView,
  type RemotePolicyView,
} from "./ipc";
import type { PairingOutcome, PendingPairing } from "../components/settings/RemotePairing";

export interface RemoteSettings {
  /// `null` until probed. `false` means this build has no remote access at all —
  /// see `probe` below for why a failed call is the honest signal for that.
  available: Accessor<boolean | null>;
  deviceKey: Accessor<string | null>;
  policy: Accessor<RemotePolicyView | null>;
  pairings: Accessor<Pairing[]>;
  revoked: Accessor<string[]>;
  error: Accessor<string | null>;
  busy: Accessor<boolean>;
  /// The pairing code currently on screen, if any.
  code: Accessor<PairingCodeView | null>;
  /// A pairing stopped at the word check. Nothing is being served while this is set.
  pending: Accessor<PendingPairing | null>;
  outcome: Accessor<PairingOutcome | null>;
  /// Why pairing cannot start — no device key, no workspace open.
  blocked: Accessor<string | null>;
  probe: () => Promise<void>;
  createIdentity: () => void;
  revoke: (peerKey: string) => void;
  unrevoke: (peerKey: string) => void;
  rename: (peerKey: string, label: string) => void;
  startPairing: (label: string) => void;
  cancelPairing: () => void;
  confirmPairing: (matched: boolean) => void;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/// Local state for the remote-access panel.
///
/// This is a hook rather than another dozen props on `SettingsPanel` because
/// remote state is not configuration: nothing here is staged and saved with the
/// Save button, every action lands immediately and irreversibly. Revoking a
/// pairing that a Cancel could undo would be a worse promise than the one §6.5
/// makes.
export function createRemoteSettings(
  /// The workspace whose session a paired browser would drive. `null` when none is
  /// open, which is a reason pairing cannot start rather than an error.
  workspace: Accessor<string | null> = () => null,
): RemoteSettings {
  const [available, setAvailable] = createSignal<boolean | null>(null);
  const [deviceKey, setDeviceKey] = createSignal<string | null>(null);
  const [policy, setPolicy] = createSignal<RemotePolicyView | null>(null);
  const [pairings, setPairings] = createSignal<Pairing[]>([]);
  const [revoked, setRevoked] = createSignal<string[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  /// Ask the device what it knows, and find out whether it can be asked at all.
  ///
  /// `remote_status` is built to answer rather than fail: a missing identity, an
  /// unreadable one, even a missing config directory all come back as `Ok` with
  /// an `error` field. So a rejection can only mean the command is not
  /// registered — which is exactly what a build without `--features remote`
  /// looks like from here. That is why availability is read off the call failing
  /// and not off matching an error string, which would break the day Tauri
  /// rewords it.
  const probe = async () => {
    try {
      const status = await remoteStatus();
      setAvailable(true);
      setDeviceKey(status.deviceKey);
      setError(status.error);
    } catch {
      setAvailable(false);
      return;
    }
    await load();
  };

  /// Each of the three is loaded independently: a damaged pairing book should not
  /// also hide the policy the user came here to read.
  const load = async () => {
    const results = await Promise.allSettled([remotePolicy(), remotePairings(), remoteRevoked()]);
    const [p, pr, rv] = results;
    if (p.status === "fulfilled") setPolicy(p.value);
    if (pr.status === "fulfilled") setPairings(pr.value);
    if (rv.status === "fulfilled") setRevoked(rv.value);

    const failed = results.find((r) => r.status === "rejected");
    if (failed?.status === "rejected") setError(message(failed.reason));
  };

  /// One mutation at a time, and always reload afterwards — including after a
  /// failure, because a revoke that half-succeeded must not leave the list
  /// claiming the peer is still paired.
  const mutate = (act: () => Promise<void>) => {
    if (busy()) return;
    setBusy(true);
    setError(null);
    void act()
      .catch((e) => setError(message(e)))
      .then(load)
      .finally(() => setBusy(false));
  };

  // --- pairing -------------------------------------------------------------

  const [code, setCode] = createSignal<PairingCodeView | null>(null);
  const [pending, setPending] = createSignal<PendingPairing | null>(null);
  const [outcome, setOutcome] = createSignal<PairingOutcome | null>(null);

  const blocked = () => {
    if (!deviceKey()) return "Create a device key first.";
    if (!workspace()) return "Open a workspace to pair a browser with its session.";
    return null;
  };

  /// Notices are subscribed to once, for the life of the panel — not only while a
  /// code is on screen. The word check arrives after the browser has scanned,
  /// which can be a minute later, and an already-paired browser reconnecting is
  /// also worth knowing about.
  void onRemoteNotice((notice) => {
    switch (notice.kind) {
      case "confirmPairing":
        // The code has done its job; what matters now is the words.
        setCode(null);
        setPending({ peerKey: notice.peerKey, label: notice.label, sas: notice.sas });
        break;
      case "paired":
        setPending(null);
        setOutcome({ kind: "paired", label: notice.label });
        void load();
        break;
      case "pairingRefused":
        setPending(null);
        setOutcome({ kind: "refused" });
        void load();
        break;
      case "connected":
        break;
    }
  }).then((stop) => onCleanup(stop));

  return {
    available,
    deviceKey,
    policy,
    pairings,
    revoked,
    error,
    busy,
    code,
    pending,
    outcome,
    blocked,
    probe,
    startPairing: (label) => {
      const ws = workspace();
      if (!ws) return;
      setOutcome(null);
      mutate(async () => {
        setCode(
          await remoteStartPairing({
            relayUrl: DEFAULT_RELAY_URL,
            workspace: ws,
            peerLabel: label,
            // Every grant expires by default (§6.3). Seven days is long enough to
            // be useful for a trip and short enough that a forgotten pairing
            // closes itself.
            pairingExpiresAt: Date.now() + 7 * 24 * 60 * 60 * 1000,
          }),
        );
      });
    },
    /// Only clears the screen. The device's own window lapses on its clock, and
    /// pretending otherwise would need a command that stops a connection — which
    /// does not exist yet, and which the user would reasonably expect to also
    /// close an established one.
    cancelPairing: () => setCode(null),
    confirmPairing: (matched) => {
      const waiting = pending();
      if (!waiting) return;
      // Cleared first: the answer is one-shot, and leaving the words on screen
      // invites a second click that would fail with "nothing is waiting".
      setPending(null);
      mutate(() => remoteConfirmPairing(waiting.peerKey, matched));
    },
    createIdentity: () =>
      mutate(async () => {
        setDeviceKey(await remoteCreateIdentity());
      }),
    revoke: (peerKey) => mutate(() => remoteRevoke(peerKey)),
    unrevoke: (peerKey) => mutate(() => remoteUnrevoke(peerKey)),
    rename: (peerKey, label) => mutate(() => remoteRenamePairing(peerKey, label)),
  };
}
