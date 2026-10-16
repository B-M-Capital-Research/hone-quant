//! What the `longbridge` client answers, and how hone-quant reads it.
//!
//! Written against longbridge-terminal 0.30 and the Longbridge SDK 5.2 it ships with:
//!
//! - `longbridge serve` (`src/cli/serve/`) returns the SDK's own serialisation of each
//!   OpenAPI response: decimals as strings (`"388.000"`), absent values as `null`, times as
//!   RFC 3339, enums as their wire names (`FilledStatus`, `Buy`). Errors are JSON-RPC errors:
//!   `-32602` for a bad parameter, `-32000` for everything else, with Longbridge's refusals
//!   reading `openapi error: code=N: message`.
//! - Orders and cancellations are gated: without `"execute": "<code>"` the method answers
//!   `{"dry_run": true, "preview": {…}, "confirmation_code": "580"}` and sends nothing.
//! - `longbridge auth status --format json` (`src/cli/auth.rs`) reports the token and the
//!   signed-in account; it exits 0 even when signed out, so its content is what counts.
//! - A failed one-shot command exits non-zero with `Error: …` (or `Authentication failed: …`)
//!   on standard error.
//!
//! Parsing is tolerant where the output legitimately varies (a number or a string, `Filled` or
//! `FilledStatus`) and strict where a mistake would matter (a dry run must describe exactly
//! the order hone-quant means to place).

use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use quant_core::costs::Side;
use rust_decimal::Decimal;
use serde_json::Value;

use crate::brokers::{
    AccountMode, BrokerError, BrokerOrder, CashBalance, Charges, Execution, Holding, Identity,
    OrderRequest, OrderState,
};

/// Longbridge's account channel for paper-trading accounts (real-money accounts are `lb`).
pub const PAPER_CHANNEL: &str = "lb_papertrading";

/// JSON-RPC: a parameter the method refuses (retrying the same request fails the same way).
const INVALID_PARAMS: i64 = -32602;

fn snippet(text: &str) -> String {
    let text = text.trim();
    let mut out: String = text.chars().take(300).collect();
    if text.chars().count() > 300 {
        out.push('…');
    }
    out
}

/// The JSON document in a one-shot command's standard output (tolerating text before it).
pub fn extract_json(stdout: &str) -> Result<Value, BrokerError> {
    let trimmed = stdout.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    let mut offset = 0;
    for line in stdout.split_inclusive('\n') {
        let start = line.trim_start();
        if (start.starts_with('{') || start.starts_with('['))
            && let Ok(value) = serde_json::from_str(stdout[offset..].trim())
        {
            return Ok(value);
        }
        offset += line.len();
    }
    Err(BrokerError::Parse(format!(
        "no JSON in the client's output: {}",
        snippet(stdout)
    )))
}

fn classify(message: String) -> BrokerError {
    let lower = message.to_lowercase();
    if lower.contains("not authenticated")
        || lower.contains("auth login")
        || lower.contains("authentication failed")
        || (lower.contains("token")
            && (lower.contains("expired")
                || lower.contains("invalid")
                || lower.contains("decrypt")))
    {
        BrokerError::NotSignedIn(message)
    } else if lower.contains("papertrading")
        || lower.contains("paper trading")
        || lower.contains("paper-trading")
    {
        BrokerError::WrongAccount(message)
    } else if lower.contains("openapi error") || lower.contains("api error") {
        BrokerError::Refused(message)
    } else {
        BrokerError::Failed(message)
    }
}

/// A one-shot command that failed (or a `serve` process that exited): its standard error reads
/// `Error: …`, `Error: API error (code N): …` or `Authentication failed: …`.
pub fn classify_failure(stderr: &str, stdout: &str) -> BrokerError {
    let text = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    // The last `Error:` line carries the reason; earlier lines may be notices.
    let line = text
        .lines()
        .rev()
        .find(|l| l.contains("Error:") || l.contains("failed:"))
        .unwrap_or(text);
    let message = snippet(line.trim().trim_start_matches("Error:").trim());
    classify(if message.is_empty() {
        snippet(text)
    } else {
        message
    })
}

/// A JSON-RPC error answered by `serve`.
pub fn rpc_error(code: i64, message: &str) -> BrokerError {
    let message = snippet(message);
    if code == INVALID_PARAMS {
        return BrokerError::Failed(format!(
            "the client refused the request's parameters: {message}"
        ));
    }
    classify(message)
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => {
            let s = s.trim();
            (!s.is_empty() && s != "-").then(|| s.to_string())
        }
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn required_text(object: &Value, key: &str) -> Result<String, BrokerError> {
    text(&object[key]).ok_or_else(|| {
        BrokerError::Parse(format!("missing {key} in {}", snippet(&object.to_string())))
    })
}

/// A decimal written as a string (`"388.000"`) or a number; `null`, `""` and `"-"` are none.
pub fn decimal(value: &Value) -> Option<Decimal> {
    let raw = text(value)?;
    Decimal::from_str(&raw)
        .or_else(|_| Decimal::from_scientific(&raw))
        .ok()
}

fn required_decimal(object: &Value, key: &str) -> Result<Decimal, BrokerError> {
    decimal(&object[key]).ok_or_else(|| {
        BrokerError::Parse(format!("missing {key} in {}", snippet(&object.to_string())))
    })
}

/// An RFC 3339 time, or Unix seconds / milliseconds.
pub fn timestamp(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::String(s) if !s.trim().is_empty() => DateTime::parse_from_rfc3339(s.trim())
            .map(|t| t.with_timezone(&Utc))
            .ok()
            .or_else(|| s.trim().parse::<i64>().ok().and_then(unix)),
        Value::Number(n) => n.as_i64().and_then(unix),
        _ => None,
    }
}

fn unix(n: i64) -> Option<DateTime<Utc>> {
    if n <= 0 {
        None
    } else if n > 100_000_000_000 {
        Utc.timestamp_millis_opt(n).single()
    } else {
        Utc.timestamp_opt(n, 0).single()
    }
}

fn side(value: &Value) -> Result<Side, BrokerError> {
    match text(value).map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("buy") => Ok(Side::Buy),
        Some("sell") => Ok(Side::Sell),
        other => Err(BrokerError::Parse(format!("unknown order side {other:?}"))),
    }
}

/// An order status (`FilledStatus` on the wire, `Filled` in the CLI's own output) as a
/// normalised state. Anything unknown counts as still working, so an order is never treated as
/// ended by mistake.
pub fn order_state(status: &str) -> OrderState {
    let name = status.trim();
    let name = name.strip_suffix("Status").unwrap_or(name);
    match name {
        "Filled" => OrderState::Filled,
        "Rejected" => OrderState::Rejected,
        "Canceled" | "Cancelled" | "PartialWithdrawal" => OrderState::Cancelled,
        "Expired" => OrderState::Expired,
        _ => OrderState::Working,
    }
}

/// Paper for Longbridge's paper channel, live for any other.
pub fn channel_mode(channel: &str) -> AccountMode {
    if channel == PAPER_CHANNEL {
        AccountMode::Paper
    } else {
        AccountMode::Live
    }
}

fn array<'a>(value: &'a Value, what: &str) -> Result<&'a Vec<Value>, BrokerError> {
    value.as_array().ok_or_else(|| {
        BrokerError::Parse(format!(
            "expected a list of {what}, got {}",
            snippet(&value.to_string())
        ))
    })
}

// ---------------------------------------------------------------------------------------------
// auth status
// ---------------------------------------------------------------------------------------------

/// `longbridge auth status --format json`: `{"token": {"status", "dc_region", …}, "account":
/// {"member_id", "account_no", "account_channel", …}}`; `account` is absent when the client
/// could not connect.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthStatus {
    pub token_status: String,
    pub dc_region: Option<String>,
    pub connected: bool,
    pub member_id: Option<String>,
    pub account_no: Option<String>,
    pub account_channel: Option<String>,
}

impl AuthStatus {
    pub fn parse(value: &Value) -> Result<Self, BrokerError> {
        let token = &value["token"];
        let token_status = text(&token["status"]).ok_or_else(|| {
            BrokerError::Parse(format!(
                "auth status without a token status: {}",
                snippet(&value.to_string())
            ))
        })?;
        let account = &value["account"];
        Ok(Self {
            token_status,
            dc_region: text(&token["dc_region"]).map(|r| r.to_ascii_lowercase()),
            connected: account.is_object(),
            member_id: text(&account["member_id"]),
            account_no: text(&account["account_no"]),
            account_channel: text(&account["account_channel"]),
        })
    }

    pub fn signed_in(&self) -> bool {
        matches!(
            self.token_status.as_str(),
            "valid" | "present" | "refresh_pending"
        )
    }

    /// Which account this is, for a client bound to `mode`.
    pub fn identity(&self, mode: AccountMode) -> Result<Identity, BrokerError> {
        if !self.signed_in() {
            return Err(BrokerError::NotSignedIn(format!(
                "the Longbridge client is not signed in (token {}); run `hone-quant longbridge auth login` on the server",
                self.token_status.replace('_', " ")
            )));
        }
        if self.dc_region.as_deref() == Some("us") {
            return Err(BrokerError::Unavailable(
                "accounts in Longbridge's US data centre are not supported (paper trading is offered in the AP data centre)".into(),
            ));
        }
        if !self.connected {
            return Err(BrokerError::Failed(
                "the Longbridge client is signed in but could not reach Longbridge".into(),
            ));
        }
        let account_mode = match self.account_channel.as_deref() {
            Some(channel) => channel_mode(channel),
            // The channel is read from the account's positions and can be missing. A paper
            // client's trading requests are refused for real-money accounts, so for it a
            // working connection is proof enough; a live client must see its channel.
            None if mode == AccountMode::Paper => AccountMode::Paper,
            None => {
                return Err(BrokerError::WrongAccount(
                    "cannot tell whether the signed-in Longbridge account is a live account".into(),
                ));
            }
        };
        if account_mode != mode {
            return Err(BrokerError::WrongAccount(format!(
                "the Longbridge client is signed in to a {} account ({}), not a {} account",
                account_mode.as_str(),
                self.account_channel.as_deref().unwrap_or("unknown channel"),
                mode.as_str()
            )));
        }
        let id = self
            .member_id
            .clone()
            .or_else(|| self.account_no.clone())
            .ok_or_else(|| {
                BrokerError::Failed("Longbridge did not say which account is signed in".into())
            })?;
        let channel = self.account_channel.clone().unwrap_or_else(|| match mode {
            AccountMode::Paper => PAPER_CHANNEL.to_string(),
            AccountMode::Live => "lb".to_string(),
        });
        Ok(Identity {
            channel,
            mode: account_mode,
            account: format!("{}:{id}", account_mode.as_str()),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// trade.account_balance, trade.stock_positions
// ---------------------------------------------------------------------------------------------

/// `trade.account_balance`: `[{"currency", "total_cash", "net_assets", "buy_power", …,
/// "cash_infos": [{"currency", "available_cash", "frozen_cash", "settling_cash",
/// "withdraw_cash"}]}]`. A currency's cash is what is available plus what open orders hold
/// (the client's own `portfolio` view counts it the same way).
pub fn parse_balances(value: &Value) -> Result<Vec<CashBalance>, BrokerError> {
    let mut out: Vec<CashBalance> = Vec::new();
    for balance in array(value, "balances")? {
        let Some(infos) = balance["cash_infos"].as_array() else {
            continue;
        };
        for info in infos {
            let currency = required_text(info, "currency")?.to_ascii_uppercase();
            if out.iter().any(|c| c.currency == currency) {
                continue;
            }
            let available = required_decimal(info, "available_cash")?;
            let frozen = decimal(&info["frozen_cash"]).unwrap_or_default();
            out.push(CashBalance {
                currency,
                total: available + frozen,
                available,
                frozen,
                settling: decimal(&info["settling_cash"]).unwrap_or_default(),
            });
        }
    }
    Ok(out)
}

/// `trade.stock_positions`: `{"list": [{"account_channel": "lb_papertrading", "stock_info":
/// [{"symbol": "NVDA.US", "symbol_name", "quantity", "available_quantity", "currency",
/// "cost_price", "market": "US", "init_quantity"}]}]}`. Returns the channels and the holdings.
pub fn parse_positions(value: &Value) -> Result<(Vec<String>, Vec<Holding>), BrokerError> {
    let list = array(&value["list"], "position channels")?;
    let mut channels = Vec::new();
    let mut holdings = Vec::new();
    for channel in list {
        if let Some(name) = text(&channel["account_channel"]) {
            channels.push(name);
        }
        let Some(stocks) = channel["stock_info"].as_array() else {
            continue;
        };
        for p in stocks {
            let broker_symbol = required_text(p, "symbol")?;
            let market = text(&p["market"]).unwrap_or_default();
            let symbol = broker_symbol
                .strip_suffix(".US")
                .filter(|_| market.is_empty() || market.eq_ignore_ascii_case("US"))
                .map(str::to_string);
            let qty = required_decimal(p, "quantity")?;
            holdings.push(Holding {
                name: text(&p["symbol_name"]).unwrap_or_else(|| broker_symbol.clone()),
                currency: text(&p["currency"]).unwrap_or_default(),
                available_qty: decimal(&p["available_quantity"]).unwrap_or(qty),
                cost_price: decimal(&p["cost_price"]).unwrap_or_default(),
                qty,
                symbol,
                broker_symbol,
            });
        }
    }
    Ok((channels, holdings))
}

// ---------------------------------------------------------------------------------------------
// Orders and executions
// ---------------------------------------------------------------------------------------------

/// `charge_detail` of an order detail: `{"total_amount", "currency", "items": [{"code":
/// "BROKER_FEES" | "THIRD_FEES", "fees": [{"amount", …}]}]}`.
fn charges(value: &Value) -> Option<Charges> {
    if !value.is_object() {
        return None;
    }
    let mut commission = Decimal::ZERO;
    let mut fees = Decimal::ZERO;
    let mut itemised = false;
    for item in value["items"].as_array().into_iter().flatten() {
        let amount: Decimal = item["fees"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| decimal(&f["amount"]))
            .sum();
        itemised = true;
        if text(&item["code"]).as_deref() == Some("BROKER_FEES") {
            commission += amount;
        } else {
            fees += amount;
        }
    }
    if !itemised {
        commission = decimal(&value["total_amount"])?;
    }
    Some(Charges {
        commission: commission.abs(),
        fees: fees.abs(),
    })
}

/// One `Order` (`trade.today_orders`) or `OrderDetail` (`trade.order_detail`).
pub fn parse_order(o: &Value) -> Result<BrokerOrder, BrokerError> {
    let status = required_text(o, "status")?;
    Ok(BrokerOrder {
        order_id: required_text(o, "order_id")?,
        broker_symbol: required_text(o, "symbol")?,
        side: side(&o["side"])?,
        state: order_state(&status),
        status,
        qty: required_decimal(o, "quantity")?,
        executed_qty: decimal(&o["executed_quantity"]).unwrap_or_default(),
        executed_price: decimal(&o["executed_price"]).filter(|p| *p > Decimal::ZERO),
        limit_price: decimal(&o["price"]),
        remark: text(&o["remark"]).unwrap_or_default(),
        message: text(&o["msg"]).unwrap_or_default(),
        submitted_at: timestamp(&o["submitted_at"]),
        updated_at: timestamp(&o["updated_at"]),
        charges: charges(&o["charge_detail"]),
    })
}

/// `trade.today_orders`: a list of `Order`.
pub fn parse_orders(value: &Value) -> Result<Vec<BrokerOrder>, BrokerError> {
    array(value, "orders")?.iter().map(parse_order).collect()
}

/// `trade.today_executions`: `[{"order_id", "trade_id", "symbol", "trade_done_at", "quantity",
/// "price", "side"}]`.
pub fn parse_executions(value: &Value) -> Result<Vec<Execution>, BrokerError> {
    array(value, "executions")?
        .iter()
        .map(|e| {
            Ok(Execution {
                trade_id: required_text(e, "trade_id")?,
                order_id: required_text(e, "order_id")?,
                broker_symbol: required_text(e, "symbol")?,
                qty: required_decimal(e, "quantity")?,
                price: required_decimal(e, "price")?,
                done_at: timestamp(&e["trade_done_at"]),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Placing and cancelling (dry run, then the confirmation code)
// ---------------------------------------------------------------------------------------------

/// The confirmation code of a dry run (`{"dry_run": true, "confirmation_code": "580", …}`).
pub fn confirmation_code(dry_run: &Value) -> Result<String, BrokerError> {
    if dry_run["dry_run"] != Value::Bool(true) {
        return Err(BrokerError::Parse(format!(
            "expected a dry run from the client, got {}",
            snippet(&dry_run.to_string())
        )));
    }
    let code = text(&dry_run["confirmation_code"]).ok_or_else(|| {
        BrokerError::Parse(format!(
            "the dry run has no confirmation code: {}",
            snippet(&dry_run.to_string())
        ))
    })?;
    if code.is_empty() || code.len() > 3 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Err(BrokerError::Parse(format!(
            "unexpected confirmation code {code:?}"
        )));
    }
    Ok(code)
}

/// The dry run must describe exactly the order hone-quant means to place.
pub fn check_order_preview(dry_run: &Value, order: &OrderRequest) -> Result<(), BrokerError> {
    let preview = &dry_run["preview"];
    let mismatch = |what: &str, got: &Value| {
        Err(BrokerError::Parse(format!(
            "the client's dry run does not match the order ({what}: {got}); nothing was placed"
        )))
    };
    if side(&preview["side"]).ok() != Some(order.side) {
        return mismatch("side", &preview["side"]);
    }
    if !text(&preview["symbol"]).is_some_and(|s| s.eq_ignore_ascii_case(&order.broker_symbol)) {
        return mismatch("symbol", &preview["symbol"]);
    }
    if decimal(&preview["quantity"]) != Some(order.qty) {
        return mismatch("quantity", &preview["quantity"]);
    }
    if decimal(&preview["price"]).map(|p| p.normalize()) != Some(order.limit_price.normalize()) {
        return mismatch("price", &preview["price"]);
    }
    if !text(&preview["order_type"]).is_some_and(|t| t.eq_ignore_ascii_case("LO")) {
        return mismatch("order type", &preview["order_type"]);
    }
    Ok(())
}

/// `{"order_id": "…"}` once an order was placed.
pub fn placed_order_id(placed: &Value) -> Result<String, BrokerError> {
    text(&placed["order_id"]).ok_or_else(|| {
        BrokerError::Parse(format!(
            "the client did not return an order id: {}",
            snippet(&placed.to_string())
        ))
    })
}

pub fn check_cancel_preview(dry_run: &Value, order_id: &str) -> Result<(), BrokerError> {
    if text(&dry_run["preview"]["order_id"]).as_deref() != Some(order_id) {
        return Err(BrokerError::Parse(format!(
            "the client's dry run is for another order ({}); nothing was cancelled",
            dry_run["preview"]["order_id"]
        )));
    }
    Ok(())
}
