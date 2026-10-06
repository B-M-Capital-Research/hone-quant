import { For, Show, createEffect, createMemo, createSignal, on } from "solid-js";
import { Icon } from "@/components/Icon";
import { Segmented, SideChip, downloadCsv } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { researchText } from "@/i18n/research";
import { fmtMoney, fmtNum, fmtPct, fmtPrice, fmtQty } from "@/lib/format";
import type { BacktestResult } from "@/lib/types";
import { Card } from "./components";
import { createMedia } from "./model";

type Trade = BacktestResult["trades"][number];
type Position = BacktestResult["final_positions"][number];


export function TradeLog(props: {
  id: number;
  trades: Trade[];
  total: number;
  assetName: (symbol: string) => string;
}) {
  const r = researchText;
  const c = common;
  const [symbol, setSymbol] = createSignal("");
  const [side, setSide] = createSignal<"all" | "buy" | "sell">("all");
  const [page, setPage] = createSignal(0);
  const narrow = createMedia("(max-width: 760px)");
  const pageSize = () => (narrow() ? 25 : 50);
  const symbols = createMemo(() => [...new Set(props.trades.map((t) => t.symbol))].sort());
  // Newest first, matching the live trade log.
  const filtered = createMemo(() => {
    const s = symbol();
    const d = side();
    const out: Trade[] = [];
    for (let i = props.trades.length - 1; i >= 0; i--) {
      const t = props.trades[i];
      if (s && t.symbol !== s) continue;
      if (d !== "all" && t.side !== d) continue;
      out.push(t);
    }
    return out;
  });
  const pages = () => Math.max(1, Math.ceil(filtered().length / pageSize()));
  createEffect(on([symbol, side, pageSize], () => setPage(0), { defer: true }));
  const visible = createMemo(() => filtered().slice(page() * pageSize(), (page() + 1) * pageSize()));
  const totals = createMemo(() => {
    let notional = 0;
    let cost = 0;
    for (const t of filtered()) {
      notional += t.notional;
      cost += t.cost;
    }
    return { notional, cost };
  });

  const exportCsv = () => {
    const d = r().detail;
    downloadCsv(
      `backtest-${props.id}-trades${symbol() ? `-${symbol()}` : ""}.csv`,
      [c().words.date, d.col_slot, c().words.symbol, c().words.name, d.col_side, d.col_reason, d.col_qty, d.col_price, d.col_notional, d.col_cost],
      filtered().map((t) => [
        t.date,
        t.slot === "open" ? c().slot.open_short : c().slot.close_short,
        t.symbol,
        props.assetName(t.symbol),
        c().side[t.side],
        c().reason[t.reason],
        t.qty,
        t.price.toFixed(4),
        t.notional.toFixed(2),
        t.cost.toFixed(2),
      ]),
    );
  };

  return (
    <Card
      title={r().detail.trades_title}
      sub={
        <>
          {tpl(r().detail.trades_sub, { n: fmtNum(props.total, 0) })}
          <Show when={props.total > props.trades.length}>
            {" · "}
            {tpl(r().detail.trades_truncated, { kept: fmtNum(props.trades.length, 0), total: fmtNum(props.total, 0) })}
          </Show>
        </>
      }
      flush
      actions={
        <button class="btn sm" type="button" onClick={exportCsv} disabled={!filtered().length}>
          <Icon name="download" size={14} />
          {c().actions.export_csv}
        </button>
      }
    >
      <div class="rs-filters">
        <select class="select rs-filter-select" aria-label={c().words.symbol} value={symbol()} onChange={(e) => setSymbol(e.currentTarget.value)}>
          <option value="">{r().detail.trades_filter_symbol}</option>
          <For each={symbols()}>{(s) => <option value={s}>{props.assetName(s) ? `${s} · ${props.assetName(s)}` : s}</option>}</For>
        </select>
        <Segmented
          value={side()}
          label={r().detail.col_side}
          onChange={setSide}
          options={[
            { value: "all", label: r().detail.trades_side_all },
            { value: "buy", label: c().side.buy },
            { value: "sell", label: c().side.sell },
          ]}
        />
        <span class="spacer" />
        <span class="muted xs num">
          {tpl(r().detail.trades_summary, {
            n: fmtNum(filtered().length, 0),
            notional: fmtMoney(totals().notional, { compact: true }),
            cost: fmtMoney(totals().cost, { compact: true }),
          })}
        </span>
      </div>
      <Show when={visible().length} fallback={<div class="empty">{r().detail.trades_none}</div>}>
        <div class="table-wrap">
          <table class="table compact">
            <thead>
              <tr>
                <th>{c().words.date}</th>
                <th>{r().detail.col_slot}</th>
                <th>{c().words.symbol}</th>
                <th>{r().detail.col_side}</th>
                <th>{r().detail.col_reason}</th>
                <th class="r">{r().detail.col_qty}</th>
                <th class="r">{r().detail.col_price}</th>
                <th class="r">{r().detail.col_notional}</th>
                <th class="r">{r().detail.col_cost}</th>
              </tr>
            </thead>
            <tbody>
              <For each={visible()}>
                {(t) => (
                  <tr>
                    <td class="num nowrap">{t.date}</td>
                    <td class="nowrap muted">{t.slot === "open" ? c().slot.open_short : c().slot.close_short}</td>
                    <td class="rs-inline-name">
                      <span class="ticker">{t.symbol}</span>
                      <span class="muted xs">{props.assetName(t.symbol)}</span>
                    </td>
                    <td>
                      <SideChip side={t.side} />
                    </td>
                    <td class="nowrap">{c().reason[t.reason]}</td>
                    <td class="r">{fmtQty(t.qty)}</td>
                    <td class="r">{fmtPrice(t.price)}</td>
                    <td class="r">{fmtMoney(t.notional)}</td>
                    <td class="r muted">{fmtMoney(t.cost)}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
      <Show when={pages() > 1}>
        <div class="card-foot rs-pager">
          <span>{tpl(r().detail.trades_page, { page: page() + 1, pages: pages() })}</span>
          <span class="spacer" />
          <button class="btn sm" type="button" disabled={page() === 0} onClick={() => setPage((p) => Math.max(0, p - 1))}>
            <Icon name="chevron_left" size={14} />
            {r().detail.trades_prev}
          </button>
          <button class="btn sm" type="button" disabled={page() >= pages() - 1} onClick={() => setPage((p) => Math.min(pages() - 1, p + 1))}>
            {r().detail.trades_next}
            <Icon name="chevron_right" size={14} />
          </button>
        </div>
      </Show>
    </Card>
  );
}

export function PositionsTable(props: {
  positions: Position[];
  cash: number;
  nav: number;
  date: string;
  assetName: (symbol: string) => string;
  sectorName: (id: string | null | undefined) => string;
}) {
  const r = researchText;
  const c = common;
  const LIMIT = 10;
  const [all, setAll] = createSignal(false);
  const maxWeight = createMemo(() => Math.max(0.05, ...props.positions.map((p) => p.weight)));
  const shown = createMemo(() => (all() ? props.positions : props.positions.slice(0, LIMIT)));
  return (
    <Card title={r().detail.positions_title} sub={tpl(r().detail.positions_sub, { n: props.positions.length, date: props.date })} flush>
      <div class="table-wrap">
        <table class="table compact">
          <thead>
            <tr>
              <th>{c().words.symbol}</th>
              <th>{c().words.sector}</th>
              <th class="r">{r().detail.col_qty}</th>
              <th class="r">{r().detail.col_value}</th>
              <th class="r">{r().detail.col_weight}</th>
            </tr>
          </thead>
          <tbody>
            <For each={shown()}>
              {(p) => (
                <tr>
                  <td>
                    <div class="name-cell">
                      <span class="ticker">{p.symbol}</span>
                      <span class="name">{props.assetName(p.symbol)}</span>
                    </div>
                  </td>
                  <td class="rs-sector-cell" title={props.sectorName(p.sector)}>
                    {props.sectorName(p.sector)}
                  </td>
                  <td class="r">{fmtQty(p.qty)}</td>
                  <td class="r">{fmtMoney(p.value, { dp: 0 })}</td>
                  <td class="r">
                    <div class="rs-weight-cell">
                      <span>{fmtPct(p.weight, { dp: 2 })}</span>
                      <span class="rs-mini-bar" aria-hidden="true">
                        <i style={{ width: `${Math.min(100, (p.weight / maxWeight()) * 100)}%` }} />
                      </span>
                    </div>
                  </td>
                </tr>
              )}
            </For>
            <Show when={props.positions.length > LIMIT}>
              <tr class="rs-more-row">
                <td colSpan={5}>
                  <button class="btn ghost sm" type="button" onClick={() => setAll((v) => !v)}>
                    <Icon name={all() ? "chevron_down" : "chevron_right"} size={14} />
                    {all() ? r().detail.show_less : tpl(r().detail.show_all, { n: props.positions.length })}
                  </button>
                </td>
              </tr>
            </Show>
          </tbody>
          <tfoot>
            <tr>
              <td colSpan={3}>{c().words.cash}</td>
              <td class="r">{fmtMoney(props.cash, { dp: 0 })}</td>
              <td class="r">{fmtPct(props.nav > 0 ? props.cash / props.nav : null, { dp: 2 })}</td>
            </tr>
            <tr>
              <td colSpan={3}>{r().detail.total_nav}</td>
              <td class="r">{fmtMoney(props.nav, { dp: 0 })}</td>
              <td class="r">100.00%</td>
            </tr>
          </tfoot>
        </table>
      </div>
    </Card>
  );
}
