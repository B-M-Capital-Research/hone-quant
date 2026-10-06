/**
 * Reminder schedules: human-readable descriptions and the next firing time.
 *
 * Custom reminders carry `next_fire_at`/`last_fired_at` from the server. Built-in reminders do
 * not (the scheduler fires them from the market calendar and records job runs instead), so
 * their next firing is derived here from the same rules as `services/scheduler.rs`:
 * - pre-open: inside [open − m, open) of each session;
 * - daily summary: close + max(m, 16) min of each session;
 * - weekly report: close + max(m, 16) min of the week's last session;
 * - plan review: due − m for each pending plan whose review window exceeds 5 minutes.
 */
import { tpl } from "@/i18n";
import { notificationsText } from "@/i18n/notifications";
import { fmtDual, zoneLabel } from "@/lib/format";
import { api } from "@/lib/api";
import type { MarketView, Plan, ReminderSchedule, Session } from "@/lib/types";
import { addDays, isoWeekday } from "./util";

/** Job names under which the scheduler records built-in reminder runs. */
export const BUILTIN_JOB: Record<string, string> = {
  pre_open: "reminder_pre_open",
  daily_summary: "reminder_daily_summary",
  weekly_report: "reminder_weekly_report",
  plan_review: "reminder_plan_review",
};

export const BUILTIN_KINDS = ["pre_open", "daily_summary", "weekly_report", "plan_review"] as const;

/** Post-close jobs wait for the closing data (scheduler: `minutes_after.max(16)`). */
const POST_CLOSE_FLOOR = 16;

export function clock(time: string, tz: string): string {
  return `${time} ${zoneLabel(tz)}`;
}

export function describeSchedule(schedule: ReminderSchedule): string {
  const d = notificationsText().reminders;
  switch (schedule.type) {
    case "pre_open":
      return tpl(d.sched.pre_open, { m: schedule.minutes_before });
    case "post_close":
      return tpl(d.sched.post_close, { m: schedule.minutes_after });
    case "week_close":
      return tpl(d.sched.week_close, { m: schedule.minutes_after });
    case "before_deadline":
      return tpl(d.sched.before_deadline, { m: schedule.minutes_before });
    case "once":
      return tpl(d.sched.once, { time: fmtDual(schedule.at, true) });
    case "daily":
      return tpl(d.sched.daily, { time: clock(schedule.time, schedule.tz) });
    case "trading_days":
      return tpl(d.sched.trading_days, { time: clock(schedule.time, schedule.tz) });
    case "weekly": {
      const days = [...new Set(schedule.weekdays)].filter((n) => n >= 1 && n <= 7).sort((a, b) => a - b);
      const time = clock(schedule.time, schedule.tz);
      if (days.length === 7) return tpl(d.sched.every_day, { time });
      if (days.join(",") === "1,2,3,4,5") return tpl(d.sched.workdays, { time });
      const names = days.length === 1 ? d.weekdays_one : d.weekdays_many;
      return tpl(d.sched.weekly, { days: days.map((n) => names[n - 1]).join(d.sched.day_sep), time });
    }
    default:
      return JSON.stringify(schedule);
  }
}

export type NextFire = { kind: "at"; at: number } | { kind: "on_demand" } | { kind: "unknown" };

/** Sessions by date, cached for the page's lifetime (GET /api/trading-days/{date}). */
const sessions = new Map<string, Promise<Session | null>>();

export function sessionOn(date: string): Promise<Session | null> {
  let hit = sessions.get(date);
  if (!hit) {
    hit = api
      .tradingDay(date)
      .then((day) => (day?.session as Session | null) ?? null)
      .catch((error) => {
        sessions.delete(date);
        throw error;
      });
    sessions.set(date, hit);
  }
  return hit;
}

async function sessionAfter(date: string): Promise<Session | null> {
  for (let i = 1; i <= 10; i++) {
    const session = await sessionOn(addDays(date, i));
    if (session) return session;
  }
  return null;
}

/** Known upcoming sessions from the market view: today's (if any) and the next one. */
function knownSessions(market: MarketView): Session[] {
  const out: Session[] = [];
  if (market.today) out.push(market.today);
  if (market.next_session && market.next_session.date !== market.today?.date) out.push(market.next_session);
  return out;
}

async function firstAfter(market: MarketView, now: number, instant: (s: Session) => number): Promise<NextFire> {
  const known = knownSessions(market);
  for (const session of known) {
    const at = instant(session);
    if (at > now) return { kind: "at", at };
  }
  let date = known.length ? known[known.length - 1].date : market.schedule_date;
  for (let i = 0; i < 3; i++) {
    const session = await sessionAfter(date);
    if (!session) break;
    const at = instant(session);
    if (at > now) return { kind: "at", at };
    date = session.date;
  }
  return { kind: "unknown" };
}

export async function builtinNextFire(
  schedule: ReminderSchedule,
  now: number,
  market: MarketView,
  today: string,
  pending: Plan[],
  /** Plan ids that already received their review nudge (job run keys). */
  nudged: Set<string> = new Set(),
): Promise<NextFire> {
  switch (schedule.type) {
    case "pre_open": {
      const lead = schedule.minutes_before * 60_000;
      return firstAfter(market, now, (s) => Date.parse(s.open) - lead);
    }
    case "post_close": {
      const lag = Math.max(schedule.minutes_after, POST_CLOSE_FLOOR) * 60_000;
      return firstAfter(market, now, (s) => Date.parse(s.close) + lag);
    }
    case "week_close": {
      const lag = Math.max(schedule.minutes_after, POST_CLOSE_FLOOR) * 60_000;
      let monday = addDays(today, 1 - isoWeekday(today));
      for (let week = 0; week < 3; week++) {
        for (let day = 4; day >= 0; day--) {
          const session = await sessionOn(addDays(monday, day));
          if (!session) continue;
          const at = Date.parse(session.close) + lag;
          if (at > now) return { kind: "at", at };
          break;
        }
        monday = addDays(monday, 7);
      }
      return { kind: "unknown" };
    }
    case "before_deadline": {
      const window = schedule.minutes_before * 60_000;
      let best: number | null = null;
      for (const plan of pending) {
        const due = Date.parse(plan.execute_after ?? plan.deadline);
        if (!(due > now) || due - Date.parse(plan.generated_at) <= 5 * 60_000 || nudged.has(String(plan.id))) continue;
        const at = Math.max(due - window, now);
        if (best === null || at < best) best = at;
      }
      return best === null ? { kind: "on_demand" } : { kind: "at", at: best };
    }
    default:
      return { kind: "unknown" };
  }
}
