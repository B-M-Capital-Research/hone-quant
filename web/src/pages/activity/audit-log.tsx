/**
 * Audit log tab: filterable, newest-first, "load more" through the `before` cursor, expandable
 * rows with field diffs and the raw payload, CSV export of the loaded rows.
 */
import { A } from "@solidjs/router";
import { For, Match, Show, Switch, batch, createEffect, createMemo, createResource, createSignal, on, onCleanup, onMount } from "solid-js";
import { createStore, reconcile, unwrap } from "solid-js/store";
import { Icon } from "@/components/Icon";
import { Empty, ErrorState, Loading, toastError } from "@/components/ui";
import { tpl } from "@/i18n";
import { auditText } from "@/i18n/audit";
import { common } from "@/i18n/common";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { DASH, MARKET_TZ, fmtDateTime, fmtNum, zoneLabel } from "@/lib/format";
import { displayTz } from "@/lib/prefs";
import { isAdmin } from "@/lib/session";
import type { AuditEntry } from "@/lib/types";
import {
  ENTITY_TYPES,
  actionGroups,
  actionLabel,
  actorKind,
  changes,
  entityHref,
  entityLabel,
  entityTypeLabel,
  isKnownAction,
  showValue,
  summarize,
} from "./audit-format";
import { DualTime, JsonView } from "./components";
import { XIcon } from "./icons";
import { ExportButton, exportRows } from "./trades-common";
import { debounce } from "./util";

const PAGE = 100;
const FAILURES = new Set(["plan.failed", "auth.login_failed"]);

export interface AuditFilters {
  actor: string;
  action: string;
  entity_type: string;
  entity_id: string;
}

export function ActorTag(props: { actor: string }) {
  const a = auditText;
  const kind = () => actorKind(props.actor);
  return (
    <span class={`aud-actor kind-${kind()}`} title={kind() === "user" ? `${a().actor_kind.user} · ${props.actor}` : props.actor}>
      <Switch>
        <Match when={kind() === "scheduler"}>
          <Icon name="clock" size={13} />
        </Match>
        <Match when={kind() === "system"}>
          <XIcon name="cpu" size={13} />
        </Match>
        <Match when={kind() === "cli"}>
          <XIcon name="terminal" size={13} />
        </Match>
        <Match when={true}>
          <Icon name="user" size={13} />
        </Match>
      </Switch>
      <span>{kind() === "user" ? props.actor : a().actor_kind[kind()]}</span>
    </span>
  );
}

export function AuditLog(props: { filters: AuditFilters; setFilters: (patch: Partial<AuditFilters>) => void }) {
  const a = auditText;
  const [state, setState] = createStore({ items: [] as AuditEntry[] });
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<unknown>(null);
  const [hasMore, setHasMore] = createSignal(false);
  const [loadingMore, setLoadingMore] = createSignal(false);
  const [open, setOpen] = createSignal<Set<number>>(new Set());
  const [idInput, setIdInput] = createSignal(props.filters.entity_id);
  let seq = 0;

  const [users] = createResource(
    () => isAdmin(),
    (admin) => (admin ? api.users().then((r) => r.users.map((u) => u.username)).catch(() => [] as string[]) : Promise.resolve([] as string[])),
  );
  const actors = createMemo(() => {
    const names = new Set<string>(users() ?? []);
    for (const row of state.items) if (actorKind(row.actor) === "user") names.add(row.actor);
    if (props.filters.actor && actorKind(props.filters.actor) === "user") names.add(props.filters.actor);
    return [...names].sort();
  });

  const query = () => ({
    actor: props.filters.actor || undefined,
    action: props.filters.action || undefined,
    entity_type: props.filters.entity_type || undefined,
    entity_id: props.filters.entity_id || undefined,
  });

  const load = async () => {
    const id = ++seq;
    setLoading(true);
    setError(null);
    try {
      const res = await api.audit({ ...query(), limit: PAGE });
      if (id !== seq) return;
      batch(() => {
        setState("items", reconcile(res.entries, { key: "id" }));
        setHasMore(res.entries.length === PAGE);
        setOpen(new Set<number>());
      });
    } catch (e) {
      if (id === seq) setError(e);
    } finally {
      if (id === seq) setLoading(false);
    }
  };

  const refreshTop = async () => {
    const id = seq;
    try {
      const res = await api.audit({ ...query(), limit: 50 });
      if (id !== seq) return;
      const current = unwrap(state.items);
      const maxId = current.length ? current[0].id : 0;
      const incoming = res.entries.filter((row) => row.id > maxId);
      if (incoming.length) setState("items", reconcile([...incoming, ...current], { key: "id" }));
    } catch {
      /* the next event catches up */
    }
  };

  const loadMore = async () => {
    const last = state.items[state.items.length - 1];
    if (!last || loadingMore()) return;
    setLoadingMore(true);
    const id = seq;
    try {
      const res = await api.audit({ ...query(), before: last.id, limit: PAGE });
      if (id !== seq) return;
      const known = new Set(state.items.map((row) => row.id));
      batch(() => {
        setState("items", (items) => [...items, ...res.entries.filter((row) => !known.has(row.id))]);
        setHasMore(res.entries.length === PAGE);
      });
    } catch (e) {
      toastError(e);
    } finally {
      setLoadingMore(false);
    }
  };

  createEffect(on(() => [props.filters.actor, props.filters.action, props.filters.entity_type, props.filters.entity_id], () => load()));
  createEffect(on(() => props.filters.entity_id, (value) => setIdInput(value), { defer: true }));

  const commitId = debounce((value: string) => props.setFilters({ entity_id: value.trim() }), 450);

  onMount(() => {
    const bump = debounce(refreshTop, 900);
    const off = onServerEvent(["plan", "settings", "strategy", "account", "universe", "backtest", "notification"], () => bump());
    onCleanup(() => {
      off();
      bump.cancel();
      commitId.cancel();
    });
  });

  const toggle = (id: number) =>
    setOpen((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const active = () => !!(props.filters.actor || props.filters.action || props.filters.entity_type || props.filters.entity_id);

  const exportCsv = () => {
    const c = a().csv;
    const cols = a().cols;
    const header = [c.id, c.time_utc, c.local_time, cols.actor, cols.action, c.action_code, c.entity_type, c.entity_id, cols.summary, cols.ip, c.detail];
    const rows = state.items.map((row) => [
      row.id,
      row.ts,
      `${fmtDateTime(row.ts)} ${zoneLabel(displayTz())}`,
      row.actor,
      actionLabel(row.action),
      row.action,
      row.entity_type,
      row.entity_id,
      summarize(row) ?? "",
      row.ip,
      JSON.stringify(row.detail ?? {}),
    ]);
    const stamp = new Date().toISOString().slice(0, 10);
    exportRows(`hone-quant-audit-log-${stamp}.csv`, header, rows, null);
  };

  return (
    <>
      <div class="act-toolbar" role="search">
        <div class="act-field">
          <label class="act-field-label" for="audit-actor">
            {a().filter.actor}
          </label>
          <select
            id="audit-actor"
            class="select act-select"
            classList={{ set: !!props.filters.actor }}
            value={props.filters.actor}
            onChange={(e) => props.setFilters({ actor: e.currentTarget.value })}
          >
            <option value="">{a().filter.all_actors}</option>
            <option value="scheduler">{a().actor_kind.scheduler}</option>
            <option value="system">{a().actor_kind.system}</option>
            <option value="cli">{a().actor_kind.cli}</option>
            <Show when={actors().length}>
              <optgroup label={a().actor_kind.user}>
                <For each={actors()}>{(name) => <option value={name}>{name}</option>}</For>
              </optgroup>
            </Show>
          </select>
        </div>
        <div class="act-field">
          <label class="act-field-label" for="audit-action">
            {a().filter.action}
          </label>
          <select
            id="audit-action"
            class="select act-select aud-action-select"
            classList={{ set: !!props.filters.action }}
            value={props.filters.action}
            onChange={(e) => props.setFilters({ action: e.currentTarget.value })}
          >
            <option value="">{a().filter.all_actions}</option>
            <For each={actionGroups()}>
              {(group) => (
                <optgroup label={group.label}>
                  <option value={group.prefix}>{tpl(a().filter.group_all, { group: group.label })}</option>
                  <For each={group.actions}>{(code) => <option value={code}>{actionLabel(code)}</option>}</For>
                </optgroup>
              )}
            </For>
            <Show when={props.filters.action && !isKnownAction(props.filters.action) && !actionGroups().some((g) => g.prefix === props.filters.action)}>
              <option value={props.filters.action}>{props.filters.action}</option>
            </Show>
          </select>
        </div>
        <div class="act-field">
          <label class="act-field-label" for="audit-entity">
            {a().filter.entity}
          </label>
          <select
            id="audit-entity"
            class="select act-select"
            classList={{ set: !!props.filters.entity_type }}
            value={props.filters.entity_type}
            onChange={(e) => props.setFilters({ entity_type: e.currentTarget.value })}
          >
            <option value="">{a().filter.all_entities}</option>
            <For each={[...ENTITY_TYPES]}>{(type) => <option value={type}>{entityTypeLabel(type)}</option>}</For>
            <Show when={props.filters.entity_type && !(ENTITY_TYPES as readonly string[]).includes(props.filters.entity_type)}>
              <option value={props.filters.entity_type}>{props.filters.entity_type}</option>
            </Show>
          </select>
        </div>
        <div class="act-field">
          <label class="act-field-label" for="audit-entity-id">
            {a().filter.entity_id}
          </label>
          <input
            id="audit-entity-id"
            class="input aud-id-input"
            classList={{ set: !!props.filters.entity_id }}
            placeholder={a().filter.entity_id_ph}
            value={idInput()}
            autocomplete="off"
            spellcheck={false}
            onInput={(e) => {
              setIdInput(e.currentTarget.value);
              commitId(e.currentTarget.value);
            }}
          />
        </div>
        <Show when={active()}>
          <button
            type="button"
            class="btn ghost sm act-clear"
            onClick={() => {
              commitId.cancel();
              setIdInput("");
              props.setFilters({ actor: "", action: "", entity_type: "", entity_id: "" });
            }}
          >
            <Icon name="x" size={13} />
            {a().filter.clear}
          </button>
        </Show>
      </div>

      <section class="card">
        <div class="card-head">
          <div>
            <h2>{a().card}</h2>
            <div class="sub">{a().card_sub}</div>
          </div>
          <span class="spacer" />
          <ExportButton onClick={exportCsv} disabled={!state.items.length} />
        </div>
        <div class="card-body flush">
          <Switch>
            <Match when={error() && !state.items.length}>
              <ErrorState error={error()} onRetry={load} />
            </Match>
            <Match when={loading() && !state.items.length}>
              <Loading />
            </Match>
            <Match when={!state.items.length}>
              <Empty title={a().empty} icon="audit">
                <span>{a().empty_hint}</span>
              </Empty>
            </Match>
            <Match when={true}>
              <div class="table-wrap" classList={{ refetching: loading() }}>
                <table class="table compact act-table aud-table">
                  <thead>
                    <tr>
                      <th class="aud-toggle-col">
                        <span class="visually-hidden">{a().expand}</span>
                      </th>
                      <th>{a().cols.time}</th>
                      <th>{a().cols.actor}</th>
                      <th>{a().cols.action}</th>
                      <th>{a().cols.entity}</th>
                      <th>{a().cols.summary}</th>
                      <th>{a().cols.ip}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={state.items}>{(row) => <AuditRow entry={row} open={open().has(row.id)} onToggle={() => toggle(row.id)} />}</For>
                  </tbody>
                </table>
              </div>
            </Match>
          </Switch>
        </div>
        <Show when={state.items.length > 0}>
          <div class="card-foot">
            <span>{tpl(a().loaded, { n: fmtNum(state.items.length, 0) })}</span>
            <span class="spacer" />
            <Show when={hasMore()} fallback={<span>{a().end}</span>}>
              <button type="button" class="btn sm" disabled={loadingMore()} onClick={loadMore}>
                {loadingMore() ? common().states.loading : a().load_more}
              </button>
            </Show>
          </div>
        </Show>
      </section>
    </>
  );
}

function AuditRow(props: { entry: AuditEntry; open: boolean; onToggle: () => void }) {
  const a = auditText;
  const e = props.entry;
  const href = () => entityHref(e);
  const summary = () => summarize(e);
  const bad = () => FAILURES.has(e.action) || (e.action === "channel.tested" && (e.detail as Record<string, unknown>)?.ok === false);
  const diff = () => changes(e);
  const detailId = `audit-detail-${e.id}`;
  return (
    <>
      <tr
        class="aud-row clickable"
        classList={{ open: props.open }}
        onClick={(event) => {
          if ((event.target as HTMLElement).closest("a, button")) return;
          props.onToggle();
        }}
      >
        <td class="aud-toggle-col">
          <button
            type="button"
            class="aud-chevron"
            aria-expanded={props.open}
            aria-controls={detailId}
            aria-label={props.open ? a().collapse : a().expand}
            title={props.open ? a().collapse : a().expand}
            onClick={() => props.onToggle()}
          >
            <Icon name="chevron_right" size={14} />
          </button>
        </td>
        <td>
          <DualTime value={e.ts} />
        </td>
        <td>
          <ActorTag actor={e.actor} />
        </td>
        <td>
          <div class="aud-action" classList={{ bad: bad() }}>
            <span class="aud-action-label">
              <Show when={bad()}>
                <Icon name="alert" size={12} />
              </Show>
              {actionLabel(e.action)}
            </span>
            <Show when={isKnownAction(e.action)}>
              <span class="aud-code">{e.action}</span>
            </Show>
          </div>
        </td>
        <td>
          <Show when={e.entity_type || e.entity_id} fallback={<span class="muted">{DASH}</span>}>
            <Show when={href()} fallback={<span class="aud-entity">{entityLabel(e)}</span>}>
              <A class="aud-entity" href={href()!}>
                {entityLabel(e)}
              </A>
            </Show>
          </Show>
        </td>
        <td class="aud-summary-cell">
          <Show when={summary()} fallback={<span class="muted">{DASH}</span>}>
            <span class="aud-summary" title={summary()!}>
              {summary()}
            </span>
          </Show>
        </td>
        <td>
          <span class="aud-ip">{e.ip || DASH}</span>
        </td>
      </tr>
      <Show when={props.open}>
        <tr class="aud-detail" id={detailId}>
          <td colspan="7">
            <div class="aud-detail-grid">
              <dl class="aud-meta">
                <dt>{tpl(a().detail.record, { id: e.id })}</dt>
                <dd class="mono">{e.ts}</dd>
                <dt>{zoneLabel(displayTz())}</dt>
                <dd class="num">{fmtDateTime(e.ts)}</dd>
                <dt>ET</dt>
                <dd class="num">{fmtDateTime(e.ts, MARKET_TZ)}</dd>
                <dt>{a().cols.actor}</dt>
                <dd>{e.actor}</dd>
                <dt>{a().cols.action}</dt>
                <dd class="mono">{e.action}</dd>
                <dt>{a().cols.entity}</dt>
                <dd class="mono">{[e.entity_type, e.entity_id].filter(Boolean).join(" / ") || DASH}</dd>
                <dt>{a().cols.ip}</dt>
                <dd class="mono">{e.ip || DASH}</dd>
              </dl>
              <div class="aud-detail-main">
                <Show when={diff()}>
                  {(list) => (
                    <div class="aud-changes">
                      <div class="kicker">{a().detail.changes}</div>
                      <Show when={list().length} fallback={<p class="muted xs">{a().detail.no_changes}</p>}>
                        <div class="table-wrap">
                          <table class="table compact aud-diff">
                            <thead>
                              <tr>
                                <th>{a().detail.field}</th>
                                <th>{a().detail.before}</th>
                                <th>{a().detail.after}</th>
                              </tr>
                            </thead>
                            <tbody>
                              <For each={list()}>
                                {(change) => (
                                  <tr>
                                    <td class="mono">{change.path}</td>
                                    <td class="mono aud-before">{showValue(change.before)}</td>
                                    <td class="mono aud-after">{showValue(change.after)}</td>
                                  </tr>
                                )}
                              </For>
                            </tbody>
                          </table>
                        </div>
                      </Show>
                    </div>
                  )}
                </Show>
                <JsonView value={e.detail} label={a().detail.raw} />
              </div>
            </div>
          </td>
        </tr>
      </Show>
    </>
  );
}
