//! Sign-in, sign-out, password changes and operator management.

use axum::Json;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use chrono::Duration;
use serde::Deserialize;
use serde_json::json;

use super::error::{ApiError, ApiResult};
use crate::auth::{self, AdminUser, CurrentUser, Role};
use crate::state::SharedState;
use crate::store::system;

#[derive(Deserialize)]
pub struct LoginBody {
    username: String,
    password: String,
}

pub async fn login(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> ApiResult<Response> {
    if state.honeclaw.is_some() {
        return Err(ApiError::Forbidden(
            "sign in at hone-claw.com with an administrator account".into(),
        ));
    }
    let ip = auth::client_ip(&headers, Some(peer));
    let user_key = format!("u:{}", body.username.trim().to_lowercase());
    let ip_key = format!("ip:{ip}");
    for key in [&user_key, &ip_key] {
        if let Err(remaining) = state.limiter.check(key) {
            return Err(ApiError::TooManyRequests(remaining.as_secs()));
        }
    }
    let client = state.pool.get().await?;
    let user = system::user_by_name(&client, body.username.trim()).await?;
    let valid = match &user {
        Some(u) => auth::verify_password(&body.password, &u.password_hash),
        None => {
            // Spend comparable time so a missing user is not distinguishable by timing.
            let _ = auth::verify_password(&body.password, auth::dummy_hash());
            false
        }
    };
    let Some(user) = user.filter(|_| valid) else {
        state.limiter.record_failure(&user_key);
        state.limiter.record_failure(&ip_key);
        system::audit(
            &client,
            body.username.trim(),
            "auth.login_failed",
            "user",
            body.username.trim(),
            json!({}),
            &ip,
        )
        .await?;
        return Err(ApiError::Forbidden("invalid username or password".into()));
    };
    state.limiter.record_success(&user_key);
    let token = auth::new_token();
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect::<String>();
    system::create_session(
        &client,
        &auth::token_hash(&token),
        user.id,
        chrono::Utc::now() + Duration::days(auth::SESSION_TTL_DAYS),
        &agent,
        &ip,
    )
    .await?;
    system::touch_login(&client, user.id).await?;
    system::audit(
        &client,
        &user.username,
        "auth.login",
        "user",
        &user.username,
        json!({}),
        &ip,
    )
    .await?;
    let secure = auth::wants_secure_cookie(state.config.cookie_security, &headers);
    Ok((
        [(header::SET_COOKIE, auth::session_cookie(&token, secure))],
        Json(json!({"user": {"username": user.username, "role": user.role}})),
    )
        .into_response())
}

pub async fn logout(
    State(state): State<SharedState>,
    headers: HeaderMap,
    user: CurrentUser,
) -> ApiResult<Response> {
    if user.external {
        // The session belongs to honeclaw; signing out happens there.
        return Ok(Json(json!({"ok": true, "login_url": login_url(&state)})).into_response());
    }
    let client = state.pool.get().await?;
    system::delete_session(&client, &user.token_hash).await?;
    system::audit(
        &client,
        &user.username,
        "auth.logout",
        "user",
        &user.username,
        json!({}),
        &user.ip,
    )
    .await?;
    let secure = auth::wants_secure_cookie(state.config.cookie_security, &headers);
    Ok((
        [(header::SET_COOKIE, auth::clear_cookie(secure))],
        Json(json!({"ok": true})),
    )
        .into_response())
}

pub async fn me(State(state): State<SharedState>, user: CurrentUser) -> Json<serde_json::Value> {
    Json(json!({
        "user": {
            "id": user.id,
            "username": user.username,
            "display_name": user.display_name,
            "role": if user.role == Role::Admin { "admin" } else { "viewer" },
            "external": user.external,
        },
        "auth": auth_info(&state),
    }))
}

/// Sign-in mode for the UI: `local` accounts or `honeclaw` administrators.
pub fn auth_info(state: &SharedState) -> serde_json::Value {
    match &state.honeclaw {
        Some(verifier) => json!({"mode": "honeclaw", "login_url": verifier.login_url()}),
        None => json!({"mode": "local"}),
    }
}

fn login_url(state: &SharedState) -> Option<String> {
    state.honeclaw.as_ref().map(|v| v.login_url().to_string())
}

/// Operator accounts live in honeclaw in honeclaw mode; the local ones are not used.
fn local_accounts_only(state: &SharedState) -> ApiResult<()> {
    if state.honeclaw.is_some() {
        return Err(ApiError::conflict(
            "operators are managed in honeclaw: hone-quant admits hone-claw.com administrators",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct PasswordBody {
    current: String,
    new: String,
}

pub async fn change_password(
    State(state): State<SharedState>,
    user: CurrentUser,
    Json(body): Json<PasswordBody>,
) -> ApiResult<Json<serde_json::Value>> {
    local_accounts_only(&state)?;
    let client = state.pool.get().await?;
    let stored = system::user_by_name(&client, &user.username)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    if !auth::verify_password(&body.current, &stored.password_hash) {
        return Err(ApiError::Forbidden("current password is incorrect".into()));
    }
    auth::check_password_strength(&body.new).map_err(ApiError::BadRequest)?;
    let hash = auth::hash_password(&body.new)?;
    system::set_password(&client, user.id, &hash).await?;
    system::delete_user_sessions(&client, user.id, Some(&user.token_hash)).await?;
    system::audit(
        &client,
        &user.username,
        "auth.password_changed",
        "user",
        &user.username,
        json!({}),
        &user.ip,
    )
    .await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn list_users(
    State(state): State<SharedState>,
    _admin: AdminUser,
) -> ApiResult<Json<serde_json::Value>> {
    local_accounts_only(&state)?;
    let client = state.pool.get().await?;
    Ok(Json(json!({"users": system::users(&client).await?})))
}

#[derive(Deserialize)]
pub struct NewUserBody {
    username: String,
    password: String,
    role: String,
}

pub async fn create_user(
    State(state): State<SharedState>,
    AdminUser(admin): AdminUser,
    Json(body): Json<NewUserBody>,
) -> ApiResult<Json<serde_json::Value>> {
    local_accounts_only(&state)?;
    let username = body.username.trim();
    if username.is_empty()
        || username.len() > 64
        || !username
            .chars()
            .all(|c| c.is_alphanumeric() || "._-".contains(c))
    {
        return Err(ApiError::bad(
            "username must be 1–64 letters, digits, '.', '_' or '-'",
        ));
    }
    if body.role != "admin" && body.role != "viewer" {
        return Err(ApiError::bad("role must be admin or viewer"));
    }
    auth::check_password_strength(&body.password).map_err(ApiError::BadRequest)?;
    let client = state.pool.get().await?;
    if system::user_by_name(&client, username).await?.is_some() {
        return Err(ApiError::conflict("that username is taken"));
    }
    let hash = auth::hash_password(&body.password)?;
    let user = system::create_user(&client, username, &hash, &body.role).await?;
    system::audit(
        &client,
        &admin.username,
        "user.created",
        "user",
        username,
        json!({"role": body.role}),
        &admin.ip,
    )
    .await?;
    Ok(Json(json!({"user": user})))
}

pub async fn delete_user(
    State(state): State<SharedState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    local_accounts_only(&state)?;
    if id == admin.id {
        return Err(ApiError::bad("you cannot delete your own account"));
    }
    let client = state.pool.get().await?;
    let n = client
        .execute("DELETE FROM users WHERE id = $1", &[&id])
        .await?;
    if n == 0 {
        return Err(ApiError::not_found("user"));
    }
    system::audit(
        &client,
        &admin.username,
        "user.deleted",
        "user",
        &id.to_string(),
        json!({}),
        &admin.ip,
    )
    .await?;
    Ok(Json(json!({"ok": true})))
}
