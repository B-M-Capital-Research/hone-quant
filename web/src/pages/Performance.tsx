import { A, useSearchParams } from "@solidjs/router";
import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { Icon } from "@/components/Icon";
import { ErrorState, Kpi, Loading, Money, Pct, Segmented } from "@/components/ui";
import { tpl } from "@/i18n";
import { researchText } from "@/i18n/research";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { fmtMoney, fmtNum, fmtPct } from "@/lib/format";
import type { Performance as PerformanceData, UniverseView } from "@/lib/types";
import { Card, KpiStrip, MetricsTable, RelativeKpis, ReturnRiskKpis, type MetricsColumn } from "./research/components";
import { RELIABLE_DAYS, assetNamer, benchLong, benchShort, benchSlot, createLoader, debounce, mean, sectorNamer, universeRef } from "./research/model";
import {
  AssetContributionCard,
  EquityCard,
  type EquityLine,
  MonthlyCard,
  RollingVolCard,
  SectorContributionCard,
  YearlyCard,
  dropBaseOnly,
} from "./research/report";

type Range = "1M" | "3M" | "6M" | "YTD" | "1Y" | "ALL";
const RANGES: Range[] = ["1M", "3M", "6M", "YTD", "1Y", "ALL"];
const RANGE_KEY = "hone-quant.performance.range";

function storedRange(): Range | null {
  try {
    const v = localStorage.getItem(RANGE_KEY);
    return RANGES.includes(v as Range) ? (v as Range) : null;
  } catch {
    return null;
  }
}

export default function Performance() {
  const r = researchText;
  const [params, setParams] = useSearchParams();
  const range = (): Range => {
    const fromUrl = typeof params.range === "string" ? (params.range.toUpperCase() as Range) : null;
    return fromUrl && RANGES.includes(fromUrl) ? fromUrl : (storedRange() ?? "ALL");
  };
  const perf = createLoader(() => api.performance(range()));
  const [universe, setUniverse] = createSignal<UniverseView>();

  createEffect(on(range, () => void perf.load()));

  onMount(() => {
    universeRef.get().then(setUniverse).catch(() => undefined);
    // Fills, snapshots and corporate actions all arrive as account events.
    const refresh = debounce(() => void perf.load(), 800);
    const off = onServerEvent(["account", "settings"], refresh);
    onCleanup(off);
  });

  const choose = (value: Range) => {
    try {
      localStorage.setItem(RANGE_KEY, value);
    } catch {
      /* storage unavailable */
    }
    setParams({ range: value === "ALL" ? undefined : value }, { replace: true });
  };

  const rangeLabel = (v: Range) => {
    const x = r().perf.ranges;
    return { "1M": x.m1, "3M": x.m3, "6M": x.m6, YTD: x.ytd, "1Y": x.y1, ALL: x.all }[v];
  };

  return (
    <div class="stack rs-page">
      <div class="page-head">
        <div style={{ flex: 1, "min-width": "260px" }}>
          <h1>{r().perf.title}</h1>
          <p class="lead">{r().perf.lead}</p>
        </div>
        <div class="rs-range-block">
          <Segmented value={range()} onChange={choose} label={r().perf.title} options={RANGES.map((v) => ({ value: v, label: rangeLabel(v) }))} />
          <Show when={perf.data()}>
            {(d) => (
              <span class="muted xs">
                {tpl(r().perf.since, { date: d().inception_date })}
                {" · "}
                {d().dates.length > 1
                  ? tpl(d().metrics.trading_days === 1 ? r().perf.window_one : r().perf.window, { from: d().dates[0], to: d().dates[d().dates.length - 1], n: d().metrics.trading_days })
                  : r().perf.window_single}
              </span>
            )}
          </Show>
        </div>
      </div>

      <Switch>
        <Match when={perf.error() && !perf.data()}>
          <div class="card">
            <ErrorState error={perf.error()} onRetry={() => void perf.load()} />
          </div>
        </Match>
        <Match when={!perf.data()}>
          <div class="card">
            <Loading />
          </div>
        </Match>
        <Match when={perf.data()}>{(d) => <PerformanceReport data={d()} universe={universe()} busy={perf.loading()} range={range()} />}</Match>
      </Switch>
    </div>
  );
}

function PerformanceReport(props: { data: PerformanceData; universe: UniverseView | undefined; busy: boolean; range: Range }) {
  const r = researchText;
  const d = () => props.data;
  const m = () => d().metrics;
  const days = () => m().trading_days;
  const young = () => d().dates.length < 2;
  const sectorName = createMemo(() => sectorNamer(props.universe));
  const assetName = createMemo(() => assetNamer(props.universe));
  const order = createMemo(() => d().benchmarks.map((b) => b.symbol));
  const primary = createMemo(() => d().benchmarks.find((b) => b.symbol === d().primary_benchmark) ?? null);
  const lastNav = () => d().nav[d().nav.length - 1] ?? null;
  const t = () => d().trades;
  const avgNav = () => mean(d().nav.filter((v) => v > 0));
  const costs = () => t().commissions + t().fees + t().slippage;
  const turnoverReliable = () => days() >= RELIABLE_DAYS && t().annual_turnover != null;
  const lines = createMemo<EquityLine[]>(() => [
    { name: r().charts.portfolio, label: r().charts.portfolio, values: d().nav, slot: 0, emphasis: true },
    ...d().benchmarks.map((b) => ({
      name: benchShort(b.symbol),
      label: benchShort(b.symbol),
      values: b.values,
      slot: benchSlot(b.symbol, order()),
      hidden: order().indexOf(b.symbol) >= 3,
    })),
  ]);
  const monthly = createMemo(() => dropBaseOnly(d().monthly_returns, d().dates, true));
  const yearly = createMemo(() => dropBaseOnly(d().yearly_returns, d().dates, false));
  const columns = createMemo<MetricsColumn[]>(() => [
    { key: "portfolio", name: r().charts.portfolio, slot: 0, metrics: m() },
    ...d().benchmarks.map((b) => ({
      key: b.symbol,
      name: benchShort(b.symbol),
      detail: benchLong(b.symbol, props.universe),
      slot: benchSlot(b.symbol, order()),
      primary: b.symbol === d().primary_benchmark,
      metrics: b.metrics,
    })),
  ]);
  const navTile = () => (
    <Kpi label={r().metrics.nav} value={fmtMoney(lastNav(), { dp: 0 })} delta={<span class="muted">{tpl(r().perf.since, { date: d().inception_date })}</span>} />
  );
  const fillsTile = () => (
    <Kpi
      label={r().perf.fills}
      value={fmtNum(t().fills, 0)}
      delta={<span class="muted">{tpl(r().detail.fills_split, { buys: fmtNum(t().buys, 0), sells: fmtNum(t().sells, 0) })}</span>}
    />
  );
  const metricsCard = () => (
    <section class="card">
      <div class="card-head">
        <div class="rs-head-text">
          <h2>{r().detail.metrics_title}</h2>
          <div class="sub">{r().detail.metrics_sub}</div>
        </div>
      </div>
      <div class="card-body flush">
        <MetricsTable columns={columns()} reliable={m().annualisation_reliable} />
      </div>
    </section>
  );

  return (
    <div class={`stack ${props.busy ? "refetching" : ""}`}>
      <Show when={days() < RELIABLE_DAYS}>
        <div class={`callout ${young() ? "info" : "warn"}`}>
          <Icon name={young() ? "info" : "clock"} size={16} />
          <div class="rs-callout-body">
            <strong>{young() ? r().perf.young_title : r().perf.unreliable_title}</strong>
            <span>
              {props.range === "ALL" || d().dates[0] === d().inception_date
                ? tpl(r().perf.young_body, { date: d().inception_date, n: days() })
                : tpl(r().perf.young_range, { n: days() })}
            </span>
            <Show when={!young()}>
              <span>{r().warnings.unreliable}</span>
            </Show>
            <div>
              <A class="btn sm" href="/backtests">
                <Icon name="backtest" size={14} />
                {r().perf.run_backtest}
              </A>
            </div>
          </div>
        </div>
      </Show>

      <Show
        when={!young()}
        fallback={
          <>
            <KpiStrip title={r().detail.kpi_returns}>
              {navTile()}
              <Kpi label={r().metrics.total_return} title={r().metrics.help.total_return} value={<Pct value={m().total_return} />} />
              <Kpi
                label={r().perf.days_recorded}
                value={fmtNum(days(), 0)}
                delta={<span class="muted">{tpl(r().perf.days_recorded_hint, { points: d().dates.length })}</span>}
              />
              {fillsTile()}
            </KpiStrip>
            <div class="grid two rs-grid">
              <Readiness days={days()} />
              <TradingCard data={d()} />
            </div>
          </>
        }
      >
        <KpiStrip title={r().detail.kpi_returns}>
          <ReturnRiskKpis
            m={m()}
            lead={navTile()}
            totalDelta={<Money value={(lastNav() ?? 0) - (d().nav[0] ?? 0)} signed compact />}
            cagrDelta={<span class="muted">{days() === 1 ? r().detail.trading_days_one : tpl(r().detail.trading_days, { n: fmtNum(days(), 0) })}</span>}
          />
        </KpiStrip>
        <KpiStrip title={tpl(r().detail.kpi_relative, { bench: benchShort(d().primary_benchmark) })}>
          <RelativeKpis m={m()} bench={benchShort(d().primary_benchmark)} />
          <Kpi
            label={r().metrics.total_costs}
            title={r().metrics.help.total_costs}
            value={fmtMoney(costs(), { compact: true })}
            delta={
              <Show when={avgNav()}>
                <span class="muted">{tpl(r().detail.cost_share_nav, { v: fmtPct(costs() / avgNav()!, { dp: 3 }) })}</span>
              </Show>
            }
          />
          <Kpi
            label={r().metrics.annual_turnover}
            title={r().metrics.help.annual_turnover}
            value={turnoverReliable() ? fmtPct(t().annual_turnover, { dp: 0 }) : <span class="muted">—</span>}
            delta={<span class="muted">{turnoverReliable() ? r().detail.one_way : r().perf.turnover_unavailable}</span>}
          />
          {fillsTile()}
        </KpiStrip>

        <EquityCard dates={d().dates} lines={lines()} drawdowns={d().drawdowns} />

        <div class="grid two rs-grid">
          <RollingVolCard dates={d().dates} values={d().rolling_vol} periodVol={m().ann_vol} have={days()} />
          <TradingCard data={d()} />
        </div>

        <Show when={monthly().length}>
          <MonthlyCard monthly={monthly()} yearly={yearly()} />
        </Show>

        <Show when={yearly().length > 1}>
          <YearlyCard
            dates={d().dates}
            yearly={yearly()}
            portfolioName={r().charts.portfolio}
            bench={primary() ? { name: benchShort(primary()!.symbol), values: primary()!.values, slot: benchSlot(primary()!.symbol, order()) } : null}
          />
        </Show>

        <div class="grid two rs-grid">
          <SectorContributionCard rows={d().sector_attribution} sectorName={sectorName()} />
          <AssetContributionCard rows={d().asset_attribution} assetName={assetName()} sectorName={sectorName()} />
        </div>

        {metricsCard()}
      </Show>
    </div>
  );
}

/** What unlocks as history accumulates (shown while the account is new). */
function Readiness(props: { days: number }) {
  const r = researchText;
  const milestones = () => [
    { label: r().perf.milestone_curve, need: 1 },
    { label: r().perf.milestone_rolling, need: 21 },
    { label: r().perf.milestone_annual, need: RELIABLE_DAYS },
    { label: r().perf.milestone_year, need: 252 },
  ];
  return (
    <Card title={r().perf.readiness} sub={r().perf.readiness_sub}>
      <ul class="rs-milestones">
        <For each={milestones()}>
          {(ms) => {
            const done = () => props.days >= ms.need;
            return (
              <li classList={{ done: done() }}>
                <span class="rs-milestone-dot">{done() ? <Icon name="check" size={12} /> : null}</span>
                <div class="rs-milestone-text">
                  <span>{ms.label}</span>
                  <div class="rs-milestone-bar">
                    <div class="progress">
                      <div style={{ width: `${Math.min(100, (props.days / ms.need) * 100)}%` }} />
                    </div>
                    <span class="muted xs num">{done() ? r().perf.milestone_ready : tpl(r().perf.milestone_days, { have: props.days, need: ms.need })}</span>
                  </div>
                </div>
              </li>
            );
          }}
        </For>
      </ul>
    </Card>
  );
}

function TradingCard(props: { data: PerformanceData }) {
  const r = researchText;
  const t = () => props.data.trades;
  const reliable = () => props.data.metrics.trading_days >= RELIABLE_DAYS;
  const avgNav = () => mean(props.data.nav.filter((v) => v > 0));
  const periodTurnover = () => {
    const a = avgNav();
    return a ? (t().bought + t().sold) / 2 / a : null;
  };
  const totalCosts = () => t().commissions + t().fees + t().slippage;
  const items = () => [
    { label: r().perf.fills, value: fmtNum(t().fills, 0) },
    { label: r().perf.buys_sells, value: `${fmtNum(t().buys, 0)} / ${fmtNum(t().sells, 0)}` },
    { label: r().perf.bought, value: fmtMoney(t().bought) },
    { label: r().perf.sold, value: fmtMoney(t().sold) },
    { label: r().perf.commissions, value: fmtMoney(t().commissions) },
    { label: r().perf.fees, value: fmtMoney(t().fees) },
    { label: r().perf.slippage, value: fmtMoney(t().slippage) },
    { label: r().perf.total_costs, value: fmtMoney(totalCosts()) },
    { label: r().perf.realized, value: <Money value={t().realized_pnl} signed /> },
    { label: r().perf.sell_record, value: `${fmtNum(t().winning_sells, 0)} / ${fmtNum(t().losing_sells, 0)}` },
    {
      label: r().perf.turnover,
      value: reliable() && t().annual_turnover != null ? fmtPct(t().annual_turnover, { dp: 0 }) : <span class="muted" title={r().perf.turnover_unavailable}>—</span>,
      hint: reliable() ? r().detail.one_way : r().perf.turnover_unavailable,
    },
    { label: r().perf.period_turnover, value: fmtPct(periodTurnover(), { dp: 1 }), hint: r().perf.period_turnover_hint },
  ];
  return (
    <Card title={r().perf.trading} sub={r().perf.trading_sub}>
      <Show when={t().fills > 0} fallback={<p class="muted small">{r().perf.no_trades}</p>}>
        <dl class="rs-stats">
          <For each={items()}>
            {(item) => (
              <div class="rs-stat">
                <dt>{item.label}</dt>
                <dd class="num">{item.value}</dd>
                <Show when={item.hint}>
                  <span class="rs-stat-hint">{item.hint}</span>
                </Show>
              </div>
            )}
          </For>
        </dl>
      </Show>
    </Card>
  );
}
