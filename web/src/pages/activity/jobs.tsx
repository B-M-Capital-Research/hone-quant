/**
 * Scheduled jobs tab: scheduler runs (data syncs, plan generation, reminders, risk checks)
 * with status, timing and expandable results. Refreshes every 5 s while a job is running.
 */
import { A } from "@solidjs/router";
import { For, Match, Show, Switch, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { Icon } from "@/components/Icon";
import { Empty, ErrorState, Loading } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { auditText } from "@/i18n/audit";
import { common } from "@/i18n/common";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { DASH, fmtNum } from "@/lib/format";
import { serverNow } from "@/lib/session";
import type { JobRun } from "@/lib/types";
import { DualTime, JsonView } from "./components";
import { useQuery } from "./query";
import { debounce, fmtDuration } from "./util";

const STATUS_TONE: Record<string, string> = { running: "blue", succeeded: "green", failed: "red", skipped: "" };

export function jobName(job: string): string {
  return (auditText().jobs.names as Record<string, string>)[job] ?? job;
}

function runKeyLabel(run: JobRun): { text: string; href: string | null } {
  const j = auditText().jobs;
  if (run.job === "plan") {
    const [date, slot] = run.run_key.split(":");
    const c = common();
    const slotName = slot === "open" || slot === "close" || slot === "manual" ? c.slot[slot] : slot ?? "";
    return { text: tpl(j.key_plan, { date, slot: slotName }), href: null };
  }
  if (run.job === "reminder_plan_review" && /^\d+$/.test(run.run_key)) {
    return { text: tpl(j.key_review, { id: run.run_key }), href: `/plans/${run.run_key}` };
  }
  return { text: run.run_key, href: null };
}

function planIdOf(run: JobRun): number | null {
  const id = (run.detail as Record<string, unknown> | null)?.plan_id;
  return typeof id === "number" ? id : null;
}

export function JobsTab(props: { job: string; setJob: (job: string) => void }) {
  const j = () => auditText().jobs;
  const [limit, setLimit] = createSignal(100);
  const [tick, setTick] = createSignal(0);
  const [open, setOpen] = createSignal<Set<number>>(new Set());

  const runs = useQuery(
    () => ({ job: props.job, limit: limit(), tick: tick() }),
    (k) => api.jobs({ job: k.job || undefined, limit: k.limit }),
  );
  const list = () => runs.data()?.jobs ?? [];
  const running = createMemo(() => list().some((run) => run.status === "running"));

  createEffect(() => {
    if (!running()) return;
    const timer = setInterval(() => setTick((v) => v + 1), 5000);
    onCleanup(() => clearInterval(timer));
  });

  onMount(() => {
    const bump = debounce(() => setTick((v) => v + 1), 1200);
    const off = onServerEvent(["plan", "account", "notification", "quotes"], (event) => {
      if (event.type === "quotes") return;
      bump();
    });
    onCleanup(() => {
      off();
      bump.cancel();
    });
  });

  const names = createMemo(() => {
    const known = Object.keys(auditText().jobs.names);
    const seen = list().map((run) => run.job);
    if (props.job) seen.push(props.job);
    return [...new Set([...known, ...seen])];
  });

  const toggle = (id: number) =>
    setOpen((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <>
      <section class="card">
        <div class="card-head aud-jobs-head">
          <div class="aud-jobs-title">
            <h2>{j().card}</h2>
            <div class="sub">{j().sub}</div>
          </div>
          <span class="spacer" />
          <label class="visually-hidden" for="jobs-filter">
            {j().filter}
          </label>
          <select
            id="jobs-filter"
            class="select act-select aud-job-select"
            classList={{ set: !!props.job }}
            value={props.job}
            onChange={(e) => props.setJob(e.currentTarget.value)}
          >
            <option value="">{j().all}</option>
            <For each={names()}>{(name) => <option value={name}>{jobName(name)}</option>}</For>
          </select>
          <button type="button" class="btn sm" onClick={() => setTick((v) => v + 1)}>
            <Icon name="refresh" size={14} />
            {common().actions.refresh}
          </button>
        </div>
        <Show when={running()}>
          <div class="aud-auto" role="status">
            <span class="spinner" />
            {j().auto}
          </div>
        </Show>
        <div class="card-body flush">
          <Switch>
            <Match when={runs.error() && !runs.data()}>
              <ErrorState error={runs.error()} onRetry={runs.refetch} />
            </Match>
            <Match when={!runs.data()}>
              <Loading />
            </Match>
            <Match when={!list().length}>
              <Empty title={j().empty} icon="clock">
                <span>{j().empty_hint}</span>
              </Empty>
            </Match>
            <Match when={true}>
              <div class="table-wrap" classList={{ refetching: runs.loading() && !running() }}>
                <table class="table compact act-table aud-table">
                  <thead>
                    <tr>
                      <th class="aud-toggle-col">
                        <span class="visually-hidden">{auditText().expand}</span>
                      </th>
                      <th>{j().cols.job}</th>
                      <th>{j().cols.key}</th>
                      <th>{j().cols.status}</th>
                      <th>{j().cols.started}</th>
                      <th>{j().cols.finished}</th>
                      <th class="r">{j().cols.duration}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={list()}>{(run) => <JobRow run={run} open={open().has(run.id)} onToggle={() => toggle(run.id)} />}</For>
                  </tbody>
                </table>
              </div>
            </Match>
          </Switch>
        </div>
        <Show when={list().length > 0}>
          <div class="card-foot">
            <span>{tpl(j().shown, { n: fmtNum(list().length, 0) })}</span>
            <span class="spacer" />
            <Show when={list().length >= limit() && limit() < 500}>
              <button type="button" class="btn sm" onClick={() => setLimit((v) => Math.min(500, v + 200))}>
                {j().show_more}
              </button>
            </Show>
          </div>
        </Show>
      </section>
    </>
  );
}

function JobRow(props: { run: JobRun; open: boolean; onToggle: () => void }) {
  const a = auditText;
  const j = () => a().jobs;
  const run = () => props.run;
  const key = () => runKeyLabel(run());
  const duration = () => {
    const start = Date.parse(run().started_at);
    const end = run().finished_at ? Date.parse(run().finished_at!) : run().status === "running" ? serverNow() : null;
    return end === null ? null : end - start;
  };
  const detailId = `job-detail-${props.run.id}`;
  const hasDetail = () => !!run().error || Object.keys(run().detail ?? {}).length > 0;
  return (
    <>
      <tr
        class="aud-row"
        classList={{ open: props.open, clickable: hasDetail() }}
        onClick={(event) => {
          if (!hasDetail() || (event.target as HTMLElement).closest("a, button")) return;
          props.onToggle();
        }}
      >
        <td class="aud-toggle-col">
          <Show when={hasDetail()}>
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
          </Show>
        </td>
        <td>
          <div class="aud-action">
            <span class="aud-action-label">{jobName(run().job)}</span>
            <span class="aud-code">{run().job}</span>
          </div>
        </td>
        <td>
          <Show when={key().href} fallback={<span class="aud-key">{key().text}</span>}>
            <A class="aud-key" href={key().href!}>
              {key().text}
            </A>
          </Show>
        </td>
        <td>
          <span class={`chip ${STATUS_TONE[run().status] ?? ""}`} title={run().error ?? undefined}>
            <Show when={run().status === "running"} fallback={<span class="dot" />}>
              <span class="dot aud-pulse" />
            </Show>
            {(j().status as Record<string, string>)[run().status] ?? run().status}
          </span>
        </td>
        <td>
          <DualTime value={run().started_at} />
        </td>
        <td>
          <Show when={run().finished_at} fallback={<span class="muted">{DASH}</span>}>
            <DualTime value={run().finished_at} />
          </Show>
        </td>
        <td class="r num">{fmtDuration(duration(), locale() === "zh")}</td>
      </tr>
      <Show when={props.open}>
        <tr class="aud-detail" id={detailId}>
          <td colspan="7">
            <div class="aud-job-detail">
              <Show when={run().error}>
                <div class="callout critical">
                  <Icon name="alert" size={16} />
                  <div>
                    <strong>{j().error}</strong>
                    <p class="aud-error">{run().error}</p>
                  </div>
                </div>
              </Show>
              <Show when={planIdOf(run())}>
                {(id) => (
                  <A class="act-plan-link" href={`/plans/${id()}`}>
                    {a().entities.plan} #{id()}
                    <Icon name="chevron_right" size={13} />
                  </A>
                )}
              </Show>
              <Show when={Object.keys(run().detail ?? {}).length > 0 || !run().error}>
                <JsonView value={run().detail} label={j().result} />
              </Show>
            </div>
          </td>
        </tr>
      </Show>
    </>
  );
}
