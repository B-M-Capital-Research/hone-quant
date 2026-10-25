/**
 * /ai — the AI sector view: status and manual run, the latest view with its evidence, the forward
 * test for the selected portfolio, and every attempt with what the model read (`?view=<id>`
 * opens one).
 */
import "@/styles/research.css";
import "@/styles/plans.css";
import "@/styles/ai.css";
import { useSearchParams } from "@solidjs/router";
import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { Icon } from "@/components/Icon";
import { ErrorState, Kpi, Loading, toast, toastError } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { aiText } from "@/i18n/ai";
import { strategyText } from "@/i18n/strategy";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { fmtDual, fmtNum, fmtPct } from "@/lib/format";
import { onPortfolioEvent } from "@/lib/portfolio";
import { isAdmin } from "@/lib/session";
import type { AiEvaluation, AiOverview, AiView, UniverseView } from "@/lib/types";
import { actorName } from "@/lib/names";
import { sectorLabel } from "./overview/util";
import { Card } from "./research/components";
import { createLoader, debounce, universeRef } from "./research/model";
import { OpinionDetail, StanceChip, opinionText } from "./ai/components";

export default function Ai() {
  const a = aiText;
  const [params, setParams] = useSearchParams<{ view?: string }>();
  const overview = createLoader(() => api.ai());
  const evaluation = createLoader(() => api.aiEvaluation().then((r) => r.evaluation));
  const [universe, setUniverse] = createSignal<UniverseView>();
  const [starting, setStarting] = createSignal(false);

  onMount(() => {
    void overview.load();
    void evaluation.load();
    universeRef.get().then(setUniverse).catch(() => undefined);
    const refresh = debounce(() => void overview.load(), 500);
    const offView = onServerEvent(["ai_view", "strategy"], refresh);
    const offPlan = onPortfolioEvent(["plan", "account"], debounce(() => void evaluation.load(), 1500));
    onCleanup(() => {
      offView();
      offPlan();
    });
  });

  const sector = (id: string) => sectorLabel(universe()?.sectors, id);

  const generate = async () => {
    setStarting(true);
    try {
      await api.generateAiView();
      toast(a().status.generate_started);
      void overview.load();
    } catch (error) {
      toastError(error);
    } finally {
      setStarting(false);
    }
  };

  return (
    <div class="ai-page">
      <header class="page-head">
        <div>
          <h1>{a().title}</h1>
          <p class="lead">{a().lead}</p>
        </div>
      </header>

      <div class="callout info">
        <Icon name="info" size={17} />
        <div>
          <strong>{a().honesty_title}</strong>
          <p>{a().honesty}</p>
        </div>
      </div>

      <Show when={overview.data()} fallback={overview.error() ? <ErrorState error={overview.error()} onRetry={() => void overview.load()} /> : <Loading />}>
        {(o) => (
          <>
            <StatusCard overview={o()} starting={starting()} onGenerate={generate} />
            <LatestCard view={o().latest} sector={sector} />
            <EvaluationCard evaluation={evaluation.data()} error={evaluation.error()} sector={sector} />
            <HistoryCard views={o().views} open={params.view ? Number(params.view) : null} onOpen={(id) => setParams({ view: id ?? undefined }, { replace: true })} />
          </>
        )}
      </Show>
    </div>
  );
}

function StatusCard(props: { overview: AiOverview; starting: boolean; onGenerate: () => void }) {
  const a = aiText;
  const o = () => props.overview;
  const busy = () => props.starting || o().running;
  const capped = () => o().attempts_24h >= o().max_attempts_24h;
  return (
    <Card
      title={a().status.title}
      sub={a().status.schedule}
      actions={
        <Show when={isAdmin()}>
          <button type="button" class="btn sm primary" disabled={!o().configured || busy() || capped()} onClick={() => props.onGenerate()}>
            <Icon name={busy() ? "clock" : "play"} size={14} />
            {busy() ? a().status.generating : a().status.generate}
          </button>
        </Show>
      }
    >
      <Show
        when={o().configured}
        fallback={
          <div class="callout warn">
            <Icon name="alert" size={17} />
            <div>
              <strong>{a().status.not_configured}</strong>
              <p>{a().status.not_configured_hint}</p>
            </div>
          </div>
        }
      >
        <dl class="ai-facts">
          <div>
            <dt>{a().status.model}</dt>
            <dd class="num">{o().model}</dd>
          </div>
          <div>
            <dt>{a().status.endpoint}</dt>
            <dd class="num">{o().endpoint}</dd>
          </div>
          <div>
            <dt>{a().status.origin}</dt>
            <dd>{o().origin}</dd>
          </div>
          <div>
            <dt>{a().status.attempts}</dt>
            <dd class="num">{tpl(a().status.attempts_value, { n: o().attempts_24h, max: o().max_attempts_24h })}</dd>
          </div>
        </dl>
      </Show>
      <div class="ai-users">
        <div class="xs muted">{a().status.users}</div>
        <Show when={o().portfolios.length} fallback={<p class="xs muted">{a().status.users_none}</p>}>
          <div class="pill-list">
            <For each={o().portfolios}>
              {(p) => (
                <span class="param">
                  {p.portfolio} · {p.version}
                  <b>{strategyText().ai_modes[p.ai.mode]}</b>
                  <span class="muted">±{fmtPct(p.ai.max_tilt, { dp: 0 })}</span>
                </span>
              )}
            </For>
          </div>
        </Show>
      </div>
    </Card>
  );
}

function LatestCard(props: { view: AiView | null; sector: (id: string) => string }) {
  const a = aiText;
  const [open, setOpen] = createSignal<string | null>(null);
  const summary = () => {
    const out = props.view?.output;
    if (!out) return "";
    return (locale() === "zh" ? out.summary_zh || out.summary_en : out.summary_en || out.summary_zh) ?? "";
  };
  return (
    <Card
      title={a().latest.title}
      sub={
        <Show when={props.view}>
          {tpl(a().latest.as_of, { date: props.view!.as_of })} · {tpl(a().latest.formed, { time: fmtDual(props.view!.created_at, true) })} · {props.view!.model}
        </Show>
      }
      flush
    >
      <Show when={props.view?.output} fallback={<p class="muted ai-pad">{a().latest.none}</p>}>
        {(out) => (
          <>
            <Show when={summary()}>
              <p class="ai-summary">{summary()}</p>
            </Show>
            <div class="table-wrap">
              <table class="table compact ai-table">
                <thead>
                  <tr>
                    <th>{a().latest.col_sector}</th>
                    <th>{a().latest.col_stance}</th>
                    <th class="r">{a().latest.col_confidence}</th>
                    <th>{a().latest.col_rationale}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={out().sectors}>
                    {(s) => (
                      <>
                        <tr class="ai-row" classList={{ open: open() === s.sector }} onClick={() => setOpen(open() === s.sector ? null : s.sector)}>
                          <td class="nowrap">
                            <Icon name={open() === s.sector ? "chevron_down" : "chevron_right"} size={12} />
                            {props.sector(s.sector)}
                          </td>
                          <td class="nowrap">
                            <StanceChip stance={s.stance} />
                            <Show when={s.proposed_stance != null}>
                              {" "}
                              <span class="chip yellow" title={tpl(a().latest.proposed, { stance: String(s.proposed_stance) })}>
                                {a().flags.insufficient_evidence}
                              </span>
                            </Show>
                          </td>
                          <td class="r num">{s.stance === 0 && s.confidence === 0 ? "—" : fmtPct(s.confidence, { dp: 0 })}</td>
                          <td class="ai-why">{opinionText(s, "rationale") || <span class="muted">{s.flags.includes("missing") ? a().flags.missing : "—"}</span>}</td>
                        </tr>
                        <Show when={open() === s.sector}>
                          <tr class="ai-subrow">
                            <td colspan={4}>
                              <OpinionDetail opinion={s} />
                            </td>
                          </tr>
                        </Show>
                      </>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
            <p class="muted xs ai-pad">{a().latest.tilt_note}</p>
          </>
        )}
      </Show>
    </Card>
  );
}

function bps(value: number | null | undefined, dp = 1): string {
  if (value == null || !Number.isFinite(value)) return "—";
  return tpl(aiText().evaluation.bps, { v: `${value > 0 ? "+" : value < 0 ? "−" : ""}${fmtNum(Math.abs(value), dp)}` });
}

function EvaluationCard(props: { evaluation: AiEvaluation | undefined; error: unknown; sector: (id: string) => string }) {
  const a = aiText;
  const e = () => props.evaluation;
  const sectors = () =>
    Object.entries(e()?.sectors ?? {})
      .filter(([, v]) => Math.abs(v) > 1e-6)
      .sort((x, y) => y[1] - x[1]);
  return (
    <Card title={a().evaluation.title} sub={a().evaluation.sub}>
      <Show when={!props.error} fallback={<ErrorState error={props.error} />}>
        <Show when={e()} fallback={<Loading />}>
          <Show when={e()!.plans > 0} fallback={<p class="muted">{a().evaluation.none}</p>}>
            <div class="kpi-strip ai-kpis">
              <Kpi label={a().evaluation.plans} value={<span class="num">{e()!.plans}</span>} />
              <Kpi label={a().evaluation.days} value={<span class="num">{e()!.days}</span>} />
              <Kpi
                label={a().evaluation.cumulative}
                value={<span class={`num ${e()!.cumulative_bps > 0 ? "up" : e()!.cumulative_bps < 0 ? "down" : ""}`}>{bps(e()!.cumulative_bps)}</span>}
              />
              <Kpi label={a().evaluation.mean} value={<span class="num">{bps(e()!.mean_bps, 2)}</span>} />
              <Kpi label={a().evaluation.hit_rate} value={<span class="num">{fmtPct(e()!.hit_rate, { dp: 0 })}</span>} />
              <Kpi label={a().evaluation.t_stat} value={<span class="num">{e()!.t_stat == null ? "—" : fmtNum(e()!.t_stat, 2)}</span>} />
              <Kpi label={a().evaluation.active_share} value={<span class="num">{fmtPct(e()!.mean_active_share, { dp: 1 })}</span>} />
            </div>
            <Show when={sectors().length}>
              <div class="xs muted ai-pad-top">{a().evaluation.by_sector}</div>
              <div class="pill-list">
                <For each={sectors()}>
                  {([id, value]) => (
                    <span class="param">
                      {props.sector(id)} <b class={value > 0 ? "up" : "down"}>{bps(value)}</b>
                    </span>
                  )}
                </For>
              </div>
            </Show>
            <p class="muted xs ai-pad-top">{a().evaluation.caution}</p>
          </Show>
        </Show>
      </Show>
    </Card>
  );
}

function HistoryCard(props: { views: AiView[]; open: number | null; onOpen: (id: number | null) => void }) {
  const a = aiText;
  return (
    <Card title={a().history.title} flush>
      <div class="table-wrap">
        <table class="table compact">
          <thead>
            <tr>
              <th>{a().history.col_created}</th>
              <th>{a().history.col_as_of}</th>
              <th>{a().history.col_status}</th>
              <th>{a().history.col_model}</th>
              <th class="r">{a().history.col_latency}</th>
              <th>{a().history.col_summary}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            <For each={props.views}>
              {(v) => (
                <>
                  <tr>
                    <td class="nowrap">
                      {fmtDual(v.created_at, true)}
                      <div class="xs muted">{tpl(a().history.by, { actor: actorName(v.created_by) })}</div>
                    </td>
                    <td class="num nowrap">{v.as_of}</td>
                    <td>{v.status === "ok" ? <span class="chip green">{a().history.ok}</span> : <span class="chip red">{a().history.failed}</span>}</td>
                    <td class="xs">{v.model}</td>
                    <td class="r num nowrap">{v.latency_ms == null ? "—" : `${fmtNum(v.latency_ms / 1000, 1)} s`}</td>
                    <td class="ai-why xs">{v.error ?? (locale() === "zh" ? v.output?.summary_zh : v.output?.summary_en) ?? ""}</td>
                    <td class="r">
                      <button type="button" class="btn sm ghost" onClick={() => props.onOpen(props.open === v.id ? null : v.id)}>
                        {props.open === v.id ? a().history.hide : a().history.show}
                      </button>
                    </td>
                  </tr>
                  <Show when={props.open === v.id}>
                    <tr class="ai-subrow">
                      <td colspan={7}>
                        <ViewMaterial id={v.id} />
                      </td>
                    </tr>
                  </Show>
                </>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Card>
  );
}

function ViewMaterial(props: { id: number }) {
  const a = aiText;
  const detail = createLoader(() => api.aiView(props.id).then((r) => r.view));
  onMount(() => void detail.load());
  return (
    <Show when={detail.data()} fallback={detail.error() ? <ErrorState error={detail.error()} /> : <Loading />}>
      {(v) => (
        <div class="ai-material">
          <div class="xs muted">{a().history.material}</div>
          <pre>{JSON.stringify(v().material, null, 2)}</pre>
          <Show when={v().raw_response}>
            <div class="xs muted">{a().history.raw}</div>
            <pre>{v().raw_response}</pre>
          </Show>
        </div>
      )}
    </Show>
  );
}
