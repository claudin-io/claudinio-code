import { createSignal, createRoot, createEffect } from "solid-js";

export type LocaleId = "pt-BR" | "en-US";

export interface LocaleDict {
  [key: string]: string | ((...args: (string | number)[]) => string);
}

// ── reactive locale signal (persisted) ──────────────────────────────
const STORAGE_KEY = "claudinio_locale";

function createLocaleState() {
  const stored = (typeof localStorage !== "undefined"
    ? localStorage.getItem(STORAGE_KEY)
    : null) as LocaleId | null;
  const initial: LocaleId = stored ?? "pt-BR";
  const [locale, _setLocale] = createSignal<LocaleId>(initial);

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

// ── loader for locale dicts ─────────────────────────────────────────
const dictCache = new Map<LocaleId, LocaleDict>();

/** @internal exported for testing */
export async function loadDict(id: LocaleId): Promise<LocaleDict> {
  if (dictCache.has(id)) return dictCache.get(id)!;
  let mod: { default: LocaleDict };
  if (id === "pt-BR") {
    mod = await import("./locales/pt-BR");
  } else {
    mod = await import("./locales/en-US");
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

// ── t() translation function ────────────────────────────────────────
export function t(key: string, ...args: (string | number)[]): string {
  const dict = currentDict();
  let val = dict[key];
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
  "pt-BR": "🇧🇷",
  "en-US": "🇺🇸",
};

export const LOCALE_LABELS: Record<LocaleId, string> = {
  "pt-BR": "PT",
  "en-US": "EN",
};
