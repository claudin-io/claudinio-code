import { createContext, useContext, type JSX } from "solid-js";

/// The handful of things the timeline needs from whoever is hosting it.
///
/// # Why a context rather than props
///
/// The timeline is a dozen components deep in places, and `openExternalUrl` is
/// needed by three leaves that are nowhere near each other. Threading it as a prop
/// means every intermediate component grows a parameter it does not use, and the
/// next leaf that needs it grows the chain again.
///
/// This is the dependency inversion that lets the timeline live outside the app.
/// Everything in this package must build without Tauri — see the README, and the
/// check in `source-hygiene.test.ts` — so anything that only the desktop app can
/// do arrives through here.
export interface TimelineHost {
  /// Open a link somewhere other than in this view.
  ///
  /// The desktop app hands this to the OS, because opening it in the webview would
  /// navigate away from the app. A browser opens a tab.
  openExternalUrl: (url: string) => void;

  /// Open a file reference from the transcript, when the host can.
  ///
  /// The desktop opens it in the editor. A browser driving a remote session cannot —
  /// the file is on the developer's machine and §2's invariants say it stays there — so
  /// this is optional and the web peer leaves it unset, which renders the reference as
  /// text rather than as a link that does nothing.
  openFile?: (path: string, line?: number) => void;

  /// Live network activity, when the host has any to show.
  ///
  /// A slot rather than a component, because it is genuinely host-specific: it
  /// reflects requests made by *this process*, and a browser driving a remote
  /// session has none — the requests are happening on the developer's machine.
  /// Rendering nothing is the correct answer there, not a stub that lies.
  networkIndicator?: () => JSX.Element;
}

/// Open a link in a new tab, refusing anything that is not plain web.
///
/// `window.open("javascript:…")` executes in this origin, so the scheme is checked
/// here rather than relying on the caller. The markdown renderer already only
/// marks `https?:` links as external and DOMPurify filters the rest, but a default
/// whose safety depends on an invariant two files away is a default that breaks
/// the day someone edits the other file.
function openInNewTab(url: string): void {
  let parsed: URL;
  try {
    parsed = new URL(url, globalThis.location?.href);
  } catch {
    return;
  }
  if (parsed.protocol !== "https:" && parsed.protocol !== "http:") return;
  // `noopener` so the opened page cannot reach back through `window.opener`.
  globalThis.open?.(parsed.href, "_blank", "noopener,noreferrer");
}

/// The default is the *browser's* behaviour, deliberately.
///
/// A missing provider then degrades to something correct rather than to a link
/// that silently does nothing or an exception inside a click handler. The desktop
/// app is the one that has to override it, and it is the one that knows it does.
const FALLBACK: TimelineHost = {
  openExternalUrl: openInNewTab,
};

const TimelineHostContext = createContext<TimelineHost>(FALLBACK);

export const TimelineHostProvider = TimelineHostContext.Provider;

export function useTimelineHost(): TimelineHost {
  return useContext(TimelineHostContext);
}
