import { A, useSearchParams } from "@solidjs/router";
import { For, Show, createEffect, createResource, createSignal, on, onCleanup, onMount } from "solid-js";
import "@/styles/strategy.css";
import { Icon } from "@/components/Icon";
import { ErrorState, Loading, PlanStatusChip, confirmAction, toast } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { strategyText } from "@/i18n/strategy";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { fmtDateTime, fmtDual } from "@/lib/format";
import { currentPortfolio, onPortfolioEvent } from "@/lib/portfolio";
import { isAdmin, serverNow } from "@/lib/session";
import type { Plan, Preset, ScheduleSettings, Sector, StrategyOverview, StrategyVersion } from "@/lib/types";
import { Methodology } from "./strategy/Methodology";
import { KeyParams, ParamView } from "./strategy/ParamView";
import { PresetCards } from "./strategy/Presets";
import { ActivationTimeline, type DialogMode, VersionDialog, VersionsTable } from "./strategy/Versions";
import { WorkbenchView, buildSources } from "./strategy/Workbench";
import { diffPaths, withDefaults } from "./strategy/params";
import { presetName } from "./strategy/format";
import { activateExisting, backtestHref, nextPlanInfo } from "./strategy/shared";
import { type Source, createWorkbench } from "./strategy/workbench-state";
import { actorName, strategyName, systemNote } from "@/lib/names";

type Tab = "overview" | "workbench" | "versions";
const TABS: Tab[] = ["overview", "workbench", "versions"];

export default function Strategy() {
  const t = strategyText;
  const [params, setParams] = useSearchParams();
  const [overview, { refetch }] = createResource(() => api.strategy());
  const [universe, { refetch: refetchUniverse }] = createResource(() => api.universe());
  const [settings] = createResource(() => api.settings().catch(() => null));
  const [pending, { refetch: refetchPending }] = createResource(() => api.plans({ status: "pending", limit: 10 }).catch(() => null));
  const wb = createWorkbench();
  const [previewRequested, setPreviewRequested] = createSignal(false);

  const tab = (): Tab => {
    const value = typeof params.tab === "string" ? params.tab : "";
    return (TABS as string[]).includes(value) ? (value as Tab) : "overview";
  };
  const setTab = (next: Tab) => {
    setParams({ tab: next === "overview" ? undefined : next });
    window.scrollTo({ top: 0 });
  };

  onMount(() => {
    const offStrategy = onPortfolioEvent(["strategy"], () => void refetch());
    const offUniverse = onServerEvent(["universe"], () => void refetchUniverse());
    const offPlans = onPortfolioEvent(["plan"], () => void refetchPending());
    onCleanup(() => {
      offStrategy();
      offUniverse();
      offPlans();
    });
  });

  const ov = (): StrategyOverview | undefined => (overview.error ? undefined : overview.latest);

  // The workbench starts from the active version (or the default preset) once data arrives,
  // and keeps pointing at the right version when another admin activates something else.
  createEffect(
    on(ov, (data) => {
      if (!data) return;
      const src = wb.source();
      if (!src) {
        const sources = buildSources(data);
        const initial = sources.find((s) => s.kind === "active") ?? sources.find((s) => s.kind === "preset");
        if (initial) wb.load(initial);
        return;
      }
      if (src.kind === "active" && data.active?.id !== src.versionId && src.versionId !== undefined) {
        wb.rebase({ ...src, key: `v:${src.versionId}`, kind: "version" });
      } else if (src.kind === "version" && data.active?.id === src.versionId) {
        wb.rebase({ ...src, key: "active", kind: "active" });
      }
    }),
  );

  /** Loads a starting point into the workbench (asking before discarding edits). */
  const startFrom = async (src: Source, preview = false) => {
    const same = wb.source()?.key === src.key;
    const dirty = wb.changed().length;
    if (dirty) {
      const ok = await confirmAction({
        title: t().workbench.discard_title,
        body: tpl(t().workbench.discard_body, { n: dirty }),
        confirmLabel: t().workbench.discard_confirm,
      });
      if (ok === null) return;
    }
    if (!same || dirty) wb.load(src);
    setTab("workbench");
    if (preview) setPreviewRequested(true);
  };

  const sourceFor = (key: string) => {
    const data = ov();
    return data ? buildSources(data).find((s) => s.key === key) : undefined;
  };
  const useVersion = (v: StrategyVersion) => {
    closeDialog();
    const src = sourceFor(ov()?.active?.id === v.id ? "active" : `v:${v.id}`);
    if (src) void startFrom(src);
  };
  const usePreset = (p: Preset) => {
    const src = sourceFor(`p:${p.id}`);
    if (src) void startFrom(src);
  };
  const previewActive = () => {
    const src = sourceFor("active");
    if (src) void startFrom(src, true);
  };

  const activate = async (v: StrategyVersion) => {
    const done = await activateExisting(v, serverNow());
    if (done) {
      closeDialog();
      void refetch();
      void refetchPending();
    }
  };

  // Version dialog, driven by the URL so /strategy?version=<id> deep-links into it.
  const dialogVersion = () => {
    const id = Number(params.version);
    if (!params.version || !Number.isFinite(id)) return null;
    return ov()?.versions.find((v) => v.id === id) ?? null;
  };
  const dialogMode = (): DialogMode => (params.mode === "diff" ? "diff" : "params");
  const openDialog = (v: StrategyVersion, mode: DialogMode) => setParams({ version: String(v.id), mode: mode === "diff" ? "diff" : undefined });
  const closeDialog = () => setParams({ version: undefined, mode: undefined });
  createEffect(
    on(ov, (data) => {
      if (!data || !params.version) return;
      if (!dialogVersion()) {
        toast(tpl(t().versions.not_found, { id: String(params.version) }), undefined, "warning");
        closeDialog();
      }
    }),
  );

  return (
    <div class="st-page">
      <div class="page-head">
        <div style={{ flex: "1 1 420px", "min-width": 0 }}>
          <h1>{t().title}</h1>
          <p class="lead">{t().lead}</p>
        </div>
        <div class="row wrap">
          <Show when={ov()?.active}>
            {(active) => (
              <A class="btn" href={backtestHref(active().id)}>
                <Icon name="backtest" size={15} /> {t().actions.backtest_active}
              </A>
            )}
          </Show>
          <Show when={isAdmin() && tab() !== "workbench"}>
            <button class="btn primary" onClick={() => setTab("workbench")}>
              <Icon name="plus" size={15} /> {t().actions.new_version}
            </button>
          </Show>
        </div>
      </div>

      <div class="tabs st-tabs" role="tablist" aria-label={t().title}>
        <For each={TABS}>
          {(id) => (
            <button role="tab" aria-selected={tab() === id} onClick={() => setTab(id)}>
              {t().tabs[id]}
              <Show when={id === "workbench" && wb.changed().length}>
                <span class="st-dot" title={tpl(t().workbench.changed, { n: wb.changed().length })} />
              </Show>
              <Show when={id === "versions" && ov()}>
                <span class="st-count">{ov()!.versions.length}</span>
              </Show>
            </button>
          )}
        </For>
      </div>

      <Show when={!overview.error} fallback={<ErrorState error={overview.error} onRetry={() => void refetch()} />}>
        <Show when={ov()} fallback={<Loading />}>
          {(data) => (
            <>
              <Show when={tab() === "overview"}>
                <OverviewTab
                  data={data()}
                  schedule={settings()?.schedule ?? null}
                  pendingPlans={pending()?.plans ?? []}
                  sectors={universe()?.sectors ?? []}
                  onPreviewActive={previewActive}
                  onUsePreset={usePreset}
                  onShowVersions={() => setTab("versions")}
                  onNewVersion={() => setTab("workbench")}
                />
              </Show>
              <Show when={tab() === "workbench"}>
                <WorkbenchView
                  wb={wb}
                  overview={data()}
                  universe={universe.error ? undefined : universe.latest}
                  previewRequested={previewRequested()}
                  onPreviewHandled={() => setPreviewRequested(false)}
                  onSaved={() => {
                    void refetch();
                    void refetchPending();
                  }}
                  onShowVersions={() => setTab("versions")}
                />
              </Show>
              <Show when={tab() === "versions"}>
                <div class="st-versions-grid">
                  <section class="card">
                    <div class="card-head">
                      <div style={{ flex: 1 }}>
                        <h2>{t().versions.title}</h2>
                        <div class="sub">{tpl(t().versions.sub, { n: data().versions.length })}</div>
                      </div>
                    </div>
                    <div class="card-body flush">
                      <VersionsTable overview={data()} onOpen={openDialog} onActivate={(v) => void activate(v)} />
                    </div>
                  </section>
                  <section class="card">
                    <div class="card-head">
                      <div style={{ flex: 1 }}>
                        <h2>{t().versions.activations}</h2>
                        <div class="sub">{t().versions.activations_sub}</div>
                      </div>
                    </div>
                    <div class="card-body">
                      <ActivationTimeline activations={data().activations} activeId={data().active?.id ?? null} />
                    </div>
                  </section>
                </div>
              </Show>

              <Show when={dialogVersion()}>
                {(v) => (
                  <VersionDialog
                    version={v()}
                    overview={data()}
                    sectors={universe()?.sectors ?? []}
                    mode={dialogMode()}
                    onClose={closeDialog}
                    onUseAsStart={useVersion}
                    onActivate={(x) => void activate(x)}
                  />
                )}
              </Show>
            </>
          )}
        </Show>
      </Show>
    </div>
  );
}

function OverviewTab(props: {
  data: StrategyOverview;
  schedule: ScheduleSettings | null;
  pendingPlans: Plan[];
  sectors: Sector[];
  onPreviewActive: () => void;
  onUsePreset: (preset: Preset) => void;
  onShowVersions: () => void;
  onNewVersion: () => void;
}) {
  const t = strategyText;
  const c = common;
  const active = () => props.data.active;
  const activeParams = () => (active() ? withDefaults(active()!.params, props.data.defaults) : null);
  const activation = () => {
    const a = props.data.activations[0];
    return a && a.strategy_version_id === active()?.id ? a : null;
  };
  const presetDiff = () => {
    const v = active();
    if (!v) return null;
    const preset = props.data.presets.find((p) => p.id === v.preset_id);
    if (!preset) return null;
    return diffPaths(withDefaults(v.params, props.data.defaults), withDefaults(preset.params, props.data.defaults)).length;
  };

  return (
    <div class="stack st-overview">
      <div class="grid main-side">
        <section class="card st-active-card">
          <div class="card-head">
            <div style={{ flex: 1, "min-width": 0 }}>
              <h2>{t().active.title}</h2>
              <Show when={activation()}>
                {(a) => (
                  <div class="sub">
                    {tpl(t().active.since, { time: fmtDual(a().activated_at, true) })} · {tpl(t().active.by, { by: actorName(a().activated_by) })}
                  </div>
                )}
              </Show>
            </div>
            <Show when={active()}>
              <button class="btn sm" onClick={() => props.onPreviewActive()}>
                <Icon name="play" size={13} /> {t().actions.preview_active}
              </button>
            </Show>
          </div>
          <div class="card-body">
            <Show
              when={active()}
              fallback={
                <div class="callout warn">
                  <Icon name="alert" size={16} />
                  <div class="stack" style={{ gap: "8px" }}>
                    <strong>{t().active.none_title}</strong>
                    <span>{t().active.none_body}</span>
                    <Show when={isAdmin()}>
                      <div>
                        <button class="btn sm" onClick={() => props.onNewVersion()}>
                          <Icon name="plus" size={13} /> {t().actions.new_version}
                        </button>
                      </div>
                    </Show>
                  </div>
                </div>
              }
            >
              {(v) => (
                <div class="stack" style={{ gap: "16px" }}>
                  <div class="st-active-title">
                    <span class="st-vid big">#{v().id}</span>
                    <h3>{strategyName(v())}</h3>
                    <span class="chip green">
                      <span class="dot" />
                      {t().active.active_chip}
                    </span>
                  </div>
                  <Show when={currentPortfolio()}>
                    {(p) => (
                      <p class="xs muted st-active-scope">
                        <Icon name="briefcase" size={12} /> {tpl(t().active.scope, { name: p().name })}
                      </p>
                    )}
                  </Show>
                  <dl class="kv st-active-kv">
                    <dt>{t().active.preset}</dt>
                    <dd>
                      {presetName(props.data.presets, v().preset_id)}
                      <Show when={presetDiff() !== null}>
                        <span class="muted"> · {presetDiff() ? tpl(t().active.differs_from_preset, { n: presetDiff()! }) : t().active.matches_preset}</span>
                      </Show>
                    </dd>
                    <dt>{t().active.created}</dt>
                    <dd>{tpl(t().active.created_value, { time: fmtDateTime(v().created_at), by: actorName(v().created_by) })}</dd>
                    <Show when={v().note}>
                      <dt>{t().active.note}</dt>
                      <dd>{systemNote(v().note)}</dd>
                    </Show>
                    <Show when={activation()?.note}>
                      <dt>{t().active.activation_note}</dt>
                      <dd>{systemNote(activation()!.note)}</dd>
                    </Show>
                  </dl>
                  <div>
                    <div class="kicker" style={{ "margin-bottom": "8px" }}>
                      {t().active.key_params}
                    </div>
                    <KeyParams params={activeParams()!} baseline={props.data.defaults} />
                  </div>
                </div>
              )}
            </Show>
          </div>
        </section>

        <section class="card">
          <div class="card-head">
            <div style={{ flex: 1, "min-width": 0 }}>
              <h2>{t().effect.title}</h2>
              <div class="sub">{t().effect.sub}</div>
            </div>
          </div>
          <div class="card-body stack" style={{ gap: "14px" }}>
            <dl class="kv st-effect-kv">
              <dt>{t().effect.next_plan}</dt>
              <dd>
                <div>{nextPlanInfo(serverNow()).label}</div>
                <Show when={nextPlanInfo(serverNow()).time}>
                  <div class="xs muted num">{nextPlanInfo(serverNow()).time}</div>
                </Show>
              </dd>
              <dt>{t().effect.pending}</dt>
              <dd>
                <Show when={props.pendingPlans.length} fallback={<span class="muted">{t().effect.pending_none}</span>}>
                  <div class="stack" style={{ gap: "4px" }}>
                    <For each={props.pendingPlans}>
                      {(plan) => (
                        <A href={`/plans/${plan.id}`} class="row" style={{ gap: "6px" }}>
                          {tpl(t().effect.pending_item, { id: plan.id, slot: c().slot[plan.slot] })}
                          <PlanStatusChip status={plan.status} />
                        </A>
                      )}
                    </For>
                  </div>
                </Show>
              </dd>
            </dl>
            <p class="xs muted">{t().effect.pending_note}</p>
            <div class="st-divider" />
            <div class="row">
              <span class="kicker">{t().effect.history}</span>
              <span class="spacer" />
              <button class="btn sm ghost" onClick={() => props.onShowVersions()}>
                {t().actions.show_all} <Icon name="chevron_right" size={13} />
              </button>
            </div>
            <ActivationTimeline activations={props.data.activations} activeId={active()?.id ?? null} limit={3} />
          </div>
        </section>
      </div>

      <Show when={activeParams()}>
        {(p) => (
          <>
            <section class="card">
              <div class="card-head">
                <div style={{ flex: 1, "min-width": 0 }}>
                  <h2>{t().how.title}</h2>
                  <div class="sub">{t().how.sub}</div>
                </div>
              </div>
              <div class="card-body">
                <Methodology params={p()} schedule={props.schedule} />
                <p class="st-footnote">
                  <Icon name="shield" size={14} /> {t().how.footer}
                </p>
              </div>
            </section>

            <section class="card">
              <div class="card-head">
                <div style={{ flex: 1, "min-width": 0 }}>
                  <h2>{t().params_card.title}</h2>
                  <div class="sub">{tpl(t().params_card.sub, { id: active()!.id })}</div>
                </div>
              </div>
              <div class="card-body">
                <ParamView params={p()} defaults={props.data.defaults} sectors={props.sectors} />
              </div>
            </section>
          </>
        )}
      </Show>

      <section class="card">
        <div class="card-head">
          <div style={{ flex: 1, "min-width": 0 }}>
            <h2>{t().presets.title}</h2>
            <div class="sub">{t().presets.sub}</div>
          </div>
        </div>
        <div class="card-body">
          <PresetCards presets={props.data.presets} defaults={props.data.defaults} activePresetId={active()?.preset_id ?? null} onUse={props.onUsePreset} />
        </div>
      </section>
    </div>
  );
}
