import { createSignal, createRoot, createEffect } from "solid-js";

export type LocaleId =
  | "en-US" | "pt-BR" | "pt-PT" | "es-ES" | "fr-FR" | "de-DE"
  | "it-IT" | "ru-RU" | "tr-TR" | "ar-SA" | "hi-IN" | "bn-BD"
  | "ur-PK" | "zh-CN" | "ja-JP" | "ko-KR" | "vi-VN" | "id-ID";

export const SUPPORTED_LOCALES: LocaleId[] = [
  "en-US", "pt-BR", "pt-PT", "es-ES", "fr-FR", "de-DE",
  "it-IT", "ru-RU", "tr-TR", "ar-SA", "hi-IN", "bn-BD",
  "ur-PK", "zh-CN", "ja-JP", "ko-KR", "vi-VN", "id-ID",
];

/** Resolve a raw locale string (from browser or OS) to a supported LocaleId.
 *  Exact match wins, then language-prefix match ("pt" → "pt-BR"), then "en-US". */
export function resolveLocale(raw: string): LocaleId {
  // Exact match
  if ((SUPPORTED_LOCALES as readonly string[]).includes(raw)) return raw as LocaleId;
  // Language prefix match (e.g. "pt" matches "pt-BR")
  const prefix = raw.split("-")[0].toLowerCase();
  for (const loc of SUPPORTED_LOCALES) {
    if (loc.toLowerCase().startsWith(prefix)) return loc;
  }
  // Fallback
  return "en-US";
}

export interface LocaleDict {
  [key: string]: string | ((...args: (string | number)[]) => string);
}

// ── reactive locale signal (persisted) ──────────────────────────────
const STORAGE_KEY = "claudinio_locale";

function createLocaleState() {
  const stored = (typeof localStorage !== "undefined"
    ? localStorage.getItem(STORAGE_KEY)
    : null) as LocaleId | null;
  const initial: LocaleId = stored ?? "en-US";
  const [locale, _setLocale] = createSignal<LocaleId>(initial);

  // If no stored preference, detect from system
  if (!stored) {
    void (async () => {
      try {
        // 1. Browser/WebView locale (reflects OS in Tauri)
        if (typeof navigator !== "undefined" && navigator.language) {
          const resolved = resolveLocale(navigator.language);
          if (resolved !== "en-US") { _setLocale(resolved); return; }
        }
        // 2. Tauri OS locale
        const { getOsLocale } = await import("../lib/ipc");
        const osLocale = await getOsLocale();
        if (osLocale) {
          const resolved = resolveLocale(osLocale);
          if (resolved !== "en-US") { _setLocale(resolved); return; }
        }
      } catch {
        // Fall through — stay on en-US
      }
    })();
  }

  const setLocale = (id: LocaleId) => {
    _setLocale(id);
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(STORAGE_KEY, id);
    }
  };

  return { locale, setLocale };
}

let localeState: ReturnType<typeof createLocaleState>;
function getLocaleState() {
  if (!localeState) {
    createRoot(() => {
      localeState = createLocaleState();
    });
  }
  return localeState;
}

export const __localeProxy = new Proxy(
  {} as {
    locale: ReturnType<typeof createLocaleState>["locale"];
    setLocale: ReturnType<typeof createLocaleState>["setLocale"];
  },
  {
    get(_target, prop) {
      const s = getLocaleState();
      if (prop === "locale") return s.locale;
      if (prop === "setLocale") return s.setLocale;
      return undefined;
    },
  },
);

export const { locale, setLocale } = __localeProxy;

/** @internal exported for testing — clears a specific locale from the dict
 * cache so its next load is genuinely pending (useful for race-condition tests
 * where earlier tests may have cached the locale). Pass no id to clear all. */
export function __clearDictCache(id?: LocaleId) {
  if (id) {
    dictCache.delete(id);
  } else {
    dictCache.clear();
  }
}

/** @internal exported for testing — applies a dict only if the locale still
 * matches, simulating the effect's stale-load guard without async timing. */
export function __applyDictIfCurrent(id: LocaleId, d: LocaleDict) {
  if (getLocaleState().locale() === id) setCurrentDict(d);
}

// ── loader for locale dicts ─────────────────────────────────────────
const dictCache = new Map<LocaleId, LocaleDict>();

/** @internal exported for testing */
export async function loadDict(id: LocaleId): Promise<LocaleDict> {
  if (dictCache.has(id)) return dictCache.get(id)!;
  let mod: { default: LocaleDict };
  switch (id) {
    case "pt-BR":   mod = await import("./locales/pt-BR"); break;
    case "en-US":   mod = await import("./locales/en-US"); break;
    case "pt-PT":   mod = await import("./locales/pt-PT"); break;
    case "es-ES":   mod = await import("./locales/es-ES"); break;
    case "fr-FR":   mod = await import("./locales/fr-FR"); break;
    case "de-DE":   mod = await import("./locales/de-DE"); break;
    case "it-IT":   mod = await import("./locales/it-IT"); break;
    case "ru-RU":   mod = await import("./locales/ru-RU"); break;
    case "tr-TR":   mod = await import("./locales/tr-TR"); break;
    case "ar-SA":   mod = await import("./locales/ar-SA"); break;
    case "hi-IN":   mod = await import("./locales/hi-IN"); break;
    case "bn-BD":   mod = await import("./locales/bn-BD"); break;
    case "ur-PK":   mod = await import("./locales/ur-PK"); break;
    case "zh-CN":   mod = await import("./locales/zh-CN"); break;
    case "ja-JP":   mod = await import("./locales/ja-JP"); break;
    case "ko-KR":   mod = await import("./locales/ko-KR"); break;
    case "vi-VN":   mod = await import("./locales/vi-VN"); break;
    case "id-ID":   mod = await import("./locales/id-ID"); break;
  }
  dictCache.set(id, mod.default);
  return mod.default;
}

const [currentDict, setCurrentDict] = createRoot(() => createSignal<LocaleDict>({}));

// Lazy-load the initial dict. Guard against a stale load clobbering a newer
// locale: only apply the result if the requested locale is still current.
const initialLocale = getLocaleState().locale();
loadDict(initialLocale).then((d) => {
  if (getLocaleState().locale() === initialLocale) setCurrentDict(d);
});

// Subscribe to locale changes
let effectStarted = false;
/** @internal exported for testing */
export function ensureDictWatcher() {
  if (effectStarted) return;
  effectStarted = true;
  createRoot(() => {
    createEffect(() => {
      const id = getLocaleState().locale();
      // Only the latest requested locale wins: a slower earlier load must not
      // overwrite a newer one that already resolved (loads race, order isn't
      // guaranteed). Without this, rapid locale switches settle on the wrong dict.
      loadDict(id).then((d) => {
        if (getLocaleState().locale() === id) setCurrentDict(d);
      });
    });
  });
}
ensureDictWatcher();

// Preload en-US so the t() fallback chain works synchronously for
// locales whose dictionaries haven't been translated yet.
loadDict("en-US");

// Keep <html lang> in sync with the current locale
createRoot(() => {
  createEffect(() => {
    document.documentElement.lang = getLocaleState().locale();
  });
});

// ── t() translation function ────────────────────────────────────────
export function t(key: string, ...args: (string | number)[]): string {
  const dict = currentDict();
  let val = dict[key];
  if (val === undefined) {
    // Fallback chain: try en-US
    const enDict = dictCache.get("en-US");
    if (enDict) val = enDict[key];
  }
  if (val === undefined) return key;
  if (typeof val === "function") return val(...args);
  let result = val;
  for (let i = 0; i < args.length; i++) {
    result = result.replace(new RegExp(`\\{${i}\\}`, "g"), String(args[i]));
  }
  return result;
}

// ── flags ───────────────────────────────────────────────────────────
export const FLAGS: Record<LocaleId, string> = {
  "en-US": "🇺🇸", "pt-BR": "🇧🇷", "pt-PT": "🇵🇹",
  "es-ES": "🇪🇸", "fr-FR": "🇫🇷", "de-DE": "🇩🇪",
  "it-IT": "🇮🇹", "ru-RU": "🇷🇺", "tr-TR": "🇹🇷",
  "ar-SA": "🇸🇦", "hi-IN": "🇮🇳", "bn-BD": "🇧🇩",
  "ur-PK": "🇵🇰", "zh-CN": "🇨🇳", "ja-JP": "🇯🇵",
  "ko-KR": "🇰🇷", "vi-VN": "🇻🇳", "id-ID": "🇮🇩",
};

export const LOCALE_LABELS: Record<LocaleId, string> = {
  "en-US": "EN", "pt-BR": "PT", "pt-PT": "PT",
  "es-ES": "ES", "fr-FR": "FR", "de-DE": "DE",
  "it-IT": "IT", "ru-RU": "RU", "tr-TR": "TR",
  "ar-SA": "العربية", "hi-IN": "हिन्दी", "bn-BD": "বাংলা",
  "ur-PK": "اردو", "zh-CN": "中文", "ja-JP": "日本語",
  "ko-KR": "한국어", "vi-VN": "VI", "id-ID": "ID",
};
