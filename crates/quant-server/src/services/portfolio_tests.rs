//! Portfolios end to end on the demo market: two portfolios trade the same session without
//! sharing holdings, cash, plans, automation, restrictions or cancelled slots.

use chrono::{Duration, TimeZone, Utc};
use quant_core::calendar::MarketCalendar;
use rust_decimal::Decimal;

use crate::services::portfolio::{self, OpenPortfolio};
use crate::services::{bootstrap, broker, marketdata, planner};
use crate::state::AppState;
use crate::store::portfolios::{self, Book, NewPortfolio};
use crate::store::settings::{AutomationMode, AutomationSettings};
use crate::store::strategy as strategy_store;
use crate::store::trading::{self, f};
use crate::testkit;

/// Opens a portfolio with the default strategy.
pub(crate) async fn open(
    state: &AppState,
    name: &str,
    owner: Option<&str>,
    mode: AutomationMode,
    cash: i64,
) -> Book {
    let mut client = state.pool.get().await.unwrap();
    let version = strategy_store::versions(&client)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("bootstrap created the default strategy");
    let tx = client.transaction().await.unwrap();
    let book = portfolio::open(
        state,
        &tx,
        &OpenPortfolio {
            portfolio: NewPortfolio {
                name,
                description: "",
                owner,
                owner_name: owner.unwrap_or(""),
                automation: &AutomationSettings {
                    mode,
                    ..AutomationSettings::default()
                },
                created_by: owner.unwrap_or("admin"),
            },
            initial_cash: Decimal::from(cash),
            strategy_version_id: version.id,
        },
    )
    .await
    .unwrap()
    .expect("name is free");
    tx.commit().await.unwrap();
    book
}

#[tokio::test]
async fn portfolios_keep_their_books_apart() {
    // Monday 2026-10-05, 11:00 New York: the session is open.
    let now = Utc.with_ymd_and_hms(2026, 10, 5, 15, 0, 0).unwrap();
    let Some((db, state)) = testkit::state_at(now).await else {
        eprintln!("skipped: HONE_QUANT_TEST_DATABASE_URL not set");
        return;
    };
    let today = MarketCalendar::local_date(now);
    bootstrap::run(&state).await.unwrap();
    marketdata::sync_daily(&state, 7, false).await.unwrap();
    marketdata::poll_quotes(&state).await.unwrap();

    let client = state.pool.get().await.unwrap();
    let books = portfolios::active_books(&client).await.unwrap();
    assert_eq!(books.len(), 1, "bootstrap opens one shared portfolio");
    let main = books[0].clone();
    assert_eq!(main.portfolio.name, "Main");
    assert_eq!(main.portfolio.owner, None);
    drop(client);
    let alice = open(
        &state,
        "Alice",
        Some("alice"),
        AutomationMode::Approval,
        50_000,
    )
    .await;

    // Names are unique per owner, not across owners.
    {
        let mut client = state.pool.get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        let automation = AutomationSettings::default();
        let same = |owner: Option<&'static str>| NewPortfolio {
            name: " alice ",
            description: "",
            owner,
            owner_name: "",
            automation: &automation,
            created_by: "test",
        };
        assert!(
            portfolios::insert(&tx, &same(Some("alice")))
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            portfolios::insert(&tx, &same(Some("bob")))
                .await
                .unwrap()
                .is_some()
        );
        // Rolled back: the portfolio for bob is not kept.
    }

    // Both portfolios plan the same slot, each with its own automation mode.
    let request = || planner::PlanRequest {
        slot: "open".into(),
        trade_date: today,
        deadline: now + Duration::hours(4),
        actor: "test".into(),
    };
    let main_plan = planner::generate(&state, main.portfolio.id, request())
        .await
        .unwrap();
    let alice_plan = planner::generate(&state, alice.portfolio.id, request())
        .await
        .unwrap();
    assert_ne!(main_plan.plan_id, alice_plan.plan_id);
    assert!(main_plan.orders > 0 && alice_plan.orders > 0);
    let client = state.pool.get().await.unwrap();
    let main_row = trading::plan(&client, main_plan.plan_id)
        .await
        .unwrap()
        .unwrap();
    let alice_row = trading::plan(&client, alice_plan.plan_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(main_row.account_id, main.account.id);
    assert_eq!(alice_row.account_id, alice.account.id);
    assert_eq!(main_row.automation_mode, "auto");
    assert!(main_row.execute_after.is_some());
    assert_eq!(alice_row.automation_mode, "approval");
    assert!(alice_row.execute_after.is_none());
    // Generating a scheduled slot again returns the existing plan.
    let again = planner::generate(&state, alice.portfolio.id, request())
        .await
        .unwrap();
    assert_eq!(again.plan_id, alice_plan.plan_id);
    drop(client);

    // Executions touch only their own books.
    broker::execute_plan(&state, main_plan.plan_id, "test")
        .await
        .unwrap();
    broker::execute_plan(&state, alice_plan.plan_id, "test")
        .await
        .unwrap();
    let client = state.pool.get().await.unwrap();
    let main_now = trading::account(&client, main.account.id)
        .await
        .unwrap()
        .unwrap();
    let alice_now = trading::account(&client, alice.account.id)
        .await
        .unwrap()
        .unwrap();
    let main_value = portfolio::valuation(&state, &client, &main_now)
        .await
        .unwrap();
    let alice_value = portfolio::valuation(&state, &client, &alice_now)
        .await
        .unwrap();
    assert!(main_value.invested > 0.0 && alice_value.invested > 0.0);
    assert!(
        (alice_value.nav - 50_000.0).abs() < 1_000.0,
        "{}",
        alice_value.nav
    );
    assert!(
        (main_value.nav - 1_000_000.0).abs() < 20_000.0,
        "{}",
        main_value.nav
    );
    // Cash moved only by each account's own fills.
    for (account, value) in [(&main_now, &main_value), (&alice_now, &alice_value)] {
        let (fills, total) = trading::list_fills(
            &client,
            account.id,
            &trading::FillFilter {
                symbol: None,
                from: None,
                to: None,
                plan_id: None,
                limit: 1000,
                offset: 0,
            },
        )
        .await
        .unwrap();
        assert!(total > 0);
        let spent: f64 = fills
            .iter()
            .map(|fill| {
                let gross = f(fill.notional) + f(fill.commission) + f(fill.fees);
                if fill.side == "buy" { gross } else { -gross }
            })
            .sum();
        let initial = f(account.initial_cash);
        assert!((initial - spent - value.cash).abs() < 0.05, "{account:?}");
    }
    let all_fills: i64 = client
        .query_one("SELECT count(*) FROM fills", &[])
        .await
        .unwrap()
        .get(0);
    let main_fills = trading::list_fills(
        &client,
        main.account.id,
        &trading::FillFilter {
            symbol: None,
            from: None,
            to: None,
            plan_id: None,
            limit: 1,
            offset: 0,
        },
    )
    .await
    .unwrap()
    .1;
    let alice_fills = trading::list_fills(
        &client,
        alice.account.id,
        &trading::FillFilter {
            symbol: None,
            from: None,
            to: None,
            plan_id: None,
            limit: 1,
            offset: 0,
        },
    )
    .await
    .unwrap()
    .1;
    assert_eq!(main_fills + alice_fills, all_fills);

    // A portfolio's own restriction affects only that portfolio; a universe-wide one wins.
    let symbol = alice_value.positions[0].symbol.clone();
    strategy_store::add_restriction(
        &client,
        Some(alice.portfolio.id),
        &symbol,
        "exclude",
        "",
        today,
        None,
        "alice",
    )
    .await
    .unwrap();
    let params = strategy_store::active_version(&client, alice.account.id)
        .await
        .unwrap()
        .unwrap()
        .strategy_params()
        .unwrap();
    let alice_proposal = planner::build_proposal(&state, &alice_now, &params, today)
        .await
        .unwrap();
    let main_proposal = planner::build_proposal(&state, &main_now, &params, today)
        .await
        .unwrap();
    assert!(alice_proposal.excluded.iter().any(|x| x.symbol == symbol));
    assert!(!main_proposal.excluded.iter().any(|x| x.symbol == symbol));
    strategy_store::add_restriction(&client, None, &symbol, "lock", "", today, None, "admin")
        .await
        .unwrap();
    let alice_proposal = planner::build_proposal(&state, &alice_now, &params, today)
        .await
        .unwrap();
    assert!(alice_proposal.frozen.iter().any(|x| x.symbol == symbol));
    assert!(!alice_proposal.excluded.iter().any(|x| x.symbol == symbol));

    // Cancelled slots are per portfolio.
    assert!(
        strategy_store::skip_slot(&client, alice.portfolio.id, today, "close", "", "alice")
            .await
            .unwrap()
    );
    assert!(
        strategy_store::is_slot_skipped(&client, alice.portfolio.id, today, "close")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        strategy_store::is_slot_skipped(&client, main.portfolio.id, today, "close")
            .await
            .unwrap()
            .is_none()
    );

    // Every active portfolio is traded; the held symbols of all of them stay tracked.
    let held = trading::held_symbols(&client).await.unwrap();
    assert!(
        alice_value
            .positions
            .iter()
            .all(|p| held.contains(&p.symbol))
    );
    assert_eq!(portfolios::active_books(&client).await.unwrap().len(), 2);
    assert_eq!(portfolios::active_count(&client).await.unwrap(), 2);
    drop(client);
    db.drop().await;
}
