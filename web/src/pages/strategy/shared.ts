/**
 * Strategy-page helpers shared by the tabs: when the next plan is generated, which plans are
 * still pending, and the activation flow (confirmation that states precisely what changes).
 */
import { confirmAction, toast, toastError } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { strategyText } from "@/i18n/strategy";
import { api } from "@/lib/api";
import { fmtDual } from "@/lib/format";
import { currentPortfolio } from "@/lib/portfolio";
import { market } from "@/lib/session";
import type { MarketView, Plan, Slot } from "@/lib/types";
import { strategyName } from "@/lib/names";

export interface NextPlan {
  slot: Slot;
  at: string;
}

/** The next scheduled plan generation on the schedule date, if any. */
export function nextPlan(m: MarketView | null, now: number): NextPlan | null {
  if (!m || m.effective_mode === "paused") return null;
  for (const slot of m.schedule) {
    if (slot.plan || slot.cancelled) continue;
    if (new Date(slot.generate_at).getTime() > now) return { slot: slot.slot, at: slot.generate_at };
  }
  return null;
}

/** The next plan generation as a label and (when scheduled) its time in both zones. */
export function nextPlanInfo(now: number): { label: string; time: string | null } {
  const m = market();
  const t = strategyText().effect;
  if (!m) return { label: "—", time: null };
  if (m.effective_mode === "paused") return { label: t.paused, time: null };
  const next = nextPlan(m, now);
  if (next) return { label: common().slot[next.slot], time: fmtDual(next.at, true) };
  return { label: tpl(t.next_session, { date: m.next_session.date }), time: null };
}

async function pendingPlans(): Promise<Plan[]> {
  try {
    const { plans } = await api.plans({ status: "pending", limit: 20 });
    return plans;
  } catch {
    return (market()?.schedule ?? []).map((s) => s.plan).filter((p): p is Plan => !!p && p.status === "pending");
  }
}

/** Confirmation text for activating a version, grounded in the live schedule and pending plans. */
export async function activationBody(name: string, id: number | null, now: number): Promise<string> {
  const t = strategyText().activate;
  const c = common();
  const m = market();
  const pending = await pendingPlans();
  const next = nextPlan(m, now);
  const lines = [id === null ? tpl(t.intro_new, { name }) : tpl(t.intro, { name, id })];
  const portfolio = currentPortfolio();
  if (portfolio) lines.push(tpl(t.scope, { name: portfolio.name }));
  lines.push(tpl(t.next, { detail: next ? tpl(t.next_detail, { slot: c.slot[next.slot], time: fmtDual(next.at, true) }) : "" }));
  lines.push(
    tpl(t.pending, {
      detail: pending.length
        ? tpl(t.pending_detail, { n: pending.length, list: pending.map((p) => `#${p.id} ${c.slot[p.slot]}`).join(listSep()) })
        : "",
    }),
  );
  if (m?.effective_mode === "paused") lines.push(t.paused);
  lines.push(t.no_orders);
  return lines.join("\n");
}

function listSep(): string {
  return locale() === "zh" ? "、" : ", ";
}

/**
 * Asks for confirmation (with an optional activation note) and activates an existing version.
 * Returns true when the version was activated.
 */
export async function activateExisting(version: { id: number; name: string; preset_id?: string | null }, now: number): Promise<boolean> {
  const t = strategyText().activate;
  const body = await activationBody(strategyName(version), version.id, now);
  const note = await confirmAction({ title: t.title, body, confirmLabel: t.confirm, askReason: true, reasonLabel: t.note });
  if (note === null) return false;
  try {
    await api.activateVersion(version.id, note.trim());
    toast(tpl(t.done, { id: version.id }), t.done_body, "success");
    return true;
  } catch (error) {
    toastError(error);
    return false;
  }
}

export function backtestHref(versionId: number): string {
  return `/backtests?new=1&version=${versionId}`;
}
