//! Trading at an external broker (Longbridge): linking a portfolio to a broker account,
//! executing its plans by submitting orders, following those orders to their end, and keeping
//! the account's books in step with the broker.
//!
//! A broker-linked account uses the same tables as a simulated one, so every page, report and
//! notification works unchanged; what differs is where fills come from:
//!
//! - **Execution.** Sells are submitted first; buys follow once the sells have ended (or after
//!   [`BUY_WAIT`]), sized to the US-dollar cash the broker reports as available. Each order is a
//!   day limit order for whole shares, priced [`LIMIT_COLLAR`] through the current quote but
//!   never beyond the plan's price tolerance (`max_price_deviation`), so it trades at the market
//!   while a sudden move cannot fill it far from the plan. The same quote checks as the
//!   simulator apply before anything is sent.
//! - **Following.** Every scheduler tick, the broker's orders and executions are read back;
//!   each execution becomes one fill (recorded once, by the broker's trade id) and moves the
//!   position and cash exactly like a simulated fill. Orders still working at the plan's
//!   deadline are cancelled. When every order has ended the plan is finalised.
//! - **Synchronisation.** The broker is the source of truth for cash and holdings. Before a plan
//!   is generated or executed, every few minutes during the session, after the close and on
//!   request, the account is compared with the broker and any difference (fees the broker
//!   charged, dividends, transfers, trades made elsewhere) is booked as an adjustment and
//!   written to the audit log. While orders are working the comparison waits, so a fill that
//!   has not been read back yet is never mistaken for a difference.
//!
//! The account manages the broker account's US-dollar cash and its US listings of companies in
//! the universe. Anything else in the broker account (other markets, other names) is reported
//! but never traded.
//!
//! Everything here runs under the trading lock, like the simulator.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Duration, Utc};
use deadpool_postgres::GenericClient;
use quant_core::costs::Side;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use serde::Serialize;
use serde_json::{Value, json};

use crate::brokers::{
    AccountMode, Broker, BrokerError, BrokerOrder, Execution, OrderRequest, OrderState, Snapshot,
};
use crate::notify::{self, Event};
use crate::services::broker::ExecutionReport;
use crate::services::{marketdata, portfolio as portfolio_service};
use crate::state::{AppState, ServerEvent};
use crate::store::portfolios::{self, Book, NewPortfolio};
use crate::store::settings::{self, ExecutionSettings};
use crate::store::strategy as strategy_store;
use crate::store::trading::{self, Account, NewFill, Order, Position, dec, f};
use crate::store::{market, system};

/// The currency a broker-linked account is kept in.
pub const BASE_CURRENCY: &str = "USD";
/// How far through the current quote a limit order is priced.
pub const LIMIT_COLLAR: f64 = 0.01;
/// Buys wait for the plan's sells to end, but no longer than this.
pub const BUY_WAIT: Duration = Duration::minutes(10);
/// A submission whose outcome is unknown and whose order cannot be found at the broker after
/// this long is given up (it is never sent again).
pub const SUBMIT_LOST_AFTER: Duration = Duration::minutes(3);

/// The remark hone-quant puts on an order (shown in Longbridge's apps).
pub fn remark(order_id: i64) -> String {
    format!("hq-{order_id}")
}

/// The client for an account's broker and mode.
pub fn client_for(state: &AppState, account: &Account) -> Result<Arc<dyn Broker>> {
    let mode = AccountMode::parse(&account.mode).ok_or_else(|| {
        anyhow!(
            "account {} has an unknown mode {}",
            account.id,
            account.mode
        )
    })?;
    state.brokers.get(&account.broker, mode).ok_or_else(|| {
        anyhow!(
            "this server has no {} {} client configured",
            account.broker,
            mode.as_str()
        )
    })
}

fn to_f(value: Decimal) -> f64 {
    value.to_f64().unwrap_or(0.0)
}

fn to_dec(value: f64) -> Decimal {
    Decimal::from_f64(value).unwrap_or_default()
}

/// US price increments: cents from $1, hundredths of a cent below.
fn tick(price: f64) -> f64 {
    if price >= 1.0 { 0.01 } else { 0.0001 }
}

fn round_up_tick(price: f64) -> f64 {
    let t = tick(price);
    ((price / t) - 1e-9).ceil() * t
}

fn round_down_tick(price: f64) -> f64 {
    let t = tick(price);
    ((price / t) + 1e-9).floor() * t
}

/// The limit price of an order: `LIMIT_COLLAR` through the quote, but never beyond the plan's
/// price tolerance around its reference price.
pub fn limit_price(side: Side, quote: f64, ref_price: f64, max_deviation: f64) -> Decimal {
    let price = match side {
        Side::Buy => {
            let collar = round_up_tick(quote * (1.0 + LIMIT_COLLAR));
            if ref_price > 0.0 {
                collar.min(round_down_tick(ref_price * (1.0 + max_deviation)))
            } else {
                collar
            }
        }
        Side::Sell => {
            let collar = round_down_tick(quote * (1.0 - LIMIT_COLLAR));
            if ref_price > 0.0 {
                collar.max(round_up_tick(ref_price * (1.0 - max_deviation)))
            } else {
                collar
            }
        }
    };
    let dp = if price >= 1.0 { 2 } else { 4 };
    to_dec(price).round_dp(dp)
}

// ---------------------------------------------------------------------------------------------
// Synchronisation
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CashView {
    pub currency: String,
    pub total: Decimal,
    pub available: Decimal,
    pub frozen: Decimal,
    pub settling: Decimal,
}

#[derive(Debug, Clone, Serialize)]
pub struct Adjustment {
    /// `cash` or a symbol.
    pub item: String,
    pub before: Decimal,
    pub after: Decimal,
}

#[derive(Debug, Clone, Serialize)]
pub struct Unmanaged {
    pub broker_symbol: String,
    pub name: String,
    pub currency: String,
    pub qty: Decimal,
}

/// What a synchronisation found and did. Stored on the account as its `sync_status`.
#[derive(Debug, Clone, Serialize)]
pub struct SyncReport {
    pub ok: bool,
    pub at: DateTime<Utc>,
    pub broker: String,
    pub mode: String,
    pub channel: Option<String>,
    pub cash: Option<CashView>,
    /// Differences booked to bring the account in line with the broker.
    pub adjustments: Vec<Adjustment>,
    /// Holdings in the broker account that hone-quant does not manage.
    pub unmanaged: Vec<Unmanaged>,
    /// Orders were working, so the comparison waits until they end.
    pub deferred: bool,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

impl SyncReport {
    fn failed(state: &AppState, account: &Account, error: String) -> Self {
        Self {
            ok: false,
            at: state.now(),
            broker: account.broker.clone(),
            mode: account.mode.clone(),
            channel: None,
            cash: None,
            adjustments: Vec::new(),
            unmanaged: Vec::new(),
            deferred: false,
            warnings: Vec::new(),
            error: Some(error),
        }
    }
}

/// The broker's cash in the base currency, and a warning when it cannot be used as is.
fn base_cash(snapshot: &Snapshot, warnings: &mut Vec<String>) -> CashView {
    match snapshot.cash_in(BASE_CURRENCY) {
        Some(c) => CashView {
            currency: c.currency.clone(),
            total: c.total,
            available: c.available,
            frozen: c.frozen,
            settling: c.settling,
        },
        None => {
            warnings.push(format!(
                "the broker reports no {BASE_CURRENCY} balance; treated as zero"
            ));
            CashView {
                currency: BASE_CURRENCY.into(),
                total: Decimal::ZERO,
                available: Decimal::ZERO,
                frozen: Decimal::ZERO,
                settling: Decimal::ZERO,
            }
        }
    }
}

/// Splits the broker's holdings into the ones the account manages (US listings of universe
/// companies, by symbol) and the rest.
fn classify(
    snapshot: &Snapshot,
    universe: &HashSet<String>,
) -> (BTreeMap<String, crate::brokers::Holding>, Vec<Unmanaged>) {
    let mut managed = BTreeMap::new();
    let mut unmanaged = Vec::new();
    for holding in &snapshot.holdings {
        match &holding.symbol {
            Some(symbol) if universe.contains(symbol) => {
                managed.insert(symbol.clone(), holding.clone());
            }
            _ => {
                if holding.qty != Decimal::ZERO {
                    unmanaged.push(Unmanaged {
                        broker_symbol: holding.broker_symbol.clone(),
                        name: holding.name.clone(),
                        currency: holding.currency.clone(),
                        qty: holding.qty,
                    });
                }
            }
        }
    }
    (managed, unmanaged)
}

async fn universe_symbols(client: &impl GenericClient) -> Result<HashSet<String>> {
    Ok(market::assets(client, true)
        .await?
        .into_iter()
        .map(|a| a.symbol)
        .collect())
}

/// Checks that the snapshot comes from the broker account this account is linked to.
fn check_identity(account: &Account, snapshot: &Snapshot) -> Result<(), BrokerError> {
    let expected = account.broker_account.as_deref().unwrap_or_default();
    if snapshot.identity.account != expected {
        return Err(BrokerError::WrongAccount(format!(
            "the broker client is signed in to {} but this portfolio is linked to {}",
            snapshot.identity.account, expected
        )));
    }
    if snapshot.identity.mode.as_str() != account.mode {
        return Err(BrokerError::WrongAccount(format!(
            "the broker account {} is a {} account but this portfolio trades {}",
            snapshot.identity.channel,
            snapshot.identity.mode.as_str(),
            account.mode
        )));
    }
    Ok(())
}

/// Synchronises one account with its broker (takes the trading lock).
pub async fn sync_account(
    state: &Arc<AppState>,
    account_id: i64,
    actor: &str,
) -> Result<SyncReport> {
    let _guard = state.trading_lock.lock().await;
    sync_locked(state, account_id, actor).await
}

/// An account that could not be synchronised.
#[derive(Debug, Clone)]
pub struct SyncFailure {
    pub account_id: i64,
    pub portfolio_id: i64,
    pub broker: String,
    pub error: String,
}

/// Synchronises every active broker-linked account. Failures are recorded on each account and
/// returned.
pub async fn sync_all(state: &Arc<AppState>, actor: &str) -> Result<Vec<SyncFailure>> {
    let client = state.pool.get().await?;
    let accounts = trading::active_external_accounts(&client).await?;
    drop(client);
    let mut failures = Vec::new();
    for account in accounts {
        if let Err(error) = sync_account(state, account.id, actor).await {
            let error = format!("{error:#}");
            tracing::warn!(account = account.id, error = %error, "broker synchronisation failed");
            failures.push(SyncFailure {
                account_id: account.id,
                portfolio_id: account.portfolio_id,
                broker: account.broker.clone(),
                error,
            });
        }
    }
    Ok(failures)
}

/// [`sync_account`] for callers that already hold the trading lock. A failure is recorded on the
/// account (and returned).
pub async fn sync_locked(
    state: &Arc<AppState>,
    account_id: i64,
    actor: &str,
) -> Result<SyncReport> {
    let client = state.pool.get().await?;
    let account = trading::account(&client, account_id)
        .await?
        .ok_or_else(|| anyhow!("account {account_id} not found"))?;
    drop(client);
    if !account.is_external() {
        bail!(
            "account {account_id} is simulated by hone-quant; there is no broker to synchronise with"
        );
    }
    match sync_inner(state, &account, actor).await {
        Ok(report) => Ok(report),
        Err(error) => {
            let message = format!("{error:#}");
            let report = SyncReport::failed(state, &account, message.clone());
            let client = state.pool.get().await?;
            trading::set_sync_status(&client, account.id, false, &serde_json::to_value(&report)?)
                .await?;
            drop(client);
            state.emit(ServerEvent::Account {
                reason: "broker_sync".into(),
                portfolio_id: account.portfolio_id,
            });
            Err(error)
        }
    }
}

async fn sync_inner(state: &AppState, account: &Account, actor: &str) -> Result<SyncReport> {
    let broker = client_for(state, account)?;
    let snapshot = broker.snapshot().await?;
    check_identity(account, &snapshot)?;
    let mut warnings = Vec::new();
    let cash = base_cash(&snapshot, &mut warnings);

    let mut client = state.pool.get().await?;
    let universe = universe_symbols(&client).await?;
    let (managed, unmanaged) = classify(&snapshot, &universe);
    let working = trading::working_orders(&client, account.id).await?;
    let mut report = SyncReport {
        ok: true,
        at: state.now(),
        broker: account.broker.clone(),
        mode: account.mode.clone(),
        channel: Some(snapshot.identity.channel.clone()),
        cash: Some(cash.clone()),
        adjustments: Vec::new(),
        unmanaged,
        deferred: !working.is_empty(),
        warnings,
        error: None,
    };
    if report.deferred {
        trading::set_sync_status(&client, account.id, true, &serde_json::to_value(&report)?)
            .await?;
        return Ok(report);
    }

    let tx = client.transaction().await?;
    let locked = trading::lock_account(&tx, account.id).await?;
    let now = state.now();
    let ours = trading::positions(&tx, account.id, false).await?;
    let mut symbols: Vec<String> = managed.keys().cloned().collect();
    symbols.extend(ours.iter().map(|p| p.symbol.clone()));
    symbols.sort();
    symbols.dedup();
    for symbol in symbols {
        let broker_qty = managed.get(&symbol).map(|h| h.qty).unwrap_or_default();
        let mut position = trading::position_for_update(&tx, account.id, &symbol)
            .await?
            .unwrap_or(Position {
                symbol: symbol.clone(),
                qty: Decimal::ZERO,
                avg_cost: Decimal::ZERO,
                realized_pnl: Decimal::ZERO,
                dividends: Decimal::ZERO,
                opened_at: None,
                updated_at: now,
            });
        if position.qty == broker_qty {
            continue;
        }
        report.adjustments.push(Adjustment {
            item: symbol.clone(),
            before: position.qty,
            after: broker_qty,
        });
        if broker_qty <= Decimal::ZERO {
            position.qty = Decimal::ZERO;
            position.opened_at = None;
        } else {
            // The broker's average cost is authoritative once the quantity differs.
            let cost = managed
                .get(&symbol)
                .map(|h| h.cost_price)
                .filter(|c| *c > Decimal::ZERO)
                .unwrap_or(position.avg_cost);
            position.qty = broker_qty;
            position.avg_cost = cost.round_dp(6);
            if position.opened_at.is_none() {
                position.opened_at = Some(now);
            }
        }
        trading::save_position(&tx, account.id, &position).await?;
    }
    let mut target_cash = cash.total.round_dp(2);
    if target_cash < Decimal::ZERO {
        report.warnings.push(format!(
            "the broker reports a negative {BASE_CURRENCY} balance ({target_cash}); hone-quant keeps zero"
        ));
        target_cash = Decimal::ZERO;
    }
    if locked.cash != target_cash {
        report.adjustments.push(Adjustment {
            item: "cash".into(),
            before: locked.cash,
            after: target_cash,
        });
        trading::post_cash(
            &tx,
            account.id,
            "adjustment",
            target_cash - locked.cash,
            None,
            Some("broker_sync"),
            None,
            &format!("synchronised with {}", account.broker),
        )
        .await?;
    }
    if !report.adjustments.is_empty() {
        system::audit(
            &tx,
            actor,
            "account.synchronised",
            "account",
            &account.id.to_string(),
            json!({"broker": account.broker, "adjustments": report.adjustments, "portfolio_id": account.portfolio_id}),
            "",
        )
        .await?;
    }
    trading::set_sync_status(&tx, account.id, true, &serde_json::to_value(&report)?).await?;
    tx.commit().await?;
    drop(client);
    if !report.adjustments.is_empty() {
        tracing::info!(
            account = account.id,
            adjustments = report.adjustments.len(),
            "account synchronised with the broker"
        );
    }
    // Pages refresh when the books changed or the connection recovered.
    if !report.adjustments.is_empty() || account.sync_status["ok"] != Value::Bool(true) {
        state.emit(ServerEvent::Account {
            reason: "broker_sync".into(),
            portfolio_id: account.portfolio_id,
        });
    }
    Ok(report)
}

// ---------------------------------------------------------------------------------------------
// Linking a portfolio
// ---------------------------------------------------------------------------------------------

/// A broker account read and checked, ready to become a portfolio's account.
pub struct PreparedLink {
    pub snapshot: Snapshot,
    pub cash: Decimal,
    /// Adopted holdings: (symbol, qty, cost price, valuation price).
    pub holdings: Vec<(String, Decimal, Decimal, f64)>,
    pub nav: Decimal,
    pub report: SyncReport,
}

/// Reads the broker account a new portfolio would be linked to and values what it would
/// manage. Nothing is written.
pub async fn prepare_link(
    state: &AppState,
    broker_name: &str,
    mode: AccountMode,
) -> Result<PreparedLink, LinkError> {
    if mode == AccountMode::Live {
        return Err(LinkError::LiveDisabled);
    }
    let broker = state
        .brokers
        .get(broker_name, mode)
        .ok_or_else(|| LinkError::NotConfigured(broker_name.to_string()))?;
    let snapshot = broker.snapshot().await.map_err(LinkError::Broker)?;
    if snapshot.identity.mode != mode {
        return Err(LinkError::Broker(BrokerError::WrongAccount(format!(
            "the broker account {} is not a {} account",
            snapshot.identity.channel,
            mode.as_str()
        ))));
    }
    let mut warnings = Vec::new();
    let cash_view = base_cash(&snapshot, &mut warnings);
    let mut cash = cash_view.total.round_dp(2);
    if cash < Decimal::ZERO {
        warnings.push(format!(
            "the broker reports a negative {BASE_CURRENCY} balance ({cash}); hone-quant keeps zero"
        ));
        cash = Decimal::ZERO;
    }
    let client = state.pool.get().await.map_err(anyhow::Error::from)?;
    let universe = universe_symbols(&client).await?;
    let (managed, unmanaged) = classify(&snapshot, &universe);
    let symbols: Vec<String> = managed.keys().cloned().collect();
    let quotes = market::quotes(&client).await?;
    let closes = market::closes_before(
        &client,
        &symbols,
        state.now().date_naive() + Duration::days(1),
    )
    .await?;
    drop(client);
    let mut holdings = Vec::new();
    let mut invested = Decimal::ZERO;
    for (symbol, holding) in &managed {
        if holding.qty <= Decimal::ZERO {
            continue;
        }
        let price = quotes
            .get(symbol)
            .map(|q| q.quote.price)
            .filter(|p| *p > 0.0)
            .or_else(|| closes.get(symbol).map(|(_, c)| *c))
            .unwrap_or_else(|| to_f(holding.cost_price));
        invested += (holding.qty * dec(price, 6)).round_dp(2);
        holdings.push((symbol.clone(), holding.qty, holding.cost_price, price));
    }
    let nav = cash + invested;
    if nav <= Decimal::ZERO {
        return Err(LinkError::Empty);
    }
    let report = SyncReport {
        ok: true,
        at: state.now(),
        broker: broker_name.to_string(),
        mode: mode.as_str().into(),
        channel: Some(snapshot.identity.channel.clone()),
        cash: Some(cash_view),
        adjustments: Vec::new(),
        unmanaged,
        deferred: false,
        warnings,
        error: None,
    };
    Ok(PreparedLink {
        snapshot,
        cash,
        holdings,
        nav,
        report,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("live trading is not enabled in this version of hone-quant")]
    LiveDisabled,
    #[error("no {0} client is configured on this server")]
    NotConfigured(String),
    #[error(transparent)]
    Broker(BrokerError),
    #[error(
        "the broker account has no US-dollar cash and no US holdings of universe companies to manage"
    )]
    Empty,
    #[error("this broker account is already linked to portfolio {0}")]
    AlreadyLinked(i64),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub struct LinkRequest<'a> {
    pub portfolio: NewPortfolio<'a>,
    pub broker: &'a str,
    pub mode: AccountMode,
    pub strategy_version_id: i64,
}

/// Creates a portfolio whose account mirrors the prepared broker account, inside the caller's
/// transaction. Holdings of universe companies are adopted at the broker's average cost, and
/// the initial capital is the cash plus their value. `None` when the owner already has an
/// active portfolio with that name.
pub async fn open_linked(
    state: &AppState,
    tx: &impl GenericClient,
    request: &LinkRequest<'_>,
    prepared: &PreparedLink,
) -> Result<Option<Book>, LinkError> {
    let identity = &prepared.snapshot.identity;
    if let Some(existing) =
        trading::active_broker_account(tx, request.broker, &identity.account).await?
    {
        return Err(LinkError::AlreadyLinked(existing.portfolio_id));
    }
    let Some(portfolio) = portfolios::insert(tx, &request.portfolio).await? else {
        return Ok(None);
    };
    let (first_session, base_date) = portfolio_service::inception_dates(state, state.now());
    let report = serde_json::to_value(&prepared.report).map_err(anyhow::Error::from)?;
    let account = trading::create_broker_account(
        tx,
        &trading::NewBrokerAccount {
            portfolio_id: portfolio.id,
            name: "Longbridge",
            broker: request.broker,
            mode: request.mode.as_str(),
            broker_account: &identity.account,
            initial_cash: prepared.nav,
            cash: prepared.cash,
            inception: first_session,
            sync_status: &report,
        },
    )
    .await?;
    let now = state.now();
    let mut invested = Decimal::ZERO;
    for (symbol, qty, cost, price) in &prepared.holdings {
        trading::save_position(
            tx,
            account.id,
            &Position {
                symbol: symbol.clone(),
                qty: *qty,
                avg_cost: cost.round_dp(6),
                realized_pnl: Decimal::ZERO,
                dividends: Decimal::ZERO,
                opened_at: Some(now),
                updated_at: now,
            },
        )
        .await?;
        invested += (*qty * dec(*price, 6)).round_dp(2);
    }
    trading::upsert_nav(
        tx,
        account.id,
        base_date,
        prepared.nav,
        prepared.cash,
        invested,
    )
    .await?;
    strategy_store::activate(
        tx,
        account.id,
        request.strategy_version_id,
        request.portfolio.created_by,
        "initial activation",
    )
    .await?;
    system::audit(
        tx,
        request.portfolio.created_by,
        "portfolio.created",
        "portfolio",
        &portfolio.id.to_string(),
        json!({
            "name": portfolio.name,
            "owner": portfolio.owner,
            "account_id": account.id,
            "broker": request.broker,
            "mode": request.mode.as_str(),
            "broker_account": identity.account,
            "initial_cash": prepared.nav,
            "cash": prepared.cash,
            "adopted": prepared.holdings.iter().map(|(s, q, c, _)| json!({"symbol": s, "qty": q, "cost": c})).collect::<Vec<_>>(),
            "unmanaged": prepared.report.unmanaged,
            "strategy_version_id": request.strategy_version_id,
            "automation": portfolio.automation,
        }),
        "",
    )
    .await?;
    Ok(Some(Book { portfolio, account }))
}

// ---------------------------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------------------------

/// What one round of submissions did.
#[derive(Debug, Default)]
struct Submitted {
    sent: usize,
    rejected: usize,
    /// The broker could not be reached; the remaining orders stay planned.
    unavailable: Option<String>,
}

/// Executes a claimed plan (status `executing`) of a broker-linked account: synchronises the
/// account, then submits what can be submitted now. The caller holds the trading lock. Errors
/// before anything was sent fail the plan; after that, problems are recorded per order.
pub async fn execute_claimed(
    state: &Arc<AppState>,
    plan: &trading::Plan,
    actor: &str,
) -> Result<ExecutionReport> {
    let client = state.pool.get().await?;
    let account = trading::account(&client, plan.account_id)
        .await?
        .ok_or_else(|| anyhow!("account {} not found", plan.account_id))?;
    drop(client);
    let broker = client_for(state, &account)?;
    sync_locked(state, account.id, actor)
        .await
        .context("synchronising with the broker before execution")?;
    let round = submit_eligible(state, broker.as_ref(), &account, plan).await?;
    if round.sent == 0
        && let Some(reason) = &round.unavailable
    {
        bail!("the broker is unavailable: {reason}");
    }
    let client = state.pool.get().await?;
    let orders = trading::orders_for_plan(&client, plan.id).await?;
    drop(client);
    let open = orders
        .iter()
        .any(|o| matches!(o.status.as_str(), "planned" | "submitting" | "submitted"));
    if !open {
        return finalize(state, plan.id, actor).await;
    }
    tracing::info!(
        plan = plan.id,
        submitted = round.sent,
        rejected = round.rejected,
        actor,
        "plan orders sent to the broker"
    );
    system::audit(
        &state.pool.get().await?,
        actor,
        "plan.submitted",
        "plan",
        &plan.id.to_string(),
        json!({"broker": account.broker, "submitted": round.sent, "rejected": round.rejected}),
        "",
    )
    .await?;
    Ok(ExecutionReport {
        plan_id: plan.id,
        status: "executing".into(),
        submitted: round.sent,
        rejected: round.rejected,
        ..ExecutionReport::default()
    })
}

/// Submits the plan's planned orders that may go now: every sell, and the buys once no sell is
/// working any more (or the sells have worked for [`BUY_WAIT`]).
async fn submit_eligible(
    state: &AppState,
    broker: &dyn Broker,
    account: &Account,
    plan: &trading::Plan,
) -> Result<Submitted> {
    let now = state.now();
    let client = state.pool.get().await?;
    let orders = trading::orders_for_plan(&client, plan.id).await?;
    let execution: ExecutionSettings = settings::get(&client, settings::EXECUTION).await?;
    drop(client);
    // Buys wait while a sell is still to be sent or working (for at most `BUY_WAIT`).
    let sells_pending = orders.iter().any(|o| {
        o.side == "sell"
            && (o.status == "planned"
                || (matches!(o.status.as_str(), "submitting" | "submitted")
                    && o.submitted_at.is_none_or(|t| now - t < BUY_WAIT)))
    });
    let mut eligible: Vec<&Order> = orders
        .iter()
        .filter(|o| o.status == "planned" && (o.side == "sell" || !sells_pending))
        .collect();
    // Sells first (they fund the buys), then the plan's own order.
    eligible.sort_by_key(|o| (o.side != "sell", o.sequence, o.id));
    let mut round = Submitted::default();
    if eligible.is_empty() {
        return Ok(round);
    }

    let snapshot = match broker.snapshot().await {
        Ok(s) => s,
        Err(error) => {
            round.unavailable = Some(error.to_string());
            return Ok(round);
        }
    };
    if let Err(error) = check_identity(account, &snapshot) {
        round.unavailable = Some(error.to_string());
        return Ok(round);
    }
    let symbols: Vec<String> = eligible.iter().map(|o| o.symbol.clone()).collect();
    let quotes = marketdata::fresh_quotes(state, &symbols).await?;
    let mut available_cash = to_f(
        snapshot
            .cash_in(BASE_CURRENCY)
            .map(|c| c.available)
            .unwrap_or_default(),
    );
    let mut sellable: HashMap<String, Decimal> = snapshot
        .holdings
        .iter()
        .filter_map(|h| Some((h.symbol.clone()?, h.available_qty.floor())))
        .collect();
    let costs = &execution.costs;

    for order in eligible {
        let side = Side::parse(&order.side).ok_or_else(|| anyhow!("bad side {}", order.side))?;
        let reject = |reason: &'static str| reason;
        let verdict: Result<(f64, Option<DateTime<Utc>>), &str> = match quotes.get(&order.symbol) {
            None => Err(reject("no_quote")),
            Some((price, quote_ts, fetched_at)) => {
                let age = (now - quote_ts.unwrap_or(*fetched_at)).num_seconds();
                if *price <= 0.0 {
                    Err(reject("no_quote"))
                } else if age > execution.max_quote_age_secs as i64 {
                    Err(reject("stale_quote"))
                } else if order.ref_price > 0.0
                    && (price / order.ref_price - 1.0).abs() > execution.max_price_deviation
                {
                    Err(reject("price_moved"))
                } else {
                    Ok((*price, *quote_ts))
                }
            }
        };
        let client = state.pool.get().await?;
        let (quote, quote_ts) = match verdict {
            Ok(v) => v,
            Err(reason) => {
                trading::set_order_status(&client, order.id, "rejected", Some(reason), None)
                    .await?;
                round.rejected += 1;
                continue;
            }
        };
        let limit = limit_price(side, quote, order.ref_price, execution.max_price_deviation);
        let limit_f = to_f(limit);
        // Whole shares only.
        let mut qty = order.qty.floor();
        match side {
            Side::Sell => {
                let held = sellable.get(&order.symbol).copied().unwrap_or_default();
                qty = qty.min(held);
            }
            Side::Buy => {
                let cost_of = |q: Decimal| {
                    let q = to_f(q);
                    q * limit_f + costs.commission(q, limit_f)
                };
                if cost_of(qty) > available_cash {
                    qty = to_dec((available_cash / (limit_f * (1.0 + 1e-9))).floor());
                    while qty > Decimal::ZERO && cost_of(qty) > available_cash {
                        qty -= Decimal::ONE;
                    }
                }
            }
        }
        if qty <= Decimal::ZERO {
            let reason = match side {
                Side::Sell => "no_position",
                Side::Buy if order.qty < Decimal::ONE => "below_one_share",
                Side::Buy => "insufficient_cash",
            };
            trading::set_order_status(&client, order.id, "rejected", Some(reason), None).await?;
            round.rejected += 1;
            continue;
        }
        trading::mark_submitting(&client, order.id, qty, limit, (quote, quote_ts)).await?;
        drop(client);
        let request = OrderRequest {
            broker_symbol: broker.broker_symbol(&order.symbol),
            side,
            qty,
            limit_price: limit,
            remark: remark(order.id),
        };
        let outcome = broker.submit(&request).await;
        let client = state.pool.get().await?;
        match outcome {
            Ok(broker_order_id) => {
                trading::mark_submitted(&client, order.id, &broker_order_id, "submitted").await?;
                round.sent += 1;
                match side {
                    Side::Sell => {
                        if let Some(held) = sellable.get_mut(&order.symbol) {
                            *held -= qty;
                        }
                    }
                    Side::Buy => {
                        available_cash -=
                            to_f(qty) * limit_f + costs.commission(to_f(qty), limit_f);
                    }
                }
            }
            Err(BrokerError::Refused(message)) => {
                trading::update_broker_order(
                    &client,
                    order.id,
                    "rejected",
                    Some("broker_rejected"),
                    "rejected",
                    Some(&message),
                    Decimal::ZERO,
                )
                .await?;
                round.rejected += 1;
            }
            Err(
                error @ (BrokerError::Unavailable(_)
                | BrokerError::NotSignedIn(_)
                | BrokerError::WrongAccount(_)),
            ) => {
                // Nothing was sent: the order goes back to planned and is tried again later.
                client
                    .execute(
                        "UPDATE orders SET status = 'planned', broker_qty = NULL, limit_price = NULL, quote_price = NULL,
                           quote_ts = NULL, submitted_at = NULL, broker_message = $2, updated_at = app_now()
                         WHERE id = $1",
                        &[&order.id, &error.to_string()],
                    )
                    .await?;
                round.unavailable = Some(error.to_string());
                break;
            }
            Err(error) => {
                // The request may or may not have reached the broker: the order stays
                // `submitting` and is looked up by its remark.
                client
                    .execute(
                        "UPDATE orders SET broker_message = $2, updated_at = app_now() WHERE id = $1",
                        &[&order.id, &error.to_string()],
                    )
                    .await?;
                tracing::warn!(order = order.id, error = %error, "order submission outcome unknown");
            }
        }
    }
    Ok(round)
}

// ---------------------------------------------------------------------------------------------
// Following orders
// ---------------------------------------------------------------------------------------------

/// Follows every broker-linked account with working orders or executing plans (scheduler).
pub async fn reconcile(state: &Arc<AppState>) -> Result<()> {
    let client = state.pool.get().await?;
    let rows = client
        .query(
            "SELECT DISTINCT a.id FROM accounts a
             WHERE a.broker <> 'sim' AND (
                 EXISTS (SELECT 1 FROM plans p WHERE p.account_id = a.id AND p.status = 'executing')
                 OR EXISTS (SELECT 1 FROM orders o WHERE o.account_id = a.id AND o.status IN ('submitting', 'submitted')))
             ORDER BY a.id",
            &[],
        )
        .await?;
    drop(client);
    for row in rows {
        let account_id: i64 = row.get(0);
        if let Err(error) = reconcile_account(state, account_id).await {
            tracing::warn!(account = account_id, error = %format!("{error:#}"), "following broker orders failed");
        }
    }
    Ok(())
}

/// Reads the account's orders and executions back from the broker, books new fills, sends what
/// became eligible, cancels what is past its deadline and finalises finished plans.
pub async fn reconcile_account(state: &Arc<AppState>, account_id: i64) -> Result<()> {
    let _guard = state.trading_lock.lock().await;
    let client = state.pool.get().await?;
    let account = trading::account(&client, account_id)
        .await?
        .ok_or_else(|| anyhow!("account {account_id} not found"))?;
    let plan_ids: Vec<i64> = client
        .query(
            "SELECT id FROM plans WHERE account_id = $1 AND (status = 'executing' OR id IN (
                 SELECT plan_id FROM orders WHERE account_id = $1 AND status IN ('submitting', 'submitted')))
             ORDER BY id",
            &[&account_id],
        )
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect();
    drop(client);
    if plan_ids.is_empty() {
        return Ok(());
    }
    let broker = client_for(state, &account)?;
    let broker_orders = broker.today_orders().await?;
    let executions = broker.today_executions().await?;
    let mut by_order: HashMap<&str, Vec<&Execution>> = HashMap::new();
    for execution in &executions {
        by_order
            .entry(execution.order_id.as_str())
            .or_default()
            .push(execution);
    }

    for plan_id in plan_ids {
        let client = state.pool.get().await?;
        let plan = trading::plan(&client, plan_id)
            .await?
            .ok_or_else(|| anyhow!("plan {plan_id} not found"))?;
        let orders = trading::orders_for_plan(&client, plan_id).await?;
        drop(client);
        for order in orders
            .iter()
            .filter(|o| matches!(o.status.as_str(), "submitting" | "submitted"))
        {
            // One order the broker cannot account for must not hold up the others.
            if let Err(error) = follow_order(
                state,
                broker.as_ref(),
                &account,
                order,
                &broker_orders,
                &by_order,
            )
            .await
            {
                tracing::warn!(order = order.id, error = %format!("{error:#}"), "following a broker order failed");
            }
        }
        if plan.status != "executing" {
            continue;
        }
        let now = state.now();
        let session_open = state
            .calendar
            .session(plan.trade_date)
            .is_some_and(|s| s.contains(now));
        if now >= plan.deadline || !session_open {
            wind_down(state, broker.as_ref(), &plan, "deadline").await?;
        } else {
            submit_eligible(state, broker.as_ref(), &account, &plan).await?;
        }
        let client = state.pool.get().await?;
        let open = trading::orders_for_plan(&client, plan_id)
            .await?
            .iter()
            .any(|o| matches!(o.status.as_str(), "planned" | "submitting" | "submitted"));
        drop(client);
        if !open {
            finalize(state, plan_id, "scheduler").await?;
        }
    }
    Ok(())
}

/// Cancels the plan's working orders at the broker and ends its planned ones (`reason`:
/// `deadline` → expired, otherwise cancelled).
async fn wind_down(
    state: &AppState,
    broker: &dyn Broker,
    plan: &trading::Plan,
    reason: &str,
) -> Result<Vec<String>> {
    let client = state.pool.get().await?;
    let orders = trading::orders_for_plan(&client, plan.id).await?;
    let (status, status_reason) = if reason == "deadline" {
        ("expired", "deadline")
    } else {
        ("cancelled", "plan_cancelled")
    };
    for order in orders.iter().filter(|o| o.status == "planned") {
        trading::set_order_status(&client, order.id, status, Some(status_reason), None).await?;
    }
    drop(client);
    let mut errors = Vec::new();
    for order in orders.iter().filter(|o| {
        // Not again for an order whose cancellation the broker is already processing.
        o.status == "submitted"
            && !o
                .broker_status
                .as_deref()
                .is_some_and(|s| s.contains("Cancel"))
    }) {
        if let Some(id) = &order.broker_order_id
            && let Err(error) = broker.cancel(id).await
        {
            tracing::warn!(order = order.id, error = %error, "cancelling the broker order failed");
            errors.push(format!("order {}: {error}", order.id));
        }
    }
    Ok(errors)
}

/// The broker order a submission with an unknown outcome became, if any: the order carrying
/// its remark, or else one with the same symbol, side, quantity and limit price placed since
/// shortly before the submission and not already known as another order.
async fn find_submission(
    client: &impl GenericClient,
    broker: &dyn Broker,
    account: &Account,
    order: &Order,
    broker_orders: &[BrokerOrder],
) -> Result<Option<BrokerOrder>> {
    let (Some(qty), Some(limit), Some(at)) =
        (order.broker_qty, order.limit_price, order.submitted_at)
    else {
        return Ok(None);
    };
    if let Some(tagged) = broker_orders.iter().find(|o| o.remark == remark(order.id)) {
        return Ok(Some(tagged.clone()));
    }
    let symbol = broker.broker_symbol(&order.symbol);
    let side = Side::parse(&order.side);
    for candidate in broker_orders {
        let same = candidate.broker_symbol.eq_ignore_ascii_case(&symbol)
            && Some(candidate.side) == side
            && candidate.qty == qty
            && candidate.limit_price.map(|p| p.normalize()) == Some(limit.normalize())
            && candidate
                .submitted_at
                .is_none_or(|t| t >= at - Duration::minutes(2));
        if !same {
            continue;
        }
        let known: bool = client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM orders WHERE account_id = $1 AND broker_order_id = $2)",
                &[&account.id, &candidate.order_id],
            )
            .await?
            .get(0);
        if !known {
            return Ok(Some(candidate.clone()));
        }
    }
    Ok(None)
}

/// Brings one working order up to date with the broker.
async fn follow_order(
    state: &AppState,
    broker: &dyn Broker,
    account: &Account,
    order: &Order,
    broker_orders: &[BrokerOrder],
    executions: &HashMap<&str, Vec<&Execution>>,
) -> Result<()> {
    let now = state.now();
    let found = match &order.broker_order_id {
        Some(id) => match broker_orders.iter().find(|o| &o.order_id == id) {
            Some(o) => Some(o.clone()),
            // An order of an earlier day is not in today's list.
            None => Some(broker.order(id).await?),
        },
        None => {
            let client = state.pool.get().await?;
            let found = find_submission(&client, broker, account, order, broker_orders).await?;
            if let Some(o) = &found {
                trading::mark_submitted(&client, order.id, &o.order_id, &o.status).await?;
            }
            found
        }
    };
    let Some(broker_order) = found else {
        if order
            .submitted_at
            .is_none_or(|t| now - t > SUBMIT_LOST_AFTER)
        {
            let client = state.pool.get().await?;
            trading::update_broker_order(
                &client,
                order.id,
                "rejected",
                Some("submit_lost"),
                "unknown",
                Some("the broker has no record of this order"),
                order.filled_qty,
            )
            .await?;
        }
        return Ok(());
    };

    // New executions become fills.
    let mut filled = order.filled_qty;
    let mut client = state.pool.get().await?;
    let listed = executions
        .get(broker_order.order_id.as_str())
        .cloned()
        .unwrap_or_default();
    for execution in listed {
        if trading::has_broker_fill(&client, account.id, &execution.trade_id).await? {
            continue;
        }
        let tx = client.transaction().await?;
        book_fill(state, &tx, account, order, execution).await?;
        tx.commit().await?;
        filled += execution.qty;
    }
    // An ended order whose executions are not listed is booked from its own totals.
    // (only once the order was already seen ended on an earlier pass, so a fill is never
    // booked twice when its execution is merely listed a moment later).
    let seen_ended = order.broker_status.as_deref() == Some(broker_order.status.as_str());
    if broker_order.state.is_final()
        && broker_order.executed_qty > filled
        && seen_ended
        && let Some(avg) = broker_order.executed_price
    {
        let booked_value: Decimal = trading::list_fills(
            &client,
            account.id,
            &trading::FillFilter {
                symbol: Some(order.symbol.clone()),
                from: None,
                to: None,
                plan_id: Some(order.plan_id),
                limit: 1000,
                offset: 0,
            },
        )
        .await?
        .0
        .iter()
        .filter(|f| f.order_id == order.id)
        .map(|f| f.qty * f.price)
        .sum();
        let missing = broker_order.executed_qty - filled;
        let price = ((avg * broker_order.executed_qty - booked_value) / missing).round_dp(6);
        let synthetic = Execution {
            trade_id: format!(
                "order-{}-{}",
                broker_order.order_id, broker_order.executed_qty
            ),
            order_id: broker_order.order_id.clone(),
            broker_symbol: broker_order.broker_symbol.clone(),
            qty: missing,
            price: if price > Decimal::ZERO { price } else { avg },
            done_at: broker_order.updated_at,
        };
        let tx = client.transaction().await?;
        book_fill(state, &tx, account, order, &synthetic).await?;
        tx.commit().await?;
        filled += missing;
    }

    let (status, reason) = if broker_order.state.is_final() {
        if broker_order.executed_qty > filled {
            // Executions not readable yet: try again next time.
            ("submitted", None)
        } else if filled >= order.qty {
            ("filled", None)
        } else if filled > Decimal::ZERO {
            let reason = match (broker_order.state, order.side.as_str()) {
                (OrderState::Filled, "sell") => "reduced_to_position",
                (OrderState::Filled, _) => "reduced_for_cash",
                (OrderState::Cancelled, _) => "cancelled",
                (OrderState::Expired, _) => "expired",
                _ => "broker_rejected",
            };
            ("partially_filled", Some(reason))
        } else {
            match broker_order.state {
                OrderState::Cancelled => ("cancelled", Some("broker_cancelled")),
                OrderState::Expired => ("expired", Some("broker_expired")),
                _ => ("rejected", Some("broker_rejected")),
            }
        }
    } else {
        ("submitted", None)
    };
    if status != "submitted" && filled > Decimal::ZERO {
        // The broker's charges for the order (its detail has them once it ended).
        let charges = match broker_order.charges {
            Some(c) => Some(c),
            None => match broker.order(&broker_order.order_id).await {
                Ok(detail) => detail.charges,
                Err(error) => {
                    tracing::warn!(order = order.id, error = %error, "reading the order's charges failed; synchronisation will settle them");
                    None
                }
            },
        };
        if let Some(charges) = charges.filter(|c| c.commission + c.fees > Decimal::ZERO) {
            let tx = client.transaction().await?;
            book_charges(&tx, account, order, charges).await?;
            tx.commit().await?;
        }
    }
    let message = (!broker_order.message.is_empty()).then_some(broker_order.message.as_str());
    trading::update_broker_order(
        &client,
        order.id,
        status,
        reason,
        &broker_order.status,
        message,
        filled,
    )
    .await?;
    Ok(())
}

/// Books one broker execution as a fill: position, average cost, realised P&L and cash, exactly
/// as the simulator does. The broker's charges are booked when the order ends
/// ([`book_charges`]).
async fn book_fill(
    state: &AppState,
    tx: &impl GenericClient,
    account: &Account,
    order: &Order,
    execution: &Execution,
) -> Result<()> {
    let side = Side::parse(&order.side).ok_or_else(|| anyhow!("bad side {}", order.side))?;
    let locked = trading::lock_account(tx, account.id).await?;
    let now = state.now();
    let mut position = trading::position_for_update(tx, account.id, &order.symbol)
        .await?
        .unwrap_or(Position {
            symbol: order.symbol.clone(),
            qty: Decimal::ZERO,
            avg_cost: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
            dividends: Decimal::ZERO,
            opened_at: None,
            updated_at: now,
        });
    let qty = execution.qty;
    let price = execution.price.round_dp(6);
    let notional = (qty * price).round_dp(2);
    let quote_price = order.quote_price.unwrap_or(order.ref_price);
    let slippage = dec((to_f(price) - quote_price).abs() * to_f(qty), 2);
    let mut realized = None;
    let cash_delta = match side {
        Side::Buy => {
            let total_cost = position.qty * position.avg_cost + notional;
            position.qty += qty;
            position.avg_cost = if position.qty > Decimal::ZERO {
                (total_cost / position.qty).round_dp(6)
            } else {
                Decimal::ZERO
            };
            if position.opened_at.is_none() {
                position.opened_at = Some(now);
            }
            -notional
        }
        Side::Sell => {
            let sold = qty.min(position.qty);
            let pnl = (notional - sold * position.avg_cost).round_dp(2);
            position.realized_pnl += pnl;
            position.qty -= qty;
            if position.qty <= Decimal::ZERO {
                position.qty = Decimal::ZERO;
                position.opened_at = None;
            }
            realized = Some(pnl);
            notional
        }
    };
    trading::save_position(tx, account.id, &position).await?;
    let fill_id = trading::insert_fill(
        tx,
        &NewFill {
            order_id: order.id,
            account_id: account.id,
            symbol: order.symbol.clone(),
            side: order.side.clone(),
            qty,
            price,
            quote_price,
            quote_ts: order.quote_ts,
            notional,
            commission: Decimal::ZERO,
            fees: Decimal::ZERO,
            slippage,
            realized_pnl: realized,
            broker_trade_id: Some(execution.trade_id.clone()),
            executed_at: execution.done_at,
        },
    )
    .await?;
    if locked.cash + cash_delta < Decimal::ZERO {
        // The account's cash lags the broker's (e.g. a deposit not synchronised yet): catch up
        // by the shortfall now; the next synchronisation settles the exact balance.
        trading::post_cash(
            tx,
            account.id,
            "adjustment",
            -(locked.cash + cash_delta),
            None,
            Some("broker_sync"),
            None,
            &format!("catch-up with {} before a fill", account.broker),
        )
        .await?;
    }
    trading::post_cash(
        tx,
        account.id,
        "trade",
        cash_delta,
        Some(&order.symbol),
        Some("fill"),
        Some(fill_id.to_string()),
        &format!("{} {} @ {}", order.side, qty, price),
    )
    .await?;
    Ok(())
}

/// Books what the broker charged for an order on its last fill: the cash, the fill's
/// commission and fees, and the position's cost (buys) or realised P&L (sells). Once per order.
async fn book_charges(
    tx: &impl GenericClient,
    account: &Account,
    order: &Order,
    charges: crate::brokers::Charges,
) -> Result<()> {
    trading::lock_account(tx, account.id).await?;
    let row = tx
        .query_opt(
            "SELECT id, COALESCE((SELECT SUM(commission + fees) FROM fills WHERE order_id = $1), 0)
             FROM fills WHERE order_id = $1 ORDER BY id DESC LIMIT 1",
            &[&order.id],
        )
        .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let (fill_id, booked): (i64, Decimal) = (row.get(0), row.get(1));
    if booked > Decimal::ZERO {
        return Ok(());
    }
    let commission = charges.commission.round_dp(2);
    let fees = charges.fees.round_dp(2);
    let total = commission + fees;
    if total <= Decimal::ZERO {
        return Ok(());
    }
    let sell = order.side == "sell";
    tx.execute(
        "UPDATE fills SET commission = $2::numeric, fees = $3::numeric,
           realized_pnl = CASE WHEN $4 THEN realized_pnl - ($2::numeric + $3::numeric) ELSE realized_pnl END
         WHERE id = $1",
        &[&fill_id, &commission, &fees, &sell],
    )
    .await?;
    if let Some(mut position) = trading::position_for_update(tx, account.id, &order.symbol).await? {
        if sell {
            position.realized_pnl -= total;
        } else if position.qty > Decimal::ZERO {
            position.avg_cost =
                ((position.qty * position.avg_cost + total) / position.qty).round_dp(6);
        }
        trading::save_position(tx, account.id, &position).await?;
    }
    let locked = trading::lock_account(tx, account.id).await?;
    if locked.cash < total {
        trading::post_cash(
            tx,
            account.id,
            "adjustment",
            total - locked.cash,
            None,
            Some("broker_sync"),
            None,
            &format!("catch-up with {} before charges", account.broker),
        )
        .await?;
    }
    trading::post_cash(
        tx,
        account.id,
        "trade",
        -total,
        Some(&order.symbol),
        Some("fill"),
        Some(fill_id.to_string()),
        &format!("{} charges", account.broker),
    )
    .await?;
    Ok(())
}

/// Ends a plan whose orders have all ended: its status, summary, audit entry and notification,
/// then a synchronisation now that no order is working.
async fn finalize(state: &Arc<AppState>, plan_id: i64, actor: &str) -> Result<ExecutionReport> {
    let mut client = state.pool.get().await?;
    let tx = client.transaction().await?;
    let plan = trading::plan_for_update(&tx, plan_id)
        .await?
        .ok_or_else(|| anyhow!("plan {plan_id} not found"))?;
    if plan.status != "executing" {
        bail!("plan {plan_id} is {}, not executing", plan.status);
    }
    let orders = trading::orders_for_plan(&tx, plan_id).await?;
    let (fills, _) = trading::list_fills(
        &tx,
        plan.account_id,
        &trading::FillFilter {
            symbol: None,
            from: None,
            to: None,
            plan_id: Some(plan_id),
            limit: 10_000,
            offset: 0,
        },
    )
    .await?;
    let mut report = ExecutionReport {
        plan_id,
        ..ExecutionReport::default()
    };
    for order in &orders {
        match order.status.as_str() {
            "filled" => report.filled += 1,
            "partially_filled" => report.partial += 1,
            "rejected" | "cancelled" | "expired" => report.rejected += 1,
            _ => {}
        }
        if order.submitted_at.is_some() {
            report.submitted += 1;
        }
    }
    for fill in &fills {
        match fill.side.as_str() {
            "buy" => report.bought += f(fill.notional),
            _ => report.sold += f(fill.notional),
        }
        report.costs += f(fill.commission + fill.fees + fill.slippage);
    }
    report.status = if report.filled + report.partial == 0 {
        "failed".into()
    } else if report.rejected > 0 || report.partial > 0 {
        "partially_executed".into()
    } else {
        "executed".into()
    };
    let error = (report.status == "failed").then_some("no order was filled at the broker");
    trading::set_plan_status(&tx, plan_id, &report.status, error).await?;
    trading::update_plan_summary(
        &tx,
        plan_id,
        &json!({"executed": {"filled": report.filled, "partial": report.partial, "rejected": report.rejected,
                              "bought": report.bought, "sold": report.sold, "costs": report.costs,
                              "submitted": report.submitted, "broker": true}}),
    )
    .await?;
    system::audit(
        &tx,
        actor,
        "plan.executed",
        "plan",
        &plan_id.to_string(),
        serde_json::to_value(&report)?,
        "",
    )
    .await?;
    tx.commit().await?;
    let portfolio = portfolios::for_account(&client, plan.account_id).await?;
    drop(client);
    tracing::info!(plan = plan_id, status = %report.status, filled = report.filled, partial = report.partial, rejected = report.rejected, "broker plan finished");
    if let Err(error) = sync_locked(state, plan.account_id, "system").await {
        tracing::warn!(plan = plan_id, error = %format!("{error:#}"), "synchronisation after the plan failed");
    }
    state.emit(ServerEvent::Plan {
        plan_id,
        status: report.status.clone(),
        portfolio_id: portfolio.id,
    });
    state.emit(ServerEvent::Account {
        reason: "execution".into(),
        portfolio_id: portfolio.id,
    });
    let _ = notify::notify_in(
        state,
        &portfolio.tag(),
        Event::PlanExecuted {
            plan_id,
            trade_date: plan.trade_date,
            slot: plan.slot.clone(),
            filled: report.filled + report.partial,
            rejected: report.rejected,
            bought: report.bought,
            sold: report.sold,
            costs: report.costs,
        },
    )
    .await;
    Ok(report)
}

/// Stops an executing plan of a broker-linked account: its working orders are cancelled at the
/// broker and its planned ones are not sent. The plan ends (with whatever already filled) once
/// the broker confirms.
pub async fn cancel_executing(
    state: &Arc<AppState>,
    plan_id: i64,
    actor: &str,
    reason: &str,
    ip: &str,
) -> Result<()> {
    let account_id = {
        let _guard = state.trading_lock.lock().await;
        let client = state.pool.get().await?;
        let plan = trading::plan(&client, plan_id)
            .await?
            .ok_or_else(|| anyhow!("plan {plan_id} not found"))?;
        if plan.status != "executing" {
            bail!("plan {plan_id} is {}, not executing", plan.status);
        }
        let account = trading::account(&client, plan.account_id)
            .await?
            .ok_or_else(|| anyhow!("account {} not found", plan.account_id))?;
        if !account.is_external() {
            bail!("plan {plan_id} is being executed by the simulator and cannot be stopped");
        }
        drop(client);
        let broker = client_for(state, &account)?;
        let errors = wind_down(state, broker.as_ref(), &plan, "operator").await?;
        let client = state.pool.get().await?;
        system::audit(
            &client,
            actor,
            "plan.stopped",
            "plan",
            &plan_id.to_string(),
            json!({"reason": reason, "errors": errors}),
            ip,
        )
        .await?;
        if !errors.is_empty() {
            bail!("some orders could not be cancelled: {}", errors.join("; "));
        }
        account.id
    };
    reconcile_account(state, account_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_marketable_and_within_the_plan_tolerance() {
        // Buy at $100: 1% through the quote.
        assert_eq!(limit_price(Side::Buy, 100.0, 100.0, 0.03), dec(101.0, 2));
        // The plan tolerance caps it: plan price 99, tolerance 1.5% → 100.48.
        assert_eq!(limit_price(Side::Buy, 100.0, 99.0, 0.015), dec(100.48, 2));
        // Sells mirror it.
        assert_eq!(limit_price(Side::Sell, 100.0, 100.0, 0.03), dec(99.0, 2));
        assert_eq!(limit_price(Side::Sell, 100.0, 101.0, 0.015), dec(99.49, 2));
        // Cents are rounded away from the market: buys up, sells down.
        assert_eq!(limit_price(Side::Buy, 33.333, 0.0, 0.03), dec(33.67, 2));
        assert_eq!(limit_price(Side::Sell, 33.333, 0.0, 0.03), dec(32.99, 2));
        // Below $1 the increment is $0.0001.
        assert_eq!(limit_price(Side::Buy, 0.5, 0.0, 0.03), dec(0.505, 4));
    }

    #[test]
    fn remarks_identify_orders() {
        assert_eq!(remark(42), "hq-42");
    }
}
