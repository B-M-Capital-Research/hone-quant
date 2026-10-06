//! Deterministic synthetic US equity market for evaluation (when no FMP key is configured), local
//! development, screenshots and tests.
//!
//! **Everything here is synthetic.** The prices are shaped to read like a 2015–2026 technology
//! market, but they are not market data and must never be presented as real: the server keeps
//! demo data in its own schema and labels it as demo wherever it is shown.
//!
//! # Determinism
//! Every value is a pure function of `(seed, symbol, sector, date)`, plus the clock time for the
//! session in progress. Random numbers come from hashing those keys (SplitMix64 and Box–Muller),
//! never from shared generator state. Separate instances, overlapping query ranges and different
//! query orders therefore agree bit for bit. The clock only decides how much of this fixed tape is
//! visible.
//!
//! # Model
//! Sessions follow the NYSE calendar from [`quant_core::calendar`] (weekends, holidays, 13:00 early
//! closes). History starts on 2015-01-02, or at a late listing's approximate first session (see
//! [`listing_date`]). A symbol's daily simple return is
//!
//! ```text
//! r = μ + β·M + Σ γ·S_sector + σ·ε + J        clamped to ±35%
//! ```
//!
//! - `M`, the market factor: about 1.0% daily volatility, a base drift and deterministic regimes.
//!   The regimes are a 2015 and an early-2016 sell-off, a calm 2017, a 2018 Q4 drawdown, a
//!   2020-style crash and recovery, a 2022-style bear market and a spring-2025 shock. Shocks are
//!   centred within each regime window, so an episode has the same size for every seed. Between
//!   regimes the accumulated noise mean-reverts slowly, so long-run levels follow the regimes too.
//! - `S`, a factor per sector (about 0.9%) shared by the sector's members, with a few sector themes
//!   such as an AI-chip boom.
//! - `ε`, idiosyncratic noise (1.4–2.2% by sector). `J` is an "earnings gap" of ±4–9% on about one
//!   session in 60.
//! - Benchmarks (sector `"benchmark"`) have no earnings gaps. SPY tracks the market factor, QQQ has
//!   β ≈ 1.15 plus some hyperscaler exposure, and SMH has β ≈ 1.4 plus AI-chip exposure.
//!
//! Sector and idiosyncratic volatility rise with the market regime's volatility. Price levels
//! (15–500 on [`model::PRICE_ANCHOR`], with the history scaled to match), share counts and volume
//! levels are hashed per symbol.
//!
//! # Bars
//! The close compounds the returns. The open gaps from the previous close by a share of the day's
//! return plus noise; an earnings gap lands overnight in full. Within the session the price follows
//! a Brownian bridge from the open to the close over 5-minute steps (78 in a full session, 42 on an
//! early close), with a U-shaped variance profile and a small wick on every step. The steps use a
//! cheap four-uniform approximation of a normal; daily draws use Box–Muller.
//!
//! The daily high and low are the extremes of those 5-minute bars, so daily and intraday data agree
//! exactly. Volume is a symbol level scaled up on big moves and spread over the session with a
//! U-shaped profile. Prices are rounded to cents. There are no splits or dividends, so adjusted
//! prices equal raw ones.
//!
//! # Clock
//! `daily_bars` returns the sessions that have closed at the clock's `now`, plus today's partial
//! bar while a session is in progress. `intraday_bars` stops at `now`. During a session, quotes read
//! the path at `now`; otherwise they show the last close. Nothing after `now` is ever returned.
//!
//! # Caching
//! The market-factor tape and each symbol's sessions are generated once and extended lazily as the
//! clock advances. Intraday paths are rebuilt on demand from the cached sessions.

mod model;
mod rng;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use quant_core::calendar::{MarketCalendar, MarketPhase, Session};

use self::model::{Day, History, Model, PRICE_ANCHOR, Profile, Progress, STEP_MINUTES, Tape};
use super::{
    DailyBar, DataSource, Dividend, Interval, IntradayBar, MarketData, MarketError, Quote, Split,
};

pub use self::model::{HISTORY_START, listing_date};

/// Sessions averaged into a quote's `avg_volume`.
const AVG_VOLUME_SESSIONS: usize = 20;

/// An instrument the demo market knows about.
#[derive(Debug, Clone)]
pub struct DemoInstrument {
    pub symbol: String,
    /// Sector id for correlated sector shocks; benchmarks use `"benchmark"`.
    pub sector: String,
}

/// Source of "now", injected so tests and screenshots can freeze time.
pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// The wall clock.
pub fn system_clock() -> Clock {
    Arc::new(Utc::now)
}

/// A clock frozen at `at`.
pub fn fixed_clock(at: DateTime<Utc>) -> Clock {
    Arc::new(move || at)
}

/// The deterministic synthetic market described in the module docs.
pub struct DemoMarket {
    instruments: Vec<DemoInstrument>,
    profiles: HashMap<String, Profile>,
    model: Model,
    clock: Clock,
    /// Market-factor tape shared by every symbol.
    tape: Mutex<Arc<Tape>>,
    /// Generated sessions per symbol.
    histories: Mutex<HashMap<String, Arc<History>>>,
}

impl DemoMarket {
    /// A market over `instruments`. Symbols match case-insensitively and the first of duplicated
    /// symbols wins. `seed` selects the synthetic world and `clock` decides "now".
    pub fn new(instruments: Vec<DemoInstrument>, seed: u64, clock: Clock) -> Self {
        let mut model = Model::new(seed);
        let mut profiles = HashMap::new();
        for instrument in &instruments {
            let symbol = normalize(&instrument.symbol);
            if symbol.is_empty() || profiles.contains_key(&symbol) {
                continue;
            }
            let sector = instrument.sector.trim().to_ascii_lowercase();
            let profile = Profile::new(&mut model, &symbol, &sector);
            profiles.insert(symbol, profile);
        }
        // Anchor price levels: scale each starting price so the close on PRICE_ANCHOR equals the
        // symbol's reference level (sessions are linear in the starting price).
        let tape = Tape::new().extended(&model, PRICE_ANCHOR);
        for profile in profiles.values_mut() {
            let history = History::new().extended(profile, &tape, PRICE_ANCHOR);
            if let Some(last) = history.days.last().filter(|d| d.close > 0.0) {
                profile.reference *= profile.reference / last.close;
            }
        }
        Self {
            instruments,
            profiles,
            model,
            clock,
            tape: Mutex::new(Arc::new(Tape::new())),
            histories: Mutex::new(HashMap::new()),
        }
    }

    pub fn instruments(&self) -> &[DemoInstrument] {
        &self.instruments
    }

    fn profile(&self, symbol: &str) -> Option<&Profile> {
        self.profiles.get(&normalize(symbol))
    }

    fn view(&self, now: DateTime<Utc>) -> View {
        let calendar = &self.model.calendar;
        let status = calendar.status_at(now);
        let today = MarketCalendar::local_date(now);
        let (closed, live) = match status.phase {
            MarketPhase::Open => (calendar.prev_trading_day(today), status.today),
            MarketPhase::PostClose => (today, None),
            MarketPhase::PreOpen | MarketPhase::Closed => (calendar.prev_trading_day(today), None),
        };
        View {
            now,
            closed: Some(closed).filter(|date| *date >= HISTORY_START),
            live: live.filter(|session| session.date >= HISTORY_START),
        }
    }

    /// The market tape through `through`, generated on first use. Generation runs outside the
    /// lock; concurrent callers produce identical tapes.
    fn tape(&self, through: NaiveDate) -> Arc<Tape> {
        let current = lock(&self.tape).clone();
        if current.through >= through {
            return current;
        }
        let extended = Arc::new(current.extended(&self.model, through));
        let mut slot = lock(&self.tape);
        if slot.through < extended.through {
            *slot = extended.clone();
        }
        extended
    }

    /// `profile`'s sessions through `through`, extending the cached history when needed.
    fn history(&self, profile: &Profile, through: NaiveDate) -> Arc<History> {
        let cached = lock(&self.histories).get(&profile.symbol).cloned();
        if let Some(history) = cached.as_ref().filter(|h| h.through >= through) {
            return history.clone();
        }
        let tape = self.tape(through);
        let base = cached.unwrap_or_else(|| Arc::new(History::new()));
        let extended = Arc::new(base.extended(profile, &tape, through));
        let mut histories = lock(&self.histories);
        let slot = histories
            .entry(profile.symbol.clone())
            .or_insert_with(|| extended.clone());
        if slot.through < extended.through {
            *slot = extended.clone();
        }
        extended
    }

    fn quotes_at(&self, symbols: &[String], now: DateTime<Utc>) -> Vec<Quote> {
        let view = self.view(now);
        let Some(horizon) = view.horizon() else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        symbols
            .iter()
            .filter_map(|symbol| self.profile(symbol))
            .filter(|profile| seen.insert(profile.symbol.as_str()))
            .filter_map(|profile| self.quote(profile, &view, horizon))
            .collect()
    }

    /// `None` until the symbol has traded.
    fn quote(&self, profile: &Profile, view: &View, horizon: NaiveDate) -> Option<Quote> {
        let history = self.history(profile, horizon);
        let closed = view
            .closed
            .map_or(&[][..], |date| history.range(HISTORY_START, date));
        let recent = &closed[closed.len().saturating_sub(AVG_VOLUME_SESSIONS)..];
        let avg_volume = (!recent.is_empty())
            .then(|| (recent.iter().map(|d| d.volume).sum::<f64>() / recent.len() as f64).round());
        let live = view
            .live
            .and_then(|session| Some((session, history.get(session.date)?)));
        if let Some((session, day)) = live {
            let snapshot = Snapshot::live(profile, day, &session, view.now);
            return Some(snapshot.quote(&profile.symbol, avg_volume, profile.shares, view.now));
        }
        let last = closed.last()?;
        let close_time = self.model.calendar.session(last.date)?.close;
        Some(Snapshot::closed(last).quote(&profile.symbol, avg_volume, profile.shares, close_time))
    }

    fn daily_bars_at(
        &self,
        symbol: &str,
        from: NaiveDate,
        to: NaiveDate,
        now: DateTime<Utc>,
    ) -> Vec<DailyBar> {
        let Some(profile) = self.profile(symbol) else {
            return Vec::new();
        };
        let view = self.view(now);
        let Some(to) = view.horizon().map(|h| h.min(to)).filter(|to| *to >= from) else {
            return Vec::new();
        };
        let history = self.history(profile, to);
        history
            .range(from, to)
            .iter()
            .map(|day| {
                let snapshot = match view.live {
                    Some(session) if session.date == day.date => {
                        Snapshot::live(profile, day, &session, view.now)
                    }
                    _ => Snapshot::closed(day),
                };
                snapshot.daily_bar(&profile.symbol, day.date)
            })
            .collect()
    }

    fn intraday_bars_at(
        &self,
        symbol: &str,
        interval: Interval,
        from: NaiveDate,
        to: NaiveDate,
        now: DateTime<Utc>,
    ) -> Vec<IntradayBar> {
        let Some(profile) = self.profile(symbol) else {
            return Vec::new();
        };
        let view = self.view(now);
        let Some(to) = view.horizon().map(|h| h.min(to)).filter(|to| *to >= from) else {
            return Vec::new();
        };
        let history = self.history(profile, to);
        let mut bars = Vec::new();
        for day in history.range(from, to) {
            let Some(session) = self.model.calendar.session(day.date) else {
                continue;
            };
            let progress = match view.live {
                Some(live) if live.date == day.date => Progress::at(&session, view.now),
                _ => Progress::complete(model::steps(&session)),
            };
            let path = profile.path(day, &session);
            let curve = profile.volume_curve(day, &session);
            let five_minute = path.bars(progress).map(|(k, fraction)| {
                let bar = path.bar(k, fraction);
                IntradayBar {
                    ts: session.open + Duration::minutes(STEP_MINUTES * k as i64),
                    open: bar.open,
                    high: bar.high,
                    low: bar.low,
                    close: bar.close,
                    volume: curve.bar(day.volume, k, fraction),
                }
            });
            aggregate_into(&mut bars, five_minute, session.open, interval.minutes());
        }
        // Rounding is monotonic, so rounding after aggregation equals aggregating rounded bars.
        for bar in &mut bars {
            bar.open = round_price(bar.open);
            bar.high = round_price(bar.high);
            bar.low = round_price(bar.low);
            bar.close = round_price(bar.close);
        }
        bars
    }
}

#[async_trait]
impl MarketData for DemoMarket {
    fn source(&self) -> DataSource {
        DataSource::Demo
    }

    async fn quotes(&self, symbols: &[String]) -> Result<Vec<Quote>, MarketError> {
        Ok(self.quotes_at(symbols, (self.clock)()))
    }

    async fn daily_bars(
        &self,
        symbol: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<DailyBar>, MarketError> {
        Ok(self.daily_bars_at(symbol, from, to, (self.clock)()))
    }

    async fn intraday_bars(
        &self,
        symbol: &str,
        interval: Interval,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<IntradayBar>, MarketError> {
        Ok(self.intraday_bars_at(symbol, interval, from, to, (self.clock)()))
    }

    async fn splits(&self, _symbol: &str) -> Result<Vec<Split>, MarketError> {
        Ok(Vec::new())
    }

    async fn dividends(&self, _symbol: &str) -> Result<Vec<Dividend>, MarketError> {
        Ok(Vec::new())
    }
}

/// What the clock lets callers see.
#[derive(Debug, Clone, Copy)]
struct View {
    now: DateTime<Utc>,
    /// The last session that has closed by `now`.
    closed: Option<NaiveDate>,
    /// The session in progress at `now`.
    live: Option<Session>,
}

impl View {
    /// The last session with anything visible.
    fn horizon(&self) -> Option<NaiveDate> {
        self.live.map(|session| session.date).or(self.closed)
    }
}

/// One session as seen at some instant, in internal (unrounded) prices.
#[derive(Debug, Clone, Copy)]
struct Snapshot {
    open: f64,
    high: f64,
    low: f64,
    last: f64,
    prev_close: f64,
    volume: f64,
}

impl Snapshot {
    fn closed(day: &Day) -> Self {
        Self {
            open: day.open,
            high: day.high,
            low: day.low,
            last: day.close,
            prev_close: day.prev_close,
            volume: day.volume,
        }
    }

    /// The part of `day` traded by `now`.
    fn live(profile: &Profile, day: &Day, session: &Session, now: DateTime<Utc>) -> Self {
        let progress = Progress::at(session, now);
        let path = profile.path(day, session);
        let (high, low) = path.range(progress);
        Self {
            open: day.open,
            high,
            low,
            last: path.price_at(progress),
            prev_close: day.prev_close,
            volume: profile
                .volume_curve(day, session)
                .traded(day.volume, progress),
        }
    }

    fn daily_bar(&self, symbol: &str, date: NaiveDate) -> DailyBar {
        let (open, close) = (round_price(self.open), round_price(self.last));
        DailyBar {
            symbol: symbol.to_string(),
            date,
            open,
            high: round_price(self.high),
            low: round_price(self.low),
            close,
            volume: self.volume,
            adj_open: Some(open),
            adj_close: Some(close),
        }
    }

    fn quote(
        &self,
        symbol: &str,
        avg_volume: Option<f64>,
        shares: f64,
        timestamp: DateTime<Utc>,
    ) -> Quote {
        let price = round_price(self.last);
        let prev_close = round_price(self.prev_close);
        let change = ((price - prev_close) * 1e4).round() / 1e4;
        Quote {
            symbol: symbol.to_string(),
            price,
            change: Some(change),
            change_pct: Some(change / prev_close),
            open: Some(round_price(self.open)),
            day_high: Some(round_price(self.high)),
            day_low: Some(round_price(self.low)),
            prev_close: Some(prev_close),
            volume: Some(self.volume),
            avg_volume,
            market_cap: Some((price * shares).round()),
            timestamp: Some(timestamp),
        }
    }
}

/// Appends one session's 5-minute bars to `out` as `minutes`-long bars aligned to the open.
fn aggregate_into(
    out: &mut Vec<IntradayBar>,
    bars: impl Iterator<Item = IntradayBar>,
    open: DateTime<Utc>,
    minutes: i64,
) {
    for bar in bars {
        let ts = open + Duration::minutes((bar.ts - open).num_minutes() / minutes * minutes);
        match out.last_mut() {
            Some(last) if last.ts == ts => {
                last.high = last.high.max(bar.high);
                last.low = last.low.min(bar.low);
                last.close = bar.close;
                last.volume += bar.volume;
            }
            _ => out.push(IntradayBar { ts, ..bar }),
        }
    }
}

/// Rounds like an exchange print: to the cent, or to 1/100 cent below $1. Monotonic, so OHLC
/// ordering survives.
fn round_price(price: f64) -> f64 {
    let scale = if price >= 1.0 { 100.0 } else { 10_000.0 };
    ((price * scale).round() / scale).max(0.0001)
}

fn normalize(symbol: &str) -> String {
    symbol.trim().to_ascii_uppercase()
}

/// The caches only ever hold complete values, so a poisoned lock is still consistent.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
