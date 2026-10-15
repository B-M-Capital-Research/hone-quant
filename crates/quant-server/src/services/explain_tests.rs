//! Plan explanations end to end on the demo market: the explanation stored at generation equals
//! the one computed when an older plan (without one) is read, and lists get its headline.

use chrono::{Duration, TimeZone, Utc};
use quant_core::calendar::MarketCalendar;
use quant_core::explain::{OrderDriver, PlanDriver, PlanExplanation};
use serde_json::Value;

use crate::services::{bootstrap, broker, explain, marketdata, planner};
use crate::store::portfolios;
use crate::store::trading;
use crate::testkit;

#[tokio::test]
async fn plans_store_an_explanation_that_matches_one_computed_on_read() {
    // Monday 2026-10-05, 11:00 New York: the session is open.
    let now = Utc.with_ymd_and_hms(2026, 10, 5, 15, 0, 0).unwrap();
    let Some((_db, state)) = testkit::state_at(now).await else {
        eprintln!("skipped: HONE_QUANT_TEST_DATABASE_URL not set");
        return;
    };
    let today = MarketCalendar::local_date(now);
    bootstrap::run(&state).await.unwrap();
    marketdata::sync_daily(&state, 7, false).await.unwrap();
    marketdata::poll_quotes(&state).await.unwrap();
    let client = state.pool.get().await.unwrap();
    let main = portfolios::active_books(&client).await.unwrap()[0].clone();
    drop(client);

    let request = |slot: &str| planner::PlanRequest {
        slot: slot.into(),
        trade_date: today,
        deadline: now + Duration::hours(4),
        actor: "test".into(),
    };
    let first = planner::generate(&state, main.portfolio.id, request("open"))
        .await
        .unwrap();
    broker::execute_plan(&state, first.plan_id, "test")
        .await
        .unwrap();
    let second = planner::generate(&state, main.portfolio.id, request("manual"))
        .await
        .unwrap();

    let client = state.pool.get().await.unwrap();
    let load = |id: i64| {
        let client = &client;
        async move {
            let plan = trading::plan(client, id).await.unwrap().unwrap();
            let diagnostics = trading::plan_diagnostics(client, id).await.unwrap();
            (plan, diagnostics)
        }
    };

    // The first plan is explained as such, every order included.
    let (plan, diagnostics) = load(first.plan_id).await;
    let stored: PlanExplanation =
        serde_json::from_value(diagnostics["explain"].clone()).expect("stored explanation");
    assert_eq!(stored.previous_plan_id, None);
    assert!(matches!(stored.drivers[0], PlanDriver::FirstPlan { .. }));
    assert_eq!(stored.orders.len(), first.orders);
    assert!(
        stored
            .orders
            .iter()
            .all(|o| o.driver == OrderDriver::FirstPlan)
    );
    assert!(diagnostics["rebalance"].get("cash_gap").is_some());
    let audit = crate::store::system::audit_entries(
        &client,
        &crate::store::system::AuditFilter {
            actor: None,
            action: Some("plan.generated".into()),
            entity_type: Some("plan".into()),
            entity_id: Some(first.plan_id.to_string()),
            before: None,
            limit: 1,
        },
    )
    .await
    .unwrap();
    assert_eq!(audit[0].detail["drivers"][0], "first_plan");

    // The second compares with the first; recomputing it from the stored diagnostics without
    // the stored explanation (as for a plan generated before explanations existed) agrees.
    let (plan2, mut diagnostics2) = load(second.plan_id).await;
    let stored2: PlanExplanation = serde_json::from_value(diagnostics2["explain"].clone()).unwrap();
    assert_eq!(stored2.previous_plan_id, Some(first.plan_id));
    diagnostics2.as_object_mut().unwrap().remove("explain");
    let recomputed = explain::explanation(&client, &plan2, &diagnostics2)
        .await
        .unwrap()
        .unwrap();
    assert_close(
        &serde_json::to_value(&recomputed).unwrap(),
        &serde_json::to_value(&stored2).unwrap(),
        "explain",
    );

    // Lists get the headline of each plan.
    let headlines = explain::headlines(&client, &[plan.clone(), plan2.clone()])
        .await
        .unwrap();
    assert_eq!(headlines.len(), 2);
    assert_eq!(
        headlines[&second.plan_id].previous_plan_id,
        Some(first.plan_id)
    );
    assert!(headlines[&first.plan_id].drivers.len() <= explain::HEADLINE_DRIVERS);
}

/// Equal up to float noise: JSONB keeps decimals, and parsing them back can move the last bit.
fn assert_close(a: &Value, b: &Value, path: &str) {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            assert!(
                (x - y).abs() <= 1e-12 * (1.0 + x.abs()),
                "{path}: {x} != {y}"
            );
        }
        (Value::Array(x), Value::Array(y)) => {
            assert_eq!(x.len(), y.len(), "{path}");
            for (k, (x, y)) in x.iter().zip(y).enumerate() {
                assert_close(x, y, &format!("{path}[{k}]"));
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            assert_eq!(x.len(), y.len(), "{path}");
            for (k, x) in x {
                assert_close(x, &y[k], &format!("{path}.{k}"));
            }
        }
        _ => assert_eq!(a, b, "{path}"),
    }
}

#[tokio::test]
async fn a_slot_skipped_by_an_error_keeps_the_error_on_the_plan() {
    let now = Utc.with_ymd_and_hms(2026, 10, 5, 15, 0, 0).unwrap();
    let Some((_db, state)) = testkit::state_at(now).await else {
        eprintln!("skipped: HONE_QUANT_TEST_DATABASE_URL not set");
        return;
    };
    bootstrap::run(&state).await.unwrap();
    let client = state.pool.get().await.unwrap();
    let main = portfolios::active_books(&client).await.unwrap()[0].clone();
    drop(client);
    let id = planner::record_skipped_slot(
        &state,
        main.portfolio.id,
        MarketCalendar::local_date(now),
        "open",
        "error",
        Some("no quotes".into()),
        now + Duration::hours(4),
    )
    .await
    .unwrap()
    .unwrap();
    let client = state.pool.get().await.unwrap();
    let plan = trading::plan(&client, id).await.unwrap().unwrap();
    assert_eq!(plan.status, "skipped");
    assert_eq!(plan.error.as_deref(), Some("no quotes"));
    assert_eq!(plan.summary["skip_reason"], "error");
}
