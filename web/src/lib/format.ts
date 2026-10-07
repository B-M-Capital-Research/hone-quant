/**
 * Number, money, percentage and time formatting. Every function is locale-aware through the
 * locale signal and safe for null/undefined/NaN (rendered as an em dash).
 */
import { locale } from "@/i18n";
import { displayTz } from "@/lib/prefs";

export const DASH = "—";
export const MARKET_TZ = "America/New_York";

type Num = number | string | null | undefined;

export function toNumber(value: Num): number | null {
  if (value === null || value === undefined || value === "") return null;
  const n = typeof value === "number" ? value : Number(value);
  return Number.isFinite(n) ? n : null;
}

/** Accepts full-width digits and separators typed through a Chinese IME, and thousands separators. */
export function normalizeNumText(text: string): string {
  return text
    .replace(/[０-９]/g, (c) => String.fromCharCode(c.charCodeAt(0) - 0xfee0))
    .replace(/[．。]/g, ".")
    .replace(/[－—–]/g, "-")
    .replace(/[,\s_，]/g, "");
}

function intl(): string {
  return locale() === "zh" ? "zh-CN" : "en-US";
}

export function fmtNum(value: Num, dp = 2): string {
  const n = toNumber(value);
  if (n === null) return DASH;
  return n.toLocaleString(intl(), { minimumFractionDigits: dp, maximumFractionDigits: dp });
}

/** Share quantities: integers without decimals, fractional shares with up to 4. */
export function fmtQty(value: Num): string {
  const n = toNumber(value);
  if (n === null) return DASH;
  const isInt = Math.abs(n - Math.round(n)) < 1e-9;
  return n.toLocaleString(intl(), { minimumFractionDigits: 0, maximumFractionDigits: isInt ? 0 : 4 });
}

export function fmtMoney(value: Num, opts: { dp?: number; sign?: boolean; compact?: boolean } = {}): string {
  const n = toNumber(value);
  if (n === null) return DASH;
  // The sign follows the printed value: −0.40 shown without decimals is "$0", not "−$0".
  const shown = Number(Math.abs(n).toFixed(opts.compact ? 2 : opts.dp ?? 2));
  const sign = shown === 0 ? "" : n < 0 ? "−" : opts.sign && n > 0 ? "+" : "";
  const abs = Math.abs(n);
  if (opts.compact) {
    if (locale() === "zh") {
      if (abs >= 1e8) return `${sign}$${(abs / 1e8).toFixed(2)}亿`;
      if (abs >= 1e4) return `${sign}$${(abs / 1e4).toFixed(abs >= 1e6 ? 1 : 2)}万`;
    } else {
      if (abs >= 1e9) return `${sign}$${(abs / 1e9).toFixed(2)}B`;
      if (abs >= 1e6) return `${sign}$${(abs / 1e6).toFixed(2)}M`;
      if (abs >= 1e4) return `${sign}$${(abs / 1e3).toFixed(1)}K`;
    }
  }
  const dp = opts.dp ?? 2;
  return `${sign}$${abs.toLocaleString("en-US", { minimumFractionDigits: dp, maximumFractionDigits: dp })}`;
}

export function fmtPrice(value: Num): string {
  const n = toNumber(value);
  if (n === null) return DASH;
  return n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: n < 1 ? 4 : 2 });
}

/** Direction of a money amount as printed with `dp` decimals. */
export function moneyPolarity(value: Num, dp = 2): "up" | "down" | "flat" {
  const n = toNumber(value);
  if (n === null) return "flat";
  const shown = Number(n.toFixed(dp));
  return shown === 0 ? "flat" : shown > 0 ? "up" : "down";
}

/** Fraction → percent. `sign` adds "+" to positive values (minus is always shown). */
export function fmtPct(value: Num, opts: { dp?: number; sign?: boolean } = {}): string {
  const n = toNumber(value);
  if (n === null) return DASH;
  const dp = opts.dp ?? 2;
  const pct = n * 100;
  const rounded = Number(pct.toFixed(dp));
  const sign = rounded < 0 ? "−" : opts.sign && rounded > 0 ? "+" : "";
  return `${sign}${Math.abs(rounded).toFixed(dp)}%`;
}

/** Weight points: 0.0123 → "1.23%" without sign. */
export function fmtWeight(value: Num, dp = 1): string {
  return fmtPct(value, { dp, sign: false });
}

/**
 * Direction of a value for colouring. With `dp`, the value is treated as a fraction shown as a
 * percentage with that many decimals, so a value that prints as "0.00%" is never coloured.
 */
export function polarity(value: Num, dp?: number): "up" | "down" | "flat" {
  const n = toNumber(value);
  if (n === null) return "flat";
  const shown = dp === undefined ? n : Number((n * 100).toFixed(dp));
  if (Math.abs(shown) < 1e-12) return "flat";
  return shown > 0 ? "up" : "down";
}

function toDate(value: string | Date | number | null | undefined): Date | null {
  if (value === null || value === undefined || value === "") return null;
  const d = value instanceof Date ? value : new Date(value);
  return Number.isNaN(d.getTime()) ? null : d;
}

/** Short label for a time zone ("SGT", "ET", …). */
export function zoneLabel(tz: string): string {
  switch (tz) {
    case MARKET_TZ:
      return "ET";
    case "Asia/Singapore":
      return "SGT";
    case "Asia/Shanghai":
    case "Asia/Hong_Kong":
    case "Asia/Taipei":
      return locale() === "zh" ? "北京" : "CST";
    case "Asia/Tokyo":
      return "JST";
    case "Europe/London":
      return "UK";
    case "UTC":
      return "UTC";
    default:
      return tz.split("/").pop()?.replace(/_/g, " ") ?? tz;
  }
}

export function fmtTime(value: string | Date | number | null | undefined, tz: string = displayTz(), seconds = false): string {
  const d = toDate(value);
  if (!d) return DASH;
  return d.toLocaleTimeString("en-GB", {
    timeZone: tz,
    hour: "2-digit",
    minute: "2-digit",
    second: seconds ? "2-digit" : undefined,
    hour12: false,
  });
}

export function fmtDate(value: string | Date | number | null | undefined, tz?: string): string {
  if (typeof value === "string" && /^\d{4}-\d{2}-\d{2}$/.test(value)) return value;
  const d = toDate(value);
  if (!d) return DASH;
  return d.toLocaleDateString("en-CA", { timeZone: tz ?? displayTz(), year: "numeric", month: "2-digit", day: "2-digit" });
}

export function fmtDateTime(value: string | Date | number | null | undefined, tz: string = displayTz()): string {
  const d = toDate(value);
  if (!d) return DASH;
  return `${fmtDate(d, tz)} ${fmtTime(d, tz)}`;
}

/**
 * "22:00 SGT · 10:00 ET": the local zone first, then New York. `withDate: true` prefixes both
 * with MM-DD; "auto" prefixes the local time only when its calendar date differs from New York's
 * (e.g. a 13:00 ET slot is "10-06 01:00 SGT · 13:00 ET").
 */
export function fmtDual(value: string | Date | number | null | undefined, withDate: boolean | "auto" = false): string {
  const d = toDate(value);
  if (!d) return DASH;
  const local = displayTz();
  const localDate = fmtDate(d, local);
  const marketDate = fmtDate(d, MARKET_TZ);
  const showLocalDate = withDate === true || (withDate === "auto" && localDate !== marketDate);
  const left = `${showLocalDate ? localDate.slice(5) + " " : ""}${fmtTime(d, local)} ${zoneLabel(local)}`;
  if (local === MARKET_TZ) return left;
  const right = `${withDate === true ? marketDate.slice(5) + " " : ""}${fmtTime(d, MARKET_TZ)} ET`;
  return `${left} · ${right}`;
}

export function fmtCountdown(ms: number): string {
  if (!Number.isFinite(ms)) return DASH;
  const total = Math.max(0, Math.round(ms / 1000));
  const d = Math.floor(total / 86400);
  const h = Math.floor((total % 86400) / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const zh = locale() === "zh";
  if (d > 0) return zh ? `${d}天${h}小时` : `${d}d ${h}h`;
  if (h > 0) return zh ? `${h}小时${m}分` : `${h}h ${m}m`;
  if (m > 0) return zh ? `${m}分${s.toString().padStart(2, "0")}秒` : `${m}m ${s.toString().padStart(2, "0")}s`;
  return zh ? `${s}秒` : `${s}s`;
}

export function fmtRelative(value: string | Date | number | null | undefined, now = Date.now()): string {
  const d = toDate(value);
  if (!d) return DASH;
  const diff = now - d.getTime();
  const zh = locale() === "zh";
  const abs = Math.abs(diff);
  const future = diff < 0;
  const unit = (n: number, zhUnit: string, enUnit: string) =>
    zh ? `${n}${zhUnit}${future ? "后" : "前"}` : future ? `in ${n}${enUnit}` : `${n}${enUnit} ago`;
  if (abs < 60_000) return zh ? "刚刚" : "just now";
  if (abs < 3_600_000) return unit(Math.round(abs / 60_000), "分钟", "m");
  if (abs < 86_400_000) return unit(Math.round(abs / 3_600_000), "小时", "h");
  if (abs < 30 * 86_400_000) return unit(Math.round(abs / 86_400_000), "天", "d");
  return fmtDate(d);
}

/** Today's date in New York (the trading date). */
export function marketToday(now = new Date()): string {
  return fmtDate(now, MARKET_TZ);
}
