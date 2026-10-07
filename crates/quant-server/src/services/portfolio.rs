//! Paper-account valuation, end-of-day snapshots, corporate actions, live performance, and
//! opening new portfolios.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use deadpool_postgres::GenericClient;
use quant_core::metrics::{self, PerformanceMetrics, PeriodReturn};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::json;

use crate::notify::{self, Event};
use crate::state::{AppState, ServerEvent};
use crate::store::portfolios::{self, Book, NewPortfolio};
use crate::store::settings::{self, BenchmarkSettings};
use crate::store::strategy as strategy_store;
use crate::store::trading::{self, Account, dec, f};
use crate::store::{market, system};

#[derive(Debug, Clone, Serialize)]
pub struct PositionView {
    pub symbol: String,
    pub name_zh: String,
    pub name_en: String,
    pub sector_id: String,
    pub qty: f64,
    pub avg_cost: f64,
    pub price: Option<f64>,
    /// `quote`, `close` or `none`.
    pub price_source: &'static str,
    pub prev_close: Option<f64>,
    pub value: f64,
    pub weight: f64,
    pub cost_basis: f64,
    pub unrealized_pnl: f64,
    pub unrealized_pct: Option<f64>,
    pub day_change_pct: Option<f64>,
    pub day_pnl: Option<f64>,
    pub realized_pnl: f64,
    pub dividends: f64,
    pub in_universe: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Valuation {
    pub account: Account,
    pub as_of: DateTime<Utc>,
    pub cash: f64,
    pub invested: f64,
    pub nav: f64,
    pub exposure: f64,
    pub reference_date: Option<NaiveDate>,
    pub reference_nav: Option<f64>,
    pub day_pnl: Option<f64>,
    pub day_return: Option<f64>,
    pub total_return: f64,
    pub peak_nav: f64,
    pub drawdown: f64,
    pub realized_pnl: f64,
    pub unrealized_pnl: f64,
    pub dividends: f64,
    pub positions: Vec<PositionView>,
}

/// Dates for a new paper account created at `now`: the first session it can trade in (today's
/// if it has not closed yet, otherwise the next one) and the base date of its starting NAV, the
/// session before that. Dating the starting NAV before the first session means day one's return
/// (including the cost of building the portfolio) is measured against the initial cash.
pub fn inception_dates(state: &AppState, now: DateTime<Utc>) -> (NaiveDate, NaiveDate) {
    let first = state.calendar.status_at(now).next_session.date;
    (first, state.calendar.prev_trading_day(first))
}

/// Date of the most recent session that has started (today during/after the session).
pub fn last_session_date(state: &AppState, now: DateTime<Utc>) -> NaiveDate {
    let today = quant_core::calendar::MarketCalendar::local_date(now);
    match state.calendar.session(today) {
        Some(session) if now >= session.open => today,
        _ => state.calendar.prev_trading_day(today),
    }
}

pub async fn valuation(
    state: &AppState,
    client: &impl GenericClient,
    account: &Account,
) -> Result<Valuation> {
    let now = state.now();
    let account = account.clone();
    let positions = trading::positions(client, account.id, false).await?;
    let quotes = market::quotes(client).await?;
    let assets: HashMap<String, market::Asset> = market::assets(client, true)
        .await?
        .into_iter()
        .map(|a| (a.symbol.clone(), a))
        .collect();
    let symbols: Vec<String> = positions.iter().map(|p| p.symbol.clone()).collect();
    let session_date = last_session_date(state, now);
    let closes = market::closes_before(client, &symbols, session_date + Duration::days(1)).await?;
    let prev_closes = market::closes_before(client, &symbols, session_date).await?;
    // Trades during the session, so today's P&L of a position counts shares bought today from
    // their fill price (costs included) rather than from yesterday's close.
    let session_flows = match state.calendar.session(session_date) {
        Some(session) => trading::fill_flows_since(client, account.id, session.open).await?,
        None => HashMap::new(),
    };

    let cash = f(account.cash);
    let mut views = Vec::with_capacity(positions.len());
    for p in &positions {
        let quote = quotes.get(&p.symbol);
        let (price, source) = match (quote, closes.get(&p.symbol)) {
            (Some(q), _) if q.quote.price > 0.0 => (Some(q.quote.price), "quote"),
            (_, Some((_, close))) => (Some(*close), "close"),
            _ => (None, "none"),
        };
        let qty = f(p.qty);
        let avg_cost = f(p.avg_cost);
        let value = price.map(|px| px * qty).unwrap_or(0.0);
        let cost_basis = avg_cost * qty;
        let prev_close = quote
            .and_then(|q| q.quote.prev_close)
            .or_else(|| prev_closes.get(&p.symbol).map(|(_, c)| *c));
        let day_change_pct = match (price, prev_close) {
            (Some(px), Some(pc)) if pc > 0.0 => Some(px / pc - 1.0),
            _ => None,
        };
        let asset = assets.get(&p.symbol);
        views.push(PositionView {
            symbol: p.symbol.clone(),
            name_zh: asset
                .map(|a| a.name_zh.clone())
                .unwrap_or_else(|| p.symbol.clone()),
            name_en: asset
                .map(|a| a.name_en.clone())
                .unwrap_or_else(|| p.symbol.clone()),
            sector_id: asset.map(|a| a.sector_id.clone()).unwrap_or_default(),
            qty,
            avg_cost,
            price,
            price_source: source,
            prev_close,
            value,
            weight: 0.0,
            cost_basis,
            unrealized_pnl: value - cost_basis,
            unrealized_pct: (cost_basis > 0.0).then(|| value / cost_basis - 1.0),
            day_change_pct,
            day_pnl: match (price, prev_close) {
                (Some(px), Some(pc)) => {
                    let flows = session_flows.get(&p.symbol).copied().unwrap_or_default();
                    let prev_qty = qty - flows.bought_qty + flows.sold_qty;
                    Some(px * qty - pc * prev_qty - flows.bought_cash + flows.sold_cash)
                }
                _ => None,
            },
            realized_pnl: f(p.realized_pnl),
            dividends: f(p.dividends),
            in_universe: asset.is_some_and(|a| a.is_active),
        });
    }
    let invested: f64 = views.iter().map(|v| v.value).sum();
    let nav = cash + invested;
    for v in views.iter_mut() {
        v.weight = if nav > 0.0 { v.value / nav } else { 0.0 };
    }
    views.sort_by(|a, b| b.value.total_cmp(&a.value));

    let history = trading::nav_history(client, account.id, None).await?;
    let reference = history.iter().rev().find(|n| n.date < session_date);
    let initial = f(account.initial_cash);
    // Accounts created before this change have their starting NAV dated on the first session
    // itself; their first day is measured against the initial cash too.
    let reference_nav = reference
        .map(|n| f(n.nav))
        .or_else(|| (account.inception_date >= session_date).then_some(initial));
    let peak_nav = history.iter().map(|n| f(n.nav)).fold(nav, f64::max);
    let all_positions = trading::positions(client, account.id, true).await?;
    Ok(Valuation {
        as_of: now,
        cash,
        invested,
        nav,
        exposure: if nav > 0.0 { invested / nav } else { 0.0 },
        reference_date: reference.map(|n| n.date),
        reference_nav,
        day_pnl: reference_nav.map(|r| nav - r),
        day_return: reference_nav.filter(|r| *r > 0.0).map(|r| nav / r - 1.0),
        total_return: if initial > 0.0 {
            nav / initial - 1.0
        } else {
            0.0
        },
        peak_nav,
        drawdown: if peak_nav > 0.0 {
            nav / peak_nav - 1.0
        } else {
            0.0
        },
        realized_pnl: all_positions.iter().map(|p| f(p.realized_pnl)).sum(),
        unrealized_pnl: views.iter().map(|v| v.unrealized_pnl).sum(),
        dividends: all_positions.iter().map(|p| f(p.dividends)).sum(),
        positions: views,
        account,
    })
}

/// Records an account's closing NAV and positions for `date` using that day's closes.
pub async fn snapshot_eod(state: &AppState, account_id: i64, date: NaiveDate) -> Result<f64> {
    let client = state.pool.get().await?;
    let account = trading::account(&client, account_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("account {account_id} not found"))?;
    let positions = trading::positions(&client, account.id, false).await?;
    let symbols: Vec<String> = positions.iter().map(|p| p.symbol.clone()).collect();
    let closes = market::closes_before(&client, &symbols, date + Duration::days(1)).await?;
    let quotes = market::quotes(&client).await?;
    let mut rows = Vec::new();
    let mut invested = Decimal::ZERO;
    for p in &positions {
        // Prefer the official close for `date`; fall back to the latest quote.
        let price = match closes.get(&p.symbol) {
            Some((d, close)) if *d == date => *close,
            _ => quotes
                .get(&p.symbol)
                .map(|q| q.quote.price)
                .or_else(|| closes.get(&p.symbol).map(|(_, c)| *c))
                .unwrap_or(0.0),
        };
        let value = (p.qty * dec(price, 6)).round_dp(2);
        invested += value;
        rows.push((p.symbol.clone(), p.qty, price, value, 0.0));
    }
    let nav = account.cash + invested;
    let nav_f = f(nav);
    for row in rows.iter_mut() {
        row.4 = if nav_f > 0.0 { f(row.3) / nav_f } else { 0.0 };
    }
    trading::upsert_nav(&client, account.id, date, nav, account.cash, invested).await?;
    trading::replace_position_snapshots(&client, account.id, date, &rows).await?;
    Ok(nav_f)
}

/// Applies splits and dividends whose ex-date is `date` (or the previous week, if missed) to
/// every active portfolio. Dividends are credited on the ex-date (a simplification: real brokers
/// pay on the payment date).
pub async fn apply_corporate_actions(state: &Arc<AppState>, date: NaiveDate) -> Result<usize> {
    let client = state.pool.get().await?;
    let books = portfolios::active_books(&client).await?;
    drop(client);
    let mut applied = 0;
    for book in &books {
        applied += apply_corporate_actions_to(state, book, date).await?;
    }
    Ok(applied)
}

async fn apply_corporate_actions_to(
    state: &Arc<AppState>,
    book: &Book,
    date: NaiveDate,
) -> Result<usize> {
    let _guard = state.trading_lock.lock().await;
    let mut client = state.pool.get().await?;
    let Some(account) = trading::account(&client, book.account.id)
        .await?
        .filter(|a| a.status == "active")
    else {
        return Ok(0);
    };
    let from = (date - Duration::days(7)).max(account.inception_date);
    let actions = market::pending_actions(&client, account.id, from, date).await?;
    let mut applied = Vec::new();
    for action in actions {
        let tx = client.transaction().await?;
        trading::lock_account(&tx, account.id).await?;
        let Some(mut position) =
            trading::position_for_update(&tx, account.id, &action.symbol).await?
        else {
            continue;
        };
        if position.qty <= Decimal::ZERO {
            continue;
        }
        let qty_before = position.qty;
        let mut cash_amount = Decimal::ZERO;
        let detail = if action.kind == "split" {
            let ratio = action.ratio.unwrap_or(1.0);
            if !(ratio > 0.0) || (ratio - 1.0).abs() < 1e-9 {
                continue;
            }
            let ratio_d = dec(ratio, 8);
            position.qty = (position.qty * ratio_d).round_dp(6);
            position.avg_cost = (position.avg_cost / ratio_d).round_dp(6);
            format!(
                "{} {}-for-1: {} → {} shares",
                action.ex_date, ratio, qty_before, position.qty
            )
        } else {
            let amount = action.amount.unwrap_or(0.0);
            if !(amount > 0.0) {
                continue;
            }
            cash_amount = (position.qty * dec(amount, 6)).round_dp(2);
            position.dividends += cash_amount;
            trading::post_cash(
                &tx,
                account.id,
                "dividend",
                cash_amount,
                Some(&action.symbol),
                Some("corporate_action"),
                Some(action.id.to_string()),
                &format!("{} × ${amount}", position.qty),
            )
            .await?;
            format!(
                "{} ${amount}/share × {} = ${}",
                action.ex_date, position.qty, cash_amount
            )
        };
        trading::save_position(&tx, account.id, &position).await?;
        tx.execute(
            "INSERT INTO corporate_action_applications (account_id, action_id, qty_before, qty_after, cash_amount) VALUES ($1, $2, $3, $4, $5)",
            &[&account.id, &action.id, &qty_before, &position.qty, &cash_amount],
        )
        .await?;
        system::audit(
            &tx,
            "system",
            &format!("corporate_action.{}", action.kind),
            "position",
            &action.symbol,
            json!({"action_id": action.id, "detail": detail}),
            "",
        )
        .await?;
        tx.commit().await?;
        applied.push((action.symbol.clone(), action.kind.clone(), detail));
    }
    drop(client);
    let tag = book.portfolio.tag();
    for (symbol, kind, detail) in &applied {
        let _ = notify::notify_in(
            state,
            &tag,
            Event::CorporateAction {
                symbol: symbol.clone(),
                kind: kind.clone(),
                detail: detail.clone(),
            },
        )
        .await;
    }
    if !applied.is_empty() {
        state.emit(ServerEvent::Account {
            reason: "corporate_action".into(),
            portfolio_id: account.portfolio_id,
        });
    }
    Ok(applied.len())
}

// ---------------------------------------------------------------------------------------------
// Opening portfolios
// ---------------------------------------------------------------------------------------------

pub struct OpenPortfolio<'a> {
    pub portfolio: NewPortfolio<'a>,
    pub initial_cash: Decimal,
    pub strategy_version_id: i64,
}

/// Creates a portfolio with its first paper account (starting NAV dated the session before its
/// first one, see [`inception_dates`]) and activates its strategy version, inside the caller's
/// transaction. `None` when the owner already has an active portfolio with that name.
pub async fn open(
    state: &AppState,
    tx: &impl GenericClient,
    request: &OpenPortfolio<'_>,
) -> Result<Option<Book>> {
    let Some(portfolio) = portfolios::insert(tx, &request.portfolio).await? else {
        return Ok(None);
    };
    let (first_session, base_date) = inception_dates(state, state.now());
    let cash = request.initial_cash;
    let account = trading::create_account(tx, portfolio.id, "Paper", cash, first_session).await?;
    trading::upsert_nav(tx, account.id, base_date, cash, cash, Decimal::ZERO).await?;
    strategy_store::activate(
        tx,
        account.id,
        request.strategy_version_id,
        request.portfolio.created_by,
        "initial activation",
    )
    .await?;
    system::audit(
        tx,
        request.portfolio.created_by,
        "portfolio.created",
        "portfolio",
        &portfolio.id.to_string(),
        json!({
            "name": portfolio.name,
            "owner": portfolio.owner,
            "initial_cash": cash,
            "account_id": account.id,
            "strategy_version_id": request.strategy_version_id,
            "automation": portfolio.automation,
        }),
        "",
    )
    .await?;
    Ok(Some(Book { portfolio, account }))
}

/// Headline figures of a portfolio, for lists.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub nav: f64,
    pub cash: f64,
    pub invested: f64,
    pub total_return: f64,
    pub day_return: Option<f64>,
    pub positions: usize,
    pub initial_cash: f64,
    pub inception_date: NaiveDate,
    pub strategy: Option<serde_json::Value>,
}

pub async fn summary(
    state: &AppState,
    client: &impl GenericClient,
    book: &Book,
) -> Result<Summary> {
    let valuation = valuation(state, client, &book.account).await?;
    let strategy = strategy_store::active_version(client, book.account.id)
        .await?
        .map(|v| json!({"id": v.id, "name": v.name, "preset_id": v.preset_id}));
    Ok(Summary {
        nav: valuation.nav,
        cash: valuation.cash,
        invested: valuation.invested,
        total_return: valuation.total_return,
        day_return: valuation.day_return,
        positions: valuation.positions.len(),
        initial_cash: f(book.account.initial_cash),
        inception_date: book.account.inception_date,
        strategy,
    })
}

// ---------------------------------------------------------------------------------------------
// Performance
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SeriesPoint {
    pub date: NaiveDate,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkSeries {
    pub symbol: String,
    pub values: Vec<Option<f64>>,
    pub metrics: Option<PerformanceMetrics>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TradeStats {
    pub fills: usize,
    pub buys: usize,
    pub sells: usize,
    pub bought: f64,
    pub sold: f64,
    pub commissions: f64,
    pub fees: f64,
    pub slippage: f64,
    pub realized_pnl: f64,
    pub winning_sells: usize,
    pub losing_sells: usize,
    /// The lesser of purchases and sales over average NAV, annualised (the fund-turnover
    /// convention, so building or winding down the portfolio is not counted as turnover).
    /// `None` until the window spans at least a month of sessions.
    pub annual_turnover: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Attribution {
    pub key: String,
    pub sector: Option<String>,
    pub contribution: f64,
    pub avg_weight: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Performance {
    pub dates: Vec<NaiveDate>,
    pub nav: Vec<f64>,
    pub benchmarks: Vec<BenchmarkSeries>,
    pub primary_benchmark: String,
    pub metrics: PerformanceMetrics,
    pub drawdowns: Vec<f64>,
    pub monthly_returns: Vec<PeriodReturn>,
    pub yearly_returns: Vec<PeriodReturn>,
    pub rolling_vol: Vec<Option<f64>>,
    pub asset_attribution: Vec<Attribution>,
    pub sector_attribution: Vec<Attribution>,
    pub trades: TradeStats,
    pub inception_date: NaiveDate,
}

pub async fn performance(
    state: &AppState,
    account: &Account,
    from: Option<NaiveDate>,
) -> Result<Performance> {
    let client = state.pool.get().await?;
    let bench_settings: BenchmarkSettings = settings::get(&client, settings::BENCHMARKS).await?;
    let mut history = trading::nav_history(&client, account.id, from).await?;
    // Live point for the current session when it is not yet snapshotted.
    let live = valuation(state, &client, account).await?;
    let session_date = last_session_date(state, state.now());
    if history.last().is_none_or(|n| n.date < session_date) {
        history.push(trading::NavPoint {
            date: session_date,
            nav: dec(live.nav, 2),
            cash: dec(live.cash, 2),
            invested: dec(live.invested, 2),
            flows: Decimal::ZERO,
        });
    }
    let dates: Vec<NaiveDate> = history.iter().map(|n| n.date).collect();
    let nav: Vec<f64> = history.iter().map(|n| f(n.nav)).collect();

    // Benchmarks aligned to the NAV dates (carry the last close forward).
    let mut benchmarks = Vec::new();
    if let Some(first) = dates.first() {
        let panel = market::adjusted_panel(
            &client,
            &bench_settings.symbols,
            *first - Duration::days(10),
            None,
        )
        .await?;
        for symbol in &bench_settings.symbols {
            let series = panel.get(symbol).cloned().unwrap_or_default();
            let mut values = Vec::with_capacity(dates.len());
            let mut idx = 0;
            let mut last: Option<f64> = None;
            let mut base: Option<f64> = None;
            for d in &dates {
                while idx < series.len() && series[idx].0 <= *d {
                    last = Some(series[idx].2);
                    idx += 1;
                }
                if base.is_none() {
                    base = last;
                }
                values.push(match (last, base) {
                    (Some(l), Some(b)) if b > 0.0 => Some(nav[0] * l / b),
                    _ => None,
                });
            }
            let complete: Option<Vec<f64>> = values.iter().copied().collect();
            let m = complete
                .as_ref()
                .map(|v| metrics::compute(&dates, v, None, bench_settings.risk_free_rate));
            benchmarks.push(BenchmarkSeries {
                symbol: symbol.clone(),
                values,
                metrics: m,
            });
        }
    }
    let primary: Option<Vec<f64>> = benchmarks
        .iter()
        .find(|b| b.symbol == bench_settings.primary)
        .and_then(|b| b.values.iter().copied().collect());
    let metrics = metrics::compute(
        &dates,
        &nav,
        primary.as_deref(),
        bench_settings.risk_free_rate,
    );

    // Attribution from consecutive closing snapshots.
    let snapshots = trading::position_snapshots(&client, account.id, from).await?;
    let mut by_date: BTreeMap<NaiveDate, Vec<&trading::PositionSnapshot>> = BTreeMap::new();
    for s in &snapshots {
        by_date.entry(s.date).or_default().push(s);
    }
    let sector_of: HashMap<String, String> = market::assets(&client, true)
        .await?
        .into_iter()
        .map(|a| (a.symbol, a.sector_id))
        .collect();
    let mut contribution: BTreeMap<String, (f64, f64, usize)> = BTreeMap::new();
    let snapshot_dates: Vec<&NaiveDate> = by_date.keys().collect();
    for pair in snapshot_dates.windows(2) {
        let prev = &by_date[pair[0]];
        let cur: HashMap<&str, f64> = by_date[pair[1]]
            .iter()
            .map(|s| (s.symbol.as_str(), s.price))
            .collect();
        for s in prev {
            let entry = contribution
                .entry(s.symbol.clone())
                .or_insert((0.0, 0.0, 0));
            if let Some(price) = cur.get(s.symbol.as_str())
                && s.price > 0.0
            {
                entry.0 += s.weight * (price / s.price - 1.0);
            }
            entry.1 += s.weight;
            entry.2 += 1;
        }
    }
    let periods = snapshot_dates.len().saturating_sub(1).max(1) as f64;
    let mut asset_attribution: Vec<Attribution> = contribution
        .into_iter()
        .map(|(symbol, (c, w, _))| Attribution {
            sector: sector_of.get(&symbol).cloned(),
            key: symbol,
            contribution: c,
            avg_weight: w / periods,
        })
        .collect();
    asset_attribution.sort_by(|a, b| b.contribution.total_cmp(&a.contribution));
    let mut sectors: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for a in &asset_attribution {
        let e = sectors
            .entry(a.sector.clone().unwrap_or_default())
            .or_insert((0.0, 0.0));
        e.0 += a.contribution;
        e.1 += a.avg_weight;
    }
    let mut sector_attribution: Vec<Attribution> = sectors
        .into_iter()
        .map(|(key, (contribution, avg_weight))| Attribution {
            key,
            sector: None,
            contribution,
            avg_weight,
        })
        .collect();
    sector_attribution.sort_by(|a, b| b.contribution.total_cmp(&a.contribution));

    // Trade statistics.
    let (fills, _) = trading::list_fills(
        &client,
        account.id,
        &trading::FillFilter {
            symbol: None,
            from: from.map(|d| d.and_hms_opt(0, 0, 0).expect("midnight").and_utc()),
            to: None,
            plan_id: None,
            limit: 100_000,
            offset: 0,
        },
    )
    .await?;
    let mut stats = TradeStats {
        fills: fills.len(),
        ..TradeStats::default()
    };
    for fill in &fills {
        if fill.side == "buy" {
            stats.buys += 1;
            stats.bought += f(fill.notional);
        } else {
            stats.sells += 1;
            stats.sold += f(fill.notional);
            if let Some(pnl) = fill.realized_pnl {
                let pnl = f(pnl);
                stats.realized_pnl += pnl;
                if pnl > 0.0 {
                    stats.winning_sells += 1;
                } else if pnl < 0.0 {
                    stats.losing_sells += 1;
                }
            }
        }
        stats.commissions += f(fill.commission);
        stats.fees += f(fill.fees);
        stats.slippage += f(fill.slippage);
    }
    const MIN_SESSIONS_FOR_ANNUAL_TURNOVER: usize = 21;
    if let (Some(first), Some(last)) = (dates.first(), dates.last())
        && dates.len() >= MIN_SESSIONS_FOR_ANNUAL_TURNOVER
    {
        let years = (*last - *first).num_days() as f64 / 365.25;
        let avg_nav = nav.iter().sum::<f64>() / nav.len() as f64;
        if avg_nav > 0.0 && years > 0.0 {
            stats.annual_turnover = Some(stats.bought.min(stats.sold) / avg_nav / years);
        }
    }

    Ok(Performance {
        drawdowns: metrics::drawdowns(&nav),
        monthly_returns: metrics::monthly_returns(&dates, &nav),
        yearly_returns: metrics::yearly_returns(&dates, &nav),
        rolling_vol: metrics::rolling_vol(&nav, 21),
        dates,
        nav,
        benchmarks,
        primary_benchmark: bench_settings.primary,
        metrics,
        asset_attribution,
        sector_attribution,
        trades: stats,
        inception_date: account.inception_date,
    })
}
