/**
 * Create/edit dialog for reminders. Built-in reminders keep their trigger type (the server
 * enforces it): only the minutes, note and on/off state change. Custom reminders choose
 * once / daily / trading days / weekly with a time zone.
 */
import { For, Match, Show, Switch, createMemo, createSignal, createUniqueId } from "solid-js";
import { Icon } from "@/components/Icon";
import { Dialog, Segmented, Switch as Toggle, toast } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { notificationsText } from "@/i18n/notifications";
import { ApiError, api } from "@/lib/api";
import { fmtRelative } from "@/lib/format";
import { displayTz } from "@/lib/prefs";
import { serverNow } from "@/lib/session";
import type { Reminder, ReminderSchedule } from "@/lib/types";
import { describeSchedule } from "./schedule";
import { addDays, dateIn, isDate, utcToZoned, zonedToUtc } from "./util";

type CustomType = "once" | "daily" | "trading_days" | "weekly";
const CUSTOM_TYPES: CustomType[] = ["once", "daily", "trading_days", "weekly"];
const ZONES = ["Asia/Singapore", "America/New_York", "Asia/Shanghai", "Asia/Hong_Kong", "Asia/Tokyo", "Europe/London", "UTC"];

export function reminderName(r: Reminder): string {
  if (r.kind === "custom") return r.title || "—";
  return (notificationsText().reminders.names as Record<string, string>)[r.kind] ?? r.kind;
}

function isCustomType(type: string): type is CustomType {
  return (CUSTOM_TYPES as string[]).includes(type);
}

function minutesOf(schedule: ReminderSchedule | undefined): number | null {
  if (!schedule) return null;
  if ("minutes_before" in schedule) return schedule.minutes_before;
  if ("minutes_after" in schedule) return schedule.minutes_after;
  return null;
}

export function ReminderDialog(props: { reminder: Reminder | null; onClose: () => void; onSaved: (reminder: Reminder) => void }) {
  const n = notificationsText;
  const formId = createUniqueId();
  const existing = props.reminder;
  const builtin = !!existing && existing.kind !== "custom";
  const initial = existing?.schedule;
  const localTz = displayTz();

  // Initial field values from the reminder being edited (or sensible defaults).
  let initDate = addDays(dateIn(serverNow(), localTz), 1);
  let initTime = "09:00";
  let initTz = localTz;
  let initDays = [5];
  if (initial?.type === "once") {
    const zoned = utcToZoned(Date.parse(initial.at), localTz);
    initDate = zoned.date;
    initTime = zoned.time;
  } else if (initial && (initial.type === "daily" || initial.type === "trading_days" || initial.type === "weekly")) {
    initTime = initial.time;
    initTz = initial.tz;
    if (initial.type === "weekly") initDays = [...initial.weekdays];
  }

  const [title, setTitle] = createSignal(existing?.title ?? "");
  const [note, setNote] = createSignal(existing?.note ?? "");
  const [enabled, setEnabled] = createSignal(existing?.enabled ?? true);
  const [type, setType] = createSignal<CustomType>(initial && isCustomType(initial.type) ? initial.type : "once");
  const [minutes, setMinutes] = createSignal(String(minutesOf(initial) ?? 30));
  const [date, setDate] = createSignal(initDate);
  const [time, setTime] = createSignal(initTime);
  const [tz, setTz] = createSignal(initTz);
  const [weekdays, setWeekdays] = createSignal<number[]>(initDays);
  const [submitted, setSubmitted] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [serverError, setServerError] = createSignal<string | null>(null);

  const errors = createMemo(() => {
    const e = n().reminders.dialog.errors;
    const out: Partial<Record<"title" | "minutes" | "time" | "date" | "weekdays", string>> = {};
    if (builtin) {
      const m = Number(minutes());
      if (!Number.isInteger(m) || m < 1 || m > 600) out.minutes = e.minutes;
      return out;
    }
    const trimmed = title().trim();
    if (!trimmed || [...trimmed].length > 120) out.title = e.title;
    if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(time())) out.time = e.time;
    if (type() === "once") {
      if (!isDate(date())) out.date = e.date;
      else {
        const at = zonedToUtc(date(), time(), tz());
        if (at !== null && at <= serverNow()) out.date = e.past;
      }
    }
    if (type() === "weekly" && weekdays().length === 0) out.weekdays = e.weekdays;
    return out;
  });
  const valid = () => Object.keys(errors()).length === 0;
  const show = (key: keyof ReturnType<typeof errors>) => (submitted() ? errors()[key] : undefined);

  const schedule = (): ReminderSchedule | null => {
    if (builtin && initial) {
      const m = Math.round(Number(minutes()));
      switch (initial.type) {
        case "pre_open":
          return { type: "pre_open", minutes_before: m };
        case "before_deadline":
          return { type: "before_deadline", minutes_before: m };
        case "post_close":
          return { type: "post_close", minutes_after: m };
        case "week_close":
          return { type: "week_close", minutes_after: m };
        default:
          return initial;
      }
    }
    switch (type()) {
      case "once": {
        const at = zonedToUtc(date(), time(), tz());
        return at === null ? null : { type: "once", at: new Date(at).toISOString() };
      }
      case "daily":
        return { type: "daily", time: time(), tz: tz() };
      case "trading_days":
        return { type: "trading_days", time: time(), tz: tz() };
      case "weekly":
        return { type: "weekly", time: time(), tz: tz(), weekdays: [...weekdays()].sort((a, b) => a - b) };
    }
  };

  const preview = () => {
    if (!valid()) return null;
    const s = schedule();
    if (!s) return null;
    const text = describeSchedule(s);
    if (s.type === "once") return tpl(n().reminders.dialog.preview_once, { text, relative: fmtRelative(s.at, serverNow()) });
    return text;
  };

  const toggleDay = (day: number) =>
    setWeekdays((days) => (days.includes(day) ? days.filter((d) => d !== day) : [...days, day].sort((a, b) => a - b)));

  const submit = async (event: Event) => {
    event.preventDefault();
    setSubmitted(true);
    setServerError(null);
    if (!valid() || saving()) return;
    const s = schedule();
    if (!s) return;
    setSaving(true);
    try {
      const body = { title: builtin ? existing!.title : title().trim(), note: note().trim(), schedule: s, enabled: enabled() };
      const res = existing ? await api.updateReminder(existing.id, body) : await api.createReminder(body);
      toast(tpl(existing ? n().reminders.dialog.saved : n().reminders.dialog.created, { name: reminderName(res.reminder) }), undefined, "success");
      props.onSaved(res.reminder);
    } catch (error) {
      setServerError(error instanceof ApiError ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  };

  const zones = () => (ZONES.includes(tz()) ? ZONES : [...ZONES, tz()]);
  const zoneName = (zone: string) => (n().reminders.tz as Record<string, string>)[zone] ?? zone;

  return (
    <Dialog
      title={existing ? (builtin ? tpl(n().reminders.dialog.edit_builtin, { name: reminderName(existing) }) : n().reminders.dialog.edit) : n().reminders.dialog.create}
      subtitle={builtin ? n().reminders.dialog.builtin_note : undefined}
      onClose={props.onClose}
      footer={
        <>
          <button type="button" class="btn" onClick={props.onClose}>
            {common().actions.cancel}
          </button>
          <button type="submit" form={formId} class="btn primary" disabled={saving() || (submitted() && !valid())}>
            {saving() ? common().actions.saving : existing ? n().reminders.dialog.save_btn : n().reminders.dialog.create_btn}
          </button>
        </>
      }
    >
      <form id={formId} class="rem-form" onSubmit={submit} novalidate>
        <Show when={!builtin}>
          <div class="field">
            <label for={`${formId}-title`}>{n().reminders.dialog.title}</label>
            <input
              id={`${formId}-title`}
              class="input"
              classList={{ invalid: !!show("title") }}
              maxLength={120}
              placeholder={n().reminders.dialog.title_ph}
              value={title()}
              onInput={(e) => setTitle(e.currentTarget.value)}
              autofocus
            />
            <Show when={show("title")}>
              <span class="error-text">{show("title")}</span>
            </Show>
          </div>

          <div class="field">
            <span class="field-label">{n().reminders.dialog.type}</span>
            <Segmented
              label={n().reminders.dialog.type}
              value={type()}
              onChange={setType}
              options={CUSTOM_TYPES.map((value) => ({ value, label: n().reminders.dialog.types[value] }))}
            />
            <span class="hint">{n().reminders.dialog.type_hints[type()]}</span>
          </div>

          <Show when={type() === "weekly"}>
            <div class="field">
              <span class="field-label">{n().reminders.dialog.weekdays}</span>
              <div class="rem-days" role="group" aria-label={n().reminders.dialog.weekdays}>
                <For each={[1, 2, 3, 4, 5, 6, 7]}>
                  {(day) => (
                    <button type="button" class="rem-day" aria-pressed={weekdays().includes(day)} onClick={() => toggleDay(day)}>
                      {n().reminders.weekdays[day - 1]}
                    </button>
                  )}
                </For>
              </div>
              <Show when={show("weekdays")}>
                <span class="error-text">{show("weekdays")}</span>
              </Show>
            </div>
          </Show>

          <div class="rem-grid" classList={{ three: type() === "once" }}>
            <Show when={type() === "once"}>
              <div class="field">
                <label for={`${formId}-date`}>{n().reminders.dialog.date}</label>
                <input
                  id={`${formId}-date`}
                  type="date"
                  class="input"
                  classList={{ invalid: !!show("date") }}
                  value={date()}
                  onInput={(e) => setDate(e.currentTarget.value)}
                />
              </div>
            </Show>
            <div class="field">
              <label for={`${formId}-time`}>{n().reminders.dialog.time}</label>
              <input
                id={`${formId}-time`}
                type="time"
                step="60"
                class="input"
                classList={{ invalid: !!show("time") }}
                value={time()}
                onInput={(e) => setTime(e.currentTarget.value.slice(0, 5))}
              />
            </div>
            <div class="field">
              <label for={`${formId}-tz`}>{n().reminders.dialog.tz}</label>
              <select id={`${formId}-tz`} class="select" value={tz()} onChange={(e) => setTz(e.currentTarget.value)}>
                <For each={zones()}>{(zone) => <option value={zone}>{zoneName(zone)}</option>}</For>
              </select>
            </div>
          </div>
          <Show when={show("date") || show("time")}>
            <span class="error-text rem-error">{show("date") ?? show("time")}</span>
          </Show>
        </Show>

        <Show when={builtin && initial}>
          <div class="field">
            <label for={`${formId}-minutes`}>
              {initial!.type === "pre_open" || initial!.type === "before_deadline"
                ? n().reminders.dialog.minutes_before
                : n().reminders.dialog.minutes_after}
            </label>
            <div class="input-group rem-minutes">
              <input
                id={`${formId}-minutes`}
                type="number"
                min="1"
                max="600"
                step="1"
                inputMode="numeric"
                class="input num"
                classList={{ invalid: !!show("minutes") }}
                value={minutes()}
                onInput={(e) => setMinutes(e.currentTarget.value)}
              />
              <span class="addon">{common().units.minutes}</span>
            </div>
            <Switch>
              <Match when={show("minutes")}>
                <span class="error-text">{show("minutes")}</span>
              </Match>
              <Match when={initial!.type === "post_close" || initial!.type === "week_close"}>
                <span class="hint">{n().reminders.dialog.post_close_hint}</span>
              </Match>
              <Match when={true}>
                <span class="hint">{n().reminders.dialog.minutes_hint}</span>
              </Match>
            </Switch>
          </div>
        </Show>

        <div class="field">
          <label for={`${formId}-note`}>{n().reminders.dialog.note}</label>
          <textarea
            id={`${formId}-note`}
            class="textarea"
            rows={3}
            maxLength={1000}
            placeholder={n().reminders.dialog.note_ph}
            value={note()}
            onInput={(e) => setNote(e.currentTarget.value)}
          />
        </div>

        <Toggle checked={enabled()} onChange={setEnabled} label={n().reminders.dialog.enabled} />

        <Show when={preview()}>
          <div class="rem-preview">
            <span class="kicker">{n().reminders.dialog.preview}</span>
            <span>{preview()}</span>
          </div>
        </Show>

        <Show when={serverError()}>
          <div class="callout critical" role="alert">
            <Icon name="alert" size={16} />
            <span>{serverError()}</span>
          </div>
        </Show>
      </form>
    </Dialog>
  );
}
