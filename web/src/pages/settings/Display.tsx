import { Show, createSignal } from "solid-js";
import { Segmented, toast } from "@/components/ui";
import { locale, setLocale } from "@/i18n";
import { common } from "@/i18n/common";
import { settingsText } from "@/i18n/settings";
import { api } from "@/lib/api";
import { fmtDual, fmtPct } from "@/lib/format";
import { type ThemePref, type UpDown, displayTz, setDisplayTz, setThemePref, setUpDown, themePref, upDown } from "@/lib/prefs";
import { loadMeta, serverNow } from "@/lib/session";
import type { DisplaySettings } from "@/lib/types";
import { type Errors, Field, FormCard, Gate, SIcon, createSectionForm, useSettings } from "./shared";
import { canonicalTimeZone } from "./time";
import { TimeZonePicker, timeZoneError } from "./TimeZonePicker";

type Form = { timezone: string; up_color: DisplaySettings["up_color"] };

function parse(f: Form, base: DisplaySettings): { value: DisplaySettings | null; errors: Errors } {
  const tz = canonicalTimeZone(f.timezone);
  if (!tz) return { value: null, errors: { timezone: timeZoneError(f.timezone) } };
  return { value: { ...base, timezone: tz, up_color: f.up_color }, errors: {} };
}

export default function DisplaySection() {
  const { bundle } = useSettings();
  return (
    <Gate loader={bundle}>
      {(b) => (
        <>
          <ServerForm source={() => b().display} defaults={() => b().defaults.display} />
          <BrowserPrefs server={() => b().display} />
        </>
      )}
    </Gate>
  );
}

function ServerForm(props: { source: () => DisplaySettings; defaults: () => DisplaySettings }) {
  const t = settingsText;
  const { bundle } = useSettings();
  const f = createSectionForm<DisplaySettings, Form>({
    source: props.source,
    defaults: props.defaults,
    toForm: (v) => ({ timezone: v.timezone, up_color: v.up_color }),
    parse,
    submit: (v) => api.putSettings("display", v),
    after: async () => {
      await bundle.reload();
      void loadMeta();
    },
    fieldFor: (m) => (/time zone/i.test(m) ? "timezone" : null),
    describe: (k) => (k === "timezone" ? t().v.timezone : undefined),
    savedLabel: () => t().display.saved_label,
  });
  return (
    <FormCard f={f} title={t().display.server_title} sub={t().display.server_sub}>
      <div class="form-grid">
        <Field id="disp-tz" label={t().display.timezone} hint={t().display.timezone_hint} error={f.error("timezone")} changed={f.changed("timezone")}>
          <TimeZonePicker
            id="disp-tz"
            value={f.form.timezone}
            onChange={(v) => f.set("timezone", v)}
            onBlur={() => f.touch("timezone")}
            disabled={!f.canEdit()}
            invalid={!!f.error("timezone")}
          />
        </Field>
        <Field label={t().display.up_color} hint={t().display.up_color_hint} changed={f.changed("up_color")}>
          <div>
            <Segmented
              value={f.form.up_color}
              onChange={(v) => f.canEdit() && f.set("up_color", v)}
              options={[
                { value: "green_up", label: t().display.green_up },
                { value: "red_up", label: t().display.red_up },
              ]}
              label={t().display.up_color}
            />
          </div>
        </Field>
      </div>
    </FormCard>
  );
}

function BrowserPrefs(props: { server: () => DisplaySettings }) {
  const t = settingsText;
  const c = common;
  const [tzText, setTzText] = createSignal(displayTz());
  const tzError = () => (canonicalTimeZone(tzText()) ? undefined : timeZoneError(tzText()));
  const onTz = (value: string) => {
    setTzText(value);
    const canonical = canonicalTimeZone(value);
    if (canonical) setDisplayTz(canonical);
  };
  const serverUpDown = (): UpDown => (props.server().up_color === "red_up" ? "red-up" : "green-up");
  const differs = () => displayTz() !== props.server().timezone || upDown() !== serverUpDown();
  const useServer = () => {
    setDisplayTz(props.server().timezone);
    setTzText(props.server().timezone);
    setUpDown(serverUpDown());
    toast(t().display.applied, undefined, "success");
  };
  return (
    <div class="card">
      <div class="card-head">
        <div class="head-text">
          <h2>{t().display.browser_title}</h2>
          <div class="sub">{t().display.browser_sub}</div>
        </div>
        <span class="spacer" />
        <button type="button" class="btn ghost sm" disabled={!differs()} onClick={useServer} title={differs() ? t().display.differs : undefined}>
          <SIcon name="history" size={14} />
          {t().display.use_server}
        </button>
      </div>
      <div class="card-body stack" style={{ gap: "20px" }}>
        <div class="form-grid">
          <Field label={t().display.language}>
            <div>
              <Segmented
                value={locale()}
                onChange={(v) => setLocale(v)}
                options={[
                  { value: "zh", label: "中文" },
                  { value: "en", label: "English" },
                ]}
                label={t().display.language}
              />
            </div>
          </Field>
          <Field label={t().display.theme}>
            <div>
              <Segmented
                value={themePref()}
                onChange={(v) => setThemePref(v as ThemePref)}
                options={[
                  { value: "auto", label: c().prefs.theme_auto },
                  { value: "light", label: c().prefs.theme_light },
                  { value: "dark", label: c().prefs.theme_dark },
                ]}
                label={t().display.theme}
              />
            </div>
          </Field>
          <Field id="pref-updown-local" label={t().display.updown}>
            <select id="pref-updown-local" class="select" value={upDown()} onChange={(e) => setUpDown(e.currentTarget.value as UpDown)}>
              <option value="green-up">{c().prefs.green_up}</option>
              <option value="red-up">{c().prefs.red_up}</option>
              <option value="blue-orange">{c().prefs.blue_orange}</option>
            </select>
          </Field>
          <Field id="pref-tz-local" label={t().display.local_tz} error={tzError()}>
            <TimeZonePicker id="pref-tz-local" value={tzText()} onChange={onTz} invalid={!!tzError()} />
          </Field>
        </div>
        <div class="pref-sample" aria-label={t().display.sample}>
          <span class="kicker">{t().display.sample}</span>
          <span class="sample-item">
            <span class="muted">{t().display.sample_up}</span> <b class="num up">{fmtPct(0.0235, { sign: true })}</b>
          </span>
          <span class="sample-item">
            <span class="muted">{t().display.sample_down}</span> <b class="num down">{fmtPct(-0.0118, { sign: true })}</b>
          </span>
          <span class="sample-item">
            <span class="muted">{t().display.sample_now}</span> <b class="num">{fmtDual(serverNow())}</b>
          </span>
          <Show when={differs()}>
            <span class="chip outline">{t().display.differs}</span>
          </Show>
        </div>
      </div>
    </div>
  );
}
