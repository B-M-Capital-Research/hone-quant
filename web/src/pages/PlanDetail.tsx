import { A, useParams, useSearchParams } from "@solidjs/router";
import { For, Show, createMemo, createResource, onCleanup, onMount } from "solid-js";
import { Icon } from "@/components/Icon";
import { Empty, ErrorState, Kpi, Loading, OrderStatusChip, PlanStatusChip, SideChip, confirmAction, toast, toastError } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { plansText } from "@/i18n/plans";
import { ApiError, api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { fmtDual, fmtMoney, fmtNum, fmtPct, fmtPrice, fmtQty, moneyPolarity, toNumber } from "@/lib/format";
import { portfolios, selectPortfolio, selectedId } from "@/lib/portfolio";
import { serverNow } from "@/lib/session";
import type { Order, Plan, PlanDetail as Detail } from "@/lib/types";
import "@/styles/plans.css";
import { actorName, strategyName } from "@/lib/names";
import { sectorLabel, throttle } from "./overview/util";
import { ParamsView } from "./plans/ParamsView";

const TABS = ["orders", "fills", "targets", "sectors", "skipped", "params", "audit"] as const;
type Tab = (typeof TABS)[number];

export default function PlanDetail() {
  const t = plansText;
  const c = common;
  const route = useParams<{ id: string }>();
  const [params, setParams] = useSearchParams<{ tab?: string }>();
  const id = () => Number(route.id);
  const [detail, { refetch }] = createResource(id, (planId) => api.plan(planId));
  const tab = (): Tab => ((TABS as readonly string[]).includes(params.tab ?? "") ? (params.tab as Tab) : "orders");

  onMount(() => {
    const refresh = throttle(() => refetch(), 1500);
    const off = onServerEvent(["plan"], (event) => {
      if (event.type !== "plan" || event.plan_id === id()) refresh();
    });
    onCleanup(off);
  });

  const name = (symbol: string) => {
    const n = detail.latest?.names[symbol];
    return n ? (locale() === "zh" ? n.zh || n.en : n.en || n.zh) : "";
  };

  const approve = async (plan: Plan) => {
    const note = await confirmAction({
      title: plan.automation_mode === "approval" ? t().detail.approve : t().detail.execute_now,
      body: confirmBody(plan),
      confirmLabel: plan.automation_mode === "approval" ? t().detail.approve : t().detail.execute_now,
      askReason: true,
      reasonLabel: c().words.note,
    });
    if (note === null) return;
    try {
      const { report } = (await api.approvePlan(plan.id, note)) as { report: { filled: number; bought: number; sold: number; costs: number } };
      toast(
        c().plan_status.executed,
        tpl(t().detail.executed_report, {
          filled: report.filled,
          bought: fmtMoney(report.bought, { dp: 0 }),
          sold: fmtMoney(report.sold, { dp: 0 }),
          costs: fmtMoney(report.costs),
        }),
        "success",
      );
    } catch (error) {
      toastError(error);
    } finally {
      refetch();
    }
  };

  const cancel = async (plan: Plan) => {
    const reason = await confirmAction({
      title: t().detail.cancel,
      body: locale() === "zh" ? "取消后该计划的订单都不会执行，当前持仓保持不变。下一个时段会照常生成新计划。" : "None of this plan's orders will execute and holdings stay as they are. The next slot generates a new plan as usual.",
      confirmLabel: t().detail.cancel,
      danger: true,
      askReason: true,
      reasonLabel: c().words.reason,
    });
    if (reason === null) return;
    try {
      await api.cancelPlan(plan.id, reason);
      toast(c().plan_status.cancelled, undefined, "success");
    } catch (error) {
      toastError(error);
    } finally {
      refetch();
    }
  };

  const removeOrder = async (order: Order) => {
    const ok = await confirmAction({
      title: tpl(t().detail.remove_title, { symbol: order.symbol }),
      body: t().detail.remove_body,
      confirmLabel: t().detail.remove_order,
      danger: true,
    });
    if (ok === null) return;
    try {
      await api.skipOrder(order.plan_id, order.id);
      toast(t().detail.removed, undefined, "success");
    } catch (error) {
      toastError(error);
    } finally {
      refetch();
    }
  };

  return (
    <Show
      when={detail.latest}
      fallback={
        detail.error ? (
          detail.error instanceof ApiError && detail.error.status === 404 ? (
            <Empty title={t().detail.not_found} icon="plans">
              <A class="btn sm" href="/plans">
                {t().detail.back}
              </A>
            </Empty>
          ) : (
            <ErrorState error={detail.error} onRetry={refetch} />
          )
        ) : (
          <Loading />
        )
      }
    >
      {(d) => {
        const plan = () => d().plan;
        const pending = () => plan().status === "pending";
        /** Authorised by the plan's own portfolio, whichever one is selected. */
        const canAct = () => pending() && d().can_trade === true;
        /** Opened from a link about another portfolio than the one being worked in. */
        const elsewhere = () => (d().portfolio && d().portfolio.id !== selectedId() ? d().portfolio : null);
        const rebalance = () => d().diagnostics.rebalance;
        const flags = () => [...(d().diagnostics.frozen ?? []), ...(d().diagnostics.excluded ?? [])];
        return (
          <div class="stack">
            <div class="page-head plan-head">
              <div style={{ flex: 1, "min-width": "280px" }}>
                <A href="/plans" class="crumb">
                  <Icon name="chevron_left" size={14} /> {t().detail.back}
                </A>
                <div class="row" style={{ gap: "12px", "flex-wrap": "wrap" }}>
                  <h1>{tpl(t().detail.title, { date: plan().trade_date, slot: c().slot[plan().slot] })}</h1>
                  <PlanStatusChip status={plan().status} />
                  <span class="chip outline">#{plan().id}</span>
                </div>
                <p class="lead">
                  <Show when={d().portfolio}>
                    <span class="nowrap">
                      <Icon name="briefcase" size={13} style={{ "vertical-align": "-2px" }} /> {tpl(t().detail.portfolio, { name: d().portfolio.name })}
                    </span>
                    {" · "}
                  </Show>
                  {tpl(t().detail.generated, { time: fmtDual(plan().generated_at, "auto") })} · {tpl(t().detail.by, { who: actorName(plan().created_by) })}
                  <Show when={d().strategy}>
                    {" · "}
                    {t().detail.strategy}{" "}
                    <A href={`/strategy?version=${d().strategy!.id}`}>
                      {strategyName(d().strategy)} #{d().strategy!.id}
                    </A>
                  </Show>
                  {" · "}
                  {tpl(t().detail.mode, { mode: c().mode[plan().automation_mode] })}
                </p>
              </div>
              <Show when={canAct()}>
                <div class="row">
                  <button class="btn danger" onClick={() => cancel(plan())}>
                    <Icon name="ban" size={15} /> {t().detail.cancel}
                  </button>
                  <button class="btn primary" onClick={() => approve(plan())}>
                    <Icon name="check" size={15} /> {plan().automation_mode === "approval" ? t().detail.approve : t().detail.execute_now}
                  </button>
                </div>
              </Show>
            </div>

            <Show when={elsewhere()}>
              {(other) => (
                <div class="callout info plan-portfolio-banner">
                  <Icon name="briefcase" size={16} />
                  <span class="text">{tpl(t().detail.other_portfolio, { name: other().name })}</span>
                  <Show when={portfolios().some((p) => p.id === other().id)}>
                    <button class="btn sm" onClick={() => selectPortfolio(other().id)} title={tpl(t().detail.switch_to, { name: other().name })}>
                      <span class="truncate">{tpl(t().detail.switch_to, { name: other().name })}</span>
                    </button>
                  </Show>
                </div>
              )}
            </Show>

            <Show when={plan().error}>
              <div class="callout critical">
                <Icon name="alert" size={16} />
                <div>
                  <strong>{t().detail.error}</strong>
                  <div class="mono xs" style={{ "white-space": "pre-wrap", "margin-top": "4px" }}>
                    {plan().error}
                  </div>
                </div>
              </div>
            </Show>
            <Show when={rebalance()?.turnover_scale != null && rebalance()!.turnover_scale! < 0.999}>
              <div class="callout info">
                <Icon name="info" size={16} />
                <span>{tpl(t().detail.turnover_capped, { pct: fmtPct(rebalance()!.turnover_scale, { dp: 0 }) })}</span>
              </div>
            </Show>
            <Show when={rebalance()?.cash_scale != null && rebalance()!.cash_scale! < 0.999}>
              <div class="callout warn">
                <Icon name="alert" size={16} />
                <span>{tpl(t().detail.cash_capped, { pct: fmtPct(rebalance()!.cash_scale, { dp: 0 }) })}</span>
              </div>
            </Show>
            <Show when={flags().length}>
              <div class="callout warn">
                <Icon name="lock" size={16} />
                <div>
                  {t().detail.flags}{" "}
                  <For each={flags()}>
                    {(f, i) => (
                      <>
                        {i() > 0 ? "，" : ""}
                        <b>{f.symbol}</b>（{(c().flag_reason as Record<string, string>)[f.reason] ?? f.reason}）
                      </>
                    )}
                  </For>
                </div>
              </div>
            </Show>

            <Lifecycle plan={plan()} />

            <div class="kpis plan-kpis">
              <Kpi
                label={t().detail.kpi_nav}
                value={<span class="num">{fmtMoney(plan().nav, { dp: 0 })}</span>}
                delta={
                  <span class="muted num">
                    {c().words.cash} {fmtMoney(plan().cash, { compact: true })}
                  </span>
                }
              />
              <Kpi label={t().detail.kpi_exposure} value={<span class="num">{fmtPct(plan().invested_target, { dp: 1 })}</span>} />
              <Kpi label={t().detail.kpi_breadth} value={<span class="num">{fmtPct(plan().breadth, { dp: 0 })}</span>} />
              <Kpi label={t().detail.kpi_vol} value={<span class="num">{fmtPct(plan().est_vol, { dp: 1 })}</span>} />
              <Kpi label={t().detail.kpi_turnover} value={<span class="num">{fmtPct(plan().turnover, { dp: 1 })}</span>} />
              <Show when={plan().summary?.executed}>
                {(e) => (
                  <Kpi
                    label={t().detail.kpi_result}
                    value={
                      <span class="num">
                        {e().filled + e().partial}/{e().filled + e().partial + e().rejected}
                      </span>
                    }
                    delta={
                      <span class="muted num">
                        {[
                          e().bought > 0 ? `${c().side.buy} ${fmtMoney(e().bought, { compact: true })}` : "",
                          e().sold > 0 ? `${c().side.sell} ${fmtMoney(e().sold, { compact: true })}` : "",
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      </span>
                    }
                  />
                )}
              </Show>
              <Kpi
                label={t().detail.kpi_costs}
                value={<span class="num">{fmtMoney(plan().est_costs)}</span>}
                delta={
                  <Show when={rebalance()}>
                    <span class="muted" title={tpl(t().detail.costs_breakdown, { commission: fmtMoney(rebalance()!.est_commission), slippage: fmtMoney(rebalance()!.est_slippage), fees: fmtMoney(rebalance()!.est_fees) })}>
                      {costsShort(rebalance()!)}
                    </span>
                  </Show>
                }
              />
            </div>

            <div class="card" style={{ "min-width": 0 }}>
                <div class="tabs plan-tabs" role="tablist">
                  <For each={TABS}>
                    {(key) => (
                      <button role="tab" aria-selected={tab() === key} onClick={() => setParams({ tab: key === "orders" ? undefined : key }, { replace: true })}>
                        {t().detail[`tab_${key}` as const]}
                        <Show when={key === "orders"}>
                          <span class="tab-count">{d().orders.length}</span>
                        </Show>
                        <Show when={key === "fills" && d().fills.length}>
                          <span class="tab-count">{d().fills.length}</span>
                        </Show>
                      </button>
                    )}
                  </For>
                </div>
                <Show when={tab() === "orders"}>
                  <OrdersTable detail={d()} name={name} editable={canAct()} onRemove={removeOrder} />
                </Show>
                <Show when={tab() === "fills"}>
                  <FillsTable detail={d()} name={name} />
                </Show>
                <Show when={tab() === "targets"}>
                  <TargetsTable detail={d()} name={name} />
                </Show>
                <Show when={tab() === "sectors"}>
                  <SectorsTable detail={d()} />
                </Show>
                <Show when={tab() === "skipped"}>
                  <SkippedTable detail={d()} name={name} />
                </Show>
                <Show when={tab() === "params"}>
                  <div class="card-body">
                    <Show when={d().diagnostics.params} fallback={<Empty title={t().detail.no_diagnostics} />}>
                      <Show when={d().diagnostics.history}>
                        <p class="muted xs" style={{ "margin-bottom": "12px" }}>
                          {tpl(t().detail.params_from, { from: d().diagnostics.history!.from ?? "—", to: d().diagnostics.history!.to ?? "—" })}
                        </p>
                      </Show>
                      <ParamsView params={d().diagnostics.params!} />
                    </Show>
                  </div>
                </Show>
                <Show when={tab() === "audit"}>
                  <AuditList detail={d()} />
                </Show>
            </div>
          </div>
        );
      }}
    </Show>
  );

  function costsShort(r: NonNullable<Detail["diagnostics"]["rebalance"]>): string {
    const zh = locale() === "zh";
    const parts = [`${zh ? "佣金" : "commission"} ${fmtMoney(r.est_commission, { dp: 0 })}`, `${zh ? "滑点" : "slippage"} ${fmtMoney(r.est_slippage, { dp: 0 })}`];
    if (r.est_fees >= 0.5) parts.push(`${zh ? "规费" : "fees"} ${fmtMoney(r.est_fees, { dp: 0 })}`);
    return parts.join(" · ");
  }

  function confirmBody(plan: Plan): string {
    return locale() === "zh"
      ? `将按最新报价立即在模拟盘中执行 ${plan.order_count} 笔订单。每笔订单会再次检查报价新鲜度与价格偏离，超出限制的订单不会成交。`
      : `${plan.order_count} orders will be executed on the paper account at the latest quotes. Each order re-checks quote freshness and price deviation; orders outside the limits are not filled.`;
  }
}

// ---------------------------------------------------------------------------------------------

function Lifecycle(props: { plan: Plan }) {
  const t = plansText;
  const p = () => props.plan;
  const now = () => serverNow();
  type Step = { label: string; detail?: string; state: "done" | "active" | "warn" | "off" | "" };
  const steps = createMemo<Step[]>(() => {
    const d = t().detail;
    const out: Step[] = [{ label: d.step_generated, detail: `${fmtDual(p().generated_at, "auto")} · ${actorName(p().created_by)}`, state: "done" }];
    const status = p().status;
    if (status === "skipped") return [{ label: d.step_skipped, detail: p().error ?? undefined, state: "off" }];
    if (status === "no_action") return [...out, { label: d.step_no_action, state: "done" }];
    if (p().automation_mode === "approval") {
      out.push({
        label: d.step_approval,
        detail: tpl(d.step_approval_until, { time: fmtDual(p().deadline) }),
        state: status === "pending" ? "active" : "done",
      });
    } else if (p().execute_after) {
      const reviewing = status === "pending" && new Date(p().execute_after!).getTime() > now();
      out.push({
        label: d.step_review,
        detail: tpl(d.step_review_until, { time: fmtDual(p().execute_after) }),
        state: reviewing ? "active" : "done",
      });
    }
    if (p().approved_at) out.push({ label: tpl(d.step_approved, { who: actorName(p().approved_by) }), detail: fmtDual(p().approved_at, "auto"), state: "done" });
    switch (status) {
      case "pending":
        if (p().automation_mode !== "approval" && p().execute_after && new Date(p().execute_after!).getTime() <= now()) {
          out.push({ label: d.step_pending_exec, state: "active" });
        }
        break;
      case "executing":
        out.push({ label: common().plan_status.executing, state: "active" });
        break;
      case "executed":
      case "partially_executed":
        out.push({ label: d.step_executed, detail: fmtDual(p().executed_at, "auto"), state: status === "executed" ? "done" : "warn" });
        break;
      case "cancelled":
        out.push({
          label: tpl(d.step_cancelled, { who: actorName(p().cancelled_by) }),
          detail: `${fmtDual(p().cancelled_at, "auto")}${p().cancel_reason ? ` · ${p().cancel_reason}` : ""}`,
          state: "off",
        });
        break;
      case "expired":
        out.push({ label: d.step_expired, detail: fmtDual(p().deadline, "auto"), state: "warn" });
        break;
      case "failed":
        out.push({ label: d.step_failed, detail: p().error ?? undefined, state: "warn" });
        break;
    }
    return out;
  });
  return (
    <div class="card stepper-card">
      <div class="stepper" aria-label={t().detail.lifecycle}>
        <For each={steps()}>
          {(step) => (
            <div class={`step ${step.state}`}>
              <div class={`tl-dot ${step.state}`}>
                <Show when={step.state === "done"}>
                  <Icon name="check" size={12} />
                </Show>
              </div>
              <div class="step-text">
                <div class="step-label">{step.label}</div>
                <Show when={step.detail}>
                  <div class="muted xs step-detail">{step.detail}</div>
                </Show>
              </div>
            </div>
          )}
        </For>
        <div class="step-deadline muted xs">
          {t().detail.deadline} · {fmtDual(p().deadline, "auto")}
        </div>
      </div>
    </div>
  );
}

function SymbolCell(props: { symbol: string; name: string }) {
  return (
    <div class="name-cell">
      <A class="ticker" href={`/universe?symbol=${encodeURIComponent(props.symbol)}`} onClick={(e) => e.stopPropagation()}>
        {props.symbol}
      </A>
      <span class="name">{props.name}</span>
    </div>
  );
}

function OrdersTable(props: { detail: Detail; name: (s: string) => string; editable: boolean; onRemove: (o: Order) => void }) {
  const t = plansText;
  const c = common;
  const totals = createMemo(() => {
    let buys = 0;
    let sells = 0;
    for (const o of props.detail.orders) {
      if (o.status === "skipped" || o.status === "cancelled") continue;
      const v = (toNumber(o.qty) ?? 0) * o.ref_price;
      if (o.side === "buy") buys += v;
      else sells += v;
    }
    return { buys, sells };
  });
  return (
    <Show when={props.detail.orders.length} fallback={<Empty title={t().detail.no_orders} icon="plans" />}>
      <div class="table-wrap">
        <table class="table compact">
          <thead>
            <tr>
              <th class="r">{t().detail.col_seq}</th>
              <th>{t().detail.col_symbol}</th>
              <th>{t().detail.col_side}</th>
              <th>{t().detail.col_reason}</th>
              <th class="r">{t().detail.col_qty}</th>
              <th class="r">{t().detail.col_ref}</th>
              <th class="r">{t().detail.col_notional}</th>
              <th class="r">{t().detail.col_weights}</th>
              <th>{t().detail.col_status}</th>
              <Show when={props.editable}>
                <th />
              </Show>
            </tr>
          </thead>
          <tbody>
            <For each={props.detail.orders}>
              {(o) => (
                <tr classList={{ dimmed: o.status === "skipped" || o.status === "cancelled" }}>
                  <td class="r num muted">{o.sequence + 1}</td>
                  <td>
                    <SymbolCell symbol={o.symbol} name={props.name(o.symbol)} />
                  </td>
                  <td>
                    <SideChip side={o.side} />
                  </td>
                  <td class="xs nowrap">{c().reason[o.reason]}</td>
                  <td class="r num">
                    {fmtQty(o.qty)}
                    <Show when={o.status === "partially_filled"}>
                      <div class="xs muted">
                        {t().detail.col_filled} {fmtQty(o.filled_qty)}
                      </div>
                    </Show>
                  </td>
                  <td class="r num">{fmtPrice(o.ref_price)}</td>
                  <td class="r num">{fmtMoney((toNumber(o.qty) ?? 0) * o.ref_price, { dp: 0 })}</td>
                  <td class="r num nowrap">
                    <span class="muted">{fmtPct(o.weight_before, { dp: 2 })}</span> → <b>{fmtPct(o.weight_target, { dp: 2 })}</b> <span class="muted">→ {fmtPct(o.weight_after, { dp: 2 })}</span>
                  </td>
                  <td>
                    <OrderStatusChip status={o.status} reason={o.status_reason} />
                  </td>
                  <Show when={props.editable}>
                    <td class="r">
                      <Show when={o.status === "planned"}>
                        <button class="btn sm ghost" onClick={() => props.onRemove(o)} title={t().detail.remove_order}>
                          <Icon name="x" size={14} /> {t().detail.remove_order}
                        </button>
                      </Show>
                    </td>
                  </Show>
                </tr>
              )}
            </For>
          </tbody>
          <tfoot>
            <tr>
              <td />
              <td colSpan={5}>{t().detail.totals}</td>
              <td class="r num nowrap" colSpan={2}>
                {tpl(t().detail.buys, { value: fmtMoney(totals().buys, { dp: 0 }) })} · {tpl(t().detail.sells, { value: fmtMoney(totals().sells, { dp: 0 }) })}
              </td>
              <td colSpan={props.editable ? 2 : 1} />
            </tr>
          </tfoot>
        </table>
      </div>
    </Show>
  );
}

function FillsTable(props: { detail: Detail; name: (s: string) => string }) {
  const t = plansText;
  return (
    <Show when={props.detail.fills.length} fallback={<Empty title={t().detail.no_fills} icon="trades" />}>
      <div class="table-wrap">
        <table class="table compact">
          <thead>
            <tr>
              <th>{t().detail.col_time}</th>
              <th>{t().detail.col_symbol}</th>
              <th>{t().detail.col_side}</th>
              <th class="r">{t().detail.col_qty}</th>
              <th class="r">{t().detail.col_price}</th>
              <th class="r">{t().detail.col_quote}</th>
              <th class="r">{t().detail.col_notional}</th>
              <th class="r">{t().detail.col_slippage}</th>
              <th class="r">{t().detail.col_commission}</th>
              <th class="r">{t().detail.col_fees}</th>
              <th class="r">{t().detail.col_realized}</th>
            </tr>
          </thead>
          <tbody>
            <For each={props.detail.fills}>
              {(f) => (
                <tr>
                  <td class="xs num nowrap">{fmtDual(f.executed_at, "auto")}</td>
                  <td>
                    <SymbolCell symbol={f.symbol} name={props.name(f.symbol)} />
                  </td>
                  <td>
                    <SideChip side={f.side} />
                  </td>
                  <td class="r num">{fmtQty(f.qty)}</td>
                  <td class="r num">{fmtPrice(f.price)}</td>
                  <td class="r num muted">{fmtPrice(f.quote_price)}</td>
                  <td class="r num">{fmtMoney(f.notional)}</td>
                  <td class="r num">{fmtMoney(f.slippage)}</td>
                  <td class="r num">{fmtMoney(f.commission)}</td>
                  <td class="r num">{fmtMoney(f.fees)}</td>
                  <td class={`r num ${moneyPolarity(f.realized_pnl)}`}>{f.realized_pnl == null ? "—" : fmtMoney(f.realized_pnl, { sign: true })}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}

function TargetsTable(props: { detail: Detail; name: (s: string) => string }) {
  const t = plansText;
  const c = common;
  const rows = createMemo(() => {
    const targets = props.detail.diagnostics.targets;
    if (!targets) return [];
    const before = new Map<string, number>();
    for (const s of props.detail.diagnostics.skipped ?? []) before.set(s.symbol, s.weight_before);
    for (const o of props.detail.orders) before.set(o.symbol, o.weight_before);
    return targets.assets
      .map((a, i) => ({ ...a, weight: targets.weights[i] ?? a.weight, before: before.get(a.symbol) ?? null }))
      .sort((a, b) => b.weight - a.weight || a.symbol.localeCompare(b.symbol));
  });
  const tone = (status: string) => (status === "active" ? "green" : status === "below_min_weight" ? "" : status === "frozen" ? "blue" : "yellow");
  return (
    <Show when={rows().length} fallback={<Empty title={t().detail.no_diagnostics} />}>
      <p class="muted xs tab-note">{t().detail.targets_note}</p>
      <div class="table-wrap">
        <table class="table compact">
          <thead>
            <tr>
              <th>{t().detail.col_symbol}</th>
              <th>{t().detail.col_asset_status}</th>
              <th class="r">{t().detail.col_vol}</th>
              <th class="r">{t().detail.col_momentum}</th>
              <th class="r">{t().detail.col_rank}</th>
              <th class="r">{t().detail.col_trend}</th>
              <th class="r">{t().detail.col_mult}</th>
              <th class="r">{t().detail.col_before}</th>
              <th class="r">{t().detail.col_target}</th>
            </tr>
          </thead>
          <tbody>
            <For each={rows()}>
              {(a) => (
                <tr classList={{ dimmed: a.weight === 0 && !a.before }}>
                  <td>
                    <SymbolCell symbol={a.symbol} name={props.name(a.symbol)} />
                  </td>
                  <td>
                    <span class={`chip ${tone(a.status)}`}>{(c().asset_status as Record<string, string>)[a.status] ?? a.status}</span>
                  </td>
                  <td class="r num">{fmtPct(a.vol, { dp: 0 })}</td>
                  <td class={`r num ${a.momentum == null ? "" : a.momentum >= 0 ? "up" : "down"}`}>{fmtPct(a.momentum, { dp: 1, sign: true })}</td>
                  <td class="r num">{a.momentum_rank == null ? "—" : fmtNum(a.momentum_rank * 100, 0)}</td>
                  <td class="r num">{fmtPct(a.trend, { dp: 1, sign: true })}</td>
                  <td class="r num">
                    {fmtNum(a.momentum_multiplier, 2)} × {fmtNum(a.trend_multiplier, 2)}
                  </td>
                  <td class="r num muted">{a.before == null ? "—" : fmtPct(a.before, { dp: 2 })}</td>
                  <td class="r num">
                    <b>{fmtPct(a.weight, { dp: 2 })}</b>
                    <Show when={a.capped}>
                      <span class="chip outline" style={{ "margin-left": "6px" }}>
                        {t().detail.capped}
                      </span>
                    </Show>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}

function SectorsTable(props: { detail: Detail }) {
  const t = plansText;
  const sectors = () => props.detail.diagnostics.targets?.sectors ?? [];
  return (
    <Show when={sectors().length} fallback={<Empty title={t().detail.no_diagnostics} />}>
      <p class="muted xs tab-note">{t().detail.sectors_note}</p>
      <div class="table-wrap">
        <table class="table compact">
          <thead>
            <tr>
              <th>{t().detail.col_sector}</th>
              <th class="r">{t().detail.col_members}</th>
              <th class="r">{t().detail.col_vol}</th>
              <th class="r">{t().detail.col_momentum}</th>
              <th class="r">{t().detail.col_rank}</th>
              <th class="r">{t().detail.col_trend_factor}</th>
              <th class="r">{t().detail.col_cap}</th>
              <th class="r">{t().detail.col_budget}</th>
            </tr>
          </thead>
          <tbody>
            <For each={[...sectors()].sort((a, b) => b.budget - a.budget)}>
              {(s) => (
                <tr>
                  <td>{sectorLabel(undefined, s.sector)}</td>
                  <td class="r num">{s.active_members}</td>
                  <td class="r num">{fmtPct(s.vol, { dp: 0 })}</td>
                  <td class={`r num ${s.momentum == null ? "" : s.momentum >= 0 ? "up" : "down"}`}>{fmtPct(s.momentum, { dp: 1, sign: true })}</td>
                  <td class="r num">{s.momentum_rank == null ? "—" : fmtNum(s.momentum_rank * 100, 0)}</td>
                  <td class="r num">{fmtNum(s.trend_factor, 2)}</td>
                  <td class="r num muted">{fmtPct(s.cap, { dp: 1 })}</td>
                  <td class="r num">
                    <b>{fmtPct(s.budget, { dp: 1 })}</b>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}

function SkippedTable(props: { detail: Detail; name: (s: string) => string }) {
  const t = plansText;
  const c = common;
  const skipped = () => props.detail.diagnostics.skipped ?? [];
  const flags = () => [...(props.detail.diagnostics.frozen ?? []), ...(props.detail.diagnostics.excluded ?? [])];
  return (
    <div>
      <Show when={flags().length}>
        <div class="card-body" style={{ "padding-bottom": "4px" }}>
          <div class="kicker" style={{ "margin-bottom": "8px" }}>
            {t().detail.flagged}
          </div>
          <div class="pill-list">
            <For each={flags()}>
              {(f) => (
                <span class="chip yellow">
                  {f.symbol} · {(c().flag_reason as Record<string, string>)[f.reason] ?? f.reason}
                </span>
              )}
            </For>
          </div>
        </div>
      </Show>
      <Show when={skipped().length} fallback={<Empty title={t().detail.no_skipped} />}>
        <div class="table-wrap">
          <table class="table compact">
            <thead>
              <tr>
                <th>{t().detail.col_symbol}</th>
                <th>{t().detail.col_reason_skip}</th>
                <th class="r">{t().detail.col_before}</th>
                <th class="r">{t().detail.col_target}</th>
              </tr>
            </thead>
            <tbody>
              <For each={[...skipped()].sort((a, b) => Math.abs(b.weight_target - b.weight_before) - Math.abs(a.weight_target - a.weight_before))}>
                {(s) => (
                  <tr>
                    <td>
                      <SymbolCell symbol={s.symbol} name={props.name(s.symbol)} />
                    </td>
                    <td class="xs">{c().skipped_trade[s.reason] ?? s.reason}</td>
                    <td class="r num">{fmtPct(s.weight_before, { dp: 2 })}</td>
                    <td class="r num">{fmtPct(s.weight_target, { dp: 2 })}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </div>
  );
}

function AuditList(props: { detail: Detail }) {
  return (
    <Show when={props.detail.audit.length} fallback={<Empty />}>
      <div class="table-wrap">
        <table class="table compact">
          <thead>
            <tr>
              <th>{common().words.time}</th>
              <th>{common().words.actor}</th>
              <th>{locale() === "zh" ? "操作" : "Action"}</th>
              <th>{locale() === "zh" ? "详情" : "Detail"}</th>
            </tr>
          </thead>
          <tbody>
            <For each={props.detail.audit}>
              {(a) => (
                <tr>
                  <td class="xs num nowrap">{fmtDual(a.ts, "auto")}</td>
                  <td class="xs">{actorName(a.actor)}</td>
                  <td class="mono xs">{a.action}</td>
                  <td class="mono xs audit-detail">{JSON.stringify(a.detail)}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}
