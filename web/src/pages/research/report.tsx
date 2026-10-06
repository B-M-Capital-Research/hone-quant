/**
 * Report sections shared by the backtest detail and the live performance page: equity and
 * drawdown, monthly and calendar-year returns, contributions, sector weights and rolling
 * volatility. Every chart has a text equivalent close by (tables, labels or tooltips).
 */
import { For, Show, createMemo, createSignal } from "solid-js";
import { Switch as Toggle } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { researchText } from "@/i18n/research";
import { readPalette } from "@/lib/charts/echarts";
import { fmtPct } from "@/lib/format";
import type { Attribution, PeriodReturn } from "@/lib/types";
import {
  canLog,
  contributionBarsOption,
  equityDrawdownOption,
  type EquityLine,
  monthlyHeatmapOption,
  rollingVolOption,
  sectorWeightsOption,
  yearlyBarsOption,
} from "./charts";
import { Card, ChartBox, NotYet } from "./components";
import { createMedia, partialYears, yearlyReturns } from "./model";

export type { EquityLine };

/** Drops a first period that only holds the base value (e.g. the prior year-end close). */
export function dropBaseOnly(returns: PeriodReturn[], dates: string[], monthly: boolean): PeriodReturn[] {
  if (dates.length < 2 || !returns.length) return returns;
  const key = (d: string) => (monthly ? d.slice(0, 7) : d.slice(0, 4));
  if (key(dates[0]) === key(dates[1])) return returns;
  const year = Number(dates[0].slice(0, 4));
  const month = Number(dates[0].slice(5, 7));
  const first = returns[0];
  const isBase = first.year === year && (monthly ? first.month === month : first.month == null);
  return isBase ? returns.slice(1) : returns;
}

export function EquityCard(props: { dates: string[]; lines: EquityLine[]; drawdowns: number[] }) {
  const r = researchText;
  const narrow = createMedia("(max-width: 760px)");
  const [log, setLog] = createSignal(false);
  const logAvailable = createMemo(() => canLog(props.lines));
  const height = () => (narrow() ? 380 : 460);
  return (
    <Card
      title={r().charts.equity}
      sub={r().charts.equity_sub}
      actions={
        <Show when={logAvailable()}>
          <Toggle checked={log()} onChange={setLog} label={<span class="xs">{r().charts.log}</span>} />
        </Show>
      }
    >
      <ChartBox
        height={height()}
        ariaLabel={r().charts.equity}
        option={(width) =>
          equityDrawdownOption(props.dates, props.lines, props.drawdowns, readPalette(), {
            width,
            height: height(),
            log: log() && logAvailable(),
            labels: { cumulative: r().charts.cumulative, drawdown: r().charts.drawdown, maxDd: r().charts.max_dd },
          })
        }
      />
    </Card>
  );
}

export function MonthlyCard(props: { monthly: PeriodReturn[]; yearly: PeriodReturn[] }) {
  const r = researchText;
  const narrow = createMedia("(max-width: 760px)");
  const years = createMemo(() => new Set([...props.monthly.map((m) => m.year), ...props.yearly.map((y) => y.year)]).size);
  const rowHeight = () => (narrow() ? 26 : 30);
  const height = () => years() * rowHeight() + 26 + 52;
  return (
    <Card title={r().charts.monthly} sub={r().charts.monthly_sub}>
      <ChartBox
        height={height()}
        ariaLabel={r().charts.monthly}
        scroll
        option={(width) =>
          monthlyHeatmapOption(props.monthly, props.yearly, readPalette(), {
            width,
            months: r().charts.months as string[],
            yearLabel: r().charts.year_total,
            rowHeight: rowHeight(),
          })
        }
      />
    </Card>
  );
}

/** Calendar-year returns of the portfolio vs the primary benchmark, plus the numbers as a table. */
export function YearlyCard(props: {
  dates: string[];
  yearly: PeriodReturn[];
  bench: { name: string; values: (number | null)[]; slot: number } | null;
  portfolioName: string;
}) {
  const r = researchText;
  const years = createMemo(() => props.yearly.filter((y) => y.month == null).map((y) => y.year));
  const benchYearly = createMemo(() => (props.bench ? yearlyReturns(props.dates, props.bench.values) : new Map<number, number>()));
  const partial = createMemo(() => partialYears(props.dates));
  const partialSet = createMemo(() => {
    const set = new Set<number>();
    const p = partial();
    if (p.first && years().includes(p.first.year)) set.add(p.first.year);
    if (p.last && years().includes(p.last.year)) set.add(p.last.year);
    return set;
  });
  const partialNote = createMemo(() => {
    const p = partial();
    const parts: string[] = [];
    if (p.first && partialSet().has(p.first.year)) parts.push(tpl(r().charts.partial_from, { year: p.first.year, date: p.first.date.slice(5) }));
    if (p.last && partialSet().has(p.last.year)) parts.push(tpl(r().charts.partial_to, { year: p.last.year, date: p.last.date.slice(5) }));
    return parts.length ? tpl(r().charts.yearly_partial, { list: parts.join(locale() === "zh" ? "；" : "; ") }) : null;
  });
  const portfolio = createMemo(() => years().map((y) => props.yearly.find((x) => x.year === y && x.month == null)?.ret ?? null));
  const bench = createMemo(() => years().map((y) => benchYearly().get(y) ?? null));
  const series = createMemo(() => {
    const out = [{ name: props.portfolioName, values: portfolio(), slot: 0 }];
    if (props.bench) out.push({ name: props.bench.name, values: bench(), slot: props.bench.slot });
    return out;
  });
  return (
    <Card title={r().charts.yearly} sub={props.bench ? tpl(r().charts.yearly_sub, { bench: props.bench.name }) : undefined}>
      <ChartBox
        height={240}
        ariaLabel={r().charts.yearly}
        option={() => yearlyBarsOption(years(), series(), readPalette(), { partial: partialSet(), excessLabel: r().charts.excess })}
      />
      <div class="table-wrap rs-yearly-table">
        <table class="table compact">
          <thead>
            <tr>
              <th />
              <For each={years()}>
                {(y) => (
                  <th class="r">
                    {y}
                    {partialSet().has(y) ? "*" : ""}
                  </th>
                )}
              </For>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td class="rs-metric-label">
                <span class="rs-series-key">
                  <i style={{ background: "var(--hq-series-1)" }} />
                  {props.portfolioName}
                </span>
              </td>
              <For each={portfolio()}>{(v) => <td class="r">{cell(v)}</td>}</For>
            </tr>
            <Show when={props.bench}>
              <tr>
                <td class="rs-metric-label">
                  <span class="rs-series-key">
                    <i style={{ background: `var(--hq-series-${(props.bench!.slot % 5) + 1})` }} />
                    {props.bench!.name}
                  </span>
                </td>
                <For each={bench()}>{(v) => <td class="r">{cell(v)}</td>}</For>
              </tr>
              <tr class="rs-excess-row">
                <td class="rs-metric-label">{r().charts.excess}</td>
                <For each={portfolio()}>
                  {(v, i) => {
                    const b = bench()[i()];
                    return <td class="r">{v != null && b != null ? cell(v - b) : <span class="muted">—</span>}</td>;
                  }}
                </For>
              </tr>
            </Show>
          </tbody>
        </table>
      </div>
      <Show when={partialNote()}>
        <p class="muted xs rs-card-note">{partialNote()}</p>
      </Show>
    </Card>
  );
}

function cell(v: number | null) {
  if (v == null) return <span class="muted">—</span>;
  return <span class={`num ${v > 0 ? "up" : v < 0 ? "down" : "flat"}`}>{fmtPct(v, { dp: 1, sign: true })}</span>;
}

export function SectorContributionCard(props: { rows: Attribution[]; sectorName: (id: string | null | undefined) => string }) {
  const r = researchText;
  const rows = createMemo(() =>
    props.rows.map((a) => ({
      label: props.sectorName(a.key),
      value: a.contribution,
      detail: tpl(r().charts.avg_weight, { v: fmtPct(a.avg_weight, { dp: 1 }) }),
    })),
  );
  return (
    <Card title={r().charts.sector_contrib} sub={r().charts.sector_contrib_sub}>
      <Show when={rows().length} fallback={<NotYet icon="layers" title={r().perf.need_snapshots} height={200} />}>
        <ChartBox height={Math.max(160, rows().length * 34 + 34)} ariaLabel={r().charts.sector_contrib} option={(width) => contributionBarsOption(rows(), readPalette(), { width })} />
      </Show>
    </Card>
  );
}

export function AssetContributionCard(props: { rows: Attribution[]; assetName: (symbol: string) => string; sectorName: (id: string | null | undefined) => string }) {
  const r = researchText;
  const N = 10;
  const rows = createMemo(() => {
    const sorted = [...props.rows].sort((a, b) => b.contribution - a.contribution);
    const picked = sorted.length > 2 * N ? [...sorted.slice(0, N), ...sorted.slice(-N)] : sorted;
    return picked.map((a) => {
      const name = props.assetName(a.key);
      return {
        label: name ? `${a.key} · ${name}` : a.key,
        short: a.key,
        value: a.contribution,
        detail: `${props.sectorName(a.sector)} · ${tpl(r().charts.avg_weight, { v: fmtPct(a.avg_weight, { dp: 2 }) })}`,
      };
    });
  });
  return (
    <Card title={r().charts.asset_contrib} sub={tpl(r().charts.asset_contrib_sub, { n: N })}>
      <Show when={rows().length} fallback={<NotYet icon="layers" title={r().perf.need_snapshots} height={200} />}>
        <ChartBox height={Math.max(160, rows().length * 24 + 34)} ariaLabel={r().charts.asset_contrib} option={(width) => contributionBarsOption(rows(), readPalette(), { width })} />
      </Show>
    </Card>
  );
}

type WeightSample = { date: string; weights: Record<string, number>; cash: number };

/** Keeps at most ~130 columns: weekly samples, else month-ends, else quarter-ends. */
function resample(samples: WeightSample[], max = 130): { samples: WeightSample[]; unit: "week" | "month" | "quarter" } {
  if (samples.length <= max) return { samples, unit: "week" };
  const lastPer = (key: (d: string) => string) => samples.filter((s, i) => i === samples.length - 1 || key(samples[i + 1].date) !== key(s.date));
  const monthly = lastPer((d) => d.slice(0, 7));
  if (monthly.length <= max) return { samples: monthly, unit: "month" };
  return { samples: lastPer((d) => `${d.slice(0, 4)}-${Math.floor((Number(d.slice(5, 7)) - 1) / 3)}`), unit: "quarter" };
}

export function SectorWeightsCard(props: { samples: WeightSample[]; sectors: { id: string; name: string }[] }) {
  const r = researchText;
  const sampled = createMemo(() => resample(props.samples));
  const sampling = () => {
    const c = r().charts;
    return { week: c.sample_week, month: c.sample_month, quarter: c.sample_quarter }[sampled().unit];
  };
  const height = () => (props.sectors.length + 1) * 24 + 64;
  return (
    <Card title={r().charts.sectors} sub={tpl(r().charts.sectors_sub, { sampling: sampling() })}>
      <ChartBox
        height={height()}
        ariaLabel={r().charts.sectors}
        option={(width) => sectorWeightsOption(sampled().samples, props.sectors, readPalette(), { width, cashLabel: r().charts.cash, weightLabel: r().charts.weight })}
      />
    </Card>
  );
}

export function RollingVolCard(props: { dates: string[]; values: (number | null)[]; periodVol: number | null; have: number }) {
  const r = researchText;
  const available = () => props.values.some((v) => v != null);
  return (
    <Card title={r().charts.rolling_vol} sub={r().charts.rolling_vol_sub}>
      <Show when={available()} fallback={<NotYet title={tpl(r().perf.need_days, { need: 21, have: props.have })} height={220} />}>
        <ChartBox
          height={220}
          ariaLabel={r().charts.rolling_vol}
          option={() =>
            rollingVolOption(props.dates, props.values, readPalette(), {
              name: r().charts.rolling_vol,
              reference: props.periodVol,
              referenceLabel: r().charts.period_vol,
            })
          }
        />
      </Show>
    </Card>
  );
}
