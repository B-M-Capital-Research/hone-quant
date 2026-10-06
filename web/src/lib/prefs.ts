/**
 * Per-browser display preferences: theme, market colour convention and the second time zone.
 * Stored in localStorage (with safe fallbacks) and reflected as data attributes on <html>.
 */
import { createSignal } from "solid-js";

export type ThemePref = "auto" | "light" | "dark";
export type UpDown = "green-up" | "red-up" | "blue-orange";

const THEME_KEY = "hone-quant.theme";
const UPDOWN_KEY = "hone-quant.updown";
const TZ_KEY = "hone-quant.tz";

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* storage unavailable */
  }
}

const storedTheme = read(THEME_KEY);
const storedUpDown = read(UPDOWN_KEY);

const [themePref, setThemePrefSignal] = createSignal<ThemePref>(
  storedTheme === "light" || storedTheme === "dark" ? storedTheme : "auto",
);
const [upDown, setUpDownSignal] = createSignal<UpDown>(
  storedUpDown === "red-up" || storedUpDown === "blue-orange" ? storedUpDown : "green-up",
);
const [displayTz, setDisplayTzSignal] = createSignal<string>(read(TZ_KEY) || "Asia/Singapore");
const [resolvedTheme, setResolvedTheme] = createSignal<"light" | "dark">("light");
/** Bumped whenever colours change so charts re-read CSS tokens. */
const [paletteVersion, setPaletteVersion] = createSignal(0);

export { themePref, upDown, displayTz, resolvedTheme, paletteVersion };

function systemDark(): boolean {
  return typeof window !== "undefined" && window.matchMedia?.("(prefers-color-scheme: dark)").matches;
}

export function applyPrefs() {
  if (typeof document === "undefined") return;
  const pref = themePref();
  const theme: "light" | "dark" = pref === "auto" ? (systemDark() ? "dark" : "light") : pref;
  setResolvedTheme(theme);
  const root = document.documentElement;
  root.dataset.theme = theme;
  root.dataset.themePref = themePref();
  root.dataset.updown = upDown();
  setPaletteVersion((v) => v + 1);
}

export function setThemePref(value: ThemePref) {
  setThemePrefSignal(value);
  write(THEME_KEY, value);
  applyPrefs();
}

export function setUpDown(value: UpDown) {
  setUpDownSignal(value);
  write(UPDOWN_KEY, value);
  applyPrefs();
}

export function setDisplayTz(value: string) {
  setDisplayTzSignal(value);
  write(TZ_KEY, value);
}

/** Adopts the server's display default unless this browser already chose one. */
export function adoptServerDisplay(timezone: string | undefined, upColor: string | undefined) {
  if (timezone && !read(TZ_KEY)) setDisplayTzSignal(timezone);
  if (upColor && !read(UPDOWN_KEY)) {
    setUpDownSignal(upColor === "red_up" ? "red-up" : "green-up");
    applyPrefs();
  }
}

export function initPrefs() {
  applyPrefs();
  window.matchMedia?.("(prefers-color-scheme: dark)").addEventListener?.("change", () => {
    if (themePref() === "auto") applyPrefs();
  });
}
