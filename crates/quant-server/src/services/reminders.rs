//! Reminders.
//!
//! Built-in reminders are tied to the market calendar (pre-open briefing, post-close daily
//! summary, weekly report, review nudge for plans awaiting approval) and are fired by the
//! scheduler; operators can switch them off or retime them. Custom reminders are free-form
//! notes with a one-off or recurring schedule in any time zone.

use anyhow::{Result, bail};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use quant_core::calendar::MarketCalendar;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::store::system;

pub const PRE_OPEN: &str = "pre_open";
pub const DAILY_SUMMARY: &str = "daily_summary";
pub const WEEKLY_REPORT: &str = "weekly_report";
pub const PLAN_REVIEW: &str = "plan_review";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Schedule {
    /// Minutes before the regular-session open on trading days.
    PreOpen {
        minutes_before: i64,
    },
    /// Minutes after the close on trading days.
    PostClose {
        minutes_after: i64,
    },
    /// Minutes after the close on the last trading day of each week.
    WeekClose {
        minutes_after: i64,
    },
    /// Minutes before a pending plan's automatic execution or approval deadline.
    BeforeDeadline {
        minutes_before: i64,
    },
    Once {
        at: DateTime<Utc>,
    },
    Daily {
        time: String,
        tz: String,
    },
    TradingDays {
        time: String,
        tz: String,
    },
    Weekly {
        time: String,
        tz: String,
        weekdays: Vec<u32>,
    },
}

impl Schedule {
    pub fn validate(&self) -> Result<()> {
        let check_time = |time: &str, tz: &str| -> Result<()> {
            NaiveTime::parse_from_str(time, "%H:%M")?;
            tz.parse::<Tz>()
                .map_err(|_| anyhow::anyhow!("unknown time zone {tz}"))?;
            Ok(())
        };
        match self {
            Schedule::PreOpen { minutes_before } | Schedule::BeforeDeadline { minutes_before } => {
                if !(1..=600).contains(minutes_before) {
                    bail!("minutes must be between 1 and 600");
                }
            }
            Schedule::PostClose { minutes_after } | Schedule::WeekClose { minutes_after } => {
                if !(1..=600).contains(minutes_after) {
                    bail!("minutes must be between 1 and 600");
                }
            }
            Schedule::Once { .. } => {}
            Schedule::Daily { time, tz } | Schedule::TradingDays { time, tz } => {
                check_time(time, tz)?
            }
            Schedule::Weekly { time, tz, weekdays } => {
                check_time(time, tz)?;
                if weekdays.is_empty() || weekdays.iter().any(|d| !(1..=7).contains(d)) {
                    bail!("weekdays must be 1 (Mon) to 7 (Sun)");
                }
            }
        }
        Ok(())
    }

    pub fn is_custom(&self) -> bool {
        matches!(
            self,
            Schedule::Once { .. }
                | Schedule::Daily { .. }
                | Schedule::TradingDays { .. }
                | Schedule::Weekly { .. }
        )
    }
}

fn local_instant(date: NaiveDate, time: &str, tz: &str) -> Option<DateTime<Utc>> {
    let tz: Tz = tz.parse().ok()?;
    let time = NaiveTime::parse_from_str(time, "%H:%M").ok()?;
    tz.from_local_datetime(&date.and_time(time))
        .earliest()
        .map(|d| d.with_timezone(&Utc))
}

/// Next firing instant strictly after `after` for custom schedules.
pub fn next_fire(
    schedule: &Schedule,
    after: DateTime<Utc>,
    calendar: &MarketCalendar,
) -> Option<DateTime<Utc>> {
    match schedule {
        Schedule::Once { at } => (*at > after).then_some(*at),
        Schedule::Daily { time, tz }
        | Schedule::TradingDays { time, tz }
        | Schedule::Weekly { time, tz, .. } => {
            let zone: Tz = tz.parse().ok()?;
            let start = after.with_timezone(&zone).date_naive();
            for offset in 0..15 {
                let date = start + Duration::days(offset);
                let allowed = match schedule {
                    Schedule::Daily { .. } => true,
                    // Trading days are New York sessions; the reminder fires at the local time
                    // on the same calendar date.
                    Schedule::TradingDays { .. } => calendar.is_trading_day(date),
                    Schedule::Weekly { weekdays, .. } => {
                        weekdays.contains(&date.weekday().number_from_monday())
                    }
                    _ => false,
                };
                if !allowed {
                    continue;
                }
                if let Some(at) = local_instant(date, time, tz)
                    && at > after
                {
                    return Some(at);
                }
            }
            None
        }
        _ => None,
    }
}

/// Inserts the built-in reminders (enabled) unless they already exist.
pub async fn seed_builtins(client: &impl deadpool_postgres::GenericClient) -> Result<()> {
    let builtins = [
        (PRE_OPEN, Schedule::PreOpen { minutes_before: 30 }),
        (DAILY_SUMMARY, Schedule::PostClose { minutes_after: 20 }),
        (WEEKLY_REPORT, Schedule::WeekClose { minutes_after: 30 }),
        (PLAN_REVIEW, Schedule::BeforeDeadline { minutes_before: 30 }),
    ];
    for (kind, schedule) in builtins {
        system::upsert_builtin_reminder(
            client,
            kind,
            &serde_json::to_value(&schedule)?,
            true,
            None,
        )
        .await?;
    }
    Ok(())
}

/// The schedule of a built-in reminder if it is enabled.
pub async fn builtin(
    client: &impl deadpool_postgres::GenericClient,
    kind: &str,
) -> Result<Option<Schedule>> {
    let reminders = system::reminders(client).await?;
    Ok(reminders
        .into_iter()
        .find(|r| r.kind == kind && r.enabled)
        .and_then(|r| serde_json::from_value(r.schedule).ok()))
}

pub fn describe(schedule: &Schedule) -> Value {
    json!(schedule)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn daily_in_singapore() {
        let cal = MarketCalendar::nyse();
        let s = Schedule::Daily {
            time: "08:00".into(),
            tz: "Asia/Singapore".into(),
        };
        // 2026-10-05 03:00 UTC = 11:00 SGT → next is 2026-10-06 08:00 SGT = 00:00 UTC.
        assert_eq!(
            next_fire(&s, at("2026-10-05T03:00:00Z"), &cal),
            Some(at("2026-10-06T00:00:00Z"))
        );
        // Just before today's 08:00 SGT.
        assert_eq!(
            next_fire(&s, at("2026-10-04T23:59:00Z"), &cal),
            Some(at("2026-10-05T00:00:00Z"))
        );
    }

    #[test]
    fn trading_days_skip_weekends_and_holidays() {
        let cal = MarketCalendar::nyse();
        let s = Schedule::TradingDays {
            time: "21:00".into(),
            tz: "Asia/Singapore".into(),
        };
        // Friday 2026-11-27 after 21:00 SGT → Monday 2026-11-30.
        let next = next_fire(&s, at("2026-11-27T14:00:00Z"), &cal).unwrap();
        assert_eq!(next, at("2026-11-30T13:00:00Z"));
        // Thanksgiving 2026-11-26 is skipped.
        let next = next_fire(&s, at("2026-11-25T14:00:00Z"), &cal).unwrap();
        assert_eq!(next, at("2026-11-27T13:00:00Z"));
    }

    #[test]
    fn weekly_and_once() {
        let cal = MarketCalendar::nyse();
        let s = Schedule::Weekly {
            time: "09:00".into(),
            tz: "Asia/Singapore".into(),
            weekdays: vec![6],
        };
        // Saturday 2026-10-10 09:00 SGT = 01:00 UTC.
        assert_eq!(
            next_fire(&s, at("2026-10-05T00:00:00Z"), &cal),
            Some(at("2026-10-10T01:00:00Z"))
        );
        let once = Schedule::Once {
            at: at("2026-10-07T10:00:00Z"),
        };
        assert_eq!(
            next_fire(&once, at("2026-10-05T00:00:00Z"), &cal),
            Some(at("2026-10-07T10:00:00Z"))
        );
        assert_eq!(next_fire(&once, at("2026-10-08T00:00:00Z"), &cal), None);
    }

    #[test]
    fn validation() {
        assert!(
            Schedule::Daily {
                time: "25:00".into(),
                tz: "Asia/Singapore".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Schedule::Daily {
                time: "08:00".into(),
                tz: "Mars/Base".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Schedule::Weekly {
                time: "08:00".into(),
                tz: "UTC".into(),
                weekdays: vec![8]
            }
            .validate()
            .is_err()
        );
        assert!(Schedule::PreOpen { minutes_before: 30 }.validate().is_ok());
        let json = serde_json::to_value(Schedule::PreOpen { minutes_before: 30 }).unwrap();
        assert_eq!(json["type"], "pre_open");
    }
}
