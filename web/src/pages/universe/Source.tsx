import { A } from "@solidjs/router";
import { For, Show } from "solid-js";
import { Icon } from "@/components/Icon";
import { tpl } from "@/i18n";
import { universeText } from "@/i18n/universe";
import { DASH, fmtDateTime, fmtQty } from "@/lib/format";
import type { BenchmarkSettings, UniverseView } from "@/lib/types";
import { type Weights, pickText, sectorNameOf } from "./helpers";
import { actorName } from "@/lib/names";

function humanSource(location: string, fallback: string): string {
  return /^https?:\/\//.test(location) ? location : fallback;
}

/** Where the universe comes from, and why membership is not a recommendation. */
export function SourceCard(props: { view: UniverseView }) {
  const t = universeText;
  const src = () => props.view.bundled_source;
  const latest = () => props.view.versions[0];
  return (
    <section class="card">
      <div class="card-head">
        <div style={{ flex: 1, "min-width": 0 }}>
          <h2>{t().source.title}</h2>
          <div class="sub">
            {t().source.name} · {tpl(t().source.counts, { sectors: props.view.sectors.length, assets: props.view.assets.length })}
          </div>
        </div>
      </div>
      <div class="card-body stack" style={{ gap: "14px" }}>
        <ul class="uv-principles">
          <li>
            <Icon name="layers" size={15} />
            <span>{t().source.fixed}</span>
          </li>
          <li>
            <Icon name="shield" size={15} />
            <span>{t().source.not_advice}</span>
          </li>
        </ul>
        <dl class="kv uv-source-kv">
          <dt>{t().source.schema}</dt>
          <dd>{src().ontology_schema_version != null ? `v${src().ontology_schema_version}` : DASH}</dd>
          <dt>{t().source.generated}</dt>
          <dd>{src().ontology_generated_at ?? DASH}</dd>
          <dt>{t().source.edits}</dt>
          <dd>{src().edits_applied}</dd>
          <dt>{t().source.built}</dt>
          <dd>{src().built_at}</dd>
          <Show when={latest()}>
            {(v) => (
              <>
                <dt>{t().source.current}</dt>
                <dd>{tpl(t().source.current_value, { id: v().id, time: fmtDateTime(v().applied_at), by: actorName(v().applied_by) })}</dd>
              </>
            )}
          </Show>
        </dl>
        <div class="row wrap" style={{ gap: "8px" }}>
          <a class="btn sm" href={humanSource(src().location, props.view.ontology_url)} target="_blank" rel="noopener noreferrer">
            <Icon name="external" size={13} /> {t().source.view_source}
          </a>
          <a class="btn sm ghost" href={props.view.ontology_url} target="_blank" rel="noopener noreferrer">
            {t().source.raw}
          </a>
        </div>
      </div>
    </section>
  );
}

export function BenchmarksCard(props: { view: UniverseView; settings: BenchmarkSettings | null }) {
  const t = universeText;
  const rows = () => {
    const known = props.view.benchmarks.map((b) => ({ symbol: b.symbol, name: pickText(b, "name") }));
    for (const symbol of props.settings?.symbols ?? []) {
      if (!known.some((k) => k.symbol === symbol)) known.push({ symbol, name: "" });
    }
    return known;
  };
  return (
    <section class="card">
      <div class="card-head">
        <div style={{ flex: 1, "min-width": 0 }}>
          <h2>{t().benchmarks.title}</h2>
          <div class="sub">{t().benchmarks.sub}</div>
        </div>
      </div>
      <div class="card-body flush">
        <ul class="uv-bench">
          <For each={rows()}>
            {(b) => (
              <li>
                <span class="ticker">{b.symbol}</span>
                <span class="uv-bench-name">{b.name || DASH}</span>
                <span class="spacer" />
                <Show when={props.settings?.primary === b.symbol}>
                  <span class="chip blue">{t().benchmarks.primary}</span>
                </Show>
                <Show when={props.settings?.symbols.includes(b.symbol) && props.settings?.primary !== b.symbol}>
                  <span class="chip outline">{t().benchmarks.compared}</span>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </div>
      <div class="card-foot">
        <A href="/settings">
          <Icon name="settings" size={12} /> {t().benchmarks.settings}
        </A>
      </div>
    </section>
  );
}

export function RemovedCard(props: { view: UniverseView; weights: Weights }) {
  const t = universeText;
  return (
    <section class="card">
      <div class="card-head">
        <div style={{ flex: 1, "min-width": 0 }}>
          <h2>{t().removed.title}</h2>
          <div class="sub">{t().removed.sub}</div>
        </div>
      </div>
      <div class="card-body flush">
        <div class="table-wrap">
          <table class="table compact">
            <thead>
              <tr>
                <th>{t().companies.h_company}</th>
                <th>{t().removed.last_sector}</th>
                <th class="r">{t().companies.h_current}</th>
              </tr>
            </thead>
            <tbody>
              <For each={props.view.removed}>
                {(a) => (
                  <tr>
                    <td>
                      <div class="name-cell">
                        <span class="ticker">{a.symbol}</span>
                        <span class="name">{pickText(a, "name")}</span>
                      </div>
                    </td>
                    <td class="small">{sectorNameOf(props.view, a.sector_id)}</td>
                    <td class="r small">
                      <Show when={props.weights.position(a.symbol)} fallback={<span class="muted">{t().companies.not_held}</span>}>
                        {(p) => <span class="chip orange">{tpl(t().removed.held, { qty: fmtQty(p().qty) })}</span>}
                      </Show>
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </div>
    </section>
  );
}
