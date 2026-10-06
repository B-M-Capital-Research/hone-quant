/** Orders tab of /trades: every order of every plan, with status, sizing and weights. */
import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on } from "solid-js";
import { Empty, ErrorState, Kpi, Loading, OrderStatusChip, SideChip } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { tradesText } from "@/i18n/trades";
import { api } from "@/lib/api";
import { DASH, fmtMoney, fmtNum, fmtPct, fmtPrice, fmtQty, fmtWeight } from "@/lib/format";
import type { OrderStatus, OrderWithPlan } from "@/lib/types";
import { Pager, PlanLink, SymbolCell } from "./components";
import { nameOf } from "./names";
import { useQuery } from "./query";
import { ExportButton, Summary, type TabProps, Toolbar, csvName, exportRows, truncationNote } from "./trades-common";
import { fetchAll, num } from "./util";

const STATUSES: OrderStatus[] = ["planned", "filled", "partially_filled", "rejected", "expired", "cancelled", "skipped"];
const CAP = 10_000;

export function OrdersTab(props: TabProps) {
  const t = tradesText;
  const c = common;
  const f = () => props.filters;
  const [offset, setOffset] = createSignal(0);
  const [limit, setLimit] = createSignal(50);
  createEffect(on(() => [f().symbol, f().from, f().to, f().status], () => setOffset(0), { defer: true }));

  const query = (k: { symbol: string; from: string; to: string }) => ({
    symbol: k.symbol || undefined,
    from: k.from || undefined,
    to: k.to || undefined,
  });

  const page = useQuery(
    () => ({ symbol: f().symbol, from: f().from, to: f().to, status: f().status, offset: offset(), limit: limit(), tick: props.tick() }),
    (k) => api.orders({ ...query(k), status: k.status || undefined, offset: k.offset, limit: k.limit }),
  );
  const all = useQuery(
    () => ({ symbol: f().symbol, from: f().from, to: f().to, tick: props.tick() }),
    (k) =>
      fetchAll(
        (offset, limit) => api.orders({ ...query(k), offset, limit }).then((r) => ({ rows: r.orders, total: r.total })),
        1000,
        CAP,
      ),
  );

  const counts = createMemo(() => {
    const out: Record<string, number> = {};
    for (const row of all.data()?.rows ?? []) out[row.status] = (out[row.status] ?? 0) + 1;
    return out;
  });
  const scoped = createMemo(() => {
    const rows = all.data()?.rows ?? [];
    return f().status ? rows.filter((r) => r.status === f().status) : rows;
  });
  const summary = createMemo(() => {
    const s = { count: 0, buys: 0, sells: 0, buyValue: 0, sellValue: 0, planned: 0, done: 0, rejected: 0, expired: 0, cancelled: 0, skipped: 0 };
    for (const o of scoped()) {
      s.count += 1;
      const value = num(o.qty) * o.ref_price;
      if (o.side === "buy") {
        s.buys += 1;
        s.buyValue += value;
      } else {
        s.sells += 1;
        s.sellValue += value;
      }
      if (o.status === "planned") s.planned += 1;
      else if (o.status === "filled" || o.status === "partially_filled") s.done += 1;
      else if (o.status === "rejected") s.rejected += 1;
      else if (o.status === "expired") s.expired += 1;
      else if (o.status === "cancelled") s.cancelled += 1;
      else if (o.status === "skipped") s.skipped += 1;
    }
    return s;
  });

  /** "3 not filled · 1 expired" — only the outcomes that occurred. */
  const notExecutedParts = () => {
    if (!all.data()) return "";
    const parts = t().orders.kpi.not_filled_parts;
    const s = summary();
    const list = (["rejected", "expired", "cancelled", "skipped"] as const)
      .filter((key) => s[key] > 0)
      .map((key) => tpl(parts[key], { n: fmtNum(s[key], 0) }));
    return list.length ? list.join(" · ") : s.count - s.planned > 0 ? t().orders.kpi.all_executed : "";
  };

  const exportCsv = () => {
    const data = all.data();
    if (!data) return;
    const tc = t().csv;
    const cols = t().orders.cols;
    const header = [
      tc.id, tc.plan_id, cols.date, tc.slot, cols.symbol, tc.name, cols.side, cols.reason, cols.qty, tc.filled_qty, cols.ref_price,
      cols.ref_value, tc.weight_before, tc.weight_target, tc.weight_after, cols.status, tc.status_reason, tc.created_at,
    ];
    const reasons = c().order_status_reason as Record<string, string>;
    const rows = scoped().map((o) => [
      o.id,
      o.plan_id,
      o.trade_date,
      c().slot[o.slot],
      o.symbol,
      nameOf(props.names(), o.symbol),
      c().side[o.side],
      c().reason[o.reason],
      o.qty,
      o.filled_qty,
      o.ref_price,
      (num(o.qty) * o.ref_price).toFixed(2),
      o.weight_before.toFixed(6),
      o.weight_target.toFixed(6),
      o.weight_after.toFixed(6),
      c().order_status[o.status],
      o.status_reason ? reasons[o.status_reason] ?? o.status_reason : "",
      o.created_at,
    ]);
    exportRows(csvName("orders", f()), header, rows, data.truncated ? data.rows.length : null);
  };

  return (
    <>
      <Toolbar
        filters={f()}
        setFilters={props.setFilters}
        names={props.names()}
        dateHint={t().filter.date_hint_orders}
        extraActive={!!f().status}
        extra={
          <div class="act-field">
            <label class="act-field-label" for="orders-status">
              {t().filter.status}
            </label>
            <select
              id="orders-status"
              class="select act-select"
              classList={{ set: !!f().status }}
              value={f().status}
              onChange={(e) => props.setFilters({ status: e.currentTarget.value })}
            >
              <option value="">
                {t().filter.all_statuses}
                {all.data() ? ` (${fmtNum(all.data()!.rows.length, 0)})` : ""}
              </option>
              <For each={STATUSES.filter((s) => counts()[s] || s === f().status)}>
                {(s) => (
                  <option value={s}>
                    {c().order_status[s]} ({fmtNum(counts()[s] ?? 0, 0)})
                  </option>
                )}
              </For>
            </select>
          </div>
        }
      />

      <Summary loading={all.loading()} note={truncationNote(all.data())}>
        <Kpi
          label={t().orders.kpi.count}
          value={<span class="num">{all.data() ? fmtNum(summary().count, 0) : DASH}</span>}
          delta={
            <span class="muted">
              {tpl(t().orders.kpi.sides, { buys: fmtNum(summary().buys, 0), sells: fmtNum(summary().sells, 0) })}
              <Show when={summary().planned > 0}> · {tpl(t().orders.kpi.pending, { n: summary().planned })}</Show>
            </span>
          }
        />
        <Kpi
          label={t().orders.kpi.buy_value}
          title={t().orders.kpi.value_hint}
          value={<span class="num">{all.data() ? fmtMoney(summary().buyValue) : DASH}</span>}
        />
        <Kpi
          label={t().orders.kpi.sell_value}
          title={t().orders.kpi.value_hint}
          value={<span class="num">{all.data() ? fmtMoney(summary().sellValue) : DASH}</span>}
        />
        <Kpi
          label={t().orders.kpi.fill_rate}
          title={t().orders.kpi.fill_rate_hint}
          value={
            <span class="num">
              {all.data() && summary().count - summary().planned > 0 ? fmtPct(summary().done / (summary().count - summary().planned), { dp: 1 }) : DASH}
            </span>
          }
          delta={
            <span class="muted">
              {tpl(t().orders.kpi.fill_rate_sub, { done: fmtNum(summary().done, 0), resolved: fmtNum(summary().count - summary().planned, 0) })}
            </span>
          }
        />
        <Kpi
          label={t().orders.kpi.not_filled}
          value={<span class="num">{all.data() ? fmtNum(summary().rejected + summary().expired + summary().cancelled + summary().skipped, 0) : DASH}</span>}
          delta={<span class="muted">{notExecutedParts()}</span>}
        />
      </Summary>

      <section class="card">
        <div class="card-head">
          <div>
            <h2>{t().orders.card}</h2>
            <div class="sub">{t().orders.sub}</div>
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
            <Match when={page.data()!.orders.length === 0}>
              <Empty title={t().orders.empty} icon="trades">
                <span>{t().orders.empty_hint}</span>
              </Empty>
            </Match>
            <Match when={page.data()}>
              <div class="table-wrap" classList={{ refetching: page.loading() }}>
                <table class="table compact act-table">
                  <thead>
                    <tr>
                      <th>{t().orders.cols.date}</th>
                      <th>{t().orders.cols.symbol}</th>
                      <th>{t().orders.cols.side}</th>
                      <th>{t().orders.cols.reason}</th>
                      <th class="r">{t().orders.cols.qty}</th>
                      <th class="r">{t().orders.cols.ref_price}</th>
                      <th class="r">{t().orders.cols.ref_value}</th>
                      <th class="r">{t().orders.cols.weight}</th>
                      <th>{t().orders.cols.status}</th>
                      <th class="r">{t().orders.cols.plan}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={page.data()!.orders}>{(o) => <OrderRow order={o} props={props} />}</For>
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

function OrderRow(p: { order: OrderWithPlan; props: TabProps }) {
  const t = tradesText;
  const c = common;
  const o = p.order;
  const partial = () => o.status === "partially_filled" || (num(o.filled_qty) > 0 && num(o.filled_qty) !== num(o.qty));
  const delta = () => o.weight_target - o.weight_before;
  return (
    <tr>
      <td>
        <div class="act-date">
          <span class="num">{o.trade_date}</span>
          <span class="act-sub">{c().slot[`${o.slot}_short` as const]}</span>
        </div>
      </td>
      <td>
        <SymbolCell symbol={o.symbol} names={p.props.names()} onFilter={(symbol) => p.props.setFilters({ symbol })} />
      </td>
      <td>
        <SideChip side={o.side} />
      </td>
      <td class="nowrap">{c().reason[o.reason]}</td>
      <td class="r">
        <span class="num">{fmtQty(o.qty)}</span>
        <Show when={partial()}>
          <div class="act-sub">{tpl(t().orders.filled_part, { qty: fmtQty(o.filled_qty) })}</div>
        </Show>
      </td>
      <td class="r">{fmtPrice(o.ref_price)}</td>
      <td class="r">{fmtMoney(num(o.qty) * o.ref_price)}</td>
      <td class="r nowrap" title={tpl(t().orders.weight_after, { after: fmtWeight(o.weight_after, 2) })}>
        <span class="num muted">{fmtWeight(o.weight_before, 2)}</span>
        <span class="act-arrow" classList={{ up: delta() > 0, down: delta() < 0 }} aria-hidden="true">
          →
        </span>
        <span class="num">{fmtWeight(o.weight_target, 2)}</span>
      </td>
      <td>
        <OrderStatusChip status={o.status} reason={o.status_reason} />
      </td>
      <td class="r">
        <PlanLink id={o.plan_id} />
      </td>
    </tr>
  );
}
