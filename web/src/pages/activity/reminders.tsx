/**
 * Reminders: built-in (market-calendar) and custom ones, with schedule, next and last firing,
 * on/off switch, edit dialog and deletion of custom reminders (admin only).
 */
import { A } from "@solidjs/router";
import { For, Match, Show, Switch, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { Icon } from "@/components/Icon";
import { Empty, ErrorState, Loading, Switch as Toggle, confirmAction, toast, toastError } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { notificationsText } from "@/i18n/notifications";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { DASH, fmtDual, fmtRelative, zoneLabel } from "@/lib/format";
import { isAdmin, market, serverNow } from "@/lib/session";
import type { JobRun, Reminder } from "@/lib/types";
import { marketDate } from "./components";
import { XIcon } from "./icons";
import { useQuery } from "./query";
import { ReminderDialog, reminderName } from "./reminder-dialog";
import { BUILTIN_JOB, BUILTIN_KINDS, type NextFire, builtinNextFire, describeSchedule } from "./schedule";
import { debounce } from "./util";
import { actorName } from "@/lib/names";

export function Reminders() {
  const n = notificationsText;
  const [tick, setTick] = createSignal(0);
  const [minute, setMinute] = createSignal(0);
  const [editing, setEditing] = createSignal<Reminder | "new" | null>(null);
  const [busy, setBusy] = createSignal<number | null>(null);

  const reminders = useQuery(
    () => ({ tick: tick() }),
    () => api.reminders(),
  );
  /** Latest scheduler runs of each built-in reminder (their "last fired"). */
  const runs = useQuery(
    () => ({ tick: tick() }),
    async () => {
      const entries = await Promise.all(
        BUILTIN_KINDS.map((kind) =>
          api
            .jobs({ job: BUILTIN_JOB[kind], limit: kind === "plan_review" ? 50 : 1 })
            .then((r) => [kind, r.jobs] as const)
            .catch(() => [kind, [] as JobRun[]] as const),
        ),
      );
      return Object.fromEntries(entries) as Record<string, JobRun[]>;
    },
  );
  const pending = useQuery(
    () => ({ tick: tick() }),
    () => api.plans({ status: "pending", limit: 20 }).catch(() => ({ plans: [], total: 0 })),
  );
  /** Where reminders go: enabled channels, reminder routing and quiet hours. */
  const routing = useQuery(
    () => ({ tick: tick() }),
    async () => {
      const [settings, channels] = await Promise.all([api.settings(), api.channels()]);
      return { prefs: settings.notifications, channels: channels.channels.filter((c) => c.enabled) };
    },
  );

  const nextFires = useQuery(
    () => {
      const list = reminders.data()?.reminders;
      const m = market();
      if (!list || !m) return null;
      return { list, m, plans: pending.data()?.plans ?? [], runs: runs.data(), minute: minute() };
    },
    async (k) => {
      const now = serverNow();
      const today = marketDate();
      const nudged = new Set((k.runs?.plan_review ?? []).map((run) => run.run_key));
      const out: Record<number, NextFire> = {};
      for (const r of k.list) {
        if (r.kind === "custom") continue;
        out[r.id] = await builtinNextFire(r.schedule, now, k.m, today, k.plans, nudged).catch((): NextFire => ({ kind: "unknown" }));
      }
      return out;
    },
  );

  onMount(() => {
    const bump = debounce(() => setTick((v) => v + 1), 800);
    const off = onServerEvent(["plan", "notification", "settings"], (event) => {
      if (event.type === "notification" && event.category !== "reminder") return;
      bump();
    });
    const timer = setInterval(() => setMinute((v) => v + 1), 60_000);
    onCleanup(() => {
      off();
      bump.cancel();
      clearInterval(timer);
    });
  });

  const builtins = createMemo(() => (reminders.data()?.reminders ?? []).filter((r) => r.kind !== "custom"));
  const customs = createMemo(() => (reminders.data()?.reminders ?? []).filter((r) => r.kind === "custom"));

  const replace = (updated: Reminder) =>
    reminders.mutate((value) =>
      value ? { reminders: value.reminders.some((r) => r.id === updated.id) ? value.reminders.map((r) => (r.id === updated.id ? updated : r)) : [...value.reminders, updated] } : value,
    );

  const toggle = async (r: Reminder, enabled: boolean) => {
    setBusy(r.id);
    try {
      const res = await api.updateReminder(r.id, { title: r.title, note: r.note, schedule: r.schedule, enabled });
      replace(res.reminder);
      toast(tpl(enabled ? n().reminders.toggled_on : n().reminders.toggled_off, { name: reminderName(r) }), undefined, "success", 2400);
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };

  const remove = async (r: Reminder) => {
    const ok = await confirmAction({
      title: n().reminders.confirm_delete_title,
      body: tpl(n().reminders.confirm_delete_body, { name: reminderName(r) }),
      confirmLabel: common().actions.delete,
      danger: true,
    });
    if (ok === null) return;
    setBusy(r.id);
    try {
      await api.deleteReminder(r.id);
      reminders.mutate((value) => (value ? { reminders: value.reminders.filter((x) => x.id !== r.id) } : value));
      toast(tpl(n().reminders.deleted, { name: reminderName(r) }), undefined, "success", 2400);
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };

  const Row = (p: { r: Reminder }) => (
    <ReminderRow
      reminder={p.r}
      next={nextFires.data()?.[p.r.id]}
      computing={!nextFires.data()}
      runs={runs.data()?.[p.r.kind] ?? []}
      busy={busy() === p.r.id}
      onToggle={(value) => toggle(p.r, value)}
      onEdit={() => setEditing(p.r)}
      onDelete={() => remove(p.r)}
    />
  );

  return (
    <div class="rem-wrap">
      <div class="callout info rem-delivery">
        <Icon name="send" size={17} />
        <div class="rem-delivery-text">
          <strong>{n().reminders.delivery_title}</strong>
          <p>{n().reminders.delivery_body}</p>
          <Show when={routing.data()}>
            {(info) => (
              <div class="rem-routing">
                <span class="rem-route">
                  <Show when={info().channels.length} fallback={<>{n().reminders.routing.no_channels}</>}>
                    {tpl(n().reminders.routing.channels, { list: info().channels.map((c) => c.label || c.name).join(" · ") })}
                  </Show>
                </span>
                <span class="rem-route">
                  <Switch fallback={<>{n().reminders.routing.category_on}</>}>
                    <Match when={info().prefs.categories?.reminder === false}>{n().reminders.routing.category_off}</Match>
                    {/* Reminders are info-level; the server only pushes events at or above min_severity. */}
                    <Match when={info().prefs.min_severity === "warning" || info().prefs.min_severity === "critical"}>
                      {tpl(n().reminders.routing.severity_blocked, { level: common().severity[info().prefs.min_severity] })}
                    </Match>
                  </Switch>
                </span>
                <span class="rem-route">
                  <Show when={info().prefs.quiet_hours?.enabled} fallback={<>{n().reminders.routing.quiet_off}</>}>
                    {tpl(n().reminders.routing.quiet, {
                      start: info().prefs.quiet_hours.start,
                      end: info().prefs.quiet_hours.end,
                      zone: zoneLabel(info().prefs.quiet_hours.timezone),
                    })}
                  </Show>
                </span>
              </div>
            )}
          </Show>
          <A class="rem-settings-link" href="/settings/notifications">
            {n().reminders.settings_link}
            <XIcon name="arrow_right" size={13} />
          </A>
        </div>
      </div>

      <Switch>
        <Match when={reminders.error() && !reminders.data()}>
          <section class="card">
            <ErrorState error={reminders.error()} onRetry={reminders.refetch} />
          </section>
        </Match>
        <Match when={!reminders.data()}>
          <section class="card">
            <Loading />
          </section>
        </Match>
        <Match when={true}>
          <section class="card">
            <div class="card-head">
              <div>
                <h2>{n().reminders.builtin}</h2>
                <div class="sub">{n().reminders.builtin_sub}</div>
              </div>
            </div>
            <div class="rem-list">
              <For each={builtins()}>{(r) => <Row r={r} />}</For>
            </div>
          </section>

          <section class="card">
            <div class="card-head">
              <div>
                <h2>{n().reminders.custom}</h2>
                <div class="sub">{n().reminders.custom_sub}</div>
              </div>
              <span class="spacer" />
              <Show when={isAdmin()} fallback={<span class="muted xs">{n().reminders.admin_only}</span>}>
                <button type="button" class="btn sm primary" onClick={() => setEditing("new")}>
                  <Icon name="plus" size={14} />
                  {n().reminders.new}
                </button>
              </Show>
            </div>
            <Show
              when={customs().length}
              fallback={
                <Empty title={n().reminders.empty} icon="clock">
                  <span>{n().reminders.empty_hint}</span>
                  <Show when={isAdmin()}>
                    <button type="button" class="btn sm" onClick={() => setEditing("new")}>
                      <Icon name="plus" size={14} />
                      {n().reminders.new}
                    </button>
                  </Show>
                </Empty>
              }
            >
              <div class="rem-list">
                <For each={customs()}>{(r) => <Row r={r} />}</For>
              </div>
            </Show>
          </section>
        </Match>
      </Switch>

      <Show when={editing()}>
        {(target) => (
          <ReminderDialog
            reminder={target() === "new" ? null : (target() as Reminder)}
            onClose={() => setEditing(null)}
            onSaved={(saved) => {
              replace(saved);
              setEditing(null);
              setTick((v) => v + 1);
            }}
          />
        )}
      </Show>
    </div>
  );
}

function ReminderRow(props: {
  reminder: Reminder;
  next: NextFire | undefined;
  computing: boolean;
  runs: JobRun[];
  busy: boolean;
  onToggle: (value: boolean) => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const n = notificationsText;
  const r = () => props.reminder;
  const custom = () => r().kind === "custom";
  const desc = () => (custom() ? "" : (n().reminders.descs as Record<string, string>)[r().kind] ?? "");

  const next = (): { at: string | null; text: string } => {
    if (!r().enabled) return { at: null, text: n().reminders.disabled };
    if (custom()) {
      if (r().next_fire_at) return { at: r().next_fire_at, text: "" };
      return { at: null, text: r().schedule.type === "once" ? n().reminders.expired : DASH };
    }
    const fire = props.next;
    if (!fire) return { at: null, text: props.computing ? n().reminders.computing : DASH };
    if (fire.kind === "at") return { at: new Date(fire.at).toISOString(), text: "" };
    if (fire.kind === "on_demand") return { at: null, text: n().reminders.on_demand };
    return { at: null, text: DASH };
  };

  const last = (): { at: string | null; failed: JobRun | null; running: boolean } => {
    if (custom()) return { at: r().last_fired_at, failed: null, running: false };
    const run = props.runs[0];
    if (!run) return { at: null, failed: null, running: false };
    return { at: run.finished_at ?? run.started_at, failed: run.status === "failed" ? run : null, running: run.status === "running" };
  };

  return (
    <div class="rem-row" classList={{ off: !r().enabled }}>
      <div class="rem-info">
        <div class="rem-title">
          <span class="rem-name">{reminderName(r())}</span>
          <Show when={custom()}>
            <span class="rem-by">{tpl(n().reminders.created_by, { user: actorName(r().created_by) })}</span>
          </Show>
        </div>
        <div class="rem-sched">
          <Icon name={custom() && r().schedule.type === "once" ? "calendar" : "clock"} size={13} />
          <span>{describeSchedule(r().schedule)}</span>
        </div>
        <Show when={desc()}>
          <p class="rem-desc">{desc()}</p>
        </Show>
        <Show when={r().note}>
          <p class="rem-note">{r().note}</p>
        </Show>
      </div>

      <div class="rem-when">
        <span class="rem-k">{n().reminders.next}</span>
        <Show when={next().at} fallback={<span class="rem-v muted">{next().text}</span>}>
          <span class="rem-v num" title={fmtDual(next().at, true)}>
            {fmtDual(next().at, true).split(" · ")[0]}
          </span>
          <span class="rem-sub num">
            {fmtDual(next().at, true).split(" · ")[1]} · {fmtRelative(next().at, serverNow())}
          </span>
        </Show>
      </div>

      <div class="rem-when">
        <span class="rem-k">{n().reminders.last}</span>
        <Show when={last().at} fallback={<span class="rem-v muted">{n().reminders.never}</span>}>
          <span class="rem-v num" title={fmtDual(last().at, true)}>
            {fmtDual(last().at, true).split(" · ")[0]}
          </span>
          <span class="rem-sub">
            <Show when={last().failed} fallback={fmtRelative(last().at, serverNow())}>
              <span class="chip red" title={last().failed?.error ?? undefined}>
                {fmtRelative(last().at, serverNow())} · ✕
              </span>
            </Show>
          </span>
        </Show>
      </div>

      <div class="rem-ctl">
        <Show
          when={isAdmin()}
          fallback={<span class={`chip ${r().enabled ? "green" : ""}`}>{r().enabled ? n().reminders.enabled : n().reminders.disabled}</span>}
        >
          <Toggle
            checked={r().enabled}
            disabled={props.busy}
            onChange={props.onToggle}
            label={<span class="rem-switch-label">{r().enabled ? n().reminders.enabled : n().reminders.disabled}</span>}
          />
          <div class="rem-actions">
            <button type="button" class="btn ghost sm" onClick={props.onEdit} disabled={props.busy}>
              <XIcon name="pencil" size={13} />
              {n().reminders.edit}
            </button>
            <Show when={custom()}>
              <button type="button" class="btn ghost sm rem-delete" onClick={props.onDelete} disabled={props.busy}>
                <Icon name="trash" size={13} />
                {n().reminders.delete}
              </button>
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
}
