import { For, type JSX, type ParentProps, Show } from "solid-js";
import { Chart } from "@/components/Chart";
import { Icon, type IconName } from "@/components/Icon";
import { Kpi, Pct } from "@/components/ui";
import { tpl } from "@/i18n";
import { researchText } from "@/i18n/research";
import { fmtNum, fmtPct } from "@/lib/format";
import type { BacktestRow, BacktestWarning, PerformanceMetrics } from "@/lib/types";
import { createWidth } from "./model";
import "@/styles/research.css";

type Option = Record<string, unknown>;

// ---------------------------------------------------------------------------------------------
// Icons not in the shared set
// ---------------------------------------------------------------------------------------------

/** Two overlapping sheets, drawn in the shared icon language (24×24, 1.7px strokes). */
export function CopyIcon(props: { size?: number }) {
  const size = () => props.size ?? 16;
  return (
    <svg width={size()} height={size()} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M9 9h10a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H11a2 2 0 0 1-2-2zM5 15H4.5A1.5 1.5 0 0 1 3 13.5V5a2 2 0 0 1 2-2h8.5A1.5 1.5 0 0 1 15 4.5V5" />
    </svg>
  );
}

// ---------------------------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------------------------

const STATUS_TONE: Record<BacktestRow["status"], string> = {
  queued: "outline",
  running: "blue",
  succeeded: "green",
  failed: "red",
};

export function BacktestStatusChip(props: { status: BacktestRow["status"] }) {
  return (
    <span class={`chip ${STATUS_TONE[props.status] ?? ""}`}>
      <Show when={props.status === "running"} fallback={<span class="dot" />}>
        <span class="rs-chip-spinner" aria-hidden="true" />
      </Show>
      {researchText().status[props.status] ?? props.status}
    </span>
  );
}

/** Indeterminate progress (the server reports state, not percentage). */
export function IndeterminateBar() {
  return (
    <div class="rs-indeterminate" role="progressbar" aria-busy="true">
      <div />
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// Cards & charts
// ---------------------------------------------------------------------------------------------

export function Card(props: ParentProps<{ title: string; sub?: JSX.Element; actions?: JSX.Element; flush?: boolean; class?: string; id?: string }>) {
  return (
    <section class={`card ${props.class ?? ""}`} id={props.id}>
      <div class="card-head">
        <div class="rs-head-text">
          <h2>{props.title}</h2>
          <Show when={props.sub}>
            <div class="sub">{props.sub}</div>
          </Show>
        </div>
        <Show when={props.actions}>
          <div class="rs-head-actions">{props.actions}</div>
        </Show>
      </div>
      <div class={`card-body ${props.flush ? "flush" : ""}`}>{props.children}</div>
    </section>
  );
}

/**
 * A chart that knows its own width, so options can adapt labels and paddings. `scroll` keeps a
 * minimum width and lets the card scroll sideways on phones (for dense grids such as heatmaps).
 */
export function ChartBox(props: { option: (width: number) => Option | null; height: number; ariaLabel?: string; busy?: boolean; scroll?: boolean }) {
  const [width, ref] = createWidth();
  const chart = (
    <div ref={ref} class="rs-chart">
      <Chart option={() => (width() > 0 ? props.option(width()) : null)} height={props.height} ariaLabel={props.ariaLabel} busy={props.busy} />
    </div>
  );
  return props.scroll ? <div class="rs-scroll-x">{chart}</div> : chart;
}

/** Placeholder for a chart that cannot be drawn yet (short history). */
export function NotYet(props: { icon?: IconName; title: string; detail?: string; height?: number }) {
  return (
    <div class="rs-notyet" style={{ "min-height": `${props.height ?? 160}px` }}>
      <Icon name={props.icon ?? "clock"} size={20} />
      <div class="title">{props.title}</div>
      <Show when={props.detail}>
        <div class="detail">{props.detail}</div>
      </Show>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------------------------

/** Ratios with a typographic minus, like the percentages. */
export const ratio = (v: number | null | undefined) => fmtNum(v, 2).replace(/^-/, "−");

/** Annualised values are muted and starred when the sample is too short to trust them. */
export function Annualised(props: ParentProps<{ reliable: boolean }>) {
  return <span class={props.reliable ? "" : "rs-unreliable"}>{props.children}</span>;
}

export function KpiStrip(props: ParentProps<{ title?: string; aside?: JSX.Element }>) {
  return (
    <div class="rs-kpi-block">
      <Show when={props.title || props.aside}>
        <div class="rs-kpi-title">
          <span class="kicker">{props.title}</span>
          <Show when={props.aside}>
            <span class="muted xs">{props.aside}</span>
          </Show>
        </div>
      </Show>
      <div class="kpis rs-kpis">{props.children}</div>
    </div>
  );
}

/** Return-and-risk tiles shared by backtests and the live account (the NAV tile leads). */
export function ReturnRiskKpis(props: { m: PerformanceMetrics; lead: JSX.Element; totalDelta?: JSX.Element; cagrDelta?: JSX.Element; sharpeDelta?: JSX.Element }) {
  const r = researchText;
  const star = (label: string) => (props.m.annualisation_reliable ? label : `${label} *`);
  const rel = () => props.m.annualisation_reliable;
  return (
    <>
      {props.lead}
      <Kpi label={r().metrics.total_return} title={r().metrics.help.total_return} value={<Pct value={props.m.total_return} />} delta={props.totalDelta} />
      <Kpi
        label={star(r().metrics.cagr)}
        title={r().metrics.help.cagr}
        value={
          <Annualised reliable={rel()}>
            <Pct value={props.m.cagr} />
          </Annualised>
        }
        delta={props.cagrDelta}
      />
      <Kpi
        label={star(r().metrics.ann_vol)}
        title={r().metrics.help.ann_vol}
        value={<Annualised reliable={rel()}>{fmtPct(props.m.ann_vol, { dp: 2 })}</Annualised>}
        delta={
          <Show when={props.m.var_95 != null}>
            <span class="muted">{tpl(r().detail.var_short, { v: fmtPct(props.m.var_95, { dp: 2, sign: true }) })}</span>
          </Show>
        }
      />
      <Kpi
        label={star(r().metrics.sharpe)}
        title={r().metrics.help.sharpe}
        value={<Annualised reliable={rel()}>{ratio(props.m.sharpe)}</Annualised>}
        delta={props.sharpeDelta}
      />
      <Kpi label={star(r().metrics.sortino)} title={r().metrics.help.sortino} value={<Annualised reliable={rel()}>{ratio(props.m.sortino)}</Annualised>} />
      <Kpi
        label={r().metrics.max_drawdown}
        title={r().metrics.help.max_drawdown}
        value={<span class={`num ${props.m.max_drawdown < 0 ? "down" : ""}`}>{fmtPct(props.m.max_drawdown, { dp: 2, sign: true })}</span>}
        delta={<DrawdownPath m={props.m} />}
      />
      <Kpi label={star(r().metrics.calmar)} title={r().metrics.help.calmar} value={<Annualised reliable={rel()}>{ratio(props.m.calmar)}</Annualised>} />
    </>
  );
}

export function DrawdownPath(props: { m: PerformanceMetrics }) {
  const r = researchText;
  return (
    <Show when={props.m.max_drawdown_peak && props.m.max_drawdown_trough} fallback={<span class="muted">—</span>}>
      <span class="rs-ddpath">
        <span class="muted">
          <span class="nowrap">{props.m.max_drawdown_peak}</span> → <span class="nowrap">{props.m.max_drawdown_trough}</span>
        </span>
        <span class={props.m.max_drawdown_recovery ? "muted" : "rs-warn-ink"}>
          {props.m.max_drawdown_recovery ? tpl(r().detail.dd_recovered, { date: props.m.max_drawdown_recovery }) : r().detail.dd_not_recovered}
        </span>
      </span>
    </Show>
  );
}

/** Relative-to-benchmark tiles. */
export function RelativeKpis(props: { m: PerformanceMetrics; bench: string }) {
  const r = researchText;
  const rel = () => props.m.annualisation_reliable;
  const star = (label: string) => (rel() ? label : `${label} *`);
  return (
    <>
      <Kpi
        label={`${r().metrics.excess_return} · ${props.bench}`}
        title={r().metrics.help.excess_return}
        value={<Pct value={props.m.excess_return} />}
        delta={
          <Show when={props.m.benchmark_total_return != null}>
            <span class="muted">{tpl(r().detail.benchmark_return, { v: fmtPct(props.m.benchmark_total_return, { dp: 2, sign: true }) })}</span>
          </Show>
        }
      />
      <Kpi
        label={r().metrics.beta}
        title={r().metrics.help.beta}
        value={ratio(props.m.beta)}
        delta={
          <Show when={props.m.correlation != null}>
            <span class="muted">{tpl(r().detail.correlation, { v: ratio(props.m.correlation) })}</span>
          </Show>
        }
      />
      <Kpi
        label={star(r().metrics.alpha)}
        title={r().metrics.help.alpha}
        value={
          <Annualised reliable={rel()}>
            <Pct value={props.m.alpha} />
          </Annualised>
        }
      />
      <Kpi
        label={star(r().metrics.information_ratio)}
        title={r().metrics.help.information_ratio}
        value={<Annualised reliable={rel()}>{ratio(props.m.information_ratio)}</Annualised>}
        delta={
          <Show when={props.m.tracking_error != null}>
            <span class="muted">{tpl(r().detail.tracking_error, { v: fmtPct(props.m.tracking_error, { dp: 2 }) })}</span>
          </Show>
        }
      />
      <Kpi
        label={r().metrics.hit_rate}
        title={r().metrics.help.hit_rate}
        value={fmtPct(props.m.hit_rate, { dp: 1 })}
        delta={
          <Show when={props.m.best_day != null}>
            <span class="muted">
              {tpl(r().detail.best_worst, { best: fmtPct(props.m.best_day, { dp: 2, sign: true }), worst: fmtPct(props.m.worst_day, { dp: 2, sign: true }) })}
            </span>
          </Show>
        }
      />
    </>
  );
}

interface MetricRow {
  key: string;
  label: string;
  annualised?: boolean;
  render: (m: PerformanceMetrics) => JSX.Element;
}

function metricRows(): MetricRow[] {
  const r = researchText().metrics;
  const pct = (v: number | null | undefined) => <Pct value={v} />;
  const plain = (v: number | null | undefined, dp = 2) => <span class="num">{fmtPct(v, { dp })}</span>;
  return [
    { key: "total_return", label: r.total_return, render: (m) => pct(m.total_return) },
    { key: "cagr", label: r.cagr, annualised: true, render: (m) => pct(m.cagr) },
    { key: "ann_vol", label: r.ann_vol, annualised: true, render: (m) => plain(m.ann_vol) },
    { key: "sharpe", label: r.sharpe, annualised: true, render: (m) => <span class="num">{ratio(m.sharpe)}</span> },
    { key: "sortino", label: r.sortino, annualised: true, render: (m) => <span class="num">{ratio(m.sortino)}</span> },
    { key: "max_drawdown", label: r.max_drawdown, render: (m) => <span class={`num ${m.max_drawdown < 0 ? "down" : ""}`}>{fmtPct(m.max_drawdown, { dp: 2, sign: true })}</span> },
    { key: "calmar", label: r.calmar, annualised: true, render: (m) => <span class="num">{ratio(m.calmar)}</span> },
    { key: "longest", label: r.longest_drawdown, render: (m) => <span class="num">{m.longest_drawdown_days ? tpl(r.days, { n: fmtNum(m.longest_drawdown_days, 0) }) : "—"}</span> },
    { key: "hit_rate", label: r.hit_rate, render: (m) => plain(m.hit_rate, 1) },
    { key: "best_day", label: r.best_day, render: (m) => pct(m.best_day) },
    { key: "worst_day", label: r.worst_day, render: (m) => pct(m.worst_day) },
    { key: "var_95", label: r.var_95, render: (m) => pct(m.var_95) },
    { key: "cvar_95", label: r.cvar_95, render: (m) => pct(m.cvar_95) },
    { key: "skew", label: r.skew, render: (m) => <span class="num">{ratio(m.skew)}</span> },
  ];
}

export interface MetricsColumn {
  key: string;
  name: string;
  detail?: string;
  slot: number;
  primary?: boolean;
  metrics: PerformanceMetrics | null;
}

/** Portfolio and benchmarks side by side (absolute metrics only; relative ones are KPIs). */
export function MetricsTable(props: { columns: MetricsColumn[]; reliable: boolean }) {
  return (
    <div class="table-wrap">
      <table class="table compact rs-metrics-table">
        <thead>
          <tr>
            <th />
            <For each={props.columns}>
              {(col) => (
                <th class="r" title={col.detail}>
                  <span class="rs-series-key">
                    <i style={{ background: `var(--hq-series-${(col.slot % 5) + 1})` }} />
                    {col.name}
                  </span>
                  <Show when={col.primary}>
                    <span class="rs-primary-tag">{researchText().detail.primary}</span>
                  </Show>
                </th>
              )}
            </For>
          </tr>
        </thead>
        <tbody>
          <For each={metricRows()}>
            {(metric) => (
              <tr>
                <td class="rs-metric-label">
                  {metric.label}
                  {metric.annualised && !props.reliable ? " *" : ""}
                </td>
                <For each={props.columns}>
                  {(col) => (
                    <td class={`r ${metric.annualised && !props.reliable ? "rs-unreliable" : ""}`}>
                      {col.metrics ? metric.render(col.metrics) : <span class="muted">—</span>}
                    </td>
                  )}
                </For>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// Warnings & methodology
// ---------------------------------------------------------------------------------------------

export function WarningCallouts(props: { warnings: BacktestWarning[]; names: (symbol: string) => string }) {
  const r = researchText;
  return (
    <div class="rs-warnings">
      <For each={props.warnings}>
        {(w) => {
          switch (w.code) {
            case "survivorship_bias":
              return (
                <div class="callout info">
                  <Icon name="info" size={16} />
                  <div>
                    <strong>{r().warnings.survivorship_title}</strong> · {r().warnings.survivorship_body}
                  </div>
                </div>
              );
            case "late_listings":
              return (
                <div class="callout warn">
                  <Icon name="calendar" size={16} />
                  <div class="rs-callout-body">
                    <div>
                      <strong>{tpl(r().warnings.late_title, { n: w.symbols.length })}</strong> · {r().warnings.late_body}
                    </div>
                    <div class="rs-symbol-list">
                      <For each={[...w.symbols].sort((a, b) => a[1].localeCompare(b[1]))}>
                        {([symbol, date]) => (
                          <span class="rs-symbol-pill" title={props.names(symbol)}>
                            <b>{symbol}</b>
                            <span>{date}</span>
                          </span>
                        )}
                      </For>
                    </div>
                  </div>
                </div>
              );
            case "missing_data":
              return (
                <div class="callout warn">
                  <Icon name="alert" size={16} />
                  <div class="rs-callout-body">
                    <div>
                      <strong>{tpl(r().warnings.missing_title, { n: w.symbols.length })}</strong> · {r().warnings.missing_body}
                    </div>
                    <div class="rs-symbol-list">
                      <For each={w.symbols}>
                        {(symbol) => (
                          <span class="rs-symbol-pill" title={props.names(symbol)}>
                            <b>{symbol}</b>
                          </span>
                        )}
                      </For>
                    </div>
                  </div>
                </div>
              );
            case "short_period":
              return (
                <div class="callout warn">
                  <Icon name="clock" size={16} />
                  <div>
                    <strong>{r().warnings.short_title}</strong> · {r().warnings.short_body}
                  </div>
                </div>
              );
            default:
              return null;
          }
        }}
      </For>
    </div>
  );
}

export function MethodologyCard() {
  const r = researchText;
  const items = (): { icon: IconName; title: string; body: string }[] => [
    { icon: "alert", title: r().method.survivorship_title, body: r().method.survivorship_body },
    { icon: "layers", title: r().method.adjusted_title, body: r().method.adjusted_body },
    { icon: "shield", title: r().method.lookahead_title, body: r().method.lookahead_body },
    { icon: "trades", title: r().method.fills_title, body: r().method.fills_body },
    { icon: "zap", title: r().method.engine_title, body: r().method.engine_body },
  ];
  return (
    <Card title={r().method.title} sub={r().method.sub} id="method">
      <div class="rs-method">
        <For each={items()}>
          {(item) => (
            <div class="rs-method-item">
              <span class="rs-method-icon">
                <Icon name={item.icon} size={16} />
              </span>
              <div>
                <div class="rs-method-title">{item.title}</div>
                <p>{item.body}</p>
              </div>
            </div>
          )}
        </For>
      </div>
    </Card>
  );
}
