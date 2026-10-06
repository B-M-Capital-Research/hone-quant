//! NYSE / Nasdaq regular-session calendar.
//!
//! Holidays and 13:00 early closes are derived from the exchange's published rules rather than
//! a downloaded table, so the scheduler keeps working for any year without a data dependency.
//! One-off closures (national days of mourning, weather) cannot be derived; they live in
//! [`MarketCalendar::extra_closures`] and can be extended from configuration.
//!
//! All session times are expressed in `America/New_York`; DST is handled by `chrono-tz`.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::America::New_York;
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// The exchange time zone used for every session boundary.
pub const MARKET_TZ: Tz = New_York;

/// Regular session open, 09:30 ET.
pub fn regular_open() -> NaiveTime {
    NaiveTime::from_hms_opt(9, 30, 0).expect("valid time")
}

/// Regular session close, 16:00 ET.
pub fn regular_close() -> NaiveTime {
    NaiveTime::from_hms_opt(16, 0, 0).expect("valid time")
}

/// Early close, 13:00 ET (day before Independence Day, day after Thanksgiving, Christmas Eve).
pub fn early_close() -> NaiveTime {
    NaiveTime::from_hms_opt(13, 0, 0).expect("valid time")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Holiday {
    NewYearsDay,
    MartinLutherKingJrDay,
    WashingtonsBirthday,
    GoodFriday,
    MemorialDay,
    Juneteenth,
    IndependenceDay,
    LaborDay,
    ThanksgivingDay,
    ChristmasDay,
    /// A closure that cannot be derived from rules (e.g. a national day of mourning).
    SpecialClosure,
}

impl Holiday {
    pub fn name_en(self) -> &'static str {
        match self {
            Holiday::NewYearsDay => "New Year's Day",
            Holiday::MartinLutherKingJrDay => "Martin Luther King Jr. Day",
            Holiday::WashingtonsBirthday => "Washington's Birthday",
            Holiday::GoodFriday => "Good Friday",
            Holiday::MemorialDay => "Memorial Day",
            Holiday::Juneteenth => "Juneteenth",
            Holiday::IndependenceDay => "Independence Day",
            Holiday::LaborDay => "Labor Day",
            Holiday::ThanksgivingDay => "Thanksgiving Day",
            Holiday::ChristmasDay => "Christmas Day",
            Holiday::SpecialClosure => "Special closure",
        }
    }

    pub fn name_zh(self) -> &'static str {
        match self {
            Holiday::NewYearsDay => "元旦",
            Holiday::MartinLutherKingJrDay => "马丁·路德·金纪念日",
            Holiday::WashingtonsBirthday => "总统日",
            Holiday::GoodFriday => "耶稣受难日",
            Holiday::MemorialDay => "阵亡将士纪念日",
            Holiday::Juneteenth => "六月节",
            Holiday::IndependenceDay => "独立日",
            Holiday::LaborDay => "劳动节",
            Holiday::ThanksgivingDay => "感恩节",
            Holiday::ChristmasDay => "圣诞节",
            Holiday::SpecialClosure => "特别休市",
        }
    }
}

/// One regular trading session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Session {
    /// Exchange-local trade date.
    pub date: NaiveDate,
    pub open: DateTime<Utc>,
    pub close: DateTime<Utc>,
    pub early_close: bool,
}

impl Session {
    pub fn contains(&self, at: DateTime<Utc>) -> bool {
        at >= self.open && at < self.close
    }

    pub fn minutes(&self) -> i64 {
        (self.close - self.open).num_minutes()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketPhase {
    /// Trading day, before the 09:30 open.
    PreOpen,
    /// Regular session in progress.
    Open,
    /// Trading day, after the close.
    PostClose,
    /// Weekend or holiday.
    Closed,
}

/// What the market is doing at a given instant, plus the next boundaries.
#[derive(Debug, Clone, Serialize)]
pub struct MarketStatus {
    pub phase: MarketPhase,
    /// The session whose trade date matches the exchange-local date, if it is a trading day.
    pub today: Option<Session>,
    /// The next session that has not closed yet (today's if still open or pre-open).
    pub next_session: Session,
    pub holiday: Option<Holiday>,
}

/// Rule-based NYSE calendar with an override set for one-off closures.
#[derive(Debug, Clone, Default)]
pub struct MarketCalendar {
    extra_closures: BTreeSet<NaiveDate>,
    extra_early_closes: BTreeMap<NaiveDate, NaiveTime>,
}

impl MarketCalendar {
    /// The NYSE calendar with known non-derivable closures since 2012.
    pub fn nyse() -> Self {
        let mut calendar = Self::default();
        for (y, m, d) in [
            (2012, 10, 29), // Hurricane Sandy
            (2012, 10, 30), // Hurricane Sandy
            (2018, 12, 5),  // National Day of Mourning, President George H. W. Bush
            (2025, 1, 9),   // National Day of Mourning, President Jimmy Carter
        ] {
            calendar
                .extra_closures
                .insert(NaiveDate::from_ymd_opt(y, m, d).expect("valid date"));
        }
        calendar
    }

    /// Adds a one-off full-day closure (from configuration).
    pub fn add_closure(&mut self, date: NaiveDate) {
        self.extra_closures.insert(date);
    }

    /// Adds a one-off early close (from configuration).
    pub fn add_early_close(&mut self, date: NaiveDate, close: NaiveTime) {
        self.extra_early_closes.insert(date, close);
    }

    pub fn extra_closures(&self) -> impl Iterator<Item = &NaiveDate> {
        self.extra_closures.iter()
    }

    /// The holiday observed on `date`, if any (weekends are not holidays).
    pub fn holiday(&self, date: NaiveDate) -> Option<Holiday> {
        if is_weekend(date) {
            return None;
        }
        if self.extra_closures.contains(&date) {
            return Some(Holiday::SpecialClosure);
        }
        rule_holiday(date)
    }

    pub fn is_trading_day(&self, date: NaiveDate) -> bool {
        !is_weekend(date) && self.holiday(date).is_none()
    }

    /// Exchange-local close time on `date` if it is a trading day.
    pub fn close_time(&self, date: NaiveDate) -> Option<NaiveTime> {
        if !self.is_trading_day(date) {
            return None;
        }
        if let Some(close) = self.extra_early_closes.get(&date) {
            return Some(*close);
        }
        if rule_early_close(date) {
            Some(early_close())
        } else {
            Some(regular_close())
        }
    }

    pub fn session(&self, date: NaiveDate) -> Option<Session> {
        let close_time = self.close_time(date)?;
        let open = local_to_utc(date, regular_open());
        let close = local_to_utc(date, close_time);
        Some(Session {
            date,
            open,
            close,
            early_close: close_time < regular_close(),
        })
    }

    /// The first trading day strictly after `date`.
    pub fn next_trading_day(&self, date: NaiveDate) -> NaiveDate {
        let mut day = date + Duration::days(1);
        while !self.is_trading_day(day) {
            day += Duration::days(1);
        }
        day
    }

    /// The last trading day strictly before `date`.
    pub fn prev_trading_day(&self, date: NaiveDate) -> NaiveDate {
        let mut day = date - Duration::days(1);
        while !self.is_trading_day(day) {
            day -= Duration::days(1);
        }
        day
    }

    /// Trading days in the inclusive range.
    pub fn trading_days(&self, from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
        let mut out = Vec::new();
        let mut day = from;
        while day <= to {
            if self.is_trading_day(day) {
                out.push(day);
            }
            day += Duration::days(1);
        }
        out
    }

    /// Exchange-local trade date for an instant.
    pub fn local_date(at: DateTime<Utc>) -> NaiveDate {
        at.with_timezone(&MARKET_TZ).date_naive()
    }

    pub fn status_at(&self, at: DateTime<Utc>) -> MarketStatus {
        let date = Self::local_date(at);
        let today = self.session(date);
        let phase = match today {
            None => MarketPhase::Closed,
            Some(session) if at < session.open => MarketPhase::PreOpen,
            Some(session) if at < session.close => MarketPhase::Open,
            Some(_) => MarketPhase::PostClose,
        };
        let next_session = match today {
            Some(session) if at < session.close => session,
            _ => self
                .session(self.next_trading_day(date))
                .expect("next trading day has a session"),
        };
        MarketStatus {
            phase,
            today,
            next_session,
            holiday: self.holiday(date),
        }
    }
}

fn local_to_utc(date: NaiveDate, time: NaiveTime) -> DateTime<Utc> {
    // Session boundaries are never inside a DST gap (transitions happen at 02:00 local).
    MARKET_TZ
        .from_local_datetime(&date.and_time(time))
        .single()
        .expect("session boundary is unambiguous")
        .with_timezone(&Utc)
}

fn is_weekend(date: NaiveDate) -> bool {
    matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

/// The `n`-th (1-based) `weekday` of a month.
fn nth_weekday(year: i32, month: u32, weekday: Weekday, n: u32) -> NaiveDate {
    NaiveDate::from_weekday_of_month_opt(year, month, weekday, n as u8).expect("valid nth weekday")
}

fn last_weekday(year: i32, month: u32, weekday: Weekday) -> NaiveDate {
    let first_next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .expect("valid date");
    let mut day = first_next - Duration::days(1);
    while day.weekday() != weekday {
        day -= Duration::days(1);
    }
    day
}

/// Gregorian Easter Sunday (anonymous Gregorian algorithm / Meeus-Jones-Butcher).
pub fn easter_sunday(year: i32) -> NaiveDate {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = ((h + l - 7 * m + 114) % 31) + 1;
    NaiveDate::from_ymd_opt(year, month as u32, day as u32).expect("valid Easter date")
}

/// Weekday observance for a fixed-date holiday: Saturday → Friday, Sunday → Monday.
fn observed(date: NaiveDate) -> NaiveDate {
    match date.weekday() {
        Weekday::Sat => date - Duration::days(1),
        Weekday::Sun => date + Duration::days(1),
        _ => date,
    }
}

fn rule_holiday(date: NaiveDate) -> Option<Holiday> {
    let year = date.year();
    let ymd = |m, d| NaiveDate::from_ymd_opt(year, m, d).expect("valid date");

    // New Year's Day: Sunday → Monday. When Jan 1 is a Saturday the NYSE does NOT close the
    // preceding Friday (Rule 7.2 accounting-period exception), so only the Sunday shift applies.
    let new_year = ymd(1, 1);
    let new_year_observed = if new_year.weekday() == Weekday::Sun {
        Some(new_year + Duration::days(1))
    } else if new_year.weekday() == Weekday::Sat {
        None
    } else {
        Some(new_year)
    };
    if new_year_observed == Some(date) {
        return Some(Holiday::NewYearsDay);
    }
    if date == nth_weekday(year, 1, Weekday::Mon, 3) {
        return Some(Holiday::MartinLutherKingJrDay);
    }
    if date == nth_weekday(year, 2, Weekday::Mon, 3) {
        return Some(Holiday::WashingtonsBirthday);
    }
    if date == easter_sunday(year) - Duration::days(2) {
        return Some(Holiday::GoodFriday);
    }
    if date == last_weekday(year, 5, Weekday::Mon) {
        return Some(Holiday::MemorialDay);
    }
    if year >= 2022 && date == observed(ymd(6, 19)) {
        return Some(Holiday::Juneteenth);
    }
    if date == observed(ymd(7, 4)) {
        return Some(Holiday::IndependenceDay);
    }
    if date == nth_weekday(year, 9, Weekday::Mon, 1) {
        return Some(Holiday::LaborDay);
    }
    if date == nth_weekday(year, 11, Weekday::Thu, 4) {
        return Some(Holiday::ThanksgivingDay);
    }
    if date == observed(ymd(12, 25)) {
        return Some(Holiday::ChristmasDay);
    }
    None
}

/// 13:00 early closes: July 3 and December 24 when they fall Monday–Thursday (a Friday July 3 or
/// December 24 is itself the observed holiday), and the day after Thanksgiving.
fn rule_early_close(date: NaiveDate) -> bool {
    let year = date.year();
    let mon_to_thu = |d: NaiveDate| {
        matches!(
            d.weekday(),
            Weekday::Mon | Weekday::Tue | Weekday::Wed | Weekday::Thu
        )
    };
    let july_3 = NaiveDate::from_ymd_opt(year, 7, 3).expect("valid date");
    if date == july_3 && mon_to_thu(date) {
        return true;
    }
    let christmas_eve = NaiveDate::from_ymd_opt(year, 12, 24).expect("valid date");
    if date == christmas_eve && mon_to_thu(date) {
        return true;
    }
    date == nth_weekday(year, 11, Weekday::Thu, 4) + Duration::days(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn holidays(cal: &MarketCalendar, year: i32) -> Vec<NaiveDate> {
        let mut out = Vec::new();
        let mut day = d(year, 1, 1);
        while day.year() == year {
            if cal.holiday(day).is_some() {
                out.push(day);
            }
            day += Duration::days(1);
        }
        out
    }

    fn early_closes(cal: &MarketCalendar, year: i32) -> Vec<NaiveDate> {
        let mut out = Vec::new();
        let mut day = d(year, 1, 1);
        while day.year() == year {
            if cal.session(day).is_some_and(|s| s.early_close) {
                out.push(day);
            }
            day += Duration::days(1);
        }
        out
    }

    #[test]
    fn easter_dates_match_known_values() {
        assert_eq!(easter_sunday(2024), d(2024, 3, 31));
        assert_eq!(easter_sunday(2025), d(2025, 4, 20));
        assert_eq!(easter_sunday(2026), d(2026, 4, 5));
        assert_eq!(easter_sunday(2027), d(2027, 3, 28));
        assert_eq!(easter_sunday(2019), d(2019, 4, 21));
    }

    #[test]
    fn nyse_2024_holidays_and_early_closes() {
        let cal = MarketCalendar::nyse();
        assert_eq!(
            holidays(&cal, 2024),
            vec![
                d(2024, 1, 1),
                d(2024, 1, 15),
                d(2024, 2, 19),
                d(2024, 3, 29),
                d(2024, 5, 27),
                d(2024, 6, 19),
                d(2024, 7, 4),
                d(2024, 9, 2),
                d(2024, 11, 28),
                d(2024, 12, 25),
            ]
        );
        assert_eq!(
            early_closes(&cal, 2024),
            vec![d(2024, 7, 3), d(2024, 11, 29), d(2024, 12, 24)]
        );
    }

    #[test]
    fn nyse_2025_includes_carter_day_of_mourning() {
        let cal = MarketCalendar::nyse();
        assert_eq!(
            holidays(&cal, 2025),
            vec![
                d(2025, 1, 1),
                d(2025, 1, 9),
                d(2025, 1, 20),
                d(2025, 2, 17),
                d(2025, 4, 18),
                d(2025, 5, 26),
                d(2025, 6, 19),
                d(2025, 7, 4),
                d(2025, 9, 1),
                d(2025, 11, 27),
                d(2025, 12, 25),
            ]
        );
        assert_eq!(cal.holiday(d(2025, 1, 9)), Some(Holiday::SpecialClosure));
        assert_eq!(
            early_closes(&cal, 2025),
            vec![d(2025, 7, 3), d(2025, 11, 28), d(2025, 12, 24)]
        );
    }

    #[test]
    fn nyse_2026_observes_independence_day_on_friday_july_3() {
        let cal = MarketCalendar::nyse();
        assert_eq!(
            holidays(&cal, 2026),
            vec![
                d(2026, 1, 1),
                d(2026, 1, 19),
                d(2026, 2, 16),
                d(2026, 4, 3),
                d(2026, 5, 25),
                d(2026, 6, 19),
                d(2026, 7, 3),
                d(2026, 9, 7),
                d(2026, 11, 26),
                d(2026, 12, 25),
            ]
        );
        // July 3 is the observed holiday, so it cannot also be an early close.
        assert_eq!(
            early_closes(&cal, 2026),
            vec![d(2026, 11, 27), d(2026, 12, 24)]
        );
    }

    #[test]
    fn nyse_2027_weekend_shifts() {
        let cal = MarketCalendar::nyse();
        assert_eq!(
            holidays(&cal, 2027),
            vec![
                d(2027, 1, 1),
                d(2027, 1, 18),
                d(2027, 2, 15),
                d(2027, 3, 26),
                d(2027, 5, 31),
                d(2027, 6, 18),
                d(2027, 7, 5),
                d(2027, 9, 6),
                d(2027, 11, 25),
                d(2027, 12, 24),
            ]
        );
        assert_eq!(early_closes(&cal, 2027), vec![d(2027, 11, 26)]);
    }

    #[test]
    fn saturday_new_year_is_not_observed_on_friday() {
        let cal = MarketCalendar::nyse();
        // 2022-01-01 was a Saturday; the NYSE traded on Friday 2021-12-31.
        assert!(cal.is_trading_day(d(2021, 12, 31)));
        // 2028-01-01 is a Saturday as well.
        assert!(cal.is_trading_day(d(2027, 12, 31)));
        assert!(cal.holiday(d(2028, 1, 1)).is_none());
    }

    #[test]
    fn session_times_follow_dst() {
        let cal = MarketCalendar::nyse();
        // Summer (EDT, UTC-4): 09:30 ET = 13:30 UTC.
        let summer = cal.session(d(2026, 10, 5)).unwrap();
        assert_eq!(summer.open.to_rfc3339(), "2026-10-05T13:30:00+00:00");
        assert_eq!(summer.close.to_rfc3339(), "2026-10-05T20:00:00+00:00");
        assert_eq!(summer.minutes(), 390);
        // Winter (EST, UTC-5): 09:30 ET = 14:30 UTC.
        let winter = cal.session(d(2026, 12, 1)).unwrap();
        assert_eq!(winter.open.to_rfc3339(), "2026-12-01T14:30:00+00:00");
        // Early close.
        let half = cal.session(d(2026, 11, 27)).unwrap();
        assert!(half.early_close);
        assert_eq!(half.close.to_rfc3339(), "2026-11-27T18:00:00+00:00");
        assert_eq!(half.minutes(), 210);
    }

    #[test]
    fn next_and_previous_trading_days_skip_weekends_and_holidays() {
        let cal = MarketCalendar::nyse();
        // Friday 2026-07-02 → next is Monday 2026-07-06 (July 3 observed holiday).
        assert_eq!(cal.next_trading_day(d(2026, 7, 2)), d(2026, 7, 6));
        assert_eq!(cal.prev_trading_day(d(2026, 7, 6)), d(2026, 7, 2));
        assert_eq!(cal.trading_days(d(2026, 11, 23), d(2026, 11, 29)).len(), 4);
    }

    #[test]
    fn status_reports_phase_and_next_session() {
        let cal = MarketCalendar::nyse();
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let pre = cal.status_at(at("2026-10-05T12:00:00Z"));
        assert_eq!(pre.phase, MarketPhase::PreOpen);
        assert_eq!(pre.next_session.date, d(2026, 10, 5));
        let open = cal.status_at(at("2026-10-05T15:00:00Z"));
        assert_eq!(open.phase, MarketPhase::Open);
        let post = cal.status_at(at("2026-10-05T21:00:00Z"));
        assert_eq!(post.phase, MarketPhase::PostClose);
        assert_eq!(post.next_session.date, d(2026, 10, 6));
        // Saturday.
        let weekend = cal.status_at(at("2026-10-10T15:00:00Z"));
        assert_eq!(weekend.phase, MarketPhase::Closed);
        assert_eq!(weekend.next_session.date, d(2026, 10, 12));
        // Thanksgiving.
        let holiday = cal.status_at(at("2026-11-26T15:00:00Z"));
        assert_eq!(holiday.phase, MarketPhase::Closed);
        assert_eq!(holiday.holiday, Some(Holiday::ThanksgivingDay));
    }

    #[test]
    fn configured_closures_and_early_closes_apply() {
        let mut cal = MarketCalendar::nyse();
        cal.add_closure(d(2026, 10, 6));
        assert!(!cal.is_trading_day(d(2026, 10, 6)));
        cal.add_early_close(d(2026, 10, 7), NaiveTime::from_hms_opt(14, 0, 0).unwrap());
        assert!(cal.session(d(2026, 10, 7)).unwrap().early_close);
    }
}
