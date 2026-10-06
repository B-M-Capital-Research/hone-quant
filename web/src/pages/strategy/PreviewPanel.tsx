import { For, Show, createMemo, createSignal } from "solid-js";
import { Icon } from "@/components/Icon";
import { Kpi, Pct, SideChip, WeightBar } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { strategyText } from "@/i18n/strategy";
import { DASH, fmtDual, fmtMoney, fmtNum, fmtPct, fmtPrice, fmtQty, fmtWeight } from "@/lib/format";
import type { Asset, AssetDiagnostics, AssetStatus, Preview, Sector, UniverseView } from "@/lib/types";
import { compact } from "./params";
import { pickText } from "./format";
import type { PreviewRun } from "./workbench";

const STATUS_TONE: Record<AssetStatus, string> = {
  active: "",
  excluded: "red",
  frozen: "blue",
  no_price: "orange",
  insufficient_history: "yellow",
  below_min_weight: "outline",
};

export function StatusChip(props: { status: AssetStatus }) {
  return <span class={`chip ${STATUS_TONE[props.status] ?? ""}`}>{common().asset_status[props.status] ?? props.status}</span>;
}

/** Signed difference in percentage points, neutral ink (sign carries the direction). */
function Delta(props: { value: number | null | undefined; dp?: number }) {
  const v = () => props.value;
  return (
    <span class="num st-delta" classList={{ zero: v() == null || Math.abs(v()!) < 5e-5 }}>
      {v() == null ? DASH : Math.abs(v()!) < 5e-5 ? (0).toFixed(props.dp ?? 2) : fmtPct(v(), { dp: props.dp ?? 2, sign: true }).replace("%", "")}
    </span>
  );
}

type Tab = "sectors" | "assets" | "orders";
type SortKey = "target" | "change" | "symbol" | "momentum";

interface Names {
  asset: (symbol: string) => Asset | undefined;
  sector: (id: string) => Sector | undefined;
}

export function PreviewPanel(props: {
  run: PreviewRun;
  universe: UniverseView | undefined;
  outdated: boolean;
  running: boolean;
  onRerun: () => void;
}) {
  const t = strategyText;
  const c = common;
  const [tab, setTab] = createSignal<Tab>("sectors");
  const [sectorFilter, setSectorFilter] = createSignal<string>("");
  const [statusFilter, setStatusFilter] = createSignal<AssetStatus | "">("");
  const [sort, setSort] = createSignal<SortKey>("target");

  const r = () => props.run.result;
  const cmp = () => (props.run.sameAsActive ? null : props.run.active);

  const names = createMemo<Names>(() => {
    const u = props.universe;
    const assets = new Map<string, Asset>();
    for (const a of [...(u?.removed ?? []), ...(u?.assets ?? [])]) assets.set(a.symbol, a);
    const sectors = new Map<string, Sector>((u?.sectors ?? []).map((s) => [s.id, s]));
    return { asset: (s) => assets.get(s), sector: (id) => sectors.get(id) };
  });
  const sectorName = (id: string) => {
    const s = names().sector(id);
    return s ? pickText(s, "name") : id;
  };
  const companyName = (symbol: string) => {
    const a = names().asset(symbol);
    return a ? pickText(a, "name") : "";
  };

  const activeAssetWeights = createMemo(() => new Map((cmp()?.targets.assets ?? []).map((a) => [a.symbol, a.weight])));
  const activeSectorWeights = createMemo(() => new Map((cmp()?.targets.sectors ?? []).map((s) => [s.sector, s.weight])));
  const current = (symbol: string) => props.run.current?.[symbol] ?? 0;

  // Sector momentum ordinal (1 = strongest) among sectors with a momentum value.
  const sectorOrdinal = createMemo(() => {
    const valid = r().targets.sectors.filter((s) => s.momentum !== null && s.active_members > 0);
    const sorted = [...valid].sort((a, b) => (b.momentum ?? 0) - (a.momentum ?? 0));
    return { rank: new Map(sorted.map((s, i) => [s.sector, i + 1])), n: sorted.length };
  });
  const sectorTilt = (rank: number | null) => (rank === null ? 1 : Math.max(0, 1 + props.run.params.sector.momentum_tilt * (2 * rank - 1)));

  const statusCounts = createMemo(() => {
    const counts = new Map<AssetStatus, number>();
    for (const a of r().targets.assets) {
      if (sectorFilter() && a.sector !== sectorFilter()) continue;
      counts.set(a.status, (counts.get(a.status) ?? 0) + 1);
    }
    return [...counts.entries()];
  });

  const assetRows = createMemo<AssetDiagnostics[]>(() => {
    const rows = r().targets.assets.filter((a) => (!sectorFilter() || a.sector === sectorFilter()) && (!statusFilter() || a.status === statusFilter()));
    const change = (a: AssetDiagnostics) => a.weight - (activeAssetWeights().get(a.symbol) ?? 0);
    const sorted = [...rows];
    switch (sort()) {
      case "symbol":
        sorted.sort((a, b) => a.symbol.localeCompare(b.symbol));
        break;
      case "change":
        sorted.sort((a, b) => Math.abs(change(b)) - Math.abs(change(a)) || b.weight - a.weight);
        break;
      case "momentum":
        sorted.sort((a, b) => (b.momentum ?? -Infinity) - (a.momentum ?? -Infinity));
        break;
      default:
        sorted.sort((a, b) => b.weight - a.weight || a.symbol.localeCompare(b.symbol));
    }
    return sorted;
  });

  const maxAssetWeight = createMemo(() => Math.max(0.05, ...r().targets.assets.map((a) => Math.max(a.weight, current(a.symbol)))));
  const maxSectorWeight = createMemo(() => Math.max(0.05, ...r().targets.sectors.map((s) => Math.max(s.weight, s.budget, s.cap))));

  const buys = () => r().orders.filter((o) => o.side === "buy");
  const sells = () => r().orders.filter((o) => o.side === "sell");
  const sum = (list: { notional: number }[]) => list.reduce((acc, o) => acc + o.notional, 0);
  const cashAfter = () => r().cash + sum(sells()) - sum(buys()) - r().est_costs;

  const skippedGroups = createMemo(() => {
    const groups = new Map<string, string[]>();
    for (const s of r().skipped) groups.set(s.reason, [...(groups.get(s.reason) ?? []), s.symbol]);
    return [...groups.entries()];
  });

  const metrics = (p: Preview) => ({
    exposure: p.targets.exposure_target,
    invested: p.targets.invested,
    vol: p.targets.est_vol,
    orders: p.orders.length,
    turnover: p.turnover,
    costs: p.est_costs,
  });

  return (
    <div class="stack" style={{ gap: "14px" }}>
      <Show when={props.outdated}>
        <div class="callout warn">
          <Icon name="alert" size={16} />
          <div class="row wrap" style={{ flex: 1, gap: "8px 14px" }}>
            <span>{t().preview.outdated}</span>
            <span class="spacer" />
            <button class="btn sm" onClick={() => props.onRerun()} disabled={props.running}>
              <Icon name="refresh" size={13} /> {t().actions.rerun}
            </button>
          </div>
        </div>
      </Show>

      <div class="kpis st-kpis" classList={{ refetching: props.running }}>
        <Kpi label={t().preview.exposure} value={fmtPct(r().targets.exposure_target, { dp: 1 })} delta={<span class="muted">{t().preview.breadth} {fmtPct(r().targets.breadth, { dp: 1 })}</span>} />
        <Kpi label={t().preview.invested} value={fmtPct(r().targets.invested, { dp: 1 })} delta={<span class="muted">{t().preview.cash} {fmtPct(r().targets.cash_weight, { dp: 1 })}</span>} />
        <Kpi
          label={t().preview.est_vol}
          value={fmtPct(r().targets.est_vol, { dp: 1 })}
          delta={
            <Show when={r().targets.vol_scale != null} fallback={<span class="muted">&nbsp;</span>}>
              <span class="muted">{tpl(t().preview.vol_scaled, { scale: compact(r().targets.vol_scale ?? 1, 2) })}</span>
            </Show>
          }
        />
        <Kpi label={t().preview.orders} value={String(r().orders.length)} delta={<span class="muted">{tpl(t().preview.buys_sells, { b: buys().length, s: sells().length })}</span>} />
        <Kpi label={t().preview.turnover} value={fmtPct(r().turnover, { dp: 1 })} delta={<span class="muted">{t().preview.costs} {fmtMoney(r().est_costs)}</span>} />
      </div>

      <p class="xs muted st-method-note">
        {tpl(t().preview.method, { from: r().history.from ?? DASH, to: r().history.to ?? DASH })} {fmtDual(r().as_of, true)}
      </p>

      <Show when={props.run.sameAsActive}>
        <div class="callout">
          <Icon name="info" size={16} />
          <span>{t().preview.same_as_active}</span>
        </div>
      </Show>
      <Show when={props.run.activeFailed}>
        <div class="callout warn">
          <Icon name="alert" size={16} />
          <span>{t().preview.active_failed}</span>
        </div>
      </Show>

      <Show when={cmp()}>
        {(active) => {
          const a = () => metrics(active());
          const b = () => metrics(r());
          const rows = () => [
            { label: t().preview.exposure, a: fmtPct(a().exposure, { dp: 1 }), b: fmtPct(b().exposure, { dp: 1 }), d: <Delta value={b().exposure - a().exposure} dp={1} /> },
            { label: t().preview.invested, a: fmtPct(a().invested, { dp: 1 }), b: fmtPct(b().invested, { dp: 1 }), d: <Delta value={b().invested - a().invested} dp={1} /> },
            {
              label: t().preview.est_vol,
              a: fmtPct(a().vol, { dp: 1 }),
              b: fmtPct(b().vol, { dp: 1 }),
              d: <Delta value={a().vol != null && b().vol != null ? b().vol! - a().vol! : null} dp={1} />,
            },
            { label: t().preview.orders, a: String(a().orders), b: String(b().orders), d: <span class="num st-delta">{signedInt(b().orders - a().orders)}</span> },
            { label: t().preview.turnover, a: fmtPct(a().turnover, { dp: 1 }), b: fmtPct(b().turnover, { dp: 1 }), d: <Delta value={b().turnover - a().turnover} dp={1} /> },
            {
              label: t().preview.costs,
              a: fmtMoney(a().costs),
              b: fmtMoney(b().costs),
              d: <span class="num st-delta">{fmtMoney(b().costs - a().costs, { sign: true })}</span>,
            },
          ];
          return (
            <div class="st-compare">
              <div class="table-wrap">
                <table class="table compact">
                  <thead>
                    <tr>
                      <th />
                      <th class="r">{t().preview.col_active}</th>
                      <th class="r">{t().preview.col_draft}</th>
                      <th class="r">{t().preview.col_diff}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={rows()}>
                      {(row) => (
                        <tr>
                          <td class="muted">{row.label}</td>
                          <td class="r">{row.a}</td>
                          <td class="r">
                            <b>{row.b}</b>
                          </td>
                          <td class="r">{row.d}</td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
              <p class="xs muted">{t().preview.diff_note}</p>
            </div>
          );
        }}
      </Show>

      <div class="tabs st-subtabs" role="tablist">
        <button role="tab" aria-selected={tab() === "sectors"} onClick={() => setTab("sectors")}>
          {t().preview.tab_sectors}
        </button>
        <button role="tab" aria-selected={tab() === "assets"} onClick={() => setTab("assets")}>
          {t().preview.tab_assets}
        </button>
        <button role="tab" aria-selected={tab() === "orders"} onClick={() => setTab("orders")}>
          {t().preview.tab_orders} <span class="st-count">{r().orders.length}</span>
        </button>
      </div>

      <Show when={tab() === "sectors"}>
        <div class="table-wrap">
          <table class="table compact st-preview-table">
            <thead>
              <tr>
                <th>{t().preview.h_sector}</th>
                <th class="r">{t().preview.h_eligible}</th>
                <th class="r">{t().preview.h_vol}</th>
                <th class="r">{t().preview.h_momentum}</th>
                <th class="r">{t().preview.h_rank}</th>
                <th class="r">{t().preview.h_tilt}</th>
                <th class="r">{t().preview.h_trend_factor}</th>
                <th class="r">{t().preview.h_cap}</th>
                <th class="r">{t().preview.h_budget}</th>
                <th>{t().preview.h_weight}</th>
                <Show when={cmp()}>
                  <th class="r">{t().preview.h_vs_active}</th>
                </Show>
              </tr>
            </thead>
            <tbody>
              <For each={r().targets.sectors}>
                {(s) => (
                  <tr>
                    <td class="nowrap">{sectorName(s.sector)}</td>
                    <td class="r">{s.active_members}</td>
                    <td class="r">{fmtPct(s.vol, { dp: 1 })}</td>
                    <td class="r">
                      <Pct value={s.momentum} dp={1} />
                    </td>
                    <td class="r muted">{sectorOrdinal().rank.get(s.sector) ? `${sectorOrdinal().rank.get(s.sector)}/${sectorOrdinal().n}` : DASH}</td>
                    <td class="r">×{fmtNum(sectorTilt(s.momentum_rank), 2)}</td>
                    <td class="r">×{fmtNum(s.trend_factor, 2)}</td>
                    <td class="r muted">{fmtWeight(s.cap, 1)}</td>
                    <td class="r">{fmtWeight(s.budget, 2)}</td>
                    <td>
                      <div class="st-bar-cell">
                        <WeightBar weight={s.weight} max={maxSectorWeight()} />
                        <b class="num">{fmtWeight(s.weight, 2)}</b>
                      </div>
                    </td>
                    <Show when={cmp()}>
                      <td class="r">
                        <Delta value={s.weight - (activeSectorWeights().get(s.sector) ?? 0)} />
                      </td>
                    </Show>
                  </tr>
                )}
              </For>
            </tbody>
            <tfoot>
              <tr>
                <td>{t().preview.total}</td>
                <td class="r">{r().targets.sectors.reduce((a, s) => a + s.active_members, 0)}</td>
                <td colspan={6} />
                <td class="r">{fmtWeight(r().targets.sectors.reduce((a, s) => a + s.budget, 0), 2)}</td>
                <td>
                  <span class="num">{fmtWeight(r().targets.invested, 2)}</span>
                  <span class="muted xs"> · {t().preview.cash_row} {fmtWeight(r().targets.cash_weight, 2)}</span>
                </td>
                <Show when={cmp()}>
                  <td class="r">
                    <Delta value={r().targets.invested - (cmp()?.targets.invested ?? 0)} />
                  </td>
                </Show>
              </tr>
            </tfoot>
          </table>
        </div>
      </Show>

      <Show when={tab() === "assets"}>
        <div class="row wrap st-filters">
          <select class="select st-select" value={sectorFilter()} onChange={(e) => setSectorFilter(e.currentTarget.value)} aria-label={t().preview.h_sector}>
            <option value="">{t().preview.all_sectors}</option>
            <For each={r().targets.sectors}>{(s) => <option value={s.sector}>{sectorName(s.sector)}</option>}</For>
          </select>
          <div class="pill-list">
            <button class="chip st-filter-chip" classList={{ selected: statusFilter() === "" }} onClick={() => setStatusFilter("")}>
              {c().words.all}
            </button>
            <For each={statusCounts()}>
              {([status, n]) => (
                <button class="chip st-filter-chip" classList={{ selected: statusFilter() === status }} onClick={() => setStatusFilter(statusFilter() === status ? "" : status)}>
                  {c().asset_status[status]} <span class="num">{n}</span>
                </button>
              )}
            </For>
          </div>
          <span class="spacer" />
          <label class="row xs muted" style={{ gap: "6px" }}>
            {t().preview.sort}
            <select class="select st-select" value={sort()} onChange={(e) => setSort(e.currentTarget.value as SortKey)}>
              <option value="target">{t().preview.sort_target}</option>
              <Show when={cmp()}>
                <option value="change">{t().preview.sort_change}</option>
              </Show>
              <option value="momentum">{t().preview.sort_momentum}</option>
              <option value="symbol">{t().preview.sort_symbol}</option>
            </select>
          </label>
        </div>
        <div class="table-wrap st-scroll-table">
          <table class="table compact st-preview-table">
            <thead>
              <tr>
                <th>{t().preview.h_company}</th>
                <th>{t().preview.h_sector}</th>
                <th>{t().preview.h_status}</th>
                <th class="r">{t().preview.h_vol}</th>
                <th class="r">{t().preview.h_momentum}</th>
                <th class="r">{t().preview.h_mom_mult}</th>
                <th class="r">{t().preview.h_trend_mult}</th>
                <th class="r">{t().preview.h_current}</th>
                <th>{t().preview.h_target}</th>
                <Show when={cmp()}>
                  <th class="r">{t().preview.h_vs_active}</th>
                </Show>
              </tr>
            </thead>
            <tbody>
              <For each={assetRows()} fallback={<tr><td colspan={10} class="muted" style={{ "text-align": "center", padding: "24px" }}>{t().preview.no_rows}</td></tr>}>
                {(a) => (
                  <tr classList={{ "st-row-muted": a.weight <= 0 && current(a.symbol) <= 0 }}>
                    <td>
                      <div class="name-cell">
                        <span class="ticker">{a.symbol}</span>
                        <span class="name">{companyName(a.symbol)}</span>
                      </div>
                    </td>
                    <td class="nowrap muted xs">{sectorName(a.sector)}</td>
                    <td>
                      <StatusChip status={a.status} />
                    </td>
                    <td class="r">{fmtPct(a.vol, { dp: 1 })}</td>
                    <td class="r">
                      <Pct value={a.momentum} dp={1} />
                    </td>
                    <td class="r">×{fmtNum(a.momentum_multiplier, 2)}</td>
                    <td class="r" classList={{ "st-penalised": a.trend_multiplier < 1 }}>
                      ×{fmtNum(a.trend_multiplier, 2)}
                    </td>
                    <td class="r muted">{fmtWeight(current(a.symbol), 2)}</td>
                    <td>
                      <div class="st-bar-cell">
                        <WeightBar weight={current(a.symbol)} target={a.weight} max={maxAssetWeight()} />
                        <b class="num">{fmtWeight(a.weight, 2)}</b>
                        <Show when={a.capped}>
                          <span class="chip outline st-mini-chip" title={t().preview.capped_hint}>
                            {t().preview.capped}
                          </span>
                        </Show>
                      </div>
                    </td>
                    <Show when={cmp()}>
                      <td class="r">
                        <Delta value={a.weight - (activeAssetWeights().get(a.symbol) ?? 0)} />
                      </td>
                    </Show>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>

      <Show when={tab() === "orders"}>
        <div class="stack" style={{ gap: "12px" }}>
          <div class="row wrap st-order-summary">
            <span>{tpl(t().preview.orders_summary, { n: r().orders.length, buy: fmtMoney(sum(buys()), { dp: 0 }), sell: fmtMoney(sum(sells()), { dp: 0 }) })}</span>
            <span class="muted">·</span>
            <span>
              {t().preview.turnover} {fmtPct(r().turnover, { dp: 2 })}
            </span>
            <span class="muted">·</span>
            <span>
              {t().preview.costs} {fmtMoney(r().est_costs)}
            </span>
            <span class="muted">·</span>
            <span>{tpl(t().preview.cash_after, { cash: fmtMoney(cashAfter(), { dp: 0 }) })}</span>
          </div>
          <Show
            when={r().orders.length}
            fallback={
              <div class="callout ok">
                <Icon name="check" size={16} />
                <span>{t().preview.no_orders}</span>
              </div>
            }
          >
            <div class="table-wrap st-scroll-table">
              <table class="table compact">
                <thead>
                  <tr>
                    <th>{t().preview.h_company}</th>
                    <th>{t().preview.h_side}</th>
                    <th>{t().preview.h_reason}</th>
                    <th class="r">{t().preview.h_qty}</th>
                    <th class="r">{t().preview.h_price}</th>
                    <th class="r">{t().preview.h_notional}</th>
                    <th class="r">{t().preview.h_weights}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={r().orders}>
                    {(o) => (
                      <tr>
                        <td>
                          <div class="name-cell">
                            <span class="ticker">{o.symbol}</span>
                            <span class="name">{companyName(o.symbol)}</span>
                          </div>
                        </td>
                        <td>
                          <SideChip side={o.side} />
                        </td>
                        <td class="nowrap">{c().reason[o.reason]}</td>
                        <td class="r">{fmtQty(o.qty)}</td>
                        <td class="r">{fmtPrice(o.price)}</td>
                        <td class="r">{fmtMoney(o.notional, { dp: 0 })}</td>
                        <td class="r nowrap">
                          <span class="muted">{fmtWeight(o.weight_before, 2)}</span> → <b>{fmtWeight(o.weight_target, 2)}</b> <span class="muted">→ {fmtWeight(o.weight_after, 2)}</span>
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </Show>
          <Show when={skippedGroups().length}>
            <div class="st-skipped">
              <div class="kicker">{t().preview.not_traded}</div>
              <For each={skippedGroups()}>
                {([reason, symbols]) => (
                  <div class="st-skipped-row">
                    <span class="st-skipped-label">
                      {c().skipped_trade[reason as keyof ReturnType<typeof c>["skipped_trade"]] ?? reason} <span class="num muted">{symbols.length}</span>
                    </span>
                    <span class="st-symbols">{symbols.join(" · ")}</span>
                  </div>
                )}
              </For>
            </div>
          </Show>
          <Show when={r().frozen.length || r().excluded.length}>
            <div class="st-skipped">
              <div class="kicker">{t().preview.flags}</div>
              <For each={[...r().frozen.map((f) => ({ ...f, kind: "frozen" as const })), ...r().excluded.map((f) => ({ ...f, kind: "excluded" as const }))]}>
                {(f) => (
                  <div class="st-skipped-row">
                    <span class="st-skipped-label">
                      <span class="ticker">{f.symbol}</span>
                    </span>
                    <span>
                      <StatusChip status={f.kind} /> {c().flag_reason[f.reason as keyof ReturnType<typeof c>["flag_reason"]] ?? f.reason}
                    </span>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
}

function signedInt(n: number): string {
  return n > 0 ? `+${n}` : n < 0 ? `−${Math.abs(n)}` : "0";
}
