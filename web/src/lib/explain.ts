/**
 * Plan explanations as text. The server explains a plan with codes and numbers
 * (quant_core::explain); this renders them in the active language. Notifications render the
 * same codes on the server (services/explain.rs).
 */
import { locale, tpl } from "@/i18n";
import { explainText } from "@/i18n/explain";
import { fmtPct } from "@/lib/format";
import type { OrderDriver, OrderExplanation, PlanDriver, PlanExplanation } from "@/lib/types";

/** Sector display name by id. */
export type SectorName = (id: string) => string;

const p1 = (value: number | null | undefined) => fmtPct(value, { dp: 1 });
/** A weight gap in percentage points, unsigned: 0.0759 → "7.6". */
const pts = (value: number) => (Math.abs(value) * 100).toFixed(1);

type Unit = "order" | "name" | "position" | "trend_break" | "trend_recovery";

/** "1 order", "3 orders" (Chinese has no plural). */
export function count(n: number, unit: Unit): string {
  const u = explainText().unit;
  return tpl(n === 1 ? u[`${unit}_one`] : u[`${unit}_many`], { n });
}

export function orderDriverName(driver: OrderDriver): string {
  return explainText().order_driver[driver] ?? driver;
}

/** One plan-level driver as a clause. */
export function driverText(d: PlanDriver, sector: SectorName): string {
  const t = explainText().driver;
  switch (d.code) {
    case "first_plan":
      return tpl(t.first_plan, { eligible: d.eligible, universe: d.universe });
    case "strategy_changed":
      return t.strategy_changed;
    case "history_limited":
      return tpl(t.history_limited, { insufficient: d.insufficient, universe: d.universe });
    case "no_action":
      return tpl(t[`no_action_${d.reason}` as const], { gap: pts(d.gap) });
    case "newly_eligible":
      return tpl(d.cut ? t.newly_eligible_cut : t.newly_eligible, { count: d.count, names: count(d.count, "name"), cut: count(d.cut, "position") });
    case "breadth": {
      const crossed =
        d.crossed_below && d.crossed_above
          ? t.crossed_both
          : d.crossed_below
            ? t.crossed_below
            : d.crossed_above
              ? t.crossed_above
              : "";
      return tpl(t.breadth, {
        before: p1(d.before),
        now: p1(d.now),
        crossed: tpl(crossed, { b: d.crossed_below, a: d.crossed_above, line: d.line }),
        e0: p1(d.exposure_before),
        e1: p1(d.exposure_now),
      });
    }
    case "trend_breaks":
      return tpl(d.orders ? t.trend_breaks_orders : t.trend_breaks, { breaks: count(d.count, "trend_break"), orders: count(d.orders, "order") });
    case "trend_recoveries":
      return tpl(t.trend_recoveries, { recoveries: count(d.count, "trend_recovery") });
    case "sector_budget":
      return tpl(t.sector_budget, { sector: sector(d.sector), before: p1(d.before), now: p1(d.now) });
    case "orders":
      return tpl(t.orders, { orders: count(d.orders, "order"), driver: orderDriverName(d.driver) });
    case "price_drift":
      return tpl(t.price_drift, { orders: count(d.orders, "order") });
    case "carry_over":
      return tpl(d.previous_cash_scale != null ? t.carry_over_cash : t.carry_over, { orders: count(d.orders, "order") });
    case "retry":
      return tpl(t.retry, { orders: count(d.orders, "order") });
    case "turnover_cap":
      return tpl(t.turnover_cap, { limit: fmtPct(d.limit, { dp: 0 }), scale: fmtPct(d.scale, { dp: 0 }) });
    case "cash_cap":
      return tpl(t.cash_cap, { scale: fmtPct(d.scale, { dp: 0 }) });
    case "band_waiver":
      return tpl(d.side === "sells" ? t.band_waiver_sells : t.band_waiver_buys, {
        now: p1(d.invested_now),
        target: p1(d.invested_target),
        gap: pts(d.gap),
      });
    case "frozen":
      return tpl(t.frozen, { count: d.count });
    case "excluded":
      return tpl(t.excluded, { count: d.count });
  }
}

/** A short chip label for a driver. */
export function driverChip(d: PlanDriver, sector: SectorName): string {
  const t = explainText().chip;
  switch (d.code) {
    case "first_plan":
    case "strategy_changed":
    case "no_action":
      return t[d.code];
    case "history_limited":
      return tpl(t.history_limited, { n: d.insufficient });
    case "newly_eligible":
    case "trend_breaks":
    case "trend_recoveries":
      return tpl(t[d.code], { n: d.count });
    case "breadth":
      return tpl(t.breadth, { before: p1(d.before), now: p1(d.now) });
    case "sector_budget":
      return tpl(t.sector_budget, { sector: sector(d.sector) });
    case "orders":
      return tpl(t.orders, { driver: orderDriverName(d.driver), n: d.orders });
    case "price_drift":
    case "carry_over":
    case "retry":
      return tpl(t[d.code], { n: d.orders });
    case "turnover_cap":
    case "cash_cap":
      return tpl(t[d.code], { scale: fmtPct(d.scale, { dp: 0 }) });
    case "band_waiver":
      return d.side === "sells" ? t.band_waiver_sells : t.band_waiver_buys;
    case "frozen":
    case "excluded":
      return tpl(t[d.code], { n: d.count });
  }
}

/** The first `n` drivers as one sentence. */
export function oneLine(drivers: PlanDriver[], sector: SectorName, n = 3): string {
  const parts = drivers.slice(0, n).map((d) => driverText(d, sector));
  if (!parts.length) return "";
  if (locale() === "zh") return `${parts.join("；")}。`;
  // Later clauses start lower-case, unless they start with an acronym ("AI chips budget").
  const lower = (p: string) => (/^[A-Z][a-z]/.test(p) ? p.charAt(0).toLowerCase() + p.slice(1) : p);
  const text = parts.map((p, i) => (i === 0 ? p : lower(p))).join("; ");
  return `${text}.`;
}

/** Why one order: its dominant driver with the numbers behind it. */
export function orderWhy(o: OrderExplanation, x: PlanExplanation | null, sector: SectorName): string {
  const t = explainText().order_why;
  switch (o.driver) {
    case "exposure":
      return tpl(t.exposure, { before: p1(x?.exposure.before), now: p1(x?.exposure.now) });
    case "sector_budget": {
      const s = x?.sectors.find((s) => s.sector === o.sector);
      return tpl(t.sector_budget, { sector: sector(o.sector ?? ""), before: p1(s?.before), now: p1(s?.now) });
    }
    case "trend":
      return tpl(t.trend, {
        before: o.trend_multiplier_before == null ? "—" : o.trend_multiplier_before.toFixed(2),
        now: o.trend_multiplier_now == null ? "—" : o.trend_multiplier_now.toFixed(2),
      });
    case "price_drift":
      return o.price_change == null
        ? tpl(t.price_drift_plain, { target: p1(o.previous_target) })
        : tpl(t.price_drift, { change: fmtPct(o.price_change, { dp: 1, sign: true }), target: p1(o.previous_target) });
    case "retry": {
      const status = o.previous_order_status ? explainText().order_status[o.previous_order_status] : "";
      const reason = o.previous_status_reason ? ` (${o.previous_status_reason})` : "";
      return tpl(t.retry, { status: `${status}${reason}` });
    }
    default:
      return t[o.driver];
  }
}

/** Secondary notes for an order: band waiver, this plan's cap, model vs drift. */
export function orderNotes(o: OrderExplanation): string[] {
  const t = explainText().order_why;
  const notes: string[] = [];
  if (o.model != null && o.drift != null) {
    notes.push(tpl(t.model, { model: fmtPct(o.model, { dp: 2, sign: true }) }));
    notes.push(tpl(t.drift, { drift: fmtPct(o.drift, { dp: 2, sign: true }) }));
  }
  if (o.band_waived) notes.push(t.waived);
  if (o.scaled) notes.push(t.scaled);
  return notes;
}

/** Tone of a driver chip. */
export function driverTone(d: PlanDriver): string {
  switch (d.code) {
    case "breadth":
      return d.now < d.before ? "yellow" : "green";
    case "trend_breaks":
    case "band_waiver":
    case "turnover_cap":
    case "cash_cap":
    case "history_limited":
      return "yellow";
    case "trend_recoveries":
    case "newly_eligible":
      return "green";
    case "frozen":
    case "excluded":
    case "retry":
      return "blue";
    default:
      return "outline";
  }
}
