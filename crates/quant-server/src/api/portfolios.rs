//! Portfolios: list, create, rename, archive.

use axum::Json;
use axum::extract::{Path, Query, State};
use deadpool_postgres::GenericClient;
use serde::Deserialize;
use serde_json::{Value, json};

use super::context::MaybePortfolio;
use super::error::{ApiError, ApiResult};
use crate::auth::{CurrentUser, Role};
use crate::services::portfolio::{self, OpenPortfolio};
use crate::state::{AppState, ServerEvent, SharedState};
use crate::store::portfolios::{self, Book, NewPortfolio, Portfolio};
use crate::store::settings::{AutomationMode, AutomationSettings};
use crate::store::strategy as strategy_store;
use crate::store::system;
use crate::store::trading::{self, dec};

const MAX_NAME: usize = 60;
const MAX_DESCRIPTION: usize = 500;

/// A portfolio as the API shows it: the stored row plus its effective automation mode, whether
/// the user may act on it, its active account and (optionally) headline figures.
async fn view(
    state: &AppState,
    client: &impl GenericClient,
    user: &CurrentUser,
    portfolio: Portfolio,
    with_summary: bool,
) -> ApiResult<Value> {
    let account = if portfolio.is_active() {
        trading::active_account(client, portfolio.id).await?
    } else {
        None
    };
    let summary = match (&account, with_summary) {
        (Some(account), true) => {
            let book = Book {
                portfolio: portfolio.clone(),
                account: account.clone(),
            };
            Some(portfolio::summary(state, client, &book).await?)
        }
        _ => None,
    };
    let mut value = serde_json::to_value(&portfolio).map_err(anyhow::Error::from)?;
    value["effective_mode"] = json!(portfolio.automation.effective_mode(state.now()).as_str());
    value["can_trade"] = json!(user.can_trade(&portfolio));
    value["account"] = json!(account);
    value["summary"] = json!(summary);
    Ok(value)
}

fn clean_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(ApiError::bad(format!(
            "name must be 1–{MAX_NAME} characters"
        )));
    }
    Ok(name.to_string())
}

fn clean_description(description: &str) -> ApiResult<String> {
    let description = description.trim();
    if description.chars().count() > MAX_DESCRIPTION {
        return Err(ApiError::bad(format!(
            "description must be at most {MAX_DESCRIPTION} characters"
        )));
    }
    Ok(description.to_string())
}

/// The portfolio, if `user` may see it (archived ones included).
async fn visible(client: &impl GenericClient, user: &CurrentUser, id: i64) -> ApiResult<Portfolio> {
    portfolios::get(client, id)
        .await?
        .filter(|p| user.can_view(p))
        .ok_or_else(|| ApiError::not_found("portfolio"))
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    include_archived: bool,
}

pub async fn list(
    State(state): State<SharedState>,
    user: CurrentUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let mut items = Vec::new();
    let mut default_id = None;
    for portfolio in portfolios::list(&client, q.include_archived).await? {
        if !user.can_view(&portfolio) {
            continue;
        }
        if default_id.is_none() && portfolio.is_active() {
            default_id = Some(portfolio.id);
        }
        items.push(view(&state, &client, &user, portfolio, true).await?);
    }
    Ok(Json(json!({
        "portfolios": items,
        "can_create": user.can_create_portfolio(),
        "default_id": default_id,
    })))
}

pub async fn detail(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let portfolio = visible(&client, &user, id).await?;
    Ok(Json(
        json!({"portfolio": view(&state, &client, &user, portfolio, true).await?}),
    ))
}

#[derive(Deserialize)]
pub struct CreateBody {
    name: String,
    #[serde(default)]
    description: String,
    initial_cash: f64,
    strategy_version_id: Option<i64>,
    automation_mode: Option<AutomationMode>,
    /// Administrators: a local username to create it for, or none for a shared portfolio.
    owner: Option<String>,
}

pub async fn create(
    State(state): State<SharedState>,
    context: MaybePortfolio,
    Json(body): Json<CreateBody>,
) -> ApiResult<Json<Value>> {
    let user = &context.user;
    if !user.can_create_portfolio() {
        return Err(ApiError::Forbidden(
            "viewers cannot create portfolios".into(),
        ));
    }
    let name = clean_name(&body.name)?;
    let description = clean_description(&body.description)?;
    if !(1_000.0..=1e10).contains(&body.initial_cash) {
        return Err(ApiError::bad(
            "initial cash must be between 1,000 and 10,000,000,000",
        ));
    }
    let mut client = state.pool.get().await?;
    // Members own what they create; administrators create shared portfolios or ones for a
    // local user.
    let (owner, owner_name) = match (user.role, body.owner.as_deref().map(str::trim)) {
        (Role::Admin, None | Some("")) => (None, String::new()),
        (Role::Admin, Some(username)) => {
            let target = system::user_by_name(&client, username)
                .await?
                .ok_or_else(|| ApiError::bad(format!("no user named {username}")))?;
            (Some(target.username.clone()), target.username)
        }
        _ => (Some(user.username.clone()), user.display_name.clone()),
    };
    let version = match body.strategy_version_id {
        Some(id) => strategy_store::version(&client, id)
            .await?
            .ok_or_else(|| ApiError::not_found("strategy version"))?,
        None => {
            let current = match &context.book {
                Some(book) => strategy_store::active_version(&client, book.account.id).await?,
                None => None,
            };
            match current {
                Some(version) => version,
                None => strategy_store::versions(&client)
                    .await?
                    .into_iter()
                    .max_by_key(|v| v.id)
                    .ok_or_else(|| ApiError::not_found("strategy version"))?,
            }
        }
    };
    version
        .strategy_params()
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    let automation = AutomationSettings {
        mode: body.automation_mode.unwrap_or(AutomationMode::Auto),
        ..AutomationSettings::default()
    };
    let tx = client.transaction().await?;
    let book = portfolio::open(
        &state,
        &tx,
        &OpenPortfolio {
            portfolio: NewPortfolio {
                name: &name,
                description: &description,
                owner: owner.as_deref(),
                owner_name: &owner_name,
                automation: &automation,
                created_by: &user.username,
            },
            initial_cash: dec(body.initial_cash, 2),
            strategy_version_id: version.id,
        },
    )
    .await?
    .ok_or_else(|| ApiError::conflict(format!("a portfolio named {name} already exists")))?;
    tx.commit().await?;
    tracing::info!(portfolio = book.portfolio.id, name = %name, owner = ?owner, actor = %user.username, "portfolio created");
    state.emit(ServerEvent::Portfolios {
        portfolio_id: book.portfolio.id,
    });
    Ok(Json(
        json!({"portfolio": view(&state, &client, user, book.portfolio, true).await?}),
    ))
}

#[derive(Deserialize)]
pub struct UpdateBody {
    name: String,
    #[serde(default)]
    description: String,
}

pub async fn update(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Json(body): Json<UpdateBody>,
) -> ApiResult<Json<Value>> {
    let name = clean_name(&body.name)?;
    let description = clean_description(&body.description)?;
    let client = state.pool.get().await?;
    let before = visible(&client, &user, id).await?;
    if !user.can_trade(&before) {
        return Err(ApiError::Forbidden(
            "you cannot change this portfolio".into(),
        ));
    }
    let portfolio = portfolios::rename(&client, id, &name, &description)
        .await?
        .ok_or_else(|| ApiError::conflict(format!("a portfolio named {name} already exists")))?;
    system::audit(
        &client,
        &user.username,
        "portfolio.updated",
        "portfolio",
        &id.to_string(),
        json!({"before": {"name": before.name, "description": before.description},
               "after": {"name": portfolio.name, "description": portfolio.description}}),
        &user.ip,
    )
    .await?;
    state.emit(ServerEvent::Portfolios { portfolio_id: id });
    Ok(Json(
        json!({"portfolio": view(&state, &client, &user, portfolio, true).await?}),
    ))
}

#[derive(Deserialize)]
pub struct ArchiveBody {
    confirm: String,
    #[serde(default)]
    reason: String,
}

/// Stops a portfolio for good: pending plans are cancelled, its account is archived and the
/// scheduler no longer trades it. Its history stays.
pub async fn archive(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Json(body): Json<ArchiveBody>,
) -> ApiResult<Json<Value>> {
    if body.confirm != "ARCHIVE" {
        return Err(ApiError::bad("type ARCHIVE to confirm"));
    }
    let _guard = state.trading_lock.lock().await;
    let mut client = state.pool.get().await?;
    let tx = client.transaction().await?;
    let portfolio = portfolios::lock(&tx, id).await?;
    if !user.can_view(&portfolio) {
        return Err(ApiError::not_found("portfolio"));
    }
    if !user.can_trade(&portfolio) {
        return Err(ApiError::Forbidden(
            "you cannot change this portfolio".into(),
        ));
    }
    let mut cancelled = Vec::new();
    if let Some(account) = trading::active_account(&tx, id).await? {
        let open = trading::open_plans(&tx, account.id).await?;
        if open.iter().any(|p| p.status == "executing") {
            return Err(ApiError::conflict(
                "a plan is executing; try again in a moment",
            ));
        }
        for plan in open {
            trading::cancel_plan(&tx, plan.id, &user.username, "portfolio archived").await?;
            cancelled.push(plan.id);
        }
        trading::archive_account(&tx, account.id).await?;
    }
    let archived = portfolios::archive(&tx, id, &user.username).await?;
    system::audit(
        &tx,
        &user.username,
        "portfolio.archived",
        "portfolio",
        &id.to_string(),
        json!({"name": archived.name, "reason": body.reason, "cancelled_plans": cancelled}),
        &user.ip,
    )
    .await?;
    tx.commit().await?;
    tracing::info!(portfolio = id, actor = %user.username, "portfolio archived");
    for plan_id in cancelled {
        state.emit(ServerEvent::Plan {
            plan_id,
            status: "cancelled".into(),
            portfolio_id: id,
        });
    }
    state.emit(ServerEvent::Portfolios { portfolio_id: id });
    Ok(Json(
        json!({"portfolio": view(&state, &client, &user, archived, false).await?}),
    ))
}
