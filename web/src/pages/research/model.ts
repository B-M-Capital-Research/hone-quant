/**
 * Data access and pure helpers shared by the backtest and performance pages: a small loader
 * that keeps the previous value while refetching, cached reference data, strategy and
 * benchmark labels, date arithmetic and the period-return maths the API does not provide
 * for benchmarks.
 */
import { type Accessor, createSignal, onCleanup } from "solid-js";
import { locale, pick, tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { researchText } from "@/i18n/research";
import { api } from "@/lib/api";
import { fmtDate, MARKET_TZ } from "@/lib/format";
import { serverNow } from "@/lib/session";
import type {
  BacktestRow,
  CostModel,
  DataStatus,
  SettingsBundle,
  StrategyOverview,
  StrategyParams,
  UniverseView,
} from "@/lib/types";
import { strategyName } from "@/lib/names";

/** Identifier of the equal-weight universe benchmark computed by the backtester. */
export const UNIVERSE_EW = "UNIVERSE_EW";
/** Matches `RELIABLE_ANNUALISATION_DAYS` in quant-core. */
export const RELIABLE_DAYS = 63;
/** Matches `MAX_STORED_TRADES` in the backtest service. */
export const MAX_STORED_TRADES = 5000;

// ---------------------------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------------------------

export interface Loader<T> {
  data: Accessor<T | undefined>;
  error: Accessor<unknown>;
  loading: Accessor<boolean>;
  load: () => Promise<void>;
}

/**
 * Fetches into signals. Unlike `createResource`, a failed refetch never throws on read: the
 * previous value stays visible and `error` is set, which suits live-updating pages.
 */
export function createLoader<T>(fetcher: () => Promise<T>): Loader<T> {
  const [data, setData] = createSignal<T | undefined>();
  const [error, setError] = createSignal<unknown>();
  const [loading, setLoading] = createSignal(false);
  let seq = 0;
  let alive = true;
  onCleanup(() => {
    alive = false;
  });
  const load = async () => {
    const id = ++seq;
    setLoading(true);
    try {
      const value = await fetcher();
      if (alive && id === seq) {
        setData(() => value);
        setError(undefined);
      }
    } catch (e) {
      if (alive && id === seq) setError(e);
    } finally {
      if (alive && id === seq) setLoading(false);
    }
  };
  return { data, error, loading, load };
}

/** Coalesces bursts of server events into one call. */
export function debounce(fn: () => void, ms: number): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  onCleanup(() => {
    if (timer) clearTimeout(timer);
  });
  return () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      fn();
    }, ms);
  };
}

/** Reference data shared between the research pages, cached briefly across navigations. */
function cached<T>(fetcher: () => Promise<T>, ttlMs: number): { get: () => Promise<T>; invalidate: () => void } {
  let entry: { at: number; promise: Promise<T> } | null = null;
  return {
    get() {
      if (entry && Date.now() - entry.at < ttlMs) return entry.promise;
      const promise = fetcher();
      entry = { at: Date.now(), promise };
      promise.catch(() => {
        if (entry?.promise === promise) entry = null;
      });
      return promise;
    },
    invalidate() {
      entry = null;
    },
  };
}

export const strategyRef = cached<StrategyOverview>(() => api.strategy(), 30_000);
export const universeRef = cached<UniverseView>(() => api.universe(), 120_000);
export const settingsRef = cached<SettingsBundle>(() => api.settings(), 30_000);
export const dataStatusRef = cached<DataStatus>(() => api.dataStatus(), 120_000);

// ---------------------------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------------------------

/** `pick` for interface-typed rows (the shared signature requires an index signature). */
export function pickText(row: object | null | undefined, field: string): string {
  return pick(row as Record<string, unknown> | null | undefined, field);
}

/** Structural equality for JSON values (parameter sets). */
export function jsonEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a === "number" && typeof b === "number") return Math.abs(a - b) <= 1e-12 * Math.max(1, Math.abs(a), Math.abs(b));
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const ka = Object.keys(a as object).filter((k) => (a as any)[k] !== undefined);
  const kb = Object.keys(b as object).filter((k) => (b as any)[k] !== undefined);
  if (ka.length !== kb.length) return false;
  return ka.every((k) => jsonEqual((a as any)[k], (b as any)[k]));
}

export type StrategyRef =
  | { kind: "version"; id: number; name: string | null }
  | { kind: "preset"; id: string; name: string }
  | { kind: "custom" };

/**
 * What a backtest ran: a stored version (recorded on the row) or, failing that, the preset
 * whose parameters it matches exactly. The API does not record the preset id itself.
 */
export function resolveStrategy(row: Pick<BacktestRow, "strategy_version_id" | "config">, overview: StrategyOverview | undefined): StrategyRef {
  if (row.strategy_version_id != null) {
    const version = overview?.versions.find((v) => v.id === row.strategy_version_id);
    return { kind: "version", id: row.strategy_version_id, name: version ? strategyName(version) : null };
  }
  const preset = overview?.presets.find((p) => jsonEqual(p.params, row.config.params));
  if (preset) return { kind: "preset", id: preset.id, name: pickText(preset, "name") };
  return { kind: "custom" };
}

export function strategyLabel(ref: StrategyRef, overview: StrategyOverview | undefined): string {
  const r = researchText();
  if (ref.kind === "version") {
    if (!overview) return r.list.loading_strategy;
    return tpl(r.source.version_short, { name: ref.name ?? common().words.version, id: ref.id });
  }
  if (ref.kind === "preset") return `${r.source.preset_prefix} · ${ref.name}`;
  return r.source.custom;
}

/** A readable explanation for the backtester's known failure messages (else the raw text). */
export function failureText(error: string | null | undefined): { text: string; known: "no_data" | "interrupted" | "no_cover" | null } {
  const d = researchText().detail;
  const raw = error ?? "";
  if (/no price history is stored/i.test(raw)) return { text: d.failed_no_data, known: "no_data" };
  if (/interrupted/i.test(raw)) return { text: d.failed_interrupted, known: "interrupted" };
  if (/does not cover the requested period/i.test(raw)) return { text: d.failed_no_cover, known: "no_cover" };
  return { text: raw || researchText().status.failed, known: null };
}

/** Short benchmark label: the ticker for ETFs, a word for the universe equal-weight index. */
export function benchShort(symbol: string): string {
  if (symbol === UNIVERSE_EW) return locale() === "zh" ? "范围等权" : "Universe EW";
  return symbol;
}

/** Long benchmark label for selects and tooltips. */
export function benchLong(symbol: string, universe: UniverseView | undefined): string {
  if (symbol === UNIVERSE_EW) return common().words.universe_ew;
  const def = universe?.benchmarks.find((b) => b.symbol === symbol);
  if (!def) return symbol;
  const name = pickText(def, "name");
  return name.includes(symbol) ? name : `${symbol} · ${name}`;
}

/**
 * Fixed colour slots so a benchmark keeps its colour everywhere: the portfolio is slot 0, the
 * configured ETFs follow in settings order and the equal-weight universe index is last.
 */
export function benchSlot(symbol: string, order: string[]): number {
  if (symbol === UNIVERSE_EW) return 4;
  const idx = order.filter((s) => s !== UNIVERSE_EW).indexOf(symbol);
  return idx < 0 ? 3 : 1 + (idx % 3);
}

export function slotsLabel(slots: ("open" | "close")[]): string {
  const r = researchText();
  const set = new Set(slots);
  if (set.has("open") && set.has("close")) return r.slots.both;
  if (set.has("open")) return r.slots.open;
  return r.slots.close;
}

export function sectorNamer(universe: UniverseView | undefined): (id: string | null | undefined) => string {
  const map = new Map((universe?.sectors ?? []).map((s) => [s.id, s]));
  return (id) => {
    if (!id) return researchText().detail.unknown_sector;
    const sector = map.get(id);
    return sector ? pickText(sector, "name") : id;
  };
}

export function assetNamer(universe: UniverseView | undefined): (symbol: string) => string {
  const map = new Map([...(universe?.assets ?? []), ...(universe?.removed ?? [])].map((a) => [a.symbol, a]));
  return (symbol) => {
    const asset = map.get(symbol);
    return asset ? pickText(asset, "name") : "";
  };
}

// ---------------------------------------------------------------------------------------------
// Dates & durations
// ---------------------------------------------------------------------------------------------

const DAY_MS = 86_400_000;

export function parseDay(value: string): Date | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return null;
  const d = new Date(`${value}T00:00:00Z`);
  return Number.isNaN(d.getTime()) ? null : d;
}

export function isoDay(d: Date): string {
  return d.toISOString().slice(0, 10);
}

export function addDays(value: string, days: number): string {
  const d = parseDay(value);
  if (!d) return value;
  return isoDay(new Date(d.getTime() + days * DAY_MS));
}

/** Same calendar day `years` earlier/later (29 Feb falls back to 28 Feb). */
export function addYears(value: string, years: number): string {
  const d = parseDay(value);
  if (!d) return value;
  const y = d.getUTCFullYear() + years;
  const m = d.getUTCMonth();
  const day = Math.min(d.getUTCDate(), new Date(Date.UTC(y, m + 1, 0)).getUTCDate());
  return isoDay(new Date(Date.UTC(y, m, day)));
}

export function daysBetween(from: string, to: string): number {
  const a = parseDay(from);
  const b = parseDay(to);
  if (!a || !b) return 0;
  return Math.round((b.getTime() - a.getTime()) / DAY_MS);
}

export function yearsBetween(from: string, to: string): number {
  return daysBetween(from, to) / 365.25;
}

/** Today's trading date on the server clock (the demo may run a shifted clock). */
export function marketTodayServer(): string {
  return fmtDate(serverNow(), MARKET_TZ);
}

export function fmtYears(years: number): string {
  return years >= 10 ? years.toFixed(0) : years.toFixed(1);
}

export function fmtDuration(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms)) return "—";
  const u = researchText().units;
  const total = Math.max(0, ms) / 1000;
  if (total < 10) return tpl(u.seconds, { n: total.toFixed(1) });
  if (total < 60) return tpl(u.seconds, { n: Math.round(total) });
  const m = Math.floor(total / 60);
  const s = Math.round(total % 60);
  return tpl(u.minutes, { m, s });
}

/** Strategy warm-up in trading days (mirrors `StrategyParams::max_lookback`). */
export function maxLookback(params: StrategyParams): number {
  const values = [
    params.universe.min_history_days,
    params.sector.vol_lookback + 1,
    params.sector.momentum_lookback + 1,
    params.asset.vol_lookback + 1,
    params.asset.momentum_lookback + 1,
    params.asset.trend_sma,
    params.exposure.vol_lookback + 1,
  ];
  if (params.exposure.breadth_scaling) values.push(params.exposure.breadth_sma);
  return Math.max(1, ...values);
}

/** Calendar days of history the backtester loads before the start (same formula as the server). */
export function warmupCalendarDays(params: StrategyParams): number {
  return Math.floor(maxLookback(params) * 1.6) + 30;
}

/** First date with prices for any universe member, from the data-coverage report. */
export function dataWindow(status: DataStatus | undefined, universe: UniverseView | undefined): { first: string; last: string } | null {
  if (!status) return null;
  const members = new Set((universe?.assets ?? []).map((a) => a.symbol));
  const rows = status.coverage.filter((c) => c.first && c.last && (members.size === 0 || members.has(c.symbol)));
  if (!rows.length) return null;
  const first = rows.map((c) => c.first!).sort()[0];
  const last = rows.map((c) => c.last!).sort().reverse()[0];
  return { first, last };
}

/** Typical run time per year of simulated history, from finished backtests. */
export function typicalMsPerYear(rows: BacktestRow[]): number | null {
  const samples = rows
    .filter((r) => r.status === "succeeded" && r.summary?.elapsed_ms)
    .map((r) => {
      const years = Math.max(0.25, yearsBetween(r.config.start, r.config.end));
      const slots = Math.max(1, r.config.slots.length);
      return r.summary!.elapsed_ms / years / slots;
    })
    .sort((a, b) => a - b);
  if (!samples.length) return null;
  return samples[Math.floor(samples.length / 2)];
}

export function estimateMs(row: BacktestRow, perYear: number | null): number | null {
  if (perYear == null) return null;
  return perYear * Math.max(0.25, yearsBetween(row.config.start, row.config.end)) * Math.max(1, row.config.slots.length);
}

// ---------------------------------------------------------------------------------------------
// Returns
// ---------------------------------------------------------------------------------------------

/**
 * Compounded returns per calendar year, the first year measured from the first value (same
 * convention as `metrics::yearly_returns`). Leading nulls are skipped.
 */
export function yearlyReturns(dates: string[], values: (number | null)[]): Map<number, number> {
  const out = new Map<number, number>();
  let base: number | null = null;
  let year: number | null = null;
  let last: number | null = null;
  for (let i = 0; i < dates.length; i++) {
    const v = values[i];
    if (v == null || !(v > 0)) continue;
    const y = Number(dates[i].slice(0, 4));
    if (base == null) {
      base = v;
      year = y;
    } else if (y !== year && last != null) {
      out.set(year!, last / base - 1);
      base = last;
      year = y;
    }
    last = v;
  }
  if (base != null && last != null && year != null) out.set(year, last / base - 1);
  return out;
}

/** Calendar years whose first or last date is not the year's first/last trading day. */
export function partialYears(dates: string[]): { first: { year: number; date: string } | null; last: { year: number; date: string } | null } {
  if (dates.length < 2) return { first: null, last: null };
  const firstDate = dates[0];
  const lastDate = dates[dates.length - 1];
  // A year is complete when the series starts within the first week of January (the base is
  // the prior close) and ends within the last week of December.
  const startsLate = !(firstDate.slice(5) <= "01-07" || firstDate.slice(5) >= "12-24");
  const endsEarly = lastDate.slice(5) < "12-24";
  return {
    first: startsLate ? { year: Number(firstDate.slice(0, 4)), date: firstDate } : null,
    last: endsEarly ? { year: Number(lastDate.slice(0, 4)), date: lastDate } : null,
  };
}

export function mean(values: number[]): number | null {
  if (!values.length) return null;
  return values.reduce((a, b) => a + b, 0) / values.length;
}

/** Live element width, quantised so charts only re-render on meaningful changes. */
export function createWidth(step = 16): [Accessor<number>, (el: HTMLElement) => void] {
  const [width, setWidth] = createSignal(0);
  let observer: ResizeObserver | undefined;
  const ref = (el: HTMLElement) => {
    observer?.disconnect();
    const measure = (w: number) => setWidth((prev) => (Math.abs(prev - w) >= step || prev === 0 ? Math.round(w) : prev));
    measure(el.getBoundingClientRect().width);
    observer = new ResizeObserver((entries) => measure(entries[0].contentRect.width));
    observer.observe(el);
  };
  onCleanup(() => observer?.disconnect());
  return [width, ref];
}

/** Matches a media query reactively. */
export function createMedia(query: string): Accessor<boolean> {
  const mql = typeof window !== "undefined" ? window.matchMedia?.(query) : undefined;
  const [matches, setMatches] = createSignal(mql?.matches ?? false);
  const onChange = (e: MediaQueryListEvent) => setMatches(e.matches);
  mql?.addEventListener?.("change", onChange);
  onCleanup(() => mql?.removeEventListener?.("change", onChange));
  return matches;
}

// ---------------------------------------------------------------------------------------------
// Costs (display units ↔ model)
// ---------------------------------------------------------------------------------------------

export type CostKey = keyof CostModel;

export const COST_FIELDS: { key: CostKey; factor: number; min: number; max: number; step: string; unit: "per_share" | "usd" | "pct" | "bp" | "per_million" }[] = [
  { key: "commission_per_share", factor: 1, min: 0, max: 1, step: "0.001", unit: "per_share" },
  { key: "commission_min", factor: 1, min: 0, max: 100, step: "0.01", unit: "usd" },
  { key: "commission_max_rate", factor: 100, min: 0, max: 5, step: "0.01", unit: "pct" },
  { key: "commission_rate", factor: 10_000, min: 0, max: 100, step: "0.1", unit: "bp" },
  { key: "slippage_bps", factor: 1, min: 0, max: 200, step: "0.5", unit: "bp" },
  { key: "sell_fee_rate", factor: 1_000_000, min: 0, max: 1000, step: "0.1", unit: "per_million" },
];

export function costToDisplay(model: CostModel): Record<CostKey, string> {
  const out = {} as Record<CostKey, string>;
  for (const f of COST_FIELDS) out[f.key] = String(+(model[f.key] * f.factor).toPrecision(10));
  return out;
}

export function costsSummary(model: CostModel): string {
  const r = researchText();
  const d = costToDisplay(model);
  return tpl(r.form.costs_summary, {
    per: d.commission_per_share,
    min: Number(d.commission_min).toFixed(2),
    cap: model.commission_max_rate > 0 ? `${d.commission_max_rate}%` : "—",
    slip: d.slippage_bps,
    fee: d.sell_fee_rate,
  });
}

export function costsEqual(a: CostModel, b: CostModel): boolean {
  return COST_FIELDS.every((f) => Math.abs(a[f.key] - b[f.key]) <= 1e-12 + 1e-9 * Math.abs(b[f.key]));
}
