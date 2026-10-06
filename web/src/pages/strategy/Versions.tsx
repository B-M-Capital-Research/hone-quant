import { A } from "@solidjs/router";
import { For, Show, createMemo, createSignal } from "solid-js";
import { Icon } from "@/components/Icon";
import { Dialog, Empty, Segmented } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { strategyText } from "@/i18n/strategy";
import { fmtDateTime, fmtDual } from "@/lib/format";
import { isAdmin } from "@/lib/session";
import type { Activation, Sector, StrategyOverview, StrategyVersion } from "@/lib/types";
import { KeyParams, ParamView } from "./ParamView";
import { diffPaths, getPath, withDefaults } from "./params";
import { fieldText, fmtParam, presetName } from "./format";
import { backtestHref } from "./shared";
import { actorName, strategyName, systemNote } from "@/lib/names";

export type DialogMode = "params" | "diff";

/** Number of parameters a version changed relative to its preset (null for custom lineage). */
function presetChanges(ov: StrategyOverview, v: StrategyVersion): number | null {
  const preset = ov.presets.find((p) => p.id === v.preset_id);
  if (!preset) return null;
  return diffPaths(withDefaults(v.params, ov.defaults), withDefaults(preset.params, ov.defaults)).length;
}

export function VersionsTable(props: {
  overview: StrategyOverview;
  onOpen: (version: StrategyVersion, mode: DialogMode) => void;
  onActivate: (version: StrategyVersion) => void;
}) {
  const t = strategyText;
  const activeId = () => props.overview.active?.id ?? null;
  const everActive = createMemo(() => new Set(props.overview.activations.map((a) => a.strategy_version_id)));
  return (
    <div class="table-wrap">
      <table class="table st-versions">
        <thead>
          <tr>
            <th>{t().versions.h_version}</th>
            <th>{t().versions.h_name}</th>
            <th>{t().versions.h_preset}</th>
            <th>{t().versions.h_created}</th>
            <th class="r">{t().versions.h_actions}</th>
          </tr>
        </thead>
        <tbody>
          <For each={props.overview.versions}>
            {(v) => {
              const isActive = () => v.id === activeId();
              const changes = () => presetChanges(props.overview, v);
              return (
                <tr classList={{ "st-row-active": isActive() }}>
                  <td class="nowrap">
                    <div class="row" style={{ gap: "8px" }}>
                      <span class="num st-vid">#{v.id}</span>
                      <Show when={isActive()}>
                        <span class="chip green">
                          <span class="dot" />
                          {t().active.active_chip}
                        </span>
                      </Show>
                      <Show when={!isActive() && everActive().has(v.id)}>
                        <span class="chip outline">{t().versions.previously}</span>
                      </Show>
                    </div>
                  </td>
                  <td class="st-name-col">
                    <div class="st-vname">{strategyName(v)}</div>
                    <Show when={v.note}>
                      <div class="xs muted st-vnote">{systemNote(v.note)}</div>
                    </Show>
                  </td>
                  <td class="nowrap">
                    <div class="small">{presetName(props.overview.presets, v.preset_id)}</div>
                    <Show when={changes() !== null}>
                      <div class="xs muted">{changes() ? tpl(t().versions.changes, { n: changes()! }) : t().active.matches_preset}</div>
                    </Show>
                  </td>
                  <td class="nowrap">
                    <div class="small num">{fmtDateTime(v.created_at)}</div>
                    <div class="xs muted">{actorName(v.created_by)}</div>
                  </td>
                  <td class="r nowrap">
                    <div class="st-row-actions">
                      <button class="btn sm ghost" onClick={() => props.onOpen(v, "params")}>
                        {t().actions.view_params}
                      </button>
                      <button class="btn sm ghost" onClick={() => props.onOpen(v, "diff")} disabled={isActive() || !props.overview.active}>
                        {t().actions.compare}
                      </button>
                      <A class="btn sm ghost" href={backtestHref(v.id)}>
                        {t().actions.backtest}
                      </A>
                      <Show when={isAdmin()}>
                        <button class="btn sm" onClick={() => props.onActivate(v)} disabled={isActive()} title={isActive() ? t().activate.already : undefined}>
                          {t().actions.activate}
                        </button>
                      </Show>
                    </div>
                  </td>
                </tr>
              );
            }}
          </For>
        </tbody>
      </table>
    </div>
  );
}

export function ActivationTimeline(props: { activations: Activation[]; activeId: number | null; limit?: number }) {
  const t = strategyText;
  const items = () => (props.limit ? props.activations.slice(0, props.limit) : props.activations);
  return (
    <Show when={props.activations.length} fallback={<Empty title={t().versions.activations_empty} icon="history" />}>
      <div class="timeline st-timeline">
        <For each={items()}>
          {(a, i) => {
            const current = () => i() === 0 && a.strategy_version_id === props.activeId;
            return (
              <div class="tl-item">
                <span class="tl-dot" classList={{ done: current() }}>
                  <Show when={current()}>
                    <Icon name="check" size={12} />
                  </Show>
                </span>
                <div class="st-tl-body">
                  <div class="row wrap" style={{ gap: "6px" }}>
                    <b class="num">#{a.strategy_version_id}</b>
                    <span class="st-tl-name">{strategyName({ name: a.version_name })}</span>
                    <Show when={current()}>
                      <span class="chip green st-mini-chip">{t().active.active_chip}</span>
                    </Show>
                  </div>
                  <div class="xs muted">
                    {fmtDual(a.activated_at, true)} · {actorName(a.activated_by)}
                  </div>
                  <Show when={a.note}>
                    <div class="xs st-tl-note">{systemNote(a.note)}</div>
                  </Show>
                </div>
              </div>
            );
          }}
        </For>
      </div>
    </Show>
  );
}

export function VersionDialog(props: {
  version: StrategyVersion;
  overview: StrategyOverview;
  sectors: Sector[];
  mode: DialogMode;
  onClose: () => void;
  onUseAsStart: (version: StrategyVersion) => void;
  onActivate: (version: StrategyVersion) => void;
}) {
  const t = strategyText;
  const c = common;
  const [mode, setMode] = createSignal<DialogMode>(props.mode);
  const params = () => withDefaults(props.version.params, props.overview.defaults);
  const active = () => props.overview.active;
  const activeParams = () => (active() ? withDefaults(active()!.params, props.overview.defaults) : null);
  const isActive = () => active()?.id === props.version.id;
  const diff = createMemo(() => (activeParams() ? diffPaths(activeParams()!, params()) : []));

  return (
    <Dialog
      wide
      title={tpl(t().versions.dialog_title, { id: props.version.id, name: strategyName(props.version) })}
      subtitle={tpl(t().versions.dialog_sub, {
        preset: presetName(props.overview.presets, props.version.preset_id),
        time: fmtDateTime(props.version.created_at),
        by: actorName(props.version.created_by),
      })}
      onClose={props.onClose}
      footer={
        <>
          <button class="btn ghost" onClick={() => props.onUseAsStart(props.version)}>
            <Icon name="sliders" size={14} /> {t().actions.use_as_start}
          </button>
          <A class="btn ghost" href={backtestHref(props.version.id)}>
            <Icon name="backtest" size={14} /> {t().actions.backtest_version}
          </A>
          <span class="spacer" />
          <Show when={isAdmin() && !isActive()}>
            <button class="btn primary" onClick={() => props.onActivate(props.version)}>
              {t().actions.activate}
            </button>
          </Show>
          <button class="btn" onClick={props.onClose}>
            {c().actions.close}
          </button>
        </>
      }
    >
      <div class="stack" style={{ gap: "14px" }}>
        <div class="row wrap" style={{ gap: "10px" }}>
          <Segmented
            value={mode()}
            onChange={setMode}
            options={[
              { value: "params", label: t().versions.mode_params },
              { value: "diff", label: t().versions.mode_diff },
            ]}
          />
          <Show when={isActive()}>
            <span class="chip green">
              <span class="dot" />
              {t().active.active_chip}
            </span>
          </Show>
        </div>
        <Show when={props.version.note}>
          <p class="small subtle">{props.version.note}</p>
        </Show>
        <Show when={mode() === "params"}>
          <KeyParams params={params()} baseline={props.overview.defaults} />
          <ParamView params={params()} defaults={props.overview.defaults} sectors={props.sectors} compact />
        </Show>
        <Show when={mode() === "diff"}>
          <Show when={active()} fallback={<Empty title={t().versions.no_active} icon="info" />}>
            <Show when={!isActive()} fallback={<Empty title={t().versions.is_active} icon="check" />}>
              <Show when={diff().length} fallback={<Empty title={t().versions.identical} icon="check" />}>
                <div class="table-wrap">
                  <table class="table compact">
                    <thead>
                      <tr>
                        <th>{t().view.parameter}</th>
                        <th class="r">{tpl(t().versions.diff_active, { id: active()!.id })}</th>
                        <th class="r">{t().versions.diff_this}</th>
                      </tr>
                    </thead>
                    <tbody>
                      <For each={diff()}>
                        {(path) => (
                          <tr>
                            <td>
                              <div>{fieldText(path).label}</div>
                              <div class="xs muted mono">{path}</div>
                            </td>
                            <td class="r nowrap muted">{fmtParam(path, getPath(activeParams()!, path))}</td>
                            <td class="r nowrap">
                              <b>{fmtParam(path, getPath(params(), path))}</b>
                            </td>
                          </tr>
                        )}
                      </For>
                    </tbody>
                  </table>
                </div>
              </Show>
            </Show>
          </Show>
        </Show>
      </div>
    </Dialog>
  );
}
