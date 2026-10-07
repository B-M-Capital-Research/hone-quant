//! Backtests and performance analysis.

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{Datelike, Duration, NaiveDate};
use quant_core::backtest::{BacktestConfig, RebalanceFrequency};
use quant_core::calendar::MarketCalendar;
use quant_core::costs::CostModel;
use quant_core::schedule::PlanSlot;
use quant_core::strategy::{StrategyParams, preset};
use serde::Deserialize;
use serde_json::{Value, json};

use super::context::PortfolioCtx;
use super::error::{ApiError, ApiResult};
use crate::auth::{AdminUser, CurrentUser};
use crate::services::{backtests, portfolio};
use crate::state::SharedState;
use crate::store::settings::{self, ExecutionSettings};
use crate::store::{strategy as strategy_store, system};

pub async fn list_backtests(
    State(state): State<SharedState>,
    _user: CurrentUser,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    Ok(Json(
        json!({"backtests": system::backtests(&client, 200).await?}),
    ))
}

#[derive(Deserialize)]
pub struct BacktestBody {
    name: String,
    start: NaiveDate,
    end: Option<NaiveDate>,
    #[serde(default = "default_cash")]
    initial_cash: f64,
    /// One of: explicit params, a stored version, or a preset.
    params: Option<Value>,
    version_id: Option<i64>,
    preset_id: Option<String>,
    costs: Option<CostModel>,
    slots: Option<Vec<PlanSlot>>,
    frequency: Option<RebalanceFrequency>,
    benchmark: Option<String>,
}

fn default_cash() -> f64 {
    1_000_000.0
}

pub async fn create_backtest(
    State(state): State<SharedState>,
    AdminUser(user): AdminUser,
    Json(body): Json<BacktestBody>,
) -> ApiResult<Json<Value>> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(ApiError::bad("name must be 1–120 characters"));
    }
    if !(1_000.0..=1e10).contains(&body.initial_cash) {
        return Err(ApiError::bad(
            "initial cash must be between 1,000 and 10,000,000,000",
        ));
    }
    let client = state.pool.get().await?;
    let (params, version_id) = match (&body.params, body.version_id, &body.preset_id) {
        (Some(params), _, _) => (
            StrategyParams::from_json_strict(params).map_err(ApiError::Validation)?,
            None,
        ),
        (None, Some(id), _) => {
            let version = strategy_store::version(&client, id)
                .await?
                .ok_or_else(|| ApiError::not_found("strategy version"))?;
            (version.strategy_params()?, Some(id))
        }
        (None, None, Some(preset_id)) => (
            preset(preset_id)
                .ok_or_else(|| ApiError::bad("unknown preset"))?
                .params,
            None,
        ),
        _ => return Err(ApiError::bad("provide params, version_id or preset_id")),
    };
    let execution: ExecutionSettings = settings::get(&client, settings::EXECUTION).await?;
    drop(client);
    let today = MarketCalendar::local_date(state.now());
    let end = body.end.unwrap_or(today).min(today);
    let config = BacktestConfig {
        start: body.start,
        end,
        initial_cash: body.initial_cash,
        params,
        costs: body.costs.unwrap_or(execution.costs),
        slots: body
            .slots
            .unwrap_or_else(|| vec![PlanSlot::Open, PlanSlot::Close]),
        frequency: body.frequency.unwrap_or(RebalanceFrequency::Daily),
        risk_free_rate: 0.0,
        benchmark: body.benchmark,
    };
    let row = backtests::submit(&state, name, config, version_id, &user.username)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    Ok(Json(json!({"backtest": row})))
}

pub async fn backtest_detail(
    State(state): State<SharedState>,
    _user: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let (row, result) = system::backtest(&client, id)
        .await?
        .ok_or_else(|| ApiError::not_found("backtest"))?;
    Ok(Json(json!({"backtest": row, "result": result})))
}

pub async fn delete_backtest(
    State(state): State<SharedState>,
    AdminUser(user): AdminUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    if !system::delete_backtest(&client, id).await? {
        return Err(ApiError::conflict("backtest not found or still running"));
    }
    system::audit(
        &client,
        &user.username,
        "backtest.deleted",
        "backtest",
        &id.to_string(),
        json!({}),
        &user.ip,
    )
    .await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct PerformanceQuery {
    range: Option<String>,
}

pub async fn performance(
    State(state): State<SharedState>,
    ctx: PortfolioCtx,
    Query(q): Query<PerformanceQuery>,
) -> ApiResult<Json<Value>> {
    let today = MarketCalendar::local_date(state.now());
    let from = match q.range.as_deref().unwrap_or("ALL") {
        "1M" => Some(today - Duration::days(31)),
        "3M" => Some(today - Duration::days(92)),
        "6M" => Some(today - Duration::days(183)),
        "YTD" => NaiveDate::from_ymd_opt(today.year(), 1, 1).map(|d| d - Duration::days(1)),
        "1Y" => Some(today - Duration::days(366)),
        "ALL" => None,
        _ => return Err(ApiError::bad("range must be 1M, 3M, 6M, YTD, 1Y or ALL")),
    };
    let perf = portfolio::performance(&state, ctx.account(), from).await?;
    Ok(Json(
        serde_json::to_value(perf).map_err(anyhow::Error::from)?,
    ))
}
