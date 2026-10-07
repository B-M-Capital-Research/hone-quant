//! HTTP API (`/api/*`), the SSE event stream and the embedded web UI.

pub mod context;
pub mod error;

mod auth_routes;
mod dashboard;
#[cfg(test)]
mod portfolio_tests;
mod portfolios;
mod research;
mod strategy;
mod system_routes;
mod trading;

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post, put};
use futures::Stream;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::auth::{ACTION_HEADER, CurrentUser};
use crate::state::{ServerEvent, SharedState};
use error::ApiError;

/// Header the Cloudflare Worker adds when `HONE_QUANT_ORIGIN_TOKEN` is configured.
pub const ORIGIN_TOKEN_HEADER: &str = "x-hone-quant-origin-token";

/// Mutating requests must carry `X-Hone-Quant-Action` (a cross-site form or image cannot set
/// it), must not be marked cross-site by the browser's fetch metadata, and — when the public
/// URL is configured — must come from that origin. Together with SameSite=Strict session
/// cookies this closes CSRF without tokens in the page.
async fn require_action_header(
    State(public_origin): State<Option<Arc<str>>>,
    request: Request,
    next: Next,
) -> Response {
    let safe = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if !safe {
        let headers = request.headers();
        if !headers.contains_key(ACTION_HEADER) {
            return ApiError::Forbidden("missing X-Hone-Quant-Action header".into())
                .into_response();
        }
        if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok())
            && !matches!(site, "same-origin" | "none")
        {
            return ApiError::Forbidden("cross-site request refused".into()).into_response();
        }
        if let (Some(expected), Some(origin)) = (
            public_origin.as_deref(),
            headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()),
        ) && origin != expected
        {
            return ApiError::Forbidden("request from another origin refused".into())
                .into_response();
        }
    }
    next.run(request).await
}

/// `scheme://host[:port]` of `HONE_QUANT_PUBLIC_URL`, for the Origin check.
fn public_origin(public_url: Option<&str>) -> Option<Arc<str>> {
    let url = reqwest::Url::parse(public_url?).ok()?;
    let origin = url.origin().ascii_serialization();
    (origin != "null").then(|| origin.into())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Before routing: the optional origin token, then the base path. Requests under the base
/// path are passed on with the prefix removed; the bare prefix redirects to `prefix/`; anything
/// else is not found. `/api/health` (and `<base>/api/health`) answer without the token so
/// local health checks keep working.
async fn front_door(
    base: Arc<str>,
    token: Option<Arc<str>>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    let health =
        path == "/api/health" || (!base.is_empty() && path == format!("{base}/api/health"));
    if !health && let Some(expected) = &token {
        let presented = request
            .headers()
            .get(ORIGIN_TOKEN_HEADER)
            .map(|v| v.as_bytes());
        if !presented.is_some_and(|p| constant_time_eq(p, expected.as_bytes())) {
            return StatusCode::NOT_FOUND.into_response();
        }
    }
    if base.is_empty() || path == "/api/health" {
        return next.run(request).await;
    }
    let query = request
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    if path == *base {
        return Redirect::permanent(&format!("{base}/{query}")).into_response();
    }
    match path.strip_prefix(&*base) {
        Some(rest) if rest.starts_with('/') => match format!("{rest}{query}").parse::<Uri>() {
            Ok(uri) => {
                *request.uri_mut() = uri;
                next.run(request).await
            }
            Err(_) => StatusCode::BAD_REQUEST.into_response(),
        },
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The complete HTTP surface: [`router`] behind the optional origin token and under the
/// configured base path (e.g. `/quant`).
pub fn app(state: SharedState) -> Router {
    let base: Arc<str> = state.config.base_path.as_str().into();
    let token: Option<Arc<str>> = state.config.origin_token.as_deref().map(Into::into);
    let inner = router(state);
    if base.is_empty() && token.is_none() {
        return inner;
    }
    Router::new()
        .fallback_service(inner)
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            front_door(base.clone(), token.clone(), request, next)
        }))
}

/// Server-sent events. Events about a portfolio reach only users who can see it.
async fn events(
    State(state): State<SharedState>,
    user: CurrentUser,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let mut receiver = state.events.subscribe();
    let mut visible = context::visible_ids(&state, &user)
        .await
        .unwrap_or(Some(Vec::new()));
    let stream = async_stream::stream! {
        yield Ok(SseEvent::default().event("hello").data("{}"));
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if let (Some(ids), Some(id)) = (visible.as_ref(), event.portfolio_id())
                        && !ids.contains(&id)
                    {
                        // A portfolio this member just created is theirs; anything else is not.
                        if !matches!(event, ServerEvent::Portfolios { .. }) {
                            continue;
                        }
                        visible = context::visible_ids(&state, &user).await.unwrap_or(Some(Vec::new()));
                        if !visible.as_ref().is_some_and(|ids| ids.contains(&id)) {
                            continue;
                        }
                    }
                    let name = serde_json::to_value(&event)
                        .ok()
                        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
                        .unwrap_or_else(|| "message".into());
                    if let Ok(data) = serde_json::to_string(&event) {
                        yield Ok(SseEvent::default().event(name).data(data));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // The client missed events; tell it to refetch everything.
                    yield Ok(SseEvent::default().event("resync").data("{}"));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

pub fn router(state: SharedState) -> Router {
    let api = Router::new()
        // Public.
        .route("/health", get(dashboard::health))
        .route("/meta", get(dashboard::meta))
        .route("/auth/login", post(auth_routes::login))
        // Session.
        .route("/auth/logout", post(auth_routes::logout))
        .route("/auth/me", get(auth_routes::me))
        .route("/auth/password", post(auth_routes::change_password))
        .route(
            "/users",
            get(auth_routes::list_users).post(auth_routes::create_user),
        )
        .route("/users/{id}", delete(auth_routes::delete_user))
        .route("/events", get(events))
        // Overview & market.
        .route("/dashboard", get(dashboard::dashboard))
        .route("/market", get(dashboard::market))
        .route("/board", get(dashboard::board))
        .route("/bars/{symbol}", get(dashboard::bars))
        .route("/quotes", get(dashboard::quotes))
        // Trading.
        .route("/plans", get(trading::list_plans))
        .route("/plans/generate", post(trading::generate_plan))
        .route("/plans/{id}", get(trading::plan_detail))
        .route("/plans/{id}/approve", post(trading::approve_plan))
        .route("/plans/{id}/cancel", post(trading::cancel_plan))
        .route(
            "/plans/{id}/orders/{order_id}/skip",
            post(trading::skip_order),
        )
        .route("/trading-days/{date}", get(trading::trading_day))
        .route("/trading-days/{date}/cancel", post(trading::cancel_day))
        .route(
            "/trading-days/{date}/cancel/{slot}",
            delete(trading::restore_slot),
        )
        .route(
            "/automation",
            get(trading::get_automation).put(trading::put_automation),
        )
        .route(
            "/restrictions",
            get(trading::list_restrictions).post(trading::add_restriction),
        )
        .route("/restrictions/{id}", delete(trading::revoke_restriction))
        .route("/orders", get(trading::list_orders))
        .route("/fills", get(trading::list_fills))
        .route("/ledger", get(trading::ledger))
        .route("/accounts", get(trading::accounts))
        .route("/account/reset", post(trading::reset_account))
        // Portfolios.
        .route(
            "/portfolios",
            get(portfolios::list).post(portfolios::create),
        )
        .route(
            "/portfolios/{id}",
            get(portfolios::detail).put(portfolios::update),
        )
        .route("/portfolios/{id}/archive", post(portfolios::archive))
        // Strategy & universe.
        .route("/strategy", get(strategy::overview))
        .route("/strategy/preview", post(strategy::preview))
        .route("/strategy/versions", post(strategy::create_version))
        .route(
            "/strategy/versions/{id}/activate",
            post(strategy::activate_version),
        )
        .route("/universe", get(strategy::universe))
        .route("/universe/check", post(strategy::universe_check))
        .route("/universe/apply", post(strategy::universe_apply))
        // Research.
        .route(
            "/backtests",
            get(research::list_backtests).post(research::create_backtest),
        )
        .route(
            "/backtests/{id}",
            get(research::backtest_detail).delete(research::delete_backtest),
        )
        .route("/performance", get(research::performance))
        // System.
        .route("/notifications", get(system_routes::notifications))
        .route("/notifications/read-all", post(system_routes::read_all))
        .route("/notifications/{id}/read", post(system_routes::read_one))
        .route(
            "/reminders",
            get(system_routes::reminders).post(system_routes::create_reminder),
        )
        .route(
            "/reminders/{id}",
            put(system_routes::update_reminder).delete(system_routes::delete_reminder),
        )
        .route("/settings", get(system_routes::settings))
        .route("/settings/{section}", put(system_routes::put_settings))
        .route("/channels", get(system_routes::channels))
        .route(
            "/channels/{name}",
            put(system_routes::put_channel).delete(system_routes::delete_channel),
        )
        .route("/channels/{name}/test", post(system_routes::test_channel))
        .route("/audit", get(system_routes::audit))
        .route("/jobs", get(system_routes::jobs))
        .route("/data/status", get(system_routes::data_status))
        .route("/data/sync", post(system_routes::data_sync))
        .route("/data/fmp-check", get(system_routes::fmp_check))
        .fallback(|| async { ApiError::not_found("endpoint") })
        .layer(middleware::from_fn_with_state(
            public_origin(state.config.public_url.as_deref()),
            require_action_header,
        ))
        .with_state(state.clone());

    Router::new()
        .nest("/api", api)
        .fallback(crate::web::serve)
        .with_state(state)
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static(
                "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
            ),
        ))
        .layer(tower_http::compression::CompressionLayer::new())
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::routing::{get, post};
    use tower::ServiceExt;

    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn inner() -> Router {
        Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .route("/api/x", get(|uri: Uri| async move { uri.to_string() }))
            .fallback(|uri: Uri| async move { format!("spa {uri}") })
    }

    fn wrapped(base: &str, token: Option<&str>) -> Router {
        let base: Arc<str> = base.into();
        let token: Option<Arc<str>> = token.map(Into::into);
        Router::new()
            .fallback_service(inner())
            .layer(middleware::from_fn(move |r: Request, n: Next| {
                front_door(base.clone(), token.clone(), r, n)
            }))
    }

    async fn send(app: &Router, request: Request) -> (StatusCode, String, Option<String>) {
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_string());
        let body = axum::body::to_bytes(response.into_body(), 1 << 16)
            .await
            .unwrap();
        (
            status,
            String::from_utf8_lossy(&body).into_owned(),
            location,
        )
    }

    fn get_req(path: &str, token: Option<&str>) -> Request {
        let mut builder = Request::builder().uri(path);
        if let Some(t) = token {
            builder = builder.header(ORIGIN_TOKEN_HEADER, t);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn base_path_is_stripped_and_enforced() {
        let app = wrapped("/quant", None);
        assert_eq!(
            send(&app, get_req("/quant/api/x?a=1", None)).await.1,
            "/api/x?a=1"
        );
        assert_eq!(
            send(&app, get_req("/quant/plans/7", None)).await.1,
            "spa /plans/7"
        );
        assert_eq!(send(&app, get_req("/quant/", None)).await.1, "spa /");
        let (status, _, location) = send(&app, get_req("/quant?lang=zh", None)).await;
        assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
        assert_eq!(location.as_deref(), Some("/quant/?lang=zh"));
        for outside in ["/quantum", "/api/x", "/", "/other/quant/api/x"] {
            assert_eq!(
                send(&app, get_req(outside, None)).await.0,
                StatusCode::NOT_FOUND,
                "{outside}"
            );
        }
        assert_eq!(send(&app, get_req("/api/health", None)).await.1, "ok");
        assert_eq!(send(&app, get_req("/quant/api/health", None)).await.1, "ok");
    }

    #[tokio::test]
    async fn origin_token_guards_everything_but_health() {
        let app = wrapped("/quant", Some(TOKEN));
        assert_eq!(
            send(&app, get_req("/quant/api/x", None)).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(&app, get_req("/quant/api/x", Some("wrong"))).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(&app, get_req("/quant/api/x", Some(TOKEN))).await.0,
            StatusCode::OK
        );
        assert_eq!(
            send(&app, get_req("/quant/", None)).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(&app, get_req("/quant/api/health", None)).await.0,
            StatusCode::OK
        );
        assert_eq!(
            send(&app, get_req("/api/health", None)).await.0,
            StatusCode::OK
        );
        // Without a base path the token still applies.
        let root = wrapped("", Some(TOKEN));
        assert_eq!(
            send(&root, get_req("/api/x", None)).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(&root, get_req("/api/x", Some(TOKEN))).await.0,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn state_changes_need_the_action_header_and_same_origin() {
        let app = Router::new()
            .route(
                "/api/do",
                post(|| async { "done" }).get(|| async { "read" }),
            )
            .layer(middleware::from_fn_with_state(
                public_origin(Some("https://hone-claw.com/quant")),
                require_action_header,
            ));
        let post_req = |headers: &[(&str, &str)]| {
            let mut builder = Request::builder().method(Method::POST).uri("/api/do");
            for (k, v) in headers {
                builder = builder.header(*k, *v);
            }
            builder.body(Body::empty()).unwrap()
        };
        let action = ("x-hone-quant-action", "1");
        assert_eq!(send(&app, get_req("/api/do", None)).await.0, StatusCode::OK);
        assert_eq!(send(&app, post_req(&[])).await.0, StatusCode::FORBIDDEN);
        assert_eq!(send(&app, post_req(&[action])).await.0, StatusCode::OK);
        let same = [
            action,
            ("sec-fetch-site", "same-origin"),
            ("origin", "https://hone-claw.com"),
        ];
        assert_eq!(send(&app, post_req(&same)).await.0, StatusCode::OK);
        for site in ["cross-site", "same-site"] {
            let request = post_req(&[action, ("sec-fetch-site", site)]);
            assert_eq!(send(&app, request).await.0, StatusCode::FORBIDDEN, "{site}");
        }
        let foreign = [action, ("origin", "https://evil.example")];
        assert_eq!(
            send(&app, post_req(&foreign)).await.0,
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn public_origin_is_scheme_host_and_port() {
        assert_eq!(
            public_origin(Some("https://hone-claw.com/quant")).as_deref(),
            Some("https://hone-claw.com")
        );
        assert_eq!(
            public_origin(Some("http://127.0.0.1:8090/")).as_deref(),
            Some("http://127.0.0.1:8090")
        );
        assert_eq!(public_origin(None), None);
        assert_eq!(public_origin(Some("not a url")), None);
    }
}
