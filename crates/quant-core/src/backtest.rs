//! Daily-bar backtester that runs the *same* strategy, rebalancer and cost model as live paper
//! trading.
//!
//! How the twice-daily live process maps onto daily bars:
//! - Opening plan → decided with closes up to the previous session plus today's open as the
//!   current price, filled at today's open (plus slippage). It never sees today's high, low or
//!   close.
//! - Pre-close plan → decided with closes up to the previous session plus today's close as the
//!   current price, filled at today's close (plus slippage). This is the standard
//!   market-on-close idealisation; the slow signals used here make the difference from a 13:00
//!   decision immaterial, and the report says so.
//!
//! Prices must be split- and dividend-adjusted so returns are total returns. The universe is
//! today's honeclaw ontology, which was chosen with hindsight: every result carries a
//! survivorship/selection-bias warning, and names only become eligible once they have the
//! required history (no look-ahead into pre-listing data).

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::costs::{CostModel, Side};
use crate::metrics::{self, PerformanceMetrics, PeriodReturn};
use crate::rebalance::{OrderReason, RebalanceInput, rebalance};
use crate::schedule::PlanSlot;
use crate::strategy::{AssetMeta, EngineInput, StrategyParams, compute_targets};

pub const UNIVERSE_EW_BENCHMARK: &str = "UNIVERSE_EW";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebalanceFrequency {
    Daily,
    Weekly,
    Monthly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BacktestConfig {
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub initial_cash: f64,
    pub params: StrategyParams,
    #[serde(default)]
    pub costs: CostModel,
    #[serde(default = "default_slots")]
    pub slots: Vec<PlanSlot>,
    #[serde(default = "default_frequency")]
    pub frequency: RebalanceFrequency,
    #[serde(default)]
    pub risk_free_rate: f64,
    /// Primary benchmark for relative statistics; defaults to the first benchmark series.
    #[serde(default)]
    pub benchmark: Option<String>,
}

fn default_slots() -> Vec<PlanSlot> {
    vec![PlanSlot::Open, PlanSlot::Close]
}

fn default_frequency() -> RebalanceFrequency {
    RebalanceFrequency::Daily
}

/// Aligned adjusted price panel. Warm-up history before `start` should be included.
pub struct BacktestData {
    pub dates: Vec<NaiveDate>,
    pub assets: Vec<AssetMeta>,
    /// Adjusted opens `[asset][t]`; `NaN` where missing.
    pub open: Vec<Vec<f64>>,
    /// Adjusted closes `[asset][t]`; `NaN` where missing.
    pub close: Vec<Vec<f64>>,
    /// Adjusted closes of benchmark instruments, aligned to `dates`.
    pub benchmarks: Vec<(String, Vec<f64>)>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum BacktestError {
    #[error("invalid strategy parameters: {0}")]
    InvalidParams(String),
    #[error("invalid cost model: {0}")]
    InvalidCosts(String),
    #[error("initial cash must be positive")]
    InvalidCash,
    #[error("the backtest needs at least one plan slot")]
    NoSlots,
    #[error("start must be before end")]
    InvalidRange,
    #[error("price data does not cover the requested period")]
    NoData,
    #[error("price panel is misaligned")]
    Misaligned,
}

#[derive(Debug, Clone, Serialize)]
pub struct BacktestPoint {
    pub date: NaiveDate,
    pub nav: f64,
    pub cash: f64,
    pub invested: f64,
    pub turnover: f64,
    pub costs: f64,
    pub trades: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct BacktestTrade {
    pub date: NaiveDate,
    pub slot: PlanSlot,
    pub symbol: String,
    pub side: Side,
    pub reason: OrderReason,
    pub qty: f64,
    pub price: f64,
    pub notional: f64,
    pub cost: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkResult {
    pub symbol: String,
    /// Normalised to the initial capital, aligned with `points`.
    pub values: Vec<f64>,
    pub metrics: PerformanceMetrics,
}

#[derive(Debug, Clone, Serialize)]
pub struct Contribution {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sector: Option<String>,
    /// Approximate return contribution Σ w(t−1)·r(t).
    pub contribution: f64,
    pub avg_weight: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SectorWeightSample {
    pub date: NaiveDate,
    pub weights: BTreeMap<String, f64>,
    pub cash: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinalPosition {
    pub symbol: String,
    pub sector: String,
    pub qty: f64,
    pub value: f64,
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum BacktestWarning {
    /// The universe is today's ontology, selected with knowledge of the period.
    SurvivorshipBias,
    /// Names whose first price falls after the start; they join once they have enough history.
    LateListings { symbols: Vec<(String, NaiveDate)> },
    /// Names with no price at all in the period.
    MissingData { symbols: Vec<String> },
    /// Less than one year of results: annualised figures are noisy.
    ShortPeriod,
}

#[derive(Debug, Clone, Serialize)]
pub struct BacktestResult {
    pub points: Vec<BacktestPoint>,
    pub metrics: PerformanceMetrics,
    pub benchmarks: Vec<BenchmarkResult>,
    pub drawdowns: Vec<f64>,
    pub monthly_returns: Vec<PeriodReturn>,
    pub yearly_returns: Vec<PeriodReturn>,
    pub sector_weights: Vec<SectorWeightSample>,
    pub asset_contributions: Vec<Contribution>,
    pub sector_contributions: Vec<Contribution>,
    pub trades: Vec<BacktestTrade>,
    pub final_positions: Vec<FinalPosition>,
    pub total_costs: f64,
    /// Sum of per-plan one-way turnover.
    pub total_turnover: f64,
    pub annual_turnover: f64,
    pub rebalances: usize,
    pub warnings: Vec<BacktestWarning>,
}

fn is_rebalance_day(
    frequency: RebalanceFrequency,
    dates: &[NaiveDate],
    t: usize,
    t0: usize,
) -> bool {
    if t == t0 {
        return true;
    }
    match frequency {
        RebalanceFrequency::Daily => true,
        RebalanceFrequency::Weekly => dates[t].iso_week() != dates[t - 1].iso_week(),
        RebalanceFrequency::Monthly => dates[t].month() != dates[t - 1].month(),
    }
}

pub fn run_backtest(
    config: &BacktestConfig,
    data: &BacktestData,
) -> Result<BacktestResult, BacktestError> {
    config.params.validate().map_err(|errors| {
        BacktestError::InvalidParams(
            errors
                .into_iter()
                .map(|e| e.message)
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;
    config
        .costs
        .validate()
        .map_err(BacktestError::InvalidCosts)?;
    if !(config.initial_cash > 0.0) {
        return Err(BacktestError::InvalidCash);
    }
    if config.slots.is_empty() {
        return Err(BacktestError::NoSlots);
    }
    if config.start >= config.end {
        return Err(BacktestError::InvalidRange);
    }
    let n = data.assets.len();
    let len = data.dates.len();
    if data.open.len() != n
        || data.close.len() != n
        || data.open.iter().chain(&data.close).any(|s| s.len() != len)
        || data.benchmarks.iter().any(|(_, s)| s.len() != len)
    {
        return Err(BacktestError::Misaligned);
    }
    let t0 = data
        .dates
        .iter()
        .position(|d| *d >= config.start)
        .ok_or(BacktestError::NoData)?;
    let t1 = data
        .dates
        .iter()
        .rposition(|d| *d <= config.end)
        .ok_or(BacktestError::NoData)?;
    if t1 <= t0 {
        return Err(BacktestError::NoData);
    }
    let mut slots = config.slots.clone();
    slots.sort();
    slots.dedup();

    let mut cash = config.initial_cash;
    let mut qty = vec![0.0; n];
    let mut last_close = vec![f64::NAN; n];
    for i in 0..n {
        // Last valid close before the first simulated day, for valuation.
        if let Some(p) = data.close[i][..t0]
            .iter()
            .rev()
            .find(|p| p.is_finite() && **p > 0.0)
        {
            last_close[i] = *p;
        }
    }
    let no_flags = vec![false; n];

    let mut points = Vec::with_capacity(t1 - t0 + 1);
    let mut trades = Vec::new();
    let mut total_costs = 0.0;
    let mut total_turnover = 0.0;
    let mut rebalances = 0usize;
    let mut eod_weights_prev: Option<Vec<f64>> = None;
    let mut contribution = vec![0.0; n];
    let mut weight_sum = vec![0.0; n];
    let mut sector_weights = Vec::new();

    for t in t0..=t1 {
        let date = data.dates[t];
        let mut day_turnover = 0.0;
        let mut day_costs = 0.0;
        let mut day_trades = 0usize;

        if is_rebalance_day(config.frequency, &data.dates, t, t0) {
            for slot in &slots {
                let prices: Vec<f64> = (0..n)
                    .map(|i| match slot {
                        PlanSlot::Open => data.open[i][t],
                        PlanSlot::Close => data.close[i][t],
                    })
                    .collect();
                let valuation: Vec<f64> = (0..n)
                    .map(|i| {
                        if prices[i].is_finite() && prices[i] > 0.0 {
                            prices[i]
                        } else {
                            last_close[i]
                        }
                    })
                    .collect();
                let positions_value: f64 = (0..n)
                    .filter(|&i| valuation[i].is_finite())
                    .map(|i| qty[i] * valuation[i])
                    .sum();
                let nav = cash + positions_value;
                let current_weights: Vec<f64> = (0..n)
                    .map(|i| {
                        if nav > 0.0 && valuation[i].is_finite() {
                            qty[i] * valuation[i] / nav
                        } else {
                            0.0
                        }
                    })
                    .collect();
                let history: Vec<&[f64]> = data.close.iter().map(|c| &c[..t]).collect();
                let targets = compute_targets(
                    &config.params,
                    &EngineInput {
                        assets: &data.assets,
                        history: &history,
                        current: &prices,
                        current_weights: &current_weights,
                        excluded: &no_flags,
                        frozen: &no_flags,
                    },
                );
                let tradable: Vec<bool> =
                    prices.iter().map(|p| p.is_finite() && *p > 0.0).collect();
                // Valuation prices let held names without a fresh print count toward NAV.
                let plan = rebalance(
                    &RebalanceInput {
                        cash,
                        qty: &qty,
                        prices: &valuation,
                        targets: &targets.weights,
                        tradable: &tradable,
                    },
                    &config.params.rebalance,
                    &config.costs,
                );
                if !plan.orders.is_empty() {
                    rebalances += 1;
                }
                day_turnover += plan.turnover;
                for order in &plan.orders {
                    let exec = config.costs.execution_price(order.side, order.price);
                    let delta = config.costs.cash_delta(order.side, order.qty, exec);
                    let notional = order.qty * exec;
                    let cost = config.costs.commission(order.qty, exec)
                        + config.costs.fees(order.side, notional)
                        + (exec - order.price).abs() * order.qty;
                    cash += delta;
                    qty[order.asset] += order.side.sign() * order.qty;
                    if qty[order.asset].abs() < 1e-9 {
                        qty[order.asset] = 0.0;
                    }
                    day_costs += cost;
                    day_trades += 1;
                    trades.push(BacktestTrade {
                        date,
                        slot: *slot,
                        symbol: data.assets[order.asset].symbol.clone(),
                        side: order.side,
                        reason: order.reason,
                        qty: order.qty,
                        price: exec,
                        notional,
                        cost,
                    });
                }
            }
        }

        // End-of-day mark to market.
        for i in 0..n {
            let c = data.close[i][t];
            if c.is_finite() && c > 0.0 {
                last_close[i] = c;
            }
        }
        let values: Vec<f64> = (0..n)
            .map(|i| {
                if last_close[i].is_finite() {
                    qty[i] * last_close[i]
                } else {
                    0.0
                }
            })
            .collect();
        let invested: f64 = values.iter().sum();
        let nav = cash + invested;

        // Attribution with yesterday's closing weights and today's close-to-close returns.
        if let Some(prev) = &eod_weights_prev {
            for i in 0..n {
                let (a, b) = (data.close[i][t - 1], data.close[i][t]);
                if prev[i] != 0.0 && a.is_finite() && b.is_finite() && a > 0.0 {
                    contribution[i] += prev[i] * (b / a - 1.0);
                }
            }
        }
        let eod_weights: Vec<f64> = values
            .iter()
            .map(|v| if nav > 0.0 { v / nav } else { 0.0 })
            .collect();
        for i in 0..n {
            weight_sum[i] += eod_weights[i];
        }
        let week_changed =
            t == t0 || data.dates[t].iso_week() != data.dates[t - 1].iso_week() || t == t1;
        if week_changed {
            let mut weights = BTreeMap::new();
            for i in 0..n {
                *weights.entry(data.assets[i].sector.clone()).or_insert(0.0) += eod_weights[i];
            }
            sector_weights.push(SectorWeightSample {
                date,
                weights,
                cash: if nav > 0.0 { cash / nav } else { 0.0 },
            });
        }
        eod_weights_prev = Some(eod_weights);

        total_turnover += day_turnover;
        total_costs += day_costs;
        points.push(BacktestPoint {
            date,
            nav,
            cash,
            invested,
            turnover: day_turnover,
            costs: day_costs,
            trades: day_trades,
        });
    }

    let dates: Vec<NaiveDate> = points.iter().map(|p| p.date).collect();
    let navs: Vec<f64> = points.iter().map(|p| p.nav).collect();

    // Benchmarks: provided instruments plus an equal-weight universe index (no costs).
    let mut benchmarks = Vec::new();
    for (symbol, series) in &data.benchmarks {
        let mut values = Vec::with_capacity(points.len());
        let mut last = last_valid_before(series, t0);
        let mut base = f64::NAN;
        for t in t0..=t1 {
            if series[t].is_finite() && series[t] > 0.0 {
                last = series[t];
            }
            if !base.is_finite() && last.is_finite() {
                base = last;
            }
            values.push(if base.is_finite() {
                config.initial_cash * last / base
            } else {
                config.initial_cash
            });
        }
        benchmarks.push((symbol.clone(), values));
    }
    // The universe as an equal-weight index, reset to equal weights on the first session of each
    // month (names join at the first reset after they list) and left to drift in between. Daily
    // rebalancing would harvest a volatility premium no investable strategy earns.
    let mut ew = Vec::with_capacity(points.len());
    let mut level = config.initial_cash;
    let mut holdings = vec![0.0; n];
    let mut month = None;
    for t in t0..=t1 {
        if t > t0 {
            for (i, value) in holdings.iter_mut().enumerate() {
                let (a, b) = (data.close[i][t - 1], data.close[i][t]);
                if *value > 0.0 && a.is_finite() && b.is_finite() && a > 0.0 && b > 0.0 {
                    *value *= b / a;
                }
            }
            let total: f64 = holdings.iter().sum();
            if total > 0.0 {
                level = total;
            }
        }
        let this_month = (data.dates[t].year(), data.dates[t].month());
        if month != Some(this_month) {
            let listed: Vec<usize> = (0..n)
                .filter(|&i| data.close[i][t].is_finite() && data.close[i][t] > 0.0)
                .collect();
            if !listed.is_empty() {
                holdings.iter_mut().for_each(|value| *value = 0.0);
                for &i in &listed {
                    holdings[i] = level / listed.len() as f64;
                }
            }
            month = Some(this_month);
        }
        ew.push(level);
    }
    benchmarks.push((UNIVERSE_EW_BENCHMARK.to_string(), ew));

    let primary = config
        .benchmark
        .as_ref()
        .and_then(|wanted| benchmarks.iter().find(|(s, _)| s == wanted))
        .or_else(|| benchmarks.first())
        .map(|(_, v)| v.clone());
    let metrics = metrics::compute(&dates, &navs, primary.as_deref(), config.risk_free_rate);
    let benchmark_results = benchmarks
        .into_iter()
        .map(|(symbol, values)| {
            let m = metrics::compute(&dates, &values, None, config.risk_free_rate);
            BenchmarkResult {
                symbol,
                values,
                metrics: m,
            }
        })
        .collect();

    // Attribution tables.
    let days = points.len() as f64;
    let mut asset_contributions: Vec<Contribution> = (0..n)
        .filter(|&i| weight_sum[i] > 0.0 || contribution[i] != 0.0)
        .map(|i| Contribution {
            key: data.assets[i].symbol.clone(),
            sector: Some(data.assets[i].sector.clone()),
            contribution: contribution[i],
            avg_weight: weight_sum[i] / days,
        })
        .collect();
    asset_contributions.sort_by(|a, b| b.contribution.total_cmp(&a.contribution));
    let mut by_sector: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for c in &asset_contributions {
        let entry = by_sector
            .entry(c.sector.clone().unwrap_or_default())
            .or_insert((0.0, 0.0));
        entry.0 += c.contribution;
        entry.1 += c.avg_weight;
    }
    let mut sector_contributions: Vec<Contribution> = by_sector
        .into_iter()
        .map(|(key, (contribution, avg_weight))| Contribution {
            key,
            sector: None,
            contribution,
            avg_weight,
        })
        .collect();
    sector_contributions.sort_by(|a, b| b.contribution.total_cmp(&a.contribution));

    let final_nav = *navs.last().unwrap_or(&config.initial_cash);
    let mut final_positions: Vec<FinalPosition> = (0..n)
        .filter(|&i| qty[i] > 0.0)
        .map(|i| {
            let value = qty[i] * last_close[i];
            FinalPosition {
                symbol: data.assets[i].symbol.clone(),
                sector: data.assets[i].sector.clone(),
                qty: qty[i],
                value,
                weight: if final_nav > 0.0 {
                    value / final_nav
                } else {
                    0.0
                },
            }
        })
        .collect();
    final_positions.sort_by(|a, b| b.value.total_cmp(&a.value));

    // Warnings.
    let mut warnings = vec![BacktestWarning::SurvivorshipBias];
    let mut late = Vec::new();
    let mut missing = Vec::new();
    for i in 0..n {
        let first = (0..=t1).find(|&t| data.close[i][t].is_finite() && data.close[i][t] > 0.0);
        match first {
            None => missing.push(data.assets[i].symbol.clone()),
            Some(t) if t > t0 => late.push((data.assets[i].symbol.clone(), data.dates[t])),
            Some(_) => {}
        }
    }
    if !late.is_empty() {
        warnings.push(BacktestWarning::LateListings { symbols: late });
    }
    if !missing.is_empty() {
        warnings.push(BacktestWarning::MissingData { symbols: missing });
    }
    if (dates[dates.len() - 1] - dates[0]).num_days() < 365 {
        warnings.push(BacktestWarning::ShortPeriod);
    }

    let years = ((dates[dates.len() - 1] - dates[0]).num_days() as f64 / 365.25).max(1.0 / 365.25);
    Ok(BacktestResult {
        drawdowns: metrics::drawdowns(&navs),
        monthly_returns: metrics::monthly_returns(&dates, &navs),
        yearly_returns: metrics::yearly_returns(&dates, &navs),
        points,
        metrics,
        benchmarks: benchmark_results,
        sector_weights,
        asset_contributions,
        sector_contributions,
        trades,
        final_positions,
        total_costs,
        total_turnover,
        annual_turnover: total_turnover / years,
        rebalances,
        warnings,
    })
}

fn last_valid_before(series: &[f64], t0: usize) -> f64 {
    series[..t0]
        .iter()
        .rev()
        .find(|p| p.is_finite() && **p > 0.0)
        .copied()
        .unwrap_or(f64::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::preset;
    use chrono::Duration;

    fn trading_dates(n: usize) -> Vec<NaiveDate> {
        let mut out = Vec::new();
        let mut d = NaiveDate::from_ymd_opt(2023, 1, 2).unwrap();
        while out.len() < n {
            if d.weekday().num_days_from_monday() < 5 {
                out.push(d);
            }
            d += Duration::days(1);
        }
        out
    }

    fn data(n_days: usize) -> BacktestData {
        let dates = trading_dates(n_days);
        let specs = [
            ("NVDA", "ai-chip", 0.0012, 0.02),
            ("AMD", "ai-chip", 0.0006, 0.03),
            ("MU", "storage", 0.0004, 0.025),
            ("VST", "power", 0.0008, 0.015),
            ("RKLB", "space", 0.0015, 0.05),
        ];
        let mut assets = Vec::new();
        let mut open = Vec::new();
        let mut close = Vec::new();
        for (k, (symbol, sector, drift, wiggle)) in specs.iter().enumerate() {
            assets.push(AssetMeta {
                symbol: symbol.to_string(),
                sector: sector.to_string(),
            });
            let c: Vec<f64> = (0..n_days)
                .map(|t| {
                    let x = t as f64;
                    100.0 * (drift * x).exp() * (1.0 + wiggle * (x * 0.37 + k as f64).sin())
                })
                .collect();
            let o: Vec<f64> = (0..n_days)
                .map(|t| {
                    if t == 0 {
                        c[0]
                    } else {
                        (c[t - 1] + c[t]) / 2.0
                    }
                })
                .collect();
            open.push(o);
            close.push(c);
        }
        let bench: Vec<f64> = (0..n_days)
            .map(|t| 100.0 * (0.0004 * t as f64).exp())
            .collect();
        BacktestData {
            dates,
            assets,
            open,
            close,
            benchmarks: vec![("SPY".to_string(), bench)],
        }
    }

    fn config(data: &BacktestData, params: StrategyParams) -> BacktestConfig {
        BacktestConfig {
            start: data.dates[260],
            end: *data.dates.last().unwrap(),
            initial_cash: 1_000_000.0,
            params,
            costs: CostModel::default(),
            slots: default_slots(),
            frequency: RebalanceFrequency::Daily,
            risk_free_rate: 0.0,
            benchmark: Some("SPY".to_string()),
        }
    }

    #[test]
    fn universe_benchmark_drifts_between_monthly_resets() {
        // Flat prices except one name that doubles for a day, inside one calendar month.
        let mut d = data(400);
        for i in 0..d.assets.len() {
            d.close[i].iter_mut().for_each(|c| *c = 100.0);
            d.open[i].iter_mut().for_each(|o| *o = 100.0);
        }
        let t0 = 260;
        let k = (t0 + 2..d.dates.len() - 1)
            .find(|&t| {
                d.dates[t - 1].month() == d.dates[t].month()
                    && d.dates[t].month() == d.dates[t + 1].month()
            })
            .unwrap();
        d.close[0][k] = 200.0;
        let cfg = config(&d, preset("equal_weight").unwrap().params);
        let result = run_backtest(&cfg, &d).unwrap();
        let ew = &result
            .benchmarks
            .iter()
            .find(|b| b.symbol == UNIVERSE_EW_BENCHMARK)
            .unwrap()
            .values;
        // One of five names doubles: +20%. It falls back the next day and, because holdings drift
        // instead of being reset daily, the index is exactly where it started (a daily-rebalanced
        // index would be 8% higher).
        assert!((ew[k - t0] / ew[k - 1 - t0] - 1.2).abs() < 1e-9);
        assert!((ew[k + 1 - t0] / ew[k - 1 - t0] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn backtest_runs_and_accounts_consistently() {
        let data = data(700);
        let cfg = config(&data, StrategyParams::default());
        let result = run_backtest(&cfg, &data).unwrap();
        assert_eq!(result.points.len(), 700 - 260);
        assert_eq!(result.points[0].date, data.dates[260]);
        assert!(result.rebalances > 0);
        assert!(!result.trades.is_empty());
        // Cash never negative; NAV identity holds every day.
        for p in &result.points {
            assert!(p.cash >= -1e-6, "negative cash {}", p.cash);
            assert!((p.nav - p.cash - p.invested).abs() < 1e-6);
        }
        let traded_costs: f64 = result.trades.iter().map(|t| t.cost).sum();
        assert!((traded_costs - result.total_costs).abs() < 1e-6);
        assert!(result.metrics.trading_days > 0);
        assert_eq!(result.benchmarks.len(), 2);
        assert_eq!(result.benchmarks[1].symbol, UNIVERSE_EW_BENCHMARK);
        assert!(result.metrics.beta.is_some());
        assert!(result.warnings.contains(&BacktestWarning::SurvivorshipBias));
        assert!(!result.sector_weights.is_empty());
    }

    #[test]
    fn frictionless_buy_and_hold_matches_the_asset() {
        // One asset, target 100%, no bands, no costs → NAV tracks the price after the first fill.
        let mut d = data(400);
        d.assets.truncate(1);
        d.open.truncate(1);
        d.close.truncate(1);
        let mut params = preset("equal_weight").unwrap().params;
        params.exposure.max_exposure = 1.0;
        params.exposure.min_exposure = 1.0;
        params.asset.max_weight = 0.5;
        params.rebalance.fractional_shares = true;
        params.rebalance.band_abs = 0.2;
        params.rebalance.band_rel = 1.0;
        params.rebalance.max_turnover = 2.0;
        params.rebalance.min_trade_value = 0.0;
        let mut cfg = config(&d, params);
        cfg.costs = CostModel::zero();
        cfg.slots = vec![PlanSlot::Close];
        let result = run_backtest(&cfg, &d).unwrap();
        // Single-name cap 50% → half invested, half cash.
        let first = &result.points[0];
        assert!((first.invested / first.nav - 0.5).abs() < 1e-3);
        let t0 = 260;
        let asset_ret = d.close[0][d.dates.len() - 1] / d.close[0][t0] - 1.0;
        // NAV return ≈ half the asset return (cash earns nothing, drift stays within the band).
        let nav_ret = result.metrics.total_return;
        assert!(
            (nav_ret - 0.5 * asset_ret).abs() < 0.05 * asset_ret.abs().max(0.01),
            "{nav_ret} vs {asset_ret}"
        );
    }

    #[test]
    fn late_listing_is_reported_and_not_traded_early() {
        let mut d = data(700);
        for t in 0..400 {
            d.close[4][t] = f64::NAN;
            d.open[4][t] = f64::NAN;
        }
        let cfg = config(&d, StrategyParams::default());
        let result = run_backtest(&cfg, &d).unwrap();
        let first_rklb_trade = result
            .trades
            .iter()
            .find(|t| t.symbol == "RKLB")
            .map(|t| t.date);
        if let Some(date) = first_rklb_trade {
            let listed = d.dates[400];
            assert!(
                date >= listed + Duration::days(150),
                "traded too early: {date}"
            );
        }
        assert!(
            result
                .warnings
                .iter()
                .any(|w| matches!(w, BacktestWarning::LateListings { .. }))
        );
    }

    #[test]
    fn weekly_frequency_trades_less() {
        let d = data(700);
        let daily = run_backtest(&config(&d, StrategyParams::default()), &d).unwrap();
        let mut cfg = config(&d, StrategyParams::default());
        cfg.frequency = RebalanceFrequency::Weekly;
        let weekly = run_backtest(&cfg, &d).unwrap();
        assert!(weekly.trades.len() <= daily.trades.len());
    }

    #[test]
    fn invalid_configs_are_rejected() {
        let d = data(300);
        let mut cfg = config(&d, StrategyParams::default());
        cfg.initial_cash = 0.0;
        assert_eq!(
            run_backtest(&cfg, &d).unwrap_err(),
            BacktestError::InvalidCash
        );
        let mut cfg = config(&d, StrategyParams::default());
        cfg.slots.clear();
        assert_eq!(run_backtest(&cfg, &d).unwrap_err(), BacktestError::NoSlots);
        let mut cfg = config(&d, StrategyParams::default());
        cfg.start = cfg.end;
        assert_eq!(
            run_backtest(&cfg, &d).unwrap_err(),
            BacktestError::InvalidRange
        );
        let mut cfg = config(&d, StrategyParams::default());
        cfg.params.asset.max_weight = 5.0;
        assert!(matches!(
            run_backtest(&cfg, &d).unwrap_err(),
            BacktestError::InvalidParams(_)
        ));
    }
}
