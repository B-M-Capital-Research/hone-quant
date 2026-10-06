//! FMP response shapes and their mapping onto the domain types.
//!
//! One row type per data kind serves both API generations: field names mostly agree, and where
//! they differ (`changePercentage` vs `changesPercentage`) both are declared. Parsing is lenient
//! per row — a `null`, a numeric string or one malformed row must not cost the whole response —
//! but strict about the envelope, so an unexpected object is reported instead of being read as
//! "no data".

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use quant_core::calendar::{MARKET_TZ, regular_close, regular_open};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::transport::FetchError;
use crate::market::{DailyBar, Dividend, IntradayBar, Quote, Split};

/// A numeric field FMP may send as a number, a numeric string or `null`. Deserialising never
/// fails; anything unusable (including NaN and infinities) becomes `None`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Num(Option<f64>);

impl Num {
    fn value(self) -> Option<f64> {
        self.0
    }

    fn positive(self) -> Option<f64> {
        self.0.filter(|value| *value > 0.0)
    }

    fn non_negative(self) -> Option<f64> {
        self.0.filter(|value| *value >= 0.0)
    }
}

impl<'de> Deserialize<'de> for Num {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let number = match Value::deserialize(deserializer)? {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.trim().parse::<f64>().ok(),
            _ => None,
        };
        Ok(Num(number.filter(|value| value.is_finite())))
    }
}

/// The data rows of a response: a bare array (stable; legacy quotes and intraday) or the
/// `historical` array of a legacy `{symbol, historical}` object. Legacy answers `{}` (or a bare
/// `{symbol}`) when it has no data.
fn envelope_rows(value: Value) -> Result<Vec<Value>, FetchError> {
    match value {
        Value::Array(rows) => Ok(rows),
        Value::Null => Ok(Vec::new()),
        Value::Object(mut object) => match object.remove("historical") {
            Some(Value::Array(rows)) => Ok(rows),
            Some(Value::Null) => Ok(Vec::new()),
            Some(other) => Err(FetchError::Parse(format!(
                "`historical` is {}, not an array",
                describe(&other)
            ))),
            None if object.keys().all(|field| field == "symbol") => Ok(Vec::new()),
            None => Err(FetchError::Parse(format!(
                "expected rows, got an object with fields {}",
                object
                    .keys()
                    .take(6)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        },
        other => Err(FetchError::Parse(format!(
            "expected rows, got {}",
            describe(&other)
        ))),
    }
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Deserialises each row on its own, so one malformed row is skipped rather than fatal.
fn parse_rows<T: DeserializeOwned>(value: Value) -> Result<Vec<T>, FetchError> {
    let rows = envelope_rows(value)?;
    let total = rows.len();
    let parsed: Vec<T> = rows
        .into_iter()
        .filter_map(|row| serde_json::from_value(row).ok())
        .collect();
    if parsed.len() < total {
        tracing::debug!(skipped = total - parsed.len(), "skipped malformed FMP rows");
    }
    Ok(parsed)
}

/// The `YYYY-MM-DD` prefix of a date or date-time string; empty strings (FMP's "unknown") are
/// `None`.
fn parse_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.trim().get(..10)?, "%Y-%m-%d").ok()
}

/// A zone-less `YYYY-MM-DD HH:MM[:SS]` timestamp.
fn parse_local_datetime(text: &str) -> Option<NaiveDateTime> {
    let text = text.trim();
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M"))
        .ok()
}

/// Unix seconds — or milliseconds, which some FMP feeds use — as UTC.
fn unix_time(value: f64) -> Option<DateTime<Utc>> {
    if value <= 0.0 {
        return None;
    }
    // 1e11 seconds is the year 5138, while 1e11 milliseconds is 1973.
    let seconds = if value >= 1e11 { value / 1000.0 } else { value };
    DateTime::from_timestamp(seconds as i64, 0)
}

/// A quote row from `/stable/quote`, `/stable/batch-quote` or `/api/v3/quote`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct QuoteRow {
    symbol: Option<String>,
    price: Num,
    change: Num,
    /// Stable spelling; percent, e.g. 1.23 for +1.23%.
    change_percentage: Num,
    /// Legacy spelling of the same percent.
    changes_percentage: Num,
    open: Num,
    day_high: Num,
    day_low: Num,
    previous_close: Num,
    volume: Num,
    /// Legacy only.
    avg_volume: Num,
    market_cap: Num,
    /// Unix seconds.
    timestamp: Num,
}

impl QuoteRow {
    fn into_quote(self) -> Option<Quote> {
        let symbol = self.symbol?.trim().to_string();
        let price = self.price.positive()?;
        if symbol.is_empty() {
            return None;
        }
        let prev_close = self.previous_close.positive();
        // Prices give the exact fraction; the percent fields are rounded and spelled differently
        // by the two API generations, so they are only the fallback.
        let change_pct = match prev_close {
            Some(prev) => Some((price - prev) / prev),
            None => self
                .change_percentage
                .value()
                .or(self.changes_percentage.value())
                .map(|percent| percent / 100.0),
        };
        Some(Quote {
            symbol,
            price,
            change: self
                .change
                .value()
                .or_else(|| prev_close.map(|prev| price - prev)),
            change_pct,
            open: self.open.positive(),
            day_high: self.day_high.positive(),
            day_low: self.day_low.positive(),
            prev_close,
            volume: self.volume.non_negative(),
            avg_volume: self.avg_volume.non_negative(),
            market_cap: self.market_cap.positive(),
            timestamp: self.timestamp.value().and_then(unix_time),
        })
    }
}

/// Quotes with a positive price; other rows are skipped.
pub(super) fn quotes(value: Value) -> Result<Vec<Quote>, FetchError> {
    Ok(parse_rows::<QuoteRow>(value)?
        .into_iter()
        .filter_map(QuoteRow::into_quote)
        .collect())
}

/// A daily or intraday price row.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct BarRow {
    /// `YYYY-MM-DD` for daily rows; `YYYY-MM-DD HH:MM:SS` New York wall-clock time for intraday.
    date: Option<String>,
    open: Num,
    high: Num,
    low: Num,
    close: Num,
    volume: Num,
    /// Legacy daily rows only: the split- and dividend-adjusted close.
    adj_close: Num,
}

struct Ohlcv {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

impl BarRow {
    /// FMP occasionally publishes a zero or null open/high/low for thinly traded sessions; those
    /// are filled from the close instead of dropping the bar. Without a close a bar is unusable.
    fn ohlcv(&self) -> Option<Ohlcv> {
        let close = self.close.positive()?;
        let open = self.open.positive().unwrap_or(close);
        Some(Ohlcv {
            open,
            high: self.high.positive().unwrap_or(open.max(close)),
            low: self.low.positive().unwrap_or(open.min(close)),
            close,
            volume: self.volume.non_negative().unwrap_or(0.0),
        })
    }
}

/// Daily bars within `[from, to]`, oldest first, one per date. Rows carrying `adjClose` (legacy)
/// also get an `adj_open` scaled by the same adjustment factor as the close.
pub(super) fn daily_bars(
    value: Value,
    symbol: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<DailyBar>, FetchError> {
    let mut bars = BTreeMap::new();
    for row in parse_rows::<BarRow>(value)? {
        let Some(date) = row.date.as_deref().and_then(parse_date) else {
            continue;
        };
        if date < from || date > to {
            continue;
        }
        let Some(Ohlcv {
            open,
            high,
            low,
            close,
            volume,
        }) = row.ohlcv()
        else {
            continue;
        };
        let adj_close = row.adj_close.positive();
        bars.entry(date).or_insert(DailyBar {
            symbol: symbol.to_string(),
            date,
            open,
            high,
            low,
            close,
            volume,
            adj_open: adj_close.map(|adj| open * adj / close),
            adj_close,
        });
    }
    Ok(bars.into_values().collect())
}

/// A row of `/stable/historical-price-eod/dividend-adjusted`, or a legacy daily row used for its
/// `adjClose`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct AdjustedRow {
    date: Option<String>,
    /// Stable rows only.
    adj_open: Num,
    adj_close: Num,
}

/// Split- and dividend-adjusted (total return) prices for one day.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct AdjustedPrices {
    pub(super) date: NaiveDate,
    /// Absent from legacy rows; derived from the close's adjustment factor when merging.
    pub(super) open: Option<f64>,
    pub(super) close: f64,
}

pub(super) fn adjusted_prices(value: Value) -> Result<Vec<AdjustedPrices>, FetchError> {
    Ok(parse_rows::<AdjustedRow>(value)?
        .into_iter()
        .filter_map(|row| {
            Some(AdjustedPrices {
                date: parse_date(row.date.as_deref()?)?,
                open: row.adj_open.positive(),
                close: row.adj_close.positive()?,
            })
        })
        .collect())
}

/// Copies adjusted prices onto the bars of the same date. Without an adjusted open, the open is
/// scaled by the close's adjustment factor: dividend adjustment is one multiplier per day.
pub(super) fn merge_adjusted(bars: &mut [DailyBar], adjusted: &[AdjustedPrices]) {
    let by_date: HashMap<NaiveDate, &AdjustedPrices> = adjusted
        .iter()
        .map(|prices| (prices.date, prices))
        .collect();
    for bar in bars {
        if let Some(prices) = by_date.get(&bar.date) {
            bar.adj_close = Some(prices.close);
            bar.adj_open = prices.open.or(Some(bar.open * prices.close / bar.close));
        }
    }
}

/// Intraday bars that start inside the regular session (09:30 ≤ t < 16:00 New York) of a day in
/// `[from, to]`, converted to UTC, oldest first, one per instant.
pub(super) fn intraday_bars(
    value: Value,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<IntradayBar>, FetchError> {
    let session = regular_open()..regular_close();
    let mut bars = BTreeMap::new();
    for row in parse_rows::<BarRow>(value)? {
        let Some(local) = row.date.as_deref().and_then(parse_local_datetime) else {
            continue;
        };
        if !session.contains(&local.time()) || local.date() < from || local.date() > to {
            continue;
        }
        // Session hours never fall inside a DST transition, so the mapping is unambiguous.
        let Some(start) = MARKET_TZ.from_local_datetime(&local).single() else {
            continue;
        };
        let Some(Ohlcv {
            open,
            high,
            low,
            close,
            volume,
        }) = row.ohlcv()
        else {
            continue;
        };
        let ts = start.with_timezone(&Utc);
        bars.entry(ts).or_insert(IntradayBar {
            ts,
            open,
            high,
            low,
            close,
            volume,
        });
    }
    Ok(bars.into_values().collect())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SplitRow {
    /// Ex-date.
    date: Option<String>,
    numerator: Num,
    denominator: Num,
}

/// Splits oldest first; rows without a date or a positive ratio are skipped.
pub(super) fn splits(value: Value, symbol: &str) -> Result<Vec<Split>, FetchError> {
    let mut splits: Vec<Split> = parse_rows::<SplitRow>(value)?
        .into_iter()
        .filter_map(|row| {
            Some(Split {
                symbol: symbol.to_string(),
                date: parse_date(row.date.as_deref()?)?,
                numerator: row.numerator.positive()?,
                denominator: row.denominator.positive()?,
            })
        })
        .collect();
    splits.sort_by_key(|split| split.date);
    splits.dedup();
    Ok(splits)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct DividendRow {
    /// Ex-dividend date.
    date: Option<String>,
    /// Empty until announced.
    payment_date: Option<String>,
    /// Declared cash amount per share.
    dividend: Num,
    /// Split-adjusted amount; the fallback when `dividend` is missing.
    adj_dividend: Num,
}

/// Cash dividends oldest first; rows without an ex-date or a positive amount are skipped.
pub(super) fn dividends(value: Value, symbol: &str) -> Result<Vec<Dividend>, FetchError> {
    let mut dividends: Vec<Dividend> = parse_rows::<DividendRow>(value)?
        .into_iter()
        .filter_map(|row| {
            Some(Dividend {
                symbol: symbol.to_string(),
                ex_date: parse_date(row.date.as_deref()?)?,
                pay_date: row.payment_date.as_deref().and_then(parse_date),
                amount: row.dividend.positive().or(row.adj_dividend.positive())?,
            })
        })
        .collect();
    dividends.sort_by_key(|dividend| dividend.ex_date);
    dividends.dedup();
    Ok(dividends)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn close_to(actual: Option<f64>, expected: f64) -> bool {
        actual.is_some_and(|value| (value - expected).abs() < 1e-9)
    }

    #[test]
    fn numbers_are_parsed_leniently() {
        let parse = |value: Value| serde_json::from_value::<Num>(value).unwrap().value();
        assert_eq!(parse(json!(1.5)), Some(1.5));
        assert_eq!(parse(json!(42)), Some(42.0));
        assert_eq!(parse(json!(" 2.25 ")), Some(2.25));
        assert_eq!(parse(json!(null)), None);
        assert_eq!(parse(json!("n/a")), None);
        assert_eq!(parse(json!("NaN")), None);
        assert_eq!(parse(json!("inf")), None);
        assert_eq!(parse(json!([1])), None);
        assert_eq!(parse(json!({"v": 1})), None);
    }

    #[test]
    fn envelopes_of_both_generations() {
        assert_eq!(envelope_rows(json!([{"a": 1}])).unwrap().len(), 1);
        assert_eq!(
            envelope_rows(json!({"symbol": "AAPL", "historical": [{"a": 1}, {"a": 2}]}))
                .unwrap()
                .len(),
            2
        );
        assert!(envelope_rows(json!({})).unwrap().is_empty());
        assert!(envelope_rows(json!({"symbol": "NOPE"})).unwrap().is_empty());
        assert!(envelope_rows(json!(null)).unwrap().is_empty());
        assert!(matches!(
            envelope_rows(json!({"status": "maintenance"})),
            Err(FetchError::Parse(message)) if message.contains("status")
        ));
        assert!(matches!(
            envelope_rows(json!({"historical": "none"})),
            Err(FetchError::Parse(_))
        ));
        assert!(matches!(
            envelope_rows(json!("oops")),
            Err(FetchError::Parse(_))
        ));
    }

    #[test]
    fn quote_change_pct_is_a_fraction() {
        let quotes = quotes(json!([
            // Computed from prices: (102 - 100) / 100.
            {"symbol": "AAA", "price": 102.0, "previousClose": 100.0, "changePercentage": 1.99},
            // No previous close: stable percent / 100.
            {"symbol": "BBB", "price": 50.0, "changePercentage": -1.5},
            // No previous close: legacy percent / 100.
            {"symbol": "CCC", "price": 20.0, "previousClose": 0, "changesPercentage": 2.5},
            {"symbol": "DDD", "price": 10.0},
        ]))
        .unwrap();
        assert!(close_to(quotes[0].change_pct, 0.02));
        assert!(
            close_to(quotes[0].change, 2.0),
            "change derived from prices"
        );
        assert!(close_to(quotes[1].change_pct, -0.015));
        assert!(close_to(quotes[2].change_pct, 0.025));
        assert_eq!(quotes[2].prev_close, None, "zero is not a price");
        assert_eq!(quotes[3].change_pct, None);
        assert_eq!(quotes[3].change, None);
    }

    #[test]
    fn quote_rows_without_a_usable_price_or_symbol_are_skipped() {
        let quotes = quotes(json!([
            {"symbol": "NULL", "price": null},
            {"symbol": "ZERO", "price": 0},
            {"symbol": "NEG", "price": -1},
            {"symbol": "TEXT", "price": "12.5"},
            {"price": 10.0},
            {"symbol": "  ", "price": 10.0},
            "not an object",
            {"symbol": "OK", "price": 1.0},
        ]))
        .unwrap();
        let symbols: Vec<&str> = quotes.iter().map(|q| q.symbol.as_str()).collect();
        assert_eq!(symbols, vec!["TEXT", "OK"]);
        assert_eq!(quotes[0].price, 12.5);
    }

    #[test]
    fn quote_timestamps_accept_seconds_and_milliseconds() {
        let expected = DateTime::parse_from_rfc3339("2025-10-03T20:00:01Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(unix_time(1_759_521_601.0), Some(expected));
        assert_eq!(unix_time(1_759_521_601_000.0), Some(expected));
        assert_eq!(unix_time(0.0), None);
        assert_eq!(unix_time(-5.0), None);
    }

    #[test]
    fn daily_rows_are_clipped_sorted_deduplicated_and_filled() {
        let bars = daily_bars(
            json!({"symbol": "AAPL", "historical": [
                {"date": "2025-01-06", "open": 10.0, "high": 12.0, "low": 9.0, "close": 11.0, "adjClose": 10.0, "volume": 100},
                {"date": "2025-01-03", "open": 0, "high": null, "low": null, "close": 9.5, "volume": null},
                {"date": "2025-01-03", "open": 1.0, "high": 1.0, "low": 1.0, "close": 1.0, "volume": 1},
                {"date": "2025-01-02", "open": 9.0, "high": 9.0, "low": 9.0, "close": null},
                {"date": "2024-12-31", "open": 9.0, "high": 9.0, "low": 9.0, "close": 9.0},
                {"date": "", "close": 9.0},
            ]}),
            "AAPL",
            d(2025, 1, 1),
            d(2025, 1, 31),
        )
        .unwrap();
        let dates: Vec<NaiveDate> = bars.iter().map(|bar| bar.date).collect();
        assert_eq!(dates, vec![d(2025, 1, 3), d(2025, 1, 6)]);
        // First row for a date wins; missing open/high/low come from the close.
        let filled = &bars[0];
        assert_eq!(
            (
                filled.open,
                filled.high,
                filled.low,
                filled.close,
                filled.volume
            ),
            (9.5, 9.5, 9.5, 9.5, 0.0)
        );
        assert_eq!(filled.adj_close, None);
        // Legacy adjClose scales the open by the same factor.
        assert_eq!(bars[1].adj_close, Some(10.0));
        assert!(close_to(bars[1].adj_open, 10.0 * 10.0 / 11.0));
    }

    #[test]
    fn adjusted_prices_merge_by_date() {
        let mut bars = daily_bars(
            json!([
                {"date": "2025-01-03", "open": 20.0, "high": 21.0, "low": 19.0, "close": 20.0, "volume": 1},
                {"date": "2025-01-02", "open": 10.0, "high": 11.0, "low": 9.0, "close": 10.0, "volume": 1},
                {"date": "2025-01-06", "open": 30.0, "high": 31.0, "low": 29.0, "close": 30.0, "volume": 1},
            ]),
            "AAPL",
            d(2025, 1, 1),
            d(2025, 1, 31),
        )
        .unwrap();
        let adjusted = adjusted_prices(json!([
            {"date": "2025-01-02", "adjOpen": 9.8, "adjClose": 9.9},
            // Legacy-style row: no adjusted open.
            {"date": "2025-01-03", "adjClose": 19.0},
            {"date": "2025-01-07", "adjOpen": 1.0, "adjClose": 1.0},
            {"date": "2025-01-08", "adjClose": null},
        ]))
        .unwrap();
        assert_eq!(adjusted.len(), 3);
        merge_adjusted(&mut bars, &adjusted);
        assert_eq!(
            (bars[0].adj_open, bars[0].adj_close),
            (Some(9.8), Some(9.9))
        );
        assert!(close_to(bars[1].adj_open, 20.0 * 19.0 / 20.0));
        assert_eq!(bars[1].adj_close, Some(19.0));
        assert_eq!((bars[2].adj_open, bars[2].adj_close), (None, None));
    }

    #[test]
    fn intraday_rows_keep_the_regular_session_in_utc() {
        let bars = intraday_bars(
            json!([
                {"date": "2026-03-09 16:00:00", "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1},
                {"date": "2026-03-09 15:59", "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1},
                {"date": "2026-03-09 09:30:00", "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1},
                {"date": "2026-03-09 09:29:59", "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1},
                {"date": "2026-03-06 09:30:00", "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1},
                {"date": "2026-03-05 10:00:00", "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1},
                {"date": "garbage", "close": 1},
            ]),
            d(2026, 3, 6),
            d(2026, 3, 9),
        )
        .unwrap();
        let stamps: Vec<String> = bars.iter().map(|bar| bar.ts.to_rfc3339()).collect();
        // US DST began on Sunday 2026-03-08: EST (UTC-5) on the 6th, EDT (UTC-4) on the 9th.
        assert_eq!(
            stamps,
            vec![
                "2026-03-06T14:30:00+00:00",
                "2026-03-09T13:30:00+00:00",
                "2026-03-09T19:59:00+00:00",
            ]
        );
    }

    #[test]
    fn splits_and_dividends_are_sorted_and_deduplicated() {
        let splits = splits(
            json!([
                {"date": "2020-08-31", "numerator": 4, "denominator": 1},
                {"date": "2014-06-09", "numerator": 7, "denominator": 1},
                {"date": "2020-08-31", "numerator": 4, "denominator": 1},
                {"date": "2010-01-01", "numerator": 0, "denominator": 1},
            ]),
            "AAPL",
        )
        .unwrap();
        let ratios: Vec<(NaiveDate, f64)> = splits.iter().map(|s| (s.date, s.ratio())).collect();
        assert_eq!(ratios, vec![(d(2014, 6, 9), 7.0), (d(2020, 8, 31), 4.0)]);

        let dividends = dividends(
            json!([
                {"date": "2025-08-11", "paymentDate": "2025-08-14", "dividend": 0.26, "adjDividend": 0.26},
                {"date": "1995-11-21", "paymentDate": "", "dividend": null, "adjDividend": 0.00107},
                {"date": "2025-08-11", "paymentDate": "2025-08-14", "dividend": 0.26, "adjDividend": 0.26},
                {"date": "2025-05-12", "paymentDate": null, "dividend": 0, "adjDividend": 0},
            ]),
            "AAPL",
        )
        .unwrap();
        assert_eq!(dividends.len(), 2);
        assert_eq!(dividends[0].ex_date, d(1995, 11, 21));
        assert_eq!(dividends[0].amount, 0.00107);
        assert_eq!(dividends[0].pay_date, None);
        assert_eq!(dividends[1].pay_date, Some(d(2025, 8, 14)));
    }
}
