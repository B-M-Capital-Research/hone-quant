/**
 * Display names for system values: built-in strategy presets (a version saved under a preset's
 * name is shown in the UI language) and non-human actors in the audit trail.
 */
import { locale } from "@/i18n";

const PRESET_NAMES: Record<string, { zh: string; en: string }> = {
  sector_risk_budget: { zh: "板块优先 · 风险预算", en: "Sector-first risk budget" },
  equal_weight: { zh: "等权基准", en: "Equal-weight baseline" },
  momentum_rotation: { zh: "动量轮动", en: "Momentum rotation" },
  defensive_low_vol: { zh: "低波防御", en: "Defensive low volatility" },
};

/**
 * A strategy version's name; versions still named after their preset follow the UI language.
 * Without a preset id (e.g. an activation record) any preset's name is recognised.
 */
export function strategyName(version: { name: string; preset_id?: string | null } | null | undefined): string {
  if (!version) return "";
  const candidates = version.preset_id ? [PRESET_NAMES[version.preset_id]].filter(Boolean) : Object.values(PRESET_NAMES);
  const preset = candidates.find((p) => version.name === p.en || version.name === p.zh);
  return preset ? preset[locale()] : version.name;
}

const SYSTEM_NOTES: Record<string, { zh: string; en: string }> = {
  "default strategy": { zh: "默认策略", en: "Default strategy" },
  "initial activation": { zh: "首次启用", en: "Initial activation" },
};

/** Notes written by the system (not by an operator) in the UI language. */
export function systemNote(note: string | null | undefined): string {
  if (!note) return "";
  return SYSTEM_NOTES[note]?.[locale()] ?? note;
}

const ACTORS: Record<string, { zh: string; en: string }> = {
  scheduler: { zh: "自动调度", en: "Scheduler" },
  system: { zh: "系统", en: "System" },
};

/** "scheduler" → 自动调度 / Scheduler; user names are returned unchanged. */
export function actorName(actor: string | null | undefined): string {
  if (!actor) return "";
  return ACTORS[actor]?.[locale()] ?? actor;
}
