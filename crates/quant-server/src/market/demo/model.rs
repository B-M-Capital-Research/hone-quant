//! Parameters and generators of the synthetic market: sector and benchmark profiles, regimes,
//! listing dates, the shared market-factor tape, per-symbol daily sessions and the 5-minute
//! intraday path every bar is cut from. All prices here are internal (unrounded).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use quant_core::calendar::{MarketCalendar, Session};

use super::rng::{Rng, hash_str, key, log_uniform};

/// First synthetic session; nothing exists before it.
pub const HISTORY_START: NaiveDate = ymd(2015, 1, 2);

/// Each symbol's hashed reference price is its close on this date; the history is scaled to match.
/// Returns do not depend on the price level, so anchoring changes no dynamics, but it keeps a
/// decade of synthetic compounding from producing five-figure share prices that whole-share
/// rebalancing cannot hold.
pub const PRICE_ANCHOR: NaiveDate = ymd(2026, 6, 30);
/// Length of one intraday step, in minutes.
pub const STEP_MINUTES: i64 = 5;
/// Steps in a full 09:30–16:00 session (a 13:00 early close has 42).
pub const MAX_STEPS: usize = 78;
/// Sector id of index ETFs.
pub const BENCHMARK: &str = "benchmark";

const TRADING_DAYS: f64 = 252.0;
/// Close-to-close returns are clamped to ±35%.
const MAX_DAILY_MOVE: f64 = 0.35;
/// Overnight gaps are clamped to ±30%.
const MAX_GAP: f64 = 0.30;

/// Daily volatility of the market factor.
const MARKET_VOL: f64 = 0.010;
/// Annualised log drift of the market factor outside the regimes.
const MARKET_BASE_DRIFT: f64 = 0.19;
/// Daily pull, between regimes, of the accumulated market noise back to the noise-free path
/// (half-life ≈ 87 sessions), so long-run levels follow the regimes whatever the seed.
const MARKET_REVERSION: f64 = 0.008;

/// Probability of an "earnings gap" on any session, and its size range.
const JUMP_PROB: f64 = 1.0 / 60.0;
const JUMP_MIN: f64 = 0.04;
const JUMP_SPREAD: f64 = 0.05;

/// Share of the day's (non-jump) return realised in the overnight gap.
const GAP_SHARE: f64 = 0.3;
/// Gap noise, in units of the symbol's typical daily volatility.
const GAP_NOISE: f64 = 0.25;
/// Intraday diffusion as a share of the session's daily volatility.
const INTRADAY_SHARE: f64 = 0.8;
/// Maximum wick of a 5-minute bar, in units of the step volatility.
const WICK: f64 = 0.6;

const STREAM_MARKET: u64 = 1;
const STREAM_SECTOR: u64 = 2;
const STREAM_SYMBOL: u64 = 3;
const STREAM_DAY: u64 = 4;
const STREAM_PATH: u64 = 5;
const STREAM_VOLUME: u64 = 6;

const fn ymd(year: i32, month: u32, day: u32) -> NaiveDate {
    match NaiveDate::from_ymd_opt(year, month, day) {
        Some(date) => date,
        None => panic!("invalid date"),
    }
}

/// Approximate first sessions of late listings (IPOs, spin-offs, direct and re-listings) in the
/// hone-quant universe. They are synthetic approximations that exist so late-listing handling is
/// exercised end to end; every other symbol starts on [`HISTORY_START`].
const LISTINGS: &[(&str, NaiveDate)] = &[
    ("BWXT", ymd(2015, 7, 1)),
    ("LITE", ymd(2015, 8, 4)),
    ("HPE", ymd(2015, 11, 2)),
    ("VST", ymd(2016, 10, 5)),
    ("BE", ymd(2018, 7, 25)),
    ("DELL", ymd(2018, 12, 28)),
    ("ONTO", ymd(2019, 10, 28)),
    ("SNOW", ymd(2020, 9, 16)),
    ("PLTR", ymd(2020, 9, 30)),
    ("HIMS", ymd(2021, 1, 21)),
    ("RKLB", ymd(2021, 8, 25)),
    ("IREN", ymd(2021, 11, 17)),
    ("CRDO", ymd(2022, 1, 27)),
    ("CEG", ymd(2022, 2, 2)),
    ("TLN", ymd(2023, 7, 12)),
    ("ARM", ymd(2023, 9, 14)),
    ("ALAB", ymd(2024, 3, 20)),
    ("GEV", ymd(2024, 4, 2)),
    ("OKLO", ymd(2024, 5, 10)),
    ("TEM", ymd(2024, 6, 14)),
    ("NBIS", ymd(2024, 10, 21)),
    ("SNDK", ymd(2025, 2, 24)),
    ("CRWV", ymd(2025, 3, 28)),
];

/// First date a symbol can trade in the demo market (approximate and synthetic).
pub fn listing_date(symbol: &str) -> NaiveDate {
    LISTINGS
        .iter()
        .find(|(s, _)| *s == symbol)
        .map_or(HISTORY_START, |(_, date)| (*date).max(HISTORY_START))
}

/// Factor loadings and risk of a sector's typical member; volatilities are daily.
struct SectorSpec {
    id: &'static str,
    beta: f64,
    /// Volatility of the factor shared by the sector.
    factor_vol: f64,
    idio_vol: f64,
    /// Annualised drift on top of the market.
    alpha: f64,
    /// Market capitalisation range at the first session, USD billions.
    cap_bn: (f64, f64),
}

const fn sector(
    id: &'static str,
    beta: f64,
    factor_vol: f64,
    idio_vol: f64,
    alpha: f64,
    cap_bn: (f64, f64),
) -> SectorSpec {
    SectorSpec {
        id,
        beta,
        factor_vol,
        idio_vol,
        alpha,
        cap_bn,
    }
}

const SECTORS: &[SectorSpec] = &[
    sector("ai-chip", 1.45, 0.010, 0.018, 0.06, (8.0, 300.0)),
    sector("storage", 1.30, 0.010, 0.019, 0.04, (4.0, 80.0)),
    sector("optical", 1.35, 0.010, 0.020, 0.05, (2.0, 40.0)),
    sector("power", 0.85, 0.009, 0.016, 0.05, (6.0, 80.0)),
    sector("neocloud", 1.60, 0.011, 0.022, 0.06, (2.0, 40.0)),
    sector("equipment", 1.30, 0.008, 0.016, 0.05, (10.0, 150.0)),
    sector("server-oem", 1.15, 0.009, 0.018, 0.03, (5.0, 80.0)),
    sector("hyperscaler", 1.10, 0.007, 0.014, 0.05, (200.0, 900.0)),
    sector("space", 1.50, 0.010, 0.022, 0.06, (1.0, 20.0)),
    sector("ai-apps", 1.30, 0.009, 0.020, 0.05, (3.0, 100.0)),
];

/// Used for sector ids the table does not know.
const OTHER_SECTOR: SectorSpec = sector("other", 1.20, 0.009, 0.018, 0.03, (2.0, 60.0));

fn sector_spec(id: &str) -> &'static SectorSpec {
    SECTORS.iter().find(|s| s.id == id).unwrap_or(&OTHER_SECTOR)
}

/// Index ETFs: exact loadings, low idiosyncratic noise, no earnings gaps.
struct BenchmarkSpec {
    symbol: &'static str,
    beta: f64,
    /// Loadings on sector factors.
    exposures: &'static [(&'static str, f64)],
    idio_vol: f64,
    alpha: f64,
    /// Typical daily share volume.
    volume: f64,
    /// Net assets at the first session, USD billions.
    aum_bn: f64,
}

const BENCHMARKS: &[BenchmarkSpec] = &[
    BenchmarkSpec {
        symbol: "SPY",
        beta: 1.0,
        exposures: &[],
        idio_vol: 0.001,
        alpha: 0.0,
        volume: 70e6,
        aum_bn: 180.0,
    },
    BenchmarkSpec {
        symbol: "QQQ",
        beta: 1.15,
        exposures: &[("hyperscaler", 0.3)],
        idio_vol: 0.003,
        alpha: 0.01,
        volume: 40e6,
        aum_bn: 40.0,
    },
    BenchmarkSpec {
        symbol: "SMH",
        beta: 1.4,
        exposures: &[("ai-chip", 0.6)],
        idio_vol: 0.004,
        alpha: 0.01,
        volume: 6e6,
        aum_bn: 0.5,
    },
];

const OTHER_BENCHMARK: BenchmarkSpec = BenchmarkSpec {
    symbol: "",
    beta: 1.0,
    exposures: &[],
    idio_vol: 0.003,
    alpha: 0.0,
    volume: 10e6,
    aum_bn: 10.0,
};

/// Scope of the market-factor regimes; sector themes use the sector id.
const MARKET: &str = "market";

/// A deterministic episode of the market factor or of one sector factor: the noise-free price
/// move over the window and a volatility multiplier.
struct RegimeSpec {
    scope: &'static str,
    from: NaiveDate,
    to: NaiveDate,
    total: f64,
    vol: f64,
}

const fn regime(
    scope: &'static str,
    from: NaiveDate,
    to: NaiveDate,
    total: f64,
    vol: f64,
) -> RegimeSpec {
    RegimeSpec {
        scope,
        from,
        to,
        total,
        vol,
    }
}

/// Episodes loosely shaped after familiar ones so charts read like a 2015–2026 tech market.
/// Dates and sizes are approximate and synthetic.
const REGIMES: &[RegimeSpec] = &[
    regime(MARKET, ymd(2015, 8, 18), ymd(2015, 8, 25), -0.11, 2.2),
    regime(MARKET, ymd(2015, 12, 30), ymd(2016, 2, 11), -0.12, 1.5),
    regime(MARKET, ymd(2017, 1, 3), ymd(2017, 12, 29), 0.19, 0.55),
    regime(MARKET, ymd(2018, 1, 29), ymd(2018, 2, 8), -0.10, 2.5),
    regime(MARKET, ymd(2018, 10, 1), ymd(2018, 12, 24), -0.19, 1.6),
    regime(MARKET, ymd(2018, 12, 26), ymd(2019, 4, 30), 0.24, 1.1),
    // 2020-style crash and V-shaped recovery.
    regime(MARKET, ymd(2020, 2, 20), ymd(2020, 3, 23), -0.34, 3.5),
    regime(MARKET, ymd(2020, 3, 24), ymd(2020, 8, 31), 0.52, 1.8),
    // 2022-style bear market.
    regime(MARKET, ymd(2022, 1, 4), ymd(2022, 10, 12), -0.25, 1.5),
    regime(MARKET, ymd(2025, 2, 19), ymd(2025, 4, 8), -0.19, 1.8),
    regime(MARKET, ymd(2025, 4, 9), ymd(2025, 7, 31), 0.27, 1.2),
    // Sector themes, on top of the market.
    regime("ai-chip", ymd(2023, 1, 3), ymd(2024, 6, 18), 0.80, 1.2),
    regime("hyperscaler", ymd(2023, 1, 3), ymd(2023, 12, 29), 0.30, 1.0),
    regime("power", ymd(2024, 1, 2), ymd(2024, 12, 31), 0.50, 1.2),
    regime("space", ymd(2024, 11, 4), ymd(2025, 2, 14), 0.40, 1.5),
    regime("neocloud", ymd(2025, 4, 9), ymd(2025, 10, 31), 0.60, 1.4),
    regime("optical", ymd(2025, 5, 1), ymd(2025, 12, 31), 0.45, 1.2),
    regime("storage", ymd(2025, 7, 1), ymd(2026, 1, 30), 0.55, 1.3),
];

fn day_number(date: NaiveDate) -> u64 {
    date.num_days_from_ce() as u64
}

/// Intraday steps of a session (sessions last a whole number of steps).
pub fn steps(session: &Session) -> usize {
    ((session.minutes() / STEP_MINUTES) as usize).clamp(1, MAX_STEPS)
}

#[derive(Debug, Clone, Copy)]
struct Regime {
    from: NaiveDate,
    to: NaiveDate,
    /// Daily simple drift; includes the volatility-drag correction.
    drift: f64,
    vol: f64,
    /// Mean shock over the window. It is subtracted so the window's shocks sum to zero, which
    /// makes the episode's size the same for every seed while the noise still shapes its path.
    mean_shock: f64,
}

/// One session of a factor: drift, a standard normal shock and the volatility multiplier.
#[derive(Debug, Clone, Copy)]
struct Draw {
    drift: f64,
    shock: f64,
    vol: f64,
    /// Inside a regime window, whose path is pinned by its centred shocks.
    pinned: bool,
}

/// A common return factor: a piecewise drift from its regimes plus hashed Gaussian shocks.
#[derive(Debug)]
pub struct Factor {
    key: u64,
    vol: f64,
    base_drift: f64,
    regimes: Vec<Regime>,
}

impl Factor {
    fn new(
        seed: u64,
        stream: u64,
        id: &str,
        vol: f64,
        base_drift: f64,
        calendar: &MarketCalendar,
    ) -> Self {
        let mut factor = Self {
            key: key(&[seed, stream, hash_str(id)]),
            vol,
            base_drift,
            regimes: Vec::new(),
        };
        factor.regimes = REGIMES
            .iter()
            .filter(|spec| spec.scope == id)
            .map(|spec| {
                let days = calendar.trading_days(spec.from, spec.to);
                let n = days.len().max(1) as f64;
                Regime {
                    from: spec.from,
                    to: spec.to,
                    drift: (1.0 + spec.total).ln() / n + 0.5 * (vol * spec.vol).powi(2),
                    vol: spec.vol,
                    mean_shock: days.iter().map(|day| factor.raw_shock(*day)).sum::<f64>() / n,
                }
            })
            .collect();
        factor
    }

    fn raw_shock(&self, date: NaiveDate) -> f64 {
        Rng::new(key(&[self.key, day_number(date)])).normal()
    }

    fn draw(&self, date: NaiveDate) -> Draw {
        let shock = self.raw_shock(date);
        match self.regimes.iter().find(|r| r.from <= date && date <= r.to) {
            Some(regime) => Draw {
                drift: regime.drift,
                shock: shock - regime.mean_shock,
                vol: regime.vol,
                pinned: true,
            },
            None => Draw {
                drift: self.base_drift,
                shock,
                vol: 1.0,
                pinned: false,
            },
        }
    }
}

/// Seed, calendar and the common factors.
#[derive(Debug)]
pub struct Model {
    pub calendar: MarketCalendar,
    seed: u64,
    market: Factor,
    sectors: HashMap<String, Arc<Factor>>,
}

impl Model {
    pub fn new(seed: u64) -> Self {
        let calendar = MarketCalendar::nyse();
        let drift = MARKET_BASE_DRIFT / TRADING_DAYS + 0.5 * MARKET_VOL * MARKET_VOL;
        let market = Factor::new(seed, STREAM_MARKET, MARKET, MARKET_VOL, drift, &calendar);
        Self {
            calendar,
            seed,
            market,
            sectors: HashMap::new(),
        }
    }

    /// The factor of sector `id`; it depends only on the seed and the id, so every symbol (and
    /// every instance) sees the same values.
    fn sector(&mut self, id: &str) -> Arc<Factor> {
        if let Some(factor) = self.sectors.get(id) {
            return factor.clone();
        }
        let vol = sector_spec(id).factor_vol;
        let factor = Arc::new(Factor::new(
            self.seed,
            STREAM_SECTOR,
            id,
            vol,
            0.0,
            &self.calendar,
        ));
        self.sectors.insert(id.to_string(), factor.clone());
        factor
    }
}

/// Per-symbol parameters, derived from `(seed, symbol, sector)`.
#[derive(Debug)]
pub struct Profile {
    pub symbol: String,
    /// The first session on or after this date is the first bar.
    pub listing: NaiveDate,
    /// Price before the first session (a synthetic offer price).
    pub reference: f64,
    /// Synthetic shares (or ETF units) outstanding.
    pub shares: f64,
    key: u64,
    /// Daily drift on top of the factors.
    drift: f64,
    beta: f64,
    exposures: Vec<(Arc<Factor>, f64)>,
    idio_vol: f64,
    jump_prob: f64,
    /// Typical daily share volume.
    volume: f64,
    /// Daily volatility in a normal regime, without jumps.
    typical_vol: f64,
}

impl Profile {
    pub fn new(model: &mut Model, symbol: &str, sector: &str) -> Self {
        let key = key(&[model.seed, STREAM_SYMBOL, hash_str(symbol)]);
        let mut rng = Rng::new(key);
        let u: [f64; 7] = std::array::from_fn(|_| rng.unit());
        let reference = log_uniform(15.0, 500.0, u[0]);
        let (beta, exposures, idio_vol, alpha, jump_prob, volume, cap_bn) = if sector == BENCHMARK {
            let spec = BENCHMARKS
                .iter()
                .find(|b| b.symbol == symbol)
                .unwrap_or(&OTHER_BENCHMARK);
            let exposures = spec
                .exposures
                .iter()
                .map(|(id, gamma)| (model.sector(id), *gamma))
                .collect();
            (
                spec.beta,
                exposures,
                spec.idio_vol,
                spec.alpha,
                0.0,
                spec.volume,
                spec.aum_bn,
            )
        } else {
            // Members differ from the sector's typical name by up to ±15% in beta, sector
            // loading and idiosyncratic volatility.
            let spec = sector_spec(sector);
            let gamma = 0.85 + 0.3 * u[1];
            (
                spec.beta * (0.85 + 0.3 * u[2]),
                vec![(model.sector(sector), gamma)],
                spec.idio_vol * (0.85 + 0.3 * u[3]),
                spec.alpha + 0.06 * (u[4] - 0.5),
                JUMP_PROB,
                log_uniform(1.5e6, 30e6, u[5]),
                log_uniform(spec.cap_bn.0, spec.cap_bn.1, u[6]),
            )
        };
        let factor_var: f64 = exposures
            .iter()
            .map(|(factor, gamma)| (gamma * factor.vol).powi(2))
            .sum();
        let typical_vol = ((beta * MARKET_VOL).powi(2) + factor_var + idio_vol * idio_vol).sqrt();
        Self {
            symbol: symbol.to_string(),
            listing: listing_date(symbol),
            reference,
            shares: (cap_bn * 1e9 / reference / 1000.0).round() * 1000.0,
            key,
            drift: alpha / TRADING_DAYS,
            beta,
            exposures,
            idio_vol,
            jump_prob,
            volume,
            typical_vol,
        }
    }

    /// Generates the session of `tape` from the previous close.
    fn day(&self, tape: &TapeDay, prev_close: f64) -> Day {
        let date = tape.session.date;
        // Sector and idiosyncratic risk rise (less than proportionally) with the market regime.
        let stress = 1.0 + 0.5 * (tape.vol - 1.0);
        let (mut sector_ret, mut sector_var) = (0.0, 0.0);
        for (factor, gamma) in &self.exposures {
            let draw = factor.draw(date);
            let sd = factor.vol * draw.vol * stress;
            sector_ret += gamma * (draw.drift + sd * draw.shock);
            sector_var += (gamma * sd).powi(2);
        }
        let mut rng = Rng::new(key(&[self.key, STREAM_DAY, day_number(date)]));
        let idio_sd = self.idio_vol * stress;
        let idio = idio_sd * rng.normal();
        let gap_noise = rng.normal();
        let volume_noise = rng.normal();
        let (jump_draw, jump_size, jump_sign) = (rng.unit(), rng.unit(), rng.unit());
        let jump = if jump_draw < self.jump_prob {
            let size = JUMP_MIN + JUMP_SPREAD * jump_size;
            if jump_sign < 0.5 { -size } else { size }
        } else {
            0.0
        };

        let ret = (self.drift + self.beta * tape.market + sector_ret + idio + jump)
            .clamp(-MAX_DAILY_MOVE, MAX_DAILY_MOVE);
        let close = prev_close * (1.0 + ret);
        // Earnings gaps land overnight; otherwise the open carries a share of the day's move.
        let gap = (jump + GAP_SHARE * (ret - jump) + GAP_NOISE * self.typical_vol * gap_noise)
            .clamp(-MAX_GAP, MAX_GAP);
        let open = prev_close * (1.0 + gap);

        let market_sd = self.beta * MARKET_VOL * tape.vol;
        let intraday_vol =
            INTRADAY_SHARE * (market_sd * market_sd + sector_var + idio_sd * idio_sd).sqrt();

        let activity = 0.6 + 0.5 * (ret.abs() / self.typical_vol).min(6.0);
        let mut volume = self.volume * activity * (0.25 * volume_noise - 0.03).exp();
        if jump != 0.0 {
            volume *= 1.6;
        }
        if tape.session.early_close {
            volume *= 0.55;
        }

        let mut day = Day {
            date,
            prev_close,
            open,
            high: open,
            low: open,
            close,
            volume: volume.round().max(100.0),
            intraday_vol,
        };
        let path = self.path(&day, &tape.session);
        (day.high, day.low) = path.range(Progress::complete(path.steps));
        day
    }

    /// The 5-minute path of a generated session.
    pub fn path(&self, day: &Day, session: &Session) -> SessionPath {
        SessionPath::new(
            key(&[self.key, STREAM_PATH, day_number(day.date)]),
            steps(session),
            day.open,
            day.close,
            day.intraday_vol,
        )
    }

    /// How a generated session's volume is spread over its steps.
    pub fn volume_curve(&self, day: &Day, session: &Session) -> VolumeCurve {
        VolumeCurve::new(
            key(&[self.key, STREAM_VOLUME, day_number(day.date)]),
            steps(session),
        )
    }
}

/// One session of the market factor.
#[derive(Debug, Clone, Copy)]
pub struct TapeDay {
    pub session: Session,
    /// Market factor return.
    market: f64,
    /// Volatility multiplier of the market regime.
    vol: f64,
}

/// The market factor for every session since [`HISTORY_START`], shared by all symbols.
#[derive(Debug, Clone)]
pub struct Tape {
    pub days: Vec<TapeDay>,
    /// Every session up to this date is in `days`.
    pub through: NaiveDate,
    /// Accumulated market noise, pulled back towards zero by [`MARKET_REVERSION`].
    deviation: f64,
}

impl Tape {
    pub fn new() -> Self {
        Self {
            days: Vec::new(),
            through: HISTORY_START.pred_opt().expect("valid date"),
            deviation: 0.0,
        }
    }

    /// A copy extended with every session up to `through`.
    pub fn extended(&self, model: &Model, through: NaiveDate) -> Self {
        let calendar = &model.calendar;
        let mut tape = self.clone();
        let mut date = match tape.days.last() {
            Some(last) => calendar.next_trading_day(last.session.date),
            None if calendar.is_trading_day(HISTORY_START) => HISTORY_START,
            None => calendar.next_trading_day(HISTORY_START),
        };
        while date <= through {
            let session = calendar.session(date).expect("trading days have a session");
            let draw = model.market.draw(date);
            // A regime's centred shocks leave the deviation unchanged across its window, so the
            // episode has the same size for every seed. The pull acts between episodes.
            let pull = if draw.pinned {
                0.0
            } else {
                MARKET_REVERSION * tape.deviation
            };
            let noise = MARKET_VOL * draw.vol * draw.shock - pull;
            tape.deviation += noise;
            tape.days.push(TapeDay {
                session,
                market: draw.drift + noise,
                vol: draw.vol,
            });
            date = calendar.next_trading_day(date);
        }
        tape.through = tape.through.max(through);
        tape
    }

    /// Index of the first session on or after `date`.
    fn index_from(&self, date: NaiveDate) -> usize {
        self.days.partition_point(|d| d.session.date < date)
    }
}

/// One generated session of one symbol.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    /// Previous session's close, or the reference price on the first session.
    pub prev_close: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    /// Whole shares.
    pub volume: f64,
    /// Diffusive volatility of the session, needed to rebuild its intraday path.
    pub intraday_vol: f64,
}

/// A symbol's generated sessions since its listing, oldest first.
#[derive(Debug, Clone)]
pub struct History {
    pub days: Vec<Day>,
    /// Every session up to this date (from the listing on) is in `days`.
    pub through: NaiveDate,
}

impl History {
    pub fn new() -> Self {
        Self {
            days: Vec::new(),
            through: HISTORY_START.pred_opt().expect("valid date"),
        }
    }

    /// A copy extended with every session up to `through`; `tape` must cover it.
    pub fn extended(&self, profile: &Profile, tape: &Tape, through: NaiveDate) -> Self {
        let mut days = self.days.clone();
        let next = tape.index_from(profile.listing) + days.len();
        let mut prev_close = days.last().map_or(profile.reference, |d| d.close);
        let pending = tape.days.get(next..).unwrap_or(&[]);
        for tape_day in pending.iter().take_while(|d| d.session.date <= through) {
            let day = profile.day(tape_day, prev_close);
            prev_close = day.close;
            days.push(day);
        }
        Self {
            days,
            through: self.through.max(through),
        }
    }

    /// Sessions in `[from, to]`.
    pub fn range(&self, from: NaiveDate, to: NaiveDate) -> &[Day] {
        let start = self.days.partition_point(|d| d.date < from);
        let end = self.days.partition_point(|d| d.date <= to);
        &self.days[start..end.max(start)]
    }

    pub fn get(&self, date: NaiveDate) -> Option<&Day> {
        self.range(date, date).first()
    }
}

/// How far into a session: `step` whole steps plus `fraction` of the next one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub step: usize,
    pub fraction: f64,
}

impl Progress {
    pub fn complete(steps: usize) -> Self {
        Self {
            step: steps,
            fraction: 0.0,
        }
    }

    /// Progress of `session` at `now`, clamped to the session.
    pub fn at(session: &Session, now: DateTime<Utc>) -> Self {
        let steps = steps(session);
        let step_ms = STEP_MINUTES * 60_000;
        let elapsed = (now - session.open)
            .num_milliseconds()
            .clamp(0, steps as i64 * step_ms);
        let step = (elapsed / step_ms) as usize;
        if step >= steps {
            return Self::complete(steps);
        }
        Self {
            step,
            fraction: (elapsed % step_ms) as f64 / step_ms as f64,
        }
    }
}

/// One bar's prices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ohlc {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

/// The U-shaped intraday variance profile for one session length: busier after the open and into
/// the close.
struct StepProfile {
    /// Share of the session's volatility in each step, `√(w_k / Σw)`.
    sd: [f64; MAX_STEPS],
    /// Share of the session's variance before each step boundary, `Σ_{i<k} w_i / Σw`.
    cum: [f64; MAX_STEPS + 1],
}

fn step_profile(steps: usize) -> &'static StepProfile {
    static PROFILES: [OnceLock<StepProfile>; MAX_STEPS + 1] =
        [const { OnceLock::new() }; MAX_STEPS + 1];
    PROFILES[steps].get_or_init(|| {
        let mut profile = StepProfile {
            sd: [0.0; MAX_STEPS],
            cum: [0.0; MAX_STEPS + 1],
        };
        let weight = |k: usize| {
            let u = 2.0 * (k as f64 + 0.5) / steps as f64 - 1.0;
            1.0 + 1.5 * u * u
        };
        let total: f64 = (0..steps).map(weight).sum();
        for k in 0..steps {
            profile.sd[k] = (weight(k) / total).sqrt();
            profile.cum[k + 1] = profile.cum[k] + weight(k) / total;
        }
        profile
    })
}

/// A session's 5-minute price path: a Brownian bridge in log price from the open to the close
/// with a U-shaped variance profile, plus a wick above and below every step.
pub struct SessionPath {
    steps: usize,
    open: f64,
    close: f64,
    /// Log prices at step boundaries; `log[0]` is the open and `log[steps]` the close.
    log: [f64; MAX_STEPS + 1],
    /// Log-price wicks above and below each step's open/close range.
    wick_up: [f64; MAX_STEPS],
    wick_down: [f64; MAX_STEPS],
}

/// Log-space margin within which [`SessionPath::range`] re-checks bars exactly. `exp` is accurate
/// to about one ulp, so a bar further than this below the log-space extreme cannot hold it.
const RANGE_MARGIN: f64 = 1e-9;

impl SessionPath {
    pub fn new(key: u64, steps: usize, open: f64, close: f64, vol: f64) -> Self {
        let steps = steps.clamp(1, MAX_STEPS);
        let profile = step_profile(steps);
        let mut rng = Rng::new(key);
        let mut walk = [0.0; MAX_STEPS + 1];
        for k in 0..steps {
            walk[k + 1] = walk[k] + vol * profile.sd[k] * rng.quick_normal();
        }
        // Pin the free walk to the close: B_k = W_k + (V_k / V_n)·(ln C − ln O − W_n).
        let (log_open, log_close) = (open.ln(), close.ln());
        let miss = log_close - log_open - walk[steps];
        let mut path = Self {
            steps,
            open,
            close,
            log: [0.0; MAX_STEPS + 1],
            wick_up: [0.0; MAX_STEPS],
            wick_down: [0.0; MAX_STEPS],
        };
        let interior = path.log[1..steps]
            .iter_mut()
            .zip(&walk[1..steps])
            .zip(&profile.cum[1..steps]);
        for ((log, walk), cum) in interior {
            *log = log_open + walk + miss * cum;
        }
        (path.log[0], path.log[steps]) = (log_open, log_close);
        let wicks = path.wick_up[..steps]
            .iter_mut()
            .zip(&mut path.wick_down[..steps]);
        for ((up, down), sd) in wicks.zip(&profile.sd) {
            let (u, d) = rng.unit_pair();
            *up = WICK * vol * sd * u;
            *down = WICK * vol * sd * d;
        }
        path
    }

    /// Price at step boundary `i`, with the exact open and close at the ends.
    fn price(&self, i: usize) -> f64 {
        if i == 0 {
            self.open
        } else if i >= self.steps {
            self.close
        } else {
            self.log[i].exp()
        }
    }

    /// `(last log price, top, bottom)` of step `k`, or of its first `fraction` while it is in
    /// progress; top and bottom include the wicks, in log units.
    #[inline]
    fn log_bar(&self, k: usize, fraction: f64) -> (f64, f64, f64) {
        let (start, end) = (self.log[k], self.log[k + 1]);
        if fraction > 0.0 && fraction < 1.0 {
            // The wicks grow with the share of the step that has traded.
            let (up, down) = (self.wick_up[k] * fraction, self.wick_down[k] * fraction);
            let now = start + fraction * (end - start);
            let (top, bottom) = log_extremes(start, now, up, down);
            (now, top, bottom)
        } else {
            let (top, bottom) = log_extremes(start, end, self.wick_up[k], self.wick_down[k]);
            (end, top, bottom)
        }
    }

    /// Step `k`, or only its first `fraction` while it is in progress (`fraction` in `(0, 1)`).
    pub fn bar(&self, k: usize, fraction: f64) -> Ohlc {
        let (end, top, bottom) = self.log_bar(k, fraction);
        let open = self.price(k);
        let close = if fraction > 0.0 && fraction < 1.0 {
            end.exp()
        } else {
            self.price(k + 1)
        };
        Ohlc {
            open,
            high: top.exp().max(open).max(close),
            low: bottom.exp().min(open).min(close),
            close,
        }
    }

    /// `(step, fraction)` of every bar traded by `progress`, oldest first; the last one is
    /// partial when `progress` is mid-step.
    pub fn bars(&self, progress: Progress) -> impl Iterator<Item = (usize, f64)> {
        let done = progress.step.min(self.steps);
        let partial = (progress.fraction > 0.0 && progress.step < self.steps)
            .then_some((progress.step, progress.fraction));
        (0..done).map(|k| (k, 1.0)).chain(partial)
    }

    /// Last price at `progress`.
    pub fn price_at(&self, progress: Progress) -> f64 {
        if progress.fraction > 0.0 && progress.step < self.steps {
            self.bar(progress.step, progress.fraction).close
        } else {
            self.price(progress.step)
        }
    }

    /// `(high, low)` traded by `progress` (just the open before the first trade): exactly the
    /// extremes of [`Self::bar`] over [`Self::bars`]. Bars are compared in log space and only
    /// those within [`RANGE_MARGIN`] of an extreme are evaluated, which keeps years of history
    /// cheap to build.
    pub fn range(&self, progress: Progress) -> (f64, f64) {
        let done = progress.step.min(self.steps);
        let partial = (progress.fraction > 0.0 && progress.step < self.steps).then(|| {
            let (_, top, bottom) = self.log_bar(done, progress.fraction);
            (top, bottom)
        });
        // Log-space (top, bottom) of every traded bar; equal to `log_bar`'s, without branching.
        let extremes = || {
            self.log[..=done]
                .windows(2)
                .zip(&self.wick_up[..done])
                .zip(&self.wick_down[..done])
                .map(|((ends, &up), &down)| log_extremes(ends[0], ends[1], up, down))
                .chain(partial)
        };
        let (top, bottom) = extremes().fold((f64::NEG_INFINITY, f64::INFINITY), |acc, bar| {
            (acc.0.max(bar.0), acc.1.min(bar.1))
        });
        extremes()
            .enumerate()
            .filter(|(_, (up, down))| *up >= top - RANGE_MARGIN || *down <= bottom + RANGE_MARGIN)
            .fold((self.open, self.open), |(high, low), (k, _)| {
                let fraction = if k < done { 1.0 } else { progress.fraction };
                let bar = self.bar(k, fraction);
                (high.max(bar.high), low.min(bar.low))
            })
    }
}

/// Log-space `(top, bottom)` of a bar between log prices `a` and `b` with the given wicks.
#[inline]
fn log_extremes(a: f64, b: f64, up: f64, down: f64) -> (f64, f64) {
    (a.max(b) + up, a.min(b) - down)
}

/// Cumulative share of a session's volume at each step boundary: U-shaped, heavier on the
/// opening and closing bars, with noise.
pub struct VolumeCurve {
    steps: usize,
    cum: [f64; MAX_STEPS + 1],
}

impl VolumeCurve {
    pub fn new(key: u64, steps: usize) -> Self {
        let steps = steps.clamp(1, MAX_STEPS);
        let mut rng = Rng::new(key);
        let mut cum = [0.0; MAX_STEPS + 1];
        for k in 0..steps {
            let u = 2.0 * (k as f64 + 0.5) / steps as f64 - 1.0;
            let mut weight = (1.0 + 2.5 * u * u) * (0.75 + 0.5 * rng.unit());
            if k == 0 {
                weight *= 1.3;
            }
            if k + 1 == steps {
                weight *= 1.8;
            }
            cum[k + 1] = cum[k] + weight;
        }
        let total = cum[steps];
        for c in &mut cum[1..=steps] {
            *c /= total;
        }
        Self { steps, cum }
    }

    fn share(&self, step: usize, fraction: f64) -> f64 {
        if step >= self.steps {
            1.0
        } else if fraction <= 0.0 {
            self.cum[step]
        } else if fraction >= 1.0 {
            self.cum[step + 1]
        } else {
            self.cum[step] + fraction * (self.cum[step + 1] - self.cum[step])
        }
    }

    /// Whole shares traded from the open to `step + fraction`, out of a day's `volume`.
    pub fn traded(&self, volume: f64, progress: Progress) -> f64 {
        (volume * self.share(progress.step, progress.fraction)).round()
    }

    /// Whole shares traded in step `k` (or its first `fraction`); bars sum to the day's volume.
    pub fn bar(&self, volume: f64, k: usize, fraction: f64) -> f64 {
        (volume * self.share(k, fraction)).round() - (volume * self.share(k, 0.0)).round()
    }
}
