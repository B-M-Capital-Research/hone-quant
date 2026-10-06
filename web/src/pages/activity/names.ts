/**
 * Company names for symbols (GET /api/universe), loaded once per page load and grouped by
 * sector for the symbol picker. Companies removed from the universe keep their names so old
 * trades stay readable.
 */
import { createResource } from "solid-js";
import { locale } from "@/i18n";
import { api } from "@/lib/api";

export interface NameEntry {
  symbol: string;
  name_zh: string;
  name_en: string;
  sector_id: string | null;
}

export interface NameGroup {
  id: string;
  name_zh: string;
  name_en: string;
  items: NameEntry[];
}

export interface NameIndex {
  map: Map<string, NameEntry>;
  groups: NameGroup[];
}

let cache: Promise<NameIndex> | null = null;

export function loadNames(): Promise<NameIndex> {
  if (!cache) {
    cache = api
      .universe()
      .then((view) => {
        const map = new Map<string, NameEntry>();
        const sectors = [...view.sectors].sort((a, b) => a.sort_order - b.sort_order);
        const groups: NameGroup[] = sectors.map((s) => ({ id: s.id, name_zh: s.name_zh, name_en: s.name_en, items: [] }));
        const bySector = new Map(groups.map((g) => [g.id, g]));
        for (const asset of [...view.assets].sort((a, b) => a.sort_order - b.sort_order)) {
          const entry = { symbol: asset.symbol, name_zh: asset.name_zh, name_en: asset.name_en, sector_id: asset.sector_id };
          map.set(asset.symbol, entry);
          bySector.get(asset.sector_id)?.items.push(entry);
        }
        const removed: NameGroup = { id: "__removed", name_zh: "", name_en: "", items: [] };
        for (const asset of view.removed ?? []) {
          if (map.has(asset.symbol)) continue;
          const entry = { symbol: asset.symbol, name_zh: asset.name_zh, name_en: asset.name_en, sector_id: null };
          map.set(asset.symbol, entry);
          removed.items.push(entry);
        }
        for (const bench of view.benchmarks ?? []) {
          if (!map.has(bench.symbol)) map.set(bench.symbol, { symbol: bench.symbol, name_zh: bench.name_zh, name_en: bench.name_en, sector_id: null });
        }
        return { map, groups: [...groups.filter((g) => g.items.length), ...(removed.items.length ? [removed] : [])] };
      })
      .catch((error) => {
        cache = null;
        throw error;
      });
  }
  return cache;
}

/** Reactive name index; `undefined` while loading or when the universe cannot be read. */
export function useNames() {
  const [names] = createResource(async () => {
    try {
      return await loadNames();
    } catch {
      return undefined;
    }
  });
  return () => names.latest;
}

export function nameOf(index: NameIndex | undefined, symbol: string | null | undefined): string {
  if (!index || !symbol) return "";
  const entry = index.map.get(symbol);
  if (!entry) return "";
  return locale() === "zh" ? entry.name_zh || entry.name_en : entry.name_en || entry.name_zh;
}

export function groupName(group: NameGroup): string {
  return locale() === "zh" ? group.name_zh || group.name_en : group.name_en || group.name_zh;
}
