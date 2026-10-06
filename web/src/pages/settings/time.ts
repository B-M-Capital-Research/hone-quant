/**
 * Time-zone arithmetic for the settings pages: wall-clock ↔ instant conversion in any IANA zone,
 * the exchange's early-close rule and a client-side replica of the server's daily plan schedule
 * (`quant_core::schedule::day_schedule`), so edits can be previewed before they are saved.
 */
import type { ScheduleSettings } from "@/lib/types";

export const NY = "America/New_York";

/** Width of each plan window and the execution cutoff before the close (server constants). */
export const SLOT_WINDOW_MINUTES = 180;
export const EXECUTION_CUTOFF_MINUTES = 5;

const formatters = new Map<string, Intl.DateTimeFormat>();

function partsFormatter(tz: string): Intl.DateTimeFormat {
  let f = formatters.get(tz);
  if (!f) {
    f = new Intl.DateTimeFormat("en-US", {
      timeZone: tz,
      hourCycle: "h23",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
    formatters.set(tz, f);
  }
  return f;
}

export interface WallTime {
  y: number;
  m: number;
  d: number;
  hh: number;
  mm: number;
  ss: number;
}

export function wallParts(ms: number, tz: string): WallTime {
  const parts = partsFormatter(tz).formatToParts(new Date(ms));
  const get = (type: Intl.DateTimeFormatPartTypes) => Number(parts.find((p) => p.type === type)?.value ?? 0);
  return { y: get("year"), m: get("month"), d: get("day"), hh: get("hour") % 24, mm: get("minute"), ss: get("second") };
}

const pad = (n: number) => String(n).padStart(2, "0");

export function wallDate(ms: number, tz: string): string {
  const p = wallParts(ms, tz);
  return `${p.y}-${pad(p.m)}-${pad(p.d)}`;
}

function offsetMs(ms: number, tz: string): number {
  const p = wallParts(ms, tz);
  return Date.UTC(p.y, p.m - 1, p.d, p.hh, p.mm, p.ss) - Math.floor(ms / 1000) * 1000;
}

/** The instant at which the wall clock in `tz` shows `date` (YYYY-MM-DD) `time` (HH:MM). */
export function zonedToUtc(date: string, time: string, tz: string): number | null {
  const dm = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  const tm = /^(\d{1,2}):(\d{2})$/.exec(time);
  if (!dm || !tm) return null;
  const wall = Date.UTC(Number(dm[1]), Number(dm[2]) - 1, Number(dm[3]), Number(tm[1]), Number(tm[2]));
  try {
    let guess = wall - offsetMs(wall, tz);
    guess = wall - offsetMs(guess, tz);
    return guess;
  } catch {
    return null;
  }
}

/** `<input type="datetime-local">` value for an instant shown in `tz`. */
export function toLocalInput(ms: number, tz: string): string {
  const p = wallParts(ms, tz);
  return `${p.y}-${pad(p.m)}-${pad(p.d)}T${pad(p.hh)}:${pad(p.mm)}`;
}

export function fromLocalInput(value: string, tz: string): number | null {
  const [date, time] = value.split("T");
  if (!date || !time) return null;
  return zonedToUtc(date, time.slice(0, 5), tz);
}

/** The canonical IANA name when the browser knows the zone, otherwise null. */
export function canonicalTimeZone(value: string): string | null {
  const tz = value.trim();
  if (!tz) return null;
  try {
    return new Intl.DateTimeFormat("en-US", { timeZone: tz }).resolvedOptions().timeZone;
  } catch {
    return null;
  }
}

/** Offset label such as "UTC+8" or "UTC−4" for the given instant. */
export function utcOffsetLabel(tz: string, at = Date.now()): string {
  try {
    const minutes = Math.round(offsetMs(at, tz) / 60_000);
    if (minutes === 0) return "UTC";
    const sign = minutes > 0 ? "+" : "−";
    const abs = Math.abs(minutes);
    return `UTC${sign}${Math.floor(abs / 60)}${abs % 60 ? `:${pad(abs % 60)}` : ""}`;
  } catch {
    return "";
  }
}

function weekday(y: number, m: number, d: number): number {
  return new Date(Date.UTC(y, m - 1, d)).getUTCDay();
}

/** The exchange's rule-based 13:00 early closes in `year` (mirrors `rule_early_close`). */
export function earlyCloses(year: number): string[] {
  const out: string[] = [];
  const monToThu = (dow: number) => dow >= 1 && dow <= 4;
  if (monToThu(weekday(year, 7, 3))) out.push(`${year}-07-03`);
  const firstThursday = 1 + ((4 - weekday(year, 11, 1) + 7) % 7);
  out.push(`${year}-11-${pad(firstThursday + 21 + 1)}`);
  if (monToThu(weekday(year, 12, 24))) out.push(`${year}-12-24`);
  return out.sort();
}

export function nextEarlyClose(from: string): string {
  const year = Number(from.slice(0, 4));
  return [...earlyCloses(year), ...earlyCloses(year + 1)].find((d) => d >= from) ?? `${year + 1}-11-27`;
}

export interface SessionTimes {
  date: string;
  open: number;
  close: number;
  early_close: boolean;
}

export function sessionOn(date: string, early: boolean): SessionTimes | null {
  const open = zonedToUtc(date, "09:30", NY);
  const close = zonedToUtc(date, early ? "13:00" : "16:00", NY);
  if (open === null || close === null) return null;
  return { date, open, close, early_close: early };
}

export interface SlotPreview {
  slot: "open" | "close";
  window_start: number;
  window_end: number;
  generate_at: number;
  execute_at: number;
  deadline: number;
}

export interface DayPreview {
  session: SessionTimes;
  slots: SlotPreview[];
  /** Why the pre-close plan does not run on this session, if it does not. */
  closeSkipped: null | "gap" | "deadline";
  openSkipped: boolean;
}

const MIN = 60_000;

/** Same rules as `quant_core::schedule::day_schedule`. */
export function daySchedule(session: SessionTimes, s: ScheduleSettings): DayPreview {
  const window = SLOT_WINDOW_MINUTES * MIN;
  const review = s.review_minutes * MIN;
  const deadline = session.close - EXECUTION_CUTOFF_MINUTES * MIN;
  const openGenerate = session.open + s.open_offset_minutes * MIN;
  const closeGenerate = session.close - s.close_offset_minutes * MIN;
  const slots: SlotPreview[] = [];
  const openRuns = openGenerate < deadline;
  if (openRuns) {
    slots.push({
      slot: "open",
      window_start: session.open,
      window_end: Math.min(session.open + window, session.close),
      generate_at: openGenerate,
      execute_at: Math.min(openGenerate + review, deadline),
      deadline,
    });
  }
  const gapOk = closeGenerate >= openGenerate + s.min_gap_minutes * MIN;
  let closeSkipped: DayPreview["closeSkipped"] = null;
  if (!gapOk) closeSkipped = "gap";
  else if (closeGenerate >= deadline) closeSkipped = "deadline";
  else {
    slots.push({
      slot: "close",
      window_start: Math.max(session.close - window, session.open),
      window_end: session.close,
      generate_at: closeGenerate,
      execute_at: Math.min(closeGenerate + review, deadline),
      deadline,
    });
  }
  return { session, slots, closeSkipped, openSkipped: !openRuns };
}
