import { For, type JSX, Show, createMemo, createSignal } from "solid-js";
import { Icon } from "@/components/Icon";
import { Empty, Segmented, WeightBar } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { universeText } from "@/i18n/universe";
import { DASH, fmtDual, fmtMoney, fmtQty, fmtWeight } from "@/lib/format";
import { canTrade } from "@/lib/portfolio";
import type { Asset, Restriction, UniverseView } from "@/lib/types";
import { type SectorRow, type Weights, modeText, otherName, pickText, sectorNameOf } from "./helpers";

export function RestrictionChip(props: { restriction: Restriction | undefined }) {
  return (
    <Show when={props.restriction} fallback={<span class="muted">{DASH}</span>}>
      {(r) => (
        <span class={`chip ${r().mode === "exclude" ? "red" : "yellow"}`} title={r().reason}>
          <Icon name={r().mode === "exclude" ? "ban" : "lock"} size={11} />
          {modeText(r().mode)}
        </span>
      )}
    </Show>
  );
}

/** Sector rows with the current weight as a bar and the latest plan's target as a tick. */
export function SectorsOverview(props: { rows: SectorRow[]; weights: Weights; selected: string; onSelect: (id: string) => void }) {
  const t = universeText;
  const max = createMemo(() => Math.max(0.05, ...props.rows.map((r) => Math.max(r.current, r.target ?? 0))) * 1.08);
  const pct = (v: number) => `${Math.min(100, (v / max()) * 100)}%`;
  const invested = () => props.rows.reduce((a, r) => a + r.current, 0);
  const targetInvested = () => (props.weights.hasTargets ? props.rows.reduce((a, r) => a + (r.target ?? 0), 0) : null);
  return (
    <div class="uv-sectors">
      <div class="uv-legend" aria-hidden="true">
        <span class="uv-key">
          <span class="uv-key-bar" />
          {t().sectors.legend_current}
        </span>
        <Show when={props.weights.hasTargets}>
          <span class="uv-key">
            <span class="uv-key-tick" />
            {t().sectors.legend_target}
          </span>
        </Show>
        <span class="spacer" />
        <span class="xs muted">
          <Show when={props.weights.planId !== null} fallback={t().sectors.no_target}>
            {tpl(t().sectors.target_from, { id: props.weights.planId ?? "", time: fmtDual(props.weights.planAt, true) })}
          </Show>
        </span>
      </div>
      <div class="uv-sector-list" role="list">
        <For each={props.rows}>
          {(row) => {
            const selected = () => props.selected === row.sector.id;
            return (
              <button
                type="button"
                role="listitem"
                class="uv-sector"
                classList={{ selected: selected() }}
                aria-pressed={selected()}
                onClick={() => props.onSelect(selected() ? "" : row.sector.id)}
                title={t().sectors.filter_hint}
              >
                <span class="uv-sector-text">
                  <span class="uv-sector-name">
                    <b>{pickText(row.sector, "name")}</b>
                    <span class="muted xs num">{tpl(t().sectors.members, { n: row.members.length })}</span>
                  </span>
                  <span class="uv-sector-summary" title={pickText(row.sector, "summary")}>
                    {pickText(row.sector, "summary")}
                  </span>
                </span>
                <span class="uv-bar" aria-hidden="true">
                  <span class="fill" style={{ width: pct(row.current) }} />
                  <Show when={row.target !== null}>
                    <span class="tick" style={{ left: `calc(${pct(row.target ?? 0)} - 1px)` }} />
                  </Show>
                </span>
                <span class="uv-sector-nums num">
                  <b>{fmtWeight(row.current, 1)}</b>
                  <Show when={row.target !== null}>
                    <span class="muted"> → {fmtWeight(row.target, 1)}</span>
                  </Show>
                </span>
              </button>
            );
          }}
        </For>
      </div>
      <div class="uv-sector-foot xs">
        <span>
          {t().sectors.invested} <b class="num">{fmtWeight(invested(), 1)}</b>
          <Show when={targetInvested() !== null}>
            <span class="muted"> → {fmtWeight(targetInvested(), 1)}</span>
          </Show>
        </span>
        <span>
          {t().sectors.cash} <b class="num">{fmtWeight(props.weights.cash, 1)}</b>
          <Show when={props.weights.targetCash !== null}>
            <span class="muted"> → {fmtWeight(props.weights.targetCash, 1)}</span>
          </Show>
        </span>
      </div>
    </div>
  );
}

type SortKey = "sector" | "symbol" | "current" | "target";
type ViewMode = "all" | "held" | "restricted";

export function CompaniesTable(props: {
  view: UniverseView;
  weights: Weights;
  restrictions: Restriction[];
  sectorFilter: string;
  onSectorFilter: (id: string) => void;
  focus: string | null;
  onClearFocus: () => void;
  onRestrict: (symbol: string) => void;
}) {
  const t = universeText;
  const [search, setSearch] = createSignal("");
  const [mode, setMode] = createSignal<ViewMode>("all");
  const [sort, setSort] = createSignal<{ key: SortKey; dir: 1 | -1 }>({ key: "sector", dir: 1 });
  const [expanded, setExpanded] = createSignal<Set<string>>(new Set());

  const restrictionOf = (symbol: string) => props.restrictions.find((r) => r.symbol === symbol);
  const toggle = (symbol: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(symbol)) next.delete(symbol);
      else next.add(symbol);
      return next;
    });
  const isExpanded = (symbol: string) => expanded().has(symbol) || props.focus === symbol;

  const filtered = createMemo<Asset[]>(() => {
    const focus = props.focus;
    if (focus) return props.view.assets.filter((a) => a.symbol === focus);
    const q = search().trim().toLowerCase();
    return props.view.assets.filter((a) => {
      if (props.sectorFilter && a.sector_id !== props.sectorFilter) return false;
      if (mode() === "held" && !props.weights.position(a.symbol)) return false;
      if (mode() === "restricted" && !restrictionOf(a.symbol)) return false;
      if (!q) return true;
      return [a.symbol, a.name_zh, a.name_en, a.subtype_zh, a.subtype_en].some((v) => v.toLowerCase().includes(q));
    });
  });

  const groups = createMemo(() => {
    const s = sort();
    const rows = filtered();
    if (s.key === "sector") {
      return [...props.view.sectors]
        .sort((a, b) => a.sort_order - b.sort_order)
        .map((sector) => ({ sector, rows: rows.filter((a) => a.sector_id === sector.id).sort((a, b) => a.sort_order - b.sort_order) }))
        .filter((g) => g.rows.length);
    }
    const value = (a: Asset): number | string => {
      if (s.key === "symbol") return a.symbol;
      if (s.key === "current") return props.weights.current(a.symbol);
      return props.weights.target(a.symbol) ?? 0;
    };
    const sorted = [...rows].sort((a, b) => {
      const va = value(a);
      const vb = value(b);
      const cmp = typeof va === "string" ? va.localeCompare(vb as string) : (va as number) - (vb as number);
      return cmp * s.dir || a.symbol.localeCompare(b.symbol);
    });
    return [{ sector: null, rows: sorted }];
  });

  const grouped = () => sort().key === "sector";
  const columns = () => (grouped() ? 7 : 8);
  const maxWeight = createMemo(() =>
    Math.max(0.05, ...props.view.assets.map((a) => Math.max(props.weights.current(a.symbol), props.weights.target(a.symbol) ?? 0))),
  );

  const header = (key: SortKey, label: string, align: "l" | "r" = "l", extra = ""): JSX.Element => {
    const active = () => sort().key === key;
    const onClick = () => {
      if (key === "sector") setSort({ key, dir: 1 });
      else setSort(active() ? { key, dir: (sort().dir * -1) as 1 | -1 } : { key, dir: key === "symbol" ? 1 : -1 });
    };
    return (
      <th class={`sortable ${align === "r" ? "r" : ""} ${extra}`} onClick={onClick} aria-sort={active() ? (sort().dir === 1 ? "ascending" : "descending") : "none"}>
        <span class="uv-th">
          {label}
          <Show when={active() && key !== "sector"}>
            <Icon name={sort().dir === 1 ? "arrow_up" : "arrow_down"} size={11} />
          </Show>
        </span>
      </th>
    );
  };

  const sectorTotals = (rows: Asset[]) => {
    const current = rows.reduce((a, x) => a + props.weights.current(x.symbol), 0);
    const target = props.weights.hasTargets ? rows.reduce((a, x) => a + (props.weights.target(x.symbol) ?? 0), 0) : null;
    return { current, target };
  };

  return (
    <div class="stack" style={{ gap: "12px" }}>
      <div class="uv-filters">
        <Show
          when={!props.focus}
          fallback={
            <span class="chip blue uv-focus-chip">
              <Icon name="filter" size={11} />
              {tpl(t().companies.focus, { symbol: props.focus! })}
              <button type="button" onClick={() => props.onClearFocus()} aria-label={t().actions.clear}>
                <Icon name="x" size={11} />
              </button>
            </span>
          }
        >
          <select class="select uv-select" value={props.sectorFilter} onChange={(e) => props.onSectorFilter(e.currentTarget.value)} aria-label={t().companies.h_sector}>
            <option value="">{t().companies.all_sectors}</option>
            <For each={[...props.view.sectors].sort((a, b) => a.sort_order - b.sort_order)}>{(s) => <option value={s.id}>{pickText(s, "name")}</option>}</For>
          </select>
          <label class="uv-search">
            <Icon name="search" size={14} />
            <input class="input" type="search" placeholder={t().companies.search} value={search()} onInput={(e) => setSearch(e.currentTarget.value)} aria-label={t().companies.search} />
          </label>
          <Segmented
            value={mode()}
            onChange={setMode}
            options={[
              { value: "all", label: t().companies.view_all },
              { value: "held", label: t().companies.view_held },
              { value: "restricted", label: t().companies.view_restricted },
            ]}
          />
        </Show>
        <span class="spacer" />
        <span class="xs muted">{tpl(t().companies.count, { n: filtered().length, total: props.view.assets.length })}</span>
      </div>

      <Show when={filtered().length} fallback={<Empty title={props.focus ? tpl(t().companies.focus_missing, { symbol: props.focus }) : t().companies.none} icon="search" />}>
        <div class="table-wrap uv-table-wrap">
          <table class="table uv-companies">
            <thead>
              <tr>
                {header("symbol", t().companies.h_company)}
                <Show when={!grouped()}>{header("sector", t().companies.h_sector)}</Show>
                <th class="uv-hide-sm">{t().companies.h_subtype}</th>
                <th class="uv-hide-sm">{t().companies.h_also}</th>
                {header("current", t().companies.h_current, "r", "uv-hide-sm")}
                {header("target", t().companies.h_target)}
                <th class="uv-hide-sm">{t().companies.h_restriction}</th>
                <th class="uv-chev-col" aria-hidden="true" />
              </tr>
            </thead>
            <For each={groups()}>
              {(group) => (
                <tbody>
                  <Show when={group.sector}>
                    {(sector) => {
                      const totals = () => sectorTotals(group.rows);
                      const full = () => props.view.assets.filter((a) => a.sector_id === sector().id).length;
                      return (
                        <tr class="uv-group-row">
                          <td colspan={columns()}>
                            <div class="uv-group-head">
                              <b>{pickText(sector(), "name")}</b>
                              <span class="muted xs num">
                                {group.rows.length === full() ? tpl(t().sectors.members, { n: full() }) : `${group.rows.length} / ${full()}`}
                              </span>
                              <span class="spacer" />
                              <span class="xs num">
                                {t().sectors.legend_current} <b>{fmtWeight(totals().current, 1)}</b>
                                <Show when={totals().target !== null}>
                                  <span class="muted">
                                    {" "}
                                    · {t().sectors.legend_target} {fmtWeight(totals().target, 1)}
                                  </span>
                                </Show>
                              </span>
                            </div>
                            <div class="uv-group-summary">{pickText(sector(), "summary")}</div>
                          </td>
                        </tr>
                      );
                    }}
                  </Show>
                  <For each={group.rows}>
                    {(asset) => {
                      const restriction = () => restrictionOf(asset.symbol);
                      const position = () => props.weights.position(asset.symbol);
                      return (
                        <>
                          <tr
                            class="uv-row clickable"
                            classList={{ expanded: isExpanded(asset.symbol), focused: props.focus === asset.symbol }}
                            onClick={() => toggle(asset.symbol)}
                            tabindex={0}
                            onKeyDown={(e) => {
                              if (e.key === "Enter" || e.key === " ") {
                                e.preventDefault();
                                toggle(asset.symbol);
                              }
                            }}
                            aria-expanded={isExpanded(asset.symbol)}
                          >
                            <td>
                              <div class="uv-name">
                                <span class="ticker">{asset.symbol}</span>
                                <span class="uv-names" title={[pickText(asset, "name"), otherName(asset, locale())].filter(Boolean).join(" · ")}>
                                  <span>{pickText(asset, "name")}</span>
                                  <Show when={otherName(asset, locale())}>
                                    <span class="muted"> · {otherName(asset, locale())}</span>
                                  </Show>
                                </span>
                                <Show when={restriction()}>
                                  <span class="uv-sm-only">
                                    <RestrictionChip restriction={restriction()} />
                                  </span>
                                </Show>
                              </div>
                            </td>
                            <Show when={!grouped()}>
                              <td class="nowrap small">{sectorNameOf(props.view, asset.sector_id)}</td>
                            </Show>
                            <td class="small subtle uv-hide-sm">
                              <span class="uv-subtype" title={pickText(asset, "subtype")}>
                                {pickText(asset, "subtype") || DASH}
                              </span>
                            </td>
                            <td class="uv-hide-sm">
                              <Show when={asset.also_in.length} fallback={<span class="muted">{DASH}</span>}>
                                <div class="uv-also">
                                  <For each={asset.also_in}>
                                    {(id) => (
                                      <span class="chip outline" title={sectorNameOf(props.view, id)}>
                                        {sectorNameOf(props.view, id)}
                                      </span>
                                    )}
                                  </For>
                                </div>
                              </Show>
                            </td>
                            <td class="r num uv-hide-sm">{position() ? fmtWeight(props.weights.current(asset.symbol), 2) : <span class="muted">{DASH}</span>}</td>
                            <td>
                              <div class="uv-target">
                                <WeightBar weight={props.weights.current(asset.symbol)} target={props.weights.target(asset.symbol)} max={maxWeight()} />
                                <span class="num uv-hide-sm">{props.weights.hasTargets ? fmtWeight(props.weights.target(asset.symbol), 2) : DASH}</span>
                                <span class="num uv-sm-only uv-sm-weights">
                                  <span class="muted">{`${fmtWeight(props.weights.current(asset.symbol), 1)} → `}</span>
                                  {props.weights.hasTargets ? fmtWeight(props.weights.target(asset.symbol), 1) : DASH}
                                </span>
                              </div>
                            </td>
                            <td class="uv-hide-sm">
                              <RestrictionChip restriction={restriction()} />
                            </td>
                            <td class="uv-chev-col">
                              <Icon name="chevron_down" size={14} class="uv-chev" />
                            </td>
                          </tr>
                          <Show when={isExpanded(asset.symbol)}>
                            <tr class="uv-detail-row">
                              <td colspan={columns()}>
                                <div class="uv-detail">
                                  <div class="uv-role">
                                    <div class="uv-label">{t().companies.role_title}</div>
                                    <Show when={asset.role_zh} fallback={<p class="muted small">{t().companies.role_empty}</p>}>
                                      <p lang="zh-CN">{asset.role_zh}</p>
                                    </Show>
                                    <p class="xs muted">{t().companies.role_note}</p>
                                  </div>
                                  <div class="uv-detail-side">
                                    <dl class="kv uv-detail-kv">
                                      <dt>{t().companies.h_sector}</dt>
                                      <dd>{sectorNameOf(props.view, asset.sector_id)}</dd>
                                      <dt>{t().companies.h_subtype}</dt>
                                      <dd>{pickText(asset, "subtype") || DASH}</dd>
                                      <Show when={asset.also_in.length}>
                                        <dt>{t().companies.h_also}</dt>
                                        <dd>{asset.also_in.map((id) => sectorNameOf(props.view, id)).join(locale() === "zh" ? "、" : ", ")}</dd>
                                      </Show>
                                    </dl>
                                    <div class="small">
                                      <Show when={position()} fallback={<span class="muted">{t().companies.not_held}</span>}>
                                        {(p) => tpl(t().companies.held, { qty: fmtQty(p().qty), value: fmtMoney(p().value, { dp: 0 }) })}
                                      </Show>
                                    </div>
                                    <Show when={restriction()}>
                                      {(r) => (
                                        <div class="small">
                                          <RestrictionChip restriction={r()} /> <span class="subtle">{r().reason}</span>
                                        </div>
                                      )}
                                    </Show>
                                    <Show when={canTrade() && !restriction()}>
                                      <div>
                                        <button
                                          class="btn sm"
                                          onClick={(e) => {
                                            e.stopPropagation();
                                            props.onRestrict(asset.symbol);
                                          }}
                                        >
                                          <Icon name="ban" size={13} /> {t().companies.restrict}
                                        </button>
                                      </div>
                                    </Show>
                                  </div>
                                </div>
                              </td>
                            </tr>
                          </Show>
                        </>
                      );
                    }}
                  </For>
                </tbody>
              )}
            </For>
          </table>
        </div>
        <p class="xs muted">{t().companies.sort_hint}</p>
      </Show>
    </div>
  );
}
