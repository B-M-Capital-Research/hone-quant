import { locale } from "@/i18n";
import type { Sector } from "@/lib/types";
import { SECTOR_SHORT } from "@/i18n/overview";

/** Calls `fn` at most once per `ms`, trailing call included, so bursts of events coalesce. */
export function throttle(fn: () => void, ms: number): () => void {
  let last = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  return () => {
    const now = Date.now();
    const wait = last + ms - now;
    if (wait <= 0) {
      last = now;
      fn();
    } else if (!timer) {
      timer = setTimeout(() => {
        timer = null;
        last = Date.now();
        fn();
      }, wait);
    }
  };
}

export function sectorLabel(sectors: Sector[] | undefined, id: string, short = false): string {
  if (short && SECTOR_SHORT[id]) return SECTOR_SHORT[id][locale()];
  const sector = sectors?.find((s) => s.id === id);
  if (!sector) return SECTOR_SHORT[id]?.[locale()] ?? id;
  return locale() === "zh" ? sector.name_zh || sector.name_en : sector.name_en || sector.name_zh;
}

const WEEKDAYS = {
  zh: ["周日", "周一", "周二", "周三", "周四", "周五", "周六"],
  en: ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"],
};

/** Weekday of a YYYY-MM-DD trading date (calendar date, no time zone shift). */
export function weekday(date: string): string {
  const [y, m, d] = date.split("-").map(Number);
  return WEEKDAYS[locale()][new Date(Date.UTC(y, m - 1, d)).getUTCDay()];
}

/** Persisted UI choice (per browser). */
export function stored<T extends string>(key: string, fallback: T, allowed: readonly T[]): T {
  try {
    const value = localStorage.getItem(`hone-quant.${key}`) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  } catch {
    return fallback;
  }
}

export function store(key: string, value: string) {
  try {
    localStorage.setItem(`hone-quant.${key}`, value);
  } catch {
    /* storage unavailable */
  }
}
