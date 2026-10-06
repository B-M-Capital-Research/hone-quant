import { A, useNavigate, useParams } from "@solidjs/router";
import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { Icon } from "@/components/Icon";
import { Empty, ErrorState, Kpi, Loading, confirmAction, toast, toastError } from "@/components/ui";
import { tpl } from "@/i18n";
import { researchText } from "@/i18n/research";
import { common } from "@/i18n/common";
import { ApiError, api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { fmtDual, fmtMoney, fmtNum, fmtPct } from "@/lib/format";
import { isAdmin, serverNow } from "@/lib/session";
import type { BacktestResult, BacktestRow, DataStatus, StrategyOverview, UniverseView } from "@/lib/types";
import {
  BacktestStatusChip,
  CopyIcon,
  IndeterminateBar,
  KpiStrip,
  MethodologyCard,
  MetricsTable,
  RelativeKpis,
  ReturnRiskKpis,
  WarningCallouts,
  type MetricsColumn,
} from "./research/components";
import {
  assetNamer,
  benchLong,
  benchShort,
  benchSlot,
  costsSummary,
  createLoader,
  dataStatusRef,
  dataWindow,
  debounce,
  failureText,
  estimateMs,
  fmtDuration,
  fmtYears,
  pickText,
  resolveStrategy,
  sectorNamer,
  slotsLabel,
  strategyLabel,
  strategyRef,
  typicalMsPerYear,
  universeRef,
  yearsBetween,
} from "./research/model";
import {
  AssetContributionCard,
  EquityCard,
  type EquityLine,
  MonthlyCard,
  SectorContributionCard,
  SectorWeightsCard,
  YearlyCard,
} from "./research/report";
import { PositionsTable, TradeLog } from "./research/tables";

export default function BacktestDetail() {
  const r = researchText;
  const c = common;
  const params = useParams();
  const navigate = useNavigate();
  const id = () => Number(params.id);
  const detail = createLoader(() => api.backtest(id()));
  const queue = createLoader(() => api.backtests().then((d) => d.backtests));
  const [universe, setUniverse] = createSignal<UniverseView>();
  const [overview, setOverview] = createSignal<StrategyOverview>();
  const [deleting, setDeleting] = createSignal(false);
  const [status, setStatus] = createSignal<DataStatus>();
  const coverage = createMemo(() => dataWindow(status(), universe()));

  createEffect(on(id, () => void detail.load()));
  // Data coverage explains the most common failure (dates outside the stored history).
  createEffect(() => {
    if (row()?.status === "failed" && !status()) dataStatusRef.get().then(setStatus).catch(() => undefined);
  });

  onMount(() => {
    universeRef.get().then(setUniverse).catch(() => undefined);
    strategyRef.get().then(setOverview).catch(() => undefined);
    const refresh = debounce(() => void detail.load(), 150);
    const off = onServerEvent(["backtest", "universe", "strategy"], (event) => {
      if (event.type === "backtest" && event.id !== id()) {
        // Another run started or finished: the queue position may have changed.
        if (row() && row()!.status === "queued") void queue.load();
        return;
      }
      if (event.type === "universe") {
        universeRef.invalidate();
        universeRef.get().then(setUniverse).catch(() => undefined);
        return;
      }
      if (event.type === "strategy") {
        strategyRef.invalidate();
        strategyRef.get().then(setOverview).catch(() => undefined);
        return;
      }
      refresh();
    });
    onCleanup(off);
  });

  const row = () => detail.data()?.backtest;
  const result = () => detail.data()?.result ?? null;
  const notFound = () => detail.error() instanceof ApiError && (detail.error() as ApiError).status === 404;
  const isActive = () => row()?.status === "queued" || row()?.status === "running";

  // While queued/running: poll as a fallback to the event stream, and load the queue for
  // position and typical duration.
  createEffect(() => {
    if (!isActive()) return;
    void queue.load();
    const timer = setInterval(() => {
      void detail.load();
      if (row()?.status === "queued") void queue.load();
    }, 4000);
    onCleanup(() => clearInterval(timer));
  });

  const ahead = () => {
    const current = row();
    const rows = queue.data();
    if (!current || !rows) return null;
    return rows.filter((b) => b.status === "running" || (b.status === "queued" && b.id < current.id)).length;
  };
  const typical = () => {
    const current = row();
    const rows = queue.data();
    if (!current || !rows) return null;
    return estimateMs(current, typicalMsPerYear(rows));
  };

  async function remove() {
    const current = row();
    if (!current) return;
    const ok = await confirmAction({
      title: r().list.delete_title,
      body: tpl(r().list.delete_body, { name: current.name }),
      confirmLabel: c().actions.delete,
      danger: true,
    });
    if (ok === null) return;
    setDeleting(true);
    try {
      await api.deleteBacktest(current.id);
      toast(r().list.deleted, current.name, "success");
      navigate("/backtests", { replace: true });
    } catch (error) {
      toastError(error);
    } finally {
      setDeleting(false);
    }
  }

  const duplicate = () => navigate(`/backtests?new=1&from=${id()}`);

  return (
    <div class="stack rs-page">
      <A href="/backtests" class="rs-back">
        <Icon name="chevron_left" size={16} />
        {r().detail.back}
      </A>
      <Switch>
        <Match when={notFound()}>
          <div class="card">
            <Empty title={r().detail.not_found} icon="search">
              <A class="btn sm" href="/backtests">
                {r().detail.back}
              </A>
            </Empty>
          </div>
        </Match>
        <Match when={detail.error() && !detail.data()}>
          <div class="card">
            <ErrorState error={detail.error()} onRetry={() => void detail.load()} />
          </div>
        </Match>
        <Match when={!row()}>
          <div class="card">
            <Loading />
          </div>
        </Match>
        <Match when={row()}>
          {(bt) => (
            <>
              <Header row={bt()} overview={overview()} universe={universe()} deleting={deleting()} onDuplicate={duplicate} onDelete={remove} />

              <Switch>
                <Match when={bt().status === "queued"}>
                  <div class="card rs-run-state">
                    <span class="rs-run-icon">
                      <Icon name="clock" size={20} />
                    </span>
                    <div class="rs-run-text">
                      <h2>{r().detail.queued_title}</h2>
                      <p>
                        {r().detail.queued_body}
                        <Show when={ahead() != null}> {ahead()! > 0 ? tpl(r().list.queued_ahead, { n: ahead()! }) : r().list.queued_next}</Show>
                      </p>
                      <p class="muted xs">{r().detail.running_body}</p>
                    </div>
                  </div>
                </Match>
                <Match when={bt().status === "running"}>
                  <div class="card rs-run-state">
                    <span class="spinner rs-run-spinner" />
                    <div class="rs-run-text">
                      <h2>{r().detail.running_title}</h2>
                      <p>
                        {tpl(r().list.running_for, { t: fmtDuration(bt().started_at ? serverNow() - Date.parse(bt().started_at!) : 0) })}
                        <Show when={typical()}> · {tpl(r().list.typical, { t: fmtDuration(typical()) })}</Show>
                      </p>
                      <IndeterminateBar />
                      <p class="muted xs">{r().detail.running_body}</p>
                    </div>
                  </div>
                </Match>
                <Match when={bt().status === "failed"}>
                  <div class="callout critical" role="alert">
                    <Icon name="alert" size={18} />
                    <div class="rs-callout-body">
                      <strong>{r().detail.failed_title}</strong>
                      <Show when={failureText(bt().error).known}>
                        <span>
                          {failureText(bt().error).text}
                          <Show when={failureText(bt().error).known === "no_data" && coverage()}>
                            {" "}
                            {tpl(r().detail.failed_no_data_window, { from: coverage()!.first, to: coverage()!.last })}
                          </Show>
                        </span>
                      </Show>
                      <code class="rs-error-code" title={r().detail.server_message}>
                        {bt().error ?? "—"}
                      </code>
                      <Show when={isAdmin()}>
                        <div>
                          <button class="btn sm" type="button" onClick={duplicate}>
                            <CopyIcon size={14} />
                            {r().detail.retry}
                          </button>
                        </div>
                      </Show>
                    </div>
                  </div>
                </Match>
                <Match when={bt().status === "succeeded" && result()}>
                  {(res) => <Report row={bt()} result={res()} universe={universe()} />}
                </Match>
                <Match when={bt().status === "succeeded"}>
                  <div class="card">
                    <Loading />
                  </div>
                </Match>
              </Switch>

              <MethodologyCard />
            </>
          )}
        </Match>
      </Switch>
    </div>
  );
}

function Header(props: {
  row: BacktestRow;
  overview: StrategyOverview | undefined;
  universe: UniverseView | undefined;
  deleting: boolean;
  onDuplicate: () => void;
  onDelete: () => void;
}) {
  const r = researchText;
  const c = common;
  const cfg = () => props.row.config;
  const strategy = () => strategyLabel(resolveStrategy(props.row, props.overview), props.overview);
  const elapsed = () => props.row.summary?.elapsed_ms ?? (props.row.started_at && props.row.finished_at ? Date.parse(props.row.finished_at) - Date.parse(props.row.started_at) : null);
  const pills = () => [
    { label: r().detail.config_period, value: `${cfg().start} → ${cfg().end} · ${tpl(r().range.years, { n: fmtYears(yearsBetween(cfg().start, cfg().end)) })}` },
    { label: r().detail.config_strategy, value: strategy() },
    { label: r().detail.config_slots, value: slotsLabel(cfg().slots) },
    { label: r().detail.config_frequency, value: r().frequency[cfg().frequency] },
    { label: r().detail.config_cash, value: fmtMoney(cfg().initial_cash, { dp: 0 }) },
    { label: r().detail.config_benchmark, value: cfg().benchmark ? benchLong(cfg().benchmark!, props.universe) : "—" },
    { label: r().detail.config_rf, value: fmtPct(cfg().risk_free_rate ?? 0, { dp: 2 }) },
    { label: r().detail.config_costs, value: costsSummary(cfg().costs), wide: true },
  ];
  return (
    <header class="rs-detail-head">
      <div class="page-head" style={{ "margin-bottom": "0" }}>
        <div class="rs-title-block">
          <div class="rs-title-row">
            <h1>{props.row.name}</h1>
            <BacktestStatusChip status={props.row.status} />
          </div>
          <p class="lead">
            #{props.row.id} · {tpl(r().detail.created, { by: props.row.created_by, at: fmtDual(props.row.created_at, true) })}
            <Show when={props.row.status === "succeeded" && elapsed() != null}> · {tpl(r().detail.elapsed, { t: fmtDuration(elapsed()) })}</Show>
          </p>
        </div>
        <Show when={isAdmin()}>
          <div class="row wrap">
            <button class="btn" type="button" onClick={props.onDuplicate}>
              <CopyIcon size={15} />
              {r().list.duplicate}
            </button>
            <button class="btn danger" type="button" onClick={props.onDelete} disabled={props.deleting || props.row.status === "running"}>
              <Icon name="trash" size={15} />
              {c().actions.delete}
            </button>
          </div>
        </Show>
      </div>
      <div class="pill-list rs-config-pills">
        <For each={pills()}>
          {(p) => (
            <span class={`param ${p.wide ? "rs-param-wide" : ""}`}>
              <span class="muted">{p.label}</span>
              <b>{p.value}</b>
            </span>
          )}
        </For>
      </div>
    </header>
  );
}

function Report(props: { row: BacktestRow; result: BacktestResult; universe: UniverseView | undefined }) {
  const r = researchText;
  const res = () => props.result;
  const cfg = () => props.row.config;
  const sectorName = createMemo(() => sectorNamer(props.universe));
  const assetName = createMemo(() => assetNamer(props.universe));
  const dates = createMemo(() => res().points.map((p) => p.date));
  const nav = createMemo(() => res().points.map((p) => p.nav));
  const order = createMemo(() => res().benchmarks.map((b) => b.symbol));
  const primary = createMemo(() => {
    const wanted = cfg().benchmark;
    return res().benchmarks.find((b) => b.symbol === wanted) ?? res().benchmarks[0] ?? null;
  });
  const lines = createMemo<EquityLine[]>(() => [
    { name: r().charts.portfolio, label: r().charts.portfolio, values: nav(), slot: 0, emphasis: true },
    ...res().benchmarks.map((b) => ({
      name: benchShort(b.symbol),
      label: benchShort(b.symbol),
      values: b.values,
      slot: benchSlot(b.symbol, order()),
      hidden: order().filter((s) => s !== "UNIVERSE_EW").indexOf(b.symbol) >= 3,
    })),
  ]);
  const last = () => res().points[res().points.length - 1];
  const m = () => res().metrics;
  const years = () => yearsBetween(cfg().start, cfg().end);
  const columns = createMemo<MetricsColumn[]>(() => [
    { key: "portfolio", name: r().charts.portfolio, slot: 0, metrics: m() },
    ...res().benchmarks.map((b) => ({
      key: b.symbol,
      name: benchShort(b.symbol),
      detail: benchLong(b.symbol, props.universe),
      slot: benchSlot(b.symbol, order()),
      primary: b.symbol === primary()?.symbol,
      metrics: b.metrics,
    })),
  ]);
  const sectors = createMemo(() => {
    const known = [...(props.universe?.sectors ?? [])].sort((a, b) => a.sort_order - b.sort_order);
    const ids = new Set(res().sector_weights.flatMap((s) => Object.keys(s.weights)));
    const rows = known.filter((s) => ids.has(s.id)).map((s) => ({ id: s.id, name: pickText(s, "name") }));
    for (const id of ids) if (!rows.some((s) => s.id === id)) rows.push({ id, name: sectorName()(id) });
    return rows;
  });

  return (
    <>
      <Show when={res().warnings.length}>
        <WarningCallouts warnings={res().warnings} names={assetName()} />
      </Show>
      <Show when={!m().annualisation_reliable}>
        <div class="callout warn">
          <Icon name="info" size={16} />
          <span>{r().warnings.unreliable}</span>
        </div>
      </Show>

      <KpiStrip title={r().detail.kpi_returns}>
        <ReturnRiskKpis
          m={m()}
          lead={
            <Kpi
              label={r().detail.final_nav}
              value={fmtMoney(last()?.nav, { dp: 0 })}
              delta={<span class="muted">{tpl(r().detail.initial_cash, { v: fmtMoney(cfg().initial_cash, { dp: 0 }) })}</span>}
            />
          }
          totalDelta={<span class="muted">{tpl(r().detail.trading_days, { n: fmtNum(m().trading_days, 0) })}</span>}
          cagrDelta={<span class="muted">{tpl(r().detail.years, { n: fmtYears(years()) })}</span>}
          sharpeDelta={<span class="muted">{tpl(r().detail.rf_note, { v: fmtPct(cfg().risk_free_rate ?? 0, { dp: 2 }) })}</span>}
        />
      </KpiStrip>
      <KpiStrip title={tpl(r().detail.kpi_relative, { bench: benchShort(primary()?.symbol ?? "") })}>
        <RelativeKpis m={m()} bench={benchShort(primary()?.symbol ?? "")} />
        <Kpi
          label={r().metrics.total_costs}
          title={r().metrics.help.total_costs}
          value={fmtMoney(res().total_costs, { compact: true })}
          delta={<span class="muted">{tpl(r().detail.cost_share, { v: fmtPct(res().total_costs / cfg().initial_cash, { dp: 2 }) })}</span>}
        />
        <Show
          when={m().annualisation_reliable}
          fallback={
            <Kpi
              label={r().detail.period_turnover}
              title={r().metrics.help.annual_turnover}
              value={fmtPct(res().total_turnover, { dp: 0 })}
              delta={<span class="muted">{r().detail.period_turnover_short}</span>}
            />
          }
        >
          <Kpi
            label={r().metrics.annual_turnover}
            title={r().metrics.help.annual_turnover}
            value={fmtPct(res().annual_turnover, { dp: 0 })}
            delta={<span class="muted">{r().detail.one_way}</span>}
          />
        </Show>
        <Kpi
          label={r().detail.rebalances_trades}
          title={r().detail.rebalances_hint}
          value={`${fmtNum(res().rebalances, 0)} / ${fmtNum(res().total_trades ?? res().trades.length, 0)}`}
          delta={<span class="muted">{r().detail.rebalances_hint}</span>}
        />
      </KpiStrip>

      <EquityCard dates={dates()} lines={lines()} drawdowns={res().drawdowns} />

      <MonthlyCard monthly={res().monthly_returns} yearly={res().yearly_returns} />

      <Show
        when={res().yearly_returns.length > 1}
        fallback={<SectorContributionCard rows={res().sector_contributions} sectorName={sectorName()} />}
      >
        <div class="grid two rs-grid">
          <YearlyCard
            dates={dates()}
            yearly={res().yearly_returns}
            portfolioName={r().charts.portfolio}
            bench={primary() ? { name: benchShort(primary()!.symbol), values: primary()!.values, slot: benchSlot(primary()!.symbol, order()) } : null}
          />
          <SectorContributionCard rows={res().sector_contributions} sectorName={sectorName()} />
        </div>
      </Show>

      <Show when={res().sector_weights.length > 1}>
        <SectorWeightsCard samples={res().sector_weights} sectors={sectors()} />
      </Show>

      <div class="grid two rs-grid">
        <AssetContributionCard rows={res().asset_contributions} assetName={assetName()} sectorName={sectorName()} />
        <PositionsTable
          positions={res().final_positions}
          cash={last()?.cash ?? 0}
          nav={last()?.nav ?? 0}
          date={last()?.date ?? cfg().end}
          assetName={assetName()}
          sectorName={sectorName()}
        />
      </div>

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

      <TradeLog id={props.row.id} trades={res().trades} total={res().total_trades ?? res().trades.length} assetName={assetName()} />
    </>
  );
}
