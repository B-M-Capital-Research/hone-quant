use std::sync::Mutex as StdMutex;
use std::time::{Duration as StdDuration, Instant};

use chrono::{Datelike, Weekday};
use quant_core::stats::{correlation, stdev};

use super::model::SessionPath;
use super::rng::Rng;
use super::*;

const SEED: u64 = 42;
/// Monday 2026-10-05, 08:00 ET: before the open.
const PRE_OPEN: &str = "2026-10-05T12:00:00Z";
/// Monday 2026-10-05, 11:00 ET: session in progress.
const MID_SESSION: &str = "2026-10-05T15:00:00Z";
/// Monday 2026-10-05, 17:00 ET: after the close.
const AFTER_CLOSE: &str = "2026-10-05T21:00:00Z";
/// Nothing traded yet.
const AT_OPEN: Progress = Progress {
    step: 0,
    fraction: 0.0,
};

fn universe() -> Vec<DemoInstrument> {
    [
        ("NVDA", "ai-chip"),
        ("AMD", "ai-chip"),
        ("AVGO", "ai-chip"),
        ("ARM", "ai-chip"),
        ("ALAB", "ai-chip"),
        ("MU", "storage"),
        ("WDC", "storage"),
        ("SNDK", "storage"),
        ("LITE", "optical"),
        ("COHR", "optical"),
        ("CRDO", "optical"),
        ("VST", "power"),
        ("CEG", "power"),
        ("GEV", "power"),
        ("TLN", "power"),
        ("OKLO", "power"),
        ("BE", "power"),
        ("BWXT", "power"),
        ("CRWV", "neocloud"),
        ("NBIS", "neocloud"),
        ("IREN", "neocloud"),
        ("AMAT", "equipment"),
        ("LRCX", "equipment"),
        ("ONTO", "equipment"),
        ("DELL", "server-oem"),
        ("HPE", "server-oem"),
        ("SMCI", "server-oem"),
        ("MSFT", "hyperscaler"),
        ("GOOGL", "hyperscaler"),
        ("AMZN", "hyperscaler"),
        ("META", "hyperscaler"),
        ("RKLB", "space"),
        ("PLTR", "ai-apps"),
        ("SNOW", "ai-apps"),
        ("TEM", "ai-apps"),
        ("HIMS", "ai-apps"),
        ("SPY", "benchmark"),
        ("QQQ", "benchmark"),
        ("SMH", "benchmark"),
    ]
    .into_iter()
    .map(|(symbol, sector)| instrument(symbol, sector))
    .collect()
}

fn instrument(symbol: &str, sector: &str) -> DemoInstrument {
    DemoInstrument {
        symbol: symbol.to_string(),
        sector: sector.to_string(),
    }
}

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn market_at(now: &str) -> DemoMarket {
    DemoMarket::new(universe(), SEED, fixed_clock(at(now)))
}

fn symbols(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn log_returns(bars: &[DailyBar]) -> Vec<f64> {
    bars.windows(2)
        .map(|w| (w[1].close / w[0].close).ln())
        .collect()
}

fn max_high(bars: &[IntradayBar]) -> f64 {
    bars.iter()
        .map(|b| b.high)
        .fold(f64::NEG_INFINITY, f64::max)
}

fn min_low(bars: &[IntradayBar]) -> f64 {
    bars.iter().map(|b| b.low).fold(f64::INFINITY, f64::min)
}

#[tokio::test]
async fn deterministic_across_instances_ranges_and_universes() {
    let a = market_at(AFTER_CLOSE);
    let b = market_at(AFTER_CLOSE);
    let full = a
        .daily_bars("NVDA", d(2015, 1, 1), d(2026, 10, 5))
        .await
        .unwrap();
    // A later range first, then an overlapping earlier one: the cache is built and extended in a
    // different order, yet every overlapping session is identical.
    let late = b
        .daily_bars("NVDA", d(2021, 6, 1), d(2022, 6, 30))
        .await
        .unwrap();
    let early = b
        .daily_bars("NVDA", d(2020, 1, 1), d(2021, 12, 31))
        .await
        .unwrap();
    let by_date: HashMap<NaiveDate, &DailyBar> = full.iter().map(|bar| (bar.date, bar)).collect();
    assert!(!late.is_empty() && !early.is_empty());
    for bar in late.iter().chain(&early) {
        assert_eq!(by_date[&bar.date], bar);
    }
    assert_eq!(
        b.daily_bars("NVDA", d(2015, 1, 1), d(2026, 10, 5))
            .await
            .unwrap(),
        full
    );

    // Values depend on (seed, symbol, sector, date) only: not on the rest of the universe, nor on
    // the symbol's spelling.
    let solo = DemoMarket::new(
        vec![instrument(" nvda ", "ai-chip")],
        SEED,
        fixed_clock(at(AFTER_CLOSE)),
    );
    assert_eq!(
        solo.daily_bars("NVDA", d(2015, 1, 1), d(2026, 10, 5))
            .await
            .unwrap(),
        full
    );

    let day = d(2026, 10, 5);
    assert_eq!(
        a.intraday_bars("NVDA", Interval::FiveMin, day, day)
            .await
            .unwrap(),
        b.intraday_bars("NVDA", Interval::FiveMin, day, day)
            .await
            .unwrap()
    );
    let list = symbols(&["NVDA", "SPY", "CRWV"]);
    assert_eq!(
        a.quotes(&list).await.unwrap(),
        b.quotes(&list).await.unwrap()
    );

    let other = DemoMarket::new(universe(), SEED + 1, fixed_clock(at(AFTER_CLOSE)));
    assert_ne!(
        other
            .daily_bars("NVDA", d(2015, 1, 1), d(2026, 10, 5))
            .await
            .unwrap(),
        full
    );
}

#[tokio::test]
async fn sessions_follow_the_nyse_calendar() {
    let market = market_at("2026-12-01T21:00:00Z");
    let calendar = MarketCalendar::nyse();

    let all = market
        .daily_bars("SPY", d(2010, 1, 1), d(2026, 12, 31))
        .await
        .unwrap();
    let dates: Vec<NaiveDate> = all.iter().map(|bar| bar.date).collect();
    assert_eq!(dates, calendar.trading_days(HISTORY_START, d(2026, 12, 1)));
    assert!(
        dates
            .iter()
            .all(|date| !matches!(date.weekday(), Weekday::Sat | Weekday::Sun))
    );
    for closed in [
        d(2026, 7, 3),
        d(2026, 11, 26),
        d(2025, 1, 9),
        d(2018, 12, 5),
    ] {
        assert!(!dates.contains(&closed), "{closed} is not a session");
    }

    // 13:00 early close: 42 five-minute bars, the last one starting 12:55 ET.
    let day = d(2026, 11, 27);
    let half = market
        .intraday_bars("NVDA", Interval::FiveMin, day, day)
        .await
        .unwrap();
    assert_eq!(half.len(), 42);
    assert_eq!(half[0].ts, at("2026-11-27T14:30:00Z"));
    assert_eq!(half[41].ts, at("2026-11-27T17:55:00Z"));
    let hourly = market
        .intraday_bars("NVDA", Interval::OneHour, day, day)
        .await
        .unwrap();
    assert_eq!(hourly.len(), 4);

    let day = d(2026, 11, 30);
    let full = market
        .intraday_bars("NVDA", Interval::FiveMin, day, day)
        .await
        .unwrap();
    assert_eq!(full.len(), 78);
    assert_eq!(full[77].ts, at("2026-11-30T20:55:00Z"));

    // Holidays and weekends have no intraday bars either.
    for (from, to) in [
        (d(2026, 11, 26), d(2026, 11, 26)),
        (d(2026, 7, 3), d(2026, 7, 5)),
    ] {
        let bars = market
            .intraday_bars("NVDA", Interval::FiveMin, from, to)
            .await
            .unwrap();
        assert!(bars.is_empty(), "{from}..{to}");
    }
}

#[tokio::test]
async fn late_listings_start_at_their_listing_date() {
    let market = market_at(AFTER_CLOSE);
    for (symbol, first) in [
        ("CRWV", d(2025, 3, 28)),
        ("SNDK", d(2025, 2, 24)),
        ("ALAB", d(2024, 3, 20)),
        ("VST", d(2016, 10, 5)),
        ("NVDA", d(2015, 1, 2)),
        ("META", d(2015, 1, 2)),
    ] {
        assert_eq!(listing_date(symbol), first);
        let bars = market
            .daily_bars(symbol, d(2010, 1, 1), d(2026, 10, 5))
            .await
            .unwrap();
        assert_eq!(bars[0].date, first, "{symbol}");
    }
    assert_eq!(listing_date("UNLISTED"), HISTORY_START);

    assert!(
        market
            .daily_bars("CRWV", d(2020, 1, 1), d(2025, 3, 27))
            .await
            .unwrap()
            .is_empty()
    );
    let first_day = market
        .intraday_bars("CRWV", Interval::FiveMin, d(2025, 3, 27), d(2025, 3, 28))
        .await
        .unwrap();
    assert_eq!(first_day.len(), 78);
    assert_eq!(first_day[0].ts, at("2025-03-28T13:30:00Z"));

    // Before the listing there is no quote; on the listing day the change is measured against
    // the synthetic offer price and there is no volume history yet.
    let before = market_at("2025-03-27T15:00:00Z");
    let quotes = before.quotes(&symbols(&["CRWV", "NVDA"])).await.unwrap();
    assert_eq!(quotes.len(), 1);
    assert_eq!(quotes[0].symbol, "NVDA");
    let listing_day = market_at("2025-03-28T15:00:00Z");
    let quote = listing_day
        .quotes(&symbols(&["CRWV"]))
        .await
        .unwrap()
        .remove(0);
    assert!(quote.prev_close.is_some_and(|p| p > 0.0));
    assert_eq!(quote.avg_volume, None);
}

#[tokio::test]
async fn price_levels_are_anchored_to_realistic_values() {
    let market = market_at(AFTER_CLOSE);
    for instrument in universe() {
        let symbol = instrument.symbol.as_str();
        let bars = market
            .daily_bars(symbol, model::PRICE_ANCHOR, model::PRICE_ANCHOR)
            .await
            .unwrap();
        let close = bars[0].close;
        assert!(
            (14.9..=500.1).contains(&close),
            "{symbol} closed at {close} on the anchor date"
        );
        // A quarter later prices have moved, but nothing has compounded into five figures.
        let latest = market
            .daily_bars(symbol, d(2026, 10, 5), d(2026, 10, 5))
            .await
            .unwrap();
        assert!(latest[0].close < 2_000.0, "{symbol} at {}", latest[0].close);
    }
}

#[tokio::test]
async fn ohlc_invariants_hold_over_full_history() {
    let market = market_at(MID_SESSION);
    for symbol in ["NVDA", "IREN", "CRWV", "MSFT", "RKLB", "SPY", "SMH"] {
        let bars = market
            .daily_bars(symbol, HISTORY_START, d(2026, 10, 5))
            .await
            .unwrap();
        assert!(bars.len() > 300, "{symbol}");
        for bar in &bars {
            assert!(bar.low > 0.0, "{bar:?}");
            assert!(bar.low <= bar.open.min(bar.close), "{bar:?}");
            assert!(bar.open.max(bar.close) <= bar.high, "{bar:?}");
            assert!(bar.volume > 0.0 && bar.volume.fract() == 0.0, "{bar:?}");
            assert_eq!(bar.adj_open, Some(bar.open));
            assert_eq!(bar.adj_close, Some(bar.close));
            assert_eq!(bar.symbol, symbol);
        }
        for pair in bars.windows(2) {
            assert!(pair[0].date < pair[1].date);
            // ±35% clamp, plus rounding to the cent.
            let move_ = pair[1].close / pair[0].close - 1.0;
            assert!(move_.abs() < 0.36, "{symbol} {}: {move_}", pair[1].date);
        }
    }
    assert!(market.splits("NVDA").await.unwrap().is_empty());
    assert!(market.dividends("NVDA").await.unwrap().is_empty());
    assert_eq!(market.source(), DataSource::Demo);
}

#[tokio::test]
async fn same_sector_returns_correlate_more_than_cross_sector() {
    let market = market_at(AFTER_CLOSE);
    let mut returns = HashMap::new();
    for symbol in [
        "NVDA", "AMD", "AVGO", "MU", "WDC", "MSFT", "GOOGL", "AMZN", "BWXT", "SPY", "SMH",
    ] {
        let bars = market
            .daily_bars(symbol, d(2016, 1, 4), d(2025, 12, 31))
            .await
            .unwrap();
        returns.insert(symbol, log_returns(&bars));
    }
    let corr = |a: &str, b: &str| correlation(&returns[a], &returns[b]).unwrap();
    let mean = |pairs: &[(&str, &str)]| {
        pairs.iter().map(|(a, b)| corr(a, b)).sum::<f64>() / pairs.len() as f64
    };
    let same = [
        ("NVDA", "AMD"),
        ("NVDA", "AVGO"),
        ("AMD", "AVGO"),
        ("MU", "WDC"),
        ("MSFT", "GOOGL"),
        ("MSFT", "AMZN"),
    ];
    let cross = [
        ("NVDA", "MSFT"),
        ("AMD", "GOOGL"),
        ("AVGO", "MU"),
        ("WDC", "AMZN"),
        ("MSFT", "BWXT"),
        ("NVDA", "BWXT"),
    ];
    let (same_mean, cross_mean) = (mean(&same), mean(&cross));
    assert!(
        same_mean > cross_mean + 0.1,
        "same-sector {same_mean:.3} vs cross-sector {cross_mean:.3}"
    );
    for (a, b) in same {
        assert!(corr(a, b) > cross_mean, "{a}/{b}: {:.3}", corr(a, b));
    }
    // Every name shares the market factor; SMH also carries the AI-chip factor.
    assert!(cross_mean > 0.1);
    assert!(corr("SMH", "NVDA") > corr("SPY", "NVDA"));
}

#[tokio::test]
async fn mid_session_quote_follows_the_intraday_path() {
    let day = d(2026, 10, 5);
    // On a step boundary (18 bars done) and mid-step (a partial 19th bar).
    for (now, bars) in [(MID_SESSION, 18), ("2026-10-05T15:02:30Z", 19)] {
        let market = market_at(now);
        let quote = market.quotes(&symbols(&["NVDA"])).await.unwrap().remove(0);
        let five = market
            .intraday_bars("NVDA", Interval::FiveMin, day, day)
            .await
            .unwrap();
        assert_eq!(five.len(), bars);
        let last = five.last().unwrap();
        assert!(last.ts < at(now));
        assert!((quote.price - last.close).abs() < 1e-9, "{now}");
        assert_eq!(quote.open, Some(five[0].open));
        assert_eq!(quote.day_high, Some(max_high(&five)));
        assert_eq!(quote.day_low, Some(min_low(&five)));
        assert_eq!(quote.volume, Some(five.iter().map(|b| b.volume).sum()));
        assert_eq!(quote.timestamp, Some(at(now)));

        let prev = market
            .daily_bars("NVDA", d(2026, 10, 2), d(2026, 10, 2))
            .await
            .unwrap();
        let prev_close = prev[0].close;
        assert_eq!(quote.prev_close, Some(prev_close));
        let change = quote.change.unwrap();
        assert!((change - (quote.price - prev_close)).abs() < 1e-9);
        let change_pct = quote.change_pct.unwrap();
        assert!((change_pct - change / prev_close).abs() < 1e-12);
        assert!(change_pct.abs() < 0.35, "change_pct is a fraction");
        let avg_volume = quote.avg_volume.unwrap();
        assert!(avg_volume > 0.0);
        assert!(quote.market_cap.unwrap() > 0.0);

        // Today's partial daily bar is the same snapshot.
        let today = market.daily_bars("NVDA", day, day).await.unwrap();
        assert_eq!(today.len(), 1);
        assert_eq!(today[0].close, quote.price);
        assert_eq!(Some(today[0].open), quote.open);
        assert_eq!(Some(today[0].high), quote.day_high);
        assert_eq!(Some(today[0].low), quote.day_low);
        assert_eq!(Some(today[0].volume), quote.volume);
    }
}

#[tokio::test]
async fn closed_market_quotes_show_the_last_close() {
    // After the close: today's close, stamped at the 16:00 ET close.
    let market = market_at(AFTER_CLOSE);
    let quote = market.quotes(&symbols(&["NVDA"])).await.unwrap().remove(0);
    let bars = market
        .daily_bars("NVDA", d(2026, 9, 1), d(2026, 10, 5))
        .await
        .unwrap();
    let (prev, today) = (&bars[bars.len() - 2], &bars[bars.len() - 1]);
    assert_eq!(today.date, d(2026, 10, 5));
    assert_eq!(quote.price, today.close);
    assert_eq!(quote.open, Some(today.open));
    assert_eq!(quote.day_high, Some(today.high));
    assert_eq!(quote.day_low, Some(today.low));
    assert_eq!(quote.volume, Some(today.volume));
    assert_eq!(quote.prev_close, Some(prev.close));
    assert_eq!(quote.timestamp, Some(at("2026-10-05T20:00:00Z")));
    let last_20: f64 = bars[bars.len() - 20..].iter().map(|b| b.volume).sum();
    assert_eq!(quote.avg_volume, Some((last_20 / 20.0).round()));

    // Before the open and over the weekend: the previous session's close.
    for (now, session) in [
        (PRE_OPEN, d(2026, 10, 2)),
        ("2026-10-10T15:00:00Z", d(2026, 10, 9)),
    ] {
        let market = market_at(now);
        let quote = market.quotes(&symbols(&["NVDA"])).await.unwrap().remove(0);
        let bars = market
            .daily_bars("NVDA", d(2026, 9, 1), d(2026, 12, 31))
            .await
            .unwrap();
        let (prev, last) = (&bars[bars.len() - 2], &bars[bars.len() - 1]);
        assert_eq!(last.date, session, "{now}");
        assert_eq!(quote.price, last.close);
        assert_eq!(quote.prev_close, Some(prev.close));
        let change = quote.change.unwrap();
        assert!((change - (last.close - prev.close)).abs() < 1e-9);
        assert!((quote.change_pct.unwrap() - change / prev.close).abs() < 1e-12);
        let close_time = MarketCalendar::nyse().session(session).unwrap().close;
        assert_eq!(quote.timestamp, Some(close_time));
    }
}

#[tokio::test]
async fn partial_bar_only_while_the_session_is_open() {
    let range = (d(2026, 9, 28), d(2026, 10, 9));
    let pre = market_at(PRE_OPEN)
        .daily_bars("NVDA", range.0, range.1)
        .await
        .unwrap();
    let mid = market_at(MID_SESSION)
        .daily_bars("NVDA", range.0, range.1)
        .await
        .unwrap();
    let post = market_at(AFTER_CLOSE)
        .daily_bars("NVDA", range.0, range.1)
        .await
        .unwrap();
    assert_eq!(pre.last().unwrap().date, d(2026, 10, 2));
    assert_eq!(mid.last().unwrap().date, d(2026, 10, 5));
    assert_eq!(post.last().unwrap().date, d(2026, 10, 5));
    // Completed sessions do not depend on the clock.
    assert_eq!(pre[..], mid[..mid.len() - 1]);
    assert_eq!(pre[..], post[..post.len() - 1]);
    let (partial, full) = (mid.last().unwrap(), post.last().unwrap());
    assert_eq!(partial.open, full.open);
    assert!(partial.high <= full.high);
    assert!(partial.low >= full.low);
    assert!(partial.volume > 0.0 && partial.volume < full.volume);
}

#[tokio::test]
async fn nothing_after_now_is_returned() {
    let now = at(MID_SESSION);
    let market = market_at(MID_SESSION);
    let daily = market
        .daily_bars("NVDA", d(2026, 1, 1), d(2030, 1, 1))
        .await
        .unwrap();
    assert_eq!(daily.last().unwrap().date, d(2026, 10, 5));
    assert!(
        market
            .daily_bars("NVDA", d(2026, 10, 6), d(2030, 1, 1))
            .await
            .unwrap()
            .is_empty()
    );
    for interval in [
        Interval::FiveMin,
        Interval::FifteenMin,
        Interval::ThirtyMin,
        Interval::OneHour,
    ] {
        let bars = market
            .intraday_bars("NVDA", interval, d(2026, 9, 30), d(2026, 12, 31))
            .await
            .unwrap();
        assert!(!bars.is_empty());
        assert!(bars.iter().all(|bar| bar.ts < now), "{interval:?}");
        assert!(bars.windows(2).all(|w| w[0].ts < w[1].ts), "{interval:?}");
    }
    assert!(
        market
            .intraday_bars("NVDA", Interval::FiveMin, d(2026, 10, 6), d(2026, 10, 9))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        market
            .daily_bars("NVDA", d(2026, 10, 5), d(2026, 10, 1))
            .await
            .unwrap()
            .is_empty()
    );

    // Before the synthetic history starts there is nothing at all.
    let early = market_at("2014-06-02T15:00:00Z");
    assert!(early.quotes(&symbols(&["NVDA"])).await.unwrap().is_empty());
    assert!(
        early
            .daily_bars("NVDA", d(2014, 1, 1), d(2016, 1, 1))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn intraday_bars_aggregate_the_five_minute_path() {
    let market = market_at(AFTER_CLOSE);
    let day = d(2026, 10, 5);
    let five = market
        .intraday_bars("NVDA", Interval::FiveMin, day, day)
        .await
        .unwrap();
    assert_eq!(five.len(), 78);
    for interval in [Interval::FifteenMin, Interval::ThirtyMin, Interval::OneHour] {
        let minutes = interval.minutes();
        let bars = market
            .intraday_bars("NVDA", interval, day, day)
            .await
            .unwrap();
        assert_eq!(bars.len() as i64, (390 + minutes - 1) / minutes);
        assert_eq!(bars[0].ts, at("2026-10-05T13:30:00Z"));
        for bar in &bars {
            let end = bar.ts + Duration::minutes(minutes);
            let parts: Vec<IntradayBar> = five
                .iter()
                .filter(|b| b.ts >= bar.ts && b.ts < end)
                .cloned()
                .collect();
            assert_eq!(bar.open, parts[0].open);
            assert_eq!(bar.close, parts.last().unwrap().close);
            assert_eq!(bar.high, max_high(&parts));
            assert_eq!(bar.low, min_low(&parts));
            assert_eq!(bar.volume, parts.iter().map(|b| b.volume).sum::<f64>());
        }
    }

    // Every daily bar is exactly the aggregate of its session's 5-minute bars.
    for symbol in ["NVDA", "IREN", "SPY"] {
        let daily = market
            .daily_bars(symbol, d(2026, 8, 1), d(2026, 10, 5))
            .await
            .unwrap();
        for bar in &daily {
            let five = market
                .intraday_bars(symbol, Interval::FiveMin, bar.date, bar.date)
                .await
                .unwrap();
            assert_eq!(five[0].open, bar.open, "{symbol} {}", bar.date);
            assert_eq!(five.last().unwrap().close, bar.close);
            assert_eq!(max_high(&five), bar.high);
            assert_eq!(min_low(&five), bar.low);
            assert_eq!(five.iter().map(|b| b.volume).sum::<f64>(), bar.volume);
        }
    }
}

#[test]
fn session_range_matches_its_bars_exactly() {
    let mut rng = Rng::new(7);
    for key in 0..2_000_u64 {
        let steps = if key % 3 == 0 { 42 } else { 78 };
        let open = 1.0 + 500.0 * rng.unit();
        let close = open * (0.7 + 0.6 * rng.unit());
        let vol = 0.002 + 0.08 * rng.unit();
        let path = SessionPath::new(key, steps, open, close, vol);
        let mid_step = Progress {
            step: key as usize % steps,
            fraction: rng.unit(),
        };
        for progress in [Progress::complete(steps), mid_step, AT_OPEN] {
            let mut expected = (open, open);
            for (k, fraction) in path.bars(progress) {
                let bar = path.bar(k, fraction);
                assert!(bar.low > 0.0);
                assert!(bar.low <= bar.open.min(bar.close));
                assert!(bar.open.max(bar.close) <= bar.high);
                expected = (expected.0.max(bar.high), expected.1.min(bar.low));
            }
            assert_eq!(path.range(progress), expected);
        }
        assert_eq!(path.price_at(Progress::complete(steps)), close);
        assert_eq!(path.price_at(AT_OPEN), open);
    }
}

#[tokio::test]
async fn unknown_symbols_are_omitted() {
    let market = market_at(MID_SESSION);
    let quotes = market
        .quotes(&symbols(&["NVDA", "NOPE", "spy", "NVDA"]))
        .await
        .unwrap();
    let returned: Vec<&str> = quotes.iter().map(|q| q.symbol.as_str()).collect();
    assert_eq!(returned, ["NVDA", "SPY"]);
    assert!(
        market
            .daily_bars("NOPE", d(2015, 1, 1), d(2026, 10, 5))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        market
            .intraday_bars("NOPE", Interval::FiveMin, d(2026, 10, 5), d(2026, 10, 5))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn history_follows_the_clock_lazily() {
    let now = Arc::new(StdMutex::new(at(MID_SESSION)));
    let shared = now.clone();
    let clock: Clock = Arc::new(move || *shared.lock().unwrap());
    let market = DemoMarket::new(universe(), SEED, clock);
    let range = (d(2026, 9, 1), d(2026, 12, 31));

    let first = market.daily_bars("NVDA", range.0, range.1).await.unwrap();
    assert_eq!(first.last().unwrap().date, d(2026, 10, 5));

    *now.lock().unwrap() = at("2026-10-07T21:00:00Z");
    let later = market.daily_bars("NVDA", range.0, range.1).await.unwrap();
    assert_eq!(later.last().unwrap().date, d(2026, 10, 7));
    let fresh = market_at("2026-10-07T21:00:00Z")
        .daily_bars("NVDA", range.0, range.1)
        .await
        .unwrap();
    assert_eq!(later, fresh);

    // Moving the clock back hides the newer sessions again.
    *now.lock().unwrap() = at(MID_SESSION);
    assert_eq!(
        market.daily_bars("NVDA", range.0, range.1).await.unwrap(),
        first
    );
}

#[tokio::test]
async fn market_regimes_shape_the_benchmarks_for_any_seed() {
    for seed in [1, SEED, 2026] {
        let market = DemoMarket::new(universe(), seed, fixed_clock(at(AFTER_CLOSE)));
        let mut vols = Vec::new();
        for symbol in ["SPY", "QQQ", "SMH"] {
            let bars = market
                .daily_bars(symbol, HISTORY_START, d(2026, 10, 5))
                .await
                .unwrap();
            let close_on = |date: NaiveDate| bars.iter().find(|b| b.date >= date).unwrap().close;
            let growth = bars.last().unwrap().close / bars[0].close;
            let crash = close_on(d(2020, 3, 23)) / close_on(d(2020, 2, 19)) - 1.0;
            let bear = close_on(d(2022, 10, 12)) / close_on(d(2022, 1, 3)) - 1.0;
            let context =
                format!("seed {seed} {symbol}: x{growth:.2} crash {crash:.2} bear {bear:.2}");
            assert!(growth > 1.5, "{context}");
            assert!(crash < -0.2, "{context}");
            assert!(bear < -0.1, "{context}");
            if symbol == "SPY" {
                // The market factor is pinned inside regimes: −34% and −25% whatever the seed.
                assert!((2.0..5.0).contains(&growth), "{context}");
                assert!((-0.40..-0.28).contains(&crash), "{context}");
                assert!((-0.31..-0.19).contains(&bear), "{context}");
            }
            vols.push(stdev(&log_returns(&bars)).unwrap() * 252f64.sqrt());
        }
        assert!(
            (0.15..0.22).contains(&vols[0]),
            "seed {seed}: SPY vol {vols:?}"
        );
        assert!(
            vols[0] < vols[1] && vols[1] < vols[2],
            "seed {seed}: {vols:?}"
        );
    }
}

#[test]
fn full_history_for_seventy_symbols_builds_quickly() {
    let sectors = [
        "ai-chip",
        "storage",
        "optical",
        "power",
        "neocloud",
        "equipment",
        "server-oem",
        "hyperscaler",
        "space",
        "ai-apps",
    ];
    let mut instruments = universe();
    for i in 0..(70 - instruments.len()) {
        instruments.push(instrument(&format!("SYN{i}"), sectors[i % sectors.len()]));
    }
    let now = at(AFTER_CLOSE);
    let market = DemoMarket::new(instruments.clone(), SEED, fixed_clock(now));
    let build = |market: &DemoMarket| {
        instruments
            .iter()
            .map(|i| {
                market
                    .daily_bars_at(&i.symbol, HISTORY_START, d(2026, 10, 5), now)
                    .len()
            })
            .sum::<usize>()
    };
    let started = Instant::now();
    let sessions = build(&market);
    let cold = started.elapsed();
    let started = Instant::now();
    assert_eq!(build(&market), sessions);
    let warm = started.elapsed();
    eprintln!(
        "{} symbols, {sessions} sessions: built in {cold:?}, cached in {warm:?}",
        instruments.len()
    );
    assert!(sessions > 150_000);
    // Generous so slow machines pass; a release build takes a fraction of a second.
    assert!(cold < StdDuration::from_secs(20), "took {cold:?}");
    assert!(warm < cold);
}
