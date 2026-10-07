import { A } from "@solidjs/router";
import { For, Show, createMemo } from "solid-js";
import { Icon } from "@/components/Icon";
import { PlanStatusChip, SideChip, confirmAction, toast, toastError } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { overviewText } from "@/i18n/overview";
import { api } from "@/lib/api";
import { fmtCountdown, fmtDual, fmtMoney, fmtPct, fmtQty, toNumber } from "@/lib/format";
import { canTrade } from "@/lib/portfolio";
import { serverNow } from "@/lib/session";
import type { MarketView, Plan, PlanWithOrders, SlotView } from "@/lib/types";
import { weekday } from "./util";

type SlotState =
  | { kind: "plan"; plan: Plan }
  | { kind: "cancelled_ahead"; who: string; reason: string }
  | { kind: "skipped"; reason: string }
  | { kind: "scheduled" }
  | { kind: "due" }
  | { kind: "missed" };

function slotState(slot: SlotView, now: number): SlotState {
  if (slot.plan) return { kind: "plan", plan: slot.plan };
  if (slot.cancelled) {
    return slot.cancelled.reason === "operator"
      ? { kind: "cancelled_ahead", who: slot.cancelled.created_by, reason: "" }
      : { kind: "skipped", reason: slot.cancelled.reason };
  }
  if (now < new Date(slot.generate_at).getTime()) return { kind: "scheduled" };
  if (now < new Date(slot.window_end).getTime()) return { kind: "due" };
  return { kind: "missed" };
}

function dotClass(state: SlotState): string {
  if (state.kind === "plan") {
    switch (state.plan.status) {
      case "executed":
      case "no_action":
        return "done";
      case "pending":
      case "executing":
        return "active";
      case "partially_executed":
      case "expired":
      case "failed":
        return "warn";
      default:
        return "off";
    }
  }
  if (state.kind === "scheduled" || state.kind === "due") return "";
  return "off";
}

export function TodayPlans(props: { market: MarketView; plans: PlanWithOrders[]; onChanged: () => void }) {
  const t = overviewText;
  const c = common;
  const now = () => serverNow();
  const m = () => props.market;
  const isToday = () => m().today?.date === m().schedule_date;
  const manual = createMemo(() => props.plans.filter((p) => p.slot === "manual"));
  const ordersOf = (id: number) => props.plans.find((p) => p.id === id)?.orders ?? [];
  const slotName = (slot: string) => c().slot[slot as "open" | "close" | "manual"] ?? slot;

  const canCancelDay = createMemo(() => {
    const n = now();
    return (
      m().schedule.some((s) => {
        const st = slotState(s, n);
        return (st.kind === "plan" && st.plan.status === "pending") || st.kind === "scheduled" || st.kind === "due";
      }) || manual().some((p) => p.status === "pending")
    );
  });

  const run = async (action: () => Promise<unknown>, done: string) => {
    try {
      await action();
      toast(done, undefined, "success");
    } catch (error) {
      toastError(error);
    } finally {
      props.onChanged();
    }
  };

  const approve = async (plan: Plan) => {
    const note = await confirmAction({
      title: tpl(t().plans.confirm_approve_title, { slot: slotName(plan.slot) }),
      body: tpl(t().plans.confirm_approve, { n: plan.order_count }),
      confirmLabel: plan.automation_mode === "approval" ? t().plans.approve : t().plans.execute_now,
      askReason: true,
      reasonLabel: c().words.note,
    });
    if (note === null) return;
    try {
      const { report } = (await api.approvePlan(plan.id, note)) as { report: { filled?: number } };
      toast(tpl(t().plans.done_execute, { filled: report?.filled ?? 0 }), undefined, "success");
    } catch (error) {
      toastError(error);
    } finally {
      props.onChanged();
    }
  };

  const cancelPlan = async (plan: Plan) => {
    const reason = await confirmAction({
      title: tpl(t().plans.confirm_cancel_title, { slot: slotName(plan.slot) }),
      body: t().plans.confirm_cancel,
      confirmLabel: t().plans.cancel,
      danger: true,
      askReason: true,
      reasonLabel: t().plans.reason,
    });
    if (reason === null) return;
    await run(() => api.cancelPlan(plan.id, reason), t().plans.done_cancel);
  };

  const skipSlot = async (slot: SlotView) => {
    const reason = await confirmAction({
      title: tpl(t().plans.confirm_skip_title, { slot: slotName(slot.slot), date: m().schedule_date }),
      body: t().plans.confirm_skip,
      confirmLabel: t().plans.skip_slot,
      danger: true,
      askReason: true,
      reasonLabel: t().plans.reason,
    });
    if (reason === null) return;
    await run(() => api.cancelDay(m().schedule_date, [slot.slot], reason), t().plans.done_skip);
  };

  const restore = (slot: SlotView) => run(() => api.restoreSlot(m().schedule_date, slot.slot), t().plans.done_restore);

  const cancelDay = async () => {
    const reason = await confirmAction({
      title: tpl(t().plans.confirm_day_title, { date: m().schedule_date }),
      body: t().plans.confirm_day,
      confirmLabel: t().plans.cancel_day,
      danger: true,
      askReason: true,
      reasonLabel: t().plans.reason,
    });
    if (reason === null) return;
    try {
      const result = await api.cancelDay(m().schedule_date, [], reason);
      if (!result.cancelled_plans.length && !result.pre_cancelled.length) toast(t().plans.nothing_to_cancel, undefined, "info");
      else toast(t().plans.done_day, undefined, "success");
    } catch (error) {
      toastError(error);
    } finally {
      props.onChanged();
    }
  };

  const PlanSummary = (p: { plan: Plan }) => {
    const plan = () => p.plan;
    const top = createMemo(() =>
      [...ordersOf(plan().id)]
        .filter((o) => o.status === "planned")
        .sort((a, b) => (toNumber(b.qty) ?? 0) * b.ref_price - (toNumber(a.qty) ?? 0) * a.ref_price)
        .slice(0, 5),
    );
    return (
      <div class="slot-body">
        <Show when={plan().status !== "skipped" && plan().status !== "no_action"}>
          <div class="slot-metrics">
            <span>
              <b class="num">{tpl(t().plans.orders, { n: plan().order_count })}</b>
              <Show when={plan().summary?.buys != null}>
                <span class="muted"> · {tpl(t().plans.buys_sells, { buys: plan().summary.buys ?? 0, sells: plan().summary.sells ?? 0 })}</span>
              </Show>
            </span>
            <span class="muted">
              {t().plans.turnover} <b class="num">{fmtPct(plan().turnover, { dp: 1 })}</b>
            </span>
            <span class="muted">
              {t().plans.costs} <b class="num">{fmtMoney(plan().est_costs, { dp: 0 })}</b>
            </span>
          </div>
        </Show>
        <Show when={plan().status === "no_action"}>
          <p class="muted xs">{t().plans.no_action}</p>
        </Show>
        <Show when={plan().status === "pending"}>
          <p class="xs slot-when">
            <Icon name="clock" size={13} />
            <Show
              when={plan().automation_mode !== "approval" && plan().execute_after && new Date(plan().execute_after!).getTime() > now()}
              fallback={<span>{tpl(t().plans.awaiting, { time: fmtDual(plan().deadline) })}</span>}
            >
              <span>{tpl(t().plans.executes_in, { time: fmtCountdown(new Date(plan().execute_after!).getTime() - now()) })}</span>
            </Show>
          </p>
          <Show when={top().length}>
            <ul class="slot-orders">
              <For each={top()}>
                {(o) => (
                  <li>
                    <SideChip side={o.side} />
                    <span class="ticker">{o.symbol}</span>
                    <span class="num muted">{fmtQty(o.qty)}</span>
                    <span class="spacer" />
                    <span class="num">{fmtMoney((toNumber(o.qty) ?? 0) * o.ref_price, { dp: 0 })}</span>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </Show>
        <Show when={plan().status === "executed" || plan().status === "partially_executed"}>
          <p class="muted xs">{tpl(t().plans.executed_at, { time: fmtDual(plan().executed_at) })}</p>
        </Show>
        <Show when={plan().status === "cancelled"}>
          <p class="muted xs">
            {tpl(t().plans.cancelled_by, { who: plan().cancelled_by ?? "", reason: plan().cancel_reason ? `：${plan().cancel_reason}` : "" })}
          </p>
        </Show>
        <Show when={plan().status === "failed" && plan().error}>
          <p class="xs down">{tpl(t().plans.failed, { error: plan().error ?? "" })}</p>
        </Show>
        <div class="slot-actions">
          <A class="btn sm" href={`/plans/${plan().id}`}>
            {t().plans.view}
          </A>
          <Show when={plan().status === "pending" && canTrade()}>
            <button class="btn sm primary" onClick={() => approve(plan())}>
              {plan().automation_mode === "approval" ? t().plans.approve : t().plans.execute_now}
            </button>
            <button class="btn sm danger" onClick={() => cancelPlan(plan())}>
              {t().plans.cancel}
            </button>
          </Show>
        </div>
      </div>
    );
  };

  return (
    <div class="card today-plans">
      <div class="card-head">
        <div style={{ "min-width": 0 }}>
          <h2>{isToday() ? t().plans.title : t().plans.title_next}</h2>
          <div class="sub">{tpl(t().plans.sub, { date: m().schedule_date, weekday: weekday(m().schedule_date) })}</div>
        </div>
        <div class="spacer" />
        <A class="btn sm ghost" href="/plans">
          {t().plans.all_plans}
          <Icon name="chevron_right" size={14} />
        </A>
      </div>
      <div class="card-body stack" style={{ gap: "14px" }}>
        <Show when={m().holiday && !isToday()}>
          <div class="callout">
            <Icon name="calendar" size={16} />
            <span>{tpl(t().plans.holiday, { name: locale() === "zh" ? m().holiday!.name_zh : m().holiday!.name_en, date: m().schedule_date })}</span>
          </div>
        </Show>
        <Show when={m().effective_mode === "paused"}>
          <div class="callout warn">
            <Icon name="pause" size={16} />
            <span>{m().automation.paused_until ? tpl(t().plans.paused_until, { time: fmtDual(m().automation.paused_until, true) }) : t().plans.paused}</span>
          </div>
        </Show>
        <Show when={m().effective_mode === "approval"}>
          <div class="callout info">
            <Icon name="info" size={16} />
            <span>{t().plans.approval_mode}</span>
          </div>
        </Show>
        <Show when={m().early_close}>
          <div class="callout info">
            <Icon name="clock" size={16} />
            <span>{t().plans.early_close}</span>
          </div>
        </Show>

        <div class="timeline">
          <For each={m().schedule}>
            {(slot) => {
              const state = createMemo(() => slotState(slot, now()));
              return (
                <div class="tl-item">
                  <div class={`tl-dot ${dotClass(state())}`}>
                    <Show when={dotClass(state()) === "done"}>
                      <Icon name="check" size={12} />
                    </Show>
                  </div>
                  <div class="slot">
                    <div class="slot-head">
                      <div style={{ "min-width": 0 }}>
                        <div class="slot-title">{slotName(slot.slot)}</div>
                        <div class="muted xs num">{fmtDual(slot.generate_at, "auto")}</div>
                      </div>
                      <span class="spacer" />
                      <Show when={state().kind === "plan"}>
                        <PlanStatusChip status={(state() as { plan: Plan }).plan.status} />
                      </Show>
                      <Show when={state().kind === "scheduled" || state().kind === "due"}>
                        <PlanStatusChip status="scheduled" />
                      </Show>
                      <Show when={state().kind === "cancelled_ahead"}>
                        <PlanStatusChip status="cancelled_ahead" />
                      </Show>
                      <Show when={state().kind === "skipped" || state().kind === "missed"}>
                        <PlanStatusChip status="skipped" />
                      </Show>
                    </div>
                    <Show when={state().kind === "plan"}>
                      <PlanSummary plan={(state() as { plan: Plan }).plan} />
                    </Show>
                    <Show when={state().kind === "scheduled" || state().kind === "due"}>
                      <div class="slot-body">
                        <p class="muted xs">
                          {state().kind === "scheduled"
                            ? tpl(t().plans.scheduled, { time: fmtCountdown(new Date(slot.generate_at).getTime() - now()) })
                            : t().plans.generating}
                        </p>
                        <Show when={canTrade()}>
                          <div class="slot-actions">
                            <button class="btn sm" onClick={() => skipSlot(slot)}>
                              <Icon name="ban" size={14} /> {t().plans.skip_slot}
                            </button>
                          </div>
                        </Show>
                      </div>
                    </Show>
                    <Show when={state().kind === "cancelled_ahead"}>
                      <div class="slot-body">
                        <p class="muted xs">{tpl(t().plans.cancelled_ahead, { who: (state() as { who: string }).who })}</p>
                        <Show when={canTrade() && now() < new Date(slot.window_end).getTime()}>
                          <div class="slot-actions">
                            <button class="btn sm" onClick={() => restore(slot)}>
                              <Icon name="refresh" size={14} /> {t().plans.restore}
                            </button>
                          </div>
                        </Show>
                      </div>
                    </Show>
                    <Show when={state().kind === "skipped"}>
                      <div class="slot-body">
                        <p class="muted xs">
                          {tpl(t().plans.skipped_reason, {
                            reason: c().skip_reason[(state() as { reason: string }).reason as "paused" | "operator" | "missed" | "error"] ?? (state() as { reason: string }).reason,
                          })}
                        </p>
                      </div>
                    </Show>
                    <Show when={state().kind === "missed"}>
                      <div class="slot-body">
                        <p class="muted xs">{tpl(t().plans.skipped_reason, { reason: c().skip_reason.missed })}</p>
                      </div>
                    </Show>
                  </div>
                </div>
              );
            }}
          </For>
          <For each={manual()}>
            {(plan) => (
              <div class="tl-item">
                <div class={`tl-dot ${dotClass({ kind: "plan", plan })}`}>
                  <Show when={dotClass({ kind: "plan", plan }) === "done"}>
                    <Icon name="check" size={12} />
                  </Show>
                </div>
                <div class="slot">
                  <div class="slot-head">
                    <div>
                      <div class="slot-title">{t().plans.manual}</div>
                      <div class="muted xs num">{fmtDual(plan.generated_at)}</div>
                    </div>
                    <span class="spacer" />
                    <PlanStatusChip status={plan.status} />
                  </div>
                  <PlanSummary plan={plan} />
                </div>
              </div>
            )}
          </For>
        </div>
      </div>
      <Show when={canTrade()}>
        <div class="card-foot">
          <button class="btn sm danger" disabled={!canCancelDay()} onClick={cancelDay}>
            <Icon name="ban" size={14} /> {t().plans.cancel_day}
          </button>
        </div>
      </Show>
    </div>
  );
}
