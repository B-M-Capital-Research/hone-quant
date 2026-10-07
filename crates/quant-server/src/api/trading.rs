//! Plans and operator interventions, always for one portfolio. Every mutation is audited and
//! announced.

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use quant_core::calendar::MarketCalendar;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

use super::context::{PortfolioCtx, TradeCtx, authorize_account};
use super::error::{ApiError, ApiResult};
use crate::auth::{CurrentUser, Role};
use crate::notify::{self, Event};
use crate::services::{broker, planner, portfolio};
use crate::state::{ServerEvent, SharedState};
use crate::store::portfolios;
use crate::store::settings::{self, AutomationMode, AutomationSettings};
use crate::store::strategy as strategy_store;
use crate::store::trading::{self, dec};
use crate::store::{market, system};

fn parse_date(value: &str) -> ApiResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| ApiError::bad("dates must be YYYY-MM-DD"))
}

fn bounded(limit: Option<i64>, default: i64, max: i64) -> i64 {
    limit.unwrap_or(default).clamp(1, max)
}

#[derive(Deserialize)]
pub struct PlanQuery {
    from: Option<String>,
    to: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

pub async fn list_plans(
    State(state): State<SharedState>,
    ctx: PortfolioCtx,
    Query(q): Query<PlanQuery>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let filter = trading::PlanFilter {
        from: q.from.as_deref().map(parse_date).transpose()?,
        to: q.to.as_deref().map(parse_date).transpose()?,
        status: q.status.filter(|s| !s.is_empty()),
        limit: bounded(q.limit, 50, 500),
        offset: q.offset.unwrap_or(0).max(0),
    };
    let (plans, total) = trading::list_plans(&client, ctx.account().id, &filter).await?;
    Ok(Json(json!({"plans": plans, "total": total})))
}

/// A plan is shown and acted on in its own portfolio, whatever portfolio is selected, so links
/// from notifications work everywhere.
pub async fn plan_detail(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let plan = trading::plan(&client, id)
        .await?
        .ok_or_else(|| ApiError::not_found("plan"))?;
    let owner = authorize_account(&state, &user, plan.account_id, false).await?;
    let orders = trading::orders_for_plan(&client, id).await?;
    let diagnostics = trading::plan_diagnostics(&client, id).await?;
    let (fills, _) = trading::list_fills(
        &client,
        plan.account_id,
        &trading::FillFilter {
            symbol: None,
            from: None,
            to: None,
            plan_id: Some(id),
            limit: 500,
            offset: 0,
        },
    )
    .await?;
    let audit = system::audit_entries(
        &client,
        &system::AuditFilter {
            actor: None,
            action: None,
            entity_type: Some("plan".into()),
            entity_id: Some(id.to_string()),
            before: None,
            limit: 50,
        },
    )
    .await?;
    let version = match plan.strategy_version_id {
        Some(v) => strategy_store::version(&client, v).await?,
        None => None,
    };
    let assets = market::assets(&client, true).await?;
    let names: serde_json::Map<String, Value> = assets
        .iter()
        .map(|a| {
            (
                a.symbol.clone(),
                json!({"zh": a.name_zh, "en": a.name_en, "sector": a.sector_id}),
            )
        })
        .collect();
    Ok(Json(json!({
        "plan": plan,
        "orders": orders,
        "fills": fills,
        "diagnostics": diagnostics,
        "audit": audit,
        "strategy": version,
        "names": names,
        "portfolio": owner.tag(),
        "can_trade": user.can_trade(&owner),
    })))
}

/// Loads a plan and checks that `user` may act on its portfolio.
async fn plan_to_act_on(
    state: &SharedState,
    user: &CurrentUser,
    id: i64,
) -> ApiResult<trading::Plan> {
    let client = state.pool.get().await?;
    let plan = trading::plan(&client, id)
        .await?
        .ok_or_else(|| ApiError::not_found("plan"))?;
    drop(client);
    authorize_account(state, user, plan.account_id, true).await?;
    Ok(plan)
}

#[derive(Deserialize, Default)]
pub struct NoteBody {
    #[serde(default)]
    note: String,
    #[serde(default)]
    reason: String,
}

/// Approve = execute now (approval mode) or skip the rest of the review window (auto mode).
pub async fn approve_plan(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    body: Option<Json<NoteBody>>,
) -> ApiResult<Json<Value>> {
    let note = body.map(|b| b.0.note).unwrap_or_default();
    let plan = plan_to_act_on(&state, &user, id).await?;
    if plan.status != "pending" {
        return Err(ApiError::conflict(format!("plan is {}", plan.status)));
    }
    {
        let client = state.pool.get().await?;
        trading::approve_plan(&client, id, &user.username).await?;
        system::audit(
            &client,
            &user.username,
            "plan.approved",
            "plan",
            &id.to_string(),
            json!({"note": note}),
            &user.ip,
        )
        .await?;
    }
    let report = broker::execute_plan(&state, id, &user.username)
        .await
        .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
    Ok(Json(json!({"report": report})))
}

pub async fn cancel_plan(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    body: Option<Json<NoteBody>>,
) -> ApiResult<Json<Value>> {
    let reason = body
        .map(|b| {
            if b.0.reason.is_empty() {
                b.0.note
            } else {
                b.0.reason
            }
        })
        .unwrap_or_default();
    plan_to_act_on(&state, &user, id).await?;
    broker::cancel_plan(&state, id, &user.username, &reason, &user.ip)
        .await
        .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
    Ok(Json(json!({"ok": true})))
}

pub async fn skip_order(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path((id, order_id)): Path<(i64, i64)>,
) -> ApiResult<Json<Value>> {
    plan_to_act_on(&state, &user, id).await?;
    broker::skip_order(&state, id, order_id, &user.username, &user.ip)
        .await
        .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
    Ok(Json(json!({"ok": true})))
}

/// Generates an extra plan right now (market must be open).
pub async fn generate_plan(
    State(state): State<SharedState>,
    TradeCtx(ctx): TradeCtx,
) -> ApiResult<Json<Value>> {
    let now = state.now();
    let today = MarketCalendar::local_date(now);
    let session = state
        .calendar
        .session(today)
        .filter(|s| s.contains(now) && now < s.close - Duration::minutes(10))
        .ok_or_else(|| {
            ApiError::conflict("manual plans can only be generated during the regular session")
        })?;
    {
        let client = state.pool.get().await?;
        if trading::open_plans(&client, ctx.account().id)
            .await?
            .iter()
            .any(|p| p.status == "pending")
        {
            return Err(ApiError::conflict(
                "another plan is still pending; approve or cancel it first",
            ));
        }
        if ctx.portfolio().automation.effective_mode(now) == AutomationMode::Paused {
            return Err(ApiError::conflict("automation is paused"));
        }
    }
    let generated = planner::generate(
        &state,
        ctx.portfolio().id,
        planner::PlanRequest {
            slot: "manual".into(),
            trade_date: today,
            deadline: session.close - Duration::minutes(5),
            actor: ctx.user.username.clone(),
        },
    )
    .await?;
    Ok(Json(json!({"plan": generated})))
}

pub async fn trading_day(
    State(state): State<SharedState>,
    ctx: PortfolioCtx,
    Path(date): Path<String>,
) -> ApiResult<Json<Value>> {
    let date = parse_date(&date)?;
    let client = state.pool.get().await?;
    let schedule_settings = settings::schedule(&client).await?;
    let session = state.calendar.session(date);
    let schedule = session
        .map(|s| quant_core::schedule::day_schedule(&s, &schedule_settings))
        .unwrap_or_default();
    Ok(Json(json!({
        "date": date,
        "session": session,
        "holiday": state.calendar.holiday(date),
        "schedule": schedule,
        "cancelled": strategy_store::skipped_slots(&client, ctx.portfolio().id, date, date).await?,
        "plans": trading::plans_for_date(&client, ctx.account().id, date).await?,
    })))
}

#[derive(Deserialize)]
pub struct CancelDayBody {
    #[serde(default)]
    slots: Vec<String>,
    #[serde(default)]
    reason: String,
}

/// "Cancel today's planned trades": cancels the portfolio's pending plans and pre-cancels its
/// slots not yet generated, for the given trade date.
pub async fn cancel_day(
    State(state): State<SharedState>,
    TradeCtx(ctx): TradeCtx,
    Path(date): Path<String>,
    Json(body): Json<CancelDayBody>,
) -> ApiResult<Json<Value>> {
    let user = &ctx.user;
    let portfolio_id = ctx.portfolio().id;
    let date = parse_date(&date)?;
    let today = MarketCalendar::local_date(state.now());
    if date < today {
        return Err(ApiError::bad("past trading days cannot be cancelled"));
    }
    if !state.calendar.is_trading_day(date) {
        return Err(ApiError::bad("that date is not a trading day"));
    }
    let slots: Vec<String> = if body.slots.is_empty() {
        vec!["open".into(), "close".into()]
    } else {
        body.slots
    };
    if slots.iter().any(|s| s != "open" && s != "close") {
        return Err(ApiError::bad("slots must be open and/or close"));
    }
    let client = state.pool.get().await?;
    let plans = trading::plans_for_date(&client, ctx.account().id, date).await?;
    drop(client);
    let mut cancelled_plans = Vec::new();
    let mut pre_cancelled = Vec::new();
    let mut untouched = Vec::new();
    for slot in &slots {
        match plans.iter().rev().find(|p| &p.slot == slot) {
            Some(plan) if plan.status == "pending" => {
                broker::cancel_plan(&state, plan.id, &user.username, &body.reason, &user.ip)
                    .await
                    .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
                cancelled_plans.push(plan.id);
            }
            Some(plan) => {
                untouched.push(json!({"slot": slot, "status": plan.status, "plan_id": plan.id}))
            }
            None => {
                let client = state.pool.get().await?;
                if strategy_store::skip_slot(
                    &client,
                    portfolio_id,
                    date,
                    slot,
                    &body.reason,
                    &user.username,
                )
                .await?
                {
                    pre_cancelled.push(slot.clone());
                }
            }
        }
    }
    // Pending manual plans of that day are cancelled too.
    for plan in plans
        .iter()
        .filter(|p| p.slot == "manual" && p.status == "pending")
    {
        broker::cancel_plan(&state, plan.id, &user.username, &body.reason, &user.ip)
            .await
            .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
        cancelled_plans.push(plan.id);
    }
    let client = state.pool.get().await?;
    system::audit(
        &client,
        &user.username,
        "trading_day.cancelled",
        "trading_day",
        &date.to_string(),
        json!({"portfolio_id": portfolio_id, "slots": slots, "reason": body.reason, "cancelled_plans": cancelled_plans, "pre_cancelled": pre_cancelled}),
        &user.ip,
    )
    .await?;
    drop(client);
    if !pre_cancelled.is_empty() || !cancelled_plans.is_empty() {
        let _ = notify::notify_in(
            &state,
            &ctx.portfolio().tag(),
            Event::SlotsCancelled {
                trade_date: date,
                slots: slots.clone(),
                actor: user.username.clone(),
                reason: body.reason.clone(),
            },
        )
        .await;
    }
    state.emit(ServerEvent::Settings {
        key: "trading_day".into(),
        portfolio_id: Some(portfolio_id),
    });
    Ok(Json(
        json!({"cancelled_plans": cancelled_plans, "pre_cancelled": pre_cancelled, "already_final": untouched}),
    ))
}

pub async fn restore_slot(
    State(state): State<SharedState>,
    TradeCtx(ctx): TradeCtx,
    Path((date, slot)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let date = parse_date(&date)?;
    let portfolio_id = ctx.portfolio().id;
    let client = state.pool.get().await?;
    if trading::plan_for_slot(&client, ctx.account().id, date, &slot)
        .await?
        .is_some()
    {
        return Err(ApiError::conflict("that slot already has a plan"));
    }
    if !strategy_store::unskip_slot(&client, portfolio_id, date, &slot).await? {
        return Err(ApiError::not_found("cancelled slot"));
    }
    system::audit(
        &client,
        &ctx.user.username,
        "trading_day.restored",
        "trading_day",
        &date.to_string(),
        json!({"portfolio_id": portfolio_id, "slot": slot}),
        &ctx.user.ip,
    )
    .await?;
    state.emit(ServerEvent::Settings {
        key: "trading_day".into(),
        portfolio_id: Some(portfolio_id),
    });
    Ok(Json(json!({"ok": true})))
}

pub async fn get_automation(
    State(state): State<SharedState>,
    ctx: PortfolioCtx,
) -> ApiResult<Json<Value>> {
    let automation = &ctx.portfolio().automation;
    Ok(Json(
        json!({"automation": automation, "effective_mode": automation.effective_mode(state.now()).as_str()}),
    ))
}

pub async fn put_automation(
    State(state): State<SharedState>,
    TradeCtx(ctx): TradeCtx,
    Json(body): Json<AutomationSettings>,
) -> ApiResult<Json<Value>> {
    if let Some(until) = body.paused_until {
        if until <= state.now() {
            return Err(ApiError::bad("paused_until must be in the future"));
        }
        if until > state.now() + Duration::days(60) {
            return Err(ApiError::bad("pauses are limited to 60 days"));
        }
    }
    let portfolio = ctx.portfolio();
    let client = state.pool.get().await?;
    portfolios::set_automation(&client, portfolio.id, &body).await?;
    system::audit(
        &client,
        &ctx.user.username,
        "automation.changed",
        "portfolio",
        &portfolio.id.to_string(),
        json!({"before": portfolio.automation, "after": body}),
        &ctx.user.ip,
    )
    .await?;
    drop(client);
    state.emit(ServerEvent::Settings {
        key: settings::AUTOMATION.into(),
        portfolio_id: Some(portfolio.id),
    });
    let _ = notify::notify_in(
        &state,
        &portfolio.tag(),
        Event::AutomationChanged {
            mode: body.mode.as_str().into(),
            paused_until: body.paused_until,
            actor: ctx.user.username.clone(),
        },
    )
    .await;
    Ok(Json(json!({"automation": body})))
}

pub async fn list_restrictions(
    State(state): State<SharedState>,
    ctx: PortfolioCtx,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let today = MarketCalendar::local_date(state.now());
    let portfolio_id = ctx.portfolio().id;
    Ok(Json(json!({
        "active": strategy_store::active_restrictions(&client, today, portfolio_id).await?,
        "history": strategy_store::all_restrictions(&client, portfolio_id).await?,
    })))
}

#[derive(Deserialize)]
pub struct RestrictionBody {
    symbol: String,
    mode: String,
    #[serde(default)]
    reason: String,
    starts_on: Option<String>,
    ends_on: Option<String>,
    /// `portfolio` (the default) or `all`: every portfolio, administrators only.
    #[serde(default)]
    scope: Option<String>,
}

pub async fn add_restriction(
    State(state): State<SharedState>,
    TradeCtx(ctx): TradeCtx,
    Json(body): Json<RestrictionBody>,
) -> ApiResult<Json<Value>> {
    let user = &ctx.user;
    if body.mode != "exclude" && body.mode != "lock" {
        return Err(ApiError::bad("mode must be exclude or lock"));
    }
    let portfolio_id = match body.scope.as_deref().unwrap_or("portfolio") {
        "portfolio" => Some(ctx.portfolio().id),
        "all" if user.role == Role::Admin => None,
        "all" => {
            return Err(ApiError::Forbidden(
                "restrictions for every portfolio require the admin role".into(),
            ));
        }
        _ => return Err(ApiError::bad("scope must be portfolio or all")),
    };
    let symbol = body.symbol.trim().to_ascii_uppercase();
    let today = MarketCalendar::local_date(state.now());
    let starts_on = body
        .starts_on
        .as_deref()
        .map(parse_date)
        .transpose()?
        .unwrap_or(today);
    let ends_on = body
        .ends_on
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(parse_date)
        .transpose()?;
    if ends_on.is_some_and(|e| e < starts_on) {
        return Err(ApiError::bad("the end date is before the start date"));
    }
    let client = state.pool.get().await?;
    if !market::assets(&client, true)
        .await?
        .iter()
        .any(|a| a.symbol == symbol)
    {
        return Err(ApiError::not_found(format!(
            "symbol {symbol} in the universe"
        )));
    }
    let restriction = strategy_store::add_restriction(
        &client,
        portfolio_id,
        &symbol,
        &body.mode,
        &body.reason,
        starts_on,
        ends_on,
        &user.username,
    )
    .await?;
    system::audit(
        &client,
        &user.username,
        "restriction.added",
        "restriction",
        &restriction.id.to_string(),
        json!(restriction),
        &user.ip,
    )
    .await?;
    state.emit(ServerEvent::Settings {
        key: "restrictions".into(),
        portfolio_id,
    });
    Ok(Json(json!({"restriction": restriction})))
}

pub async fn revoke_restriction(
    State(state): State<SharedState>,
    TradeCtx(ctx): TradeCtx,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let user = &ctx.user;
    let client = state.pool.get().await?;
    let existing = strategy_store::restriction(&client, id)
        .await?
        .ok_or_else(|| ApiError::not_found("active restriction"))?;
    match existing.portfolio_id {
        None if user.role != Role::Admin => {
            return Err(ApiError::Forbidden(
                "restrictions for every portfolio require the admin role".into(),
            ));
        }
        Some(owner) if owner != ctx.portfolio().id => {
            return Err(ApiError::not_found("active restriction"));
        }
        _ => {}
    }
    let revoked = strategy_store::revoke_restriction(&client, id, &user.username)
        .await?
        .ok_or_else(|| ApiError::not_found("active restriction"))?;
    system::audit(
        &client,
        &user.username,
        "restriction.revoked",
        "restriction",
        &id.to_string(),
        json!(revoked),
        &user.ip,
    )
    .await?;
    state.emit(ServerEvent::Settings {
        key: "restrictions".into(),
        portfolio_id: revoked.portfolio_id,
    });
    Ok(Json(json!({"restriction": revoked})))
}

#[derive(Deserialize)]
pub struct LedgerQuery {
    symbol: Option<String>,
    from: Option<String>,
    to: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

pub async fn list_orders(
    State(state): State<SharedState>,
    ctx: PortfolioCtx,
    Query(q): Query<LedgerQuery>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let (orders, total) = trading::list_orders(
        &client,
        ctx.account().id,
        &trading::OrderFilter {
            symbol: q
                .symbol
                .filter(|s| !s.is_empty())
                .map(|s| s.to_ascii_uppercase()),
            from: q.from.as_deref().map(parse_date).transpose()?,
            to: q.to.as_deref().map(parse_date).transpose()?,
            status: q.status.filter(|s| !s.is_empty()),
            limit: bounded(q.limit, 100, 1000),
            offset: q.offset.unwrap_or(0).max(0),
        },
    )
    .await?;
    Ok(Json(json!({"orders": orders, "total": total})))
}

fn day_start(date: Option<NaiveDate>) -> Option<DateTime<Utc>> {
    date.map(|d| d.and_hms_opt(0, 0, 0).expect("midnight").and_utc())
}

pub async fn list_fills(
    State(state): State<SharedState>,
    ctx: PortfolioCtx,
    Query(q): Query<LedgerQuery>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let from = q.from.as_deref().map(parse_date).transpose()?;
    let to = q.to.as_deref().map(parse_date).transpose()?;
    let (fills, total) = trading::list_fills(
        &client,
        ctx.account().id,
        &trading::FillFilter {
            symbol: q
                .symbol
                .filter(|s| !s.is_empty())
                .map(|s| s.to_ascii_uppercase()),
            from: day_start(from),
            to: day_start(to.map(|d| d + Duration::days(1))),
            plan_id: None,
            limit: bounded(q.limit, 100, 5000),
            offset: q.offset.unwrap_or(0).max(0),
        },
    )
    .await?;
    Ok(Json(json!({"fills": fills, "total": total})))
}

pub async fn ledger(
    State(state): State<SharedState>,
    ctx: PortfolioCtx,
    Query(q): Query<LedgerQuery>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    Ok(Json(
        json!({"entries": trading::ledger(&client, ctx.account().id, bounded(q.limit, 200, 5000)).await?}),
    ))
}

/// The portfolio's paper accounts: the active one and those archived by resets.
pub async fn accounts(
    State(state): State<SharedState>,
    ctx: PortfolioCtx,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    Ok(Json(
        json!({"accounts": trading::list_accounts(&client, ctx.portfolio().id).await?}),
    ))
}

#[derive(Deserialize)]
pub struct ResetBody {
    initial_cash: f64,
    confirm: String,
}

/// Archives the portfolio's paper account and starts a fresh one in the same portfolio (history
/// stays queryable).
pub async fn reset_account(
    State(state): State<SharedState>,
    TradeCtx(ctx): TradeCtx,
    Json(body): Json<ResetBody>,
) -> ApiResult<Json<Value>> {
    let user = &ctx.user;
    let portfolio = ctx.portfolio().clone();
    if body.confirm != "RESET" {
        return Err(ApiError::bad("type RESET to confirm"));
    }
    if !(body.initial_cash >= 1_000.0 && body.initial_cash <= 1e10) {
        return Err(ApiError::bad(
            "initial cash must be between 1,000 and 10,000,000,000",
        ));
    }
    let _guard = state.trading_lock.lock().await;
    let mut client = state.pool.get().await?;
    let tx = client.transaction().await?;
    portfolios::lock(&tx, portfolio.id).await?;
    let old = trading::active_account(&tx, portfolio.id)
        .await?
        .ok_or(ApiError::PortfolioNotFound)?;
    if trading::open_plans(&tx, old.id)
        .await?
        .iter()
        .any(|p| p.status == "executing")
    {
        return Err(ApiError::conflict(
            "a plan is executing; try again in a moment",
        ));
    }
    for plan in trading::open_plans(&tx, old.id).await? {
        trading::cancel_plan(&tx, plan.id, &user.username, "account reset").await?;
    }
    let version = strategy_store::active_version(&tx, old.id).await?;
    trading::archive_account(&tx, old.id).await?;
    let (first_session, base_date) = portfolio::inception_dates(&state, state.now());
    let cash = dec(body.initial_cash, 2);
    let account = trading::create_account(&tx, portfolio.id, "Paper", cash, first_session).await?;
    trading::upsert_nav(&tx, account.id, base_date, cash, cash, Decimal::ZERO).await?;
    if let Some(version) = version {
        strategy_store::activate(
            &tx,
            account.id,
            version.id,
            &user.username,
            "carried over on account reset",
        )
        .await?;
    }
    system::audit(
        &tx,
        &user.username,
        "account.reset",
        "account",
        &account.id.to_string(),
        json!({"portfolio_id": portfolio.id, "archived": old.id, "initial_cash": body.initial_cash}),
        &user.ip,
    )
    .await?;
    tx.commit().await?;
    drop(client);
    state.emit(ServerEvent::Account {
        reason: "reset".into(),
        portfolio_id: portfolio.id,
    });
    let _ = notify::notify_in(
        &state,
        &portfolio.tag(),
        Event::AccountReset {
            initial_cash: body.initial_cash,
            actor: user.username.clone(),
        },
    )
    .await;
    let client = state.pool.get().await?;
    let valuation = portfolio::valuation(&state, &client, &account).await?;
    Ok(Json(json!({"account": account, "valuation": valuation})))
}
