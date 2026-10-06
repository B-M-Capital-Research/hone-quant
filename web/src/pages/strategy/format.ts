/**
 * Display helpers for strategy parameters: labels, value formatting with units, localized
 * validation messages and the key-parameter summary.
 */
import { locale, pick, tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { type FieldPath, strategyText } from "@/i18n/strategy";
import { DASH, fmtMoney } from "@/lib/format";
import type { Preset, StrategyParams } from "@/lib/types";
import { FIELD_BY_PATH, type FieldDef, type Issue, compact, fieldForIssue, getPath, tiltRange } from "./params";

/**
 * `pick` from `@/i18n` requires an index signature, which the interface-typed API rows lack;
 * this wrapper accepts any object (shared-file typing issue, worked around locally).
 */
export function pickText(row: object | null | undefined, field: string): string {
  return pick(row as Record<string, unknown> | null | undefined, field);
}

export function fieldText(path: string): { label: string; hint: string } {
  const entry = strategyText().fields[path as FieldPath];
  return entry ?? { label: path, hint: "" };
}

/** Percentage with up to two decimals and no trailing zeros: 0.98 → "98%", 0.005 → "0.5%". */
export function pctText(value: number | null | undefined, dp = 2): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return DASH;
  return `${compact(value * 100, dp)}%`;
}

export function ppText(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return DASH;
  const n = compact(value * 100, 2);
  return locale() === "zh" ? `${n} ${strategyText().units.pp}` : `${n} pp`;
}

export function daysText(value: number): string {
  return locale() === "zh" ? `${value} ${strategyText().units.days}` : `${value} ${strategyText().units.days}`;
}

export function methodText(method: string): string {
  const m = strategyText().methods as Record<string, string>;
  return m[method] ?? method;
}

/** A parameter value with its unit, for read-only views. */
export function fmtParam(path: string, value: unknown): string {
  const def = FIELD_BY_PATH[path];
  const t = strategyText();
  if (!def) return value === null || value === undefined ? DASH : String(value);
  switch (def.kind) {
    case "int":
      if (typeof value !== "number") return DASH;
      if (def.zeroOff && value === 0) return t.values.off;
      return daysText(value);
    case "pct":
      return pctText(value as number);
    case "pp":
      return ppText(value as number);
    case "optpct":
      return value === null || value === undefined ? t.values.not_set : pctText(value as number);
    case "num":
      return typeof value === "number" ? compact(value, 2) : DASH;
    case "mult":
      return typeof value === "number" ? `×${compact(value, 2)}` : DASH;
    case "money":
      return typeof value === "number" ? fmtMoney(value, { dp: 0 }) : DASH;
    case "bool":
      return value ? t.values.on : t.values.off;
    case "method":
      return methodText(String(value));
    case "budgets": {
      const entries = Object.entries((value as Record<string, number>) ?? {});
      return entries.length ? tpl(t.values.budgets_count, { n: entries.length }) : t.values.budgets_none;
    }
  }
}

/** A bound (min or max) in display units, without the unit word for days. */
export function fmtBound(def: FieldDef, value: number | undefined): string {
  if (value === undefined) return DASH;
  switch (def.kind) {
    case "pct":
    case "optpct":
      return pctText(value);
    case "pp":
      return ppText(value);
    case "money":
      return fmtMoney(value, { dp: 0 });
    default:
      return compact(value, 3);
  }
}

/** Localized validation message for a client- or server-side issue. */
export function issueText(issue: Pick<Issue, "path" | "code" | "min" | "max" | "message">): string {
  const e = strategyText().errors;
  const def = fieldForIssue(issue.path);
  switch (issue.code) {
    case "required":
      return issue.path === "sector.custom_budgets" ? e.budgets_required : e.required;
    case "integer":
      return e.integer;
    case "out_of_range": {
      if (issue.path.startsWith("sector.custom_budgets.")) return tpl(e.out_of_range, { min: compact(issue.min ?? 0), max: compact(issue.max ?? 100) });
      if (!def) return issue.message || e.required;
      return tpl(e.out_of_range, { min: fmtBound(def, issue.min), max: fmtBound(def, issue.max) });
    }
    case "skip_not_below_lookback":
      return e.skip_not_below_lookback;
    case "min_above_max":
      return e.min_above_max;
    case "unknown_field":
      return `${issue.path}: ${e.unknown_field}`;
    default:
      return issue.message || issue.code;
  }
}

export function presetName(presets: Preset[] | undefined, id: string): string {
  if (id === "custom") return strategyText().active.custom_preset;
  const preset = presets?.find((p) => p.id === id);
  return preset ? pickText(preset, "name") : id;
}

/** Human time span for schedule offsets: 30 → "30 分钟", 180 → "3 小时". */
export function minutesText(minutes: number): string {
  const h = strategyText().how;
  if (minutes >= 60 && minutes % 60 === 0) return tpl(h.hours, { n: minutes / 60 });
  return tpl(h.minutes, { n: minutes });
}

export function powerText(power: number): string {
  const h = strategyText().how;
  if (power === 0) return h.power_equal;
  if (power === 1) return h.power_inverse_vol;
  if (power === 2) return h.power_inverse_var;
  return tpl(h.power_custom, { p: compact(power, 2) });
}

export interface KeyParam {
  label: string;
  value: string;
  paths: string[];
}

/** The handful of parameters that characterise a strategy at a glance. */
export function keyParams(p: StrategyParams): KeyParam[] {
  const t = strategyText();
  const k = t.key;
  const e = p.exposure;
  const tilt = (v: number) => pctText(v);
  return [
    {
      label: k.exposure,
      value: e.breadth_scaling ? tpl(k.exposure_breadth, { min: pctText(e.min_exposure), max: pctText(e.max_exposure) }) : pctText(e.max_exposure),
      paths: ["exposure.min_exposure", "exposure.max_exposure", "exposure.breadth_scaling"],
    },
    { label: k.method, value: methodText(p.sector.method), paths: ["sector.method"] },
    {
      label: k.sector_range,
      value: `${pctText(p.sector.min_weight)}–${pctText(p.sector.max_weight)}`,
      paths: ["sector.min_weight", "sector.max_weight"],
    },
    { label: k.cap, value: pctText(p.asset.max_weight), paths: ["asset.max_weight"] },
    {
      label: k.tilt,
      value: tpl(k.tilt_value, { sector: tilt(p.sector.momentum_tilt), asset: tilt(p.asset.momentum_tilt) }),
      paths: ["sector.momentum_tilt", "asset.momentum_tilt"],
    },
    {
      label: k.trend,
      value: p.asset.trend_sma > 0 ? tpl(k.trend_value, { sma: p.asset.trend_sma, penalty: compact(p.asset.trend_penalty, 2) }) : t.values.off,
      paths: ["asset.trend_sma", "asset.trend_penalty"],
    },
    { label: k.vol_target, value: e.target_vol == null ? t.values.not_set : pctText(e.target_vol), paths: ["exposure.target_vol"] },
    { label: k.turnover, value: pctText(p.rebalance.max_turnover), paths: ["rebalance.max_turnover"] },
    {
      label: k.bands,
      value: tpl(k.bands_value, { abs: ppText(p.rebalance.band_abs), rel: pctText(p.rebalance.band_rel) }),
      paths: ["rebalance.band_abs", "rebalance.band_rel"],
    },
    { label: k.min_trade, value: fmtMoney(p.rebalance.min_trade_value, { dp: 0 }), paths: ["rebalance.min_trade_value"] },
  ];
}

/** Tilt multipliers as text for the methodology. */
export function tiltText(strength: number): { hi: string; lo: string } {
  return tiltRange(strength);
}

export function valueOf(p: StrategyParams, path: string): string {
  return fmtParam(path, getPath(p, path));
}

export function yesNo(value: boolean): string {
  return value ? common().words.yes : common().words.no;
}
