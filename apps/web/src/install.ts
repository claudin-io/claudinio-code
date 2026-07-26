/// Registering the worker, and the one place an install is offered.
///
/// §0.1 puts the manifest and the worker in phase 3 rather than phase 6, and is explicit
/// about the shape: **offered, never required — the uninstalled browser tab stays a fully
/// working peer.** Everything here follows from that. Nothing in this file is on the path
/// to pairing, nothing here can fail in a way the user has to deal with, and none of it
/// runs before the session does.

/// Register the service worker.
///
/// Failure is swallowed on purpose. It fails for reasons that are none of the user's
/// business and that do not affect the peer: private browsing, a blocked-storage setting,
/// an http origin during development, a browser that has no worker at all. The one thing
/// that must not happen is a page that reports a problem the user cannot act on and does
/// not need to. Whether it registered is visible in devtools, which is where anyone who
/// cares to know is already looking.
export function registerServiceWorker(
  container: ServiceWorkerContainer | undefined = typeof navigator === "undefined"
    ? undefined
    : navigator.serviceWorker,
): void {
  if (!container) return;
  // Scope stated rather than inferred. It is inferred from the script's directory, which
  // is the root today — saying so means a build that ever moves the file fails loudly
  // instead of registering a worker that controls a subdirectory nobody navigates to.
  void container.register("/sw.js", { scope: "/" }).catch(() => {});
}

// ── offering the install ─────────────────────────────────────────────────────

/// What to tell the user, when there is anything to tell them.
export interface InstallOffer {
  headline: string;
  detail: string;
}

/// Just enough of `Navigator` to decide, and to fake.
interface NavigatorLike {
  platform?: string;
  userAgent?: string;
  maxTouchPoints?: number;
  standalone?: boolean;
}

/// Just enough of `Window`.
interface WindowLike {
  navigator: NavigatorLike;
  matchMedia?: (query: string) => { matches: boolean };
  localStorage?: Pick<Storage, "getItem" | "setItem">;
}

const DISMISSED = "claudinio.install-offer.dismissed";

/// Whether the page is already running as an installed app.
export function isStandalone(win: WindowLike): boolean {
  // iOS has its own flag and, on older versions, no `display-mode` media query at all.
  if (win.navigator.standalone === true) return true;
  return win.matchMedia?.("(display-mode: standalone)").matches === true;
}

/// Whether this is an iPhone or an iPad.
///
/// The only family the offer is made to — see `installOffer`.
export function isIosFamily(nav: NavigatorLike): boolean {
  const platform = nav.platform ?? "";
  if (/^(iPhone|iPad|iPod)/.test(platform)) return true;
  // iPadOS 13 and later claim to be a Mac. A Mac with a touchscreen is an iPad.
  if (platform === "MacIntel" && (nav.maxTouchPoints ?? 0) > 1) return true;
  return /iPhone|iPad|iPod/.test(nav.userAgent ?? "");
}

/// Whether to offer the install, and in what words.
///
/// Offered to iOS only, which is not a hedge — it is the whole reason the offer exists.
/// Chrome and Edge put an install control in their own UI and can show a notification
/// from an ordinary tab, so there is nothing here that the browser is not already saying
/// better. Safari on iOS does neither: there is no prompt to trigger, and Web Push is
/// available *only* to a page launched from the home screen. So on iOS an uninstalled
/// page will silently never be able to reach someone whose phone is in their pocket, and
/// the only way they will find out is being told.
///
/// Returns nothing once it is installed, and nothing once it has been dismissed. An offer
/// that comes back is an advertisement.
export function installOffer(win: WindowLike): InstallOffer | null {
  if (isStandalone(win)) return null;
  if (!isIosFamily(win.navigator)) return null;
  if (dismissed(win)) return null;

  return {
    headline: "Add this to your home screen",
    detail:
      // Future tense, because the write path is phase 4 and the notification does not
      // exist yet. Promising it in the present would be a lie the user can check.
      "Tap Share, then Add to Home Screen. This tab keeps working either way — but only an installed page will be able to notify you when the agent needs an answer.",
  };
}

/// Remember that the offer was refused.
///
/// `localStorage` throws rather than returning null in a Safari with storage blocked, and
/// the whole feature is a line of text — so a failure to remember means the line comes
/// back next time, which is the worst thing that can happen here.
export function dismissInstallOffer(win: WindowLike): void {
  try {
    win.localStorage?.setItem(DISMISSED, "1");
  } catch {
    /* nothing to do about it, and nothing that depends on it */
  }
}

function dismissed(win: WindowLike): boolean {
  try {
    return win.localStorage?.getItem(DISMISSED) === "1";
  } catch {
    return false;
  }
}
