import { For, type JSX, Show, createSignal } from "solid-js";
import { Icon } from "@/components/Icon";
import { ErrorState, confirmAction, toast, toastError } from "@/components/ui";
import { tpl } from "@/i18n";
import { universeText } from "@/i18n/universe";
import { api } from "@/lib/api";
import { DASH, fmtDateTime, fmtQty } from "@/lib/format";
import { isAdmin } from "@/lib/session";
import type { UniverseChanges, UniverseVersion, UniverseView } from "@/lib/types";
import { type Weights, assetOf, pickText, sectorNameOf } from "./helpers";
import { actorName } from "@/lib/names";

interface CheckSource {
  kind?: string;
  location?: string;
  ontology_schema_version?: number | null;
  ontology_generated_at?: string | null;
  edits_applied?: number;
  built_at?: string;
}

interface CheckResult {
  changes: UniverseChanges;
  source: CheckSource;
  sectors: number;
  assets: number;
}

function isEmpty(c: UniverseChanges): boolean {
  return !c.added.length && !c.removed.length && !c.moved.length && !c.sectors_added.length && !c.sectors_removed.length;
}

function hostOf(location: string): string {
  try {
    const url = new URL(location);
    return url.host + url.pathname.replace(/\/[^/]*$/, "/…");
  } catch {
    return location;
  }
}

export function ChangeChips(props: { changes: UniverseChanges; emptyLabel?: string }) {
  const t = universeText().updates;
  const c = () => props.changes;
  return (
    <div class="pill-list">
      <Show when={c().first_load}>
        <span class="chip blue">{tpl(t.first_load, { n: c().added.length })}</span>
      </Show>
      <Show when={!c().first_load && c().added.length}>
        <span class="chip green">{tpl(t.chip_added, { n: c().added.length })}</span>
      </Show>
      <Show when={c().removed.length}>
        <span class="chip red">{tpl(t.chip_removed, { n: c().removed.length })}</span>
      </Show>
      <Show when={c().moved.length}>
        <span class="chip yellow">{tpl(t.chip_moved, { n: c().moved.length })}</span>
      </Show>
      <Show when={!c().first_load && (c().sectors_added.length || c().sectors_removed.length)}>
        <span class="chip outline">{tpl(t.chip_sectors, { a: c().sectors_added.length, r: c().sectors_removed.length })}</span>
      </Show>
      <Show when={isEmpty(c()) && !c().first_load}>
        <span class="chip">{props.emptyLabel ?? t.no_changes}</span>
      </Show>
    </div>
  );
}

function DiffBlock(props: { title: string; count: number; children: JSX.Element }) {
  return (
    <Show when={props.count}>
      <div class="uv-diff-block">
        <div class="uv-diff-title">
          {props.title} <span class="num muted">{props.count}</span>
        </div>
        {props.children}
      </div>
    </Show>
  );
}

export function CheckPanel(props: { view: UniverseView; weights: Weights; onApplied: () => void }) {
  const t = universeText;
  const [ontology, setOntology] = createSignal("");
  const [edits, setEdits] = createSignal("");
  const [advanced, setAdvanced] = createSignal(false);
  const [checking, setChecking] = createSignal(false);
  const [applying, setApplying] = createSignal(false);
  const [result, setResult] = createSignal<CheckResult | null>(null);
  const [error, setError] = createSignal<unknown>(null);

  const current = () => props.view.versions[0];
  const location = () => (advanced() ? ontology().trim() || undefined : undefined);
  const editLog = () => (advanced() ? edits().trim() || undefined : undefined);

  const check = async () => {
    setChecking(true);
    setError(null);
    try {
      const res = (await api.universeCheck(location(), editLog())) as CheckResult;
      setResult(res);
    } catch (e) {
      setResult(null);
      setError(e);
    } finally {
      setChecking(false);
    }
  };

  const newerContent = () => {
    const r = result();
    if (!r) return false;
    const cur = current();
    return !cur || r.source.ontology_generated_at !== cur.ontology_generated_at || r.source.ontology_schema_version !== cur.ontology_schema_version || (r.source.edits_applied ?? 0) > 0;
  };
  const canApply = () => {
    const r = result();
    return !!r && isAdmin() && (!isEmpty(r.changes) || newerContent());
  };

  const apply = async () => {
    const r = result();
    if (!r) return;
    const u = t().updates;
    const lines = [u.apply_intro];
    if (r.changes.added.length) lines.push(tpl(u.apply_added, { n: r.changes.added.length }));
    if (r.changes.removed.length) {
      lines.push(tpl(u.apply_removed, { n: r.changes.removed.length }));
      const held = r.changes.removed
        .map((s) => ({ s, p: props.weights.position(s) }))
        .filter((x) => x.p)
        .map((x) => `${x.s} (${fmtQty(x.p!.qty)})`);
      if (held.length) lines.push(tpl(u.apply_removed_held, { list: held.join(", ") }));
    }
    if (r.changes.moved.length) lines.push(tpl(u.apply_moved, { n: r.changes.moved.length }));
    if (r.changes.sectors_added.length || r.changes.sectors_removed.length) {
      lines.push(tpl(u.apply_sectors, { added: r.changes.sectors_added.length, removed: r.changes.sectors_removed.length }));
    }
    lines.push(u.apply_sync, "", u.apply_outro);
    const ok = await confirmAction({ title: u.apply_title, body: lines.join("\n"), confirmLabel: u.apply_confirm, danger: r.changes.removed.length > 0 });
    if (ok === null) return;
    setApplying(true);
    try {
      const res = await api.universeApply(location(), editLog());
      toast(isEmpty(res.changes) ? u.applied_none : u.applied, undefined, "success");
      setResult(null);
      props.onApplied();
    } catch (e) {
      toastError(e);
    } finally {
      setApplying(false);
    }
  };

  const nameOf = (symbol: string) => {
    const a = assetOf(props.view, symbol);
    return a ? pickText(a, "name") : "";
  };

  return (
    <div class="stack" style={{ gap: "14px" }}>
      <Show when={!isAdmin()}>
        <div class="callout info">
          <Icon name="info" size={16} />
          <span>{t().updates.viewer}</span>
        </div>
      </Show>
      <div class="row wrap" style={{ gap: "10px" }}>
        <button class="btn primary" onClick={() => void check()} disabled={!isAdmin() || checking()}>
          <Show when={checking()} fallback={<Icon name="refresh" size={14} />}>
            <span class="spinner uv-spinner-sm" />
          </Show>
          {checking() ? t().updates.checking : t().updates.check}
        </button>
        <button class="btn ghost sm" onClick={() => setAdvanced(!advanced())} aria-expanded={advanced()} disabled={!isAdmin()}>
          <Icon name={advanced() ? "chevron_down" : "chevron_right"} size={13} /> {t().updates.advanced}
        </button>
      </div>
      <Show when={advanced()}>
        <div class="uv-advanced">
          <div class="field">
            <label for="uv-ontology">{t().updates.ontology}</label>
            <input id="uv-ontology" class="input mono" placeholder={props.view.ontology_url} value={ontology()} onInput={(e) => setOntology(e.currentTarget.value)} spellcheck={false} />
            <span class="hint">{t().updates.ontology_hint}</span>
          </div>
          <div class="field">
            <label for="uv-edits">{t().updates.edits}</label>
            <input id="uv-edits" class="input mono" value={edits()} onInput={(e) => setEdits(e.currentTarget.value)} spellcheck={false} />
            <span class="hint">{t().updates.edits_hint}</span>
          </div>
        </div>
      </Show>

      <Show when={error()}>
        <ErrorState error={error()} onRetry={() => void check()} />
      </Show>

      <Show when={result()}>
        {(r) => (
          <div class="uv-result">
            <div class="row wrap" style={{ gap: "8px 12px" }}>
              <h3>{t().updates.result}</h3>
              <ChangeChips changes={r().changes} emptyLabel={t().updates.no_membership_changes} />
            </div>
            <p class="xs muted">
              {tpl(t().updates.result_meta, {
                schema: r().source.ontology_schema_version != null ? `v${r().source.ontology_schema_version}` : DASH,
                generated: r().source.ontology_generated_at ?? DASH,
                edits: r().source.edits_applied ?? 0,
                sectors: r().sectors,
                assets: r().assets,
              })}
            </p>
            <Show when={isEmpty(r().changes)}>
              <div class={`callout ${newerContent() ? "info" : "ok"}`}>
                <Icon name={newerContent() ? "info" : "check"} size={16} />
                <span>{newerContent() ? t().updates.newer : t().updates.up_to_date}</span>
              </div>
            </Show>

            <DiffBlock title={t().updates.added} count={r().changes.added.length}>
              <div class="pill-list">
                <For each={r().changes.added}>
                  {(s) => (
                    <span class="param">
                      <b class="mono">{s}</b>
                      <span class="muted">{nameOf(s) || t().updates.not_in_db}</span>
                    </span>
                  )}
                </For>
              </div>
            </DiffBlock>
            <DiffBlock title={t().updates.removed} count={r().changes.removed.length}>
              <ul class="uv-diff-list">
                <For each={r().changes.removed}>
                  {(s) => (
                    <li>
                      <span class="ticker">{s}</span>
                      <span>{nameOf(s)}</span>
                      <span class="spacer" />
                      <Show when={props.weights.position(s)}>
                        {(p) => <span class="chip orange">{tpl(t().updates.held, { qty: fmtQty(p().qty) })}</span>}
                      </Show>
                    </li>
                  )}
                </For>
              </ul>
            </DiffBlock>
            <DiffBlock title={t().updates.moved} count={r().changes.moved.length}>
              <ul class="uv-diff-list">
                <For each={r().changes.moved}>
                  {([s, from, to]) => (
                    <li>
                      <span class="ticker">{s}</span>
                      <span>{nameOf(s)}</span>
                      <span class="spacer" />
                      <span class="small">
                        {sectorNameOf(props.view, from)} <Icon name="chevron_right" size={12} /> <b>{sectorNameOf(props.view, to)}</b>
                      </span>
                    </li>
                  )}
                </For>
              </ul>
            </DiffBlock>
            <DiffBlock title={t().updates.sectors_added} count={r().changes.sectors_added.length}>
              <div class="pill-list">
                <For each={r().changes.sectors_added}>{(id) => <span class="chip green">{sectorNameOf(props.view, id)}</span>}</For>
              </div>
            </DiffBlock>
            <DiffBlock title={t().updates.sectors_removed} count={r().changes.sectors_removed.length}>
              <div class="pill-list">
                <For each={r().changes.sectors_removed}>{(id) => <span class="chip red">{sectorNameOf(props.view, id)}</span>}</For>
              </div>
            </DiffBlock>

            <Show when={canApply()}>
              <div class="row" style={{ "justify-content": "flex-end" }}>
                <button class={`btn ${r().changes.removed.length ? "danger" : "primary"}`} onClick={() => void apply()} disabled={applying()}>
                  {t().actions.apply}
                </button>
              </div>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}

export function VersionsTable(props: { versions: UniverseVersion[] }) {
  const t = universeText;
  return (
    <div class="table-wrap">
      <table class="table compact">
        <thead>
          <tr>
            <th>{t().updates.h_version}</th>
            <th>{t().updates.h_applied}</th>
            <th>{t().updates.h_source}</th>
            <th>{t().updates.h_changes}</th>
            <th>{t().updates.hash}</th>
          </tr>
        </thead>
        <tbody>
          <For each={props.versions}>
            {(v) => (
              <tr>
                <td class="num nowrap">#{v.id}</td>
                <td class="nowrap">
                  <div class="small num">{fmtDateTime(v.applied_at)}</div>
                  <div class="xs muted">{actorName(v.applied_by)}</div>
                </td>
                <td>
                  <div class="small">
                    {v.ontology_schema_version != null ? `v${v.ontology_schema_version}` : DASH} · {v.ontology_generated_at ?? DASH}
                  </div>
                  <div class="xs muted uv-location" title={v.source}>
                    {hostOf(v.source)}
                  </div>
                </td>
                <td>
                  <ChangeChips changes={v.changes} />
                </td>
                <td class="mono xs muted" title={v.content_hash}>
                  {v.content_hash.slice(0, 10)}
                </td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  );
}
