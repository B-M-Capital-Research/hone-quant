//! External brokers: the accounts a portfolio can trade at instead of hone-quant's own paper
//! simulator.
//!
//! [`Broker`] is what the execution and synchronisation services need from a brokerage account,
//! in hone-quant's own terms (US symbols without a market suffix, `Decimal` money, normalised
//! order states). [`longbridge`] implements it on top of Longbridge's `longbridge` command-line
//! client. Each client is bound to one account mode: a paper client can only ever reach a paper
//! account (the broker itself refuses anything else), so code that holds a paper client cannot
//! trade real money by mistake.

pub mod longbridge;

#[cfg(test)]
pub mod fake;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use quant_core::costs::Side;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// hone-quant's own simulator (no external broker).
pub const SIM: &str = "sim";
pub const LONGBRIDGE: &str = "longbridge";

/// Whether an account trades simulated money or real money.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountMode {
    Paper,
    Live,
}

impl AccountMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AccountMode::Paper => "paper",
            AccountMode::Live => "live",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "paper" => Some(AccountMode::Paper),
            "live" => Some(AccountMode::Live),
            _ => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    /// The client program is missing or cannot be started.
    #[error("{0}")]
    Unavailable(String),
    /// The client has no valid sign-in (or it expired).
    #[error("not signed in to the broker: {0}")]
    NotSignedIn(String),
    /// The broker account is not the kind this client is bound to (e.g. a live account behind a
    /// paper client). Never retried.
    #[error("{0}")]
    WrongAccount(String),
    /// The broker refused the request (an order rejected, an unknown order id, ...).
    #[error("broker refused the request: {0}")]
    Refused(String),
    #[error("broker request failed: {0}")]
    Failed(String),
    #[error("unexpected broker response: {0}")]
    Parse(String),
}

/// Which account the client is connected to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Identity {
    /// The broker's name for the account kind, e.g. Longbridge's `lb_papertrading`.
    pub channel: String,
    pub mode: AccountMode,
    /// Stable identifier stored with the hone-quant account, so two portfolios can never trade
    /// the same broker account.
    pub account: String,
}

/// One currency's cash in the broker account.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CashBalance {
    pub currency: String,
    /// Settled and unsettled cash, including what open orders hold.
    pub total: Decimal,
    /// What new orders can use now.
    pub available: Decimal,
    /// Held by open orders.
    pub frozen: Decimal,
    /// Unsettled proceeds.
    pub settling: Decimal,
}

/// A holding as the broker reports it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Holding {
    /// The broker's symbol, e.g. `NVDA.US`.
    pub broker_symbol: String,
    /// hone-quant's symbol for US listings (`NVDA`); `None` for other markets.
    pub symbol: Option<String>,
    pub name: String,
    pub currency: String,
    pub qty: Decimal,
    /// Shares that can be sold now.
    pub available_qty: Decimal,
    pub cost_price: Decimal,
}

/// Cash and holdings of the broker account.
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub identity: Identity,
    pub cash: Vec<CashBalance>,
    pub holdings: Vec<Holding>,
}

impl Snapshot {
    pub fn cash_in(&self, currency: &str) -> Option<&CashBalance> {
        self.cash.iter().find(|c| c.currency == currency)
    }
}

/// An order's state, normalised across brokers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderState {
    /// Accepted (possibly partly filled) and still able to fill.
    Working,
    /// Completely filled.
    Filled,
    /// Cancelled (possibly after a partial fill).
    Cancelled,
    /// Refused by the broker or the exchange.
    Rejected,
    /// Ended unfilled (or partly filled) at the end of its time in force.
    Expired,
}

impl OrderState {
    pub fn is_final(self) -> bool {
        self != OrderState::Working
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BrokerOrder {
    pub order_id: String,
    pub broker_symbol: String,
    pub side: Side,
    pub state: OrderState,
    /// The broker's own status, verbatim (`FilledStatus`, `NewStatus`, ...).
    pub status: String,
    pub qty: Decimal,
    pub executed_qty: Decimal,
    pub executed_price: Option<Decimal>,
    pub limit_price: Option<Decimal>,
    pub remark: String,
    pub message: String,
    pub submitted_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    /// What the broker charged for the order, when it says (order detail only): its own
    /// commission and third-party fees, in the order's currency.
    pub charges: Option<Charges>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Charges {
    pub commission: Decimal,
    pub fees: Decimal,
}

/// One execution (trade) of an order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Execution {
    pub trade_id: String,
    pub order_id: String,
    pub broker_symbol: String,
    pub qty: Decimal,
    pub price: Decimal,
    pub done_at: Option<DateTime<Utc>>,
}

/// A day limit order for whole shares during the regular session (the only kind hone-quant
/// sends).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrderRequest {
    pub broker_symbol: String,
    pub side: Side,
    pub qty: Decimal,
    pub limit_price: Decimal,
    /// Carried by the broker with the order; hone-quant uses it to find an order whose
    /// submission it did not see complete.
    pub remark: String,
}

/// What the status page shows about a broker connection.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionStatus {
    pub broker: &'static str,
    pub mode: AccountMode,
    /// The client program and its version, when it could be started.
    pub client: Option<String>,
    pub signed_in: bool,
    pub identity: Option<Identity>,
    pub error: Option<String>,
    pub checked_at: DateTime<Utc>,
}

#[async_trait]
pub trait Broker: Send + Sync {
    fn name(&self) -> &'static str;

    /// The account mode this client is bound to.
    fn mode(&self) -> AccountMode;

    /// The broker's symbol for a US listing (`NVDA` → `NVDA.US`).
    fn broker_symbol(&self, symbol: &str) -> String;

    /// Checks the client, the sign-in and the account it reaches. Never fails: problems are
    /// reported in the result.
    async fn status(&self) -> ConnectionStatus;

    async fn snapshot(&self) -> Result<Snapshot, BrokerError>;

    /// Submits an order and returns the broker's order id.
    async fn submit(&self, order: &OrderRequest) -> Result<String, BrokerError>;

    async fn cancel(&self, order_id: &str) -> Result<(), BrokerError>;

    /// Orders of the current trading day.
    async fn today_orders(&self) -> Result<Vec<BrokerOrder>, BrokerError>;

    async fn order(&self, order_id: &str) -> Result<BrokerOrder, BrokerError>;

    /// Executions of the current trading day.
    async fn today_executions(&self) -> Result<Vec<Execution>, BrokerError>;
}

/// The broker clients this server can use, by broker and account mode.
#[derive(Clone, Default)]
pub struct Brokers {
    clients: HashMap<(&'static str, AccountMode), Arc<dyn Broker>>,
}

impl Brokers {
    pub fn with(mut self, client: Arc<dyn Broker>) -> Self {
        self.clients.insert((client.name(), client.mode()), client);
        self
    }

    pub fn get(&self, broker: &str, mode: AccountMode) -> Option<Arc<dyn Broker>> {
        self.clients
            .iter()
            .find(|((name, m), _)| *name == broker && *m == mode)
            .map(|(_, client)| client.clone())
    }

    pub fn all(&self) -> Vec<Arc<dyn Broker>> {
        let mut clients: Vec<_> = self.clients.values().cloned().collect();
        clients.sort_by_key(|c| (c.name(), c.mode().as_str()));
        clients
    }
}
