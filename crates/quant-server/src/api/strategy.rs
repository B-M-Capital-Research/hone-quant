//! Strategy versions (a library shared by every portfolio; each portfolio activates one of
//! them) and the universe.

use axum::Json;
use axum::extract::{Path, State};
use quant_core::calendar::MarketCalendar;
use quant_core::strategy::{StrategyParams, preset, presets};
use serde::Deserialize;
use serde_json::{Value, json};

use super::context::{MaybePortfolio, PortfolioCtx, TradeCtx};
use super::error::{ApiError, ApiResult};
use crate::auth::{AdminUser, Role};
use crate::notify::{self, Event};
use crate::services::planner;
use crate::state::{ServerEvent, SharedState};
use crate::store::strategy as strategy_store;
use crate::store::{market, system};
use crate::universe;

pub async fn overview(
    State(state): State<SharedState>,
    ctx: PortfolioCtx,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let account = ctx.account();
    Ok(Json(json!({
        "portfolio": ctx.portfolio().tag(),
        "active": strategy_store::active_version(&client, account.id).await?,
        "versions": strategy_store::versions(&client).await?,
        "activations": strategy_store::activations(&client, account.id).await?,
        "presets": presets(),
        "defaults": StrategyParams::default(),
    })))
}

#[derive(Deserialize)]
pub struct PreviewBody {
    params: Value,
}

/// Dry run: what the given parameters would trade right now in the selected portfolio. Nothing
/// is stored.
pub async fn preview(
    State(state): State<SharedState>,
    ctx: PortfolioCtx,
    Json(body): Json<PreviewBody>,
) -> ApiResult<Json<Value>> {
    let params = StrategyParams::from_json_strict(&body.params).map_err(ApiError::Validation)?;
    let today = MarketCalendar::local_date(state.now());
    let proposal = planner::build_proposal(&state, ctx.account(), &params, today).await?;
    Ok(Json(json!({
        "as_of": state.now(),
        "nav": proposal.nav,
        "cash": proposal.cash,
        "targets": proposal.targets,
        "orders": proposal.orders_json(),
        "skipped": proposal.skipped(),
        "frozen": proposal.frozen,
        "excluded": proposal.excluded,
        "turnover": proposal.plan.turnover,
        "est_costs": proposal.plan.est_commission + proposal.plan.est_fees + proposal.plan.est_slippage,
        "history": {"from": proposal.history_from, "to": proposal.history_to},
    })))
}

#[derive(Deserialize)]
pub struct VersionBody {
    name: String,
    preset_id: String,
    params: Value,
    #[serde(default)]
    note: String,
    #[serde(default)]
    activate: bool,
}

/// Adds a version to the shared library (administrators); `activate` also activates it in the
/// selected portfolio.
pub async fn create_version(
    State(state): State<SharedState>,
    ctx: PortfolioCtx,
    Json(body): Json<VersionBody>,
) -> ApiResult<Json<Value>> {
    let user = &ctx.user;
    if user.role != Role::Admin {
        return Err(ApiError::Forbidden(
            "this action requires the admin role".into(),
        ));
    }
    if body.activate && !ctx.can_trade() {
        return Err(ApiError::Forbidden(
            "you cannot act on this portfolio".into(),
        ));
    }
    let params = StrategyParams::from_json_strict(&body.params).map_err(ApiError::Validation)?;
    if preset(&body.preset_id).is_none() && body.preset_id != "custom" {
        return Err(ApiError::bad("unknown preset"));
    }
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError::bad("name must be 1–80 characters"));
    }
    let mut client = state.pool.get().await?;
    let account = ctx.account();
    let tx = client.transaction().await?;
    let version = strategy_store::insert_version(
        &tx,
        name,
        &body.preset_id,
        &params,
        body.note.trim(),
        &user.username,
    )
    .await?;
    system::audit(
        &tx,
        &user.username,
        "strategy.version_created",
        "strategy_version",
        &version.id.to_string(),
        json!({"name": name, "preset_id": body.preset_id, "params": params, "note": body.note}),
        &user.ip,
    )
    .await?;
    if body.activate {
        strategy_store::activate(
            &tx,
            account.id,
            version.id,
            &user.username,
            body.note.trim(),
        )
        .await?;
        system::audit(
            &tx,
            &user.username,
            "strategy.activated",
            "strategy_version",
            &version.id.to_string(),
            json!({"portfolio_id": ctx.portfolio().id}),
            &user.ip,
        )
        .await?;
    }
    tx.commit().await?;
    drop(client);
    state.emit(ServerEvent::Strategy {
        version_id: version.id,
        portfolio_id: body.activate.then_some(ctx.portfolio().id),
    });
    if body.activate {
        let _ = notify::notify_in(
            &state,
            &ctx.portfolio().tag(),
            Event::StrategyActivated {
                version_id: version.id,
                name: version.name.clone(),
                preset_id: version.preset_id.clone(),
                actor: user.username.clone(),
            },
        )
        .await;
    }
    Ok(Json(
        json!({"version": version, "activated": body.activate}),
    ))
}

#[derive(Deserialize, Default)]
pub struct ActivateBody {
    #[serde(default)]
    note: String,
}

/// Activates a library version in the selected portfolio.
pub async fn activate_version(
    State(state): State<SharedState>,
    TradeCtx(ctx): TradeCtx,
    Path(id): Path<i64>,
    body: Option<Json<ActivateBody>>,
) -> ApiResult<Json<Value>> {
    let user = &ctx.user;
    let note = body.map(|b| b.0.note).unwrap_or_default();
    let client = state.pool.get().await?;
    let account = ctx.account();
    let version = strategy_store::version(&client, id)
        .await?
        .ok_or_else(|| ApiError::not_found("strategy version"))?;
    version
        .strategy_params()
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    if strategy_store::active_version(&client, account.id)
        .await?
        .is_some_and(|v| v.id == id)
    {
        return Err(ApiError::conflict("that version is already active"));
    }
    strategy_store::activate(&client, account.id, id, &user.username, note.trim()).await?;
    system::audit(
        &client,
        &user.username,
        "strategy.activated",
        "strategy_version",
        &id.to_string(),
        json!({"note": note, "portfolio_id": ctx.portfolio().id}),
        &user.ip,
    )
    .await?;
    drop(client);
    state.emit(ServerEvent::Strategy {
        version_id: id,
        portfolio_id: Some(ctx.portfolio().id),
    });
    let _ = notify::notify_in(
        &state,
        &ctx.portfolio().tag(),
        Event::StrategyActivated {
            version_id: id,
            name: version.name.clone(),
            preset_id: version.preset_id.clone(),
            actor: user.username.clone(),
        },
    )
    .await;
    Ok(Json(json!({"version": version})))
}

pub async fn universe(
    State(state): State<SharedState>,
    context: MaybePortfolio,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let versions = client
        .query(
            "SELECT id, source, ontology_schema_version, ontology_generated_at, content_hash, changes, applied_by, applied_at
             FROM universe_versions ORDER BY id DESC LIMIT 20",
            &[],
        )
        .await?;
    let versions: Vec<Value> = versions
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<_, i64>(0),
                "source": r.get::<_, String>(1),
                "ontology_schema_version": r.get::<_, Option<i32>>(2),
                "ontology_generated_at": r.get::<_, Option<String>>(3),
                "content_hash": r.get::<_, String>(4),
                "changes": r.get::<_, Value>(5),
                "applied_by": r.get::<_, String>(6),
                "applied_at": r.get::<_, chrono::DateTime<chrono::Utc>>(7),
            })
        })
        .collect();
    let today = MarketCalendar::local_date(state.now());
    Ok(Json(json!({
        "sectors": market::sectors(&client).await?,
        "assets": market::assets(&client, false).await?,
        "removed": market::assets(&client, true).await?.into_iter().filter(|a| !a.is_active).collect::<Vec<_>>(),
        "benchmarks": universe::bundled().benchmarks,
        "versions": versions,
        // The selected portfolio's restrictions (without one: only the universe-wide ones; no
        // portfolio has id 0).
        "restrictions": strategy_store::active_restrictions(
            &client,
            today,
            context.book.as_ref().map_or(0, |b| b.portfolio.id),
        )
        .await?,
        "bundled_source": universe::bundled().source,
        "ontology_url": universe::ONTOLOGY_URL,
    })))
}

#[derive(Deserialize, Default)]
pub struct UniverseBody {
    ontology: Option<String>,
    edits: Option<String>,
}

async fn build_universe(body: &UniverseBody) -> ApiResult<universe::UniverseFile> {
    let location = body
        .ontology
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| universe::ONTOLOGY_URL.into());
    let ontology = universe::read_json(&location)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    let edits = match body.edits.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(location) => Some(
            universe::read_json(location)
                .await
                .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?,
        ),
        None => None,
    };
    universe::build_from_ontology(&ontology, edits.as_ref(), &universe::overlay(), &location)
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))
}

/// Builds the universe from the honeclaw ontology and reports what would change.
pub async fn universe_check(
    State(state): State<SharedState>,
    _admin: AdminUser,
    body: Option<Json<UniverseBody>>,
) -> ApiResult<Json<Value>> {
    let body = body.map(|b| b.0).unwrap_or_default();
    let built = build_universe(&body).await?;
    let client = state.pool.get().await?;
    let changes = universe::diff(&client, &built).await?;
    Ok(Json(
        json!({"changes": changes, "source": built.source, "sectors": built.sectors.len(), "assets": built.assets.len()}),
    ))
}

pub async fn universe_apply(
    State(state): State<SharedState>,
    AdminUser(user): AdminUser,
    body: Option<Json<UniverseBody>>,
) -> ApiResult<Json<Value>> {
    let body = body.map(|b| b.0).unwrap_or_default();
    let built = build_universe(&body).await?;
    let mut client = state.pool.get().await?;
    let changes = universe::sync_to_db(&mut client, &built, &user.username).await?;
    system::audit(
        &client,
        &user.username,
        "universe.applied",
        "universe",
        "",
        json!({"changes": changes, "source": built.source}),
        &user.ip,
    )
    .await?;
    drop(client);
    state.emit(ServerEvent::Universe);
    if !changes.is_empty() {
        let _ = notify::notify(
            &state,
            Event::UniverseChanged {
                added: changes.added.clone(),
                removed: changes.removed.clone(),
            },
        )
        .await;
    }
    Ok(Json(json!({"changes": changes})))
}
