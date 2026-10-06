//! Backtests run in the background on the same engine, rebalancer and cost model as live
//! paper trading, over adjusted daily history stored in PostgreSQL.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

use anyhow::{Result, anyhow, bail};
use chrono::{Duration, NaiveDate};
use quant_core::backtest::{BacktestConfig, BacktestData, run_backtest};
use quant_core::strategy::AssetMeta;
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use crate::state::{AppState, ServerEvent};
use crate::store::settings::{self, BenchmarkSettings};
use crate::store::{market, system};

/// Backtests are CPU-bound; run one at a time so live trading never competes for cores.
fn permits() -> &'static Semaphore {
    static PERMITS: OnceLock<Semaphore> = OnceLock::new();
    PERMITS.get_or_init(|| Semaphore::new(1))
}

/// Trades kept in the stored result (the most recent ones); the total is always reported.
const MAX_STORED_TRADES: usize = 5_000;

pub async fn submit(
    state: &Arc<AppState>,
    name: &str,
    mut config: BacktestConfig,
    strategy_version_id: Option<i64>,
    actor: &str,
) -> Result<system::BacktestRow> {
    if let Err(errors) = config.params.validate() {
        bail!("invalid strategy parameters: {}", errors[0].message);
    }
    config.costs.validate().map_err(|e| anyhow!(e))?;
    if config.start >= config.end {
        bail!("the start date must be before the end date");
    }
    if (config.end - config.start).num_days() > 366 * 25 {
        bail!("backtests are limited to 25 years");
    }
    let client = state.pool.get().await?;
    let bench: BenchmarkSettings = settings::get(&client, settings::BENCHMARKS).await?;
    if config.benchmark.is_none() {
        config.benchmark = Some(bench.primary.clone());
    }
    let row = system::insert_backtest(
        &client,
        name,
        &serde_json::to_value(&config)?,
        strategy_version_id,
        actor,
    )
    .await?;
    system::audit(
        &client,
        actor,
        "backtest.submitted",
        "backtest",
        &row.id.to_string(),
        json!({"name": name}),
        "",
    )
    .await?;
    drop(client);
    let id = row.id;
    let state2 = state.clone();
    tokio::spawn(async move {
        let _permit = permits().acquire().await.expect("semaphore open");
        if let Err(error) = run(&state2, id, config, bench).await {
            tracing::warn!(backtest = id, error = %format!("{error:#}"), "backtest failed");
            if let Ok(client) = state2.pool.get().await {
                let _ =
                    system::finish_backtest(&client, id, None, None, Some(&format!("{error:#}")))
                        .await;
            }
            state2.emit(ServerEvent::Backtest {
                id,
                status: "failed".into(),
            });
        }
    });
    Ok(row)
}

async fn run(
    state: &Arc<AppState>,
    id: i64,
    config: BacktestConfig,
    bench: BenchmarkSettings,
) -> Result<()> {
    let client = state.pool.get().await?;
    system::start_backtest(&client, id).await?;
    state.emit(ServerEvent::Backtest {
        id,
        status: "running".into(),
    });
    let assets = market::assets(&client, false).await?;
    let warmup = Duration::days((config.params.max_lookback() as f64 * 1.6) as i64 + 30);
    let from = config.start - warmup;
    let mut symbols: Vec<String> = assets.iter().map(|a| a.symbol.clone()).collect();
    let bench_symbols = bench.symbols.clone();
    symbols.extend(bench_symbols.iter().cloned());
    let panel = market::adjusted_panel(&client, &symbols, from, Some(config.end)).await?;
    drop(client);

    // Trading dates = dates on which any universe member traded.
    let dates: Vec<NaiveDate> = assets
        .iter()
        .filter_map(|a| panel.get(&a.symbol))
        .flatten()
        .map(|(d, _, _)| *d)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if dates.is_empty() {
        bail!("no price history is stored for this period; wait for the history sync to finish");
    }
    let index: HashMap<NaiveDate, usize> = dates.iter().enumerate().map(|(i, d)| (*d, i)).collect();
    let column = |symbol: &str, pick_open: bool| -> Vec<f64> {
        let mut out = vec![f64::NAN; dates.len()];
        if let Some(series) = panel.get(symbol) {
            for (d, open, close) in series {
                if let Some(&i) = index.get(d) {
                    out[i] = if pick_open { *open } else { *close };
                }
            }
        }
        out
    };
    let data = BacktestData {
        assets: assets
            .iter()
            .map(|a| AssetMeta {
                symbol: a.symbol.clone(),
                sector: a.sector_id.clone(),
            })
            .collect(),
        open: assets.iter().map(|a| column(&a.symbol, true)).collect(),
        close: assets.iter().map(|a| column(&a.symbol, false)).collect(),
        benchmarks: bench_symbols
            .iter()
            .map(|s| (s.clone(), column(s, false)))
            .collect(),
        dates,
    };
    let started = std::time::Instant::now();
    let result = tokio::task::spawn_blocking(move || run_backtest(&config, &data)).await??;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let total_trades = result.trades.len();
    let mut value = serde_json::to_value(&result)?;
    if let Some(trades) = value.get_mut("trades").and_then(Value::as_array_mut)
        && trades.len() > MAX_STORED_TRADES
    {
        let drop_n = trades.len() - MAX_STORED_TRADES;
        trades.drain(..drop_n);
    }
    value["total_trades"] = json!(total_trades);
    value["elapsed_ms"] = json!(elapsed_ms);
    let summary = json!({
        "metrics": result.metrics,
        "final_nav": result.points.last().map(|p| p.nav),
        "total_costs": result.total_costs,
        "annual_turnover": result.annual_turnover,
        "trades": total_trades,
        "benchmarks": result.benchmarks.iter().map(|b| json!({"symbol": b.symbol, "total_return": b.metrics.total_return, "cagr": b.metrics.cagr, "max_drawdown": b.metrics.max_drawdown, "sharpe": b.metrics.sharpe})).collect::<Vec<_>>(),
        "elapsed_ms": elapsed_ms,
    });
    let client = state.pool.get().await?;
    system::finish_backtest(&client, id, Some(&summary), Some(&value), None).await?;
    state.emit(ServerEvent::Backtest {
        id,
        status: "succeeded".into(),
    });
    tracing::info!(
        backtest = id,
        elapsed_ms,
        trades = total_trades,
        "backtest finished"
    );
    Ok(())
}
