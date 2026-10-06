/**
 * Research chart builders (backtest and performance reports):
 * - equity and drawdown share one canvas as two stacked grids (one value axis each, never a
 *   dual axis) so the crosshair and zoom stay in sync; end labels are selective (collisions
 *   are dropped, not nudged) and the right padding is measured so they are never clipped;
 * - heatmap cell labels pick ink or surface text by the cell's luminance;
 * - axis labels are measured and truncated instead of overflowing on narrow screens;
 * - negative bars round their data end and put their value label on the outer side.
 */
import { axisBase, esc, type Palette, tooltipBase } from "@/lib/charts/echarts";
import { fmtNum, fmtPct } from "@/lib/format";
import type { PeriodReturn } from "@/lib/types";

type Option = Record<string, any>;

// ---------------------------------------------------------------------------------------------
// Text & colour utilities
// ---------------------------------------------------------------------------------------------

let measureCtx: CanvasRenderingContext2D | null | undefined;

export function textWidth(text: string, font: string, size = 11, weight = 400): number {
  if (measureCtx === undefined) measureCtx = typeof document === "undefined" ? null : document.createElement("canvas").getContext("2d");
  if (!measureCtx) return text.length * size * 0.62;
  measureCtx.font = `${weight} ${size}px ${font}`;
  return measureCtx.measureText(text).width;
}

/** Truncates to fit `max` pixels with an ellipsis. */
export function fitText(text: string, font: string, max: number, size = 11): string {
  if (textWidth(text, font, size) <= max) return text;
  let lo = 0;
  let hi = text.length;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (textWidth(`${text.slice(0, mid)}…`, font, size) <= max) lo = mid;
    else hi = mid - 1;
  }
  return `${text.slice(0, Math.max(1, lo))}…`;
}

function parseColor(value: string): [number, number, number, number] | null {
  const v = value.trim();
  let m = /^#([0-9a-f]{3,8})$/i.exec(v);
  if (m) {
    let hex = m[1];
    if (hex.length === 3 || hex.length === 4) hex = [...hex].map((c) => c + c).join("");
    const n = parseInt(hex.slice(0, 6), 16);
    const a = hex.length === 8 ? parseInt(hex.slice(6, 8), 16) / 255 : 1;
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255, a];
  }
  m = /^rgba?\(([^)]+)\)$/i.exec(v);
  if (m) {
    const parts = m[1].split(/[,\s/]+/).filter(Boolean).map(Number);
    return [parts[0], parts[1], parts[2], parts[3] ?? 1];
  }
  return null;
}

function mix(a: string, b: string, t: number): [number, number, number] {
  const ca = parseColor(a) ?? [128, 128, 128, 1];
  const cb = parseColor(b) ?? [128, 128, 128, 1];
  return [0, 1, 2].map((i) => ca[i] + (cb[i] - ca[i]) * t) as [number, number, number];
}

function luminance([r, g, b]: [number, number, number]): number {
  const f = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

function contrast(a: number, b: number): number {
  const [hi, lo] = a > b ? [a, b] : [b, a];
  return (hi + 0.05) / (lo + 0.05);
}

/** Ink or surface text, whichever reads better on `fill`. */
function textOn(fill: [number, number, number], p: Palette): string {
  const lf = luminance(fill);
  const ink = parseColor(p.ink);
  const surface = parseColor(p.surface);
  if (!ink || !surface) return p.ink;
  const li = luminance([ink[0], ink[1], ink[2]]);
  const ls = luminance([surface[0], surface[1], surface[2]]);
  return contrast(lf, li) >= contrast(lf, ls) ? p.ink : p.surface;
}

/** Signed number; a value that rounds to zero at `dp` carries no sign (never "−0.0"). */
export const signed = (v: number, dp: number) => {
  const zero = Number(Math.abs(v).toFixed(dp)) === 0;
  return `${zero ? "" : v > 0 ? "+" : "−"}${fmtNum(Math.abs(v), dp)}`;
};

function row(color: string | null, name: string, value: string, p: Palette, strong = true): string {
  const key = color ? `<i style="display:inline-block;width:12px;height:2px;border-radius:1px;background:${color}"></i>` : "";
  return `<div style="display:flex;align-items:center;justify-content:space-between;gap:16px;line-height:1.7"><span style="display:flex;align-items:center;gap:6px;color:${p.ink600}">${key}${esc(name)}</span><span style="font-variant-numeric:tabular-nums;${strong ? "font-weight:650" : ""}">${esc(value)}</span></div>`;
}

// ---------------------------------------------------------------------------------------------
// Equity curve + drawdown (two grids, one canvas)
// ---------------------------------------------------------------------------------------------

export interface EquityLine {
  /** Legend and tooltip name. */
  name: string;
  /** Short end-label name. */
  label: string;
  values: (number | null)[];
  /** Palette slot; the portfolio is 0. */
  slot: number;
  emphasis?: boolean;
  /** Initially hidden (toggle in the legend). */
  hidden?: boolean;
}

export interface EquityLabels {
  cumulative: string;
  drawdown: string;
  maxDd: string;
}

export function canLog(lines: EquityLine[]): boolean {
  let min = Infinity;
  let max = -Infinity;
  for (const line of lines) {
    const first = line.values.find((v) => v != null && v > 0);
    if (first == null) continue;
    for (const v of line.values) {
      if (v == null || !(v > 0)) continue;
      min = Math.min(min, v / first);
      max = Math.max(max, v / first);
    }
  }
  return Number.isFinite(min) && max / min >= 1.8;
}

export function equityDrawdownOption(
  dates: string[],
  lines: EquityLine[],
  drawdowns: number[],
  p: Palette,
  o: { width: number; height: number; log: boolean; labels: EquityLabels },
): Option {
  const base = axisBase(p);
  const narrow = o.width > 0 && o.width < 560;
  const n = dates.length;
  const zoom = n > 300;
  const top = 34;
  const gap = 42;
  const bottom = zoom ? 64 : 28;
  const avail = Math.max(120, o.height - top - gap - bottom);
  const topH = Math.round(avail * 0.7);
  const ddH = avail - topH;
  const left = narrow ? 46 : 56;

  // Series data: cumulative % (linear) or growth multiple (log).
  const transformed = lines.map((line) => {
    const first = line.values.find((v) => v != null && v > 0) ?? null;
    return line.values.map((v) => {
      if (v == null || first == null) return null;
      return o.log ? +(v / first).toFixed(6) : +((v / first - 1) * 100).toFixed(4);
    });
  });
  const lastOf = (data: (number | null)[]) => [...data].reverse().find((v) => v != null) ?? null;
  const toPct = (v: number) => (o.log ? (v - 1) * 100 : v);
  const values = transformed.flat().filter((v): v is number => v != null);
  // Moves under 1% (a young account) get two decimals so they do not all read "0.0%".
  const valueDp = values.every((v) => Math.abs(toPct(v)) < 1) ? 2 : 1;
  const fmtVal = (v: number | null) => (v == null ? "—" : `${signed(toPct(v), valueDp)}%`);

  // Selective end labels: portfolio first, then benchmarks in slot order; a label that would
  // collide with an accepted one is dropped (the legend and tooltip still carry it).
  const pos = (v: number) => (o.log ? Math.log2(Math.max(v, 1e-6)) : v);
  const lo = values.length ? Math.min(...values.map(pos)) : 0;
  const hi = values.length ? Math.max(...values.map(pos)) : 1;
  const pxPerUnit = topH / Math.max(1e-9, hi - lo);
  const order = lines
    .map((line, i) => ({ i, line }))
    .filter(({ line }) => !line.hidden)
    .sort((a, b) => (a.line.emphasis ? -1 : b.line.emphasis ? 1 : a.line.slot - b.line.slot));
  const accepted: { i: number; y: number }[] = [];
  if (!narrow) {
    for (const { i } of order) {
      const last = lastOf(transformed[i]);
      if (last == null) continue;
      const y = pos(last) * pxPerUnit;
      if (accepted.every((a) => Math.abs(a.y - y) >= 15)) accepted.push({ i, y });
    }
  }
  const labelled = new Set(accepted.map((a) => a.i));
  const labelWidth = Math.max(
    0,
    ...accepted.map(({ i }) => textWidth(`${lines[i].label} ${fmtVal(lastOf(transformed[i]))}`, p.font, 11, lines[i].emphasis ? 650 : 400)),
  );
  const right = labelled.size ? Math.ceil(labelWidth) + 16 : narrow ? 12 : 20;

  const series: any[] = lines.map((line, i) => {
    const color = p.series[line.slot % p.series.length];
    return {
      type: "line",
      name: line.name,
      data: transformed[i],
      xAxisIndex: 0,
      yAxisIndex: 0,
      showSymbol: false,
      connectNulls: true,
      sampling: "lttb",
      lineStyle: { width: line.emphasis ? 2 : 1.4, color, opacity: line.emphasis ? 1 : 0.9 },
      itemStyle: { color },
      emphasis: { focus: "series", lineStyle: { width: line.emphasis ? 2.4 : 2 } },
      z: line.emphasis ? 5 : 3,
      endLabel: {
        show: labelled.has(i),
        color: p.ink,
        fontFamily: p.font,
        fontSize: 11,
        fontWeight: line.emphasis ? 650 : 400,
        distance: 6,
        formatter: (param: any) => `${line.label} ${fmtVal(Array.isArray(param.value) ? param.value[1] : param.value)}`,
      },
    };
  });

  // Drawdown pane with the worst point labelled.
  const ddData = drawdowns.map((d) => +(d * 100).toFixed(3));
  let troughIdx = -1;
  for (let i = 0; i < ddData.length; i++) if (troughIdx < 0 || ddData[i] < ddData[troughIdx]) troughIdx = i;
  const trough = troughIdx >= 0 && ddData[troughIdx] < 0 ? troughIdx : -1;
  const troughRight = trough >= 0 && trough / Math.max(1, n - 1) > 0.6;
  // Axis precision follows the depth, so a shallow drawdown is not labelled "0%, −0%, −0%".
  const ddDepth = trough >= 0 ? Math.abs(ddData[trough]) : 0;
  const ddDp = ddDepth >= 3 ? 0 : ddDepth >= 0.3 ? 1 : 2;
  series.push({
    type: "line",
    name: o.labels.drawdown,
    data: ddData,
    xAxisIndex: 1,
    yAxisIndex: 1,
    showSymbol: false,
    sampling: "lttb",
    lineStyle: { width: 1.4, color: p.down },
    itemStyle: { color: p.down },
    areaStyle: { color: p.down, opacity: 0.1 },
    markPoint:
      trough >= 0
        ? {
            silent: true,
            animation: false,
            symbol: "circle",
            symbolSize: 8,
            itemStyle: { color: p.down, borderColor: p.surface, borderWidth: 2 },
            data: [{ coord: [dates[trough], ddData[trough]] }],
            label: {
              show: true,
              position: troughRight ? "left" : "right",
              distance: 6,
              color: p.ink,
              fontFamily: p.font,
              fontSize: 11,
              backgroundColor: p.surface,
              padding: [2, 4],
              borderRadius: 4,
              formatter: () => o.labels.maxDd.replace("{v}", `${signed(ddData[trough], Math.max(1, ddDp))}%`),
            },
          }
        : undefined,
  });

  const fmtAxis = (v: number) => {
    const pct = toPct(v);
    const dp = !o.log && hi - lo < 1 ? 2 : Math.abs(pct) < 10 && hi - lo < (o.log ? 0.3 : 20) ? 1 : 0;
    return `${signed(pct, dp)}%`;
  };
  const visibleNames = lines.map((l) => l.name);
  const selected: Record<string, boolean> = {};
  for (const line of lines) selected[line.name] = !line.hidden;

  return {
    animation: false,
    textStyle: { fontFamily: p.font },
    legend: {
      type: "scroll",
      top: 0,
      left: 0,
      right: 0,
      data: visibleNames,
      selected,
      icon: "roundRect",
      itemWidth: 14,
      itemHeight: 3,
      itemGap: 14,
      textStyle: { color: p.label, fontSize: 11 },
      pageIconColor: p.ink600,
      pageIconInactiveColor: p.axis,
      pageTextStyle: { color: p.label },
    },
    grid: [
      { left, right, top, height: topH },
      { left, right, top: top + topH + gap, height: ddH },
    ],
    axisPointer: { link: [{ xAxisIndex: "all" }], label: { backgroundColor: p.ink600 } },
    tooltip: {
      ...tooltipBase(p),
      trigger: "axis",
      axisPointer: { type: "line", lineStyle: { color: p.crosshair } },
      formatter: (params: any[]) => {
        const idx = params?.[0]?.dataIndex;
        if (idx == null) return "";
        const shown = new Set(params.map((s) => s.seriesName));
        const rows = lines
          .map((line, i) => ({ line, v: transformed[i][idx] }))
          .filter(({ line, v }) => v != null && (shown.size <= 1 || shown.has(line.name)))
          .sort((a, b) => (b.v as number) - (a.v as number))
          .map(({ line, v }) => row(p.series[line.slot % p.series.length], line.name, fmtVal(v), p, !!line.emphasis));
        const dd = ddData[idx];
        return `<div style="font-weight:650;margin-bottom:4px">${esc(dates[idx] ?? "")}</div>${rows.join("")}<div style="height:1px;background:${p.line};margin:6px 0"></div>${row(p.down, o.labels.drawdown, dd == null ? "—" : `${signed(dd, 2)}%`, p)}`;
      },
    },
    xAxis: [
      {
        type: "category",
        gridIndex: 0,
        data: dates,
        boundaryGap: false,
        ...base,
        axisLine: { show: false, onZero: false },
        axisLabel: { show: false },
        splitLine: { show: false },
      },
      {
        type: "category",
        gridIndex: 1,
        data: dates,
        boundaryGap: false,
        ...base,
        axisLine: { ...base.axisLine, onZero: false },
        splitLine: { show: false },
        axisLabel: { ...base.axisLabel, hideOverlap: true, formatter: (v: string) => (n > 500 ? v.slice(0, 7) : v) },
      },
    ],
    yAxis: [
      {
        type: o.log ? "log" : "value",
        logBase: 2,
        gridIndex: 0,
        scale: true,
        ...base,
        axisLine: { show: false },
        splitNumber: 5,
        axisLabel: { ...base.axisLabel, formatter: fmtAxis },
      },
      {
        type: "value",
        gridIndex: 1,
        max: 0,
        name: o.labels.drawdown,
        nameGap: 10,
        nameTextStyle: { color: p.label, fontSize: 11, align: "left", padding: [0, 0, 0, -left + 4] },
        ...base,
        axisLine: { show: false },
        splitNumber: 2,
        axisLabel: { ...base.axisLabel, formatter: (v: number) => (v === 0 ? "0%" : `${signed(v, ddDp)}%`) },
      },
    ],
    dataZoom: zoom
      ? [
          { type: "inside", xAxisIndex: [0, 1], minValueSpan: 20 },
          {
            type: "slider",
            xAxisIndex: [0, 1],
            height: 20,
            bottom: 8,
            left,
            right,
            borderColor: p.line,
            fillerColor: p.band,
            backgroundColor: "transparent",
            dataBackground: { lineStyle: { color: p.axis, width: 1 }, areaStyle: { color: p.band } },
            selectedDataBackground: { lineStyle: { color: p.ink500, width: 1 }, areaStyle: { color: p.band } },
            handleStyle: { color: p.surface, borderColor: p.ink500 },
            moveHandleSize: 0,
            textStyle: { color: p.label, fontSize: 10 },
            labelFormatter: (_: number, value: string) => value,
          },
        ]
      : undefined,
    series,
  };
}

// ---------------------------------------------------------------------------------------------
// Rolling volatility (single series → no legend)
// ---------------------------------------------------------------------------------------------

export function rollingVolOption(dates: string[], values: (number | null)[], p: Palette, o: { name: string; reference: number | null; referenceLabel: string }): Option {
  const base = axisBase(p);
  const data = values.map((v) => (v == null ? null : +(v * 100).toFixed(3)));
  const ref = o.reference == null ? null : +(o.reference * 100).toFixed(2);
  const refText = ref == null ? "" : o.referenceLabel.replace("{v}", `${fmtNum(ref, 1)}%`);
  const right = ref == null ? 20 : Math.ceil(textWidth(refText, p.font, 11)) + 16;
  return {
    animation: false,
    textStyle: { fontFamily: p.font },
    grid: { left: 48, right, top: 16, bottom: 26 },
    tooltip: {
      ...tooltipBase(p),
      trigger: "axis",
      axisPointer: { type: "line", lineStyle: { color: p.crosshair } },
      formatter: (params: any[]) => {
        const idx = params?.[0]?.dataIndex;
        const v = data[idx];
        return `<div style="font-weight:650;margin-bottom:4px">${esc(dates[idx] ?? "")}</div>${row(p.series[0], o.name, v == null ? "—" : `${fmtNum(v, 1)}%`, p)}`;
      },
    },
    xAxis: { type: "category", data: dates, boundaryGap: false, ...base, splitLine: { show: false }, axisLabel: { ...base.axisLabel, hideOverlap: true } },
    yAxis: { type: "value", scale: true, min: 0, ...base, axisLine: { show: false }, splitNumber: 3, axisLabel: { ...base.axisLabel, formatter: (v: number) => `${fmtNum(v, 0)}%` } },
    series: [
      {
        type: "line",
        name: o.name,
        data,
        showSymbol: false,
        connectNulls: false,
        lineStyle: { width: 2, color: p.series[0] },
        itemStyle: { color: p.series[0] },
        areaStyle: { color: p.series[0], opacity: 0.08 },
        markLine:
          ref == null
            ? undefined
            : {
                silent: true,
                symbol: "none",
                lineStyle: { color: p.ink500, width: 1, type: "solid", opacity: 0.7 },
                label: { position: "end", color: p.ink600, fontSize: 11, fontFamily: p.font, formatter: () => refText },
                data: [{ yAxis: ref }],
              },
      },
    ],
  };
}

// ---------------------------------------------------------------------------------------------
// Monthly returns heatmap with a full-year column
// ---------------------------------------------------------------------------------------------

export function monthlyHeatmapOption(
  monthly: PeriodReturn[],
  yearly: PeriodReturn[],
  p: Palette,
  o: { width: number; months: string[]; yearLabel: string; rowHeight: number },
): Option {
  const months = monthly.filter((r) => r.month != null);
  const years = [...new Set([...months.map((r) => r.year), ...yearly.map((r) => r.year)])].sort((a, b) => a - b);
  const abs = months.map((r) => Math.abs(r.ret * 100)).sort((a, b) => a - b);
  // Saturate at the 90th percentile so one extreme month does not wash out the rest.
  const extent = Math.max(2, abs.length ? abs[Math.min(abs.length - 1, Math.floor(abs.length * 0.9))] : 2);
  const left = 44;
  const right = 6;
  const cell = (Math.max(260, o.width) - left - right) / 13;
  const dp = cell >= 42 ? 1 : 0;
  const showLabels = cell >= 24;
  const fontSize = cell >= 34 ? 11 : 10;
  const fill = (v: number): [number, number, number] => {
    const t = Math.max(-1, Math.min(1, v / extent));
    return t < 0 ? mix(p.divergeMid, p.down, -t) : mix(p.divergeMid, p.up, t);
  };
  const monthData = months.map((r) => {
    const v = +(r.ret * 100).toFixed(2);
    return {
      value: [(r.month as number) - 1, years.indexOf(r.year), v],
      label: { color: textOn(fill(v), p) },
    };
  });
  const yearData = yearly
    .filter((r) => r.month == null)
    .map((r) => ({ value: [12, years.indexOf(r.year), +(r.ret * 100).toFixed(2)] }));
  // Neutral tint for the full-year column: ink at low alpha reads as a quiet cell in both themes.
  const inkRgb = parseColor(p.ink) ?? [128, 128, 128, 1];
  const yearFill = `rgba(${inkRgb[0]}, ${inkRgb[1]}, ${inkRgb[2]}, 0.07)`;
  const base = axisBase(p);
  const label = (bold: boolean) => ({
    show: showLabels,
    fontSize,
    fontFamily: p.font,
    fontWeight: bold ? 650 : 400,
    formatter: (param: any) => signed(param.value[2], dp),
  });
  return {
    animation: false,
    textStyle: { fontFamily: p.font },
    grid: { left, right, top: 26, bottom: 44 },
    tooltip: {
      ...tooltipBase(p),
      formatter: (param: any) => {
        const [m, y, v] = param.value;
        const when = m === 12 ? `${years[y]} · ${o.yearLabel}` : `${years[y]}-${String(m + 1).padStart(2, "0")}`;
        return `<div style="font-weight:650;margin-bottom:2px">${esc(when)}</div><div style="font-variant-numeric:tabular-nums;font-weight:650">${signed(v, 2)}%</div>`;
      },
    },
    xAxis: {
      type: "category",
      position: "top",
      data: [...o.months, o.yearLabel],
      ...base,
      axisLine: { show: false },
      splitLine: { show: false },
      axisLabel: {
        ...base.axisLabel,
        interval: 0,
        fontSize: cell < 34 ? 10 : 11,
        formatter: (v: string) => {
          if (cell >= 34 || v === o.yearLabel) return v;
          return /月$/.test(v) ? v.replace("月", "") : v.slice(0, 1);
        },
      },
    },
    yAxis: {
      type: "category",
      data: years.map(String),
      inverse: true,
      ...base,
      axisLine: { show: false },
      splitLine: { show: false },
      axisLabel: { ...base.axisLabel, fontFamily: p.font },
    },
    visualMap: [
      {
      type: "continuous",
      seriesIndex: 0,
      min: -extent,
      max: extent,
      calculable: false,
      orient: "horizontal",
      left: "center",
      bottom: 4,
      itemWidth: 10,
      itemHeight: 120,
      text: [`+${fmtNum(extent, 0)}%`, `−${fmtNum(extent, 0)}%`],
      textGap: 8,
      textStyle: { color: p.label, fontSize: 10 },
      inRange: { color: [p.down, p.divergeMid, p.up] },
      },
      { type: "piecewise", seriesIndex: 1, show: false, pieces: [{ min: -1e12, max: 1e12 }], inRange: { color: [yearFill] }, outOfRange: { color: [yearFill] } },
    ],
    series: [
      {
        type: "heatmap",
        data: monthData,
        label: label(false),
        itemStyle: { borderColor: p.surface, borderWidth: 2, borderRadius: 4 },
        emphasis: { itemStyle: { borderColor: p.ink, borderWidth: 1 } },
      },
      {
        type: "heatmap",
        data: yearData,
        label: { ...label(true), color: p.ink },
        itemStyle: { borderColor: p.surface, borderWidth: 2, borderRadius: 4 },
        emphasis: { itemStyle: { borderColor: p.ink, borderWidth: 1 } },
      },
    ],
  };
}

// ---------------------------------------------------------------------------------------------
// Calendar-year returns: portfolio vs one benchmark
// ---------------------------------------------------------------------------------------------

export function yearlyBarsOption(
  years: number[],
  series: { name: string; values: (number | null)[]; slot: number }[],
  p: Palette,
  o: { partial: Set<number>; excessLabel: string },
): Option {
  const base = axisBase(p);
  const labels = years.map((y) => (o.partial.has(y) ? `${y}*` : String(y)));
  return {
    animation: false,
    textStyle: { fontFamily: p.font },
    legend: { top: 0, left: 0, icon: "roundRect", itemWidth: 12, itemHeight: 8, textStyle: { color: p.label, fontSize: 11 } },
    grid: { left: 48, right: 12, top: 34, bottom: 26 },
    tooltip: {
      ...tooltipBase(p),
      trigger: "axis",
      axisPointer: { type: "shadow", shadowStyle: { color: p.band } },
      formatter: (params: any[]) => {
        const idx = params?.[0]?.dataIndex;
        if (idx == null) return "";
        const rows = series.map((s) => {
          const v = s.values[idx];
          return row(p.series[s.slot % p.series.length], s.name, v == null ? "—" : `${signed(v * 100, 2)}%`, p);
        });
        const a = series[0]?.values[idx];
        const b = series[1]?.values[idx];
        if (a != null && b != null) rows.push(`<div style="height:1px;background:${p.line};margin:6px 0"></div>${row(null, o.excessLabel, `${signed((a - b) * 100, 2)}%`, p)}`);
        return `<div style="font-weight:650;margin-bottom:4px">${esc(labels[idx])}</div>${rows.join("")}`;
      },
    },
    xAxis: {
      type: "category",
      data: labels,
      ...base,
      axisLine: { ...base.axisLine, onZero: false },
      splitLine: { show: false },
      axisLabel: { ...base.axisLabel, interval: 0, hideOverlap: true },
    },
    yAxis: {
      type: "value",
      ...base,
      axisLine: { show: false },
      splitNumber: 4,
      axisLabel: { ...base.axisLabel, formatter: (v: number) => `${v < 0 ? "−" : ""}${fmtNum(Math.abs(v), 0)}%` },
    },
    series: series.map((s, i) => ({
      type: "bar",
      name: s.name,
      data: s.values.map((v) =>
        v == null
          ? null
          : { value: +(v * 100).toFixed(2), itemStyle: { borderRadius: v >= 0 ? [4, 4, 0, 0] : [0, 0, 4, 4] } },
      ),
      barMaxWidth: 18,
      barGap: "20%",
      itemStyle: { color: p.series[s.slot % p.series.length] },
      markLine:
        i === 0
          ? { silent: true, symbol: "none", label: { show: false }, lineStyle: { color: p.axis, width: 1, type: "solid" }, data: [{ yAxis: 0 }] }
          : undefined,
    })),
  };
}

// ---------------------------------------------------------------------------------------------
// Sector weights over time (sequential heatmap; cash on its own neutral scale)
// ---------------------------------------------------------------------------------------------

export function sectorWeightsOption(
  samples: { date: string; weights: Record<string, number>; cash: number }[],
  sectors: { id: string; name: string }[],
  p: Palette,
  o: { width: number; cashLabel: string; weightLabel: string },
): Option {
  const dates = samples.map((s) => s.date);
  const names = [...sectors.map((s) => s.name), o.cashLabel];
  const data: [number, number, number][] = [];
  const cash: [number, number, number][] = [];
  samples.forEach((s, x) => {
    sectors.forEach((sector, y) => data.push([x, y, +((s.weights[sector.id] ?? 0) * 100).toFixed(2)]));
    cash.push([x, sectors.length, +(s.cash * 100).toFixed(2)]);
  });
  const max = Math.max(1, ...data.map((d) => d[2]));
  const cashMax = Math.max(1, ...cash.map((d) => d[2]));
  const narrow = o.width > 0 && o.width < 440;
  const maxLabel = narrow ? 92 : 190;
  const longest = Math.max(...names.map((n) => textWidth(n, p.font, 11)));
  const left = Math.min(maxLabel, Math.ceil(longest) + 8) + 14;
  const cellW = (Math.max(300, o.width) - left - 28) / Math.max(1, dates.length);
  const border = cellW >= 6 ? 1 : 0;
  const base = axisBase(p);
  const span = dates.length ? Number(dates[dates.length - 1].slice(0, 4)) - Number(dates[0].slice(0, 4)) : 0;
  return {
    animation: false,
    textStyle: { fontFamily: p.font },
    grid: { left, right: 28, top: 6, bottom: 52 },
    tooltip: {
      ...tooltipBase(p),
      formatter: (param: any) => {
        const [x, y, v] = param.value;
        return `<div style="font-weight:650;margin-bottom:2px">${esc(names[y])}</div><div style="color:${p.ink600}">${esc(dates[x])}</div>${row(null, o.weightLabel, `${fmtNum(v, 1)}%`, p)}`;
      },
    },
    xAxis: {
      type: "category",
      data: dates,
      ...base,
      axisLine: { show: false },
      splitLine: { show: false },
      axisLabel: { ...base.axisLabel, hideOverlap: true, formatter: (v: string) => (span >= 2 ? v.slice(0, 7) : v.slice(5)) },
    },
    yAxis: {
      type: "category",
      data: names,
      inverse: true,
      ...base,
      axisLine: { show: false },
      splitLine: { show: false },
      axisLabel: { ...base.axisLabel, width: left - 14, overflow: "truncate", ellipsis: "…" },
    },
    visualMap: [
      {
        type: "continuous",
        seriesIndex: 0,
        min: 0,
        max,
        calculable: false,
        orient: "horizontal",
        left: "center",
        bottom: 2,
        itemWidth: 10,
        itemHeight: 120,
        text: [`${fmtNum(max, 0)}%`, "0%"],
        textGap: 8,
        textStyle: { color: p.label, fontSize: 10 },
        inRange: { color: [p.seq[0], p.seq[2], p.seq[4]] },
      },
      {
        type: "continuous",
        seriesIndex: 1,
        show: false,
        min: 0,
        max: cashMax,
        inRange: { color: [p.divergeMid, p.ink500] },
      },
    ],
    series: [
      { type: "heatmap", data, itemStyle: { borderColor: p.surface, borderWidth: border }, emphasis: { itemStyle: { borderColor: p.ink, borderWidth: 1 } }, progressive: 0 },
      { type: "heatmap", data: cash, itemStyle: { borderColor: p.surface, borderWidth: border }, emphasis: { itemStyle: { borderColor: p.ink, borderWidth: 1 } }, progressive: 0 },
    ],
  };
}

// ---------------------------------------------------------------------------------------------
// Horizontal contribution bars (single series; sign carried by colour AND the signed label)
// ---------------------------------------------------------------------------------------------

export function contributionBarsOption(
  rows: { label: string; short?: string; value: number; detail?: string }[],
  p: Palette,
  o: { width: number },
): Option {
  const narrow = o.width > 0 && o.width < 440;
  const sorted = [...rows].sort((a, b) => a.value - b.value).map((r) => ({ ...r, label: narrow && r.short ? r.short : r.label, full: r.label }));
  const longest = Math.max(0, ...sorted.map((r) => textWidth(r.label, p.font, 11)));
  const left = Math.min(narrow ? 116 : 210, Math.ceil(longest) + 8) + 14;
  const base = axisBase(p);
  const values = sorted.map((r) => r.value * 100);
  const hasNeg = values.some((v) => v < 0);
  const hasPos = values.some((v) => v > 0);
  const labelW = 56;
  return {
    animation: false,
    textStyle: { fontFamily: p.font },
    grid: { left: left + (hasNeg ? labelW : 0), right: hasPos ? labelW : 16, top: 4, bottom: 24 },
    tooltip: {
      ...tooltipBase(p),
      trigger: "item",
      formatter: (param: any) => {
        const r = sorted[param.dataIndex];
        return `<div style="font-weight:650;margin-bottom:2px">${esc(r.full)}</div><div style="font-variant-numeric:tabular-nums;font-weight:650">${signed(r.value * 100, 2)}%</div>${r.detail ? `<div style="color:${p.ink600}">${esc(r.detail)}</div>` : ""}`;
      },
    },
    xAxis: {
      type: "value",
      ...base,
      splitNumber: narrow ? 2 : 5,
      axisLabel: { ...base.axisLabel, hideOverlap: true, formatter: (v: number) => `${v < 0 ? "−" : ""}${fmtNum(Math.abs(v), Math.abs(v) < 2 && v !== 0 ? 1 : 0)}%` },
    },
    yAxis: {
      type: "category",
      data: sorted.map((r) => r.label),
      ...base,
      axisLine: { show: false, onZero: false },
      splitLine: { show: false },
      axisLabel: { ...base.axisLabel, width: left - 14, overflow: "truncate", ellipsis: "…", margin: hasNeg ? labelW + 8 : 8 },
    },
    series: [
      {
        type: "bar",
        data: sorted.map((r) => {
          const v = +(r.value * 100).toFixed(3);
          const neg = v < 0;
          return {
            value: v,
            itemStyle: { color: neg ? p.down : p.up, borderRadius: neg ? [4, 0, 0, 4] : [0, 4, 4, 0] },
            label: { position: neg ? "left" : "right" },
          };
        }),
        barMaxWidth: 14,
        barCategoryGap: "35%",
        label: {
          show: true,
          color: p.ink600,
          fontSize: 11,
          fontFamily: p.font,
          distance: 6,
          formatter: (param: any) => `${signed(param.value, 2)}%`,
        },
      },
    ],
  };
}

/** Formats a return for chart captions. */
export const pctLabel = (v: number | null | undefined, dp = 1) => fmtPct(v, { dp, sign: true });
