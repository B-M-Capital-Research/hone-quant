import { For, Show, createMemo, createSignal } from "solid-js";
import { Segmented, Switch, toast } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { settingsText } from "@/i18n/settings";
import { api } from "@/lib/api";
import { serverNow } from "@/lib/session";
import type { NotificationSettings, Severity } from "@/lib/types";
import { ChannelsCard } from "./Channels";
import { type Errors, Field, FormCard, Gate, GroupTitle, SIcon, createSectionForm, useSettings } from "./shared";
import { canonicalTimeZone, wallParts } from "./time";
import { TimeZonePicker, timeZoneError, zoneName } from "./TimeZonePicker";

/** Categories the server routes (`store::settings::CATEGORIES`). */
const CATEGORIES = ["plan", "execution", "risk", "system", "reminder", "report", "data"] as const;
type Category = (typeof CATEGORIES)[number];

interface Form {
  language: "zh" | "en";
  min_severity: Severity;
  categories: Record<string, boolean>;
  quiet_enabled: boolean;
  quiet_start: string;
  quiet_end: string;
  quiet_tz: string;
  critical_bypass: boolean;
  browser: boolean;
}

const TIME = /^([01]\d|2[0-3]):[0-5]\d$/;

/** "730" / "7:30" / "0730" / "7：30" → "07:30"; anything else is left for validation to flag. */
function normalizeTime(value: string): string {
  const s = value.trim().replace(/[：.．]/g, ":");
  const colon = /^(\d{1,2}):(\d{2})$/.exec(s);
  if (colon) return `${colon[1].padStart(2, "0")}:${colon[2]}`;
  const digits = /^(\d{3,4})$/.exec(s);
  if (digits) {
    const d = digits[1].padStart(4, "0");
    return `${d.slice(0, 2)}:${d.slice(2)}`;
  }
  return value;
}

/** 24-hour HH:MM entry (a native time input follows the browser locale and may show AM/PM). */
function TimeInput(props: { id: string; value: string; onChange: (v: string) => void; onBlur: () => void; invalid: boolean; disabled: boolean }) {
  return (
    <div class="input-group time-group">
      <input
        id={props.id}
        class="input num mono"
        classList={{ invalid: props.invalid }}
        aria-invalid={props.invalid ? "true" : "false"}
        inputmode="numeric"
        autocomplete="off"
        spellcheck={false}
        maxLength={5}
        placeholder="HH:MM"
        value={props.value}
        disabled={props.disabled}
        onInput={(e) => props.onChange(e.currentTarget.value)}
        onBlur={(e) => {
          const normalized = normalizeTime(e.currentTarget.value);
          if (normalized !== e.currentTarget.value) props.onChange(normalized);
          props.onBlur();
        }}
      />
      <span class="addon">24h</span>
    </div>
  );
}

function toForm(v: NotificationSettings): Form {
  return {
    language: v.language,
    min_severity: v.min_severity,
    categories: Object.fromEntries([...CATEGORIES.map((c) => [c, v.categories[c] ?? true]), ...Object.entries(v.categories)]),
    quiet_enabled: v.quiet_hours.enabled,
    quiet_start: v.quiet_hours.start,
    quiet_end: v.quiet_hours.end,
    quiet_tz: v.quiet_hours.timezone,
    critical_bypass: v.critical_bypasses_quiet_hours,
    browser: v.browser,
  };
}

function parse(f: Form, base: NotificationSettings): { value: NotificationSettings | null; errors: Errors } {
  const t = settingsText();
  const errors: Errors = {};
  // The server validates start, end and zone even while quiet hours are off.
  if (!TIME.test(f.quiet_start)) errors.quiet_start = t.v.time;
  if (!TIME.test(f.quiet_end)) errors.quiet_end = t.v.time;
  const tz = canonicalTimeZone(f.quiet_tz);
  if (!tz) errors.quiet_tz = timeZoneError(f.quiet_tz);
  const value: NotificationSettings = {
    ...base,
    language: f.language,
    min_severity: f.min_severity,
    categories: { ...base.categories, ...f.categories },
    quiet_hours: { ...base.quiet_hours, enabled: f.quiet_enabled, start: f.quiet_start, end: f.quiet_end, timezone: tz ?? f.quiet_tz },
    critical_bypasses_quiet_hours: f.critical_bypass,
    browser: f.browser,
  };
  return { value: Object.keys(errors).length ? null : value, errors };
}

function fieldFor(message: string): string | null {
  if (/time zone/i.test(message)) return "quiet_tz";
  if (/premature end|invalid|input/i.test(message)) return "quiet_start";
  return null;
}

/** Same rule as `QuietHours::contains`: the window may wrap past midnight. */
function inQuietHours(start: string, end: string, tz: string, now: number): boolean | null {
  if (!TIME.test(start) || !TIME.test(end) || !canonicalTimeZone(tz)) return null;
  const toMin = (s: string) => Number(s.slice(0, 2)) * 60 + Number(s.slice(3, 5));
  const p = wallParts(now, tz);
  const local = p.hh * 60 + p.mm;
  const a = toMin(start);
  const b = toMin(end);
  return a <= b ? local >= a && local < b : local >= a || local < b;
}

export default function NotificationsSection() {
  const { bundle } = useSettings();
  return (
    <>
      <Gate loader={bundle}>{(b) => <PreferencesForm source={() => b().notifications} defaults={() => b().defaults.notifications} />}</Gate>
      <ChannelsCard />
    </>
  );
}

function PreferencesForm(props: { source: () => NotificationSettings; defaults: () => NotificationSettings }) {
  const t = settingsText;
  const c = common;
  const { bundle } = useSettings();
  const f = createSectionForm<NotificationSettings, Form>({
    source: props.source,
    defaults: props.defaults,
    toForm,
    parse,
    submit: (v) => api.putSettings("notifications", v),
    after: () => bundle.reload(),
    fieldFor,
    describe: (k) => (k === "quiet_tz" ? t().v.timezone : k === "quiet_start" || k === "quiet_end" ? t().v.time : undefined),
    savedLabel: () => t().notifications.saved_label,
  });

  const severityOptions = createMemo(() => {
    const n = t().notifications;
    const list: { value: Severity; label: string }[] = [
      { value: "info", label: n.sev_info },
      { value: "warning", label: n.sev_warning },
      { value: "critical", label: n.sev_critical },
    ];
    if (f.form.min_severity === "success" || f.baseline().min_severity === "success") list.splice(1, 0, { value: "success", label: n.sev_success });
    return list;
  });

  const extraCategories = () => Object.keys(f.form.categories).filter((k) => !(CATEGORIES as readonly string[]).includes(k));
  const catDescription = (k: string) => {
    const n = t().notifications as Record<string, string>;
    return n[`cat_${k}`] ?? "";
  };
  const catLabel = (k: string) => (c().category as Record<string, string>)[k] ?? k;

  const quietNow = createMemo(() => inQuietHours(f.form.quiet_start, f.form.quiet_end, f.form.quiet_tz, serverNow()));

  // Browser permission (per browser, not a server setting).
  const readPermission = (): NotificationPermission | "unsupported" =>
    typeof Notification === "undefined" ? "unsupported" : Notification.permission;
  const [permission, setPermission] = createSignal(readPermission());
  const requestPermission = async () => {
    try {
      const result = await Notification.requestPermission();
      setPermission(result);
      if (result === "granted") toast(t().notifications.perm_granted, undefined, "success");
    } catch {
      setPermission(readPermission());
    }
  };
  const permissionText = () => {
    const n = t().notifications;
    const p = permission();
    return p === "granted" ? n.perm_granted : p === "denied" ? n.perm_denied : p === "default" ? n.perm_default : n.perm_unsupported;
  };

  return (
    <FormCard f={f} title={t().notifications.form_title} sub={t().notifications.form_sub}>
      <div class="stack" style={{ gap: "28px" }}>
        <section>
          <GroupTitle>{t().notifications.outbound_title}</GroupTitle>
          <div class="form-grid">
            <Field label={t().notifications.language} hint={t().notifications.language_hint} changed={f.changed("language")}>
              <div>
                <Segmented
                  value={f.form.language}
                  onChange={(v) => f.canEdit() && f.set("language", v)}
                  options={[
                    { value: "zh", label: "中文" },
                    { value: "en", label: "English" },
                  ]}
                  label={t().notifications.language}
                />
              </div>
            </Field>
            <Field id="notif-severity" label={t().notifications.min_severity} hint={t().notifications.min_severity_hint} changed={f.changed("min_severity")}>
              <select
                id="notif-severity"
                class="select"
                value={f.form.min_severity}
                disabled={!f.canEdit()}
                onChange={(e) => f.set("min_severity", e.currentTarget.value as Severity)}
              >
                <For each={severityOptions()}>{(o) => <option value={o.value}>{o.label}</option>}</For>
              </select>
            </Field>
          </div>
        </section>

        <section>
          <GroupTitle sub={t().notifications.categories_sub}>{t().notifications.categories_title}</GroupTitle>
          <div class="toggle-grid">
            <For each={[...CATEGORIES, ...extraCategories()]}>
              {(k) => (
                <div class="toggle-row" classList={{ off: !f.form.categories[k] }}>
                  <div class="toggle-text">
                    <b>
                      {catLabel(k as Category)}
                      <Show when={f.form.categories[k] !== (f.baseline().categories[k] ?? true)}>
                        <span class="changed-dot" title={t().form.changed} />
                      </Show>
                    </b>
                    <span>{catDescription(k)}</span>
                  </div>
                  <Switch
                    checked={!!f.form.categories[k]}
                    disabled={!f.canEdit()}
                    onChange={(v) => f.set("categories", { ...f.form.categories, [k]: v })}
                  />
                </div>
              )}
            </For>
          </div>
        </section>

        <section>
          <GroupTitle>{t().notifications.quiet_title}</GroupTitle>
          <div class="stack" style={{ gap: "14px" }}>
            <Switch
              checked={f.form.quiet_enabled}
              disabled={!f.canEdit()}
              onChange={(v) => f.set("quiet_enabled", v)}
              label={<span class="switch-label">{t().notifications.quiet_enabled}</span>}
            />
            <div class="quiet-grid" classList={{ dim: !f.form.quiet_enabled }}>
              <Field id="quiet-start" label={t().notifications.quiet_start} error={f.error("quiet_start")} changed={f.changed("quiet_start")}>
                <TimeInput
                  id="quiet-start"
                  value={f.form.quiet_start}
                  invalid={!!f.error("quiet_start")}
                  disabled={!f.canEdit()}
                  onChange={(v) => f.set("quiet_start", v)}
                  onBlur={() => f.touch("quiet_start")}
                />
              </Field>
              <Field id="quiet-end" label={t().notifications.quiet_end} error={f.error("quiet_end")} changed={f.changed("quiet_end")}>
                <TimeInput
                  id="quiet-end"
                  value={f.form.quiet_end}
                  invalid={!!f.error("quiet_end")}
                  disabled={!f.canEdit()}
                  onChange={(v) => f.set("quiet_end", v)}
                  onBlur={() => f.touch("quiet_end")}
                />
              </Field>
              <Field id="quiet-tz" label={t().notifications.quiet_tz} error={f.error("quiet_tz")} changed={f.changed("quiet_tz")} class="quiet-tz">
                <TimeZonePicker
                  id="quiet-tz"
                  value={f.form.quiet_tz}
                  onChange={(v) => f.set("quiet_tz", v)}
                  onBlur={() => f.touch("quiet_tz")}
                  disabled={!f.canEdit()}
                  invalid={!!f.error("quiet_tz")}
                />
              </Field>
            </div>
            <p class="muted xs">
              {t().notifications.quiet_hint}
              <Show when={f.form.quiet_enabled && f.form.quiet_start === f.form.quiet_end}>
                {" "}
                <span class="warn-text">{t().notifications.quiet_same}</span>
              </Show>
              <Show when={f.form.quiet_enabled && quietNow() !== null && f.form.quiet_start !== f.form.quiet_end}>
                {" "}
                {tpl(t().notifications.quiet_now, { state: quietNow() ? t().notifications.quiet_in : t().notifications.quiet_out })}{" "}
                <span class="muted">({zoneName(f.form.quiet_tz)})</span>
              </Show>
            </p>
            <div class="toggle-row">
              <div class="toggle-text">
                <b>
                  {t().notifications.critical_bypass}
                  <Show when={f.changed("critical_bypass")}>
                    <span class="changed-dot" title={t().form.changed} />
                  </Show>
                </b>
                <span>{t().notifications.critical_bypass_hint}</span>
              </div>
              <Switch checked={f.form.critical_bypass} disabled={!f.canEdit()} onChange={(v) => f.set("critical_bypass", v)} />
            </div>
          </div>
        </section>

        <section>
          <GroupTitle>{t().notifications.browser_title}</GroupTitle>
          <div class="toggle-row">
            <div class="toggle-text">
              <b>
                {t().notifications.browser}
                <Show when={f.changed("browser")}>
                  <span class="changed-dot" title={t().form.changed} />
                </Show>
              </b>
              <span>{t().notifications.browser_hint}</span>
            </div>
            <Switch checked={f.form.browser} disabled={!f.canEdit()} onChange={(v) => f.set("browser", v)} />
          </div>
          <div class="permission-row">
            <span class="small">
              <SIcon name="bell" size={14} /> {tpl(t().notifications.permission, { state: permissionText() })}
            </span>
            <Show when={permission() === "default"}>
              <button type="button" class="btn sm" onClick={() => void requestPermission()}>
                {t().notifications.request_permission}
              </button>
            </Show>
          </div>
        </section>
      </div>
    </FormCard>
  );
}
