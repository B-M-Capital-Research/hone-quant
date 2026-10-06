/**
 * Readable audit entries: bilingual action and object labels, links to the object, one-line
 * summaries of the detail payload and field-level diffs of `{before, after}` records.
 * Action codes come from `crates/quant-server/src` (every `system::audit` call site).
 */
import { locale, tpl } from "@/i18n";
import { auditText } from "@/i18n/audit";
import { common } from "@/i18n/common";
import { fmtDual, fmtMoney, fmtQty, toNumber } from "@/lib/format";
import type { AuditEntry } from "@/lib/types";

export type ActorKind = "user" | "scheduler" | "system" | "cli";

export function actorKind(actor: string): ActorKind {
  if (actor === "scheduler") return "scheduler";
  if (actor === "system") return "system";
  if (actor === "cli") return "cli";
  return "user";
}

export function actionLabel(action: string): string {
  const known = (auditText().actions as Record<string, string>)[action];
  return known ?? action;
}

export function isKnownAction(action: string): boolean {
  return action in (auditText().actions as Record<string, string>);
}

/** Action groups in display order, each with its known actions. */
export function actionGroups(): { prefix: string; label: string; actions: string[] }[] {
  const a = auditText();
  const groups = a.groups as Record<string, string>;
  const codes = Object.keys(a.actions);
  return Object.keys(groups).map((key) => ({
    prefix: `${key}.`,
    label: groups[key],
    actions: codes.filter((code) => code.startsWith(`${key}.`)),
  }));
}

export const ENTITY_TYPES = [
  "plan",
  "order",
  "trading_day",
  "strategy_version",
  "settings",
  "restriction",
  "universe",
  "channel",
  "reminder",
  "backtest",
  "account",
  "position",
  "data",
  "user",
] as const;

export function entityTypeLabel(type: string): string {
  return (auditText().entities as Record<string, string>)[type] ?? type;
}

export function entityLabel(entry: Pick<AuditEntry, "entity_type" | "entity_id">): string {
  const type = entityTypeLabel(entry.entity_type);
  const id = entry.entity_id;
  if (!id) return type;
  if (/^\d+$/.test(id)) return `${type} #${id}`;
  if (entry.entity_type === "settings") {
    const section = (auditText().sections as Record<string, string>)[id];
    if (section) return `${type} · ${section}`;
  }
  return `${type} · ${id}`;
}

const record = (value: unknown): Record<string, unknown> =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};

const text = (value: unknown): string => (typeof value === "string" ? value.trim() : "");

/** In-app link for the entry's object, when it still exists and has a page. */
export function entityHref(entry: AuditEntry): string | null {
  const id = entry.entity_id;
  const detail = record(entry.detail);
  if (entry.action.endsWith(".deleted")) return null;
  switch (entry.entity_type) {
    case "plan":
      return /^\d+$/.test(id) ? `/plans/${id}` : null;
    case "order":
      return typeof detail.plan_id === "number" ? `/plans/${detail.plan_id}` : null;
    case "strategy_version":
      return /^\d+$/.test(id) ? `/strategy?version=${id}` : null;
    case "backtest":
      return /^\d+$/.test(id) ? `/backtests/${id}` : null;
    case "position":
      return id ? `/trades?symbol=${encodeURIComponent(id)}` : null;
    case "trading_day":
      return /^\d{4}-\d{2}-\d{2}$/.test(id) ? `/plans?from=${id}&to=${id}` : "/plans";
    case "restriction":
    case "universe":
      return "/universe";
    case "reminder":
      return "/notifications?tab=reminders";
    case "channel":
      return "/settings/notifications";
    case "settings":
      return id === "notifications" ? "/settings/notifications" : "/settings";
    case "account":
      return "/trades?tab=ledger";
    case "data":
      return "/settings/data";
    default:
      return null;
  }
}

export interface FieldChange {
  path: string;
  before: unknown;
  after: unknown;
}

function flatten(value: unknown, prefix: string, out: Map<string, unknown>) {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const entries = Object.entries(value as Record<string, unknown>);
    if (!entries.length && prefix) out.set(prefix, value);
    for (const [key, child] of entries) flatten(child, prefix ? `${prefix}.${key}` : key, out);
  } else if (prefix) {
    out.set(prefix, value);
  }
}

/** Leaf-level differences between `detail.before` and `detail.after` (null when not a diff record). */
export function changes(entry: AuditEntry): FieldChange[] | null {
  const detail = record(entry.detail);
  if (!("before" in detail) || !("after" in detail)) return null;
  const before = new Map<string, unknown>();
  const after = new Map<string, unknown>();
  flatten(detail.before, "", before);
  flatten(detail.after, "", after);
  const ignore = new Set(["updated_at"]);
  const paths = [...new Set([...before.keys(), ...after.keys()])].filter((p) => !ignore.has(p)).sort();
  return paths
    .filter((p) => JSON.stringify(before.get(p) ?? null) !== JSON.stringify(after.get(p) ?? null))
    .map((p) => ({ path: p, before: before.get(p), after: after.get(p) }));
}

export function showValue(value: unknown): string {
  if (value === undefined) return "—";
  if (value === null) return "null";
  if (typeof value === "string") return value === "" ? '""' : value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return JSON.stringify(value);
}

function slotName(slot: unknown): string {
  const c = common();
  return typeof slot === "string" && slot in c.slot ? c.slot[slot as "open" | "close" | "manual"] : text(slot);
}

function modeName(mode: unknown): string {
  const c = common();
  return typeof mode === "string" && (mode === "auto" || mode === "approval" || mode === "paused") ? c.mode[mode] : text(mode);
}

function money(value: unknown): string {
  return fmtMoney(toNumber(value as number | string | null));
}

/** One line describing the payload, or null when there is nothing worth showing. */
export function summarize(entry: AuditEntry): string | null {
  const s = auditText().summary;
  const d = record(entry.detail);
  const sep = locale() === "zh" ? "、" : ", ";
  const reasonOrNote = () => (text(d.reason) ? tpl(s.reason, { text: text(d.reason) }) : text(d.note) ? tpl(s.note, { text: text(d.note) }) : null);

  switch (entry.action) {
    case "plan.generated":
      return tpl(s.plan_generated, { slot: slotName(d.slot), orders: String(d.orders ?? "—"), mode: modeName(d.mode) });
    case "plan.executed":
      return tpl(s.plan_executed, {
        filled: String((toNumber(d.filled as number) ?? 0) + (toNumber(d.partial as number) ?? 0)),
        rejected: String(d.rejected ?? 0),
        bought: money(d.bought),
        sold: money(d.sold),
      });
    case "plan.failed":
      return text(d.error) || null;
    case "plan.approved":
    case "plan.cancelled":
    case "strategy.activated":
      return reasonOrNote() ?? (text(d.preset) ? tpl(s.preset, { preset: text(d.preset) }) : null);
    case "order.skipped": {
      const side = d.side === "buy" || d.side === "sell" ? common().side[d.side] : text(d.side);
      return tpl(s.order_skipped, { side, symbol: text(d.symbol), qty: fmtQty(d.qty as string), plan: String(d.plan_id ?? "—") });
    }
    case "trading_day.cancelled": {
      const slots = Array.isArray(d.slots) ? d.slots.map(slotName).join(sep) : "";
      const reason = reasonOrNote();
      return [tpl(s.slots, { slots }), reason].filter(Boolean).join(" · ");
    }
    case "trading_day.restored":
      return tpl(s.slot, { slot: slotName(d.slot) });
    case "automation.changed": {
      const before = record(d.before);
      const after = record(d.after);
      const parts = [tpl(s.mode_change, { before: modeName(before.mode), after: modeName(after.mode) })];
      if (typeof after.paused_until === "string") parts.push(tpl(s.paused_until, { time: fmtDual(after.paused_until, true) }));
      if (text(after.note)) parts.push(tpl(s.note, { text: text(after.note) }));
      return parts.join(" · ");
    }
    case "restriction.added":
    case "restriction.revoked": {
      const mode = (s.restriction_mode as Record<string, string>)[text(d.mode)] ?? text(d.mode);
      const head = tpl(s.restriction, { symbol: text(d.symbol), mode });
      return text(d.reason) ? `${head} · ${tpl(s.reason, { text: text(d.reason) })}` : head;
    }
    case "strategy.version_created":
      return [text(d.name), text(d.note) ? tpl(s.note, { text: text(d.note) }) : ""].filter(Boolean).join(" · ") || null;
    case "universe.applied":
    case "universe.synced": {
      const c = record(d.changes ?? d);
      const count = (v: unknown) => (Array.isArray(v) ? v.length : 0);
      return tpl(s.universe, { added: count(c.added), removed: count(c.removed), moved: count(c.moved) });
    }
    case "channel.saved":
      return tpl(s.channel, { kind: text(d.kind), state: d.enabled ? auditText().enabled : auditText().disabled });
    case "channel.tested":
      return d.ok ? s.channel_test_ok : tpl(s.channel_test_failed, { error: text(d.error) || "—" });
    case "reminder.created":
      return text(d.title) || null;
    case "backtest.submitted":
      return text(d.name) || null;
    case "account.created":
    case "account.reset":
      return tpl(s.initial_cash, { cash: money(d.initial_cash) });
    case "corporate_action.split":
    case "corporate_action.dividend":
      return text(d.detail) || null;
    case "data.sync_requested":
      return entry.entity_id ? tpl(s.sync, { kind: entry.entity_id }) : null;
    case "user.created":
      return text(d.role) ? tpl(s.role, { role: text(d.role) }) : null;
    default:
      break;
  }
  const diff = changes(entry);
  if (diff) {
    if (!diff.length) return s.unchanged;
    const shown = diff.slice(0, 3).map((c) => c.path);
    return tpl(s.changed, { n: diff.length, fields: shown.join(sep) + (diff.length > 3 ? "…" : "") });
  }
  return reasonOrNote();
}
