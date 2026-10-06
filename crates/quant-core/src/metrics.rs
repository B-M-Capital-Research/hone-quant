//! Performance analytics over a daily value series (NAV or a benchmark).
//!
//! Conventions: daily simple returns; annualisation with 252 trading days; Sharpe and Sortino
//! use excess returns over a daily-compounded risk-free rate; drawdowns are fractions ≤ 0;
//! relative statistics (beta, alpha, tracking error, information ratio) require a benchmark
//! series aligned to the same dates.

use chrono::{Datelike, NaiveDate};
use serde::Serialize;

use crate::stats::{self, TRADING_DAYS_PER_YEAR};

/// Below this many daily returns annualised figures are shown but flagged as unreliable.
pub const RELIABLE_ANNUALISATION_DAYS: usize = 63;
/// Below this many daily returns (about one trading month) annualised and regression statistics
/// (CAGR, volatility, Sharpe, Sortino, Calmar, beta, alpha, correlation, tracking error,
/// information ratio) are not computed at all: a few days say nothing about a yearly rate, and
/// a one-day "CAGR" of +130% only misleads.
pub const MIN_ANNUALISATION_DAYS: usize = 21;

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct PerformanceMetrics {
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    pub trading_days: usize,
    pub annualisation_reliable: bool,
    pub total_return: f64,
    pub cagr: Option<f64>,
    pub ann_vol: Option<f64>,
    pub sharpe: Option<f64>,
    pub sortino: Option<f64>,
    pub max_drawdown: f64,
    pub max_drawdown_peak: Option<NaiveDate>,
    pub max_drawdown_trough: Option<NaiveDate>,
    pub max_drawdown_recovery: Option<NaiveDate>,
    /// Longest stretch below a prior peak, in calendar days (ongoing stretches count to the end).
    pub longest_drawdown_days: i64,
    pub calmar: Option<f64>,
    pub best_day: Option<f64>,
    pub worst_day: Option<f64>,
    pub hit_rate: Option<f64>,
    /// 5% one-day historical value-at-risk, as a (negative) return.
    pub var_95: Option<f64>,
    /// Mean of the returns at or below the VaR threshold.
    pub cvar_95: Option<f64>,
    pub skew: Option<f64>,
    pub benchmark_total_return: Option<f64>,
    pub excess_return: Option<f64>,
    pub beta: Option<f64>,
    pub alpha: Option<f64>,
    pub correlation: Option<f64>,
    pub tracking_error: Option<f64>,
    pub information_ratio: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct PeriodReturn {
    pub year: i32,
    /// 1–12 for monthly returns, `None` for calendar-year returns.
    pub month: Option<u32>,
    pub ret: f64,
}

pub fn returns_from_values(values: &[f64]) -> Vec<f64> {
    values
        .windows(2)
        .map(|w| if w[0] > 0.0 { w[1] / w[0] - 1.0 } else { 0.0 })
        .collect()
}

/// Drawdown from the running peak at each point (≤ 0).
pub fn drawdowns(values: &[f64]) -> Vec<f64> {
    let mut peak = f64::MIN;
    values
        .iter()
        .map(|v| {
            peak = peak.max(*v);
            if peak > 0.0 { v / peak - 1.0 } else { 0.0 }
        })
        .collect()
}

fn daily_rf(rf_annual: f64) -> f64 {
    (1.0 + rf_annual).powf(1.0 / TRADING_DAYS_PER_YEAR) - 1.0
}

pub fn compute(
    dates: &[NaiveDate],
    values: &[f64],
    benchmark: Option<&[f64]>,
    rf_annual: f64,
) -> PerformanceMetrics {
    assert_eq!(dates.len(), values.len(), "dates and values must align");
    let mut m = PerformanceMetrics {
        start: dates.first().copied(),
        end: dates.last().copied(),
        trading_days: values.len().saturating_sub(1),
        ..PerformanceMetrics::default()
    };
    if values.len() < 2 || values[0] <= 0.0 {
        return m;
    }
    let returns = returns_from_values(values);
    let first = values[0];
    let last = values[values.len() - 1];
    m.total_return = last / first - 1.0;
    m.annualisation_reliable = returns.len() >= RELIABLE_ANNUALISATION_DAYS;
    let annualise = returns.len() >= MIN_ANNUALISATION_DAYS;

    let days = (dates[dates.len() - 1] - dates[0]).num_days();
    if annualise && days > 0 && last > 0.0 {
        m.cagr = Some((last / first).powf(365.25 / days as f64) - 1.0);
    }
    let rf = daily_rf(rf_annual);
    if annualise {
        let sd = stats::stdev(&returns);
        m.ann_vol = sd.map(|s| s * TRADING_DAYS_PER_YEAR.sqrt());
        let excess: Vec<f64> = returns.iter().map(|r| r - rf).collect();
        let mean_excess = stats::mean(&excess).unwrap_or(0.0);
        if let Some(s) = sd.filter(|s| *s > 0.0) {
            m.sharpe = Some(mean_excess / s * TRADING_DAYS_PER_YEAR.sqrt());
        }
        let downside =
            (excess.iter().map(|x| x.min(0.0).powi(2)).sum::<f64>() / excess.len() as f64).sqrt();
        if downside > 0.0 {
            m.sortino = Some(mean_excess / downside * TRADING_DAYS_PER_YEAR.sqrt());
        }
    }

    // Drawdowns.
    let dd = drawdowns(values);
    let (trough_idx, worst) = dd
        .iter()
        .copied()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("non-empty");
    m.max_drawdown = worst.min(0.0);
    if worst < 0.0 {
        let peak_idx = (0..=trough_idx).rev().find(|&i| dd[i] == 0.0).unwrap_or(0);
        m.max_drawdown_peak = Some(dates[peak_idx]);
        m.max_drawdown_trough = Some(dates[trough_idx]);
        m.max_drawdown_recovery = (trough_idx..values.len())
            .find(|&i| values[i] >= values[peak_idx])
            .map(|i| dates[i]);
    }
    let mut longest = 0i64;
    let mut underwater_since: Option<usize> = None;
    for (i, d) in dd.iter().enumerate() {
        if *d < 0.0 {
            underwater_since.get_or_insert(i.saturating_sub(1));
        } else if let Some(start) = underwater_since.take() {
            longest = longest.max((dates[i] - dates[start]).num_days());
        }
    }
    if let Some(start) = underwater_since {
        longest = longest.max((dates[dates.len() - 1] - dates[start]).num_days());
    }
    m.longest_drawdown_days = longest;
    if let Some(cagr) = m.cagr
        && m.max_drawdown < 0.0
    {
        m.calmar = Some(cagr / m.max_drawdown.abs());
    }

    m.best_day = returns.iter().copied().max_by(|a, b| a.total_cmp(b));
    m.worst_day = returns.iter().copied().min_by(|a, b| a.total_cmp(b));
    m.hit_rate = Some(returns.iter().filter(|r| **r > 0.0).count() as f64 / returns.len() as f64);
    m.var_95 = stats::quantile(&returns, 0.05);
    if let Some(var) = m.var_95 {
        let tail: Vec<f64> = returns.iter().copied().filter(|r| *r <= var).collect();
        m.cvar_95 = stats::mean(&tail);
    }
    m.skew = stats::skewness(&returns);

    if let Some(bench) = benchmark {
        assert_eq!(
            bench.len(),
            values.len(),
            "benchmark must align with values"
        );
        if bench[0] > 0.0 && bench.iter().all(|b| b.is_finite() && *b > 0.0) {
            let bench_total = bench[bench.len() - 1] / bench[0] - 1.0;
            m.benchmark_total_return = Some(bench_total);
            m.excess_return = Some(m.total_return - bench_total);
            if annualise {
                let bench_returns = returns_from_values(bench);
                if let (Some(cov), Some(var)) = (
                    stats::covariance(&returns, &bench_returns),
                    stats::stdev(&bench_returns).map(|s| s * s),
                ) && var > 0.0
                {
                    let beta = cov / var;
                    m.beta = Some(beta);
                    let mean_p = stats::mean(&returns).unwrap_or(0.0);
                    let mean_b = stats::mean(&bench_returns).unwrap_or(0.0);
                    m.alpha = Some(((mean_p - rf) - beta * (mean_b - rf)) * TRADING_DAYS_PER_YEAR);
                }
                m.correlation = stats::correlation(&returns, &bench_returns);
                let active: Vec<f64> = returns
                    .iter()
                    .zip(&bench_returns)
                    .map(|(p, b)| p - b)
                    .collect();
                if let Some(te) = stats::stdev(&active).filter(|s| *s > 0.0) {
                    let te_annual = te * TRADING_DAYS_PER_YEAR.sqrt();
                    m.tracking_error = Some(te_annual);
                    m.information_ratio = Some(
                        stats::mean(&active).unwrap_or(0.0) * TRADING_DAYS_PER_YEAR / te_annual,
                    );
                }
            }
        }
    }
    m
}

/// Compounded returns per calendar month. The first month is measured from the first value.
pub fn monthly_returns(dates: &[NaiveDate], values: &[f64]) -> Vec<PeriodReturn> {
    period_returns(dates, values, |d| (d.year(), Some(d.month())))
}

/// Compounded returns per calendar year. The first year is measured from the first value.
pub fn yearly_returns(dates: &[NaiveDate], values: &[f64]) -> Vec<PeriodReturn> {
    period_returns(dates, values, |d| (d.year(), None))
}

fn period_returns(
    dates: &[NaiveDate],
    values: &[f64],
    key: impl Fn(NaiveDate) -> (i32, Option<u32>),
) -> Vec<PeriodReturn> {
    let mut out = Vec::new();
    if values.len() < 2 {
        return out;
    }
    let mut base = values[0];
    let mut current_key = key(dates[0]);
    for i in 1..values.len() {
        let k = key(dates[i]);
        if k != current_key {
            let end = values[i - 1];
            if base > 0.0 {
                out.push(PeriodReturn {
                    year: current_key.0,
                    month: current_key.1,
                    ret: end / base - 1.0,
                });
            }
            base = end;
            current_key = k;
        }
    }
    let end = values[values.len() - 1];
    if base > 0.0 {
        out.push(PeriodReturn {
            year: current_key.0,
            month: current_key.1,
            ret: end / base - 1.0,
        });
    }
    out
}

/// Rolling annualised volatility over `window` daily returns, aligned to `values`.
pub fn rolling_vol(values: &[f64], window: usize) -> Vec<Option<f64>> {
    let returns = returns_from_values(values);
    let mut out = vec![None; values.len()];
    for end in window..=returns.len() {
        out[end] =
            stats::stdev(&returns[end - window..end]).map(|s| s * TRADING_DAYS_PER_YEAR.sqrt());
    }
    out
}

/// Rolling Sharpe ratio over `window` daily returns, aligned to `values`.
pub fn rolling_sharpe(values: &[f64], window: usize, rf_annual: f64) -> Vec<Option<f64>> {
    let returns = returns_from_values(values);
    let rf = daily_rf(rf_annual);
    let mut out = vec![None; values.len()];
    for end in window..=returns.len() {
        let slice = &returns[end - window..end];
        if let (Some(mean), Some(sd)) = (stats::mean(slice), stats::stdev(slice))
            && sd > 0.0
        {
            out[end] = Some((mean - rf) / sd * TRADING_DAYS_PER_YEAR.sqrt());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn dates(n: usize) -> Vec<NaiveDate> {
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        (0..n).map(|i| start + Duration::days(i as i64)).collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn total_return_cagr_and_drawdown() {
        let values = [100.0, 110.0, 99.0, 121.0];
        let d = dates(4);
        let m = compute(&d, &values, None, 0.0);
        assert!(close(m.total_return, 0.21));
        assert!(close(m.max_drawdown, 99.0 / 110.0 - 1.0));
        assert_eq!(m.max_drawdown_peak, Some(d[1]));
        assert_eq!(m.max_drawdown_trough, Some(d[2]));
        assert_eq!(m.max_drawdown_recovery, Some(d[3]));
        assert_eq!(m.longest_drawdown_days, 2);
        assert!(!m.annualisation_reliable);
        assert!(close(m.hit_rate.unwrap(), 2.0 / 3.0));
    }

    #[test]
    fn annualised_figures_need_a_month_of_returns() {
        let series = |n: usize| -> Vec<f64> {
            (0..n)
                .map(|i| 100.0 * 1.001f64.powi(i as i32) * (1.0 + 0.004 * ((i as f64) * 1.7).sin()))
                .collect()
        };
        // Three returns: total return and drawdown, but no yearly rate, ratio or beta.
        let short = series(4);
        let m = compute(&dates(4), &short, Some(&short), 0.0);
        assert!(m.total_return != 0.0);
        assert!(m.excess_return.is_some());
        for value in [
            m.cagr, m.ann_vol, m.sharpe, m.sortino, m.calmar, m.beta, m.alpha,
        ] {
            assert_eq!(value, None);
        }
        assert_eq!(m.correlation, None);
        // From MIN_ANNUALISATION_DAYS returns on they are computed, flagged until reliable.
        let n = MIN_ANNUALISATION_DAYS + 1;
        let month = series(n);
        let m = compute(&dates(n), &month, Some(&month), 0.0);
        assert!(m.cagr.unwrap() > 0.0);
        assert!(m.ann_vol.is_some() && m.sharpe.is_some() && m.beta.is_some());
        assert!(!m.annualisation_reliable);
    }

    #[test]
    fn ongoing_drawdown_counts_to_the_end() {
        let values = [100.0, 120.0, 90.0, 95.0];
        let m = compute(&dates(4), &values, None, 0.0);
        assert_eq!(m.max_drawdown_recovery, None);
        assert_eq!(m.longest_drawdown_days, 2);
    }

    #[test]
    fn benchmark_against_itself_has_unit_beta_and_no_tracking_error() {
        let values: Vec<f64> = (0..100)
            .map(|i| 100.0 * (1.0 + 0.01 * ((i as f64) * 0.5).sin()) * 1.001f64.powi(i))
            .collect();
        let m = compute(&dates(100), &values, Some(&values), 0.02);
        assert!(close(m.beta.unwrap(), 1.0));
        assert!(m.alpha.unwrap().abs() < 1e-9);
        assert!(close(m.correlation.unwrap(), 1.0));
        assert_eq!(m.tracking_error, None);
        assert!(close(m.excess_return.unwrap(), 0.0));
    }

    #[test]
    fn levered_series_has_beta_two() {
        let bench: Vec<f64> = (0..80)
            .map(|i| 100.0 + ((i as f64) * 0.9).sin() * 3.0 + i as f64 * 0.1)
            .collect();
        let br = returns_from_values(&bench);
        let mut values = vec![100.0];
        for r in &br {
            let last = *values.last().unwrap();
            values.push(last * (1.0 + 2.0 * r));
        }
        let m = compute(&dates(80), &values, Some(&bench), 0.0);
        assert!((m.beta.unwrap() - 2.0).abs() < 1e-9);
        assert!(m.tracking_error.unwrap() > 0.0);
    }

    #[test]
    fn sharpe_positive_for_rising_noisy_series() {
        let values: Vec<f64> = (0..300)
            .map(|i| 100.0 * 1.002f64.powi(i) * (1.0 + 0.005 * ((i as f64) * 1.3).sin()))
            .collect();
        let m = compute(&dates(300), &values, None, 0.0);
        assert!(m.sharpe.unwrap() > 1.0);
        assert!(m.sortino.unwrap() > m.sharpe.unwrap());
        assert!(m.annualisation_reliable);
        // CVaR averages the tail at or beyond the VaR threshold.
        assert!(m.cvar_95.unwrap() <= m.var_95.unwrap() + 1e-12);
    }

    #[test]
    fn monthly_and_yearly_returns_compound() {
        let d = vec![
            NaiveDate::from_ymd_opt(2024, 12, 30).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 2).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 2, 3).unwrap(),
        ];
        let v = [100.0, 110.0, 99.0, 121.0, 133.1];
        let monthly = monthly_returns(&d, &v);
        assert_eq!(monthly.len(), 3);
        assert!(close(monthly[0].ret, 0.10));
        assert!(close(monthly[1].ret, 0.10));
        assert!(close(monthly[2].ret, 0.10));
        let yearly = yearly_returns(&d, &v);
        assert_eq!(yearly.len(), 2);
        assert!(close(yearly[1].ret, 133.1 / 110.0 - 1.0));
        assert_eq!(yearly[1].month, None);
    }

    #[test]
    fn rolling_windows_align_with_values() {
        let values: Vec<f64> = (0..10).map(|i| 100.0 + i as f64).collect();
        let vol = rolling_vol(&values, 5);
        assert_eq!(vol.len(), 10);
        assert!(vol[4].is_none());
        assert!(vol[5].is_some());
        let sharpe = rolling_sharpe(&values, 5, 0.0);
        assert!(sharpe[9].unwrap() > 0.0);
    }

    #[test]
    fn degenerate_inputs_are_safe() {
        let m = compute(&dates(1), &[100.0], None, 0.0);
        assert_eq!(m.trading_days, 0);
        assert_eq!(m.sharpe, None);
        assert!(monthly_returns(&dates(1), &[1.0]).is_empty());
    }
}
