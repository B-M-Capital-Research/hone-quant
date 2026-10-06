import { For, Show, createMemo, createSignal } from "solid-js";
import { Icon } from "@/components/Icon";
import { Segmented, Switch, WeightBar } from "@/components/ui";
import { pick, tpl } from "@/i18n";
import { overviewText } from "@/i18n/overview";
import { fmtMoney, fmtPct, fmtPrice, fmtQty, moneyPolarity, polarity } from "@/lib/format";
import type { Board, Valuation } from "@/lib/types";
import { sectorLabel, store, stored } from "./util";

interface Row {
  symbol: string;
  name: string;
  sector: string;
  qty: number;
  avgCost: number | null;
  price: number | null;
  priceIsClose: boolean;
  dayPct: number | null;
  value: number;
  weight: number;
  target: number | null;
  unrealized: number | null;
  unrealizedPct: number | null;
  dayPnl: number | null;
  inUniverse: boolean;
  pending: boolean;
}

type SortKey = "symbol" | "value" | "weight" | "gap" | "unrealized" | "day";

const sum = (xs: (number | null | undefined)[]) => xs.reduce<number>((a, b) => a + (b ?? 0), 0);

export function Holdings(props: { valuation: Valuation; targets: Record<string, number> | null; board: Board | undefined; onPick: (symbol: string) => void }) {
  const t = overviewText;
  const [mode, setMode] = createSignal(stored("holdings.mode", "grouped", ["grouped", "flat"] as const));
  const [showPending, setShowPending] = createSignal(stored("holdings.pending", "on", ["on", "off"] as const) === "on");
  const [sort, setSort] = createSignal<{ key: SortKey; dir: 1 | -1 }>({ key: "weight", dir: -1 });

  const rows = createMemo<Row[]>(() => {
    const v = props.valuation;
    const targets = props.targets ?? {};
    const out: Row[] = v.positions
      .filter((p) => p.qty > 0)
      .map((p) => ({
        symbol: p.symbol,
        name: pick(p, "name"),
        sector: p.sector_id,
        qty: p.qty,
        avgCost: p.avg_cost,
        price: p.price,
        priceIsClose: p.price_source !== "quote",
        dayPct: p.day_change_pct,
        value: p.value,
        weight: p.weight,
        target: targets[p.symbol] ?? (props.targets ? 0 : null),
        unrealized: p.unrealized_pnl,
        unrealizedPct: p.unrealized_pct,
        dayPnl: p.day_pnl,
        inUniverse: p.in_universe,
        pending: false,
      }));
    if (showPending()) {
      const held = new Set(out.map((r) => r.symbol));
      for (const [symbol, target] of Object.entries(targets)) {
        if (held.has(symbol) || target <= 0) continue;
        const item = props.board?.items.find((i) => i.symbol === symbol);
        out.push({
          symbol,
          name: item ? pick(item, "name") : "",
          sector: item?.sector_id ?? "",
          qty: 0,
          avgCost: null,
          price: item?.close ?? null,
          priceIsClose: !item?.live,
          dayPct: null,
          value: 0,
          weight: 0,
          target,
          unrealized: null,
          unrealizedPct: null,
          dayPnl: null,
          inUniverse: true,
          pending: true,
        });
      }
    }
    return out;
  });

  const sorted = createMemo(() => {
    const { key, dir } = sort();
    const value = (r: Row): number | string => {
      switch (key) {
        case "symbol":
          return r.symbol;
        case "value":
          return r.value;
        case "weight":
          return r.weight;
        case "gap":
          return (r.target ?? 0) - r.weight;
        case "unrealized":
          return r.unrealized ?? -Infinity;
        case "day":
          return r.dayPct ?? -Infinity;
      }
    };
    return [...rows()].sort((a, b) => {
      const x = value(a);
      const y = value(b);
      return (x < y ? -1 : x > y ? 1 : 0) * dir;
    });
  });

  const groups = createMemo(() => {
    const sectors = props.board?.sectors ?? [];
    const order = sectors.map((s) => s.id);
    const known = new Set(order);
    const extra = [...new Set(rows().map((r) => r.sector).filter((s) => !known.has(s)))];
    return [...order, ...extra]
      .map((id) => {
        const members = rows()
          .filter((r) => r.sector === id)
          .sort((a, b) => b.weight - a.weight || (b.target ?? 0) - (a.target ?? 0));
        return {
          id,
          members,
          held: members.filter((m) => !m.pending).length,
          value: sum(members.map((m) => m.value)),
          weight: sum(members.map((m) => m.weight)),
          target: props.targets ? sum(members.map((m) => m.target)) : null,
          unrealized: sum(members.map((m) => m.unrealized)),
          dayPnl: members.some((m) => m.dayPnl != null) ? sum(members.map((m) => m.dayPnl)) : null,
        };
      })
      .filter((g) => g.members.length);
  });

  const targetCash = () => (props.targets ? Math.max(0, 1 - sum(Object.values(props.targets))) : null);
  const held = () => rows().filter((r) => !r.pending).length;

  const header = (key: SortKey, label: string, align: "r" | "" = "r") => (
    <th
      class={`${align} ${mode() === "flat" ? "sortable" : ""}`}
      onClick={() => {
        if (mode() !== "flat") return;
        setSort((s) => (s.key === key ? { key, dir: (s.dir * -1) as 1 | -1 } : { key, dir: key === "symbol" ? 1 : -1 }));
      }}
      aria-sort={mode() === "flat" && sort().key === key ? (sort().dir === 1 ? "ascending" : "descending") : undefined}
    >
      {label}
      <Show when={mode() === "flat" && sort().key === key}>
        <Icon name={sort().dir === 1 ? "arrow_up" : "arrow_down"} size={11} style={{ "margin-left": "3px", "vertical-align": "-1px" }} />
      </Show>
    </th>
  );

  const RowView = (r: Row) => (
    <tr class="clickable" classList={{ pending: r.pending }} onClick={() => props.onPick(r.symbol)}>
      <td>
        <div class="name-cell">
          <span class="row" style={{ gap: "6px" }}>
            <span class="ticker">{r.symbol}</span>
            <Show when={r.pending}>
              <span class="chip outline">{t().holdings.pending}</span>
            </Show>
            <Show when={!r.inUniverse}>
              <span class="chip yellow">{t().holdings.not_in_universe}</span>
            </Show>
          </span>
          <span class="name">{r.name}</span>
        </div>
      </td>
      <td class="r num">{r.pending ? "—" : fmtQty(r.qty)}</td>
      <td class="r num">{fmtPrice(r.avgCost)}</td>
      <td class="r">
        <div class="num" title={r.priceIsClose ? t().holdings.price_close : undefined}>
          {fmtPrice(r.price)}
          <Show when={r.priceIsClose && r.price != null}>
            <span class="muted">*</span>
          </Show>
        </div>
        <div class={`num xs ${polarity(r.dayPct, 2)}`}>{r.dayPct == null ? "" : fmtPct(r.dayPct, { sign: true })}</div>
      </td>
      <td class="r num">{r.pending ? "—" : fmtMoney(r.value, { dp: 0 })}</td>
      <td class="r">
        <div class="weight-cell">
          <WeightBar weight={r.weight} target={r.target} max={0.06} />
          <span class="num">
            {fmtPct(r.weight, { dp: 1 })}
            <span class="muted"> / {r.target == null ? "—" : fmtPct(r.target, { dp: 1 })}</span>
          </span>
        </div>
      </td>
      <td class="r">
        <Show when={r.unrealized != null} fallback={<span class="muted">—</span>}>
          <div class={`num ${moneyPolarity(r.unrealized, 0)}`}>{fmtMoney(r.unrealized, { sign: true, dp: 0 })}</div>
          <div class={`num xs ${polarity(r.unrealizedPct, 2)}`}>{fmtPct(r.unrealizedPct, { sign: true })}</div>
        </Show>
      </td>
      <td class={`r num ${moneyPolarity(r.dayPnl, 0)}`}>{r.dayPnl == null ? "—" : fmtMoney(r.dayPnl, { sign: true, dp: 0 })}</td>
    </tr>
  );

  return (
    <div class="card">
      <div class="card-head">
        <div style={{ "min-width": 0 }}>
          <h2>{t().holdings.title}</h2>
          <div class="sub">
            {tpl(t().holdings.sub, {
              held: held(),
              invested: fmtMoney(props.valuation.invested, { compact: true }),
              cash: fmtMoney(props.valuation.cash, { compact: true }),
            })}
          </div>
        </div>
        <div class="spacer" />
        <Show when={props.targets}>
          <Switch
            checked={showPending()}
            onChange={(on) => {
              setShowPending(on);
              store("holdings.pending", on ? "on" : "off");
            }}
            label={<span class="xs hide-sm">{t().holdings.show_pending}</span>}
          />
        </Show>
        <Segmented
          value={mode()}
          onChange={(v) => {
            setMode(v);
            store("holdings.mode", v);
          }}
          options={[
            { value: "grouped", label: t().holdings.grouped },
            { value: "flat", label: t().holdings.flat },
          ]}
        />
      </div>
      <div class="card-body flush">
        <Show when={rows().length} fallback={<div class="empty">{t().holdings.empty}</div>}>
          <div class="table-wrap holdings-scroll">
            <table class="table holdings-table">
              <thead>
                <tr>
                  {header("symbol", t().holdings.col_name, "")}
                  <th class="r">{t().holdings.col_qty}</th>
                  <th class="r">{t().holdings.col_cost}</th>
                  {header("day", t().holdings.col_price)}
                  {header("value", t().holdings.col_value)}
                  {header("weight", t().holdings.col_weight)}
                  {header("unrealized", t().holdings.col_unrealized)}
                  <th class="r">{t().holdings.col_day}</th>
                </tr>
              </thead>
              <Show
                when={mode() === "grouped"}
                fallback={
                  <tbody>
                    <For each={sorted()}>{(r) => RowView(r)}</For>
                  </tbody>
                }
              >
                <For each={groups()}>
                  {(g) => (
                    <tbody>
                      <tr class="group-row">
                        <td>
                          <span class="group-name">{sectorLabel(props.board?.sectors, g.id)}</span>
                          <span class="muted xs"> · {tpl(t().holdings.sector_subtotal, { n: g.held })}</span>
                        </td>
                        <td />
                        <td />
                        <td />
                        <td class="r num">{fmtMoney(g.value, { dp: 0 })}</td>
                        <td class="r num">
                          {fmtPct(g.weight, { dp: 1 })}
                          <span class="muted"> / {g.target == null ? "—" : fmtPct(g.target, { dp: 1 })}</span>
                        </td>
                        <td class={`r num ${moneyPolarity(g.unrealized, 0)}`}>{fmtMoney(g.unrealized, { sign: true, dp: 0 })}</td>
                        <td class={`r num ${moneyPolarity(g.dayPnl, 0)}`}>{g.dayPnl == null ? "—" : fmtMoney(g.dayPnl, { sign: true, dp: 0 })}</td>
                      </tr>
                      <For each={g.members}>{(r) => RowView(r)}</For>
                    </tbody>
                  )}
                </For>
              </Show>
              <tfoot>
                <tr>
                  <td>{t().holdings.cash_row}</td>
                  <td />
                  <td />
                  <td />
                  <td class="r num">{fmtMoney(props.valuation.cash, { dp: 0 })}</td>
                  <td class="r num">
                    {fmtPct(props.valuation.nav > 0 ? props.valuation.cash / props.valuation.nav : null, { dp: 1 })}
                    <span class="muted"> / {targetCash() == null ? "—" : fmtPct(targetCash(), { dp: 1 })}</span>
                  </td>
                  <td />
                  <td />
                </tr>
                <tr>
                  <td>{t().holdings.total_row}</td>
                  <td />
                  <td />
                  <td />
                  <td class="r num">{fmtMoney(props.valuation.nav, { dp: 0 })}</td>
                  <td class="r num">100%</td>
                  <td class={`r num ${moneyPolarity(props.valuation.unrealized_pnl, 0)}`}>{fmtMoney(props.valuation.unrealized_pnl, { sign: true, dp: 0 })}</td>
                  <td class={`r num ${moneyPolarity(props.valuation.day_pnl, 0)}`}>{props.valuation.day_pnl == null ? "—" : fmtMoney(props.valuation.day_pnl, { sign: true, dp: 0 })}</td>
                </tr>
              </tfoot>
            </table>
          </div>
        </Show>
      </div>
      <Show when={rows().some((r) => r.priceIsClose && !r.pending)}>
        <div class="card-foot">* {t().holdings.price_close}</div>
      </Show>
    </div>
  );
}

