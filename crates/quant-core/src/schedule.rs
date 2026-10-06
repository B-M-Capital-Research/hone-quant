//! Daily plan slots.
//!
//! The operating rule is: one trading plan inside the first three hours after the open and one
//! three hours before the close. Both are expressed as offsets from the session boundaries so
//! that DST changes and 13:00 early closes are handled by the calendar, not by wall-clock math.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::calendar::Session;

/// Width of each plan window: the first / last three hours of the regular session.
pub const SLOT_WINDOW_MINUTES: u32 = 180;
/// Orders are never executed in the last few minutes before the close.
pub const EXECUTION_CUTOFF_MINUTES: i64 = 5;
/// A slot scheduled at the very end of its window (offset = the full window) may still be
/// generated this long after its time, so the scheduler's tick cannot make it impossible.
pub const GENERATION_GRACE_MINUTES: i64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanSlot {
    /// Generated inside the first three hours after the open.
    Open,
    /// Generated three hours before the close.
    Close,
}

impl PlanSlot {
    pub fn as_str(self) -> &'static str {
        match self {
            PlanSlot::Open => "open",
            PlanSlot::Close => "close",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "open" => Some(PlanSlot::Open),
            "close" => Some(PlanSlot::Close),
            _ => None,
        }
    }

    pub const ALL: [PlanSlot; 2] = [PlanSlot::Open, PlanSlot::Close];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScheduleSettings {
    /// Minutes after the open at which the opening plan is generated (0..=180).
    pub open_offset_minutes: u32,
    /// Minutes before the close at which the pre-close plan is generated (30..=180).
    pub close_offset_minutes: u32,
    /// Review window between generation and automatic execution (0..=60).
    pub review_minutes: u32,
    /// On short sessions the pre-close plan is skipped when it would start less than this many
    /// minutes after the opening plan.
    pub min_gap_minutes: u32,
}

impl Default for ScheduleSettings {
    fn default() -> Self {
        Self {
            open_offset_minutes: 30,
            close_offset_minutes: 180,
            review_minutes: 10,
            min_gap_minutes: 60,
        }
    }
}

impl ScheduleSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.open_offset_minutes > SLOT_WINDOW_MINUTES {
            return Err("open_offset_minutes must be within the first 180 minutes".into());
        }
        if !(30..=SLOT_WINDOW_MINUTES).contains(&self.close_offset_minutes) {
            return Err("close_offset_minutes must be between 30 and 180".into());
        }
        if self.review_minutes > 60 {
            return Err("review_minutes must be at most 60".into());
        }
        if self.min_gap_minutes > 240 {
            return Err("min_gap_minutes must be at most 240".into());
        }
        Ok(())
    }
}

/// When one slot generates and (if automation allows) executes on a given session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SlotTime {
    pub slot: PlanSlot,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub generate_at: DateTime<Utc>,
    pub execute_at: DateTime<Utc>,
    /// Plans not executed by this instant expire.
    pub execute_deadline: DateTime<Utc>,
}

impl SlotTime {
    /// End (exclusive) of the period in which this slot's plan may still be generated: the end of
    /// the slot's window (at least a short grace after `generate_at`), never past the execution
    /// deadline. A service that was down through the opening window therefore records the opening
    /// slot as missed instead of trading it late in the day.
    pub fn generation_closes(&self) -> DateTime<Utc> {
        self.window_end
            .max(self.generate_at + Duration::minutes(GENERATION_GRACE_MINUTES))
            .min(self.execute_deadline)
    }
}

/// The plan slots of one session. Short sessions (13:00 closes) normally get only the opening
/// plan because "three hours before the close" coincides with the opening window.
pub fn day_schedule(session: &Session, settings: &ScheduleSettings) -> Vec<SlotTime> {
    let window = Duration::minutes(SLOT_WINDOW_MINUTES as i64);
    let review = Duration::minutes(settings.review_minutes as i64);
    let deadline = session.close - Duration::minutes(EXECUTION_CUTOFF_MINUTES);

    let open_generate = session.open + Duration::minutes(settings.open_offset_minutes as i64);
    let close_generate = session.close - Duration::minutes(settings.close_offset_minutes as i64);

    let mut slots = Vec::with_capacity(2);
    if open_generate < deadline {
        slots.push(SlotTime {
            slot: PlanSlot::Open,
            window_start: session.open,
            window_end: (session.open + window).min(session.close),
            generate_at: open_generate,
            execute_at: (open_generate + review).min(deadline),
            execute_deadline: deadline,
        });
    }
    let gap_ok =
        close_generate >= open_generate + Duration::minutes(settings.min_gap_minutes as i64);
    if gap_ok && close_generate < deadline {
        slots.push(SlotTime {
            slot: PlanSlot::Close,
            window_start: (session.close - window).max(session.open),
            window_end: session.close,
            generate_at: close_generate,
            execute_at: (close_generate + review).min(deadline),
            execute_deadline: deadline,
        });
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::MarketCalendar;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn regular_day_has_two_slots_at_10_and_13_et() {
        let cal = MarketCalendar::nyse();
        let session = cal.session(d(2026, 10, 5)).unwrap();
        let slots = day_schedule(&session, &ScheduleSettings::default());
        assert_eq!(slots.len(), 2);
        // EDT: 10:00 ET = 14:00 UTC (22:00 SGT); 13:00 ET = 17:00 UTC (01:00 SGT).
        assert_eq!(slots[0].slot, PlanSlot::Open);
        assert_eq!(
            slots[0].generate_at.to_rfc3339(),
            "2026-10-05T14:00:00+00:00"
        );
        assert_eq!(
            slots[0].execute_at.to_rfc3339(),
            "2026-10-05T14:10:00+00:00"
        );
        assert_eq!(
            slots[0].window_end.to_rfc3339(),
            "2026-10-05T16:30:00+00:00"
        );
        assert_eq!(slots[1].slot, PlanSlot::Close);
        assert_eq!(
            slots[1].generate_at.to_rfc3339(),
            "2026-10-05T17:00:00+00:00"
        );
        assert_eq!(
            slots[1].window_start.to_rfc3339(),
            "2026-10-05T17:00:00+00:00"
        );
        assert_eq!(
            slots[1].execute_deadline.to_rfc3339(),
            "2026-10-05T19:55:00+00:00"
        );
    }

    #[test]
    fn winter_schedule_shifts_with_dst() {
        let cal = MarketCalendar::nyse();
        let session = cal.session(d(2026, 12, 1)).unwrap();
        let slots = day_schedule(&session, &ScheduleSettings::default());
        assert_eq!(
            slots[0].generate_at.to_rfc3339(),
            "2026-12-01T15:00:00+00:00"
        );
        assert_eq!(
            slots[1].generate_at.to_rfc3339(),
            "2026-12-01T18:00:00+00:00"
        );
    }

    #[test]
    fn early_close_day_keeps_only_the_opening_plan() {
        let cal = MarketCalendar::nyse();
        let session = cal.session(d(2026, 11, 27)).unwrap();
        let slots = day_schedule(&session, &ScheduleSettings::default());
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].slot, PlanSlot::Open);
    }

    #[test]
    fn early_close_day_can_keep_both_plans_with_a_shorter_offset() {
        let cal = MarketCalendar::nyse();
        let session = cal.session(d(2026, 11, 27)).unwrap();
        let settings = ScheduleSettings {
            open_offset_minutes: 15,
            close_offset_minutes: 90,
            ..ScheduleSettings::default()
        };
        let slots = day_schedule(&session, &settings);
        assert_eq!(slots.len(), 2);
        assert!(slots[1].generate_at > slots[0].generate_at);
    }

    #[test]
    fn opening_plan_is_not_generated_after_its_window() {
        let cal = MarketCalendar::nyse();
        let session = cal.session(d(2026, 10, 5)).unwrap();
        let slots = day_schedule(&session, &ScheduleSettings::default());
        let at = |s: &str| s.parse::<DateTime<Utc>>().unwrap();
        // The opening window ends at 12:30 ET (16:30 UTC): a service started at 12:47 ET must
        // record the slot as missed rather than trade it shortly before the pre-close plan.
        assert_eq!(slots[0].generation_closes(), at("2026-10-05T16:30:00Z"));
        assert!(at("2026-10-05T16:29:59Z") < slots[0].generation_closes());
        assert!(at("2026-10-05T16:47:00Z") >= slots[0].generation_closes());
        // The pre-close plan may be generated until the execution deadline.
        assert_eq!(slots[1].generation_closes(), slots[1].execute_deadline);
        assert_eq!(slots[1].generation_closes(), at("2026-10-05T19:55:00Z"));
    }

    #[test]
    fn a_full_window_offset_keeps_a_short_grace() {
        let cal = MarketCalendar::nyse();
        let session = cal.session(d(2026, 10, 5)).unwrap();
        let settings = ScheduleSettings {
            open_offset_minutes: SLOT_WINDOW_MINUTES,
            ..ScheduleSettings::default()
        };
        let open = day_schedule(&session, &settings)[0];
        assert_eq!(open.generate_at, open.window_end);
        assert_eq!(
            open.generation_closes(),
            open.generate_at + Duration::minutes(GENERATION_GRACE_MINUTES)
        );
    }

    #[test]
    fn execution_is_clamped_before_the_cutoff() {
        let cal = MarketCalendar::nyse();
        let session = cal.session(d(2026, 10, 5)).unwrap();
        let settings = ScheduleSettings {
            close_offset_minutes: 30,
            review_minutes: 60,
            ..ScheduleSettings::default()
        };
        let slots = day_schedule(&session, &settings);
        let close = slots.iter().find(|s| s.slot == PlanSlot::Close).unwrap();
        assert_eq!(close.execute_at, close.execute_deadline);
    }

    #[test]
    fn validation_rejects_out_of_window_offsets() {
        let bad = ScheduleSettings {
            open_offset_minutes: 200,
            ..ScheduleSettings::default()
        };
        assert!(bad.validate().is_err());
        let bad = ScheduleSettings {
            close_offset_minutes: 10,
            ..ScheduleSettings::default()
        };
        assert!(bad.validate().is_err());
        assert!(ScheduleSettings::default().validate().is_ok());
    }
}
