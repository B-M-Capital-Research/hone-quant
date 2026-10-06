/**
 * Universe-page helpers: weights per company and sector (current from the valuation, target from
 * the latest plan), restriction state on a given New York date, and name lookups.
 */
import { pick } from "@/i18n";
import { universeText } from "@/i18n/universe";
import type { Asset, Dashboard, PositionView, Restriction, Sector, UniverseView } from "@/lib/types";

/** `pick` for interface-typed rows (see the note in pages/strategy/format.ts). */
export function pickText(row: object | null | undefined, field: string): string {
  return pick(row as Record<string, unknown> | null | undefined, field);
}

export interface Weights {
  /** Current weight per symbol (0 when not held). */
  current: (symbol: string) => number;
  /** Target weight per symbol from the latest plan; null when there is no plan yet. */
  target: (symbol: string) => number | null;
  position: (symbol: string) => PositionView | undefined;
  hasTargets: boolean;
  cash: number | null;
  targetCash: number | null;
  planId: number | null;
  planAt: string | null;
}

export function makeWeights(dashboard: Dashboard | null | undefined): Weights {
  const positions = new Map((dashboard?.valuation.positions ?? []).map((p) => [p.symbol, p]));
  const targets = dashboard?.targets?.weights ?? null;
  const invested = [...positions.values()].reduce((a, p) => a + p.weight, 0);
  const targetInvested = targets ? Object.values(targets).reduce((a, w) => a + w, 0) : null;
  return {
    current: (s) => positions.get(s)?.weight ?? 0,
    target: (s) => (targets ? targets[s] ?? 0 : null),
    position: (s) => positions.get(s),
    hasTargets: !!targets,
    cash: dashboard ? Math.max(0, 1 - invested) : null,
    targetCash: targetInvested === null ? null : Math.max(0, 1 - targetInvested),
    planId: dashboard?.targets?.plan_id ?? null,
    planAt: dashboard?.targets?.generated_at ?? null,
  };
}

export interface SectorRow {
  sector: Sector;
  members: Asset[];
  current: number;
  target: number | null;
}

export function sectorRows(view: UniverseView, weights: Weights): SectorRow[] {
  return [...view.sectors]
    .sort((a, b) => a.sort_order - b.sort_order)
    .map((sector) => {
      const members = view.assets.filter((a) => a.sector_id === sector.id).sort((a, b) => a.sort_order - b.sort_order);
      const current = members.reduce((acc, a) => acc + weights.current(a.symbol), 0);
      const target = weights.hasTargets ? members.reduce((acc, a) => acc + (weights.target(a.symbol) ?? 0), 0) : null;
      return { sector, members, current, target };
    });
}

export type RestrictionState = "active" | "scheduled" | "expired" | "revoked";

export function restrictionState(r: Restriction, today: string): RestrictionState {
  if (r.revoked_at) return "revoked";
  if (r.ends_on && r.ends_on < today) return "expired";
  if (r.starts_on > today) return "scheduled";
  return "active";
}

export function modeText(mode: Restriction["mode"]): string {
  const t = universeText().restrictions;
  return mode === "exclude" ? t.mode_exclude : t.mode_lock;
}

export function stateText(state: RestrictionState): string {
  const t = universeText().restrictions;
  return { active: t.state_active, scheduled: t.state_scheduled, expired: t.state_expired, revoked: t.state_revoked }[state];
}

export function sectorNameOf(view: UniverseView | undefined, id: string): string {
  const sector = view?.sectors.find((s) => s.id === id);
  return sector ? pickText(sector, "name") : id;
}

export function assetOf(view: UniverseView | undefined, symbol: string): Asset | undefined {
  return view?.assets.find((a) => a.symbol === symbol) ?? view?.removed.find((a) => a.symbol === symbol);
}

/** Secondary-language name, shown muted under the primary one. */
export function otherName(asset: Asset, locale: "zh" | "en"): string {
  const other = locale === "zh" ? asset.name_en : asset.name_zh;
  const primary = pickText(asset, "name");
  return other && other !== primary ? other : "";
}
