import { A, useNavigate, useSearchParams } from "@solidjs/router";
import { For, Show, createMemo, createResource, onCleanup, onMount } from "solid-js";
import { Icon } from "@/components/Icon";
import { Empty, ErrorState, Loading, PlanStatusChip, Segmented, confirmAction, downloadCsv, toast, toastError } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { plansText } from "@/i18n/plans";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { MARKET_TZ, fmtDate, fmtDual, fmtMoney, fmtPct } from "@/lib/format";
import { isAdmin, market, serverNow } from "@/lib/session";
import { actorName } from "@/lib/names";
import type { Plan, PlanStatus } from "@/lib/types";
import "@/styles/plans.css";
import { throttle, weekday } from "./overview/util";

const PAGE = 50;
const STATUSES: PlanStatus[] = ["pending", "executing", "executed", "partially_executed", "no_action", "cancelled", "expired", "skipped", "failed"];
const RANGES = ["7", "30", "90", "all"] as const;
type Range = (typeof RANGES)[number];

function shiftDate(date: string, days: number): string {
  const [y, m, d] = date.split("-").map(Number);
  const t = new Date(Date.UTC(y, m - 1, d) - days * 86_400_000);
  return t.toISOString().slice(0, 10);
}

export default function Plans() {
  const t = plansText;
  const c = common;
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams<{ range?: string; status?: string; page?: string }>();

  const range = (): Range => (RANGES as readonly string[]).includes(params.range ?? "") ? (params.range as Range) : "30";
  const status = () => (STATUSES as string[]).includes(params.status ?? "") ? params.status! : "";
  const page = () => Math.max(0, Number(params.page ?? 0) || 0);
  const query = createMemo(() => {
    const today = fmtDate(new Date(serverNow()), MARKET_TZ);
    const r = range();
    return {
      from: r === "all" ? undefined : shiftDate(today, Number(r)),
      status: status() || undefined,
      limit: PAGE,
      offset: page() * PAGE,
    };
  }, undefined, { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) });

  const [plans, { refetch }] = createResource(query, (q) => api.plans(q));

  onMount(() => {
    const off = onServerEvent(["plan", "account"], throttle(() => refetch(), 3000));
    onCleanup(off);
  });

  const inSession = () => {
    const m = market();
    if (!m?.today || m.phase !== "open") return false;
    return serverNow() < new Date(m.today.close).getTime() - 10 * 60_000;
  };

  const generate = async () => {
    const note = await confirmAction({ title: t().list.confirm_generate_title, body: t().list.confirm_generate, confirmLabel: t().list.generate });
    if (note === null) return;
    try {
      const { plan } = await api.generatePlan();
      toast(t().list.generated, undefined, "success");
      navigate(`/plans/${plan.plan_id}`);
    } catch (error) {
      toastError(error);
    }
  };

  const exportCsv = () => {
    const rows = plans.latest?.plans ?? [];
    downloadCsv(
      `${t().list.export_name}.csv`,
      ["id", "trade_date", "slot", "status", "orders", "turnover", "est_costs", "invested_target", "generated_at", "executed_at", "created_by", "cancel_reason"],
      rows.map((p) => [p.id, p.trade_date, p.slot, p.status, p.order_count, p.turnover, p.est_costs, p.invested_target, p.generated_at, p.executed_at, p.created_by, p.cancel_reason]),
    );
  };

  const total = () => plans.latest?.total ?? 0;
  const mode = () => market()?.effective_mode;

  return (
    <div class="stack">
      <div class="page-head">
        <div style={{ flex: 1, "min-width": "280px" }}>
          <h1>{t().list.title}</h1>
          <p class="lead">{t().list.lead}</p>
        </div>
        <Show when={isAdmin()}>
          <button class="btn primary" onClick={generate} disabled={!inSession() || mode() === "paused"} title={inSession() ? t().list.generate_hint : t().list.generate_closed}>
            <Icon name="plus" size={16} /> {t().list.generate}
          </button>
        </Show>
      </div>

      <Show when={market()}>
        {(m) => (
          <div class="schedule-strip">
            <span class="kicker">{t().list.schedule}</span>
            <For each={m().schedule}>
              {(slot) => (
                <span class="schedule-item">
                  <b>{c().slot[slot.slot]}</b>
                  <span class="num">{fmtDual(slot.generate_at, "auto")}</span>
                </span>
              )}
            </For>
            <Show when={m().schedule.length}>
              <span class="schedule-item muted">
                {tpl(t().list.review, {
                  n: Math.round((new Date(m().schedule[0].execute_at).getTime() - new Date(m().schedule[0].generate_at).getTime()) / 60_000),
                })}
              </span>
            </Show>
            <span class="schedule-item">
              <span class="muted">{t().list.mode}</span> <b>{c().mode[m().effective_mode]}</b>
            </span>
            <span class="spacer" />
            <A class="btn sm ghost" href="/settings/schedule">
              <Icon name="clock" size={14} /> {t().list.schedule_settings}
            </A>
          </div>
        )}
      </Show>

      <div class="card">
        <div class="card-head" style={{ "flex-wrap": "wrap", "row-gap": "10px" }}>
          <Segmented
            value={range()}
            onChange={(v) => setParams({ range: v, page: undefined })}
            options={[
              { value: "7", label: t().list.range_7 },
              { value: "30", label: t().list.range_30 },
              { value: "90", label: t().list.range_90 },
              { value: "all", label: t().list.range_all },
            ]}
          />
          <select class="select" style={{ width: "auto", "min-width": "150px" }} value={status()} onChange={(e) => setParams({ status: e.currentTarget.value || undefined, page: undefined })} aria-label={t().list.col_status}>
            <option value="">{t().list.status_all}</option>
            <For each={STATUSES}>{(s) => <option value={s}>{c().plan_status[s]}</option>}</For>
          </select>
          <div class="spacer" />
          <span class="muted xs">{tpl(t().list.total, { n: total() })}</span>
          <button class="btn sm" onClick={exportCsv} disabled={!plans.latest?.plans.length}>
            <Icon name="download" size={14} /> {c().actions.export_csv}
          </button>
        </div>
        <div class="card-body flush">
          <Show
            when={plans.latest}
            fallback={
              <Show when={plans.error} fallback={<Loading />}>
                {(error) => <ErrorState error={error()} onRetry={refetch} />}
              </Show>
            }
          >
            <Show when={plans.latest!.plans.length} fallback={<Empty title={t().list.empty} icon="plans" />}>
              <div class="table-wrap">
                <table class={`table ${plans.loading ? "refetching" : ""}`}>
                  <thead>
                    <tr>
                      <th>{t().list.col_date}</th>
                      <th>{t().list.col_slot}</th>
                      <th>{t().list.col_status}</th>
                      <th class="r">{t().list.col_orders}</th>
                      <th class="r">{t().list.col_turnover}</th>
                      <th class="r">{t().list.col_costs}</th>
                      <th>{t().list.col_result}</th>
                      <th class="r">{t().list.col_exposure}</th>
                      <th>{t().list.col_generated}</th>
                      <th>{t().list.col_by}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={plans.latest!.plans}>{(p) => <PlanRow plan={p} onOpen={() => navigate(`/plans/${p.id}`)} />}</For>
                  </tbody>
                </table>
              </div>
            </Show>
          </Show>
        </div>
        <Show when={total() > PAGE}>
          <div class="card-foot">
            <span>{tpl(t().list.page, { from: page() * PAGE + 1, to: Math.min(total(), (page() + 1) * PAGE) })}</span>
            <span class="spacer" />
            <button class="btn sm" disabled={page() === 0} onClick={() => setParams({ page: page() - 1 || undefined })}>
              <Icon name="chevron_left" size={14} /> {t().list.prev}
            </button>
            <button class="btn sm" disabled={(page() + 1) * PAGE >= total()} onClick={() => setParams({ page: page() + 1 })}>
              {t().list.next} <Icon name="chevron_right" size={14} />
            </button>
          </div>
        </Show>
      </div>
    </div>
  );
}

function PlanRow(props: { plan: Plan; onOpen: () => void }) {
  const t = plansText;
  const c = common;
  const p = () => props.plan;
  const executed = () => p().summary?.executed;
  return (
    <tr class="clickable" onClick={props.onOpen}>
      <td class="nowrap">
        <A href={`/plans/${p().id}`} class="num" onClick={(e) => e.stopPropagation()}>
          {p().trade_date}
        </A>
        <span class="muted xs"> {weekday(p().trade_date)}</span>
      </td>
      <td class="nowrap">{c().slot[p().slot]}</td>
      <td>
        <PlanStatusChip status={p().status} />
      </td>
      <td class="r num">
        {p().order_count}
        <Show when={p().summary?.buys != null && p().order_count > 0}>
          <div class="muted xs">
            {c().side.buy} {p().summary.buys} · {c().side.sell} {p().summary.sells}
          </div>
        </Show>
      </td>
      <td class="r num">{p().order_count ? fmtPct(p().turnover, { dp: 1 }) : "—"}</td>
      <td class="r num">{p().order_count ? fmtMoney(p().est_costs, { dp: 0 }) : "—"}</td>
      <td class="xs">
        <Show when={executed()} fallback={<span class="muted">{p().status === "skipped" && p().summary?.skip_reason ? c().skip_reason[p().summary.skip_reason as "paused"] ?? p().summary.skip_reason : "—"}</span>}>
          {tpl(t().list.result, { filled: executed()!.filled, partial: executed()!.partial, rejected: executed()!.rejected })}
        </Show>
      </td>
      <td class="r num">{fmtPct(p().invested_target, { dp: 1 })}</td>
      <td class="nowrap xs num">{fmtDual(p().generated_at, "auto")}</td>
      <td class="xs">{actorName(p().created_by)}</td>
    </tr>
  );
}
