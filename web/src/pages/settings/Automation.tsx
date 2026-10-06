import { For, Show, createMemo, onCleanup, onMount } from "solid-js";
import { Segmented, Switch, confirmAction } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { settingsText } from "@/i18n/settings";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { fmtDual, zoneLabel } from "@/lib/format";
import { displayTz } from "@/lib/prefs";
import { market, refreshMarket, serverNow } from "@/lib/session";
import type { AutomationMode, AutomationSettings } from "@/lib/types";
import { type Errors, Field, FormCard, Gate, SIcon, type SIconName, createLoader, createSectionForm } from "./shared";
import { NY, fromLocalInput, toLocalInput } from "./time";

type Zone = "local" | "market";

interface Form {
  mode: AutomationMode;
  pause: boolean;
  until: string;
  zone: Zone;
  note: string;
}

const MODES: { mode: AutomationMode; icon: SIconName; tone: string }[] = [
  { mode: "auto", icon: "zap", tone: "green" },
  { mode: "approval", icon: "check", tone: "blue" },
  { mode: "paused", icon: "pause", tone: "yellow" },
];

const DAY = 86_400_000;
const MAX_PAUSE = 60 * DAY;

const ms = (iso: string | null | undefined) => (iso ? new Date(iso).getTime() : null);
const zoneTz = (zone: Zone) => (zone === "market" ? NY : displayTz());

/** A pause only matters while it lies in the future and the mode is not already "paused". */
function activePause(v: AutomationSettings, now: number): number | null {
  const until = ms(v.paused_until);
  return v.mode !== "paused" && until !== null && until > now ? until : null;
}

export default function AutomationSection() {
  const loader = createLoader(() => api.automation());
  onMount(() => {
    const off = onServerEvent(["settings"], (event) => {
      if (event.type !== "settings" || event.key === "automation") void loader.reload();
    });
    onCleanup(off);
  });
  return (
    <Gate loader={loader}>
      {(data) => <AutomationForm source={() => data().automation} effective={() => data().effective_mode} reload={() => loader.reload()} />}
    </Gate>
  );
}

function AutomationForm(props: { source: () => AutomationSettings; effective: () => AutomationMode; reload: () => Promise<void> }) {
  const t = settingsText;
  const c = common;

  const toForm = (v: AutomationSettings): Form => {
    const until = activePause(v, serverNow());
    return { mode: v.mode, pause: until !== null, until: until !== null ? toLocalInput(until, displayTz()) : "", zone: "local", note: "" };
  };

  const parse = (form: Form, base: AutomationSettings): { value: AutomationSettings | null; errors: Errors } => {
    const errors: Errors = {};
    let paused_until: string | null = null;
    if (form.pause && form.mode !== "paused") {
      const tz = zoneTz(form.zone);
      const baseUntil = activePause(base, serverNow());
      const at = fromLocalInput(form.until, tz);
      if (at === null) errors.until = t().automation.err_date;
      else if (baseUntil !== null && form.until === toLocalInput(baseUntil, tz)) paused_until = base.paused_until;
      else if (at <= serverNow()) errors.until = t().automation.err_future;
      else if (at > serverNow() + MAX_PAUSE) errors.until = t().automation.err_max;
      else paused_until = new Date(at).toISOString();
    }
    const value: AutomationSettings = { mode: form.mode, paused_until, note: form.note.trim() };
    return { value: Object.keys(errors).length ? null : value, errors };
  };

  const f = createSectionForm<AutomationSettings, Form>({
    source: props.source,
    toForm,
    parse,
    // The note explains a change; it is not itself something to save.
    compareKey: (v) => {
      const until = activePause(v, serverNow());
      return { mode: v.mode, paused_until: until === null ? null : Math.floor(until / 60_000) };
    },
    confirm: async (v) => {
      const before = props.source();
      const lines = [
        before.mode === v.mode
          ? tpl(t().automation.confirm_mode_same, { mode: c().mode[v.mode] })
          : tpl(t().automation.confirm_mode, { from: c().mode[before.mode], to: c().mode[v.mode] }),
      ];
      if (v.paused_until) lines.push(tpl(t().automation.confirm_pause, { time: fmtDual(v.paused_until, true) }));
      else if (activePause(before, serverNow()) !== null) lines.push(t().automation.confirm_pause_clear);
      if (v.note) lines.push(tpl(t().automation.confirm_note, { note: v.note }));
      lines.push("", t().automation.confirm_notify);
      const answer = await confirmAction({ title: t().automation.confirm_title, body: lines.join("\n"), confirmLabel: t().automation.save });
      return answer !== null;
    },
    submit: (v) => api.setAutomation(v),
    after: async () => {
      await props.reload();
      void refreshMarket();
    },
    fieldFor: (m) => (m.includes("paused_until") || m.includes("pause") ? "until" : null),
    savedLabel: () => t().automation.saved_label,
  });

  const untilMs = createMemo(() => (f.form.pause ? fromLocalInput(f.form.until, zoneTz(f.form.zone)) : null));

  const setZone = (zone: Zone) => {
    const at = untilMs();
    f.set("zone", zone);
    if (at !== null) f.set("until", toLocalInput(at, zoneTz(zone)));
  };

  const quick = (at: number) => {
    const rounded = Math.ceil(at / 60_000) * 60_000;
    f.set("pause", true);
    f.set("until", toLocalInput(rounded, zoneTz(f.form.zone)));
    f.touch("until");
  };

  const reviewMinutes = () => {
    const slot = market()?.schedule.find((s) => s.slot === "open") ?? market()?.schedule[0];
    if (!slot) return "10";
    return String(Math.round((new Date(slot.execute_at).getTime() - new Date(slot.generate_at).getTime()) / 60_000));
  };

  const modeBody = (m: AutomationMode) =>
    m === "auto" ? tpl(t().automation.auto_body, { review: reviewMinutes() }) : m === "approval" ? t().automation.approval_body : t().automation.paused_body;
  const modeTitle = (m: AutomationMode) =>
    m === "auto" ? t().automation.auto_title : m === "approval" ? t().automation.approval_title : t().automation.paused_title;

  const currentPause = () => activePause(props.source(), serverNow());
  const expiredPause = () => {
    const v = props.source();
    const until = ms(v.paused_until);
    return v.mode !== "paused" && until !== null && until <= serverNow() ? until : null;
  };
  const effectiveTone = () => MODES.find((m) => m.mode === props.effective())?.tone ?? "";

  return (
    <>
      <div class="card">
        <div class="card-head">
          <div class="head-text">
            <h2>{t().automation.status_title}</h2>
          </div>
        </div>
        <div class="card-body">
          <div class="status-row">
            <span class="muted small">{t().automation.effective}</span>
            <span class={`chip ${effectiveTone()}`} style={{ height: "26px", padding: "0 12px", "font-size": "12px" }}>
              <SIcon name={props.effective() === "paused" ? "pause" : "zap"} size={13} />
              {c().mode[props.effective()]}
            </span>
            <Show when={currentPause()}>
              {(until) => (
                <span class="small">
                  {tpl(t().automation.paused_until, { time: fmtDual(until(), true) })} · {tpl(t().automation.then_resume, { mode: c().mode[props.source().mode] })}
                </span>
              )}
            </Show>
          </div>
          <p class="muted small" style={{ "margin-top": "8px" }}>
            {c().mode[`${props.effective()}_hint` as const]}
          </p>
          <Show when={props.source().note}>
            <p class="small" style={{ "margin-top": "8px" }}>
              <span class="muted">{t().automation.note_label}</span>
              {props.source().note}
            </p>
          </Show>
        </div>
      </div>

      <FormCard f={f} title={t().automation.form_title} sub={t().automation.form_sub} saveLabel={t().automation.save}>
        <div class="stack" style={{ gap: "22px" }}>
          <div class="mode-cards" role="radiogroup" aria-label={t().automation.form_title}>
            <For each={MODES}>
              {(m) => (
                <label class={`mode-card tone-${m.tone}`} classList={{ selected: f.form.mode === m.mode, disabled: !f.canEdit() }}>
                  <input
                    type="radio"
                    name="automation-mode"
                    value={m.mode}
                    checked={f.form.mode === m.mode}
                    disabled={!f.canEdit()}
                    onChange={() => f.set("mode", m.mode)}
                  />
                  <span class="mode-icon">
                    <SIcon name={m.icon} size={16} />
                  </span>
                  <span class="mode-text">
                    <b>{modeTitle(m.mode)}</b>
                    <span>{modeBody(m.mode)}</span>
                  </span>
                </label>
              )}
            </For>
          </div>

          <Show when={f.form.mode !== "paused"}>
            <div class="subsection">
              <div class="subsection-head">
                <h3>{t().automation.pause_title}</h3>
                <Switch checked={f.form.pause} onChange={(v) => f.set("pause", v)} disabled={!f.canEdit()} label={t().automation.pause_toggle} />
              </div>
              <p class="muted xs">{t().automation.pause_hint}</p>
              <Show when={expiredPause()}>
                {(at) => (
                  <div class="callout warn" style={{ "margin-top": "10px" }}>
                    <SIcon name="clock" size={16} />
                    <span>{tpl(t().automation.pause_expired, { time: fmtDual(at(), true) })}</span>
                  </div>
                )}
              </Show>
              <Show when={f.form.pause}>
                <div class="pause-grid">
                  <Field id="auto-until" label={t().automation.pause_until} error={f.error("until")} changed={f.changed("until")}>
                    <input
                      id="auto-until"
                      type="datetime-local"
                      class="input"
                      classList={{ invalid: !!f.error("until") }}
                      aria-invalid={f.error("until") ? "true" : "false"}
                      value={f.form.until}
                      disabled={!f.canEdit()}
                      onInput={(e) => f.set("until", e.currentTarget.value)}
                      onBlur={() => f.touch("until")}
                    />
                  </Field>
                  <div class="field">
                    <span class="field-label">{t().automation.zone_label}</span>
                    <Segmented
                      value={f.form.zone}
                      onChange={(z) => setZone(z)}
                      options={[
                        { value: "local", label: tpl(t().automation.zone_local, { zone: zoneLabel(displayTz()) }) },
                        { value: "market", label: t().automation.zone_market },
                      ]}
                    />
                  </div>
                </div>
                <div class="quick-row">
                  <Show when={market()}>
                    {(m) => (
                      <button type="button" class="btn sm" disabled={!f.canEdit()} onClick={() => quick(new Date(m().next_session.close).getTime())}>
                        {tpl(t().automation.quick_session, { date: m().next_session.date.slice(5) })}
                      </button>
                    )}
                  </Show>
                  <button type="button" class="btn sm" disabled={!f.canEdit()} onClick={() => quick(serverNow() + DAY)}>
                    {t().automation.quick_1d}
                  </button>
                  <button type="button" class="btn sm" disabled={!f.canEdit()} onClick={() => quick(serverNow() + 3 * DAY)}>
                    {t().automation.quick_3d}
                  </button>
                  <button type="button" class="btn sm" disabled={!f.canEdit()} onClick={() => quick(serverNow() + 7 * DAY)}>
                    {t().automation.quick_1w}
                  </button>
                </div>
                <Show when={untilMs() !== null && !f.parsed().errors.until}>
                  <p class="small pause-result">
                    <SIcon name="clock" size={14} />
                    {tpl(t().automation.pause_result, { time: fmtDual(untilMs(), true), mode: c().mode[f.form.mode] })}
                  </p>
                </Show>
              </Show>
            </div>
          </Show>

          <Field id="auto-note" label={t().automation.note} hint={t().automation.note_hint}>
            <textarea
              id="auto-note"
              class="textarea"
              rows={2}
              maxLength={500}
              placeholder={t().automation.note_placeholder}
              value={f.form.note}
              disabled={!f.canEdit()}
              onInput={(e) => f.set("note", e.currentTarget.value)}
            />
          </Field>
        </div>
      </FormCard>
    </>
  );
}
