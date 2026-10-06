/**
 * Numeric form fields: text in display units ↔ the value the server stores.
 *
 * A field is described once (range in display units, integer or not, and the divisor that turns
 * the display value into the stored one — 100 for percentages, 1e6 for "$ per million"), and the
 * same description drives parsing, client-side range validation (mirroring the server's checks),
 * hints and error messages.
 */
import { tpl } from "@/i18n";
import { settingsText } from "@/i18n/settings";

export interface NumSpec {
  min: number;
  max: number;
  int?: boolean;
  /** stored = display / div (default 1). */
  div?: number;
  /** Unit appended to range messages, e.g. "%", " min". */
  unit?: () => string;
  /** Currency prefix shown before both bounds, e.g. "$". */
  prefix?: string;
}

/** Accepts full-width digits and separators typed through a Chinese IME, and thousands separators. */
export function normalizeNumText(text: string): string {
  return text
    .replace(/[０-９]/g, (c) => String.fromCharCode(c.charCodeAt(0) - 0xfee0))
    .replace(/[．。]/g, ".")
    .replace(/[－—–]/g, "-")
    .replace(/[,\s_，]/g, "");
}

/** Rounds away binary noise (0.07 * 100 = 7.000000000000001). */
export function clean(n: number): number {
  return Number(n.toPrecision(12));
}

export function fmtPlain(n: number): string {
  if (!Number.isFinite(n)) return "";
  return String(clean(n));
}

/** Display text for a stored value. */
export function toText(stored: number | null | undefined, spec: NumSpec): string {
  if (stored === null || stored === undefined || !Number.isFinite(stored)) return "";
  return fmtPlain(stored * (spec.div ?? 1));
}

export function rangeText(spec: NumSpec): { min: string; max: string; unit: string } {
  const n = (v: number) => `${spec.prefix ?? ""}${v.toLocaleString("en-US", { maximumFractionDigits: 6 })}`;
  return { min: n(spec.min), max: n(spec.max), unit: spec.unit?.() ?? "" };
}

export function rangeMessage(spec: NumSpec): string {
  const t = settingsText();
  return tpl(spec.int ? t.v.range_int : t.v.range, rangeText(spec));
}

export type NumResult = { ok: true; display: number; stored: number } | { ok: false; error: string };

export function parseNum(text: string, spec: NumSpec, opts: { required?: boolean } = {}): NumResult {
  const t = settingsText();
  const raw = normalizeNumText(text);
  if (!raw) return { ok: false, error: opts.required === false ? "" : t.v.required };
  if (!/^-?(\d+\.?\d*|\.\d+)$/.test(raw)) return { ok: false, error: t.v.number };
  const display = Number(raw);
  if (!Number.isFinite(display)) return { ok: false, error: t.v.number };
  if (spec.int && !Number.isInteger(display)) return { ok: false, error: t.v.integer };
  const eps = 1e-9 * Math.max(1, Math.abs(spec.max));
  if (display < spec.min - eps || display > spec.max + eps) return { ok: false, error: rangeMessage(spec) };
  const div = spec.div ?? 1;
  const lo = spec.min / div;
  const hi = spec.max / div;
  const stored = Math.min(Math.max(clean(display / div), lo), hi);
  return { ok: true, display, stored };
}

/** Same numeric value (after normalisation), so "30" and "30.0" are not a change. */
export function sameNumText(a: string, b: string): boolean {
  const na = normalizeNumText(a);
  const nb = normalizeNumText(b);
  if (na === nb) return true;
  const x = Number(na);
  const y = Number(nb);
  return na !== "" && nb !== "" && Number.isFinite(x) && Number.isFinite(y) && Math.abs(x - y) < 1e-12;
}
