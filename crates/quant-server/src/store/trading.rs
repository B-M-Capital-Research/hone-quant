//! Paper account, plans, orders, fills and valuation history.
//!
//! Money and quantities are `rust_decimal::Decimal` (PostgreSQL NUMERIC) so the ledger is exact;
//! the analytics layer converts to `f64` at its boundary.

use anyhow::{Context, Result};
use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use deadpool_postgres::GenericClient;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use serde::Serialize;
use serde_json::Value;
use tokio_postgres::Row;

pub fn dec(value: f64, dp: u32) -> Decimal {
    Decimal::from_f64(value).unwrap_or_default().round_dp(dp)
}

pub fn f(value: Decimal) -> f64 {
    value.to_f64().unwrap_or(0.0)
}

// ---------------------------------------------------------------------------------------------
// Accounts and positions
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Account {
    pub id: i64,
    pub portfolio_id: i64,
    pub name: String,
    pub base_currency: String,
    pub mode: String,
    pub initial_cash: Decimal,
    pub cash: Decimal,
    pub inception_date: NaiveDate,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

impl Account {
    fn from_row(row: &Row) -> Self {
        Self {
            id: row.get("id"),
            portfolio_id: row.get("portfolio_id"),
            name: row.get("name"),
            base_currency: row.get("base_currency"),
            mode: row.get("mode"),
            initial_cash: row.get("initial_cash"),
            cash: row.get("cash"),
            inception_date: row.get("inception_date"),
            status: row.get("status"),
            created_at: row.get("created_at"),
            archived_at: row.get("archived_at"),
        }
    }
}

const ACCOUNT_COLUMNS: &str = "id, portfolio_id, name, base_currency, mode, initial_cash, cash, inception_date, status, created_at, archived_at";

/// The portfolio's active paper account.
pub async fn active_account(
    client: &impl GenericClient,
    portfolio_id: i64,
) -> Result<Option<Account>> {
    let row = client
        .query_opt(
            &format!(
                "SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE portfolio_id = $1 AND status = 'active'"
            ),
            &[&portfolio_id],
        )
        .await?;
    Ok(row.as_ref().map(Account::from_row))
}

pub async fn account(client: &impl GenericClient, id: i64) -> Result<Option<Account>> {
    let row = client
        .query_opt(
            &format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = $1"),
            &[&id],
        )
        .await?;
    Ok(row.as_ref().map(Account::from_row))
}

/// Locks the account row for the rest of the transaction.
pub async fn lock_account(client: &impl GenericClient, id: i64) -> Result<Account> {
    let row = client
        .query_one(
            &format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = $1 FOR UPDATE"),
            &[&id],
        )
        .await
        .context("account not found")?;
    Ok(Account::from_row(&row))
}

/// A portfolio's accounts, newest first (the active one and those archived by resets).
pub async fn list_accounts(client: &impl GenericClient, portfolio_id: i64) -> Result<Vec<Account>> {
    let rows = client
        .query(
            &format!(
                "SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE portfolio_id = $1 ORDER BY id DESC"
            ),
            &[&portfolio_id],
        )
        .await?;
    Ok(rows.iter().map(Account::from_row).collect())
}

pub async fn create_account(
    client: &impl GenericClient,
    portfolio_id: i64,
    name: &str,
    initial_cash: Decimal,
    inception: NaiveDate,
) -> Result<Account> {
    let row = client
        .query_one(
            &format!(
                "INSERT INTO accounts (portfolio_id, name, initial_cash, cash, inception_date) VALUES ($1, $2, $3, $3, $4)
                 RETURNING {ACCOUNT_COLUMNS}"
            ),
            &[&portfolio_id, &name, &initial_cash, &inception],
        )
        .await?;
    let account = Account::from_row(&row);
    client
        .execute(
            "INSERT INTO cash_ledger (account_id, kind, amount, balance_after, note) VALUES ($1, 'deposit', $2, $2, 'initial paper capital')",
            &[&account.id, &initial_cash],
        )
        .await?;
    Ok(account)
}

pub async fn archive_account(client: &impl GenericClient, id: i64) -> Result<()> {
    client
        .execute(
            "UPDATE accounts SET status = 'archived', archived_at = app_now() WHERE id = $1",
            &[&id],
        )
        .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct Position {
    pub symbol: String,
    pub qty: Decimal,
    pub avg_cost: Decimal,
    pub realized_pnl: Decimal,
    pub dividends: Decimal,
    pub opened_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

fn position_from_row(row: &Row) -> Position {
    Position {
        symbol: row.get("symbol"),
        qty: row.get("qty"),
        avg_cost: row.get("avg_cost"),
        realized_pnl: row.get("realized_pnl"),
        dividends: row.get("dividends"),
        opened_at: row.get("opened_at"),
        updated_at: row.get("updated_at"),
    }
}

/// Symbols held by any active account (what market data must keep covering).
pub async fn held_symbols(client: &impl GenericClient) -> Result<Vec<String>> {
    let rows = client
        .query(
            "SELECT DISTINCT p.symbol FROM positions p JOIN accounts a ON a.id = p.account_id
             WHERE a.status = 'active' AND p.qty > 0 ORDER BY p.symbol",
            &[],
        )
        .await?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// Positions of an account; closed positions (qty 0) are kept for realized P&L history.
pub async fn positions(
    client: &impl GenericClient,
    account_id: i64,
    include_closed: bool,
) -> Result<Vec<Position>> {
    let rows = client
        .query(
            "SELECT symbol, qty, avg_cost, realized_pnl, dividends, opened_at, updated_at
             FROM positions WHERE account_id = $1 AND ($2 OR qty > 0) ORDER BY symbol",
            &[&account_id, &include_closed],
        )
        .await?;
    Ok(rows.iter().map(position_from_row).collect())
}

pub async fn position_for_update(
    client: &impl GenericClient,
    account_id: i64,
    symbol: &str,
) -> Result<Option<Position>> {
    let row = client
        .query_opt(
            "SELECT symbol, qty, avg_cost, realized_pnl, dividends, opened_at, updated_at
             FROM positions WHERE account_id = $1 AND symbol = $2 FOR UPDATE",
            &[&account_id, &symbol],
        )
        .await?;
    Ok(row.as_ref().map(position_from_row))
}

pub async fn save_position(
    client: &impl GenericClient,
    account_id: i64,
    p: &Position,
) -> Result<()> {
    client
        .execute(
            "INSERT INTO positions (account_id, symbol, qty, avg_cost, realized_pnl, dividends, opened_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, app_now())
             ON CONFLICT (account_id, symbol) DO UPDATE SET qty = EXCLUDED.qty, avg_cost = EXCLUDED.avg_cost,
               realized_pnl = EXCLUDED.realized_pnl, dividends = EXCLUDED.dividends,
               opened_at = EXCLUDED.opened_at, updated_at = app_now()",
            &[&account_id, &p.symbol, &p.qty, &p.avg_cost, &p.realized_pnl, &p.dividends, &p.opened_at],
        )
        .await?;
    Ok(())
}

/// Applies a cash movement and appends the ledger row in the same transaction.
#[allow(clippy::too_many_arguments)]
pub async fn post_cash(
    client: &impl GenericClient,
    account_id: i64,
    kind: &str,
    amount: Decimal,
    symbol: Option<&str>,
    ref_type: Option<&str>,
    ref_id: Option<String>,
    note: &str,
) -> Result<Decimal> {
    let row = client
        .query_one(
            "UPDATE accounts SET cash = cash + $2 WHERE id = $1 RETURNING cash",
            &[&account_id, &amount],
        )
        .await
        .context("cash update rejected (would the balance go negative?)")?;
    let balance: Decimal = row.get(0);
    client
        .execute(
            "INSERT INTO cash_ledger (account_id, kind, amount, balance_after, symbol, ref_type, ref_id, note)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[&account_id, &kind, &amount, &balance, &symbol, &ref_type, &ref_id, &note],
        )
        .await?;
    Ok(balance)
}

#[derive(Debug, Clone, Serialize)]
pub struct LedgerEntry {
    pub id: i64,
    pub ts: DateTime<Utc>,
    pub kind: String,
    pub amount: Decimal,
    pub balance_after: Decimal,
    pub symbol: Option<String>,
    pub ref_type: Option<String>,
    pub ref_id: Option<String>,
    pub note: String,
}

pub async fn ledger(
    client: &impl GenericClient,
    account_id: i64,
    limit: i64,
) -> Result<Vec<LedgerEntry>> {
    let rows = client
        .query(
            "SELECT id, ts, kind, amount, balance_after, symbol, ref_type, ref_id, note
             FROM cash_ledger WHERE account_id = $1 ORDER BY id DESC LIMIT $2",
            &[&account_id, &limit],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| LedgerEntry {
            id: r.get(0),
            ts: r.get(1),
            kind: r.get(2),
            amount: r.get(3),
            balance_after: r.get(4),
            symbol: r.get(5),
            ref_type: r.get(6),
            ref_id: r.get(7),
            note: r.get(8),
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Plans
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub id: i64,
    pub account_id: i64,
    pub trade_date: NaiveDate,
    pub slot: String,
    pub status: String,
    pub strategy_version_id: Option<i64>,
    pub automation_mode: String,
    pub generated_at: DateTime<Utc>,
    pub execute_after: Option<DateTime<Utc>>,
    pub deadline: DateTime<Utc>,
    pub approved_at: Option<DateTime<Utc>>,
    pub approved_by: Option<String>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub cancelled_by: Option<String>,
    pub cancel_reason: Option<String>,
    pub executed_at: Option<DateTime<Utc>>,
    pub nav: Decimal,
    pub cash: Decimal,
    pub exposure_target: Option<f64>,
    pub invested_target: Option<f64>,
    pub breadth: Option<f64>,
    pub est_vol: Option<f64>,
    pub turnover: f64,
    pub est_costs: Decimal,
    pub order_count: i32,
    pub summary: Value,
    pub error: Option<String>,
    pub created_by: String,
}

const PLAN_COLUMNS: &str = "id, account_id, trade_date, slot, status, strategy_version_id, automation_mode, generated_at,
    execute_after, deadline, approved_at, approved_by, cancelled_at, cancelled_by, cancel_reason, executed_at,
    nav, cash, exposure_target, invested_target, breadth, est_vol, turnover, est_costs, order_count, summary, error, created_by";

fn plan_from_row(r: &Row) -> Plan {
    Plan {
        id: r.get("id"),
        account_id: r.get("account_id"),
        trade_date: r.get("trade_date"),
        slot: r.get("slot"),
        status: r.get("status"),
        strategy_version_id: r.get("strategy_version_id"),
        automation_mode: r.get("automation_mode"),
        generated_at: r.get("generated_at"),
        execute_after: r.get("execute_after"),
        deadline: r.get("deadline"),
        approved_at: r.get("approved_at"),
        approved_by: r.get("approved_by"),
        cancelled_at: r.get("cancelled_at"),
        cancelled_by: r.get("cancelled_by"),
        cancel_reason: r.get("cancel_reason"),
        executed_at: r.get("executed_at"),
        nav: r.get("nav"),
        cash: r.get("cash"),
        exposure_target: r.get("exposure_target"),
        invested_target: r.get("invested_target"),
        breadth: r.get("breadth"),
        est_vol: r.get("est_vol"),
        turnover: r.get("turnover"),
        est_costs: r.get("est_costs"),
        order_count: r.get("order_count"),
        summary: r.get("summary"),
        error: r.get("error"),
        created_by: r.get("created_by"),
    }
}

pub struct NewPlan<'a> {
    pub account_id: i64,
    pub trade_date: NaiveDate,
    pub slot: &'a str,
    pub status: &'a str,
    pub strategy_version_id: Option<i64>,
    pub automation_mode: &'a str,
    pub execute_after: Option<DateTime<Utc>>,
    pub deadline: DateTime<Utc>,
    pub nav: Decimal,
    pub cash: Decimal,
    pub exposure_target: Option<f64>,
    pub invested_target: Option<f64>,
    pub breadth: Option<f64>,
    pub est_vol: Option<f64>,
    pub turnover: f64,
    pub est_costs: Decimal,
    pub order_count: i32,
    pub diagnostics: Value,
    pub summary: Value,
    pub error: Option<String>,
    pub created_by: &'a str,
}

/// Inserts a plan. For scheduled slots the unique index makes this idempotent: `None` means a
/// plan for that account/date/slot already exists.
pub async fn insert_plan(client: &impl GenericClient, p: &NewPlan<'_>) -> Result<Option<i64>> {
    let row = client
        .query_opt(
            "INSERT INTO plans (account_id, trade_date, slot, status, strategy_version_id, automation_mode,
                 execute_after, deadline, nav, cash, exposure_target, invested_target, breadth, est_vol,
                 turnover, est_costs, order_count, diagnostics, summary, error, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21)
             ON CONFLICT DO NOTHING
             RETURNING id",
            &[
                &p.account_id,
                &p.trade_date,
                &p.slot,
                &p.status,
                &p.strategy_version_id,
                &p.automation_mode,
                &p.execute_after,
                &p.deadline,
                &p.nav,
                &p.cash,
                &p.exposure_target,
                &p.invested_target,
                &p.breadth,
                &p.est_vol,
                &p.turnover,
                &p.est_costs,
                &p.order_count,
                &p.diagnostics,
                &p.summary,
                &p.error,
                &p.created_by,
            ],
        )
        .await?;
    Ok(row.map(|r| r.get(0)))
}

pub async fn plan(client: &impl GenericClient, id: i64) -> Result<Option<Plan>> {
    let row = client
        .query_opt(
            &format!("SELECT {PLAN_COLUMNS} FROM plans WHERE id = $1"),
            &[&id],
        )
        .await?;
    Ok(row.as_ref().map(plan_from_row))
}

pub async fn plan_for_update(client: &impl GenericClient, id: i64) -> Result<Option<Plan>> {
    let row = client
        .query_opt(
            &format!("SELECT {PLAN_COLUMNS} FROM plans WHERE id = $1 FOR UPDATE"),
            &[&id],
        )
        .await?;
    Ok(row.as_ref().map(plan_from_row))
}

pub async fn plan_diagnostics(client: &impl GenericClient, id: i64) -> Result<Value> {
    let row = client
        .query_one("SELECT diagnostics FROM plans WHERE id = $1", &[&id])
        .await?;
    Ok(row.get(0))
}

pub async fn plans_for_date(
    client: &impl GenericClient,
    account_id: i64,
    date: NaiveDate,
) -> Result<Vec<Plan>> {
    let rows = client
        .query(
            &format!("SELECT {PLAN_COLUMNS} FROM plans WHERE account_id = $1 AND trade_date = $2 ORDER BY generated_at"),
            &[&account_id, &date],
        )
        .await?;
    Ok(rows.iter().map(plan_from_row).collect())
}

pub async fn plan_for_slot(
    client: &impl GenericClient,
    account_id: i64,
    date: NaiveDate,
    slot: &str,
) -> Result<Option<Plan>> {
    let row = client
        .query_opt(
            &format!("SELECT {PLAN_COLUMNS} FROM plans WHERE account_id = $1 AND trade_date = $2 AND slot = $3 ORDER BY id DESC LIMIT 1"),
            &[&account_id, &date, &slot],
        )
        .await?;
    Ok(row.as_ref().map(plan_from_row))
}

/// Plans in `pending` or `executing` state of every active account.
pub async fn open_plans_all(client: &impl GenericClient) -> Result<Vec<Plan>> {
    let cols = PLAN_COLUMNS
        .split(',')
        .map(|c| format!("p.{}", c.trim()))
        .collect::<Vec<_>>()
        .join(", ");
    let rows = client
        .query(
            &format!(
                "SELECT {cols} FROM plans p JOIN accounts a ON a.id = p.account_id
                 WHERE a.status = 'active' AND p.status IN ('pending', 'executing') ORDER BY p.generated_at"
            ),
            &[],
        )
        .await?;
    Ok(rows.iter().map(plan_from_row).collect())
}

/// Plans in `pending` or `executing` state for an account.
pub async fn open_plans(client: &impl GenericClient, account_id: i64) -> Result<Vec<Plan>> {
    let rows = client
        .query(
            &format!("SELECT {PLAN_COLUMNS} FROM plans WHERE account_id = $1 AND status IN ('pending', 'executing') ORDER BY generated_at"),
            &[&account_id],
        )
        .await?;
    Ok(rows.iter().map(plan_from_row).collect())
}

pub struct PlanFilter {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub status: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn list_plans(
    client: &impl GenericClient,
    account_id: i64,
    filter: &PlanFilter,
) -> Result<(Vec<Plan>, i64)> {
    let where_clause = "account_id = $1
        AND ($2::date IS NULL OR trade_date >= $2)
        AND ($3::date IS NULL OR trade_date <= $3)
        AND ($4::text IS NULL OR status = $4)";
    let rows = client
        .query(
            &format!(
                "SELECT {PLAN_COLUMNS} FROM plans WHERE {where_clause}
                 ORDER BY trade_date DESC, generated_at DESC LIMIT $5 OFFSET $6"
            ),
            &[
                &account_id,
                &filter.from,
                &filter.to,
                &filter.status,
                &filter.limit,
                &filter.offset,
            ],
        )
        .await?;
    let total: i64 = client
        .query_one(
            &format!("SELECT count(*) FROM plans WHERE {where_clause}"),
            &[&account_id, &filter.from, &filter.to, &filter.status],
        )
        .await?
        .get(0);
    Ok((rows.iter().map(plan_from_row).collect(), total))
}

pub async fn set_plan_status(
    client: &impl GenericClient,
    id: i64,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    client
        .execute(
            "UPDATE plans SET status = $2, error = COALESCE($3, error),
               executed_at = CASE WHEN $2 IN ('executed', 'partially_executed') THEN app_now() ELSE executed_at END
             WHERE id = $1",
            &[&id, &status, &error],
        )
        .await?;
    Ok(())
}

pub async fn approve_plan(client: &impl GenericClient, id: i64, actor: &str) -> Result<()> {
    client
        .execute(
            "UPDATE plans SET approved_at = app_now(), approved_by = $2 WHERE id = $1",
            &[&id, &actor],
        )
        .await?;
    Ok(())
}

pub async fn cancel_plan(
    client: &impl GenericClient,
    id: i64,
    actor: &str,
    reason: &str,
) -> Result<()> {
    client
        .execute(
            "UPDATE plans SET status = 'cancelled', cancelled_at = app_now(), cancelled_by = $2, cancel_reason = $3 WHERE id = $1",
            &[&id, &actor, &reason],
        )
        .await?;
    client
        .execute(
            "UPDATE orders SET status = 'cancelled', status_reason = 'plan_cancelled', updated_at = app_now()
             WHERE plan_id = $1 AND status = 'planned'",
            &[&id],
        )
        .await?;
    Ok(())
}

pub async fn update_plan_summary(
    client: &impl GenericClient,
    id: i64,
    summary: &Value,
) -> Result<()> {
    client
        .execute(
            "UPDATE plans SET summary = summary || $2 WHERE id = $1",
            &[&id, summary],
        )
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Orders and fills
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Order {
    pub id: i64,
    pub plan_id: i64,
    pub symbol: String,
    pub side: String,
    pub reason: String,
    pub qty: Decimal,
    pub ref_price: f64,
    pub weight_before: f64,
    pub weight_target: f64,
    pub weight_after: f64,
    pub status: String,
    pub status_reason: Option<String>,
    pub filled_qty: Decimal,
    pub sequence: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const ORDER_COLUMNS: &str =
    "id, plan_id, symbol, side, reason, qty, ref_price, weight_before, weight_target,
    weight_after, status, status_reason, filled_qty, sequence, created_at, updated_at";

fn order_from_row(r: &Row) -> Order {
    Order {
        id: r.get("id"),
        plan_id: r.get("plan_id"),
        symbol: r.get("symbol"),
        side: r.get("side"),
        reason: r.get("reason"),
        qty: r.get("qty"),
        ref_price: r.get("ref_price"),
        weight_before: r.get("weight_before"),
        weight_target: r.get("weight_target"),
        weight_after: r.get("weight_after"),
        status: r.get("status"),
        status_reason: r.get("status_reason"),
        filled_qty: r.get("filled_qty"),
        sequence: r.get("sequence"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_order(
    client: &impl GenericClient,
    plan_id: i64,
    account_id: i64,
    symbol: &str,
    side: &str,
    reason: &str,
    qty: Decimal,
    ref_price: f64,
    weights: (f64, f64, f64),
    sequence: i32,
) -> Result<i64> {
    let row = client
        .query_one(
            "INSERT INTO orders (plan_id, account_id, symbol, side, reason, qty, ref_price, weight_before, weight_target, weight_after, status, sequence)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'planned', $11) RETURNING id",
            &[&plan_id, &account_id, &symbol, &side, &reason, &qty, &ref_price, &weights.0, &weights.1, &weights.2, &sequence],
        )
        .await?;
    Ok(row.get(0))
}

pub async fn orders_for_plan(client: &impl GenericClient, plan_id: i64) -> Result<Vec<Order>> {
    let rows = client
        .query(
            &format!("SELECT {ORDER_COLUMNS} FROM orders WHERE plan_id = $1 ORDER BY sequence, id"),
            &[&plan_id],
        )
        .await?;
    Ok(rows.iter().map(order_from_row).collect())
}

pub async fn order(client: &impl GenericClient, id: i64) -> Result<Option<Order>> {
    let row = client
        .query_opt(
            &format!("SELECT {ORDER_COLUMNS} FROM orders WHERE id = $1"),
            &[&id],
        )
        .await?;
    Ok(row.as_ref().map(order_from_row))
}

pub async fn set_order_status(
    client: &impl GenericClient,
    id: i64,
    status: &str,
    reason: Option<&str>,
    filled_qty: Option<Decimal>,
) -> Result<()> {
    client
        .execute(
            "UPDATE orders SET status = $2, status_reason = $3, filled_qty = COALESCE($4, filled_qty), updated_at = app_now() WHERE id = $1",
            &[&id, &status, &reason, &filled_qty],
        )
        .await?;
    Ok(())
}

pub struct OrderFilter {
    pub symbol: Option<String>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub status: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderWithPlan {
    #[serde(flatten)]
    pub order: Order,
    pub trade_date: NaiveDate,
    pub slot: String,
}

pub async fn list_orders(
    client: &impl GenericClient,
    account_id: i64,
    filter: &OrderFilter,
) -> Result<(Vec<OrderWithPlan>, i64)> {
    let where_clause = "o.account_id = $1
        AND ($2::text IS NULL OR o.symbol = $2)
        AND ($3::date IS NULL OR p.trade_date >= $3)
        AND ($4::date IS NULL OR p.trade_date <= $4)
        AND ($5::text IS NULL OR o.status = $5)";
    let cols = ORDER_COLUMNS
        .split(',')
        .map(|c| format!("o.{}", c.trim()))
        .collect::<Vec<_>>()
        .join(", ");
    let rows = client
        .query(
            &format!(
                "SELECT {cols}, p.trade_date, p.slot FROM orders o JOIN plans p ON p.id = o.plan_id
                 WHERE {where_clause} ORDER BY o.created_at DESC, o.sequence LIMIT $6 OFFSET $7"
            ),
            &[
                &account_id,
                &filter.symbol,
                &filter.from,
                &filter.to,
                &filter.status,
                &filter.limit,
                &filter.offset,
            ],
        )
        .await?;
    let total: i64 = client
        .query_one(
            &format!("SELECT count(*) FROM orders o JOIN plans p ON p.id = o.plan_id WHERE {where_clause}"),
            &[&account_id, &filter.symbol, &filter.from, &filter.to, &filter.status],
        )
        .await?
        .get(0);
    Ok((
        rows.iter()
            .map(|r| OrderWithPlan {
                order: order_from_row(r),
                trade_date: r.get("trade_date"),
                slot: r.get("slot"),
            })
            .collect(),
        total,
    ))
}

#[derive(Debug, Clone, Serialize)]
pub struct Fill {
    pub id: i64,
    pub order_id: i64,
    pub plan_id: i64,
    pub symbol: String,
    pub side: String,
    pub qty: Decimal,
    pub price: Decimal,
    pub quote_price: f64,
    pub quote_ts: Option<DateTime<Utc>>,
    pub notional: Decimal,
    pub commission: Decimal,
    pub fees: Decimal,
    pub slippage: Decimal,
    pub realized_pnl: Option<Decimal>,
    pub executed_at: DateTime<Utc>,
}

pub struct NewFill {
    pub order_id: i64,
    pub account_id: i64,
    pub symbol: String,
    pub side: String,
    pub qty: Decimal,
    pub price: Decimal,
    pub quote_price: f64,
    pub quote_ts: Option<DateTime<Utc>>,
    pub notional: Decimal,
    pub commission: Decimal,
    pub fees: Decimal,
    pub slippage: Decimal,
    pub realized_pnl: Option<Decimal>,
}

pub async fn insert_fill(client: &impl GenericClient, fill: &NewFill) -> Result<i64> {
    let row = client
        .query_one(
            "INSERT INTO fills (order_id, account_id, symbol, side, qty, price, quote_price, quote_ts, notional, commission, fees, slippage, realized_pnl)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) RETURNING id",
            &[
                &fill.order_id,
                &fill.account_id,
                &fill.symbol,
                &fill.side,
                &fill.qty,
                &fill.price,
                &fill.quote_price,
                &fill.quote_ts,
                &fill.notional,
                &fill.commission,
                &fill.fees,
                &fill.slippage,
                &fill.realized_pnl,
            ],
        )
        .await?;
    Ok(row.get(0))
}

pub struct FillFilter {
    pub symbol: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub plan_id: Option<i64>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn list_fills(
    client: &impl GenericClient,
    account_id: i64,
    filter: &FillFilter,
) -> Result<(Vec<Fill>, i64)> {
    let where_clause = "f.account_id = $1
        AND ($2::text IS NULL OR f.symbol = $2)
        AND ($3::timestamptz IS NULL OR f.executed_at >= $3)
        AND ($4::timestamptz IS NULL OR f.executed_at <= $4)
        AND ($5::bigint IS NULL OR o.plan_id = $5)";
    let rows = client
        .query(
            &format!(
                "SELECT f.id, f.order_id, o.plan_id, f.symbol, f.side, f.qty, f.price, f.quote_price, f.quote_ts,
                        f.notional, f.commission, f.fees, f.slippage, f.realized_pnl, f.executed_at
                 FROM fills f JOIN orders o ON o.id = f.order_id
                 WHERE {where_clause} ORDER BY f.executed_at DESC, f.id DESC LIMIT $6 OFFSET $7"
            ),
            &[&account_id, &filter.symbol, &filter.from, &filter.to, &filter.plan_id, &filter.limit, &filter.offset],
        )
        .await?;
    let total: i64 = client
        .query_one(
            &format!("SELECT count(*) FROM fills f JOIN orders o ON o.id = f.order_id WHERE {where_clause}"),
            &[&account_id, &filter.symbol, &filter.from, &filter.to, &filter.plan_id],
        )
        .await?
        .get(0);
    Ok((
        rows.iter()
            .map(|r| Fill {
                id: r.get(0),
                order_id: r.get(1),
                plan_id: r.get(2),
                symbol: r.get(3),
                side: r.get(4),
                qty: r.get(5),
                price: r.get(6),
                quote_price: r.get(7),
                quote_ts: r.get(8),
                notional: r.get(9),
                commission: r.get(10),
                fees: r.get(11),
                slippage: r.get(12),
                realized_pnl: r.get(13),
                executed_at: r.get(14),
            })
            .collect(),
        total,
    ))
}

// ---------------------------------------------------------------------------------------------
// Valuation history
// ---------------------------------------------------------------------------------------------

/// Net trading per symbol since an instant: shares and cash in both directions (cash includes
/// commissions and fees).
#[derive(Debug, Clone, Copy, Default)]
pub struct FillFlows {
    pub bought_qty: f64,
    pub bought_cash: f64,
    pub sold_qty: f64,
    pub sold_cash: f64,
}

pub async fn fill_flows_since(
    client: &impl GenericClient,
    account_id: i64,
    since: DateTime<Utc>,
) -> Result<HashMap<String, FillFlows>> {
    let rows = client
        .query(
            "SELECT symbol,
                    COALESCE(SUM(qty) FILTER (WHERE side = 'buy'), 0)::float8,
                    COALESCE(SUM(notional + commission + fees) FILTER (WHERE side = 'buy'), 0)::float8,
                    COALESCE(SUM(qty) FILTER (WHERE side = 'sell'), 0)::float8,
                    COALESCE(SUM(notional - commission - fees) FILTER (WHERE side = 'sell'), 0)::float8
             FROM fills WHERE account_id = $1 AND executed_at >= $2 GROUP BY symbol",
            &[&account_id, &since],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get::<_, String>(0),
                FillFlows {
                    bought_qty: r.get(1),
                    bought_cash: r.get(2),
                    sold_qty: r.get(3),
                    sold_cash: r.get(4),
                },
            )
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct NavPoint {
    pub date: NaiveDate,
    pub nav: Decimal,
    pub cash: Decimal,
    pub invested: Decimal,
    pub flows: Decimal,
}

pub async fn upsert_nav(
    client: &impl GenericClient,
    account_id: i64,
    date: NaiveDate,
    nav: Decimal,
    cash: Decimal,
    invested: Decimal,
) -> Result<()> {
    client
        .execute(
            "INSERT INTO nav_snapshots (account_id, date, nav, cash, invested) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (account_id, date) DO UPDATE SET nav = EXCLUDED.nav, cash = EXCLUDED.cash,
               invested = EXCLUDED.invested, created_at = app_now()",
            &[&account_id, &date, &nav, &cash, &invested],
        )
        .await?;
    Ok(())
}

pub async fn nav_history(
    client: &impl GenericClient,
    account_id: i64,
    from: Option<NaiveDate>,
) -> Result<Vec<NavPoint>> {
    let rows = client
        .query(
            "SELECT date, nav, cash, invested, flows FROM nav_snapshots
             WHERE account_id = $1 AND ($2::date IS NULL OR date >= $2) ORDER BY date",
            &[&account_id, &from],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| NavPoint {
            date: r.get(0),
            nav: r.get(1),
            cash: r.get(2),
            invested: r.get(3),
            flows: r.get(4),
        })
        .collect())
}

pub async fn replace_position_snapshots(
    client: &impl GenericClient,
    account_id: i64,
    date: NaiveDate,
    rows: &[(String, Decimal, f64, Decimal, f64)],
) -> Result<()> {
    client
        .execute(
            "DELETE FROM position_snapshots WHERE account_id = $1 AND date = $2",
            &[&account_id, &date],
        )
        .await?;
    for (symbol, qty, price, value, weight) in rows {
        client
            .execute(
                "INSERT INTO position_snapshots (account_id, date, symbol, qty, price, value, weight) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[&account_id, &date, symbol, qty, price, value, weight],
            )
            .await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct PositionSnapshot {
    pub date: NaiveDate,
    pub symbol: String,
    pub qty: Decimal,
    pub price: f64,
    pub value: Decimal,
    pub weight: f64,
}

pub async fn position_snapshots(
    client: &impl GenericClient,
    account_id: i64,
    from: Option<NaiveDate>,
) -> Result<Vec<PositionSnapshot>> {
    let rows = client
        .query(
            "SELECT date, symbol, qty, price, value, weight FROM position_snapshots
             WHERE account_id = $1 AND ($2::date IS NULL OR date >= $2) ORDER BY date, symbol",
            &[&account_id, &from],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| PositionSnapshot {
            date: r.get(0),
            symbol: r.get(1),
            qty: r.get(2),
            price: r.get(3),
            value: r.get(4),
            weight: r.get(5),
        })
        .collect())
}

/// Target weights from the most recent plan that ran the engine.
pub async fn latest_targets(
    client: &impl GenericClient,
    account_id: i64,
) -> Result<Option<(i64, DateTime<Utc>, std::collections::HashMap<String, f64>)>> {
    let row = client
        .query_opt(
            "SELECT id, generated_at, diagnostics->'targets'->'assets' FROM plans
             WHERE account_id = $1 AND diagnostics ? 'targets'
             ORDER BY generated_at DESC LIMIT 1",
            &[&account_id],
        )
        .await?;
    Ok(row.map(|r| {
        let assets: Value = r.get(2);
        let weights = assets
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|a| {
                        Some((
                            a.get("symbol")?.as_str()?.to_string(),
                            a.get("weight")?.as_f64()?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        (r.get(0), r.get(1), weights)
    }))
}
