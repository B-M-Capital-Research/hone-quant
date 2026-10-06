import { A } from "@solidjs/router";
import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { Kpi, Switch, confirmAction, toast, toastError } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { settingsText } from "@/i18n/settings";
import { ApiError, api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { fmtDual, fmtMoney, fmtNum, fmtRelative } from "@/lib/format";
import { isAdmin, market, serverNow } from "@/lib/session";
import type { Coverage, DataStatus, JobRun } from "@/lib/types";
import { Gate, SIcon, createLoader, localName, useSettings } from "./shared";

type SyncKind = "quotes" | "daily" | "full" | "corporate_actions";
type CoverageIssue = "stale" | "insufficient" | "short" | "unadjusted" | "empty";

const JOBS = ["preopen_sync", "postclose_sync", "eod_snapshot", "plan"] as const;

interface FmpCheck {
  endpoint: string;
  api: string;
  ok: boolean;
  detail: string;
}

export default function DataSection() {
  const t = settingsText;
  const status = createLoader(() => api.dataStatus());
  const universe = createLoader(() => api.universe());

  // Quote polls announce themselves; refresh the status at most every 15 seconds.
  let last = 0;
  let pending: ReturnType<typeof setTimeout> | null = null;
  const refresh = () => {
    const wait = Math.max(0, 15_000 - (Date.now() - last));
    if (pending) return;
    pending = setTimeout(() => {
      pending = null;
      last = Date.now();
      void status.reload();
    }, wait);
  };
  onMount(() => {
    const off = onServerEvent(["quotes", "account"], refresh);
    onCleanup(() => {
      off();
      if (pending) clearTimeout(pending);
    });
  });

  return (
    <Gate loader={status}>
      {(s) => (
        <>
          <Overview status={s()} />
          <Show when={s().source === "demo"}>
            <div class="callout warn">
              <SIcon name="alert" size={16} />
              <span>{t().data.demo_note}</span>
            </div>
          </Show>
          <SourceCard status={s()} />
          <JobsCard jobs={s().jobs} />
          <Show when={isAdmin()}>
            <ActionsCard onStarted={() => setTimeout(() => void status.reload(), 4000)} />
          </Show>
          <Show when={isAdmin()}>
            <FmpCard demo={s().source === "demo"} />
          </Show>
          <CoverageCard coverage={s().coverage} universe={universe.value()} />
        </>
      )}
    </Gate>
  );
}

// ---------------------------------------------------------------------------------------------
// Coverage analysis
// ---------------------------------------------------------------------------------------------

function analyse(coverage: Coverage[]): Map<string, CoverageIssue[]> {
  const newest = coverage.reduce<string | null>((max, c) => (c.last && (!max || c.last > max) ? c.last : max), null);
  const out = new Map<string, CoverageIssue[]>();
  for (const c of coverage) {
    const issues: CoverageIssue[] = [];
    if (!c.last || c.bars === 0) issues.push("empty");
    else {
      if (newest && c.last < newest) issues.push("stale");
      if (c.bars < 126) issues.push("insufficient");
      else if (c.bars < 252) issues.push("short");
      if (c.adjusted < c.bars) issues.push("unadjusted");
    }
    out.set(c.symbol, issues);
  }
  return out;
}

function Overview(props: { status: DataStatus }) {
  const t = settingsText;
  const { bundle } = useSettings();
  const issues = createMemo(() => [...analyse(props.status.coverage).values()].filter((i) => i.length > 0).length);
  const freshness = createMemo<"fresh" | "stale" | "off" | "none">(() => {
    const newest = props.status.quotes.newest;
    if (!newest) return "none";
    if (market()?.phase !== "open") return "off";
    const maxAge = (bundle.value()?.execution.max_quote_age_secs ?? 300) * 1000;
    return serverNow() - new Date(newest).getTime() <= maxAge ? "fresh" : "stale";
  });
  const freshChip = () => {
    const f = freshness();
    const d = t().data;
    return f === "fresh"
      ? { tone: "green", text: d.quote_fresh }
      : f === "stale"
        ? { tone: "red", text: d.quote_stale }
        : f === "off"
          ? { tone: "", text: d.quote_off_hours }
          : { tone: "yellow", text: d.quote_none };
  };
  return (
    <div class="kpis data-kpis">
      <Kpi
        label={t().data.kpi_source}
        value={
          <span class={`chip ${props.status.source === "demo" ? "yellow" : "green"}`} style={{ height: "26px", "font-size": "12px" }}>
            <SIcon name="database" size={13} />
            {props.status.source === "demo" ? t().data.source_demo : t().data.source_fmp}
          </span>
        }
        delta={<span class="muted">{t().data.history} {tpl(t().data.years, { count: props.status.history_years })}</span>}
      />
      <Kpi
        label={t().data.kpi_newest}
        value={<span class="num">{props.status.quotes.newest ? fmtRelative(props.status.quotes.newest, serverNow()) : "—"}</span>}
        delta={
          <span class="row" style={{ gap: "6px" }}>
            <span class={`chip ${freshChip().tone}`}>{freshChip().text}</span>
            <Show when={props.status.quotes.newest}>
              <span class="muted">{fmtDual(props.status.quotes.newest)}</span>
            </Show>
          </span>
        }
      />
      <Kpi
        label={t().data.kpi_quotes}
        value={<span class="num">{tpl(t().data.symbols, { count: props.status.quotes.count })}</span>}
        delta={
          <Show when={props.status.quotes.oldest}>
            <span class="muted">{tpl(t().data.oldest, { time: fmtRelative(props.status.quotes.oldest, serverNow()) })}</span>
          </Show>
        }
      />
      <Kpi
        label={t().data.kpi_coverage}
        value={<span class="num">{tpl(t().data.symbols, { count: props.status.coverage.length })}</span>}
        delta={
          <span class={issues() ? "warn-text" : "ok-text"}>
            {issues() ? tpl(t().data.issues, { count: issues() }) : t().data.no_issues}
          </span>
        }
      />
    </div>
  );
}

function SourceCard(props: { status: DataStatus }) {
  const t = settingsText;
  const fmp = () => props.status.fmp;
  const apiMode = () => (fmp().api_mode === "auto" ? t().data.api_auto : fmp().api_mode);
  return (
    <div class="card">
      <div class="card-head">
        <div class="head-text">
          <h2>{t().data.config_title}</h2>
          <div class="sub">{t().data.config_sub}</div>
        </div>
      </div>
      <div class="card-body">
        <dl class="kv settings-kv">
          <dt>{t().data.kpi_source}</dt>
          <dd>{props.status.source === "demo" ? t().data.source_demo : t().data.source_fmp}</dd>
          <dt>{t().data.fmp_keys}</dt>
          <dd>
            <Show when={fmp().keys > 0} fallback={<span class="muted">{t().data.keys_none}</span>}>
              {tpl(t().data.keys_count, { count: fmp().keys })}
              <span class="muted"> · {tpl(t().data.key_origin, { origin: fmp().key_origin })}</span>
            </Show>
          </dd>
          <dt>{t().data.base_url}</dt>
          <dd class="mono small break">{fmp().base_url}</dd>
          <dt>{t().data.api_mode}</dt>
          <dd>{apiMode()}</dd>
          <dt>{t().data.rate_limit}</dt>
          <dd>{tpl(t().data.per_minute, { count: fmp().requests_per_minute })}</dd>
          <dt>{t().data.history}</dt>
          <dd>{tpl(t().data.years, { count: props.status.history_years })}</dd>
          <dt>{t().data.database}</dt>
          <dd>
            <span class="mono small break">{props.status.database}</span>
            <div style={{ "margin-top": "4px" }}>
              <span class={`chip ${props.status.database_borrowed_from_honeclaw ? "blue" : "outline"}`}>
                {props.status.database_borrowed_from_honeclaw ? t().data.db_borrowed : t().data.db_own}
              </span>
            </div>
          </dd>
        </dl>
      </div>
    </div>
  );
}

const JOB_TONE: Record<JobRun["status"], string> = { running: "blue", succeeded: "green", failed: "red", skipped: "" };

function duration(job: JobRun): string {
  if (!job.finished_at) return "—";
  const ms = new Date(job.finished_at).getTime() - new Date(job.started_at).getTime();
  if (!Number.isFinite(ms) || ms < 0) return "—";
  if (ms < 1000) return `${ms} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  return `${Math.round(ms / 60_000)} min`;
}

function JobDetail(props: { job: JobRun }) {
  const t = settingsText;
  const d = () => props.job.detail as Record<string, unknown>;
  const num = (v: unknown) => (typeof v === "number" ? v : null);
  const parts = createMemo(() => {
    const out: string[] = [];
    const x = d();
    const bars = num(x.bars);
    if (bars !== null) out.push(tpl(t().data.detail_bars, { count: bars.toLocaleString("en-US") }));
    const failed = x.failed && typeof x.failed === "object" ? Object.keys(x.failed as object).length : num(x.failed);
    if (failed) out.push(tpl(t().data.detail_failed, { count: failed }));
    const actions = num(x.actions);
    if (actions) out.push(tpl(t().data.detail_actions, { count: actions }));
    const applied = num(x.applied);
    if (applied) out.push(tpl(t().data.detail_applied, { count: applied }));
    const orders = num(x.orders);
    if (orders !== null) out.push(tpl(t().data.detail_orders, { count: orders }));
    const nav = num(x.nav) ?? (typeof x.nav === "string" ? Number(x.nav) : null);
    if (nav !== null && Number.isFinite(nav)) out.push(tpl(t().data.detail_nav, { value: fmtMoney(nav) }));
    return out;
  });
  return (
    <div class="job-detail">
      <Show when={props.job.error}>
        <div class="error-line">{props.job.error}</div>
      </Show>
      <span class="muted">{parts().join(" · ")}</span>
      <Show when={num(d().plan_id)}>
        {(id) => (
          <>
            {parts().length ? " · " : ""}
            <A href={`/plans/${id()}`}>{tpl(t().data.detail_plan, { id: id() })}</A>
          </>
        )}
      </Show>
    </div>
  );
}

function JobsCard(props: { jobs: DataStatus["jobs"] }) {
  const t = settingsText;
  const jobName = (job: (typeof JOBS)[number]) => t().data[`job_${job}`];
  return (
    <div class="card">
      <div class="card-head">
        <div class="head-text">
          <h2>{t().data.jobs_title}</h2>
          <div class="sub">{t().data.jobs_sub}</div>
        </div>
      </div>
      <div class="table-wrap">
        <table class="table jobs-table">
          <thead>
            <tr>
              <th>{t().data.col_job}</th>
              <th>{common().words.status}</th>
              <th>{t().data.col_started}</th>
              <th class="r">{t().data.col_duration}</th>
              <th>{t().data.col_result}</th>
            </tr>
          </thead>
          <tbody>
            <For each={JOBS}>
              {(name) => (
                <tr>
                  <td class="nowrap">
                    <b>{jobName(name)}</b>
                  </td>
                  <Show
                    when={props.jobs[name]}
                    fallback={
                      <td colSpan={4}>
                        <span class="muted small">{t().data.job_never}</span>
                      </td>
                    }
                  >
                    {(job) => (
                      <>
                        <td>
                          <span class={`chip ${JOB_TONE[job().status]}`}>
                            <span class="dot" />
                            {t().data[`job_status_${job().status}`]}
                          </span>
                        </td>
                        <td class="nowrap small num" title={fmtDual(job().started_at, true)}>
                          {fmtRelative(job().started_at, serverNow())}
                          <div class="muted xs">{fmtDual(job().started_at, true)}</div>
                        </td>
                        <td class="r small num">{duration(job())}</td>
                        <td class="small">
                          <JobDetail job={job()} />
                        </td>
                      </>
                    )}
                  </Show>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </div>
  );
}

function ActionsCard(props: { onStarted: () => void }) {
  const t = settingsText;
  const [running, setRunning] = createSignal<SyncKind | null>(null);
  const actions = (): { kind: SyncKind; title: string; body: string; icon: "refresh" | "candle" | "history" | "layers"; danger?: boolean }[] => [
    { kind: "quotes", title: t().data.sync_quotes, body: t().data.sync_quotes_body, icon: "refresh" },
    { kind: "daily", title: t().data.sync_daily, body: t().data.sync_daily_body, icon: "candle" },
    { kind: "corporate_actions", title: t().data.sync_ca, body: t().data.sync_ca_body, icon: "layers" },
    { kind: "full", title: t().data.sync_full, body: t().data.sync_full_body, icon: "history", danger: true },
  ];
  const run = async (a: ReturnType<typeof actions>[number]) => {
    const ok = await confirmAction({
      title: t().data.sync_confirm_title,
      body: tpl(t().data.sync_confirm, { action: a.title, body: a.body }),
      confirmLabel: a.title,
      danger: a.danger,
    });
    if (ok === null) return;
    setRunning(a.kind);
    try {
      await api.dataSync(a.kind);
      toast(tpl(t().data.sync_started, { action: a.title }), undefined, "success");
      props.onStarted();
    } catch (error) {
      toastError(error);
    } finally {
      setRunning(null);
    }
  };
  return (
    <div class="card">
      <div class="card-head">
        <div class="head-text">
          <h2>{t().data.actions_title}</h2>
          <div class="sub">{t().data.actions_sub}</div>
        </div>
      </div>
      <ul class="action-list">
        <For each={actions()}>
          {(a) => (
            <li>
              <span class="action-icon">
                <SIcon name={a.icon} size={16} />
              </span>
              <div class="action-text">
                <b>{a.title}</b>
                <span>{a.body}</span>
              </div>
              <button type="button" class={`btn sm ${a.danger ? "danger" : ""}`} disabled={running() !== null} onClick={() => void run(a)}>
                {running() === a.kind ? t().data.starting : t().data.run}
              </button>
            </li>
          )}
        </For>
      </ul>
    </div>
  );
}

function FmpCard(props: { demo: boolean }) {
  const t = settingsText;
  const [state, setState] = createSignal<
    { kind: "idle" } | { kind: "running" } | { kind: "done"; checks: FmpCheck[] } | { kind: "error"; message: string; demo: boolean }
  >({ kind: "idle" });
  const run = async () => {
    setState({ kind: "running" });
    try {
      const { checks } = await api.fmpCheck();
      setState({ kind: "done", checks });
    } catch (error) {
      const demo = error instanceof ApiError && error.status === 400 && /demo/i.test(error.message);
      if (!demo) toastError(error);
      setState({ kind: "error", message: error instanceof ApiError ? error.message : String(error), demo });
    }
  };
  const done = () => {
    const s = state();
    return s.kind === "done" ? s.checks : null;
  };
  const failure = () => {
    const s = state();
    return s.kind === "error" ? s : null;
  };
  return (
    <div class="card">
      <div class="card-head">
        <div class="head-text">
          <h2>{t().data.fmp_title}</h2>
          <div class="sub">{t().data.fmp_sub}</div>
        </div>
        <span class="spacer" />
        <button type="button" class="btn sm" disabled={state().kind === "running"} onClick={() => void run()}>
          <SIcon name="zap" size={13} />
          {state().kind === "running" ? t().data.fmp_running : t().data.fmp_run}
        </button>
      </div>
      <div class="card-body">
        <Show when={props.demo && state().kind === "idle"}>
          <p class="muted small">{t().data.fmp_demo}</p>
        </Show>
        <Show when={!props.demo && state().kind === "idle"}>
          <p class="muted small">{t().data.fmp_idle}</p>
        </Show>
        <Show when={failure()}>
          {(f) => (
            <div class={`callout ${f().demo ? "info" : "critical"}`}>
              <SIcon name={f().demo ? "info" : "alert"} size={16} />
              <span>{f().demo ? t().data.fmp_demo : f().message}</span>
            </div>
          )}
        </Show>
        <Show when={done()}>
          {(checks) => (
            <div class="stack" style={{ gap: "10px" }}>
              <p class="small">
                <b>{tpl(t().data.fmp_summary, { ok: checks().filter((c) => c.ok).length, total: checks().length })}</b>
              </p>
              <div class="table-wrap">
                <table class="table compact">
                  <thead>
                    <tr>
                      <th>{t().data.fmp_col_endpoint}</th>
                      <th>{t().data.fmp_col_api}</th>
                      <th>{t().data.fmp_col_result}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={checks()}>
                      {(c) => (
                        <tr>
                          <td class="mono small">{c.endpoint}</td>
                          <td class="small">{c.api}</td>
                          <td class="small">
                            <span class={`chip ${c.ok ? "green" : "red"}`}>{c.ok ? t().data.fmp_ok : t().data.fmp_fail}</span>
                            <Show when={c.detail}>
                              <span class="muted"> {c.detail}</span>
                            </Show>
                          </td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </Show>
      </div>
    </div>
  );
}

type SortKey = "symbol" | "first" | "last" | "bars";

function CoverageCard(props: { coverage: Coverage[]; universe: Awaited<ReturnType<typeof api.universe>> | undefined }) {
  const t = settingsText;
  const [query, setQuery] = createSignal("");
  const [onlyIssues, setOnlyIssues] = createSignal(false);
  const [sort, setSort] = createSignal<{ key: SortKey; dir: 1 | -1 }>({ key: "symbol", dir: 1 });
  const issues = createMemo(() => analyse(props.coverage));
  const names = createMemo(() => {
    const map = new Map<string, { name: string; benchmark: boolean }>();
    for (const a of props.universe?.assets ?? []) map.set(a.symbol, { name: localName(a), benchmark: false });
    for (const b of props.universe?.benchmarks ?? []) map.set(b.symbol, { name: localName(b), benchmark: true });
    return map;
  });
  const span = (c: Coverage) => {
    if (!c.first || !c.last) return null;
    return (new Date(c.last).getTime() - new Date(c.first).getTime()) / (365.25 * 86_400_000);
  };
  const rows = createMemo(() => {
    const q = query().trim().toLowerCase();
    const { key, dir } = sort();
    return props.coverage
      .filter((c) => !onlyIssues() || (issues().get(c.symbol)?.length ?? 0) > 0)
      .filter((c) => !q || c.symbol.toLowerCase().includes(q) || (names().get(c.symbol)?.name ?? "").toLowerCase().includes(q))
      .slice()
      .sort((a, b) => {
        const va = key === "bars" ? a.bars : key === "symbol" ? a.symbol : (a[key] ?? "");
        const vb = key === "bars" ? b.bars : key === "symbol" ? b.symbol : (b[key] ?? "");
        return (va < vb ? -1 : va > vb ? 1 : 0) * dir;
      });
  });
  const toggleSort = (key: SortKey) => setSort((s) => ({ key, dir: s.key === key ? ((-s.dir) as 1 | -1) : 1 }));
  const arrow = (key: SortKey) => (sort().key === key ? (sort().dir === 1 ? " ↑" : " ↓") : "");
  const issueChip = (issue: CoverageIssue) => {
    const d = t().data;
    const map: Record<CoverageIssue, [string, string]> = {
      stale: ["red", d.st_stale],
      insufficient: ["red", d.st_insufficient],
      short: ["yellow", d.st_short],
      unadjusted: ["yellow", d.st_unadjusted],
      empty: ["red", d.st_empty],
    };
    return map[issue];
  };
  const issueCount = () => [...issues().values()].filter((i) => i.length > 0).length;
  return (
    <div class="card">
      <div class="card-head wrap">
        <div class="head-text">
          <h2>{t().data.coverage_title}</h2>
          <div class="sub">
            {t().data.coverage_sub} · {tpl(t().data.symbols, { count: props.coverage.length })}
            <Show when={issueCount() > 0}> · {tpl(t().data.issues, { count: issueCount() })}</Show>
          </div>
        </div>
        <span class="spacer" />
        <div class="coverage-tools">
          <div class="search-box">
            <SIcon name="search" size={14} />
            <input class="input" type="search" placeholder={t().data.search} value={query()} onInput={(e) => setQuery(e.currentTarget.value)} aria-label={t().data.search} />
          </div>
          <Switch checked={onlyIssues()} onChange={setOnlyIssues} label={<span class="switch-label small">{t().data.only_issues}</span>} />
        </div>
      </div>
      <div class="table-wrap coverage-wrap">
        <table class="table compact coverage-table">
          <thead>
            <tr>
              <th class="sortable" onClick={() => toggleSort("symbol")}>
                {common().words.symbol}
                {arrow("symbol")}
              </th>
              <th class="sortable" onClick={() => toggleSort("first")}>
                {t().data.col_first}
                {arrow("first")}
              </th>
              <th class="sortable" onClick={() => toggleSort("last")}>
                {t().data.col_last}
                {arrow("last")}
              </th>
              <th class="sortable r" onClick={() => toggleSort("bars")}>
                {t().data.col_bars}
                {arrow("bars")}
              </th>
              <th class="r">{t().data.col_adjusted}</th>
              <th class="r">{t().data.col_span}</th>
              <th>{t().data.col_status}</th>
            </tr>
          </thead>
          <tbody>
            <For each={rows()} fallback={<tr><td colSpan={7} class="muted small c">{t().data.no_match}</td></tr>}>
              {(c) => {
                const list = () => issues().get(c.symbol) ?? [];
                return (
                  <tr classList={{ "has-issue": list().length > 0 }}>
                    <td>
                      <div class="name-cell">
                        <span class="ticker">
                          {c.symbol}
                          <Show when={names().get(c.symbol)?.benchmark}>
                            <span class="chip outline bench-tag">{t().data.benchmark_tag}</span>
                          </Show>
                        </span>
                        <span class="name">{names().get(c.symbol)?.name ?? ""}</span>
                      </div>
                    </td>
                    <td class="num small nowrap">{c.first ?? "—"}</td>
                    <td class="num small nowrap" classList={{ "warn-text": list().includes("stale") }}>
                      {c.last ?? "—"}
                    </td>
                    <td class="r small" classList={{ "warn-text": list().includes("insufficient") || list().includes("short") }}>
                      {c.bars.toLocaleString("en-US")}
                    </td>
                    <td class="r small" classList={{ "warn-text": list().includes("unadjusted") }}>
                      {c.adjusted.toLocaleString("en-US")}
                    </td>
                    <td class="r small muted nowrap">{span(c) === null ? "—" : tpl(t().data.span_years, { value: fmtNum(span(c), 1) })}</td>
                    <td>
                      <Show when={list().length > 0} fallback={<span class="chip green">{t().data.st_ok}</span>}>
                        <span class="chip-row">
                          <For each={list()}>{(i) => <span class={`chip ${issueChip(i)[0]}`}>{issueChip(i)[1]}</span>}</For>
                        </span>
                      </Show>
                    </td>
                  </tr>
                );
              }}
            </For>
          </tbody>
        </table>
      </div>
      <div class="card-foot">
        <SIcon name="info" size={13} />
        <span>{t().data.legend}</span>
      </div>
    </div>
  );
}
