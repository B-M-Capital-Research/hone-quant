import { A } from "@solidjs/router";
import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import { Icon } from "@/components/Icon";
import { Dialog, ErrorState, Loading, confirmAction, toast, toastError } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { strategyText } from "@/i18n/strategy";
import { api } from "@/lib/api";
import { fmtDateTime, fmtDual } from "@/lib/format";
import { isAdmin, serverNow } from "@/lib/session";
import type { StrategyOverview, StrategyParams, StrategyVersion, UniverseView } from "@/lib/types";
import { ParamEditor, fieldDomId } from "./ParamEditor";
import { PreviewPanel } from "./PreviewPanel";
import { cleanParams, fieldForIssue, getPath, sameParams, withDefaults } from "./params";
import { fieldText, fmtParam, issueText, pickText, presetName } from "./format";
import { activationBody, backtestHref } from "./shared";
import type { Source, Workbench } from "./workbench";
import { actorName, strategyName } from "@/lib/names";

/** All starting points: the active version, every saved version and every preset. */
export function buildSources(ov: StrategyOverview): Source[] {
  const out: Source[] = [];
  const params = (p: StrategyParams) => withDefaults(p, ov.defaults);
  if (ov.active) {
    out.push({ key: "active", kind: "active", versionId: ov.active.id, presetId: ov.active.preset_id, name: strategyName(ov.active), params: params(ov.active.params) });
  }
  for (const v of ov.versions) {
    if (ov.active && v.id === ov.active.id) continue;
    out.push({ key: `v:${v.id}`, kind: "version", versionId: v.id, presetId: v.preset_id, name: strategyName(v), params: params(v.params) });
  }
  for (const p of ov.presets) {
    out.push({ key: `p:${p.id}`, kind: "preset", presetId: p.id, name: pickText(p, "name"), params: params(p.params) });
  }
  return out;
}

export function sourceLabel(src: Source): string {
  if (src.kind === "preset") return tpl(strategyText().workbench.preset_prefix, { name: src.name });
  return `#${src.versionId} ${src.name}`;
}

/** "宽度均线：须在 10 到 400 之间" lines for a toast. */
function issueSummary(wb: Workbench): string {
  const sep = locale() === "zh" ? "：" : ": ";
  const lines = wb.issues().map((i) => {
    const def = fieldForIssue(i.path);
    return def ? `${fieldText(def.path).label}${sep}${issueText(i)}` : issueText(i);
  });
  return [...new Set([...lines, ...wb.general()])].join("\n");
}

function scrollToField(path: string) {
  const def = fieldForIssue(path);
  if (!def) return;
  const el = document.querySelector<HTMLElement>(`[data-path="${def.path}"]`);
  el?.scrollIntoView({ behavior: "smooth", block: "center" });
  const input = document.getElementById(fieldDomId(def.path)) as HTMLInputElement | null;
  setTimeout(() => input?.focus({ preventScroll: true }), 250);
}

export function WorkbenchView(props: {
  wb: Workbench;
  overview: StrategyOverview;
  universe: UniverseView | undefined;
  previewRequested: boolean;
  onPreviewHandled: () => void;
  onSaved: () => void;
  onShowVersions: () => void;
}) {
  const t = strategyText;
  const wb = props.wb;
  const sources = createMemo(() => buildSources(props.overview));
  const [saving, setSaving] = createSignal(false);
  const [saveOpen, setSaveOpen] = createSignal(false);
  let previewCard: HTMLDivElement | undefined;
  let sourceSelect: HTMLSelectElement | undefined;

  // Keep the select showing the starting point even when its options re-render (e.g. after a
  // save adds a version): the DOM value is not re-applied automatically when options change.
  createEffect(() => {
    const key = wb.source()?.key ?? "";
    sources();
    queueMicrotask(() => {
      if (sourceSelect && sourceSelect.value !== key) sourceSelect.value = key;
    });
  });

  const activeParams = () => (props.overview.active ? withDefaults(props.overview.active.params, props.overview.defaults) : null);
  const sectors = () => props.universe?.sectors ?? [];
  const memberCounts = createMemo(() => {
    const counts: Record<string, number> = {};
    for (const a of props.universe?.assets ?? []) counts[a.sector_id] = (counts[a.sector_id] ?? 0) + 1;
    return counts;
  });

  const pickSource = async (key: string) => {
    const next = sources().find((s) => s.key === key);
    if (!next || next.key === wb.source()?.key) return;
    if (wb.changed().length) {
      const ok = await confirmAction({
        title: t().workbench.discard_title,
        body: tpl(t().workbench.discard_body, { n: wb.changed().length }),
        confirmLabel: t().workbench.discard_confirm,
      });
      if (ok === null) return;
    }
    wb.load(next);
  };

  const runPreview = async () => {
    // Only client-side issues block: the server re-validates every request anyway.
    if (wb.clientIssues().length) {
      toast(t().workbench.fix_first, undefined, "warning");
      scrollToField(wb.clientIssues()[0].path);
      return;
    }
    const outcome = await wb.preview(activeParams());
    if (outcome === "invalid") {
      toast(t().workbench.server_rejected, issueSummary(wb), "critical", 7000);
      if (wb.issues().length) scrollToField(wb.issues()[0].path);
    } else if (outcome === "ok") {
      requestAnimationFrame(() => previewCard?.scrollIntoView({ behavior: "smooth", block: "start" }));
    }
  };

  // A preview requested from another tab ("Preview active strategy") runs once we are on screen.
  createEffect(() => {
    if (props.previewRequested && wb.draft()) {
      props.onPreviewHandled();
      queueMicrotask(() => void runPreview());
    }
  });

  const describe = () => {
    const src = wb.source();
    if (!src) return "";
    if (src.kind === "preset") {
      const preset = props.overview.presets.find((p) => p.id === src.presetId);
      return preset ? pickText(preset, "summary") : "";
    }
    const v = props.overview.versions.find((x) => x.id === src.versionId);
    if (!v) return "";
    const parts = [`${t().active.preset}: ${presetName(props.overview.presets, v.preset_id)}`, `${fmtDateTime(v.created_at)} · ${actorName(v.created_by)}`];
    if (v.note) parts.push(v.note);
    return parts.join(" · ");
  };

  return (
    <div class="stack st-workbench">
      <section class="card">
        <div class="card-head">
          <span class="st-step-badge">1</span>
          <div style={{ flex: 1, "min-width": 0 }}>
            <h2>{t().workbench.start_title}</h2>
            <div class="sub">{t().workbench.start_sub}</div>
          </div>
        </div>
        <div class="card-body stack" style={{ gap: "10px" }}>
          <div class="st-source-row">
            <label class="field" style={{ flex: "1 1 320px" }}>
              <span class="field-label">{t().workbench.source}</span>
              <select
                ref={sourceSelect}
                class="select"
                value={wb.source()?.key ?? ""}
                onChange={(e) => {
                  const el = e.currentTarget;
                  void pickSource(el.value).then(() => {
                    el.value = wb.source()?.key ?? "";
                  });
                }}
              >
                <Show when={sources().some((s) => s.kind === "active")}>
                  <optgroup label={t().workbench.group_active}>
                    <For each={sources().filter((s) => s.kind === "active")}>{(s) => <option value={s.key}>{sourceLabel(s)}</option>}</For>
                  </optgroup>
                </Show>
                <Show when={sources().some((s) => s.kind === "version")}>
                  <optgroup label={t().workbench.group_versions}>
                    <For each={sources().filter((s) => s.kind === "version")}>{(s) => <option value={s.key}>{sourceLabel(s)}</option>}</For>
                  </optgroup>
                </Show>
                <optgroup label={t().workbench.group_presets}>
                  <For each={sources().filter((s) => s.kind === "preset")}>{(s) => <option value={s.key}>{s.name}</option>}</For>
                </optgroup>
              </select>
            </label>
          </div>
          <Show when={describe()}>
            <p class="small subtle">{describe()}</p>
          </Show>
          <Show when={!isAdmin()}>
            <div class="callout info">
              <Icon name="info" size={16} />
              <span>{t().workbench.viewer_note}</span>
            </div>
          </Show>
        </div>
      </section>

      <Show when={wb.saved()}>
        {(saved) => (
          <div class="callout ok st-saved">
            <Icon name="check" size={16} />
            <div class="stack" style={{ gap: "6px", flex: 1 }}>
              <strong>{tpl(saved().activated ? t().workbench.saved_activated : t().workbench.saved_title, { id: saved().id })}</strong>
              <Show when={!saved().activated}>
                <span>{t().workbench.saved_body}</span>
              </Show>
              <div class="row wrap">
                <A class="btn sm" href={backtestHref(saved().id)}>
                  <Icon name="backtest" size={13} /> {t().actions.backtest_version}
                </A>
                <button class="btn sm ghost" onClick={() => props.onShowVersions()}>
                  {t().workbench.view_versions}
                </button>
              </div>
            </div>
          </div>
        )}
      </Show>

      <section class="card">
        <div class="card-head">
          <span class="st-step-badge">2</span>
          <div style={{ flex: 1, "min-width": 0 }}>
            <h2>{t().workbench.form_title}</h2>
            <div class="sub">{t().workbench.form_sub}</div>
          </div>
        </div>
        <div class="card-body">
          <Show when={wb.general().length}>
            <div class="callout critical" style={{ "margin-bottom": "14px" }}>
              <Icon name="alert" size={16} />
              <div>
                <strong>{t().workbench.server_rejected}</strong>
                <For each={wb.general()}>{(line) => <div class="xs">{line}</div>}</For>
              </div>
            </div>
          </Show>
          <Show when={wb.draft()}>
            <ParamEditor wb={wb} defaults={props.overview.defaults} sectors={sectors()} memberCounts={memberCounts()} />
          </Show>
        </div>
      </section>

      <div class="st-actionbar" role="region" aria-label={t().workbench.form_title}>
        <div class="st-actionbar-status">
          <span class="chip" classList={{ yellow: wb.changed().length > 0 }}>
            <Show when={wb.changed().length} fallback={t().workbench.unchanged}>
              {tpl(t().workbench.changed, { n: wb.changed().length })}
            </Show>
          </span>
          <Show when={wb.issues().length}>
            <button class="chip red st-issue-chip" onClick={() => scrollToField(wb.issues()[0].path)}>
              <Icon name="alert" size={11} /> {tpl(t().workbench.issues, { n: wb.issues().length })}
            </button>
          </Show>
        </div>
        <span class="spacer" />
        <button class="btn" onClick={() => wb.revert()} disabled={!wb.changed().length}>
          {t().actions.revert}
        </button>
        <button class="btn" onClick={() => void runPreview()} disabled={wb.running() || !wb.draft()}>
          <Show when={wb.running()} fallback={<Icon name="play" size={13} />}>
            <span class="spinner st-spinner-sm" />
          </Show>
          {t().actions.preview}
        </button>
        <button
          class="btn primary"
          onClick={() => {
            if (wb.clientIssues().length) {
              toast(t().workbench.fix_first, undefined, "warning");
              scrollToField(wb.clientIssues()[0].path);
              return;
            }
            setSaveOpen(true);
          }}
          disabled={!isAdmin() || !wb.draft() || saving()}
          title={isAdmin() ? undefined : t().save.viewer}
        >
          {t().actions.save_version}
        </button>
      </div>

      <section class="card" ref={previewCard}>
        <div class="card-head">
          <span class="st-step-badge">3</span>
          <div style={{ flex: 1, "min-width": 0 }}>
            <h2>{t().preview.title}</h2>
            <Show when={wb.run()} fallback={<div class="sub">{t().preview.empty}</div>}>
              {(run) => <div class="sub">{tpl(t().preview.sub, { time: fmtDual(run().result.as_of, true) })}</div>}
            </Show>
          </div>
          <Show when={wb.run()}>
            <button class="btn sm" onClick={() => void runPreview()} disabled={wb.running()}>
              <Icon name="refresh" size={13} /> {t().actions.rerun}
            </button>
          </Show>
        </div>
        <div class="card-body">
          <Show when={wb.runError()}>
            <ErrorState error={wb.runError()} onRetry={() => void runPreview()} />
          </Show>
          <Show when={!wb.runError()}>
            <Show
              when={wb.run()}
              fallback={
                <Show
                  when={wb.running()}
                  fallback={
                    <div class="st-preview-empty">
                      <Icon name="target" size={22} />
                      <p>{t().preview.empty}</p>
                      <button class="btn" onClick={() => void runPreview()} disabled={!wb.draft()}>
                        <Icon name="play" size={13} /> {t().actions.preview}
                      </button>
                    </div>
                  }
                >
                  <Loading label={t().preview.running} />
                </Show>
              }
            >
              {(run) => <PreviewPanel run={run()} universe={props.universe} outdated={wb.outdated()} running={wb.running()} onRerun={() => void runPreview()} />}
            </Show>
          </Show>
        </div>
      </section>

      <Show when={saveOpen()}>
        <SaveDialog
          wb={wb}
          overview={props.overview}
          onClose={() => setSaveOpen(false)}
          onReopen={() => setSaveOpen(true)}
          busy={saving()}
          setBusy={setSaving}
          onSaved={(version, activated) => {
            setSaveOpen(false);
            wb.rebase({
              key: activated ? "active" : `v:${version.id}`,
              kind: activated ? "active" : "version",
              versionId: version.id,
              presetId: version.preset_id,
              name: version.name,
              params: withDefaults(version.params, props.overview.defaults),
            });
            wb.setSaved({ id: version.id, activated });
            props.onSaved();
            window.scrollTo({ top: 0, behavior: "smooth" });
          }}
        />
      </Show>
    </div>
  );
}

/** Name, note, lineage and "activate now" — with the activation confirmed separately. */
function SaveDialog(props: {
  wb: Workbench;
  overview: StrategyOverview;
  onClose: () => void;
  onReopen: () => void;
  onSaved: (version: StrategyVersion, activated: boolean) => void;
  busy: boolean;
  setBusy: (v: boolean) => void;
}) {
  const t = strategyText;
  const c = common;
  const wb = props.wb;
  const src = () => wb.source();
  const [name, setName] = createSignal(saveState.name);
  const [note, setNote] = createSignal(saveState.note);
  const [presetId, setPresetId] = createSignal(saveState.presetId || src()?.presetId || "custom");
  const [activate, setActivate] = createSignal(saveState.activate);
  const [nameError, setNameError] = createSignal(false);

  const params = () => cleanParams(wb.draft() as StrategyParams);
  const identical = createMemo(() =>
    props.overview.versions.find((v) => sameParams(withDefaults(v.params, props.overview.defaults), params())),
  );
  const previewed = () => !!wb.run() && !wb.outdated();
  const changes = () => wb.changed();

  const remember = () => {
    saveState.name = name();
    saveState.note = note();
    saveState.presetId = presetId();
    saveState.activate = activate();
  };

  const submit = async () => {
    const trimmed = name().trim();
    if (!trimmed || [...trimmed].length > 80) {
      setNameError(true);
      return;
    }
    remember();
    if (activate()) {
      props.onClose();
      const body = await activationBody(trimmed, null, serverNow());
      const ok = await confirmAction({ title: t().activate.title, body, confirmLabel: t().activate.confirm });
      if (ok === null) {
        props.onReopen();
        return;
      }
    }
    props.setBusy(true);
    try {
      const res = await api.createVersion({ name: trimmed, preset_id: presetId(), params: params(), note: note().trim(), activate: activate() });
      saveState.name = "";
      saveState.note = "";
      saveState.presetId = "";
      saveState.activate = false;
      toast(
        res.activated ? tpl(t().activate.done, { id: res.version.id }) : tpl(t().workbench.saved_title, { id: res.version.id }),
        res.activated ? t().activate.done_body : undefined,
        "success",
      );
      props.onSaved(res.version, res.activated);
    } catch (error) {
      if (wb.absorbServerError(error)) {
        props.onClose();
        toast(t().workbench.server_rejected, issueSummary(wb), "critical", 7000);
        if (wb.issues().length) scrollToField(wb.issues()[0].path);
      } else {
        toastError(error);
        if (activate()) props.onReopen();
      }
    } finally {
      props.setBusy(false);
    }
  };

  return (
    <Dialog
      title={t().save.title}
      onClose={() => {
        remember();
        props.onClose();
      }}
      footer={
        <>
          <button
            class="btn"
            onClick={() => {
              remember();
              props.onClose();
            }}
          >
            {c().actions.cancel}
          </button>
          <button class="btn primary" onClick={() => void submit()} disabled={props.busy}>
            {props.busy ? c().actions.saving : activate() ? t().save.submit_activate : t().save.submit}
          </button>
        </>
      }
    >
      <div class="stack" style={{ gap: "14px" }}>
        <div class="field">
          <label for="st-save-name">{t().save.name}</label>
          <input
            id="st-save-name"
            class="input"
            classList={{ invalid: nameError() }}
            maxLength={80}
            placeholder={t().save.name_placeholder}
            value={name()}
            onInput={(e) => {
              setName(e.currentTarget.value);
              setNameError(false);
            }}
            autofocus
          />
          <Show when={nameError()}>
            <span class="error-text">{t().save.name_required}</span>
          </Show>
        </div>
        <div class="field">
          <label for="st-save-note">{t().save.note}</label>
          <textarea id="st-save-note" class="textarea" rows={2} placeholder={t().save.note_placeholder} value={note()} onInput={(e) => setNote(e.currentTarget.value)} />
        </div>
        <div class="st-save-grid">
          <div class="field">
            <span class="field-label">{t().save.based_on}</span>
            <span class="small">{src() ? sourceLabel(src()!) : "—"}</span>
          </div>
          <div class="field">
            <label for="st-save-preset">{t().save.preset}</label>
            <select id="st-save-preset" class="select" value={presetId()} onChange={(e) => setPresetId(e.currentTarget.value)}>
              <For each={props.overview.presets}>{(p) => <option value={p.id}>{pickText(p, "name")}</option>}</For>
              <option value="custom">{t().active.custom_preset}</option>
            </select>
            <span class="hint">{t().save.preset_hint}</span>
          </div>
        </div>

        <div class="field">
          <span class="field-label">{t().save.changes}</span>
          <Show when={changes().length} fallback={<span class="small muted">{t().save.no_changes}</span>}>
            <ul class="st-change-list">
              <For each={changes()}>
                {(path) => (
                  <li>
                    <span>{fieldText(path).label}</span>
                    <span class="num">
                      <span class="muted">{fmtParam(path, getPath(src()!.params, path))}</span> → <b>{fmtParam(path, getPath(wb.draft() as StrategyParams, path))}</b>
                    </span>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </div>

        <Show when={identical()}>
          {(v) => (
            <div class="callout warn">
              <Icon name="alert" size={16} />
              <span>{tpl(t().save.identical, { id: v().id, name: strategyName(v()) })}</span>
            </div>
          )}
        </Show>
        <Show when={!previewed()}>
          <div class="callout info">
            <Icon name="info" size={16} />
            <span>{t().save.not_previewed}</span>
          </div>
        </Show>

        <div class="st-activate-box" classList={{ on: activate() }}>
          <label class="row" style={{ gap: "10px", cursor: "pointer", "align-items": "flex-start" }}>
            <input type="checkbox" checked={activate()} onChange={(e) => setActivate(e.currentTarget.checked)} style={{ "margin-top": "3px" }} />
            <span>
              <strong class="small">{t().save.activate}</strong>
              <span class="xs muted" style={{ display: "block" }}>
                {t().save.activate_hint}
              </span>
            </span>
          </label>
        </div>
      </div>
    </Dialog>
  );
}

/** Keeps the dialog's fields while it is closed for the activation confirmation. */
const saveState = { name: "", note: "", presetId: "", activate: false };
