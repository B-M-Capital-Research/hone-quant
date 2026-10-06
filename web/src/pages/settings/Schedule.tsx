import { For, Show, createMemo } from "solid-js";
import { common } from "@/i18n/common";
import { locale, tpl } from "@/i18n";
import { settingsText } from "@/i18n/settings";
import { api } from "@/lib/api";
import { fmtDual, fmtTime, marketToday, zoneLabel } from "@/lib/format";
import { displayTz } from "@/lib/prefs";
import { market, serverNow } from "@/lib/session";
import type { ScheduleSettings } from "@/lib/types";
import { type NumSpec, parseNum, rangeMessage, toText } from "./num";
import { type Errors, Field, FormCard, Gate, NumInput, SIcon, createSectionForm, useSettings, withDefault } from "./shared";
import { type DayPreview, NY, type SessionTimes, type SlotPreview, daySchedule, nextEarlyClose, sessionOn, wallDate, wallParts } from "./time";

type Key = keyof ScheduleSettings;
type Form = Record<Key, string>;

const KEYS: Key[] = ["open_offset_minutes", "close_offset_minutes", "review_minutes", "min_gap_minutes"];

const minutes = () => settingsText().units.minutes;
const SPECS: Record<Key, NumSpec> = {
  open_offset_minutes: { min: 0, max: 180, int: true, unit: minutes },
  close_offset_minutes: { min: 30, max: 180, int: true, unit: minutes },
  review_minutes: { min: 0, max: 60, int: true, unit: minutes },
  min_gap_minutes: { min: 0, max: 240, int: true, unit: minutes },
};

export function weekdayLabel(date: string): string {
  return new Date(`${date}T12:00:00Z`).toLocaleDateString(locale() === "zh" ? "zh-CN" : "en-US", { weekday: "short", timeZone: "UTC" });
}

function toForm(v: ScheduleSettings): Form {
  return Object.fromEntries(KEYS.map((k) => [k, toText(v[k], SPECS[k])])) as Form;
}

function parse(f: Form, base: ScheduleSettings): { value: ScheduleSettings | null; errors: Errors } {
  const errors: Errors = {};
  const out: ScheduleSettings = { ...base };
  for (const k of KEYS) {
    const r = parseNum(f[k], SPECS[k]);
    if (r.ok) out[k] = r.stored;
    else errors[k] = r.error;
  }
  return { value: Object.keys(errors).length ? null : out, errors };
}

/** Maps a server message ("close_offset_minutes must be …") to the field it names. */
function fieldFor(message: string): string | null {
  return KEYS.find((k) => message.includes(k)) ?? null;
}

export default function ScheduleSection() {
  const { bundle } = useSettings();
  return <Gate loader={bundle}>{(b) => <ScheduleForm source={() => b().schedule} defaults={() => b().defaults.schedule} />}</Gate>;
}

function ScheduleForm(props: { source: () => ScheduleSettings; defaults: () => ScheduleSettings }) {
  const t = settingsText;
  const { bundle } = useSettings();
  const f = createSectionForm<ScheduleSettings, Form>({
    source: props.source,
    defaults: props.defaults,
    toForm,
    parse,
    submit: (v) => api.putSettings("schedule", v),
    after: () => bundle.reload(),
    fieldFor,
    describe: (k) => (k in SPECS ? rangeMessage(SPECS[k as Key]) : undefined),
    savedLabel: () => t().sections.schedule.title,
  });

  /** Valid form values, falling back to the saved value field by field, for the live preview. */
  const effective = createMemo<ScheduleSettings>(() => {
    const out = { ...f.baseline() };
    for (const k of KEYS) {
      const r = parseNum(f.form[k], SPECS[k]);
      if (r.ok) out[k] = r.stored;
    }
    return out;
  });

  /** The session whose schedule is shown — same rule as the server's market view. */
  const session = createMemo<{ s: SessionTimes; today: boolean } | null>(() => {
    const m = market();
    if (!m) return null;
    const now = serverNow();
    const today = m.today && now < new Date(m.today.close).getTime() + 8 * 3600_000 ? m.today : null;
    const pick = today ?? m.next_session;
    return {
      s: { date: pick.date, open: new Date(pick.open).getTime(), close: new Date(pick.close).getTime(), early_close: pick.early_close },
      today: !!today,
    };
  });

  const day = createMemo(() => {
    const s = session();
    return s ? daySchedule(s.s, effective()) : null;
  });

  const earlyDate = createMemo(() => nextEarlyClose(marketToday(new Date(serverNow()))));
  const earlyDay = createMemo(() => {
    const s = sessionOn(earlyDate(), true);
    return s ? daySchedule(s, effective()) : null;
  });

  const atTime = (k: "open_offset_minutes" | "close_offset_minutes") => {
    const d = day();
    if (!d) return undefined;
    const slot = d.slots.find((s) => s.slot === (k === "open_offset_minutes" ? "open" : "close"));
    if (!slot || !parseNum(f.form[k], SPECS[k]).ok) return undefined;
    return tpl(t().schedule.at_time, { time: `${fmtTime(slot.generate_at, NY)} ET` });
  };

  const num = (k: Key, label: string, hint: string, prefix?: string, aside?: () => string | undefined) => (
    <Field
      id={`sched-${k}`}
      label={label}
      hint={withDefault(hint, `${props.defaults()[k]}${t().units.minutes}`)}
      error={f.error(k)}
      changed={f.changed(k)}
      aside={aside?.()}
    >
      <NumInput
        id={`sched-${k}`}
        value={f.form[k]}
        onInput={(v) => f.set(k, v)}
        onBlur={() => f.touch(k)}
        prefix={prefix}
        unit={t().units.min_short}
        int
        invalid={!!f.error(k)}
        disabled={!f.canEdit()}
      />
    </Field>
  );

  return (
    <>
      <FormCard f={f} title={t().schedule.form_title} sub={t().schedule.form_sub}>
        <div class="form-grid">
          {num("open_offset_minutes", t().schedule.open_offset, t().schedule.open_hint, t().schedule.open_prefix, () => atTime("open_offset_minutes"))}
          {num("close_offset_minutes", t().schedule.close_offset, t().schedule.close_hint, t().schedule.close_prefix, () => atTime("close_offset_minutes"))}
          {num("review_minutes", t().schedule.review, t().schedule.review_hint)}
          {num("min_gap_minutes", t().schedule.min_gap, t().schedule.min_gap_hint)}
        </div>
      </FormCard>

      <div class="card">
        <div class="card-head">
          <div class="head-text">
            <h2>{t().schedule.preview_title}</h2>
            <div class="sub">
              <Show when={session()} fallback={t().schedule.preview_basis}>
                {(s) => (
                  <>
                    {tpl(s().today ? t().schedule.preview_today : t().schedule.preview_next, {
                      date: s().s.date,
                      weekday: weekdayLabel(s().s.date),
                    })}
                    {" · "}
                    {t().schedule.preview_basis}
                  </>
                )}
              </Show>
            </div>
          </div>
          <span class="spacer" />
          <Show when={f.dirty()}>
            <span class="chip orange">
              <span class="dot" />
              {t().schedule.preview_unsaved}
            </span>
          </Show>
        </div>
        <div class="card-body">
          <Show when={day()} fallback={<p class="muted small">{t().schedule.no_market}</p>}>
            {(d) => (
              <div class="stack" style={{ gap: "18px" }}>
                <SessionLine day={d()} />
                <SessionBar day={d()} />
                <Timetable day={d()} gap={effective().min_gap_minutes} />
                <Show when={market()?.effective_mode === "approval"}>
                  <div class="callout info">
                    <SIcon name="info" size={16} />
                    <span>{t().schedule.approval_now}</span>
                  </div>
                </Show>
                <Show when={market()?.effective_mode === "paused"}>
                  <div class="callout warn">
                    <SIcon name="pause" size={16} />
                    <span>{t().schedule.paused_now}</span>
                  </div>
                </Show>
              </div>
            )}
          </Show>
        </div>
      </div>

      <div class="card">
        <div class="card-head">
          <div class="head-text">
            <h2>{t().schedule.early_title}</h2>
            <div class="sub">{tpl(t().schedule.early_next, { date: earlyDate(), weekday: weekdayLabel(earlyDate()) })}</div>
          </div>
        </div>
        <div class="card-body stack" style={{ gap: "16px" }}>
          <p class="small subtle">{t().schedule.early_body}</p>
          <Show when={earlyDay()}>
            {(d) => (
              <>
                <SessionBar day={d()} compact />
                <div class="callout" classList={{ ok: d().slots.length === 2, info: d().slots.length < 2 }}>
                  <SIcon name={d().slots.length === 2 ? "check" : "info"} size={16} />
                  <span>
                    {d().slots.length === 2
                      ? t().schedule.early_both
                      : tpl(t().schedule.early_open_only, {
                          reason:
                            d().closeSkipped === "deadline"
                              ? t().schedule.skipped_deadline
                              : tpl(t().schedule.skipped_gap, { gap: effective().min_gap_minutes }),
                        })}
                  </span>
                </div>
              </>
            )}
          </Show>
        </div>
      </div>

      <div class="card">
        <div class="card-head">
          <div class="head-text">
            <h2>{t().schedule.rules_title}</h2>
          </div>
        </div>
        <div class="card-body">
          <ul class="rule-list">
            <li>
              <SIcon name="clock" size={15} />
              <span>{t().schedule.rule_deadline}</span>
            </li>
            <li>
              <SIcon name="alert" size={15} />
              <span>{t().schedule.rule_offline}</span>
            </li>
            <li>
              <SIcon name="calendar" size={15} />
              <span>{t().schedule.rule_calendar}</span>
            </li>
            <li>
              <SIcon name="pause" size={15} />
              <span>{t().schedule.rule_paused}</span>
            </li>
          </ul>
        </div>
      </div>
    </>
  );
}

function SessionLine(props: { day: DayPreview }) {
  const t = settingsText;
  return (
    <div class="session-line small">
      <span>
        <span class="muted">{t().schedule.open} </span>
        <b class="num">{fmtDual(props.day.session.open)}</b>
      </span>
      <span class="sep" />
      <span>
        <span class="muted">{t().schedule.close} </span>
        <b class="num">{fmtDual(props.day.session.close)}</b>
      </span>
      <Show when={props.day.session.early_close}>
        <span class="chip yellow">{t().schedule.early_tag}</span>
      </Show>
    </div>
  );
}

/** Horizontal session timeline: plan windows, generation and execution pins, the deadline. */
export function SessionBar(props: { day: DayPreview; compact?: boolean }) {
  const t = settingsText;
  const c = common;
  const s = () => props.day.session;
  const span = () => Math.max(1, s().close - s().open);
  const pct = (at: number) => Math.min(100, Math.max(0, ((at - s().open) / span()) * 100));
  const ticks = createMemo(() => {
    const out: number[] = [];
    const p = wallParts(s().open, NY);
    let at = s().open + ((60 - p.mm) % 60) * 60_000;
    if (at === s().open) at += 3600_000;
    for (; at < s().close - 20 * 60_000; at += 3600_000) out.push(at);
    return out;
  });
  const anchor = (p: number) => (p < 12 ? "start" : p > 88 ? "end" : "middle");
  const openSlot = () => props.day.slots.find((x) => x.slot === "open");
  const closeSlot = () => props.day.slots.find((x) => x.slot === "close");
  const deadline = () => s().close - 5 * 60_000;
  return (
    <div class="sbar" classList={{ compact: !!props.compact }} role="img" aria-label={t().schedule.preview_title}>
      <div class="sbar-labels top">
        <Show when={openSlot()}>
          {(slot) => (
            <span class={`sbar-label open ${anchor(pct(slot().generate_at))}`} style={{ left: `${pct(slot().generate_at)}%` }}>
              {c().slot.open} · <b class="num">{fmtTime(slot().generate_at, NY)}</b>
            </span>
          )}
        </Show>
      </div>
      <div class="sbar-track">
        <Show when={openSlot()}>
          {(slot) => (
            <span
              class="sbar-win open"
              title={t().schedule.window_open}
              style={{ left: `${pct(slot().window_start)}%`, width: `${pct(slot().window_end) - pct(slot().window_start)}%` }}
            />
          )}
        </Show>
        <Show when={closeSlot()}>
          {(slot) => (
            <span
              class="sbar-win close"
              title={t().schedule.window_close}
              style={{ left: `${pct(slot().window_start)}%`, width: `${pct(slot().window_end) - pct(slot().window_start)}%` }}
            />
          )}
        </Show>
        <span class="sbar-deadline" title={t().schedule.deadline_marker} style={{ left: `${pct(deadline())}%` }} />
        <For each={props.day.slots}>
          {(slot) => (
            <>
              <span class={`sbar-pin gen ${slot.slot}`} style={{ left: `${pct(slot.generate_at)}%` }} title={`${c().slot[slot.slot]} · ${t().schedule.col_generate} ${fmtDual(slot.generate_at)}`} />
              <Show when={pct(slot.execute_at) - pct(slot.generate_at) >= 1.2}>
                <span class={`sbar-pin exec ${slot.slot}`} style={{ left: `${pct(slot.execute_at)}%` }} title={`${c().slot[slot.slot]} · ${t().schedule.col_execute} ${fmtDual(slot.execute_at)}`} />
              </Show>
            </>
          )}
        </For>
      </div>
      <div class="sbar-labels bottom">
        <Show when={closeSlot()}>
          {(slot) => (
            <span class={`sbar-label close ${anchor(pct(slot().generate_at))}`} style={{ left: `${pct(slot().generate_at)}%` }}>
              {c().slot.close} · <b class="num">{fmtTime(slot().generate_at, NY)}</b>
            </span>
          )}
        </Show>
      </div>
      <div class="sbar-axis">
        <span class="start">{fmtTime(s().open, NY)}</span>
        <For each={ticks()}>
          {(at, i) => (
            <span class="tick" classList={{ odd: i() % 2 === 1, "near-edge": pct(at) < 16 || pct(at) > 84 }} style={{ left: `${pct(at)}%` }}>
              {fmtTime(at, NY)}
            </span>
          )}
        </For>
        <span class="end">{fmtTime(s().close, NY)} ET</span>
      </div>
    </div>
  );
}

/** "22:00 SGT" over "10:00 ET", with the local date when it differs from the trade date. */
function TimeCell(props: { at: number; tradeDate: string; inline?: boolean }) {
  const local = () => displayTz();
  const localDate = () => wallDate(props.at, local());
  return (
    <div class="tcell" classList={{ inline: !!props.inline }} title={fmtDual(props.at, true)}>
      <span class="t-main num">
        <Show when={localDate() !== props.tradeDate}>
          <span class="date">{localDate().slice(5)} </span>
        </Show>
        {fmtTime(props.at, local())} <span class="zone">{zoneLabel(local())}</span>
      </span>
      <Show when={local() !== NY}>
        <span class="alt num">{fmtTime(props.at, NY)} ET</span>
      </Show>
    </div>
  );
}

function Timetable(props: { day: DayPreview; gap: number }) {
  const t = settingsText;
  const c = common;
  const rows = () => (["open", "close"] as const).map((slot) => ({ slot, data: props.day.slots.find((s) => s.slot === slot) ?? null }));
  const cell = (slot: SlotPreview, key: "generate_at" | "execute_at" | "deadline", label: string) => (
    <div class="tl-row">
      <span class="muted xs">{label}</span>
      <TimeCell at={slot[key]} tradeDate={props.day.session.date} inline />
    </div>
  );
  return (
    <>
    <div class="timetable-list">
      <For each={rows()}>
        {(row) => (
          <div class="tl-card">
            <span class="plan-name">
              <span class={`legend-dot ${row.slot}`} />
              {c().slot[row.slot]}
            </span>
            <Show
              when={row.data}
              fallback={
                <p class="small">
                  <span class="chip">{t().schedule.not_run}</span>{" "}
                  <span class="muted">
                    {props.day.closeSkipped === "deadline" ? t().schedule.skipped_deadline : tpl(t().schedule.skipped_gap, { gap: props.gap })}
                  </span>
                </p>
              }
            >
              {(slot) => (
                <>
                  {cell(slot(), "generate_at", t().schedule.col_generate)}
                  {cell(slot(), "execute_at", t().schedule.col_execute)}
                  {cell(slot(), "deadline", t().schedule.col_deadline)}
                </>
              )}
            </Show>
          </div>
        )}
      </For>
    </div>
    <div class="table-wrap timetable-wrap">
      <table class="table timetable">
        <thead>
          <tr>
            <th>{t().schedule.col_plan}</th>
            <th>{t().schedule.col_generate}</th>
            <th>{t().schedule.col_execute}</th>
            <th>{t().schedule.col_deadline}</th>
          </tr>
        </thead>
        <tbody>
          <For each={rows()}>
            {(row) => (
              <tr>
                <td>
                  <span class="plan-name">
                    <span class={`legend-dot ${row.slot}`} />
                    {c().slot[row.slot]}
                  </span>
                </td>
                <Show
                  when={row.data}
                  fallback={
                    <td colSpan={3}>
                      <span class="chip">{t().schedule.not_run}</span>{" "}
                      <span class="muted small">
                        {props.day.closeSkipped === "deadline" ? t().schedule.skipped_deadline : tpl(t().schedule.skipped_gap, { gap: props.gap })}
                      </span>
                    </td>
                  }
                >
                  {(slot) => (
                    <>
                      <td>
                        <TimeCell at={slot().generate_at} tradeDate={props.day.session.date} />
                      </td>
                      <td>
                        <TimeCell at={slot().execute_at} tradeDate={props.day.session.date} />
                      </td>
                      <td>
                        <TimeCell at={slot().deadline} tradeDate={props.day.session.date} />
                      </td>
                    </>
                  )}
                </Show>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
    </>
  );
}
