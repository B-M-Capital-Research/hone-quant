//! Market data synchronisation: daily history, quotes, corporate actions and cached intraday
//! bars. All provider calls go through the `MarketData` trait, so FMP and the demo source share
//! this code.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use futures::StreamExt;
use serde::Serialize;

use crate::market::{Interval, IntradayBar, MarketError};
use crate::state::{AppState, ServerEvent};
use crate::store::{market, trading};
use crate::universe;

/// Years of daily history kept for signals and backtests.
pub fn history_years() -> i64 {
    std::env::var("HONE_QUANT_HISTORY_YEARS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10i64)
        .clamp(2, 25)
}

/// Universe members (active), benchmarks and anything any portfolio holds, deduplicated.
pub async fn tracked_symbols(state: &AppState) -> Result<Vec<String>> {
    let client = state.pool.get().await?;
    let mut symbols: BTreeSet<String> = market::assets(&client, false)
        .await?
        .into_iter()
        .map(|a| a.symbol)
        .collect();
    let bench: crate::store::settings::BenchmarkSettings =
        crate::store::settings::get(&client, crate::store::settings::BENCHMARKS).await?;
    symbols.extend(bench.symbols);
    symbols.extend(universe::bundled().benchmarks.into_iter().map(|b| b.symbol));
    symbols.extend(trading::held_symbols(&client).await?);
    Ok(symbols.into_iter().collect())
}

#[derive(Debug, Default, Serialize)]
pub struct SyncReport {
    pub symbols: usize,
    pub bars: u64,
    pub failed: BTreeMap<String, String>,
}

/// Fetches daily bars so every tracked symbol is covered from `history_years()` ago (or its
/// listing) to today. Existing history is only topped up (re-fetching the last `overlap_days`
/// so late corrections land), unless `full` is set.
pub async fn sync_daily(state: &AppState, overlap_days: i64, full: bool) -> Result<SyncReport> {
    let today = state.now().date_naive();
    let symbols = tracked_symbols(state).await?;
    let last = {
        let client = state.pool.get().await?;
        market::last_bar_dates(&client).await?
    };
    let earliest = today - Duration::days(365 * history_years());
    let jobs: Vec<(String, NaiveDate)> = symbols
        .iter()
        .map(|s| {
            let from = match last.get(s) {
                Some(d) if !full => (*d - Duration::days(overlap_days)).max(earliest),
                _ => earliest,
            };
            (s.clone(), from)
        })
        .collect();
    let mut report = SyncReport {
        symbols: jobs.len(),
        ..SyncReport::default()
    };
    let results: Vec<(String, Result<u64>)> = futures::stream::iter(jobs)
        .map(|(symbol, from)| async move {
            let result = async {
                let bars = state.market.daily_bars(&symbol, from, today).await?;
                let client = state.pool.get().await?;
                market::upsert_daily_bars(&client, &bars).await
            }
            .await;
            (symbol, result)
        })
        .buffer_unordered(4)
        .collect()
        .await;
    for (symbol, result) in results {
        match result {
            Ok(n) => report.bars += n,
            Err(error) => {
                tracing::warn!(%symbol, error = %format!("{error:#}"), "daily sync failed");
                report.failed.insert(symbol, format!("{error:#}"));
            }
        }
    }
    Ok(report)
}

/// Fetches and stores quotes for every tracked symbol; returns how many arrived.
pub async fn poll_quotes(state: &AppState) -> Result<usize> {
    let symbols = tracked_symbols(state).await?;
    let quotes = state.market.quotes(&symbols).await?;
    let client = state.pool.get().await?;
    market::upsert_quotes(&client, &quotes).await?;
    state.emit(ServerEvent::Quotes { at: state.now() });
    Ok(quotes.len())
}

/// Fresh quotes for specific symbols (plan generation and execution), persisted as a side
/// effect. Falls back to stored quotes when the provider fails.
pub async fn fresh_quotes(
    state: &AppState,
    symbols: &[String],
) -> Result<BTreeMap<String, (f64, Option<DateTime<Utc>>, DateTime<Utc>)>> {
    let mut out = BTreeMap::new();
    match state.market.quotes(symbols).await {
        Ok(quotes) => {
            let client = state.pool.get().await?;
            market::upsert_quotes(&client, &quotes).await?;
            let now = state.now();
            for q in quotes {
                out.insert(q.symbol.clone(), (q.price, q.timestamp, now));
            }
        }
        Err(error) => tracing::warn!(%error, "live quotes unavailable; using stored quotes"),
    }
    let missing: Vec<&String> = symbols.iter().filter(|s| !out.contains_key(*s)).collect();
    if !missing.is_empty() {
        let client = state.pool.get().await?;
        let stored = market::quotes(&client).await?;
        for symbol in missing {
            if let Some(q) = stored.get(symbol) {
                out.insert(
                    symbol.clone(),
                    (q.quote.price, q.quote.timestamp, q.fetched_at),
                );
            }
        }
    }
    Ok(out)
}

/// Refreshes split and dividend history for symbols any portfolio holds (and those passed in).
pub async fn sync_corporate_actions(state: &AppState, extra: &[String]) -> Result<usize> {
    let mut symbols: BTreeSet<String> = extra.iter().cloned().collect();
    {
        let client = state.pool.get().await?;
        symbols.extend(trading::held_symbols(&client).await?);
    }
    let mut count = 0;
    for symbol in symbols {
        let splits = match state.market.splits(&symbol).await {
            Ok(s) => s,
            Err(MarketError::PlanRestricted(_)) => Vec::new(),
            Err(error) => {
                tracing::warn!(%symbol, %error, "split history unavailable");
                Vec::new()
            }
        };
        let dividends = match state.market.dividends(&symbol).await {
            Ok(d) => d,
            Err(MarketError::PlanRestricted(_)) => Vec::new(),
            Err(error) => {
                tracing::warn!(%symbol, %error, "dividend history unavailable");
                Vec::new()
            }
        };
        let client = state.pool.get().await?;
        market::upsert_splits(&client, &splits).await?;
        market::upsert_dividends(&client, &dividends).await?;
        count += splits.len() + dividends.len();
    }
    Ok(count)
}

/// Intraday bars for the last `days` sessions, cached in the database for a few minutes.
pub async fn intraday(
    state: &Arc<AppState>,
    symbol: &str,
    interval: Interval,
    days: i64,
) -> Result<Vec<IntradayBar>> {
    let now = state.now();
    let today = now.date_naive();
    let mut from = today;
    for _ in 1..days.max(1) {
        from = state.calendar.prev_trading_day(from);
    }
    if !state.calendar.is_trading_day(from) {
        from = state.calendar.prev_trading_day(from);
    }
    let from_ts = state
        .calendar
        .session(from)
        .map(|s| s.open)
        .unwrap_or_else(|| now - Duration::days(days + 3));
    let client = state.pool.get().await?;
    let (cached, fetched_at) =
        market::intraday(&client, symbol, interval.as_str(), from_ts).await?;
    let fresh_enough = fetched_at.is_some_and(|t| now - t < Duration::minutes(3));
    if fresh_enough && !cached.is_empty() {
        return Ok(cached);
    }
    drop(client);
    match state
        .market
        .intraday_bars(symbol, interval, from, today)
        .await
    {
        Ok(bars) => {
            let client = state.pool.get().await?;
            market::replace_intraday(&client, symbol, interval.as_str(), &bars).await?;
            Ok(bars.into_iter().filter(|b| b.ts >= from_ts).collect())
        }
        Err(error) if !cached.is_empty() => {
            tracing::warn!(%symbol, %error, "intraday refresh failed; serving cache");
            Ok(cached)
        }
        Err(error) => Err(error.into()),
    }
}
