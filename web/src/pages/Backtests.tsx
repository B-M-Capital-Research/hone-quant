import { A, useNavigate, useSearchParams } from "@solidjs/router";
import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup, onMount, untrack } from "solid-js";
import { Icon } from "@/components/Icon";
import { Empty, ErrorState, Loading, Pct, confirmAction, toast, toastError } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { researchText } from "@/i18n/research";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { fmtDate, fmtDateTime, fmtDual, fmtPct } from "@/lib/format";
import { isAdmin, serverNow } from "@/lib/session";
import type { BacktestRow, StrategyOverview } from "@/lib/types";
import { BacktestStatusChip, CopyIcon, IndeterminateBar, MethodologyCard, ratio } from "./research/components";
import {
  benchShort,
  createLoader,
  debounce,
  estimateMs,
  failureText,
  fmtDuration,
  fmtYears,
  resolveStrategy,
  slotsLabel,
  strategyLabel,
  strategyRef,
  typicalMsPerYear,
  yearsBetween,
} from "./research/model";
import { NewBacktestDialog, type Prefill } from "./research/NewBacktestDialog";
import { actorName } from "@/lib/names";

const ACTIVE = new Set<BacktestRow["status"]>(["queued", "running"]);

export default function Backtests() {
  const r = researchText;
  const c = common;
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const list = createLoader(() => api.backtests().then((d) => d.backtests));
  const [overview, setOverview] = createSignal<StrategyOverview>();
  const [prefill, setPrefill] = createSignal<Prefill | null>(null);
  const [deleting, setDeleting] = createSignal<number | null>(null);

  const loadOverview = (fresh = false) => {
    if (fresh) strategyRef.invalidate();
    strategyRef.get().then(setOverview).catch(() => undefined);
  };

  onMount(() => {
    void list.load();
    loadOverview();
    const refresh = debounce(() => void list.load(), 200);
    const off = onServerEvent(["backtest", "strategy"], (event) => {
      if (event.type === "strategy" || event.type === "resync") loadOverview(true);
      refresh();
    });
    onCleanup(off);
  });

  const rows = () => list.data() ?? [];
  const active = createMemo(() => rows().filter((b) => ACTIVE.has(b.status)));
  const perYear = createMemo(() => typicalMsPerYear(rows()));

  // Fallback polling while something runs, in case the event stream is interrupted.
  createEffect(() => {
    if (!active().length) return;
    const timer = setInterval(() => void list.load(), 5000);
    onCleanup(() => clearInterval(timer));
  });

  // URL contract: /backtests?new=1[&version=<id>|&preset=<id>|&from=<backtest id>]
  createEffect(
    on(
      () => [params.new, params.from, params.version, params.preset, !!list.data()] as const,
      ([isNew, from, version, preset, loaded]) => {
        if (isNew !== "1") {
          setPrefill(null);
          return;
        }
        if (untrack(prefill)) return;
        if (!isAdmin()) {
          toast(c().states.no_permission, undefined, "warning");
          closeDialog();
          return;
        }
        if (from) {
          if (!loaded) return;
          const row = untrack(rows).find((b) => String(b.id) === String(from));
          setPrefill(row ? { kind: "duplicate", row } : { kind: "default" });
          return;
        }
        if (version && Number.isFinite(Number(version))) setPrefill({ kind: "version", versionId: Number(version) });
        else if (preset) setPrefill({ kind: "preset", presetId: String(preset) });
        else setPrefill({ kind: "default" });
      },
    ),
  );

  function openNew() {
    setParams({ new: "1", from: undefined, version: undefined, preset: undefined });
  }

  function duplicate(row: BacktestRow) {
    setParams({ new: "1", from: String(row.id), version: undefined, preset: undefined });
  }

  function closeDialog() {
    setParams({ new: undefined, from: undefined, version: undefined, preset: undefined }, { replace: true });
  }

  async function remove(row: BacktestRow) {
    const ok = await confirmAction({
      title: r().list.delete_title,
      body: tpl(r().list.delete_body, { name: row.name }),
      confirmLabel: c().actions.delete,
      danger: true,
    });
    if (ok === null) return;
    setDeleting(row.id);
    try {
      await api.deleteBacktest(row.id);
      toast(r().list.deleted, row.name, "success");
      await list.load();
    } catch (error) {
      toastError(error);
    } finally {
      setDeleting(null);
    }
  }

  const queuePosition = (row: BacktestRow) => rows().filter((b) => b.status === "running" || (b.status === "queued" && b.id < row.id)).length;

  /** One cell spanning the result columns while a backtest has no results. */
  const progressText = (row: BacktestRow) => {
    if (row.status === "running") {
      const elapsed = row.started_at ? serverNow() - Date.parse(row.started_at) : 0;
      const typical = estimateMs(row, perYear());
      return `${tpl(r().list.running_for, { t: fmtDuration(elapsed) })}${typical ? ` · ${tpl(r().list.typical, { t: fmtDuration(typical) })}` : ""}`;
    }
    const ahead = queuePosition(row);
    return ahead > 0 ? tpl(r().list.queued_ahead, { n: ahead }) : r().list.queued_next;
  };

  const strategyText = (row: BacktestRow) => strategyLabel(resolveStrategy(row, overview()), overview());

  const actions = (row: BacktestRow) => (
    <div class="rs-row-actions" onClick={(e) => e.stopPropagation()}>
      <Show when={isAdmin()} fallback={<Icon name="chevron_right" size={16} class="rs-row-chevron" />}>
        <button class="btn ghost icon sm" type="button" title={r().list.duplicate} aria-label={r().list.duplicate} onClick={() => duplicate(row)}>
          <CopyIcon size={15} />
        </button>
        <button
          class="btn ghost icon sm rs-danger-icon"
          type="button"
          title={r().list.delete}
          aria-label={r().list.delete}
          disabled={row.status === "running" || deleting() === row.id}
          onClick={() => remove(row)}
        >
          <Icon name="trash" size={15} />
        </button>
      </Show>
    </div>
  );

  const metrics = (row: BacktestRow) => row.summary?.metrics;

  return (
    <div class="stack rs-page">
      <div class="page-head">
        <div style={{ flex: 1, "min-width": "260px" }}>
          <h1>{r().list.title}</h1>
          <p class="lead">{r().list.lead}</p>
          <Show when={!isAdmin()}>
            <p class="muted xs rs-viewer-note">
              <Icon name="lock" size={12} /> {r().list.viewer_note}
            </p>
          </Show>
        </div>
        <Show when={isAdmin()}>
          <button class="btn primary" type="button" onClick={openNew}>
            <Icon name="plus" size={16} />
            {r().list.new}
          </button>
        </Show>
      </div>

      <Show when={active().length > 0}>
        <div class="rs-activity" role="status">
          <span class="spinner" />
          <div class="rs-activity-text">
            <b>
              {[
                active().some((b) => b.status === "running") ? tpl(r().list.activity_running, { n: active().filter((b) => b.status === "running").length }) : "",
                active().some((b) => b.status === "queued") ? tpl(r().list.activity_queued, { n: active().filter((b) => b.status === "queued").length }) : "",
              ]
                .filter(Boolean)
                .join(" · ")}
            </b>
            <span class="muted xs">{r().list.activity_hint}</span>
          </div>
        </div>
      </Show>

      <Switch>
        <Match when={list.error() && !list.data()}>
          <div class="card">
            <ErrorState error={list.error()} onRetry={() => void list.load()} />
          </div>
        </Match>
        <Match when={!list.data()}>
          <div class="card">
            <Loading />
          </div>
        </Match>
        <Match when={rows().length === 0}>
          <div class="card">
            <Empty title={r().list.empty_title} icon="backtest">
              <p class="rs-empty-body">{isAdmin() ? r().list.empty_body : r().list.empty_viewer}</p>
              <Show when={isAdmin()}>
                <button class="btn primary sm" type="button" onClick={openNew}>
                  <Icon name="plus" size={14} />
                  {r().list.new}
                </button>
              </Show>
            </Empty>
          </div>
        </Match>
        <Match when={rows().length > 0}>
          <section class="card rs-bt-list">
            <div class="card-head">
              <div class="rs-head-text">
                <h2>{r().list.runs}</h2>
                <div class="sub">{tpl(r().list.count, { n: rows().length })}</div>
              </div>
              <Show when={list.error()}>
                <span class="chip yellow" title={String((list.error() as Error)?.message ?? "")}>
                  <Icon name="alert" size={12} /> {c().states.stale}
                </span>
              </Show>
            </div>

            {/* Desktop table */}
            <div class="table-wrap rs-bt-table">
              <table class="table">
                <thead>
                  <tr>
                    <th>{r().list.col_name}</th>
                    <th>{r().list.col_status}</th>
                    <th>{r().list.col_setup}</th>
                    <th class="r">{r().list.col_total_return}</th>
                    <th class="r">{r().list.col_cagr}</th>
                    <th class="r">{r().list.col_sharpe}</th>
                    <th class="r">{r().list.col_max_dd}</th>
                    <th class="r">{r().list.col_excess}</th>
                    <th class="r">
                      <span class="visually-hidden">{r().list.col_actions}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  <For each={rows()}>
                    {(row) => (
                      <tr class="clickable" onClick={() => navigate(`/backtests/${row.id}`)}>
                        <td class="rs-name-cell">
                          <A href={`/backtests/${row.id}`} class="rs-name-link" onClick={(e) => e.stopPropagation()}>
                            {row.name}
                          </A>
                          <span class="rs-name-meta" title={fmtDual(row.created_at, true)}>
                            #{row.id} · {actorName(row.created_by)} · {fmtDate(row.created_at)}
                          </span>
                        </td>
                        <td>
                          <BacktestStatusChip status={row.status} />
                        </td>
                        <td class="rs-setup-cell" title={`${strategyText(row)} · ${slotsLabel(row.config.slots)} · ${r().frequency[row.config.frequency]}`}>
                          <span class="nowrap">
                            <span class="num">
                              {row.config.start} → {row.config.end}
                            </span>
                            <span class="muted xs"> · {tpl(r().range.years, { n: fmtYears(yearsBetween(row.config.start, row.config.end)) })}</span>
                          </span>
                          <span class="rs-setup-strategy">{strategyText(row)}</span>
                          <span class="muted xs">
                            {slotsLabel(row.config.slots)} · {r().frequency[row.config.frequency]}
                          </span>
                        </td>
                        <Show
                          when={row.status === "succeeded" && metrics(row)}
                          fallback={
                            <td colSpan={5} class="rs-progress-cell">
                              <Switch>
                                <Match when={row.status === "failed"}>
                                  <span class="rs-error-text" title={row.error ?? ""}>
                                    <Icon name="alert" size={13} /> {failureText(row.error).text}
                                  </span>
                                </Match>
                                <Match when={ACTIVE.has(row.status)}>
                                  <div class="rs-progress">
                                    <span class="muted xs">{progressText(row)}</span>
                                    <Show when={row.status === "running"}>
                                      <IndeterminateBar />
                                    </Show>
                                  </div>
                                </Match>
                              </Switch>
                            </td>
                          }
                        >
                          {(m) => (
                            <>
                              <td class="r">
                                <Pct value={m().total_return} />
                              </td>
                              <td class={`r ${m().annualisation_reliable ? "" : "rs-unreliable"}`}>
                                <Pct value={m().cagr} />
                              </td>
                              <td class={`r num ${m().annualisation_reliable ? "" : "rs-unreliable"}`}>{ratio(m().sharpe)}</td>
                              <td class="r">
                                <span class="num down">{fmtPct(m().max_drawdown, { dp: 2, sign: true })}</span>
                              </td>
                              <td class="r rs-stack-cell end">
                                <Pct value={m().excess_return} />
                                <span class="muted xs">{tpl(r().list.vs, { bench: benchShort(row.config.benchmark ?? "") })}</span>
                              </td>
                            </>
                          )}
                        </Show>
                        <td class="r">{actions(row)}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>

            {/* Narrower containers: one card per run */}
            <ul class="rs-bt-cards">
              <For each={rows()}>
                {(row) => (
                  <li class="rs-bt-card" onClick={() => navigate(`/backtests/${row.id}`)}>
                    <div class="rs-bt-card-main">
                      <div class="rs-bt-card-head">
                        <A href={`/backtests/${row.id}`} class="rs-name-link" onClick={(e) => e.stopPropagation()}>
                          {row.name}
                        </A>
                        <BacktestStatusChip status={row.status} />
                      </div>
                      <span class="muted xs">
                        #{row.id} · {actorName(row.created_by)} · {fmtDateTime(row.created_at)}
                      </span>
                    </div>
                    <div class="rs-bt-card-config">
                      <span class="num">
                        {row.config.start} → {row.config.end} · {tpl(r().range.years, { n: fmtYears(yearsBetween(row.config.start, row.config.end)) })}
                      </span>
                      <span class="muted xs">
                        {strategyText(row)} · {slotsLabel(row.config.slots)} · {r().frequency[row.config.frequency]}
                      </span>
                    </div>
                    <div class="rs-bt-card-result">
                      <Switch>
                        <Match when={row.status === "succeeded" && metrics(row)}>
                          {(m) => (
                            <dl class="rs-bt-card-metrics">
                              <div>
                                <dt>{r().list.col_total_return}</dt>
                                <dd>
                                  <Pct value={m().total_return} />
                                </dd>
                              </div>
                              <div>
                                <dt>{r().list.col_cagr}</dt>
                                <dd class={m().annualisation_reliable ? "" : "rs-unreliable"}>
                                  <Pct value={m().cagr} />
                                </dd>
                              </div>
                              <div>
                                <dt>{r().list.col_sharpe}</dt>
                                <dd class="num">{ratio(m().sharpe)}</dd>
                              </div>
                              <div>
                                <dt>{r().list.col_max_dd}</dt>
                                <dd class="num down">{fmtPct(m().max_drawdown, { dp: 2, sign: true })}</dd>
                              </div>
                            </dl>
                          )}
                        </Match>
                        <Match when={row.status === "failed"}>
                          <span class="rs-error-text">
                            <Icon name="alert" size={13} /> {failureText(row.error).text}
                          </span>
                        </Match>
                        <Match when={ACTIVE.has(row.status)}>
                          <div class="rs-progress">
                            <span class="muted xs">{progressText(row)}</span>
                            <Show when={row.status === "running"}>
                              <IndeterminateBar />
                            </Show>
                          </div>
                        </Match>
                      </Switch>
                    </div>
                    <div class="rs-bt-card-actions">{actions(row)}</div>
                  </li>
                )}
              </For>
            </ul>
          </section>
        </Match>
      </Switch>

      <MethodologyCard />

      <Show when={prefill()}>
        {(p) => (
          <NewBacktestDialog
            prefill={p()}
            onClose={closeDialog}
            onCreated={(row) => {
              setPrefill(null);
              navigate(`/backtests/${row.id}`);
            }}
          />
        )}
      </Show>
    </div>
  );
}
