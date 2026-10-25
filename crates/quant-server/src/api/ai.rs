//! The AI sector view: status and history, one view in full, forming one by hand, and the
//! forward evaluation of what it did to a portfolio.

use axum::Json;
use axum::extract::{Path, State};
use quant_core::strategy::AiMode;
use serde_json::{Value, json};

use super::context::PortfolioCtx;
use super::error::{ApiError, ApiResult};
use crate::auth::{AdminUser, CurrentUser};
use crate::services::ai_view;
use crate::state::SharedState;
use crate::store::ai as store;
use crate::store::{portfolios, strategy as strategy_store, system};

pub async fn overview(
    State(state): State<SharedState>,
    user: CurrentUser,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let views = store::list(&client, 30).await?;
    let latest = views
        .iter()
        .find(|v| v.status == "ok")
        .map(|v| v.id)
        .map(|id| store::get(&client, id));
    let latest = match latest {
        Some(future) => future.await?.map(|mut v| {
            // The briefing is large; the detail endpoint serves it.
            v.material = None;
            v.raw_response = None;
            v
        }),
        None => None,
    };
    // Which active portfolios the user can see consult the view, and how.
    let mut users = Vec::new();
    for book in portfolios::active_books(&client).await? {
        if !user.can_view(&book.portfolio) {
            continue;
        }
        if let Some(version) = strategy_store::active_version(&client, book.account.id).await? {
            let ai = version.strategy_params()?.ai;
            if ai.mode != AiMode::Off {
                users.push(json!({
                    "portfolio_id": book.portfolio.id,
                    "portfolio": book.portfolio.name,
                    "version_id": version.id,
                    "version": version.name,
                    "ai": ai,
                }));
            }
        }
    }
    let since = state.now() - chrono::Duration::hours(24);
    let llm = state.config.ai.llm.as_ref();
    Ok(Json(json!({
        "configured": state.llm.is_some(),
        "model": llm.map(|l| &l.model),
        "endpoint": llm.map(|l| endpoint_host(&l.base_url)),
        "origin": llm.map(|l| &l.origin),
        "ontology": state.config.ai.ontology,
        "running": ai_view::running(),
        "attempts_24h": store::attempts_since(&client, since).await?,
        "max_attempts_24h": ai_view::MAX_ATTEMPTS_PER_DAY,
        "portfolios": users,
        "latest": latest,
        "views": views,
    })))
}

/// Scheme, host and port only: enough to tell which route is used.
fn endpoint_host(base_url: &str) -> String {
    reqwest::Url::parse(base_url)
        .map(|u| {
            format!(
                "{}://{}{}",
                u.scheme(),
                u.host_str().unwrap_or_default(),
                u.port().map(|p| format!(":{p}")).unwrap_or_default()
            )
        })
        .unwrap_or_default()
}

pub async fn view_detail(
    State(state): State<SharedState>,
    _user: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let view = store::get(&client, id)
        .await?
        .ok_or_else(|| ApiError::not_found("AI view"))?;
    Ok(Json(json!({ "view": view })))
}

/// Forms a view now, in the background; the result arrives as an `ai_view` event.
pub async fn generate(
    State(state): State<SharedState>,
    AdminUser(user): AdminUser,
) -> ApiResult<Json<Value>> {
    if state.llm.is_none() {
        return Err(ApiError::bad(
            "no model is configured: set HONE_QUANT_LLM_BASE_URL or HONE_QUANT_LLM_PROFILE",
        ));
    }
    if ai_view::running() {
        return Err(ApiError::conflict(
            "an AI sector view is already being formed",
        ));
    }
    if ai_view::capped(&state).await? {
        return Err(ApiError::conflict(format!(
            "the model was already asked {} times in the last 24 hours",
            ai_view::MAX_ATTEMPTS_PER_DAY
        )));
    }
    {
        let client = state.pool.get().await?;
        system::audit(
            &client,
            &user.username,
            "ai_view.requested",
            "ai_view",
            "",
            json!({}),
            &user.ip,
        )
        .await?;
    }
    let s = state.clone();
    let actor = user.username.clone();
    tokio::spawn(async move {
        if let Err(error) = ai_view::generate(&s, &actor).await {
            tracing::warn!(error = %format!("{error:#}"), "manual AI sector view failed");
        }
    });
    Ok(Json(json!({ "started": true })))
}

pub async fn evaluation(
    State(state): State<SharedState>,
    context: PortfolioCtx,
) -> ApiResult<Json<Value>> {
    let evaluation = ai_view::evaluate(&state, context.account().id).await?;
    Ok(Json(json!({ "evaluation": evaluation })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_show_no_path_or_credentials() {
        assert_eq!(
            endpoint_host("http://127.0.0.1:8898/v1"),
            "http://127.0.0.1:8898"
        );
        assert_eq!(
            endpoint_host("https://user:pw@generativelanguage.googleapis.com/v1beta/openai"),
            "https://generativelanguage.googleapis.com"
        );
    }
}
