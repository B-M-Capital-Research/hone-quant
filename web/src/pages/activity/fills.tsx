/** Fills tab of /trades: each execution with prices, costs and realised P&L. */
import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on } from "solid-js";
import { Empty, ErrorState, Kpi, Loading, Money, SideChip } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { tradesText } from "@/i18n/trades";
import { api } from "@/lib/api";
import { DASH, MARKET_TZ, fmtDateTime, fmtDual, fmtMoney, fmtNum, fmtPrice, fmtQty, toNumber } from "@/lib/format";
import type { Fill } from "@/lib/types";
import { DualTime, Pager, PlanLink, SymbolCell } from "./components";
import { nameOf } from "./names";
import { useQuery } from "./query";
import { ExportButton, Summary, type TabProps, Toolbar, csvName, exportRows, truncationNote } from "./trades-common";
import { fetchAll, num } from "./util";

const CAP = 20_000;

/** Fill prices carry six decimals; show four (enough to see slippage) and keep the rest on hover. */
function fillPrice(value: string): string {
  const n = toNumber(value);
  if (n === null) return DASH;
  return n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 4 });
}

export function FillsTab(props: TabProps) {
  const t = tradesText;
  const c = common;
  const f = () => props.filters;
  const [offset, setOffset] = createSignal(0);
  const [limit, setLimit] = createSignal(50);
  createEffect(on(() => [f().symbol, f().from, f().to], () => setOffset(0), { defer: true }));

  const query = (k: { symbol: string; from: string; to: string }) => ({
    symbol: k.symbol || undefined,
    from: k.from || undefined,
    to: k.to || undefined,
  });

  const page = useQuery(
    () => ({ symbol: f().symbol, from: f().from, to: f().to, offset: offset(), limit: limit(), tick: props.tick() }),
    (k) => api.fills({ ...query(k), offset: k.offset, limit: k.limit }),
  );
  const all = useQuery(
    () => ({ symbol: f().symbol, from: f().from, to: f().to, tick: props.tick() }),
    (k) =>
      fetchAll(
        (offset, limit) => api.fills({ ...query(k), offset, limit }).then((r) => ({ rows: r.fills, total: r.total })),
        5000,
        CAP,
      ),
  );

  const summary = createMemo(() => {
    const s = { count: 0, buys: 0, sells: 0, bought: 0, sold: 0, commission: 0, fees: 0, slippage: 0, pnl: 0, wins: 0, losses: 0 };
    for (const fill of all.data()?.rows ?? []) {
      s.count += 1;
      const notional = num(fill.notional);
      if (fill.side === "buy") {
        s.buys += 1;
        s.bought += notional;
      } else {
        s.sells += 1;
        s.sold += notional;
      }
      s.commission += num(fill.commission);
      s.fees += num(fill.fees);
      s.slippage += num(fill.slippage);
      const pnl = toNumber(fill.realized_pnl);
      if (pnl !== null) {
        s.pnl += pnl;
        if (pnl > 0) s.wins += 1;
        else if (pnl < 0) s.losses += 1;
      }
    }
    return s;
  });
  const slipBp = () => {
    const turnover = summary().bought + summary().sold;
    return turnover > 0 ? (summary().slippage / turnover) * 1e4 : null;
  };

  const exportCsv = () => {
    const data = all.data();
    if (!data) return;
    const tc = t().csv;
    const cols = t().fills.cols;
    const header = [
      tc.id, tc.order_id, tc.plan_id, tc.executed_at, tc.local_time, tc.exchange_time, cols.symbol, tc.name, cols.side, cols.qty,
      cols.price, cols.quote, tc.quote_ts, cols.slippage, cols.commission, cols.fees, cols.notional, cols.pnl,
    ];
    const rows = data.rows.map((x) => [
      x.id,
      x.order_id,
      x.plan_id,
      x.executed_at,
      fmtDateTime(x.executed_at),
      fmtDateTime(x.executed_at, MARKET_TZ),
      x.symbol,
      nameOf(props.names(), x.symbol),
      c().side[x.side],
      x.qty,
      x.price,
      x.quote_price,
      x.quote_ts ?? "",
      x.slippage,
      x.commission,
      x.fees,
      x.notional,
      x.realized_pnl ?? "",
    ]);
    exportRows(csvName("fills", f()), header, rows, data.truncated ? data.rows.length : null);
  };

  const ready = () => !!all.data();

  return (
    <>
      <Toolbar filters={f()} setFilters={props.setFilters} names={props.names()} dateHint={t().filter.date_hint_fills} />

      <Summary loading={all.loading()} note={truncationNote(all.data())}>
        <Kpi
          label={t().fills.kpi.count}
          value={<span class="num">{ready() ? fmtNum(summary().count, 0) : DASH}</span>}
          delta={<span class="muted">{tpl(t().fills.kpi.sides, { buys: fmtNum(summary().buys, 0), sells: fmtNum(summary().sells, 0) })}</span>}
        />
        <Kpi label={t().fills.kpi.bought} value={<span class="num">{ready() ? fmtMoney(summary().bought) : DASH}</span>} />
        <Kpi label={t().fills.kpi.sold} value={<span class="num">{ready() ? fmtMoney(summary().sold) : DASH}</span>} />
        <Kpi
          label={t().fills.kpi.commission}
          title={t().fills.kpi.costs_hint}
          value={<span class="num">{ready() ? fmtMoney(summary().commission) : DASH}</span>}
          delta={
            <span class="muted">
              {tpl(t().fills.kpi.costs_sub, { total: fmtMoney(summary().commission + summary().fees + summary().slippage) })}
            </span>
          }
        />
        <Kpi
          label={t().fills.kpi.fees}
          title={t().fills.kpi.fees_hint}
          value={<span class="num">{ready() ? fmtMoney(summary().fees) : DASH}</span>}
          delta={<span class="muted">{t().fills.kpi.fees_sub}</span>}
        />
        <Kpi
          label={t().fills.kpi.slippage}
          title={t().fills.slip_hint}
          value={<span class="num">{ready() ? fmtMoney(summary().slippage) : DASH}</span>}
          delta={<span class="muted">{tpl(t().fills.kpi.slippage_sub, { bp: slipBp() === null ? DASH : fmtNum(slipBp(), 1) })}</span>}
        />
        <Kpi
          label={t().fills.kpi.pnl}
          title={t().fills.pnl_hint}
          value={ready() ? <Money value={summary().pnl} signed /> : <span class="num">{DASH}</span>}
          delta={
            <span class="muted">
              {tpl(t().fills.kpi.pnl_sub, { n: fmtNum(summary().sells, 0), wins: fmtNum(summary().wins, 0), losses: fmtNum(summary().losses, 0) })}
            </span>
          }
        />
      </Summary>

      <section class="card">
        <div class="card-head">
          <div>
            <h2>{t().fills.card}</h2>
            <div class="sub">{t().fills.sub}</div>
          </div>
          <span class="spacer" />
          <ExportButton onClick={exportCsv} busy={all.loading() && !all.data()} disabled={!all.data()} />
        </div>
        <div class="card-body flush">
          <Switch>
            <Match when={page.error() && !page.data()}>
              <ErrorState error={page.error()} onRetry={page.refetch} />
            </Match>
            <Match when={!page.data()}>
              <Loading />
            </Match>
            <Match when={page.data()!.fills.length === 0}>
              <Empty title={t().fills.empty} icon="trades">
                <span>{t().fills.empty_hint}</span>
              </Empty>
            </Match>
            <Match when={page.data()}>
              <div class="table-wrap" classList={{ refetching: page.loading() }}>
                <table class="table compact act-table">
                  <thead>
                    <tr>
                      <th>{t().fills.cols.time}</th>
                      <th>{t().fills.cols.symbol}</th>
                      <th>{t().fills.cols.side}</th>
                      <th class="r">{t().fills.cols.qty}</th>
                      <th class="r">{t().fills.cols.price}</th>
                      <th class="r">{t().fills.cols.quote}</th>
                      <th class="r" title={t().fills.slip_hint}>
                        {t().fills.cols.slippage}
                      </th>
                      <th class="r">{t().fills.cols.commission}</th>
                      <th class="r">{t().fills.cols.fees}</th>
                      <th class="r">{t().fills.cols.notional}</th>
                      <th class="r" title={t().fills.pnl_hint}>
                        {t().fills.cols.pnl}
                      </th>
                      <th class="r">{t().fills.cols.plan}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={page.data()!.fills}>{(x) => <FillRow fill={x} props={props} />}</For>
                  </tbody>
                </table>
              </div>
            </Match>
          </Switch>
        </div>
        <Show when={page.data() && page.data()!.total > 0}>
          <Pager
            offset={offset()}
            limit={limit()}
            total={page.data()!.total}
            onOffset={setOffset}
            onLimit={(n) => {
              setLimit(n);
              setOffset(0);
            }}
          />
        </Show>
      </section>
    </>
  );
}

function FillRow(p: { fill: Fill; props: TabProps }) {
  const t = tradesText;
  const x = p.fill;
  const bp = () => {
    const notional = num(x.notional);
    return notional > 0 ? (num(x.slippage) / notional) * 1e4 : null;
  };
  return (
    <tr>
      <td>
        <DualTime value={x.executed_at} />
      </td>
      <td>
        <SymbolCell symbol={x.symbol} names={p.props.names()} onFilter={(symbol) => p.props.setFilters({ symbol })} />
      </td>
      <td>
        <SideChip side={x.side} />
      </td>
      <td class="r">{fmtQty(x.qty)}</td>
      <td class="r" title={x.price}>
        {fillPrice(x.price)}
      </td>
      <td class="r muted" title={x.quote_ts ? tpl(t().fills.quote_at, { time: fmtDual(x.quote_ts, true) }) : undefined}>
        {fmtPrice(x.quote_price)}
      </td>
      <td class="r">
        <span class="num">{fmtMoney(x.slippage)}</span>
        <Show when={bp() !== null}>
          <div class="act-sub">{fmtNum(bp(), 1)} bp</div>
        </Show>
      </td>
      <td class="r">{fmtMoney(x.commission)}</td>
      <td class="r">{fmtMoney(x.fees)}</td>
      <td class="r">{fmtMoney(x.notional)}</td>
      <td class="r">
        <Show when={x.realized_pnl !== null && x.realized_pnl !== undefined} fallback={<span class="muted">{DASH}</span>}>
          <Money value={x.realized_pnl} signed />
        </Show>
      </td>
      <td class="r">
        <PlanLink id={x.plan_id} />
      </td>
    </tr>
  );
}
