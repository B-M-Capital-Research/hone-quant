/**
 * ECharts, tree-shaken to the chart types hone-quant uses, plus a palette read from the CSS
 * design tokens at render time so every chart follows the theme and the up/down convention.
 */
import * as echarts from "echarts/core";
import { BarChart, CandlestickChart, HeatmapChart, LineChart, ScatterChart } from "echarts/charts";
import {
  AxisPointerComponent,
  DataZoomComponent,
  GridComponent,
  LegendComponent,
  MarkAreaComponent,
  MarkLineComponent,
  MarkPointComponent,
  TooltipComponent,
  VisualMapComponent,
} from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";

echarts.use([
  BarChart,
  CandlestickChart,
  HeatmapChart,
  LineChart,
  ScatterChart,
  AxisPointerComponent,
  DataZoomComponent,
  GridComponent,
  LegendComponent,
  MarkAreaComponent,
  MarkLineComponent,
  MarkPointComponent,
  TooltipComponent,
  VisualMapComponent,
  CanvasRenderer,
]);

export { echarts };

export interface Palette {
  up: string;
  down: string;
  upInk: string;
  downInk: string;
  upSoft: string;
  downSoft: string;
  flat: string;
  surface: string;
  grid: string;
  axis: string;
  label: string;
  band: string;
  crosshair: string;
  ink: string;
  ink600: string;
  ink500: string;
  line: string;
  paper: string;
  coral: string;
  series: string[];
  seq: string[];
  divergeMid: string;
  font: string;
  mono: string;
}

function token(style: CSSStyleDeclaration, name: string, fallback: string): string {
  const value = style.getPropertyValue(name).trim();
  return value || fallback;
}

export function readPalette(): Palette {
  const style = getComputedStyle(document.documentElement);
  const t = (name: string, fallback = "#888888") => token(style, name, fallback);
  return {
    up: t("--hq-up"),
    down: t("--hq-down"),
    upInk: t("--hq-up-ink"),
    downInk: t("--hq-down-ink"),
    upSoft: t("--hq-up-soft"),
    downSoft: t("--hq-down-soft"),
    flat: t("--hq-flat"),
    surface: t("--hq-chart-surface", "#ffffff"),
    grid: t("--hq-chart-grid"),
    axis: t("--hq-chart-axis"),
    label: t("--hq-chart-label"),
    band: t("--hq-chart-band"),
    crosshair: t("--hq-chart-crosshair"),
    ink: t("--hone-ink-950"),
    ink600: t("--hone-ink-600"),
    ink500: t("--hone-ink-500"),
    line: t("--hone-line"),
    paper: t("--hone-paper-100"),
    coral: t("--hone-coral-600"),
    series: [1, 2, 3, 4, 5].map((i) => t(`--hq-series-${i}`)),
    seq: ["100", "250", "400", "550", "700"].map((s) => t(`--hq-seq-${s}`)),
    divergeMid: t("--hq-diverge-mid"),
    font: t("--hone-font-body", "sans-serif"),
    mono: t("--hone-font-label", "monospace"),
  };
}

/** Common tooltip styling: surface card, hairline border, readable ink. */
export function tooltipBase(p: Palette) {
  return {
    backgroundColor: p.surface,
    borderColor: p.line,
    borderWidth: 1,
    padding: [10, 12],
    textStyle: { color: p.ink, fontFamily: p.font, fontSize: 12 },
    extraCssText: "border-radius:10px;box-shadow:0 12px 32px rgba(0,0,0,0.12);",
  };
}

export function axisBase(p: Palette) {
  return {
    axisLine: { lineStyle: { color: p.axis } },
    axisTick: { show: false },
    axisLabel: { color: p.label, fontFamily: p.font, fontSize: 11 },
    splitLine: { lineStyle: { color: p.grid, width: 1 } },
  };
}

/** Escapes text for tooltip HTML (names come from data). */
export function esc(value: unknown): string {
  return String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}
