/**
 * Strategy parameter metadata: every field the engine accepts, with its group, kind and the
 * exact validation range from `quant-core/src/strategy.rs` (`StrategyParams::validate`), so the
 * form can reject what the server would reject before a request is made.
 */
import type { FieldError, StrategyParams } from "@/lib/types";

export type GroupId = "universe" | "exposure" | "sector" | "asset" | "rebalance";

/**
 * - `int`: whole trading days; `pct`: fraction shown as a percentage; `pp`: fraction shown as
 *   percentage points; `num`: plain decimal (exponents); `mult`: multiplier; `money`: USD;
 * - `optpct`: nullable percentage (volatility target); `bool`; `method`; `budgets` (map).
 */
export type FieldKind = "int" | "pct" | "pp" | "num" | "mult" | "money" | "optpct" | "bool" | "method" | "budgets";

export interface FieldDef {
  path: string;
  group: GroupId;
  kind: FieldKind;
  /** Inclusive range in stored units. */
  min?: number;
  max?: number;
  /** For `int`: 0 is allowed and means "off" (the trend filter). */
  zeroOff?: boolean;
}

export const GROUPS: GroupId[] = ["universe", "exposure", "sector", "asset", "rebalance"];

export const FIELDS: FieldDef[] = [
  { path: "universe.min_history_days", group: "universe", kind: "int", min: 20, max: 756 },

  { path: "exposure.max_exposure", group: "exposure", kind: "pct", min: 0, max: 1 },
  { path: "exposure.min_exposure", group: "exposure", kind: "pct", min: 0, max: 1 },
  { path: "exposure.breadth_scaling", group: "exposure", kind: "bool" },
  { path: "exposure.breadth_sma", group: "exposure", kind: "int", min: 10, max: 400 },
  { path: "exposure.target_vol", group: "exposure", kind: "optpct", min: 0.03, max: 1.5 },
  { path: "exposure.vol_lookback", group: "exposure", kind: "int", min: 10, max: 504 },

  { path: "sector.method", group: "sector", kind: "method" },
  { path: "sector.vol_power", group: "sector", kind: "num", min: 0, max: 3 },
  { path: "sector.vol_lookback", group: "sector", kind: "int", min: 10, max: 504 },
  { path: "sector.momentum_tilt", group: "sector", kind: "pct", min: 0, max: 0.95 },
  { path: "sector.momentum_lookback", group: "sector", kind: "int", min: 10, max: 504 },
  { path: "sector.momentum_skip", group: "sector", kind: "int", min: 0, max: 63 },
  { path: "sector.trend_aware", group: "sector", kind: "bool" },
  { path: "sector.min_weight", group: "sector", kind: "pct", min: 0, max: 0.2 },
  { path: "sector.max_weight", group: "sector", kind: "pct", min: 0.02, max: 1 },
  { path: "sector.custom_budgets", group: "sector", kind: "budgets", min: 0, max: 100 },

  { path: "asset.vol_power", group: "asset", kind: "num", min: 0, max: 3 },
  { path: "asset.vol_lookback", group: "asset", kind: "int", min: 10, max: 504 },
  { path: "asset.momentum_tilt", group: "asset", kind: "pct", min: 0, max: 0.95 },
  { path: "asset.momentum_lookback", group: "asset", kind: "int", min: 10, max: 504 },
  { path: "asset.momentum_skip", group: "asset", kind: "int", min: 0, max: 63 },
  { path: "asset.trend_sma", group: "asset", kind: "int", min: 10, max: 400, zeroOff: true },
  { path: "asset.trend_penalty", group: "asset", kind: "mult", min: 0, max: 1 },
  { path: "asset.trend_ramp", group: "asset", kind: "pct", min: 0, max: 0.5 },
  { path: "asset.min_weight", group: "asset", kind: "pct", min: 0, max: 0.05 },
  { path: "asset.max_weight", group: "asset", kind: "pct", min: 0.005, max: 0.5 },

  { path: "rebalance.band_abs", group: "rebalance", kind: "pp", min: 0, max: 0.2 },
  { path: "rebalance.band_rel", group: "rebalance", kind: "pct", min: 0, max: 1 },
  { path: "rebalance.min_trade_value", group: "rebalance", kind: "money", min: 0, max: 1_000_000 },
  { path: "rebalance.max_turnover", group: "rebalance", kind: "pct", min: 0.01, max: 2 },
  { path: "rebalance.fractional_shares", group: "rebalance", kind: "bool" },
];

export const FIELD_BY_PATH: Record<string, FieldDef> = Object.fromEntries(FIELDS.map((f) => [f.path, f]));

export function fieldsOf(group: GroupId): FieldDef[] {
  return FIELDS.filter((f) => f.group === group);
}

// ---------------------------------------------------------------------------------------------
// Access
// ---------------------------------------------------------------------------------------------

export function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

export function getPath(params: StrategyParams, path: string): unknown {
  let node: unknown = params;
  for (const key of path.split(".")) {
    if (node === null || node === undefined || typeof node !== "object") return undefined;
    node = (node as Record<string, unknown>)[key];
  }
  return node;
}

/** Returns a copy of `params` with `path` set (copies only along the path). */
export function setPath(params: StrategyParams, path: string, value: unknown): StrategyParams {
  const keys = path.split(".");
  const root: Record<string, unknown> = { ...(params as unknown as Record<string, unknown>) };
  let node = root;
  for (let i = 0; i < keys.length - 1; i++) {
    const next = { ...((node[keys[i]] as Record<string, unknown>) ?? {}) };
    node[keys[i]] = next;
    node = next;
  }
  node[keys[keys.length - 1]] = value;
  return root as unknown as StrategyParams;
}

/** Normalises older or partial parameter objects onto the full shape (server defaults fill gaps). */
export function withDefaults(params: Partial<StrategyParams> | null | undefined, defaults: StrategyParams): StrategyParams {
  const out = clone(defaults);
  if (!params) return out;
  for (const group of GROUPS) {
    const source = (params as Record<string, unknown>)[group];
    if (source && typeof source === "object") {
      Object.assign((out as unknown as Record<string, Record<string, unknown>>)[group], clone(source));
    }
  }
  return out;
}

function sameValue(a: unknown, b: unknown): boolean {
  if (typeof a === "number" && typeof b === "number") return a === b || (Number.isNaN(a) && Number.isNaN(b)) || Math.abs(a - b) < 1e-12;
  if (a && b && typeof a === "object" && typeof b === "object") {
    const ka = Object.keys(a as object).filter((k) => (a as Record<string, unknown>)[k] !== undefined);
    const kb = Object.keys(b as object).filter((k) => (b as Record<string, unknown>)[k] !== undefined);
    if (ka.length !== kb.length) return false;
    return ka.every((k) => sameValue((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
  }
  return a === b || ((a === null || a === undefined) && (b === null || b === undefined));
}

/** Paths whose values differ between two parameter sets, in display order. */
export function diffPaths(a: StrategyParams, b: StrategyParams): string[] {
  return FIELDS.filter((f) => !sameValue(getPath(a, f.path), getPath(b, f.path))).map((f) => f.path);
}

export function sameParams(a: StrategyParams, b: StrategyParams): boolean {
  return diffPaths(a, b).length === 0;
}

// ---------------------------------------------------------------------------------------------
// Validation (mirrors StrategyParams::validate)
// ---------------------------------------------------------------------------------------------

export type Issue = Pick<FieldError, "code" | "min" | "max" | "message"> & { path: string };

export function validateParams(p: StrategyParams): Issue[] {
  const issues: Issue[] = [];
  const num = (path: string): number => {
    const v = getPath(p, path);
    return typeof v === "number" ? v : Number.NaN;
  };
  const range = (path: string, min: number, max: number) => {
    const v = num(path);
    if (!Number.isFinite(v)) issues.push({ path, code: "required", message: "" });
    else if (v < min || v > max) issues.push({ path, code: "out_of_range", min, max, message: "" });
  };
  const int = (path: string, min: number, max: number) => {
    const v = num(path);
    if (!Number.isFinite(v)) issues.push({ path, code: "required", message: "" });
    else if (!Number.isInteger(v)) issues.push({ path, code: "integer", message: "" });
    else if (v < min || v > max) issues.push({ path, code: "out_of_range", min, max, message: "" });
  };

  for (const field of FIELDS) {
    const { path, kind, min = 0, max = 0 } = field;
    if (kind === "int") {
      if (field.zeroOff && num(path) === 0) continue;
      int(path, min, max);
    } else if (kind === "pct" || kind === "pp" || kind === "num" || kind === "mult" || kind === "money") {
      range(path, min, max);
    } else if (kind === "optpct") {
      const v = getPath(p, path);
      if (v !== null && v !== undefined) range(path, min, max);
    }
  }

  const rule = (path: string, code: string, applies: boolean) => {
    if (applies && !issues.some((i) => i.path === path)) issues.push({ path, code, message: "" });
  };
  rule("sector.momentum_skip", "skip_not_below_lookback", num("sector.momentum_skip") >= num("sector.momentum_lookback"));
  rule("sector.min_weight", "min_above_max", num("sector.min_weight") > num("sector.max_weight"));
  rule("asset.momentum_skip", "skip_not_below_lookback", num("asset.momentum_skip") >= num("asset.momentum_lookback"));
  rule("asset.min_weight", "min_above_max", num("asset.min_weight") > num("asset.max_weight"));
  rule("exposure.min_exposure", "min_above_max", num("exposure.min_exposure") > num("exposure.max_exposure"));

  if (p.sector.method === "custom") {
    const budgets = Object.entries(p.sector.custom_budgets ?? {});
    if (!budgets.length || budgets.every(([, v]) => !(v > 0))) {
      issues.push({ path: "sector.custom_budgets", code: "required", message: "" });
    }
    for (const [sector, value] of budgets) {
      if (!Number.isFinite(value) || value < 0 || value > 100) {
        issues.push({ path: `sector.custom_budgets.${sector}`, code: "out_of_range", min: 0, max: 100, message: "" });
      }
    }
  }
  return issues;
}

/** Maps an issue path (including `sector.custom_budgets.<id>`) to the field that shows it. */
export function fieldForIssue(path: string): FieldDef | undefined {
  if (FIELD_BY_PATH[path]) return FIELD_BY_PATH[path];
  if (path.startsWith("sector.custom_budgets")) return FIELD_BY_PATH["sector.custom_budgets"];
  return undefined;
}

/** Parameters ready to send: custom budgets without blank entries. */
export function cleanParams(p: StrategyParams): StrategyParams {
  const out = clone(p);
  const budgets: Record<string, number> = {};
  for (const [k, v] of Object.entries(out.sector.custom_budgets ?? {})) {
    if (typeof v === "number" && Number.isFinite(v)) budgets[k] = v;
  }
  out.sector.custom_budgets = budgets;
  return out;
}

// ---------------------------------------------------------------------------------------------
// Number helpers for display and inputs
// ---------------------------------------------------------------------------------------------

/** Trims float noise: 0.07 * 100 → 7, 0.005 * 100 → 0.5. */
export function tidy(value: number, dp = 6): number {
  return Number(value.toFixed(dp));
}

/** Locale-free compact decimal with up to `dp` decimals ("98", "0.5", "1.25"). */
export function compact(value: number, dp = 2): string {
  if (!Number.isFinite(value)) return "—";
  return String(tidy(value, dp));
}

/** Value shown in an input box, in display units. */
export function toInputText(kind: FieldKind, value: unknown): string {
  if (typeof value !== "number" || !Number.isFinite(value)) return "";
  if (kind === "pct" || kind === "pp" || kind === "optpct") return String(tidy(value * 100, 6));
  return String(tidy(value, 8));
}

/** Parses input text (display units) back to stored units; NaN when blank or not a number. */
export function fromInputText(kind: FieldKind, text: string): number {
  const cleaned = text.replace(/[,\s$%]/g, "").replace(/[，]/g, "");
  if (!cleaned) return Number.NaN;
  const n = Number(cleaned);
  if (!Number.isFinite(n)) return Number.NaN;
  if (kind === "pct" || kind === "pp" || kind === "optpct") return tidy(n / 100, 10);
  return n;
}

export function inputStep(kind: FieldKind): string {
  switch (kind) {
    case "int":
      return "1";
    case "money":
      return "100";
    case "pct":
    case "pp":
    case "optpct":
      return "0.5";
    default:
      return "0.1";
  }
}

/** Tilt multipliers for a strength: strongest ×(1+s), weakest ×(1−s). */
export function tiltRange(strength: number): { hi: string; lo: string } {
  return { hi: compact(1 + strength, 2), lo: compact(Math.max(0, 1 - strength), 2) };
}
