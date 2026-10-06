//! Small, allocation-light statistics used by signals, the backtester and analytics.
//!
//! Price series are oldest → newest. Missing observations are `NaN`; every function either
//! ignores them explicitly or returns `None` when the window it needs is incomplete, so a late
//! listing can never leak a partial window into a signal.

pub const TRADING_DAYS_PER_YEAR: f64 = 252.0;

pub fn is_valid(x: f64) -> bool {
    x.is_finite()
}

/// The last `n` values of a slice, or `None` if the slice is shorter or any of them is invalid.
pub fn tail(prices: &[f64], n: usize) -> Option<&[f64]> {
    if n == 0 || prices.len() < n {
        return None;
    }
    let window = &prices[prices.len() - n..];
    window
        .iter()
        .all(|p| is_valid(*p) && *p > 0.0)
        .then_some(window)
}

/// Number of consecutive valid observations ending at the newest one.
pub fn trailing_valid(prices: &[f64]) -> usize {
    prices
        .iter()
        .rev()
        .take_while(|p| is_valid(**p) && **p > 0.0)
        .count()
}

pub fn mean(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    Some(xs.iter().sum::<f64>() / xs.len() as f64)
}

/// Sample standard deviation (n − 1).
pub fn stdev(xs: &[f64]) -> Option<f64> {
    if xs.len() < 2 {
        return None;
    }
    let m = mean(xs)?;
    let var = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (xs.len() - 1) as f64;
    Some(var.sqrt())
}

/// Simple returns of a fully valid window.
pub fn simple_returns(prices: &[f64]) -> Vec<f64> {
    prices.windows(2).map(|w| w[1] / w[0] - 1.0).collect()
}

/// Log returns of a fully valid window.
pub fn log_returns(prices: &[f64]) -> Vec<f64> {
    prices.windows(2).map(|w| (w[1] / w[0]).ln()).collect()
}

/// Annualised volatility of daily log returns over the last `lookback` returns.
pub fn annualized_vol(prices: &[f64], lookback: usize) -> Option<f64> {
    let window = tail(prices, lookback + 1)?;
    let returns = log_returns(window);
    stdev(&returns).map(|s| s * TRADING_DAYS_PER_YEAR.sqrt())
}

/// Simple moving average of the last `n` prices.
pub fn sma(prices: &[f64], n: usize) -> Option<f64> {
    tail(prices, n).and_then(mean)
}

/// Total return from `lookback` observations ago to `skip` observations ago.
/// `momentum(p, 126, 21)` is the classic "6-month return skipping the most recent month".
pub fn momentum(prices: &[f64], lookback: usize, skip: usize) -> Option<f64> {
    if skip >= lookback {
        return None;
    }
    let window = tail(prices, lookback + 1)?;
    let start = window[0];
    let end = window[window.len() - 1 - skip];
    Some(end / start - 1.0)
}

/// Percentile ranks in `[0, 1]` (average rank for ties). `None` inputs stay `None`.
/// A single valid value ranks 0.5 so it receives a neutral tilt.
pub fn percentile_ranks(values: &[Option<f64>]) -> Vec<Option<f64>> {
    let mut indexed: Vec<(usize, f64)> = values
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.filter(|x| x.is_finite()).map(|x| (i, x)))
        .collect();
    let n = indexed.len();
    let mut out = vec![None; values.len()];
    if n == 0 {
        return out;
    }
    if n == 1 {
        out[indexed[0].0] = Some(0.5);
        return out;
    }
    indexed.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && indexed[j + 1].1 == indexed[i].1 {
            j += 1;
        }
        let avg_rank = (i + j) as f64 / 2.0;
        for item in &indexed[i..=j] {
            out[item.0] = Some(avg_rank / (n - 1) as f64);
        }
        i = j + 1;
    }
    out
}

/// Sample covariance of two equal-length series.
pub fn covariance(a: &[f64], b: &[f64]) -> Option<f64> {
    if a.len() != b.len() || a.len() < 2 {
        return None;
    }
    let ma = mean(a)?;
    let mb = mean(b)?;
    let sum: f64 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
    Some(sum / (a.len() - 1) as f64)
}

pub fn correlation(a: &[f64], b: &[f64]) -> Option<f64> {
    let cov = covariance(a, b)?;
    let sa = stdev(a)?;
    let sb = stdev(b)?;
    if sa == 0.0 || sb == 0.0 {
        return None;
    }
    Some(cov / (sa * sb))
}

/// Third standardised moment (sample skewness, adjusted Fisher-Pearson).
pub fn skewness(xs: &[f64]) -> Option<f64> {
    let n = xs.len();
    if n < 3 {
        return None;
    }
    let m = mean(xs)?;
    let s = stdev(xs)?;
    if s == 0.0 {
        return None;
    }
    let nf = n as f64;
    let sum: f64 = xs.iter().map(|x| ((x - m) / s).powi(3)).sum();
    Some(nf / ((nf - 1.0) * (nf - 2.0)) * sum)
}

/// Empirical quantile (linear interpolation, `q` in `[0, 1]`).
pub fn quantile(xs: &[f64], q: f64) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = xs.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let pos = q.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let frac = pos - lo as f64;
    Some(sorted[lo] + (sorted[hi] - sorted[lo]) * frac)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn tail_requires_complete_valid_window() {
        let p = [f64::NAN, 1.0, 2.0, 3.0];
        assert_eq!(tail(&p, 3), Some(&p[1..]));
        assert_eq!(tail(&p, 4), None);
        assert_eq!(tail(&p, 5), None);
        assert_eq!(trailing_valid(&p), 3);
        assert_eq!(trailing_valid(&[1.0, f64::NAN]), 0);
    }

    #[test]
    fn moments_and_stdev() {
        let xs = [1.0, 2.0, 3.0, 4.0];
        assert!(close(mean(&xs).unwrap(), 2.5));
        assert!(close(stdev(&xs).unwrap(), (5.0f64 / 3.0).sqrt()));
        assert_eq!(stdev(&[1.0]), None);
    }

    #[test]
    fn vol_of_constant_growth_is_zero() {
        let p: Vec<f64> = (0..30).map(|i| 100.0 * 1.01f64.powi(i)).collect();
        assert!(annualized_vol(&p, 20).unwrap() < 1e-12);
        assert_eq!(annualized_vol(&p, 40), None);
    }

    #[test]
    fn momentum_skips_recent_observations() {
        let p: Vec<f64> = (1..=11).map(|i| i as f64).collect(); // 1..=11
        // lookback 10 → start = 1; skip 2 → end = 9.
        assert!(close(momentum(&p, 10, 2).unwrap(), 8.0));
        assert_eq!(momentum(&p, 10, 10), None);
        assert_eq!(momentum(&p, 11, 0), None);
    }

    #[test]
    fn sma_uses_last_n() {
        let p = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert!(close(sma(&p, 2).unwrap(), 4.5));
    }

    #[test]
    fn percentile_ranks_handle_ties_and_missing() {
        let r = percentile_ranks(&[Some(3.0), None, Some(1.0), Some(3.0), Some(2.0)]);
        assert_eq!(r[1], None);
        assert!(close(r[2].unwrap(), 0.0));
        assert!(close(r[4].unwrap(), 1.0 / 3.0));
        assert!(close(r[0].unwrap(), 2.5 / 3.0));
        assert_eq!(r[0], r[3]);
        assert_eq!(percentile_ranks(&[Some(7.0)]), vec![Some(0.5)]);
    }

    #[test]
    fn correlation_of_scaled_series_is_one() {
        let a = [1.0, 2.0, 4.0, 3.0];
        let b = [2.0, 4.0, 8.0, 6.0];
        assert!(close(correlation(&a, &b).unwrap(), 1.0));
    }

    #[test]
    fn quantile_interpolates() {
        let xs = [4.0, 1.0, 3.0, 2.0];
        assert!(close(quantile(&xs, 0.0).unwrap(), 1.0));
        assert!(close(quantile(&xs, 0.5).unwrap(), 2.5));
        assert!(close(quantile(&xs, 1.0).unwrap(), 4.0));
    }

    #[test]
    fn skewness_is_zero_for_symmetric_data() {
        assert!(close(skewness(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap(), 0.0));
    }
}
