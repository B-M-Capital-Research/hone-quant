/**
 * Small helpers shared by the activity pages: query-string access, date arithmetic on
 * `YYYY-MM-DD` strings, paged "fetch everything" with a cap, and debouncing.
 */
import { toNumber } from "@/lib/format";

/** First value of a router search param ("" when absent). */
export function qstr(value: string | string[] | undefined): string {
  if (Array.isArray(value)) return value[0] ?? "";
  return value ?? "";
}

export const isDate = (value: string) => /^\d{4}-\d{2}-\d{2}$/.test(value);

/** Adds `days` to a `YYYY-MM-DD` date. */
export function addDays(date: string, days: number): string {
  const [y, m, d] = date.split("-").map(Number);
  const t = new Date(Date.UTC(y, m - 1, d + days));
  return t.toISOString().slice(0, 10);
}

/** ISO weekday (1 = Monday … 7 = Sunday) of a `YYYY-MM-DD` date. */
export function isoWeekday(date: string): number {
  const [y, m, d] = date.split("-").map(Number);
  const day = new Date(Date.UTC(y, m - 1, d)).getUTCDay();
  return day === 0 ? 7 : day;
}

/** `n` with a fallback of 0 for missing/invalid decimals. */
export function num(value: string | number | null | undefined): number {
  return toNumber(value) ?? 0;
}

export function debounce<A extends unknown[]>(fn: (...args: A) => void, ms: number): ((...args: A) => void) & { cancel: () => void } {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const wrapped = (...args: A) => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      fn(...args);
    }, ms);
  };
  wrapped.cancel = () => {
    if (timer) clearTimeout(timer);
    timer = null;
  };
  return wrapped;
}

export interface Page<T> {
  rows: T[];
  total: number;
}

export interface AllRows<T> {
  rows: T[];
  total: number;
  truncated: boolean;
}

/**
 * Fetches every page of a paginated list (newest first) up to `cap` rows, a few requests at a
 * time. Rows are de-duplicated by id because new rows can arrive at the top while paging.
 */
export async function fetchAll<T extends { id: number }>(
  fetchPage: (offset: number, limit: number) => Promise<Page<T>>,
  pageSize: number,
  cap: number,
  concurrency = 4,
): Promise<AllRows<T>> {
  const first = await fetchPage(0, pageSize);
  const target = Math.min(first.total, cap);
  const offsets: number[] = [];
  for (let offset = pageSize; offset < target; offset += pageSize) offsets.push(offset);
  const pages: T[][] = new Array(offsets.length);
  let cursor = 0;
  const worker = async () => {
    while (cursor < offsets.length) {
      const index = cursor++;
      pages[index] = (await fetchPage(offsets[index], pageSize)).rows;
    }
  };
  await Promise.all(Array.from({ length: Math.min(concurrency, offsets.length) }, worker));
  const seen = new Set<number>();
  const rows: T[] = [];
  for (const row of [first.rows, ...pages].flat()) {
    if (seen.has(row.id)) continue;
    seen.add(row.id);
    rows.push(row);
    if (rows.length >= target) break;
  }
  return { rows, total: first.total, truncated: first.total > cap };
}

/** Today's date in a time zone as `YYYY-MM-DD`. */
export function dateIn(ms: number, tz: string): string {
  return new Date(ms).toLocaleDateString("en-CA", { timeZone: tz, year: "numeric", month: "2-digit", day: "2-digit" });
}

/** Offset (ms) of `tz` from UTC at instant `ms`. */
function tzOffset(ms: number, tz: string): number {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone: tz,
    hourCycle: "h23",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).formatToParts(new Date(ms));
  const get = (type: string) => Number(parts.find((p) => p.type === type)?.value ?? 0);
  const asUtc = Date.UTC(get("year"), get("month") - 1, get("day"), get("hour") % 24, get("minute"), get("second"));
  return asUtc - Math.floor(ms / 1000) * 1000;
}

/** Wall-clock `date` + `time` ("HH:MM") in `tz` → UTC instant (ms). */
export function zonedToUtc(date: string, time: string, tz: string): number | null {
  if (!isDate(date) || !/^\d{2}:\d{2}$/.test(time)) return null;
  const [y, m, d] = date.split("-").map(Number);
  const [hh, mm] = time.split(":").map(Number);
  const wall = Date.UTC(y, m - 1, d, hh, mm);
  try {
    let guess = wall - tzOffset(wall, tz);
    guess = wall - tzOffset(guess, tz);
    return guess;
  } catch {
    return null;
  }
}

/** UTC instant → wall-clock `{ date, time }` in `tz`. */
export function utcToZoned(ms: number, tz: string): { date: string; time: string } {
  const date = dateIn(ms, tz);
  const time = new Date(ms).toLocaleTimeString("en-GB", { timeZone: tz, hour: "2-digit", minute: "2-digit", hour12: false });
  return { date, time };
}

/** Milliseconds as "850 ms", "4.2 s", "3 min 05 s" or "1 h 02 min". */
export function fmtDuration(ms: number | null, zh: boolean): string {
  if (ms === null || !Number.isFinite(ms) || ms < 0) return "—";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  const s = ms / 1000;
  if (s < 60) return zh ? `${s.toFixed(s < 10 ? 1 : 0)} 秒` : `${s.toFixed(s < 10 ? 1 : 0)} s`;
  const total = Math.round(s);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = total % 60;
  if (h > 0) return zh ? `${h} 小时 ${String(m).padStart(2, "0")} 分` : `${h} h ${String(m).padStart(2, "0")} min`;
  return zh ? `${m} 分 ${String(sec).padStart(2, "0")} 秒` : `${m} min ${String(sec).padStart(2, "0")} s`;
}
