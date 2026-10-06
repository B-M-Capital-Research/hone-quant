import { For, Show, createMemo } from "solid-js";
import { tpl } from "@/i18n";
import { strategyText } from "@/i18n/strategy";
import type { Sector, StrategyParams } from "@/lib/types";
import { GROUPS, diffPaths, fieldsOf, getPath } from "./params";
import { fieldText, fmtParam, keyParams, pickText } from "./format";

/** Grouped, read-only parameter tables with the default value and changed rows marked. */
export function ParamView(props: { params: StrategyParams; defaults: StrategyParams; sectors?: Sector[]; compact?: boolean }) {
  const t = strategyText;
  const changed = createMemo(() => new Set(diffPaths(props.params, props.defaults)));
  const sectorName = (id: string) => {
    const sector = props.sectors?.find((s) => s.id === id);
    return sector ? pickText(sector, "name") : id;
  };
  return (
    <div class={`st-groups ${props.compact ? "single" : ""}`}>
      <For each={GROUPS}>
        {(group) => (
          <section class="st-group">
            <header class="st-group-head">
              <h3>{t().groups[group]}</h3>
              <span class="muted xs">{t().group_hints[group]}</span>
            </header>
            <table class="table compact st-param-table">
              <thead>
                <tr>
                  <th>{t().view.parameter}</th>
                  <th class="r">{t().view.value}</th>
                  <th class="r">{t().view.default}</th>
                </tr>
              </thead>
              <tbody>
                <For each={fieldsOf(group)}>
                  {(field) => {
                    const isChanged = () => changed().has(field.path);
                    const budgets = () => Object.entries((getPath(props.params, field.path) as Record<string, number>) ?? {});
                    return (
                      <>
                        <tr classList={{ "st-changed": isChanged() }}>
                          <td>
                            <div class="st-plabel">
                              <span>{fieldText(field.path).label}</span>
                              <Show when={isChanged()}>
                                <span class="chip yellow st-mini-chip">{t().view.changed}</span>
                              </Show>
                            </div>
                            <div class="st-phint">{fieldText(field.path).hint}</div>
                          </td>
                          <td class="r nowrap st-pvalue">{fmtParam(field.path, getPath(props.params, field.path))}</td>
                          <td class="r nowrap muted">{fmtParam(field.path, getPath(props.defaults, field.path))}</td>
                        </tr>
                        <Show when={field.kind === "budgets" && budgets().length > 0}>
                          <tr class="st-subrow">
                            <td colspan={3}>
                              <div class="pill-list">
                                <For each={budgets()}>
                                  {([id, value]) => (
                                    <span class="param">
                                      {sectorName(id)} <b>{value}</b>
                                    </span>
                                  )}
                                </For>
                              </div>
                            </td>
                          </tr>
                        </Show>
                      </>
                    );
                  }}
                </For>
              </tbody>
            </table>
          </section>
        )}
      </For>
    </div>
  );
}

/** Key parameters as `.param` pills; pills whose values differ from `baseline` are marked. */
export function KeyParams(props: { params: StrategyParams; baseline?: StrategyParams }) {
  const changed = createMemo(() => (props.baseline ? new Set(diffPaths(props.params, props.baseline)) : new Set<string>()));
  return (
    <div class="pill-list">
      <For each={keyParams(props.params)}>
        {(item) => (
          <span class="param" classList={{ "st-param-changed": item.paths.some((p) => changed().has(p)) }}>
            <span class="muted">{item.label}</span>
            <b>{item.value}</b>
          </span>
        )}
      </For>
    </div>
  );
}

/** "3 项与默认值不同" / "全部为默认值". */
export function changedCountText(params: StrategyParams, defaults: StrategyParams): string {
  const n = diffPaths(params, defaults).length;
  return n ? tpl(strategyText().view.changed_count, { n }) : strategyText().view.all_default;
}
