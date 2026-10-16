//! Broker-linked portfolios end to end on the demo market and an in-memory broker: linking,
//! planning on the broker's books, sells before buys, fills read back once, charges, the
//! deadline, an operator stopping a plan, and synchronisation.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use rust_decimal::Decimal;

use crate::brokers::fake::FakeBroker;
use crate::brokers::{AccountMode, Brokers, Charges, LONGBRIDGE, OrderState};
use crate::db::testing::TestDb;
use crate::services::brokerage::{self, LinkError, LinkRequest};
use crate::services::{bootstrap, broker, marketdata, planner};
use crate::state::AppState;
use crate::store::portfolios::{Book, NewPortfolio};
use crate::store::settings::{AutomationMode, AutomationSettings};
use crate::store::strategy as strategy_store;
use crate::store::trading::{self, dec, f};
use crate::store::{market, system};
use crate::testkit;

/// Monday 2026-10-05, 11:00 New York: the session is open.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 5, 15, 0, 0).unwrap()
}

async fn setup(fake: Arc<FakeBroker>) -> Option<(TestDb, Arc<AppState>)> {
    let (db, state) = testkit::state_with(now(), Brokers::default().with(fake)).await?;
    bootstrap::run(&state).await.unwrap();
    marketdata::sync_daily(&state, 7, false).await.unwrap();
    marketdata::poll_quotes(&state).await.unwrap();
    Some((db, state))
}

async fn link(state: &Arc<AppState>, name: &str) -> Result<Book, LinkError> {
    let prepared = brokerage::prepare_link(state, LONGBRIDGE, AccountMode::Paper).await?;
    let mut client = state.pool.get().await.unwrap();
    let version = strategy_store::versions(&client)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let tx = client.transaction().await.unwrap();
    let book = brokerage::open_linked(
        state,
        &tx,
        &LinkRequest {
            portfolio: NewPortfolio {
                name,
                description: "",
                owner: None,
                owner_name: "",
                automation: &AutomationSettings {
                    mode: AutomationMode::Approval,
                    ..AutomationSettings::default()
                },
                created_by: "admin",
            },
            broker: LONGBRIDGE,
            mode: AccountMode::Paper,
            strategy_version_id: version.id,
        },
        &prepared,
    )
    .await?
    .expect("name is free");
    tx.commit().await.unwrap();
    Ok(book)
}

/// The demo market's current price of every symbol.
async fn prices(state: &AppState) -> HashMap<String, Decimal> {
    let client = state.pool.get().await.unwrap();
    market::quotes(&client)
        .await
        .unwrap()
        .into_iter()
        .map(|(symbol, q)| (symbol, dec(q.quote.price, 2)))
        .collect()
}

async fn plan_status(state: &AppState, plan_id: i64) -> String {
    let client = state.pool.get().await.unwrap();
    trading::plan(&client, plan_id)
        .await
        .unwrap()
        .unwrap()
        .status
}

async fn generate(state: &Arc<AppState>, book: &Book) -> i64 {
    generate_unchecked(state, book).await.unwrap()
}

async fn generate_unchecked(state: &Arc<AppState>, book: &Book) -> anyhow::Result<i64> {
    let session = state.calendar.session(now().date_naive()).unwrap();
    planner::generate(
        state,
        book.portfolio.id,
        planner::PlanRequest {
            slot: "manual".into(),
            trade_date: now().date_naive(),
            deadline: session.close - Duration::minutes(5),
            actor: "admin".into(),
        },
    )
    .await
    .map(|p| p.plan_id)
}

#[tokio::test]
async fn a_linked_portfolio_trades_at_the_broker() {
    let fake = Arc::new(FakeBroker::paper(400_000).at(now()));
    // A holding far above the single-name cap: the plan sells part of it.
    fake.hold("NVDA", 2000, Decimal::from(100));
    fake.hold_foreign("700.HK", 100);
    let Some((db, state)) = setup(fake.clone()).await else {
        eprintln!("skipped: HONE_QUANT_TEST_DATABASE_URL not set");
        return;
    };
    let quotes = prices(&state).await;

    // Linking adopts the US-dollar cash and the universe holdings; the rest is reported.
    let book = link(&state, "Longbridge paper").await.unwrap();
    let account = &book.account;
    assert_eq!(account.broker, "longbridge");
    assert_eq!(account.mode, "paper");
    assert_eq!(account.broker_account.as_deref(), Some("lb_papertrading"));
    assert_eq!(account.cash, Decimal::from(400_000));
    let nvda_value = Decimal::from(2000) * quotes["NVDA"];
    assert_eq!(account.initial_cash, Decimal::from(400_000) + nvda_value);
    assert_eq!(
        account.sync_status["unmanaged"][0]["broker_symbol"],
        "700.HK"
    );
    let client = state.pool.get().await.unwrap();
    let positions = trading::positions(&client, account.id, false)
        .await
        .unwrap();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].qty, Decimal::from(2000));
    assert_eq!(positions[0].avg_cost, Decimal::from(100));
    drop(client);

    // One portfolio per broker account.
    assert!(matches!(
        link(&state, "Second").await,
        Err(LinkError::AlreadyLinked(id)) if id == book.portfolio.id
    ));

    // The plan is sized on the broker's books and waits for approval.
    let plan_id = generate(&state, &book).await;
    assert_eq!(plan_status(&state, plan_id).await, "pending");
    let client = state.pool.get().await.unwrap();
    let orders = trading::orders_for_plan(&client, plan_id).await.unwrap();
    drop(client);
    assert!(!orders.is_empty());
    let sells = orders.iter().filter(|o| o.side == "sell").count();
    let buys = orders.len() - sells;
    assert!(sells > 0, "the oversized holding is trimmed");
    assert!(buys > 0, "400k of cash is put to work");

    // Executing sends the sells first; the buys wait for them.
    let report = broker::execute_plan(&state, plan_id, "admin")
        .await
        .unwrap();
    assert_eq!(report.status, "executing");
    assert_eq!(plan_status(&state, plan_id).await, "executing");
    let working = fake.orders();
    assert_eq!(working.len(), sells);
    assert!(
        working
            .iter()
            .all(|o| o.side == quant_core::costs::Side::Sell)
    );
    // Still working: the buys keep waiting.
    brokerage::reconcile_account(&state, account.id)
        .await
        .unwrap();
    assert_eq!(fake.orders().len(), sells);
    for order in &working {
        let limit = order.limit_price.unwrap();
        assert!(
            (limit * Decimal::from(100)).fract().is_zero(),
            "{limit} is in cents"
        );
        assert!(order.remark.starts_with("hq-"));
    }

    // Every broker fill is booked once, buys follow the sells, the plan ends.
    fake.charge(Some(Charges {
        commission: Decimal::new(99, 2),
        fees: Decimal::new(3, 2),
    }));
    for _ in 0..4 {
        fake.fill_working(|symbol| quotes[symbol]);
        brokerage::reconcile_account(&state, account.id)
            .await
            .unwrap();
        brokerage::reconcile_account(&state, account.id)
            .await
            .unwrap();
        if plan_status(&state, plan_id).await != "executing" {
            break;
        }
    }
    // The last buys can be trimmed to the cash the broker reports as available.
    let status = plan_status(&state, plan_id).await;
    assert!(
        matches!(status.as_str(), "executed" | "partially_executed"),
        "{status}"
    );
    assert_eq!(fake.orders().len(), orders.len(), "each order sent once");
    assert!(fake.orders().iter().all(|o| o.state == OrderState::Filled));

    let client = state.pool.get().await.unwrap();
    let orders = trading::orders_for_plan(&client, plan_id).await.unwrap();
    assert!(
        orders.iter().all(|o| o.status == "filled"
            || (o.status == "partially_filled"
                && o.status_reason.as_deref() == Some("reduced_for_cash"))),
        "{orders:#?}"
    );
    assert!(orders.iter().all(|o| o.broker_order_id.is_some()));
    let (fills, total) = trading::list_fills(
        &client,
        account.id,
        &trading::FillFilter {
            symbol: None,
            from: None,
            to: None,
            plan_id: Some(plan_id),
            limit: 1000,
            offset: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(total as usize, orders.len());
    assert!(fills.iter().all(|f| f.broker_trade_id.is_some()));
    // The broker's charges are on the fills.
    assert!(
        fills
            .iter()
            .all(|f| f.commission == Decimal::new(99, 2) && f.fees == Decimal::new(3, 2))
    );

    // The books are the broker's: no synchronisation difference is left.
    let account = trading::account(&client, account.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.cash, fake.cash().round_dp(2));
    for p in trading::positions(&client, account.id, false)
        .await
        .unwrap()
    {
        assert_eq!(p.qty, fake.qty(&p.symbol), "{}", p.symbol);
    }
    assert_eq!(account.sync_status["ok"], true);
    assert_eq!(
        account.sync_status["adjustments"].as_array().map(Vec::len),
        Some(0),
        "{}",
        account.sync_status
    );
    let plan = trading::plan(&client, plan_id).await.unwrap().unwrap();
    assert_eq!(plan.summary["executed"]["broker"], true);
    drop(client);

    // Reading back again books nothing twice.
    brokerage::reconcile(&state).await.unwrap();
    let client = state.pool.get().await.unwrap();
    let (_, again) = trading::list_fills(
        &client,
        account.id,
        &trading::FillFilter {
            symbol: None,
            from: None,
            to: None,
            plan_id: Some(plan_id),
            limit: 1,
            offset: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(again, total);
    drop(client);
    db.drop().await;
}

#[tokio::test]
async fn differences_at_the_broker_are_synchronised() {
    let fake = Arc::new(FakeBroker::paper(100_000).at(now()));
    fake.hold("NVDA", 50, Decimal::from(100));
    let Some((db, state)) = setup(fake.clone()).await else {
        eprintln!("skipped: HONE_QUANT_TEST_DATABASE_URL not set");
        return;
    };
    let book = link(&state, "Sync").await.unwrap();

    // A dividend paid and shares bought outside hone-quant.
    fake.set_cash(Decimal::new(10_012_345, 2));
    fake.hold("NVDA", 60, Decimal::new(10250, 2));
    fake.hold("AMD", 10, Decimal::from(150));
    let report = brokerage::sync_account(&state, book.account.id, "admin")
        .await
        .unwrap();
    assert!(report.ok);
    let items: Vec<&str> = report.adjustments.iter().map(|a| a.item.as_str()).collect();
    assert_eq!(items, vec!["AMD", "NVDA", "cash"]);
    let client = state.pool.get().await.unwrap();
    let account = trading::account(&client, book.account.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.cash, Decimal::new(10_012_345, 2));
    assert!(account.synced_at.is_some());
    let positions = trading::positions(&client, account.id, false)
        .await
        .unwrap();
    let nvda = positions.iter().find(|p| p.symbol == "NVDA").unwrap();
    assert_eq!(nvda.qty, Decimal::from(60));
    assert_eq!(nvda.avg_cost, Decimal::new(10250, 2));
    let ledger = trading::ledger(&client, account.id, 5).await.unwrap();
    assert_eq!(ledger[0].kind, "adjustment");
    assert_eq!(ledger[0].ref_type.as_deref(), Some("broker_sync"));
    let audit = system::audit_entries(
        &client,
        &system::AuditFilter {
            actor: None,
            action: Some("account.synchronised".into()),
            entity_type: None,
            entity_id: None,
            before: None,
            limit: 5,
        },
    )
    .await
    .unwrap();
    assert_eq!(audit.len(), 1);
    drop(client);

    // A second synchronisation finds nothing new.
    let report = brokerage::sync_account(&state, book.account.id, "admin")
        .await
        .unwrap();
    assert!(report.adjustments.is_empty());

    // Signed in to another account: refused, recorded, nothing changed.
    fake.sign_in_as(AccountMode::Paper, "lb_papertrading", "paper:someone-else");
    fake.set_cash(Decimal::from(5));
    let error = brokerage::sync_account(&state, book.account.id, "admin")
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("someone-else"), "{error:#}");
    let client = state.pool.get().await.unwrap();
    let account = trading::account(&client, book.account.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.sync_status["ok"], false);
    assert_eq!(account.cash, Decimal::new(10_012_345, 2));
    drop(client);
    db.drop().await;
}

#[tokio::test]
async fn unfilled_orders_end_at_the_deadline_or_when_stopped() {
    let fake = Arc::new(FakeBroker::paper(300_000).at(now()));
    let Some((db, state)) = setup(fake.clone()).await else {
        eprintln!("skipped: HONE_QUANT_TEST_DATABASE_URL not set");
        return;
    };
    let book = link(&state, "Deadline").await.unwrap();

    // Stopped by an operator: the broker orders are cancelled, the plan ends unfilled.
    let plan_id = generate(&state, &book).await;
    let report = broker::execute_plan(&state, plan_id, "admin")
        .await
        .unwrap();
    assert!(report.submitted > 0, "all cash: buys go out at once");
    broker::cancel_plan(&state, plan_id, "admin", "changed my mind", "")
        .await
        .unwrap();
    assert_eq!(fake.cancelled(), report.submitted);
    assert_eq!(plan_status(&state, plan_id).await, "failed");
    let client = state.pool.get().await.unwrap();
    let orders = trading::orders_for_plan(&client, plan_id).await.unwrap();
    assert!(
        orders
            .iter()
            .all(|o| matches!(o.status.as_str(), "cancelled" | "rejected"))
    );
    drop(client);

    // Past the deadline: working orders are cancelled; a partial fill is kept.
    let plan_id = generate(&state, &book).await;
    broker::execute_plan(&state, plan_id, "admin")
        .await
        .unwrap();
    let first = fake
        .orders()
        .into_iter()
        .find(|o| o.state == OrderState::Working)
        .unwrap();
    fake.fill(&first.order_id, Decimal::ONE, first.limit_price.unwrap());
    let client = state.pool.get().await.unwrap();
    client
        .execute(
            "UPDATE plans SET deadline = $2 WHERE id = $1",
            &[&plan_id, &(now() - Duration::minutes(1))],
        )
        .await
        .unwrap();
    drop(client);
    brokerage::reconcile(&state).await.unwrap();
    brokerage::reconcile(&state).await.unwrap();
    assert_eq!(plan_status(&state, plan_id).await, "partially_executed");
    let client = state.pool.get().await.unwrap();
    let orders = trading::orders_for_plan(&client, plan_id).await.unwrap();
    let partial = orders
        .iter()
        .find(|o| o.broker_order_id.as_deref() == Some(first.order_id.as_str()))
        .unwrap();
    assert_eq!(partial.status, "partially_filled");
    assert_eq!(partial.filled_qty, Decimal::ONE);
    let account = trading::account(&client, book.account.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.cash, fake.cash().round_dp(2));
    drop(client);
    db.drop().await;
}

#[tokio::test]
async fn a_broker_that_cannot_be_reached_fails_the_plan_without_trading() {
    let fake = Arc::new(FakeBroker::paper(200_000).at(now()));
    let Some((db, state)) = setup(fake.clone()).await else {
        eprintln!("skipped: HONE_QUANT_TEST_DATABASE_URL not set");
        return;
    };
    let book = link(&state, "Refusals").await.unwrap();

    // Every order refused by the broker: the plan fails, nothing is booked.
    let plan_id = generate(&state, &book).await;
    fake.refuse(Some("insufficient buying power"));
    let report = broker::execute_plan(&state, plan_id, "admin")
        .await
        .unwrap();
    assert_eq!(report.status, "failed");
    let client = state.pool.get().await.unwrap();
    let orders = trading::orders_for_plan(&client, plan_id).await.unwrap();
    assert!(orders.iter().all(|o| o.status == "rejected"
        && o.status_reason.as_deref() == Some("broker_rejected")
        && o.broker_message.as_deref() == Some("insufficient buying power")));
    let account = trading::account(&client, book.account.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(f(account.cash), 200_000.0);
    drop(client);

    // Not signed in: the plan fails before anything is sent.
    fake.refuse(None);
    fake.sign_in_as(AccountMode::Live, "lb", "live:1");
    let plan_id = generate_unchecked(&state, &book).await;
    assert!(plan_id.is_err(), "planning needs the broker's books");
    db.drop().await;
}

#[tokio::test]
async fn live_accounts_cannot_be_linked() {
    let fake = Arc::new(FakeBroker::paper(200_000).reaching(AccountMode::Live, "lb"));
    let Some((db, state)) = setup(fake).await else {
        eprintln!("skipped: HONE_QUANT_TEST_DATABASE_URL not set");
        return;
    };
    assert!(matches!(
        link(&state, "Live").await,
        Err(LinkError::Broker(
            crate::brokers::BrokerError::WrongAccount(_)
        ))
    ));
    assert!(matches!(
        brokerage::prepare_link(&state, LONGBRIDGE, AccountMode::Live).await,
        Err(LinkError::LiveDisabled)
    ));
    db.drop().await;
}
