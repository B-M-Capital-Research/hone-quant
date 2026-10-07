//! The portfolio API through the real router: visibility, the right to act, the selected
//! portfolio, and per-portfolio automation and restrictions.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use chrono::{Duration, TimeZone, Utc};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::context::PORTFOLIO_HEADER;
use crate::auth;
use crate::notify::{self, Event};
use crate::services::bootstrap;
use crate::state::AppState;
use crate::store::{portfolios, system};
use crate::testkit;

/// Creates a local user with a session; returns the session cookie.
async fn sign_in(state: &AppState, username: &str, role: &str) -> String {
    let client = state.pool.get().await.unwrap();
    let user = system::create_user(&client, username, "unused", role)
        .await
        .unwrap();
    let token = auth::new_token();
    system::create_session(
        &client,
        &auth::token_hash(&token),
        user.id,
        Utc::now() + Duration::days(1),
        "test",
        "",
    )
    .await
    .unwrap();
    format!("{}={token}", auth::SESSION_COOKIE)
}

struct Client {
    app: axum::Router,
    cookie: String,
}

impl Client {
    async fn send(
        &self,
        method: Method,
        path: &str,
        portfolio: Option<i64>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method.clone())
            .uri(path)
            .header("cookie", &self.cookie);
        if method != Method::GET {
            builder = builder.header(auth::ACTION_HEADER, "1");
        }
        if let Some(id) = portfolio {
            builder = builder.header(PORTFOLIO_HEADER, id.to_string());
        }
        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string())),
            None => builder.body(Body::empty()),
        }
        .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 22)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn get(&self, path: &str, portfolio: Option<i64>) -> (StatusCode, Value) {
        self.send(Method::GET, path, portfolio, None).await
    }

    async fn post(&self, path: &str, portfolio: Option<i64>, body: Value) -> (StatusCode, Value) {
        self.send(Method::POST, path, portfolio, Some(body)).await
    }
}

fn ids(list: &Value) -> Vec<i64> {
    list["portfolios"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn members_see_and_run_only_their_own_portfolios() {
    let now = Utc.with_ymd_and_hms(2026, 10, 5, 15, 0, 0).unwrap();
    let Some((db, state)) = testkit::state_at(now).await else {
        eprintln!("skipped: HONE_QUANT_TEST_DATABASE_URL not set");
        return;
    };
    bootstrap::run(&state).await.unwrap();
    let client = state.pool.get().await.unwrap();
    let main = portfolios::list(&client, false).await.unwrap()[0].id;
    drop(client);
    let app = super::router(state.clone());
    let as_user = |cookie: String| Client {
        app: app.clone(),
        cookie,
    };
    let admin = as_user(sign_in(&state, "root", "admin").await);
    let alice = as_user(sign_in(&state, "alice", "member").await);
    let viewer = as_user(sign_in(&state, "val", "viewer").await);

    // A new member sees nothing yet and may create a portfolio.
    let (status, list) = alice.get("/api/portfolios", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ids(&list).is_empty());
    assert_eq!(list["can_create"], true);
    let (status, body) = alice.get("/api/dashboard", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "no_portfolio");
    // The market bar still works without a portfolio.
    assert_eq!(alice.get("/api/market", None).await.0, StatusCode::OK);

    let (status, created) = alice
        .post(
            "/api/portfolios",
            None,
            json!({"name": "Alice growth", "initial_cash": 100000, "automation_mode": "approval", "owner": "root"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let mine = created["portfolio"]["id"].as_i64().unwrap();
    assert_eq!(
        created["portfolio"]["owner"], "alice",
        "members always own what they create"
    );
    assert_eq!(created["portfolio"]["can_trade"], true);
    assert_eq!(created["portfolio"]["effective_mode"], "approval");
    assert_eq!(created["portfolio"]["summary"]["nav"], 100000.0);

    // Without a header a member lands in their own portfolio; another one is not there for them.
    let (_, dashboard) = alice.get("/api/dashboard", None).await;
    assert_eq!(dashboard["portfolio"]["id"], mine);
    let (status, body) = alice.get("/api/dashboard", Some(main)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "portfolio_not_found");
    let (status, _) = alice
        .post(
            &format!("/api/portfolios/{main}/archive"),
            None,
            json!({"confirm": "ARCHIVE"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Administrators and viewers see both; only administrators act on any of them.
    assert_eq!(
        ids(&admin.get("/api/portfolios", None).await.1),
        vec![main, mine]
    );
    let (_, list) = viewer.get("/api/portfolios", None).await;
    assert_eq!(ids(&list), vec![main, mine]);
    assert_eq!(list["can_create"], false);
    assert_eq!(list["portfolios"][1]["can_trade"], false);
    let (status, _) = viewer
        .post(
            "/api/portfolios",
            None,
            json!({"name": "Mine", "initial_cash": 5000}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let paused = json!({"mode": "paused", "paused_until": null, "note": "holiday"});
    let (status, _) = viewer
        .send(
            Method::PUT,
            "/api/automation",
            Some(mine),
            Some(paused.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Automation is per portfolio.
    let (status, _) = admin
        .send(Method::PUT, "/api/automation", Some(mine), Some(paused))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        alice.get("/api/automation", None).await.1["effective_mode"],
        "paused"
    );
    assert_eq!(
        admin.get("/api/automation", Some(main)).await.1["effective_mode"],
        "auto"
    );

    // Members restrict their own portfolio only.
    let (status, _) = alice
        .post(
            "/api/restrictions",
            None,
            json!({"symbol": "NVDA", "mode": "exclude", "scope": "all"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = alice
        .post(
            "/api/restrictions",
            None,
            json!({"symbol": "NVDA", "mode": "exclude"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["restriction"]["portfolio_id"], mine);
    let (_, main_restrictions) = admin.get("/api/restrictions", Some(main)).await;
    assert!(main_restrictions["active"].as_array().unwrap().is_empty());

    // Plans are reached by id in their own portfolio, by those who may see it.
    let client = state.pool.get().await.unwrap();
    let main_account = crate::store::trading::active_account(&client, main)
        .await
        .unwrap()
        .unwrap();
    let plan_id: i64 = client
        .query_one(
            "INSERT INTO plans (account_id, trade_date, slot, status, automation_mode, deadline)
             VALUES ($1, '2026-10-05', 'manual', 'no_action', 'auto', now()) RETURNING id",
            &[&main_account.id],
        )
        .await
        .unwrap()
        .get(0);
    drop(client);
    let (status, detail) = admin
        .get(&format!("/api/plans/{plan_id}"), Some(mine))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["portfolio"]["id"], main);
    assert_eq!(detail["can_trade"], true);
    assert_eq!(
        alice.get(&format!("/api/plans/{plan_id}"), None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        alice
            .post(&format!("/api/plans/{plan_id}/cancel"), None, json!({}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    // Notifications about another portfolio stay out of a member's inbox.
    let client = state.pool.get().await.unwrap();
    let main_tag = portfolios::get(&client, main).await.unwrap().unwrap().tag();
    drop(client);
    notify::notify_in(&state, &main_tag, Event::DataStale { minutes: 1 })
        .await
        .unwrap();
    notify::notify(&state, Event::DataStale { minutes: 2 })
        .await
        .unwrap();
    let (_, inbox) = alice.get("/api/notifications", None).await;
    let rows = inbox["notifications"].as_array().unwrap();
    assert!(rows.iter().all(|n| n["portfolio_id"] != main));
    assert!(rows.iter().any(|n| n["portfolio_id"].is_null()));
    let (_, inbox) = admin.get("/api/notifications", None).await;
    let tagged = inbox["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["portfolio_id"] == main)
        .expect("administrators see every portfolio's notifications")
        .clone();
    assert_eq!(tagged["portfolio_name"], "Main");
    assert!(tagged["title_en"].as_str().unwrap().starts_with("[Main] "));

    // Administrators create for a local user, with names unique per owner.
    let (status, _) = admin
        .post(
            "/api/portfolios",
            None,
            json!({"name": "alice growth", "initial_cash": 5000, "owner": "alice"}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = admin
        .post(
            "/api/portfolios",
            None,
            json!({"name": "X", "initial_cash": 5000, "owner": "nobody"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Archiving hides a portfolio from the list and from the context.
    let (status, archived) = alice
        .post(
            &format!("/api/portfolios/{mine}/archive"),
            None,
            json!({"confirm": "ARCHIVE"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{archived}");
    assert_eq!(archived["portfolio"]["status"], "archived");
    assert_eq!(archived["portfolio"]["can_trade"], false);
    assert!(ids(&alice.get("/api/portfolios", None).await.1).is_empty());
    assert_eq!(
        ids(&alice
            .get("/api/portfolios?include_archived=true", None)
            .await
            .1),
        vec![mine]
    );
    assert_eq!(
        alice.get("/api/dashboard", Some(mine)).await.1["error"],
        "portfolio_not_found"
    );
    db.drop().await;
}
