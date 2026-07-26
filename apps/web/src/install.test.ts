import { describe, it, expect, vi } from "vitest";
import {
  dismissInstallOffer,
  installOffer,
  isIosFamily,
  isStandalone,
  registerServiceWorker,
} from "./install";

/// A window, as much of one as any of this reads.
function fakeWindow(
  options: {
    platform?: string;
    userAgent?: string;
    maxTouchPoints?: number;
    standalone?: boolean;
    displayMode?: string;
    storage?: Record<string, string> | "blocked";
  } = {},
) {
  const stored: Record<string, string> =
    options.storage === "blocked" || options.storage === undefined ? {} : options.storage;

  const blocked = options.storage === "blocked";
  const localStorage = {
    getItem: (key: string) => {
      if (blocked) throw new Error("The operation is insecure.");
      return stored[key] ?? null;
    },
    setItem: (key: string, value: string) => {
      if (blocked) throw new Error("The operation is insecure.");
      stored[key] = value;
    },
  };

  return {
    stored,
    win: {
      navigator: {
        platform: options.platform ?? "iPhone",
        userAgent: options.userAgent ?? "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)",
        maxTouchPoints: options.maxTouchPoints ?? 5,
        ...(options.standalone === undefined ? {} : { standalone: options.standalone }),
      },
      matchMedia: (query: string) => ({
        matches: options.displayMode !== undefined && query.includes(options.displayMode),
      }),
      localStorage,
    },
  };
}

describe("registerServiceWorker", () => {
  it("registers the worker at the root", () => {
    const register = vi.fn().mockResolvedValue({});

    registerServiceWorker({ register } as unknown as ServiceWorkerContainer);

    expect(register).toHaveBeenCalledWith("/sw.js", { scope: "/" });
  });

  /// Offered, never required (§0.1). A browser with no service worker at all — an
  /// insecure origin during development, a locked-down configuration — must get a
  /// working peer and no complaint.
  it("does nothing when the browser has no service worker", () => {
    expect(() => registerServiceWorker(undefined)).not.toThrow();
  });

  /// The one that would otherwise reach the console as an unhandled rejection, on a page
  /// whose actual job is unaffected. Private browsing in Safari refuses the registration.
  it("swallows a refused registration", async () => {
    const register = vi.fn().mockRejectedValue(new Error("The operation is insecure."));
    const unhandled = vi.fn();
    process.on("unhandledRejection", unhandled);

    registerServiceWorker({ register } as unknown as ServiceWorkerContainer);
    await new Promise((resolve) => setTimeout(resolve, 5));

    process.off("unhandledRejection", unhandled);
    expect(unhandled).not.toHaveBeenCalled();
  });
});

describe("isStandalone", () => {
  it("reads iOS's own flag", () => {
    expect(isStandalone(fakeWindow({ standalone: true }).win)).toBe(true);
  });

  it("reads the display-mode media query", () => {
    expect(isStandalone(fakeWindow({ displayMode: "standalone" }).win)).toBe(true);
  });

  it("is false in an ordinary tab", () => {
    expect(isStandalone(fakeWindow().win)).toBe(false);
  });

  /// Older iOS has the flag and not the query. Reading only the query there would offer
  /// an install to a page that is already installed.
  it("does not need matchMedia to exist", () => {
    const { win } = fakeWindow({ standalone: true });
    expect(isStandalone({ ...win, matchMedia: undefined })).toBe(true);
  });
});

describe("isIosFamily", () => {
  it("knows an iPhone", () => {
    expect(isIosFamily({ platform: "iPhone" })).toBe(true);
  });

  /// iPadOS 13 and later claim to be a Mac. A Mac with a touchscreen is an iPad, and an
  /// iPad is where this offer matters as much as on a phone.
  it("knows an iPad pretending to be a Mac", () => {
    expect(isIosFamily({ platform: "MacIntel", maxTouchPoints: 5 })).toBe(true);
  });

  it("does not mistake a Mac for an iPad", () => {
    expect(isIosFamily({ platform: "MacIntel", maxTouchPoints: 0 })).toBe(false);
  });

  it("falls back to the user agent when there is no platform", () => {
    expect(
      isIosFamily({ userAgent: "Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X) Safari" }),
    ).toBe(true);
  });

  it("says no for Android", () => {
    expect(
      isIosFamily({ platform: "Linux armv8l", userAgent: "Mozilla/5.0 (Linux; Android 15)" }),
    ).toBe(false);
  });
});

describe("installOffer", () => {
  it("offers the install on an uninstalled iPhone", () => {
    const offer = installOffer(fakeWindow().win);

    expect(offer?.headline).toBe("Add this to your home screen");
    expect(offer?.detail).toContain("Add to Home Screen");
  });

  /// The detail says what the install buys, and says it in the future tense: the write
  /// path is phase 4 and there is no notification yet. A page that promised one now
  /// would be making a claim the user can check.
  it("does not promise a notification it cannot yet send", () => {
    const offer = installOffer(fakeWindow().win);

    expect(offer?.detail).toContain("will be able to notify you");
    expect(offer?.detail).toContain("keeps working either way");
  });

  it("says nothing once it is installed", () => {
    expect(installOffer(fakeWindow({ standalone: true }).win)).toBeNull();
  });

  /// Not a hedge — the whole reason the offer exists. Chrome and Edge put an install
  /// control in their own UI and can notify from an ordinary tab, so there is nothing
  /// here they are not already saying better.
  it("says nothing on a browser that offers its own install", () => {
    expect(
      installOffer(
        fakeWindow({ platform: "Linux armv8l", userAgent: "Android 15", maxTouchPoints: 5 }).win,
      ),
    ).toBeNull();
  });

  it("says nothing once it has been dismissed", () => {
    const { win } = fakeWindow();
    dismissInstallOffer(win);

    expect(installOffer(win)).toBeNull();
  });

  /// An offer that comes back is an advertisement, and this one lives on the origin that
  /// drives someone's machine. Its credibility is the point.
  it("stays dismissed across a fresh page load", () => {
    const first = fakeWindow();
    dismissInstallOffer(first.win);

    const second = fakeWindow({ storage: first.stored });
    expect(installOffer(second.win)).toBeNull();
  });
});

describe("dismissInstallOffer", () => {
  /// Safari with storage blocked throws rather than returning null. The whole feature is
  /// a line of text, so the worst outcome of not remembering is that it comes back.
  it("does not fail when storage is blocked", () => {
    const { win } = fakeWindow({ storage: "blocked" });

    expect(() => dismissInstallOffer(win)).not.toThrow();
    expect(() => installOffer(win)).not.toThrow();
  });

  it("does not fail when there is no storage at all", () => {
    const { win } = fakeWindow();

    expect(() => dismissInstallOffer({ ...win, localStorage: undefined })).not.toThrow();
    expect(installOffer({ ...win, localStorage: undefined })).not.toBeNull();
  });
});
