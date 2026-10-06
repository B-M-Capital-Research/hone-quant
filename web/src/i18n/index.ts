/**
 * Bilingual UI (简体中文 / English).
 *
 * Chinese is the canonical dictionary and English is type-checked against it, so a key can
 * never exist in one language only. Dictionaries are split per page (`defineMessages`) and read
 * through an accessor, which makes every JSX read reactive to the locale signal.
 */
import { createSignal } from "solid-js";

export type Locale = "zh" | "en";

const STORAGE_KEY = "hone-quant.locale";

function detect(): Locale {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "zh" || stored === "en") return stored;
  } catch {
    /* storage unavailable */
  }
  return typeof navigator !== "undefined" && navigator.language?.toLowerCase().startsWith("zh") ? "zh" : "en";
}

const [locale, setLocaleSignal] = createSignal<Locale>(detect());

export { locale };

export function applyLocale(value: Locale) {
  if (typeof document !== "undefined") {
    document.documentElement.lang = value === "zh" ? "zh-CN" : "en";
  }
}

export function setLocale(value: Locale) {
  setLocaleSignal(value);
  try {
    localStorage.setItem(STORAGE_KEY, value);
  } catch {
    /* storage unavailable */
  }
  applyLocale(value);
}

/** Widens string literals so the English tree only has to match the Chinese tree's shape. */
export type Messages<T> = T extends string
  ? string
  : T extends (...args: infer A) => infer R
    ? (...args: A) => R
    : T extends readonly (infer U)[]
      ? readonly Messages<U>[]
      : { readonly [K in keyof T]: Messages<T[K]> };

export function defineMessages<T extends object>(zh: T, en: Messages<T>): () => Messages<T> {
  return () => (locale() === "zh" ? (zh as unknown as Messages<T>) : en);
}

/** Fills `{name}` placeholders. */
export function tpl(template: string, vars: Record<string, string | number | undefined | null>): string {
  return template.replace(/\{(\w+)\}/g, (_, key: string) => {
    const value = vars[key];
    return value === undefined || value === null ? "" : String(value);
  });
}

/** Picks `${field}_zh` or `${field}_en` from an API row. */
export function pick(row: object | undefined | null, field: string): string {
  if (!row) return "";
  const record = row as Record<string, unknown>;
  const zh = record[`${field}_zh`];
  const en = record[`${field}_en`];
  const value = locale() === "zh" ? zh || en : en || zh;
  return typeof value === "string" ? value : "";
}
