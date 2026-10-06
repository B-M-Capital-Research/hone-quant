import { Show, createMemo } from "solid-js";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { settingsText } from "@/i18n/settings";
import { api } from "@/lib/api";
import { fmtPct } from "@/lib/format";
import type { RiskSettings } from "@/lib/types";
import { type NumSpec, parseNum, rangeMessage, toText } from "./num";
import { type Errors, Field, FormCard, Gate, NumInput, SIcon, createLoader, createSectionForm, useSettings, withDefault } from "./shared";

type Key = keyof RiskSettings;
type Form = Record<Key, string>;

const KEYS: Key[] = ["drawdown_alert", "daily_loss_alert"];
const pct = () => settingsText().units.pct;
/** Ranges mirror `RiskSettings::validate`. */
const SPECS: Record<Key, NumSpec> = {
  drawdown_alert: { min: 1, max: 90, div: 100, unit: pct },
  daily_loss_alert: { min: 0.5, max: 50, div: 100, unit: pct },
};

function toForm(v: RiskSettings): Form {
  return { drawdown_alert: toText(v.drawdown_alert, SPECS.drawdown_alert), daily_loss_alert: toText(v.daily_loss_alert, SPECS.daily_loss_alert) };
}

function parse(f: Form, base: RiskSettings): { value: RiskSettings | null; errors: Errors } {
  const errors: Errors = {};
  const out: RiskSettings = { ...base };
  for (const k of KEYS) {
    const r = parseNum(f[k], SPECS[k]);
    if (r.ok) out[k] = r.stored;
    else errors[k] = r.error;
  }
  return { value: Object.keys(errors).length ? null : out, errors };
}

export default function RiskSection() {
  const { bundle } = useSettings();
  return <Gate loader={bundle}>{(b) => <RiskForm source={() => b().risk} defaults={() => b().defaults.risk} />}</Gate>;
}

function RiskForm(props: { source: () => RiskSettings; defaults: () => RiskSettings }) {
  const t = settingsText;
  const { bundle } = useSettings();
  const f = createSectionForm<RiskSettings, Form>({
    source: props.source,
    defaults: props.defaults,
    toForm,
    parse,
    submit: (v) => api.putSettings("risk", v),
    after: () => bundle.reload(),
    fieldFor: (m) => KEYS.find((k) => m.includes(k)) ?? null,
    describe: (k) => (k in SPECS ? rangeMessage(SPECS[k as Key]) : undefined),
    savedLabel: () => t().risk.saved_label,
  });

  const dash = createLoader(() => api.dashboard());

  const threshold = (k: Key) => {
    const r = parseNum(f.form[k], SPECS[k]);
    return r.ok ? r.stored : f.baseline()[k];
  };

  const field = (k: Key, label: string, hint: string) => (
    <Field id={`risk-${k}`} label={label} hint={withDefault(hint, fmtPct(props.defaults()[k], { dp: k === "daily_loss_alert" ? 1 : 0 }))} error={f.error(k)} changed={f.changed(k)}>
      <NumInput
        id={`risk-${k}`}
        value={f.form[k]}
        onInput={(v) => f.set(k, v)}
        onBlur={() => f.touch(k)}
        unit="%"
        invalid={!!f.error(k)}
        disabled={!f.canEdit()}
      />
    </Field>
  );

  return (
    <FormCard f={f} title={t().risk.form_title} sub={t().risk.form_sub}>
      <div class="stack" style={{ gap: "22px" }}>
        <div class="form-grid">
          {field("drawdown_alert", t().risk.drawdown, t().risk.drawdown_hint)}
          {field("daily_loss_alert", t().risk.daily_loss, t().risk.daily_loss_hint)}
        </div>
        <div class="subsection">
          <div class="subsection-head">
            <h3>{t().risk.now_title}</h3>
          </div>
          <Show
            when={dash.value()}
            fallback={<p class="muted small">{dash.error() ? t().risk.unavailable : common().states.loading}</p>}
          >
            {(d) => (
              <div class="meters">
                <Meter label={t().risk.now_drawdown} value={d().valuation.drawdown} threshold={threshold("drawdown_alert")} />
                <Meter label={t().risk.now_day} value={d().valuation.day_return} threshold={threshold("daily_loss_alert")} />
              </div>
            )}
          </Show>
        </div>
        <div class="callout info">
          <SIcon name="info" size={16} />
          <span>{t().risk.note}</span>
        </div>
      </div>
    </FormCard>
  );
}

/** How close a (negative) portfolio move is to its alert threshold. */
function Meter(props: { label: string; value: number | null; threshold: number }) {
  const t = settingsText;
  const loss = createMemo(() => (props.value === null ? null : Math.max(0, -props.value)));
  const ratio = () => {
    const l = loss();
    return l === null || props.threshold <= 0 ? 0 : Math.min(1, l / props.threshold);
  };
  const tone = () => (ratio() >= 1 ? "red" : ratio() >= 0.7 ? "orange" : "neutral");
  return (
    <div class={`meter ${tone()}`}>
      <div class="meter-top">
        <span class="muted small">{props.label}</span>
        <b class="num">{fmtPct(props.value, { sign: true })}</b>
        <span class="spacer" />
        <Show when={ratio() >= 1}>
          <span class="chip red">{t().risk.triggered}</span>
        </Show>
        <span class="muted xs num">{tpl(t().risk.of_threshold, { value: `−${fmtPct(props.threshold, { dp: props.threshold < 0.01 ? 2 : 1 })}` })}</span>
      </div>
      <div class="meter-track" role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(ratio() * 100)} aria-label={props.label}>
        <div class="meter-fill" style={{ width: `${ratio() * 100}%` }} />
      </div>
    </div>
  );
}
