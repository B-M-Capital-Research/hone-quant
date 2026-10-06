//! Read models for the main screen: market status and today's schedule, the cross-sectional
//! candle board (every asset's move over a period on one chart), per-asset candles with trade
//! markers, quotes, and the combined dashboard payload.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use quant_core::calendar::{Holiday, MarketCalendar, MarketPhase, Session};
use quant_core::schedule::day_schedule;
use quant_core::stats;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::error::{ApiError, ApiResult};
use crate::auth::CurrentUser;
use crate::market::{DailyBar, Interval};
use crate::services::{marketdata, portfolio};
use crate::state::{AppState, SharedState};
use crate::store::settings::{self, AutomationSettings, BenchmarkSettings, DisplaySettings};
use crate::store::strategy::{self as strategy_store, SkippedSlot};
use crate::store::trading::{self, Order, Plan};
use crate::store::{market, system};
use crate::universe;

pub async fn health(State(state): State<SharedState>) -> Json<Value> {
    let db = crate::db::health(&state.pool).await;
    Json(
        json!({"ok": db, "db": db, "version": env!("CARGO_PKG_VERSION"), "revision": crate::REVISION}),
    )
}

pub async fn meta(State(state): State<SharedState>) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let has_users = system::user_count(&client).await? > 0;
    let display: DisplaySettings = settings::get(&client, settings::DISPLAY).await?;
    Ok(Json(json!({
        "app": "hone-quant",
        "version": env!("CARGO_PKG_VERSION"),
        "revision": crate::REVISION,
        "data_source": state.market.source(),
        "demo": state.market.source() == crate::market::DataSource::Demo,
        "server_time": state.now(),
        "market_timezone": "America/New_York",
        "display": display,
        "has_users": has_users,
        "paper_only": true,
        "auth": super::auth_routes::auth_info(&state),
        "base_path": state.config.base_path,
    })))
}

#[derive(Serialize)]
pub struct HolidayView {
    id: Holiday,
    name_zh: &'static str,
    name_en: &'static str,
}

#[derive(Serialize)]
pub struct SlotView {
    slot: &'static str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    generate_at: DateTime<Utc>,
    execute_at: DateTime<Utc>,
    execute_deadline: DateTime<Utc>,
    plan: Option<Plan>,
    cancelled: Option<SkippedSlot>,
}

#[derive(Serialize)]
pub struct MarketView {
    now: DateTime<Utc>,
    phase: MarketPhase,
    today: Option<Session>,
    next_session: Session,
    holiday: Option<HolidayView>,
    /// The session whose schedule is shown (today if it trades, else the next session).
    schedule_date: NaiveDate,
    schedule: Vec<SlotView>,
    early_close: bool,
    automation: AutomationSettings,
    effective_mode: &'static str,
}

pub async fn market_view(state: &AppState) -> anyhow::Result<MarketView> {
    let now = state.now();
    let status = state.calendar.status_at(now);
    let client = state.pool.get().await?;
    let schedule_settings = settings::schedule(&client).await?;
    let automation: AutomationSettings = settings::get(&client, settings::AUTOMATION).await?;
    let session = status
        .today
        .filter(|s| now < s.close + Duration::hours(8))
        .unwrap_or(status.next_session);
    let account = trading::active_account(&client).await?;
    let plans = match &account {
        Some(a) => trading::plans_for_date(&client, a.id, session.date).await?,
        None => Vec::new(),
    };
    let skipped = strategy_store::skipped_slots(&client, session.date, session.date).await?;
    let schedule = day_schedule(&session, &schedule_settings)
        .into_iter()
        .map(|slot| {
            let name = slot.slot.as_str();
            SlotView {
                slot: name,
                window_start: slot.window_start,
                window_end: slot.window_end,
                generate_at: slot.generate_at,
                execute_at: slot.execute_at,
                execute_deadline: slot.execute_deadline,
                plan: plans.iter().rev().find(|p| p.slot == name).cloned(),
                cancelled: skipped.iter().find(|s| s.slot == name).cloned(),
            }
        })
        .collect();
    Ok(MarketView {
        now,
        phase: status.phase,
        today: status.today,
        next_session: status.next_session,
        holiday: status.holiday.map(|h| HolidayView {
            id: h,
            name_zh: h.name_zh(),
            name_en: h.name_en(),
        }),
        schedule_date: session.date,
        early_close: session.early_close,
        schedule,
        effective_mode: automation.effective_mode(now).as_str(),
        automation,
    })
}

pub async fn market(
    State(state): State<SharedState>,
    _user: CurrentUser,
) -> ApiResult<Json<MarketView>> {
    Ok(Json(market_view(&state).await?))
}

#[derive(Serialize)]
pub struct PlanWithOrders {
    #[serde(flatten)]
    plan: Plan,
    orders: Vec<Order>,
}

pub async fn dashboard(
    State(state): State<SharedState>,
    _user: CurrentUser,
) -> ApiResult<Json<Value>> {
    let market = market_view(&state).await?;
    let client = state.pool.get().await?;
    let valuation = portfolio::valuation(&state, &client).await?;
    let account_id = valuation.account.id;
    let version = strategy_store::active_version(&client, account_id).await?;
    let targets = trading::latest_targets(&client, account_id).await?;
    let mut plans = Vec::new();
    for p in trading::plans_for_date(&client, account_id, market.schedule_date).await? {
        let orders = trading::orders_for_plan(&client, p.id).await?;
        plans.push(PlanWithOrders { plan: p, orders });
    }
    let (recent, _) = trading::list_plans(
        &client,
        account_id,
        &trading::PlanFilter {
            from: None,
            to: None,
            status: None,
            limit: 6,
            offset: 0,
        },
    )
    .await?;
    let unread = system::unread_count(&client).await?;
    let quotes = market::quotes(&client).await?;
    let restrictions = strategy_store::active_restrictions(&client, market.schedule_date).await?;
    let last_quote = quotes.values().map(|q| q.fetched_at).max();
    Ok(Json(json!({
        "market": market,
        "valuation": valuation,
        "strategy": version.map(|v| json!({"id": v.id, "name": v.name, "preset_id": v.preset_id, "params": v.params, "created_at": v.created_at})),
        "targets": targets.map(|(plan_id, at, weights)| json!({"plan_id": plan_id, "generated_at": at, "weights": weights})),
        "plans": plans,
        "recent_plans": recent,
        "restrictions": restrictions,
        "unread_notifications": unread,
        "data": {"source": state.market.source(), "last_quote_at": last_quote},
    })))
}

pub async fn quotes(
    State(state): State<SharedState>,
    _user: CurrentUser,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let quotes = market::quotes(&client).await?;
    Ok(Json(json!({"quotes": quotes})))
}

// ---------------------------------------------------------------------------------------------
// Candle board
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct BoardQuery {
    period: Option<String>,
}

#[derive(Serialize)]
pub struct BoardItem {
    symbol: String,
    name_zh: String,
    name_en: String,
    sector_id: String,
    reference: Option<f64>,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    /// Moves relative to the close before the period (fractions).
    open_pct: Option<f64>,
    high_pct: Option<f64>,
    low_pct: Option<f64>,
    close_pct: Option<f64>,
    weight: f64,
    target_weight: Option<f64>,
    live: bool,
}

fn period_start(calendar: &MarketCalendar, last: NaiveDate, period: &str) -> Option<NaiveDate> {
    let sessions = match period {
        "1D" => 1,
        "5D" => 5,
        "1M" => 21,
        "3M" => 63,
        "6M" => 126,
        "1Y" => 252,
        "YTD" => {
            let jan1 = NaiveDate::from_ymd_opt(last.year(), 1, 1)?;
            return Some(if calendar.is_trading_day(jan1) {
                jan1
            } else {
                calendar.next_trading_day(jan1)
            });
        }
        _ => return None,
    };
    let mut start = last;
    for _ in 1..sessions {
        start = calendar.prev_trading_day(start);
    }
    Some(start)
}

/// Merges the live quote into today's bar when a session is in progress.
fn with_live_bar(
    mut bars: Vec<DailyBar>,
    quote: Option<&market::StoredQuote>,
    today: NaiveDate,
    live: bool,
) -> (Vec<DailyBar>, bool) {
    let Some(q) = quote.filter(|_| live) else {
        return (bars, false);
    };
    let price = q.quote.price;
    if price <= 0.0 {
        return (bars, false);
    }
    match bars.last_mut() {
        Some(bar) if bar.date == today => {
            bar.close = price;
            bar.high = bar.high.max(q.quote.day_high.unwrap_or(price)).max(price);
            bar.low = bar.low.min(q.quote.day_low.unwrap_or(price)).min(price);
            if let Some(v) = q.quote.volume {
                bar.volume = bar.volume.max(v);
            }
        }
        _ => {
            let open = q.quote.open.unwrap_or(price);
            bars.push(DailyBar {
                symbol: q.quote.symbol.clone(),
                date: today,
                open,
                high: q.quote.day_high.unwrap_or(price).max(price).max(open),
                low: q.quote.day_low.unwrap_or(price).min(price).min(open),
                close: price,
                volume: q.quote.volume.unwrap_or(0.0),
                adj_open: None,
                adj_close: None,
            });
        }
    }
    (bars, true)
}

pub async fn board(
    State(state): State<SharedState>,
    _user: CurrentUser,
    Query(query): Query<BoardQuery>,
) -> ApiResult<Json<Value>> {
    let period = query.period.unwrap_or_else(|| "1D".into());
    let now = state.now();
    let today = MarketCalendar::local_date(now);
    let last = portfolio::last_session_date(&state, now);
    let live = last == today && state.calendar.session(today).is_some_and(|s| now >= s.open);
    let start = period_start(&state.calendar, last, &period)
        .ok_or_else(|| ApiError::bad("period must be 1D, 5D, 1M, 3M, 6M, YTD or 1Y"))?;
    let client = state.pool.get().await?;
    let assets = market::assets(&client, false).await?;
    let sectors = market::sectors(&client).await?;
    let symbols: Vec<String> = assets.iter().map(|a| a.symbol.clone()).collect();
    let panel =
        market::daily_ohlc_panel(&client, &symbols, start - Duration::days(10), last).await?;
    let quotes = market::quotes(&client).await?;
    let valuation = portfolio::valuation(&state, &client).await?;
    let weights: HashMap<&str, f64> = valuation
        .positions
        .iter()
        .map(|p| (p.symbol.as_str(), p.weight))
        .collect();
    let targets = trading::latest_targets(&client, valuation.account.id)
        .await?
        .map(|t| t.2)
        .unwrap_or_default();

    let mut items = Vec::new();
    for asset in &assets {
        let series = panel.get(&asset.symbol).cloned().unwrap_or_default();
        let (series, live_bar) = with_live_bar(series, quotes.get(&asset.symbol), today, live);
        let reference = series
            .iter()
            .rev()
            .find(|b| b.date < start)
            .map(|b| b.close);
        let window: Vec<&DailyBar> = series
            .iter()
            .filter(|b| b.date >= start && b.date <= last)
            .collect();
        let (Some(first), Some(last_bar)) = (window.first(), window.last()) else {
            continue;
        };
        let high = window.iter().map(|b| b.high).fold(f64::MIN, f64::max);
        let low = window.iter().map(|b| b.low).fold(f64::MAX, f64::min);
        let pct = |x: f64| reference.filter(|r| *r > 0.0).map(|r| x / r - 1.0);
        items.push(BoardItem {
            symbol: asset.symbol.clone(),
            name_zh: asset.name_zh.clone(),
            name_en: asset.name_en.clone(),
            sector_id: asset.sector_id.clone(),
            reference,
            open: first.open,
            high,
            low,
            close: last_bar.close,
            volume: window.iter().map(|b| b.volume).sum(),
            open_pct: pct(first.open),
            high_pct: pct(high),
            low_pct: pct(low),
            close_pct: pct(last_bar.close),
            weight: weights.get(asset.symbol.as_str()).copied().unwrap_or(0.0),
            target_weight: targets.get(&asset.symbol).copied(),
            live: live_bar,
        });
    }
    Ok(Json(json!({
        "period": period,
        "from": start,
        "to": last,
        "live": live,
        "as_of": now,
        "sectors": sectors,
        "items": items,
    })))
}

// ---------------------------------------------------------------------------------------------
// Asset candles
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct BarsQuery {
    range: Option<String>,
}

pub async fn bars(
    State(state): State<SharedState>,
    _user: CurrentUser,
    Path(symbol): Path<String>,
    Query(query): Query<BarsQuery>,
) -> ApiResult<Json<Value>> {
    let symbol = symbol.to_ascii_uppercase();
    let range = query.range.unwrap_or_else(|| "6M".into());
    let client = state.pool.get().await?;
    let asset = market::assets(&client, true)
        .await?
        .into_iter()
        .find(|a| a.symbol == symbol);
    let benchmark = universe::bundled()
        .benchmarks
        .into_iter()
        .find(|b| b.symbol == symbol);
    let bench_settings: BenchmarkSettings = settings::get(&client, settings::BENCHMARKS).await?;
    if asset.is_none() && benchmark.is_none() && !bench_settings.symbols.contains(&symbol) {
        return Err(ApiError::not_found(format!("symbol {symbol}")));
    }
    let now = state.now();
    let today = MarketCalendar::local_date(now);
    let last = portfolio::last_session_date(&state, now);
    let live = last == today && state.calendar.session(today).is_some_and(|s| now >= s.open);
    let quotes = market::quotes(&client).await?;
    let quote = quotes.get(&symbol);
    let account = trading::require_active_account(&client).await?;
    drop(client);

    let (interval, bars, sma50, sma200, from_ts) = match range.as_str() {
        "1D" | "5D" => {
            let (interval, days) = if range == "1D" {
                (Interval::FiveMin, 1)
            } else {
                (Interval::FifteenMin, 5)
            };
            let intraday = marketdata::intraday(&state, &symbol, interval, days)
                .await
                .unwrap_or_default();
            let from_ts = intraday.first().map(|b| b.ts);
            let bars: Vec<Value> = intraday
                .iter()
                .map(|b| json!({"t": b.ts, "o": b.open, "h": b.high, "l": b.low, "c": b.close, "v": b.volume}))
                .collect();
            (
                interval.as_str().to_string(),
                bars,
                Vec::new(),
                Vec::new(),
                from_ts,
            )
        }
        _ => {
            let start = match range.as_str() {
                "1M" => last - Duration::days(31),
                "3M" => last - Duration::days(92),
                "6M" => last - Duration::days(183),
                "1Y" => last - Duration::days(366),
                "5Y" => last - Duration::days(366 * 5),
                "MAX" => NaiveDate::from_ymd_opt(1990, 1, 1).expect("valid date"),
                _ => {
                    return Err(ApiError::bad(
                        "range must be 1D, 5D, 1M, 3M, 6M, 1Y, 5Y or MAX",
                    ));
                }
            };
            let client = state.pool.get().await?;
            let history = market::daily_bars(
                &client,
                &symbol,
                Some(start - Duration::days(300)),
                Some(last),
            )
            .await?;
            drop(client);
            let (history, _) = with_live_bar(history, quote, today, live);
            let closes: Vec<f64> = history.iter().map(|b| b.close).collect();
            let first = history
                .iter()
                .position(|b| b.date >= start)
                .unwrap_or(history.len());
            let ma = |n: usize| -> Vec<Option<f64>> {
                (first..closes.len())
                    .map(|i| stats::sma(&closes[..=i], n))
                    .collect()
            };
            let bars: Vec<Value> = history[first..]
                .iter()
                .map(|b| json!({"t": b.date, "o": b.open, "h": b.high, "l": b.low, "c": b.close, "v": b.volume}))
                .collect();
            let from_ts = history
                .get(first)
                .map(|b| b.date.and_hms_opt(0, 0, 0).expect("midnight").and_utc());
            (String::from("1day"), bars, ma(50), ma(200), from_ts)
        }
    };

    let client = state.pool.get().await?;
    let (fills, _) = trading::list_fills(
        &client,
        account.id,
        &trading::FillFilter {
            symbol: Some(symbol.clone()),
            from: from_ts,
            to: None,
            plan_id: None,
            limit: 500,
            offset: 0,
        },
    )
    .await?;
    let positions = trading::positions(&client, account.id, false).await?;
    let targets = trading::latest_targets(&client, account.id)
        .await?
        .map(|t| t.2)
        .unwrap_or_default();
    let display: DisplaySettings = settings::get(&client, settings::DISPLAY).await?;
    Ok(Json(json!({
        "symbol": symbol,
        "range": range,
        "interval": interval,
        "bars": bars,
        "sma50": sma50,
        "sma200": sma200,
        "trades": fills.iter().map(|f| json!({"t": f.executed_at, "side": f.side, "qty": f.qty, "price": f.price, "plan_id": f.plan_id})).collect::<Vec<_>>(),
        "quote": quote,
        "asset": asset,
        "benchmark": benchmark,
        "position": positions.iter().find(|p| p.symbol == symbol),
        "target_weight": targets.get(&symbol),
        "live": live,
        "display_timezone": display.timezone,
    })))
}
