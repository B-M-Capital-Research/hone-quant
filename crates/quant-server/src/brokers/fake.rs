//! An in-memory broker for tests: a US-dollar cash balance, holdings, orders that fill when the
//! test says so (or immediately, at prices set in advance) and the executions they produce.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use quant_core::costs::Side;
use rust_decimal::Decimal;

use super::{
    AccountMode, Broker, BrokerError, BrokerOrder, CashBalance, Charges, ConnectionStatus,
    Execution, Holding, Identity, OrderRequest, OrderState, Snapshot,
};

#[derive(Default)]
struct Book {
    cash: Decimal,
    /// Symbol (`NVDA`) → (qty, cost price).
    holdings: BTreeMap<String, (Decimal, Decimal)>,
    /// Holdings in other markets, reported as they are.
    foreign: Vec<Holding>,
    orders: Vec<BrokerOrder>,
    executions: Vec<Execution>,
    next_id: u64,
    /// Orders for these symbols fill completely on submission, at these prices.
    auto_fill: HashMap<String, Decimal>,
    /// Submissions fail with this message.
    refuse: Option<String>,
    submitted: usize,
    cancelled: usize,
    /// Charged (and taken from the cash) when an order fills completely.
    charges: Option<Charges>,
    now: Option<DateTime<Utc>>,
}

pub struct FakeBroker {
    mode: AccountMode,
    identity: Mutex<Identity>,
    book: Mutex<Book>,
}

impl FakeBroker {
    pub fn paper(cash: i64) -> Self {
        Self {
            mode: AccountMode::Paper,
            identity: Mutex::new(Identity {
                channel: "lb_papertrading".into(),
                mode: AccountMode::Paper,
                account: "lb_papertrading".into(),
            }),
            book: Mutex::new(Book {
                cash: Decimal::from(cash),
                next_id: 1000,
                ..Book::default()
            }),
        }
    }

    /// A client bound to `mode` that reaches an account of another kind.
    pub fn reaching(self, mode: AccountMode, channel: &str) -> Self {
        self.sign_in_as(mode, channel, channel);
        self
    }

    /// The client is signed in to another account from now on.
    pub fn sign_in_as(&self, mode: AccountMode, channel: &str, account: &str) {
        *self.identity.lock().unwrap() = Identity {
            channel: channel.into(),
            mode,
            account: account.into(),
        };
    }

    pub fn hold(&self, symbol: &str, qty: i64, cost: Decimal) {
        self.book
            .lock()
            .unwrap()
            .holdings
            .insert(symbol.into(), (Decimal::from(qty), cost));
    }

    pub fn hold_foreign(&self, broker_symbol: &str, qty: i64) {
        self.book.lock().unwrap().foreign.push(Holding {
            broker_symbol: broker_symbol.into(),
            symbol: None,
            name: broker_symbol.into(),
            currency: "HKD".into(),
            qty: Decimal::from(qty),
            available_qty: Decimal::from(qty),
            cost_price: Decimal::ONE,
        });
    }

    pub fn set_cash(&self, cash: Decimal) {
        self.book.lock().unwrap().cash = cash;
    }

    pub fn cash(&self) -> Decimal {
        self.book.lock().unwrap().cash
    }

    pub fn qty(&self, symbol: &str) -> Decimal {
        self.book
            .lock()
            .unwrap()
            .holdings
            .get(symbol)
            .map(|h| h.0)
            .unwrap_or_default()
    }

    /// The broker's clock (the test's frozen time instead of the wall clock).
    pub fn at(self, now: DateTime<Utc>) -> Self {
        self.book.lock().unwrap().now = Some(now);
        self
    }

    pub fn charge(&self, charges: Option<Charges>) {
        self.book.lock().unwrap().charges = charges;
    }

    pub fn auto_fill(&self, prices: HashMap<String, Decimal>) {
        self.book.lock().unwrap().auto_fill = prices;
    }

    pub fn refuse(&self, message: Option<&str>) {
        self.book.lock().unwrap().refuse = message.map(str::to_string);
    }

    pub fn orders(&self) -> Vec<BrokerOrder> {
        self.book.lock().unwrap().orders.clone()
    }

    pub fn submitted(&self) -> usize {
        self.book.lock().unwrap().submitted
    }

    pub fn cancelled(&self) -> usize {
        self.book.lock().unwrap().cancelled
    }

    /// Fills `qty` of a working order at `price`.
    pub fn fill(&self, order_id: &str, qty: Decimal, price: Decimal) {
        let mut book = self.book.lock().unwrap();
        fill_locked(&mut book, order_id, qty, price);
    }

    /// Fills every working order completely at `price(symbol)`.
    pub fn fill_working(&self, price: impl Fn(&str) -> Decimal) {
        let mut book = self.book.lock().unwrap();
        let working: Vec<(String, String, Decimal)> = book
            .orders
            .iter()
            .filter(|o| o.state == OrderState::Working)
            .map(|o| {
                (
                    o.order_id.clone(),
                    o.broker_symbol.clone(),
                    o.qty - o.executed_qty,
                )
            })
            .collect();
        for (id, symbol, qty) in working {
            let p = price(symbol.trim_end_matches(".US"));
            fill_locked(&mut book, &id, qty, p);
        }
    }

    /// Ends every working order as the exchange would at the close.
    pub fn expire_working(&self) {
        let mut book = self.book.lock().unwrap();
        for order in book.orders.iter_mut() {
            if order.state == OrderState::Working {
                order.state = OrderState::Expired;
                order.status = "ExpiredStatus".into();
            }
        }
    }
}

fn fill_locked(book: &mut Book, order_id: &str, qty: Decimal, price: Decimal) {
    let next = book.executions.len() + 1;
    let now = book.now.unwrap_or_else(Utc::now);
    let charges = book.charges;
    let Some(order) = book.orders.iter_mut().find(|o| o.order_id == order_id) else {
        panic!("no order {order_id}");
    };
    assert_eq!(
        order.state,
        OrderState::Working,
        "order {order_id} is final"
    );
    let qty = qty.min(order.qty - order.executed_qty);
    let symbol = order.broker_symbol.trim_end_matches(".US").to_string();
    let previous = order.executed_qty * order.executed_price.unwrap_or_default();
    order.executed_qty += qty;
    order.executed_price = Some((previous + qty * price) / order.executed_qty);
    order.updated_at = Some(now);
    let mut charged = Decimal::ZERO;
    if order.executed_qty == order.qty {
        order.state = OrderState::Filled;
        order.status = "FilledStatus".into();
        if let Some(c) = charges {
            order.charges = Some(c);
            charged = c.commission + c.fees;
        }
    } else {
        order.status = "PartialFilledStatus".into();
    }
    let side = order.side;
    let execution = Execution {
        trade_id: format!("T{next}"),
        order_id: order_id.into(),
        broker_symbol: order.broker_symbol.clone(),
        qty,
        price,
        done_at: Some(now),
    };
    book.executions.push(execution);
    book.cash -= charged;
    let notional = qty * price;
    let entry = book
        .holdings
        .entry(symbol.clone())
        .or_insert((Decimal::ZERO, Decimal::ZERO));
    match side {
        Side::Buy => {
            let cost = entry.0 * entry.1 + notional;
            entry.0 += qty;
            entry.1 = cost / entry.0;
            book.cash -= notional;
        }
        Side::Sell => {
            entry.0 -= qty;
            book.cash += notional;
            if entry.0 <= Decimal::ZERO {
                book.holdings.remove(&symbol);
            }
        }
    }
}

#[async_trait]
impl Broker for FakeBroker {
    fn name(&self) -> &'static str {
        super::LONGBRIDGE
    }

    fn mode(&self) -> AccountMode {
        self.mode
    }

    fn broker_symbol(&self, symbol: &str) -> String {
        format!("{symbol}.US")
    }

    async fn status(&self) -> ConnectionStatus {
        ConnectionStatus {
            broker: super::LONGBRIDGE,
            mode: self.mode,
            client: Some("fake 1.0".into()),
            signed_in: true,
            identity: Some(self.identity.lock().unwrap().clone()),
            error: None,
            checked_at: Utc::now(),
        }
    }

    async fn snapshot(&self) -> Result<Snapshot, BrokerError> {
        let identity = self.identity.lock().unwrap().clone();
        if identity.mode != self.mode {
            return Err(BrokerError::WrongAccount(format!(
                "the signed-in account ({}) is not a {} account",
                identity.channel,
                self.mode.as_str()
            )));
        }
        let book = self.book.lock().unwrap();
        let frozen: Decimal = book
            .orders
            .iter()
            .filter(|o| o.state == OrderState::Working && o.side == Side::Buy)
            .map(|o| (o.qty - o.executed_qty) * o.limit_price.unwrap_or_default())
            .sum();
        let mut holdings: Vec<Holding> = book
            .holdings
            .iter()
            .map(|(symbol, (qty, cost))| {
                let pending_sells: Decimal = book
                    .orders
                    .iter()
                    .filter(|o| {
                        o.state == OrderState::Working
                            && o.side == Side::Sell
                            && o.broker_symbol == format!("{symbol}.US")
                    })
                    .map(|o| o.qty - o.executed_qty)
                    .sum();
                Holding {
                    broker_symbol: format!("{symbol}.US"),
                    symbol: Some(symbol.clone()),
                    name: symbol.clone(),
                    currency: "USD".into(),
                    qty: *qty,
                    available_qty: *qty - pending_sells,
                    cost_price: *cost,
                }
            })
            .collect();
        holdings.extend(book.foreign.iter().cloned());
        Ok(Snapshot {
            identity,
            cash: vec![CashBalance {
                currency: "USD".into(),
                total: book.cash,
                available: book.cash - frozen,
                frozen,
                settling: Decimal::ZERO,
            }],
            holdings,
        })
    }

    async fn submit(&self, request: &OrderRequest) -> Result<String, BrokerError> {
        let mut book = self.book.lock().unwrap();
        if let Some(message) = &book.refuse {
            return Err(BrokerError::Refused(message.clone()));
        }
        let symbol = request.broker_symbol.trim_end_matches(".US").to_string();
        match request.side {
            Side::Sell => {
                let held = book.holdings.get(&symbol).map(|h| h.0).unwrap_or_default();
                if request.qty > held {
                    return Err(BrokerError::Refused("insufficient position".into()));
                }
            }
            Side::Buy => {
                let frozen: Decimal = book
                    .orders
                    .iter()
                    .filter(|o| o.state == OrderState::Working && o.side == Side::Buy)
                    .map(|o| (o.qty - o.executed_qty) * o.limit_price.unwrap_or_default())
                    .sum();
                if request.qty * request.limit_price > book.cash - frozen {
                    return Err(BrokerError::Refused("insufficient buying power".into()));
                }
            }
        }
        book.next_id += 1;
        book.submitted += 1;
        let id = book.next_id.to_string();
        let now = book.now.unwrap_or_else(Utc::now);
        book.orders.push(BrokerOrder {
            order_id: id.clone(),
            broker_symbol: request.broker_symbol.clone(),
            side: request.side,
            state: OrderState::Working,
            status: "NewStatus".into(),
            qty: request.qty,
            executed_qty: Decimal::ZERO,
            executed_price: None,
            limit_price: Some(request.limit_price),
            remark: request.remark.clone(),
            message: String::new(),
            submitted_at: Some(now),
            updated_at: Some(now),
            charges: None,
        });
        if let Some(price) = book.auto_fill.get(&symbol).copied() {
            fill_locked(&mut book, &id, request.qty, price);
        }
        Ok(id)
    }

    async fn cancel(&self, order_id: &str) -> Result<(), BrokerError> {
        let mut book = self.book.lock().unwrap();
        book.cancelled += 1;
        let order = book
            .orders
            .iter_mut()
            .find(|o| o.order_id == order_id)
            .ok_or_else(|| BrokerError::Refused(format!("no order {order_id}")))?;
        if order.state == OrderState::Working {
            order.state = OrderState::Cancelled;
            order.status = "CanceledStatus".into();
        }
        Ok(())
    }

    async fn today_orders(&self) -> Result<Vec<BrokerOrder>, BrokerError> {
        // Like Longbridge's list, without the charges (the order detail has them).
        Ok(self
            .orders()
            .into_iter()
            .map(|o| BrokerOrder { charges: None, ..o })
            .collect())
    }

    async fn order(&self, order_id: &str) -> Result<BrokerOrder, BrokerError> {
        self.orders()
            .into_iter()
            .find(|o| o.order_id == order_id)
            .ok_or_else(|| BrokerError::Refused(format!("no order {order_id}")))
    }

    async fn today_executions(&self) -> Result<Vec<Execution>, BrokerError> {
        Ok(self.book.lock().unwrap().executions.clone())
    }
}
