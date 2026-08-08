import { useCallback, useEffect, useState } from "react";
import { load, type Store } from "@tauri-apps/plugin-store";

export type Theme = "light" | "dark";

const STORE_FILE = "conduit.json";
const KEY = "theme";

let storePromise: Promise<Store> | null = null;
function store() {
  storePromise ??= load(STORE_FILE, { autoSave: true });
  return storePromise;
}

function systemTheme(): Theme {
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/** Applied to <html> so CSS variables and `color-scheme` switch together. */
function apply(theme: Theme) {
  document.documentElement.dataset.theme = theme;
}

/**
 * Reads the persisted theme, falling back to the OS preference. Applied
 * synchronously on first paint by `bootTheme` so there is no flash; this hook
 * only owns subsequent changes.
 */
export function useTheme() {
  const [theme, setThemeState] = useState<Theme>(
    () => (document.documentElement.dataset.theme as Theme) ?? systemTheme(),
  );

  useEffect(() => {
    let alive = true;
    store()
      .then((s) => s.get<Theme>(KEY))
      .then((saved) => {
        if (!alive || !saved) return;
        setThemeState(saved);
        apply(saved);
      })
      .catch(() => {
        /* store unavailable (e.g. running in a plain browser) — keep OS value */
      });
    return () => {
      alive = false;
    };
  }, []);

  // Track the OS only while the user hasn't pinned a preference.
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = async () => {
      const s = await store().catch(() => null);
      const saved = await s?.get<Theme>(KEY);
      if (saved) return;
      const next = systemTheme();
      setThemeState(next);
      apply(next);
    };
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const setTheme = useCallback((next: Theme) => {
    setThemeState(next);
    apply(next);
    store()
      .then((s) => s.set(KEY, next))
      .catch(() => {});
  }, []);

  const toggle = useCallback(
    () => setTheme(theme === "dark" ? "light" : "dark"),
    [theme, setTheme],
  );

  return { theme, setTheme, toggle };
}

/**
 * Called before React mounts. Guesses from the OS immediately so the first
 * frame is already the right colour, then corrects from the store if the user
 * had pinned something else.
 */
export function bootTheme() {
  // `?theme=dark` pins the theme when reviewing the UI in a browser.
  const forced = new URLSearchParams(window.location.search).get("theme");
  if (forced === "dark" || forced === "light") {
    apply(forced);
    return;
  }

  apply(systemTheme());
  store()
    .then((s) => s.get<Theme>(KEY))
    .then((saved) => saved && apply(saved))
    .catch(() => {});
}
