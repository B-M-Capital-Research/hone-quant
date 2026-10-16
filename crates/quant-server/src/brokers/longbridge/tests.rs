//! The Longbridge client against the client's documented payloads (fixtures) and against a
//! stand-in `longbridge` executable that speaks the `serve` protocol.

use std::path::{Path, PathBuf};

use quant_core::costs::Side;
use rust_decimal::Decimal;
use serde_json::Value;

use super::wire::{self, AuthStatus};
use super::{LongbridgeCli, LongbridgeConfig};
use crate::brokers::{AccountMode, Broker, BrokerError, OrderRequest, OrderState};

/// A decimal literal, exactly as written.
macro_rules! dec {
    ($value:expr) => {
        stringify!($value)
            .replace(' ', "")
            .parse::<Decimal>()
            .unwrap()
    };
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/longbridge")
}

fn fixture(name: &str) -> Value {
    let text = std::fs::read_to_string(fixtures().join(name)).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn balances_take_each_currency_once() {
    let cash = wire::parse_balances(&fixture("serve_account_balance.json")).unwrap();
    assert_eq!(cash.len(), 2);
    let usd = cash.iter().find(|c| c.currency == "USD").unwrap();
    assert_eq!(usd.available, dec!(95210.40));
    assert_eq!(usd.frozen, dec!(1502.10));
    // Cash is what is available plus what open orders hold.
    assert_eq!(usd.total, dec!(96712.50));
    assert_eq!(usd.settling, dec!(-3010.55));
}

#[test]
fn positions_map_us_listings_to_universe_symbols() {
    let (channels, holdings) =
        wire::parse_positions(&fixture("serve_stock_positions.json")).unwrap();
    assert_eq!(channels, vec!["lb_papertrading".to_string()]);
    assert_eq!(holdings.len(), 3);
    let nvda = &holdings[0];
    assert_eq!(nvda.symbol.as_deref(), Some("NVDA"));
    assert_eq!(nvda.qty, dec!(120));
    assert_eq!(nvda.cost_price, dec!(118.4512));
    let avgo = &holdings[1];
    assert_eq!(
        avgo.available_qty,
        dec!(30),
        "shares held by an open sell are not available"
    );
    let tencent = &holdings[2];
    assert_eq!(tencent.symbol, None, "other markets are never mapped");
    assert_eq!(tencent.broker_symbol, "700.HK");
}

#[test]
fn orders_keep_exact_prices_and_normalise_states() {
    let orders = wire::parse_orders(&fixture("serve_today_orders.json")).unwrap();
    assert_eq!(orders.len(), 3);
    let filled = &orders[0];
    assert_eq!(filled.state, OrderState::Filled);
    assert_eq!(filled.status, "FilledStatus");
    assert_eq!(filled.side, Side::Buy);
    assert_eq!(filled.executed_price, Some(dec!(120.172)));
    assert_eq!(filled.limit_price, Some(dec!(121.37)));
    assert_eq!(filled.remark, "hq-41");
    assert!(filled.updated_at.is_some());
    let partial = &orders[1];
    assert_eq!(partial.state, OrderState::Working);
    assert_eq!(partial.side, Side::Sell);
    assert_eq!(partial.executed_qty, dec!(4));
    let rejected = &orders[2];
    assert_eq!(rejected.state, OrderState::Rejected);
    assert_eq!(rejected.executed_price, None);
    assert_eq!(rejected.message, "Insufficient buying power");
    assert_eq!(rejected.charges, None, "the list carries no charges");
}

#[test]
fn every_order_status_has_a_state() {
    for (status, state) in [
        ("NotReported", OrderState::Working),
        ("WaitToNew", OrderState::Working),
        ("NewStatus", OrderState::Working),
        ("PartialFilledStatus", OrderState::Working),
        ("PendingCancelStatus", OrderState::Working),
        ("WaitToCancel", OrderState::Working),
        ("PendingReplaceStatus", OrderState::Working),
        ("ReplacedStatus", OrderState::Working),
        ("Unknown", OrderState::Working),
        ("FilledStatus", OrderState::Filled),
        ("Filled", OrderState::Filled),
        ("RejectedStatus", OrderState::Rejected),
        ("CanceledStatus", OrderState::Cancelled),
        ("PartialWithdrawal", OrderState::Cancelled),
        ("ExpiredStatus", OrderState::Expired),
    ] {
        assert_eq!(wire::order_state(status), state, "{status}");
    }
}

#[test]
fn order_detail_carries_the_charges() {
    let detail = wire::parse_order(&fixture("serve_order_detail.json")).unwrap();
    let charges = detail.charges.unwrap();
    assert_eq!(charges.commission, dec!(0.99));
    assert_eq!(charges.fees, dec!(0.04));
}

#[test]
fn executions_are_exact() {
    let executions = wire::parse_executions(&fixture("serve_today_executions.json")).unwrap();
    assert_eq!(executions.len(), 3);
    assert_eq!(executions[1].price, dec!(120.205));
    assert_eq!(executions[1].qty, dec!(8));
    assert_eq!(executions[0].order_id, "1069345312341434368");
    assert!(executions[0].done_at.is_some());
}

fn request() -> OrderRequest {
    OrderRequest {
        broker_symbol: "NVDA.US".into(),
        side: Side::Buy,
        qty: dec!(20),
        limit_price: dec!(121.37),
        remark: "hq-41".into(),
    }
}

#[test]
fn a_dry_run_must_match_the_order_exactly() {
    let dry_run = fixture("serve_submit_dry_run.json");
    assert_eq!(wire::confirmation_code(&dry_run).unwrap(), "073");
    wire::check_order_preview(&dry_run, &request()).unwrap();
    for change in [
        |r: &mut OrderRequest| r.side = Side::Sell,
        |r: &mut OrderRequest| r.qty = dec!(21),
        |r: &mut OrderRequest| r.limit_price = dec!(121.36),
        |r: &mut OrderRequest| r.broker_symbol = "AMD.US".into(),
    ] {
        let mut other = request();
        change(&mut other);
        assert!(wire::check_order_preview(&dry_run, &other).is_err());
    }
    // Trailing zeros are the same price.
    let mut same = request();
    same.limit_price = dec!(121.370);
    wire::check_order_preview(&dry_run, &same).unwrap();
    // Something that is not a dry run never yields a code.
    assert!(wire::confirmation_code(&serde_json::json!({"order_id": "1"})).is_err());
    let cancel = fixture("serve_cancel_dry_run.json");
    assert_eq!(wire::confirmation_code(&cancel).unwrap(), "412");
    wire::check_cancel_preview(&cancel, "1069345399999999999").unwrap();
    assert!(wire::check_cancel_preview(&cancel, "1").is_err());
}

#[test]
fn identity_comes_from_auth_status() {
    let paper = AuthStatus::parse(&fixture("auth_status_paper.json")).unwrap();
    let identity = paper.identity(AccountMode::Paper).unwrap();
    assert_eq!(identity.channel, "lb_papertrading");
    assert_eq!(identity.account, "paper:3162001");
    assert!(matches!(
        paper.identity(AccountMode::Live),
        Err(BrokerError::WrongAccount(_))
    ));
    let live = AuthStatus::parse(&fixture("auth_status_live.json")).unwrap();
    assert!(matches!(
        live.identity(AccountMode::Paper),
        Err(BrokerError::WrongAccount(_))
    ));
    let out = AuthStatus::parse(&fixture("auth_status_signed_out.json")).unwrap();
    assert!(!out.signed_in());
    assert!(matches!(
        out.identity(AccountMode::Paper),
        Err(BrokerError::NotSignedIn(_))
    ));
    // US data centre accounts are refused.
    let mut us = paper.clone();
    us.dc_region = Some("us".into());
    assert!(matches!(
        us.identity(AccountMode::Paper),
        Err(BrokerError::Unavailable(_))
    ));
}

#[test]
fn failures_are_classified() {
    assert!(matches!(
        wire::classify_failure(
            "Authentication failed: Not authenticated. Please run `longbridge auth login` first.\n",
            ""
        ),
        BrokerError::NotSignedIn(_)
    ));
    assert!(matches!(
        wire::classify_failure(
            "Error: API error (code 602012): order quantity invalid\n  trace_id: abc\n",
            ""
        ),
        BrokerError::Refused(_)
    ));
    assert!(matches!(
        wire::classify_failure("Error: error sending request for url (https://…)\n", ""),
        BrokerError::Failed(_)
    ));
    assert!(matches!(
        wire::rpc_error(-32000, "openapi error: code=602012: order quantity invalid"),
        BrokerError::Refused(_)
    ));
    assert!(matches!(
        wire::rpc_error(-32602, "`quantity` must be a string"),
        BrokerError::Failed(_)
    ));
    assert!(matches!(
        wire::rpc_error(
            -32000,
            "openapi error: code=403: papertrading account mismatch"
        ),
        BrokerError::WrongAccount(_)
    ));
}

#[test]
fn json_is_found_after_a_line_of_text() {
    let value = wire::extract_json(
        "Submitting Buy order: 20 NVDA.US @ 121.37\n{\n  \"order_id\": \"7\"\n}\n",
    )
    .unwrap();
    assert_eq!(value["order_id"], "7");
    assert!(wire::extract_json("nothing here").is_err());
}

// ---------------------------------------------------------------------------------------------
// A stand-in executable
// ---------------------------------------------------------------------------------------------

/// A `longbridge` that answers `--version`, `auth status` and `serve` from the fixtures and logs
/// every `serve` request (and whether the paper-trading guard was set) to `requests.log`.
#[cfg(unix)]
fn fake_client(dir: &Path, auth: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let fixtures = fixtures();
    let script = format!(
        r#"#!/bin/sh
F='{fixtures}'
LOG='{log}'
one_line() {{ tr -d '\n' < "$F/$1"; }}
case "$1" in
  --version) echo "longbridge 0.30.0"; exit 0 ;;
  auth) cat "$F/{auth}"; exit 0 ;;
  serve) ;;
  *) echo "Error: unexpected command $*" >&2; exit 2 ;;
esac
if [ "$LONGBRIDGE_PAPERTRADING" != "true" ]; then
  echo "Authentication failed: papertrading guard missing" >&2
  exit 1
fi
while IFS= read -r line; do
  echo "$line" >> "$LOG"
  id=$(printf '%s' "$line" | sed -E 's/.*"id":([0-9]+).*/\1/')
  method=$(printf '%s' "$line" | sed -E 's/.*"method":"([^"]+)".*/\1/')
  params=$(printf '%s' "$line" | sed -E 's/.*"params":(\{{[^}}]*\}}).*/\1/')
  # A quote notification first, as a live session would send.
  echo '{{"jsonrpc":"2.0","method":"quote.updated","params":{{"symbol":"NVDA.US"}}}}'
  case "$method" in
    trade.account_balance) echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":$(one_line serve_account_balance.json)}}" ;;
    trade.stock_positions) echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":$(one_line serve_stock_positions.json)}}" ;;
    trade.today_orders) echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":$(one_line serve_today_orders.json)}}" ;;
    trade.order_detail) echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":$(one_line serve_order_detail.json)}}" ;;
    trade.today_executions) echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":$(one_line serve_today_executions.json)}}" ;;
    trade.submit_order|trade.cancel_order)
      case "$line" in
        *'"execute":"123"'*)
          if [ "$method" = trade.submit_order ]; then
            case "$line" in
              *'"symbol":"REJECT.US"'*) echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{{\"code\":-32000,\"message\":\"openapi error: code=602012: order quantity invalid\"}}}}" ;;
              *) echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{{\"order_id\":\"9001\"}}}}" ;;
            esac
          else
            echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":null}}"
          fi ;;
        *'"execute"'*) echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{{\"code\":-32000,\"message\":\"Confirmation code does not match\"}}}}" ;;
        *) echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{{\"dry_run\":true,\"preview\":$params,\"confirmation_code\":\"123\"}}}}" ;;
      esac ;;
    shutdown) echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":null}}"; exit 0 ;;
    *) echo "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{{\"code\":-32601,\"message\":\"unknown method\"}}}}" ;;
  esac
done
"#,
        fixtures = fixtures.display(),
        log = dir.join("requests.log").display(),
        auth = auth,
    );
    let path = dir.join("longbridge");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hq-lb-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(unix)]
#[tokio::test]
async fn the_client_trades_through_serve_with_the_paper_guard() {
    let dir = temp_dir();
    let bin = fake_client(&dir, "auth_status_paper.json");
    let client = LongbridgeCli::new(
        LongbridgeConfig {
            bin,
            home: Some(dir.clone()),
        },
        AccountMode::Paper,
    );

    let status = client.status().await;
    assert_eq!(status.client.as_deref(), Some("longbridge 0.30.0"));
    assert!(status.signed_in);
    assert_eq!(status.identity.unwrap().account, "paper:3162001");

    let snapshot = client.snapshot().await.unwrap();
    assert_eq!(snapshot.identity.channel, "lb_papertrading");
    assert_eq!(snapshot.cash_in("USD").unwrap().total, dec!(96712.50));
    assert_eq!(snapshot.holdings.len(), 3);

    let orders = client.today_orders().await.unwrap();
    assert_eq!(orders.len(), 3);
    let executions = client.today_executions().await.unwrap();
    assert_eq!(executions.len(), 3);
    let detail = client.order("1069345312341434368").await.unwrap();
    assert!(detail.charges.is_some());

    let id = client.submit(&request()).await.unwrap();
    assert_eq!(id, "9001");
    client.cancel("1069345399999999999").await.unwrap();

    // Refused by the broker: reported as a refusal, the session carries on.
    let mut refused = request();
    refused.broker_symbol = "REJECT.US".into();
    assert!(matches!(
        client.submit(&refused).await,
        Err(BrokerError::Refused(_))
    ));
    assert_eq!(client.today_orders().await.unwrap().len(), 3);

    // Fractional quantities never leave hone-quant.
    let mut fractional = request();
    fractional.qty = dec!(1.5);
    assert!(client.submit(&fractional).await.is_err());

    let log = std::fs::read_to_string(dir.join("requests.log")).unwrap();
    let submits: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("trade.submit_order"))
        .collect();
    assert_eq!(
        submits.len(),
        4,
        "a dry run and an execution per order:\n{log}"
    );
    assert!(!submits[0].contains("execute"), "the dry run comes first");
    assert!(submits[1].contains(r#""execute":"123""#));
    for expected in [
        r#""symbol":"NVDA.US""#,
        r#""side":"buy""#,
        r#""order_type":"LO""#,
        r#""quantity":"20""#,
        r#""price":"121.37""#,
        r#""time_in_force":"day""#,
        r#""outside_rth":"RTH_ONLY""#,
        r#""remark":"hq-41""#,
    ] {
        assert!(
            submits[1].contains(expected),
            "{expected} in {}",
            submits[1]
        );
    }
    let cancels: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("trade.cancel_order"))
        .collect();
    assert_eq!(cancels.len(), 2);
    assert!(cancels[1].contains(r#""execute":"123""#));
    std::fs::remove_dir_all(dir).ok();
}

#[cfg(unix)]
#[tokio::test]
async fn sign_in_problems_are_reported_as_such() {
    let dir = temp_dir();
    let bin = fake_client(&dir, "auth_status_signed_out.json");
    let client = LongbridgeCli::new(
        LongbridgeConfig {
            bin: bin.clone(),
            home: Some(dir.clone()),
        },
        AccountMode::Paper,
    );
    let status = client.status().await;
    assert!(!status.signed_in);
    assert!(status.error.unwrap().contains("auth login"));
    assert!(matches!(
        client.snapshot().await,
        Err(BrokerError::NotSignedIn(_))
    ));

    // A live account behind a paper client.
    let live = LongbridgeCli::new(
        LongbridgeConfig {
            bin: fake_client(&dir, "auth_status_live.json"),
            home: Some(dir.clone()),
        },
        AccountMode::Paper,
    );
    assert!(matches!(
        live.snapshot().await,
        Err(BrokerError::WrongAccount(_))
    ));

    // A live client never sets the paper guard; the stand-in exits like a client whose
    // session cannot start, and the reason is on its standard error.
    let unguarded = LongbridgeCli::new(
        LongbridgeConfig {
            bin,
            home: Some(dir.clone()),
        },
        AccountMode::Live,
    );
    assert!(matches!(
        unguarded.today_orders().await,
        Err(BrokerError::NotSignedIn(_))
    ));

    // No client installed.
    let missing = LongbridgeCli::new(
        LongbridgeConfig {
            bin: dir.join("no-such-longbridge"),
            home: None,
        },
        AccountMode::Paper,
    );
    let status = missing.status().await;
    assert!(status.client.is_none());
    assert!(status.error.unwrap().contains("not installed"));
    assert!(matches!(
        missing.today_orders().await,
        Err(BrokerError::Unavailable(_))
    ));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn quantities_are_whole_numbers_on_the_wire() {
    assert_eq!(Decimal::from(20).trunc().normalize().to_string(), "20");
    assert_eq!(dec!(20.000000).trunc().normalize().to_string(), "20");
    assert_eq!(dec!(121.370).normalize().to_string(), "121.37");
}
