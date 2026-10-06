//! Market data sources.
//!
//! Production uses [`fmp::FmpClient`] (Financial Modeling Prep — the provider honeclaw already
//! integrates). [`demo::DemoMarket`] is a deterministic synthetic source for evaluation, local
//! development and tests; the server always runs it against a separate database schema so demo
//! prices and trades can never mix with real ones.

pub mod demo;
pub mod fmp;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSource {
    Fmp,
    Demo,
}

impl DataSource {
    pub fn as_str(self) -> &'static str {
        match self {
            DataSource::Fmp => "fmp",
            DataSource::Demo => "demo",
        }
    }
}

/// A real-time (or delayed) quote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    pub symbol: String,
    pub price: f64,
    pub change: Option<f64>,
    /// Percent change versus the previous close, as a fraction (0.0123 = +1.23%).
    pub change_pct: Option<f64>,
    pub open: Option<f64>,
    pub day_high: Option<f64>,
    pub day_low: Option<f64>,
    pub prev_close: Option<f64>,
    pub volume: Option<f64>,
    pub avg_volume: Option<f64>,
    pub market_cap: Option<f64>,
    /// When the provider says the price was printed.
    pub timestamp: Option<DateTime<Utc>>,
}

/// One daily bar. `open..close` are split-adjusted display prices; `adj_open`/`adj_close` are
/// split- and dividend-adjusted (total return) prices used for signals and backtests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyBar {
    pub symbol: String,
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub adj_open: Option<f64>,
    pub adj_close: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Interval {
    #[serde(rename = "5min")]
    FiveMin,
    #[serde(rename = "15min")]
    FifteenMin,
    #[serde(rename = "30min")]
    ThirtyMin,
    #[serde(rename = "1hour")]
    OneHour,
}

impl Interval {
    pub fn as_str(self) -> &'static str {
        match self {
            Interval::FiveMin => "5min",
            Interval::FifteenMin => "15min",
            Interval::ThirtyMin => "30min",
            Interval::OneHour => "1hour",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "5min" => Some(Interval::FiveMin),
            "15min" => Some(Interval::FifteenMin),
            "30min" => Some(Interval::ThirtyMin),
            "1hour" => Some(Interval::OneHour),
            _ => None,
        }
    }

    pub fn minutes(self) -> i64 {
        match self {
            Interval::FiveMin => 5,
            Interval::FifteenMin => 15,
            Interval::ThirtyMin => 30,
            Interval::OneHour => 60,
        }
    }
}

/// One intraday bar; `ts` is the bar's start instant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntradayBar {
    pub ts: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Split {
    pub symbol: String,
    /// Ex-date.
    pub date: NaiveDate,
    pub numerator: f64,
    pub denominator: f64,
}

impl Split {
    /// Shares after the split per share before it (e.g. 10.0 for a 10-for-1 split).
    pub fn ratio(&self) -> f64 {
        if self.denominator > 0.0 {
            self.numerator / self.denominator
        } else {
            1.0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dividend {
    pub symbol: String,
    pub ex_date: NaiveDate,
    pub pay_date: Option<NaiveDate>,
    /// Cash amount per share.
    pub amount: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum MarketError {
    #[error("market data is not configured: {0}")]
    NotConfigured(String),
    /// Every configured key was rejected (invalid, exhausted or rate limited).
    #[error("market data keys rejected: {0}")]
    Auth(String),
    /// The subscription plan does not include this endpoint or symbol (HTTP 402 and similar).
    #[error("not included in the market data plan: {0}")]
    PlanRestricted(String),
    #[error("market data request failed: {0}")]
    Http(String),
    #[error("unexpected market data response: {0}")]
    Parse(String),
}

#[async_trait]
pub trait MarketData: Send + Sync {
    fn source(&self) -> DataSource;

    /// Latest quotes. Symbols the provider does not return are simply absent from the result.
    async fn quotes(&self, symbols: &[String]) -> Result<Vec<Quote>, MarketError>;

    /// Daily bars in `[from, to]`, oldest first.
    async fn daily_bars(
        &self,
        symbol: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<DailyBar>, MarketError>;

    /// Intraday bars for the regular session(s) in `[from, to]`, oldest first.
    async fn intraday_bars(
        &self,
        symbol: &str,
        interval: Interval,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<IntradayBar>, MarketError>;

    /// Historical splits, oldest first.
    async fn splits(&self, symbol: &str) -> Result<Vec<Split>, MarketError>;

    /// Historical cash dividends, oldest first.
    async fn dividends(&self, symbol: &str) -> Result<Vec<Dividend>, MarketError>;
}
