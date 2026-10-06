import { A } from "@solidjs/router";
import { For, Show, createMemo } from "solid-js";
import { Icon } from "@/components/Icon";
import { PlanStatusChip } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { overviewText } from "@/i18n/overview";
import { fmtDual, fmtPct } from "@/lib/format";
import { strategyName } from "@/lib/names";
import type { Board, Dashboard, Plan, Restriction, Valuation } from "@/lib/types";
import { sectorLabel } from "./util";

export function SectorAllocation(props: { valuation: Valuation; targets: Record<string, number> | null; board: Board | undefined }) {
  const t = overviewText;
  // Long English sector names do not fit the narrow card; use the short form (full name in the tooltip).
  const label = (id: string) => {
    const full = sectorLabel(props.board?.sectors, id);
    return locale() === "en" && full.length > 13 ? sectorLabel(props.board?.sectors, id, true) : full;
  };
  const rows = createMemo(() => {
    const sectorOf = new Map<string, string>();
    for (const item of props.board?.items ?? []) sectorOf.set(item.symbol, item.sector_id);
    for (const p of props.valuation.positions) sectorOf.set(p.symbol, p.sector_id);
    const current = new Map<string, number>();
    const target = new Map<string, number>();
    for (const p of props.valuation.positions) current.set(p.sector_id, (current.get(p.sector_id) ?? 0) + p.weight);
    for (const [symbol, w] of Object.entries(props.targets ?? {})) {
      const sector = sectorOf.get(symbol);
      if (sector) target.set(sector, (target.get(sector) ?? 0) + w);
    }
    const ids = (props.board?.sectors ?? []).map((s) => s.id);
    for (const id of [...current.keys(), ...target.keys()]) if (!ids.includes(id)) ids.push(id);
    return ids
      .map((id) => ({ id, current: current.get(id) ?? 0, target: props.targets ? target.get(id) ?? 0 : null }))
      .filter((r) => r.current > 0 || (r.target ?? 0) > 0)
      .sort((a, b) => (b.target ?? b.current) - (a.target ?? a.current));
  });
  const cash = () => ({
    current: props.valuation.nav > 0 ? props.valuation.cash / props.valuation.nav : 0,
    target: props.targets ? Math.max(0, 1 - Object.values(props.targets).reduce((a, b) => a + b, 0)) : null,
  });
  const max = createMemo(() => Math.max(0.05, cash().current, cash().target ?? 0, ...rows().flatMap((r) => [r.current, r.target ?? 0])));
  const Bar = (p: { current: number; target: number | null; muted?: boolean }) => (
    <div class="alloc-bar" classList={{ muted: p.muted }}>
      <div class="fill" style={{ width: `${(p.current / max()) * 100}%` }} />
      <Show when={p.target != null}>
        <div class="target" style={{ left: `calc(${((p.target ?? 0) / max()) * 100}% - 1px)` }} />
      </Show>
    </div>
  );
  return (
    <div class="card">
      <div class="card-head">
        <div>
          <h2>{t().sectors.title}</h2>
          <div class="sub">{t().sectors.sub}</div>
        </div>
      </div>
      <div class="card-body alloc">
        <For each={rows()}>
          {(r) => (
            <div class="alloc-row" title={`${sectorLabel(props.board?.sectors, r.id)}: ${fmtPct(r.current, { dp: 2 })} / ${r.target == null ? "—" : fmtPct(r.target, { dp: 2 })}`}>
              <span class="alloc-name truncate">{label(r.id)}</span>
              <Bar current={r.current} target={r.target} />
              <span class="alloc-num num">
                {fmtPct(r.current, { dp: 1 })}
                <span class="muted"> / {r.target == null ? "—" : fmtPct(r.target, { dp: 1 })}</span>
              </span>
            </div>
          )}
        </For>
        <div class="alloc-row cash">
          <span class="alloc-name">{t().sectors.cash}</span>
          <Bar current={cash().current} target={cash().target} muted />
          <span class="alloc-num num">
            {fmtPct(cash().current, { dp: 1 })}
            <span class="muted"> / {cash().target == null ? "—" : fmtPct(cash().target, { dp: 1 })}</span>
          </span>
        </div>
        <div class="alloc-legend xs muted">
          <span>
            <i class="lg-bar" /> {common().words.actual}
          </span>
          <span>
            <i class="lg-target" /> {common().words.target}
          </span>
        </div>
      </div>
    </div>
  );
}

export function StrategyCard(props: { dashboard: Dashboard }) {
  const t = overviewText;
  const strategy = () => props.dashboard.strategy;
  const latest = createMemo<Plan | undefined>(() => {
    const id = props.dashboard.targets?.plan_id;
    return id ? [...props.dashboard.plans, ...props.dashboard.recent_plans].find((p) => p.id === id) : undefined;
  });
  return (
    <div class="card">
      <div class="card-head">
        <div style={{ "min-width": 0 }}>
          <h2>{t().strategy.title}</h2>
          <Show when={strategy()} fallback={<div class="sub">{t().strategy.none}</div>}>
            <div class="sub truncate">
              {strategyName(strategy())} · #{strategy()!.id}
            </div>
          </Show>
        </div>
        <div class="spacer" />
        <A class="btn sm ghost" href="/strategy">
          {t().strategy.edit}
          <Icon name="chevron_right" size={14} />
        </A>
      </div>
      <Show when={strategy()}>
        <div class="card-body stack" style={{ gap: "12px" }}>
          <Show when={latest()}>
            <dl class="kv">
              <dt>{t().strategy.exposure}</dt>
              <dd>{fmtPct(latest()!.invested_target, { dp: 1 })}</dd>
              <dt title={t().strategy.breadth_hint}>{t().strategy.breadth}</dt>
              <dd>{fmtPct(latest()!.breadth, { dp: 0 })}</dd>
              <dt>{t().strategy.est_vol}</dt>
              <dd>{fmtPct(latest()!.est_vol, { dp: 1 })}</dd>
            </dl>
            <p class="muted xs">{tpl(t().strategy.from_plan, { time: fmtDual(latest()!.generated_at, true) })}</p>
          </Show>
          <div class="pill-list">
            <span class="param">
              {t().strategy.name_cap} <b>{fmtPct(strategy()!.params.asset.max_weight, { dp: 0 })}</b>
            </span>
            <span class="param">
              {t().strategy.sector_cap} <b>{fmtPct(strategy()!.params.sector.max_weight, { dp: 0 })}</b>
            </span>
            <span class="param">
              {t().strategy.max_turnover} <b>{fmtPct(strategy()!.params.rebalance.max_turnover, { dp: 0 })}</b>
            </span>
            <span class="param">
              {t().strategy.band} <b>±{fmtPct(strategy()!.params.rebalance.band_abs, { dp: 1 })}</b>
            </span>
          </div>
        </div>
      </Show>
    </div>
  );
}

export function RecentPlans(props: { plans: Plan[] }) {
  const t = overviewText;
  const c = common;
  return (
    <div class="card">
      <div class="card-head">
        <h2>{t().plans.recent}</h2>
        <div class="spacer" />
        <A class="btn sm ghost" href="/plans">
          {t().plans.all_plans}
          <Icon name="chevron_right" size={14} />
        </A>
      </div>
      <div class="card-body flush">
        <Show when={props.plans.length} fallback={<div class="empty">{t().plans.none_recent}</div>}>
          <ul class="recent-plans">
            <For each={props.plans.slice(0, 6)}>
              {(p) => (
                <li>
                  <A href={`/plans/${p.id}`}>
                    <span class="num">{p.trade_date.slice(5)}</span>
                    <span class="subtle">{c().slot[`${p.slot}_short` as "open_short" | "close_short" | "manual_short"]}</span>
                    <span class="muted xs num">{p.order_count ? tpl(t().plans.orders, { n: p.order_count }) : ""}</span>
                    <span class="spacer" />
                    <PlanStatusChip status={p.status} />
                  </A>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </div>
    </div>
  );
}

export function Restrictions(props: { restrictions: Restriction[] }) {
  const t = overviewText;
  return (
    <Show when={props.restrictions.length}>
      <div class="card">
        <div class="card-head">
          <h2>{t().restrictions.title}</h2>
          <div class="spacer" />
          <A class="btn sm ghost" href="/universe">
            {t().restrictions.manage}
            <Icon name="chevron_right" size={14} />
          </A>
        </div>
        <div class="card-body pill-list">
          <For each={props.restrictions}>
            {(r) => (
              <span class={`chip ${r.mode === "lock" ? "blue" : "yellow"}`} title={r.reason}>
                <Icon name={r.mode === "lock" ? "lock" : "ban"} size={11} />
                {r.symbol} · {r.mode === "lock" ? t().restrictions.lock : t().restrictions.exclude}
              </span>
            )}
          </For>
        </div>
      </div>
    </Show>
  );
}
