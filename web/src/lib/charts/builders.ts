/**
 * ECharts option builders. Conventions (see the dataviz guidance):
 * - one value axis per chart (no dual axes): drawdown, volume and weights get their own grids;
 * - thin marks, solid hairline grids, legends for ≥ 2 series plus selective direct labels;
 * - up candles are hollow and down candles filled, so direction never relies on colour alone;
 * - text uses ink tokens, never series colours.
 */
import type { Bar, Board, BoardItem, Sector } from "@/lib/types";
import { axisBase, esc, type Palette, tooltipBase } from "./echarts";
import { fmtNum, fmtPct, fmtPrice, fmtTime, MARKET_TZ } from "@/lib/format";

type Option = Record<string, any>;

const pct = (v: number | null | undefined, dp = 2) => fmtPct(v, { dp, sign: true });

// ---------------------------------------------------------------------------------------------
// Candle board: every asset's move over a period, grouped by sector
// ---------------------------------------------------------------------------------------------

export interface BoardLayout {
  /** One entry per category column: a company, or null for spacing between sectors. */
  slots: (BoardItem | null)[];
  /** Sector spans in column order (including their padding columns). */
  spans: { sector: Sector; start: number; count: number; members: number; avg: number | null }[];
}

const GAP_PREFIX = "⁣gap";

/**
 * Orders companies by sector (ontology order) and by period change within a sector. Sectors
 * are separated by an empty column, and narrow sectors are padded to `minSpan` columns so their
 * header label has room.
 */
export function layoutBoard(board: Board, visibleSectors: Set<string> | null, opts: { gap?: number; minSpan?: number } = {}): BoardLayout {
  const gap = opts.gap ?? 1;
  const minSpan = opts.minSpan ?? 3;
  const slots: (BoardItem | null)[] = [];
  const spans: BoardLayout["spans"] = [];
  for (const sector of board.sectors) {
    if (visibleSectors && !visibleSectors.has(sector.id)) continue;
    const members = board.items
      .filter((i) => i.sector_id === sector.id)
      .sort((a, b) => (b.close_pct ?? -Infinity) - (a.close_pct ?? -Infinity));
    if (!members.length) continue;
    if (slots.length) for (let i = 0; i < gap; i++) slots.push(null);
    const pad = Math.max(0, minSpan - members.length);
    const start = slots.length;
    for (let i = 0; i < Math.floor(pad / 2); i++) slots.push(null);
    slots.push(...members);
    for (let i = 0; i < Math.ceil(pad / 2); i++) slots.push(null);
    const valid = members.map((m) => m.close_pct).filter((v): v is number => v !== null);
    spans.push({
      sector,
      start,
      count: slots.length - start,
      members: members.length,
      avg: valid.length ? valid.reduce((a, b) => a + b, 0) / valid.length : null,
    });
  }
  return { slots, spans };
}

export const BOARD_GRID = { left: 56, right: 18 };

export function boardOption(
  layout: BoardLayout,
  p: Palette,
  labels: {
    weight: string;
    target: string;
    change: string;
    open: string;
    high: string;
    low: string;
    close: string;
    weightAxis: string;
    name: (i: BoardItem) => string;
    sector: (id: string) => string;
  },
): Option {
  const { slots } = layout;
  // Unique category names keep ECharts from merging the empty spacer columns.
  const categories = slots.map((s, i) => (s ? s.symbol : `${GAP_PREFIX}${i}`));
  const candles = slots.map((i) =>
    i ? [(i.open_pct ?? 0) * 100, (i.close_pct ?? 0) * 100, (i.low_pct ?? 0) * 100, (i.high_pct ?? 0) * 100] : "-",
  );
  const weights = slots.map((i) => (i ? +(i.weight * 100).toFixed(3) : "-"));
  const targets = slots.map((i) => (i && i.target_weight != null ? +(i.target_weight * 100).toFixed(3) : "-"));
  const bands = layout.spans
    .filter((_, idx) => idx % 2 === 0)
    .map((s) => [{ xAxis: s.start - 0.5 }, { xAxis: s.start + s.count - 0.5 }]);
  const base = axisBase(p);
  const numeric = slots.flatMap((i) => (i ? [i.weight * 100, (i.target_weight ?? 0) * 100] : []));
  const maxWeight = Math.max(2, ...numeric) * 1.15;
  const hideGap = (v: string) => (v.startsWith(GAP_PREFIX) ? "" : v);

  return {
    animation: false,
    textStyle: { fontFamily: p.font },
    grid: [
      { left: BOARD_GRID.left, right: BOARD_GRID.right, top: 10, height: "61%" },
      { left: BOARD_GRID.left, right: BOARD_GRID.right, top: "72%", bottom: 52 },
    ],
    axisPointer: { link: [{ xAxisIndex: "all" }] },
    tooltip: {
      ...tooltipBase(p),
      trigger: "axis",
      axisPointer: { type: "shadow", shadowStyle: { color: p.band } },
      formatter: (params: any[]) => {
        const idx = params?.[0]?.dataIndex;
        const item = slots[idx];
        if (!item) return "";
        const row = (k: string, v: string) =>
          `<div style="display:flex;justify-content:space-between;gap:18px"><span style="color:${p.ink500}">${esc(k)}</span><b style="font-variant-numeric:tabular-nums">${esc(v)}</b></div>`;
        return [
          `<div style="font-weight:700;margin-bottom:2px">${esc(item.symbol)} <span style="font-weight:500;color:${p.ink500}">${esc(labels.name(item))}</span></div>`,
          `<div style="color:${p.ink500};margin-bottom:6px">${esc(labels.sector(item.sector_id))}</div>`,
          row(labels.change, pct(item.close_pct)),
          row(labels.open, `${fmtPrice(item.open)} (${pct(item.open_pct)})`),
          row(labels.high, `${fmtPrice(item.high)} (${pct(item.high_pct)})`),
          row(labels.low, `${fmtPrice(item.low)} (${pct(item.low_pct)})`),
          row(labels.close, fmtPrice(item.close)),
          `<div style="height:1px;background:${p.line};margin:6px 0"></div>`,
          row(labels.weight, fmtPct(item.weight, { dp: 2 })),
          row(labels.target, item.target_weight == null ? "—" : fmtPct(item.target_weight, { dp: 2 })),
        ].join("");
      },
    },
    xAxis: [
      {
        type: "category",
        data: categories,
        gridIndex: 0,
        ...base,
        axisLabel: { show: false },
        splitLine: { show: false },
        axisLine: { show: false },
      },
      {
        type: "category",
        data: categories,
        gridIndex: 1,
        ...base,
        axisLabel: { ...base.axisLabel, rotate: 90, fontFamily: p.mono, fontSize: 10, interval: 0, margin: 8, formatter: hideGap },
        splitLine: { show: false },
      },
    ],
    yAxis: [
      {
        type: "value",
        gridIndex: 0,
        scale: false,
        ...base,
        axisLine: { show: false },
        axisLabel: { ...base.axisLabel, formatter: (v: number) => `${v > 0 ? "+" : ""}${fmtNum(v, Math.abs(v) < 10 ? 1 : 0)}%` },
      },
      {
        type: "value",
        gridIndex: 1,
        max: maxWeight,
        splitNumber: 2,
        name: labels.weightAxis,
        nameLocation: "end",
        nameGap: 6,
        nameTextStyle: { color: p.label, fontSize: 10, align: "right", padding: [0, 6, 0, 0] },
        ...base,
        axisLine: { show: false },
        axisLabel: { ...base.axisLabel, formatter: (v: number) => `${fmtNum(v, 0)}%` },
      },
    ],
    series: [
      {
        type: "candlestick",
        name: labels.change,
        xAxisIndex: 0,
        yAxisIndex: 0,
        data: candles,
        barMaxWidth: 14,
        itemStyle: {
          color: p.surface,
          color0: p.down,
          borderColor: p.up,
          borderColor0: p.down,
          borderWidth: 1.4,
        },
        markArea: { silent: true, itemStyle: { color: p.band }, data: bands },
        markLine: {
          silent: true,
          symbol: "none",
          lineStyle: { color: p.axis, width: 1, type: "solid" },
          label: { show: false },
          data: [{ yAxis: 0 }],
        },
      },
      {
        type: "bar",
        name: labels.weight,
        xAxisIndex: 1,
        yAxisIndex: 1,
        data: weights,
        barMaxWidth: 8,
        itemStyle: { color: p.ink600, borderRadius: [3, 3, 0, 0], opacity: 0.55 },
        markArea: { silent: true, itemStyle: { color: p.band }, data: bands },
      },
      {
        type: "scatter",
        name: labels.target,
        xAxisIndex: 1,
        yAxisIndex: 1,
        data: targets,
        symbol: "rect",
        symbolSize: [12, 2.5],
        itemStyle: { color: p.coral },
        z: 5,
      },
    ],
  };
}

// ---------------------------------------------------------------------------------------------
// Single-asset candles with volume, moving averages and trade markers
// ---------------------------------------------------------------------------------------------

export interface TradeMark {
  t: string;
  side: "buy" | "sell";
  qty: number;
  price: number;
}

/**
 * Rolls daily bars up into weekly (weeks start on Monday) or monthly candles, so a five- or
 * ten-year range stays legible. Each candle is dated by its first session; the moving averages
 * are read at its last session.
 */
export function aggregateBars(
  bars: Bar[],
  sma50: (number | null)[],
  sma200: (number | null)[],
  period: "week" | "month",
): { bars: Bar[]; sma50: (number | null)[]; sma200: (number | null)[]; interval: "1week" | "1month" } {
  const key = (t: string) => {
    if (period === "month") return t.slice(0, 7);
    const [y, m, d] = t.slice(0, 10).split("-").map(Number);
    const day = new Date(Date.UTC(y, m - 1, d));
    day.setUTCDate(day.getUTCDate() - ((day.getUTCDay() + 6) % 7));
    return day.toISOString().slice(0, 10);
  };
  const out: Bar[] = [];
  const s50: (number | null)[] = [];
  const s200: (number | null)[] = [];
  let current: string | null = null;
  bars.forEach((b, i) => {
    const k = key(b.t);
    if (k !== current) {
      current = k;
      out.push({ ...b });
      s50.push(sma50[i] ?? null);
      s200.push(sma200[i] ?? null);
      return;
    }
    const last = out[out.length - 1];
    last.h = Math.max(last.h, b.h);
    last.l = Math.min(last.l, b.l);
    last.c = b.c;
    last.v += b.v;
    s50[s50.length - 1] = sma50[i] ?? null;
    s200[s200.length - 1] = sma200[i] ?? null;
  });
  return { bars: out, sma50: s50, sma200: s200, interval: period === "week" ? "1week" : "1month" };
}

/** Fewest pixels per candle before the chart opens zoomed in on the most recent bars. */
const MIN_CANDLE_PX = 3;

export function candleOption(
  input: {
    bars: Bar[];
    interval: string;
    sma50: (number | null)[];
    sma200: (number | null)[];
    trades: TradeMark[];
    lastPrice: number | null;
    /** Plot width in pixels; when the candles would get too thin the chart opens on the latest. */
    plotWidth?: number;
  },
  p: Palette,
  labels: {
    open: string;
    high: string;
    low: string;
    close: string;
    volume: string;
    sma50: string;
    sma200: string;
    buy: string;
    sell: string;
    change: string;
    /** Tooltip heading of a weekly candle; `{d}` is the week's first session. */
    weekOf: string;
  },
): Option {
  const daily = input.interval === "1day";
  const weekly = input.interval === "1week";
  const monthly = input.interval === "1month";
  const intraday = !daily && !weekly && !monthly;
  const cats = input.bars.map((b) => b.t);
  const display = (t: string) => {
    if (daily) return t.slice(5);
    if (weekly || monthly) return t.slice(0, 7);
    return fmtTime(new Date(t), MARKET_TZ);
  };
  const candles = input.bars.map((b) => [b.o, b.c, b.l, b.h]);
  const volumes = input.bars.map((b, i) => ({
    value: b.v,
    itemStyle: { color: b.c >= b.o ? p.up : p.down, opacity: 0.35 },
    _i: i,
  }));
  // Map each trade to its bar (for weekly and monthly candles, the period that contains it).
  const barIndex = (t: string): number => {
    if (!input.bars.length) return -1;
    if (daily) {
      const day = new Date(t).toLocaleDateString("en-CA", { timeZone: MARKET_TZ });
      return input.bars.findIndex((b) => b.t === day);
    }
    if (weekly || monthly) {
      const day = new Date(t).toLocaleDateString("en-CA", { timeZone: MARKET_TZ });
      let idx = -1;
      for (let i = 0; i < input.bars.length && input.bars[i].t <= day; i++) idx = i;
      return idx;
    }
    const ts = new Date(t).getTime();
    let idx = -1;
    for (let i = 0; i < input.bars.length; i++) {
      if (new Date(input.bars[i].t).getTime() <= ts) idx = i;
      else break;
    }
    return idx;
  };
  const marks = input.trades
    .map((tr) => {
      const i = barIndex(tr.t);
      if (i < 0) return null;
      const bar = input.bars[i];
      const buy = tr.side === "buy";
      return {
        coord: [cats[i], buy ? bar.l : bar.h],
        value: buy ? labels.buy : labels.sell,
        symbol: "triangle",
        symbolSize: 9,
        symbolRotate: buy ? 0 : 180,
        symbolOffset: [0, buy ? 9 : -9],
        itemStyle: { color: buy ? p.up : p.down, borderColor: p.surface, borderWidth: 1.5 },
        label: { show: false },
        tooltip: {
          formatter: () => `${esc(buy ? labels.buy : labels.sell)} ${fmtNum(tr.qty, 0)} @ ${fmtPrice(tr.price)}`,
        },
      };
    })
    .filter(Boolean);
  const base = axisBase(p);
  // The whole range is shown unless candles would get thinner than MIN_CANDLE_PX; long series
  // keep a zoom slider either way.
  const zoom = input.bars.length > 90;
  const fit = input.plotWidth ? Math.max(30, Math.floor(input.plotWidth / MIN_CANDLE_PX)) : 90;
  const start = input.bars.length > fit ? Math.max(0, 100 - (fit / input.bars.length) * 100) : 0;
  const series: any[] = [
    {
      type: "candlestick",
      name: labels.close,
      data: candles,
      barMaxWidth: 12,
      itemStyle: { color: p.surface, color0: p.down, borderColor: p.up, borderColor0: p.down, borderWidth: 1.2 },
      markPoint: { data: marks, animation: false },
      markLine:
        input.lastPrice != null
          ? {
              silent: true,
              symbol: "none",
              lineStyle: { color: p.ink500, width: 1, type: "solid", opacity: 0.6 },
              // A price tag outside the plot on the right; the price axis sits on the left.
              label: {
                position: "end",
                color: p.ink,
                fontFamily: p.font,
                fontSize: 11,
                fontWeight: 600,
                backgroundColor: p.surface,
                padding: [2, 5],
                borderRadius: 3,
                formatter: () => fmtPrice(input.lastPrice),
              },
              data: [{ yAxis: input.lastPrice }],
            }
          : undefined,
    },
    {
      type: "bar",
      name: labels.volume,
      xAxisIndex: 1,
      yAxisIndex: 1,
      data: volumes,
      barMaxWidth: 12,
    },
  ];
  const hasSma = !intraday && input.sma50.some((v) => v != null);
  if (hasSma) {
    series.push(
      { type: "line", name: labels.sma50, data: input.sma50, showSymbol: false, smooth: false, lineStyle: { width: 1.5, color: p.series[1] }, itemStyle: { color: p.series[1] } },
      { type: "line", name: labels.sma200, data: input.sma200, showSymbol: false, smooth: false, lineStyle: { width: 1.5, color: p.series[3] }, itemStyle: { color: p.series[3] } },
    );
  }
  return {
    animation: false,
    textStyle: { fontFamily: p.font },
    legend: hasSma
      ? { top: 0, right: 8, data: [labels.sma50, labels.sma200], icon: "roundRect", itemWidth: 14, itemHeight: 3, textStyle: { color: p.label, fontSize: 11 } }
      : undefined,
    grid: [
      { left: 60, right: 62, top: hasSma ? 28 : 12, height: "64%" },
      { left: 60, right: 62, top: "78%", bottom: zoom ? 46 : 26 },
    ],
    axisPointer: { link: [{ xAxisIndex: "all" }], label: { backgroundColor: p.ink600 } },
    tooltip: {
      ...tooltipBase(p),
      trigger: "axis",
      axisPointer: { type: "cross", lineStyle: { color: p.crosshair }, crossStyle: { color: p.crosshair } },
      formatter: (params: any[]) => {
        const i = params?.[0]?.dataIndex;
        const b = input.bars[i];
        if (!b) return "";
        const prev = i > 0 ? input.bars[i - 1].c : null;
        const change = prev ? b.c / prev - 1 : null;
        const row = (k: string, v: string) =>
          `<div style="display:flex;justify-content:space-between;gap:18px"><span style="color:${p.ink500}">${esc(k)}</span><b style="font-variant-numeric:tabular-nums">${esc(v)}</b></div>`;
        const when = intraday
          ? `${new Date(b.t).toLocaleDateString("en-CA", { timeZone: MARKET_TZ })} ${fmtTime(b.t, MARKET_TZ)} ET`
          : weekly
            ? labels.weekOf.replace("{d}", b.t)
            : monthly
              ? b.t.slice(0, 7)
              : b.t;
        const rows = [
          `<div style="font-weight:700;margin-bottom:6px">${esc(when)}</div>`,
          row(labels.open, fmtPrice(b.o)),
          row(labels.high, fmtPrice(b.h)),
          row(labels.low, fmtPrice(b.l)),
          row(labels.close, fmtPrice(b.c)),
          row(labels.change, change == null ? "—" : pct(change)),
          row(labels.volume, fmtNum(b.v, 0)),
        ];
        if (hasSma) {
          rows.push(row(labels.sma50, input.sma50[i] == null ? "—" : fmtPrice(input.sma50[i])));
          rows.push(row(labels.sma200, input.sma200[i] == null ? "—" : fmtPrice(input.sma200[i])));
        }
        return rows.join("");
      },
    },
    xAxis: [
      {
        type: "category",
        data: cats,
        boundaryGap: true,
        ...base,
        axisLabel: { ...base.axisLabel, formatter: display, hideOverlap: true },
        splitLine: { show: false },
      },
      { type: "category", gridIndex: 1, data: cats, ...base, axisLabel: { show: false }, splitLine: { show: false } },
    ],
    yAxis: [
      { type: "value", scale: true, position: "left", ...base, axisLine: { show: false }, axisLabel: { ...base.axisLabel, formatter: (v: number) => fmtPrice(v) } },
      {
        type: "value",
        gridIndex: 1,
        position: "left",
        splitNumber: 2,
        ...base,
        axisLine: { show: false },
        axisLabel: { ...base.axisLabel, formatter: (v: number) => (v >= 1e6 ? `${fmtNum(v / 1e6, 0)}M` : v >= 1e3 ? `${fmtNum(v / 1e3, 0)}K` : fmtNum(v, 0)) },
      },
    ],
    dataZoom: zoom
      ? [
          { type: "inside", xAxisIndex: [0, 1], start, end: 100 },
          {
            type: "slider",
            xAxisIndex: [0, 1],
            start,
            end: 100,
            height: 18,
            bottom: 6,
            borderColor: p.line,
            fillerColor: p.band,
            backgroundColor: "transparent",
            dataBackground: { lineStyle: { color: p.axis }, areaStyle: { color: p.band } },
            handleStyle: { color: p.surface, borderColor: p.ink500 },
            textStyle: { color: p.label, fontSize: 10 },
            labelFormatter: (_: number, value: string) => display(value),
          },
        ]
      : [{ type: "inside", xAxisIndex: [0, 1] }],
    series,
  };
}
