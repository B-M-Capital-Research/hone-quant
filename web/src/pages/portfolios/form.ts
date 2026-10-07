/**
 * Client-side checks for the portfolio dialogs, mirroring the server's rules: names of 1–60
 * characters and initial cash between $1,000 and $10,000,000,000 with at most two decimals.
 * Plus the dialogs' shared focus handling.
 */
import { normalizeNumText } from "@/lib/format";

export const NAME_MAX = 60;
export const MIN_CASH = 1_000;
export const MAX_CASH = 10_000_000_000;
export const DEFAULT_CASH = 1_000_000;

export type NameProblem = "required" | "too_long";

/** Length is counted in characters (code points), as the server does. */
export function nameProblem(name: string): NameProblem | null {
  const trimmed = name.trim();
  if (!trimmed) return "required";
  return [...trimmed].length > NAME_MAX ? "too_long" : null;
}

/** Typed initial cash (thousands separators and full-width digits allowed); null when invalid or out of range. */
export function parseCash(text: string): number | null {
  const raw = normalizeNumText(text);
  if (!/^\d+(\.\d{1,2})?$/.test(raw)) return null;
  const value = Number(raw);
  return value >= MIN_CASH && value <= MAX_CASH ? value : null;
}

/** "1,000,000" — the form's display of an amount. */
export function formatCash(value: number): string {
  return value.toLocaleString("en-US", { maximumFractionDigits: 2 });
}

/** Moves focus to the first field of a portfolio form flagged invalid. */
export function focusInvalid() {
  requestAnimationFrame(() => document.querySelector<HTMLElement>(".pf-form [aria-invalid='true']")?.focus());
}
