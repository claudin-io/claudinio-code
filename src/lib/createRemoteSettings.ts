import { createSignal, type Accessor } from "solid-js";
import {
  remoteStatus,
  remoteCreateIdentity,
  remotePolicy,
  remotePairings,
  remoteRevoked,
  remoteRevoke,
  remoteUnrevoke,
  remoteRenamePairing,
  type Pairing,
  type RemotePolicyView,
} from "./ipc";

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
  probe: () => Promise<void>;
  createIdentity: () => void;
  revoke: (peerKey: string) => void;
  unrevoke: (peerKey: string) => void;
  rename: (peerKey: string, label: string) => void;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/// Local state for the remote-access panel.
///
/// This is a hook rather than another dozen props on `SettingsPanel` because
/// remote state is not configuration: nothing here is staged and saved with the
/// Save button, every action lands immediately and irreversibly. Revoking a
/// pairing that a Cancel could undo would be a worse promise than the one §6.5
/// makes.
export function createRemoteSettings(): RemoteSettings {
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

  return {
    available,
    deviceKey,
    policy,
    pairings,
    revoked,
    error,
    busy,
    probe,
    createIdentity: () =>
      mutate(async () => {
        setDeviceKey(await remoteCreateIdentity());
      }),
    revoke: (peerKey) => mutate(() => remoteRevoke(peerKey)),
    unrevoke: (peerKey) => mutate(() => remoteUnrevoke(peerKey)),
    rename: (peerKey, label) => mutate(() => remoteRenamePairing(peerKey, label)),
  };
}
