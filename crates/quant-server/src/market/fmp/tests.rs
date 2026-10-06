//! End-to-end tests against an in-process mock FMP server (axum on `127.0.0.1:0`) that serves the
//! fixtures in `tests/fixtures/fmp/`. The mock records every request, so each test asserts both
//! the parsed result and the exact traffic: which endpoints, with which key, how many times.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use chrono::DateTime;
use serde_json::json;
use tokio::time::Instant;

use super::*;

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/fmp/",
            $name
        ))
    };
}

const KEY_A: &str = "test-key-alpha-0001";
const KEY_B: &str = "test-key-bravo-0002";
const KEY_C: &str = "test-key-charlie-0003";

// Provider messages as FMP words them.
const LIMIT_REACHED: &str = r#"{"Error Message":"Limit Reach . Please upgrade your plan or visit our documentation for more details at https://site.financialmodelingprep.com/developer/docs/pricing "}"#;
const INVALID_KEY: &str = r#"{"Error Message":"Invalid API KEY. Feel free to create a Free API Key or visit https://site.financialmodelingprep.com/faqs?search=why-is-my-api-key-invalid for more information."}"#;
const PREMIUM_SYMBOL: &str = "Premium Query Parameter: 'Special Endpoint : This value set for 'symbol' is not available under your current subscription please visit our subscription page to upgrade your plan at https://site.financialmodelingprep.com/developer/docs/pricing'";
const EXCLUSIVE_ENDPOINT: &str = r#"{"Error Message":"Exclusive Endpoint : This endpoint is not available under your current subscription agreement, please visit our subscription page to upgrade your plan or contact us at https://site.financialmodelingprep.com/developer/docs/pricing"}"#;
const LEGACY_ENDPOINT: &str = r#"{"Error Message":"Legacy Endpoint : Due to Legacy endpoints being no longer supported - This endpoint is only available for legacy users who have valid subscriptions prior August 31, 2025. Please visit our subscription page to upgrade your plan or contact us at https://site.financialmodelingprep.com/developer/docs/pricing"}"#;

// ---------------------------------------------------------------------------------------------
// Mock server
// ---------------------------------------------------------------------------------------------

/// One request as the mock saw it; path and query values are percent-decoded.
#[derive(Debug, Clone)]
struct Seen {
    path: String,
    raw_query: String,
    query: Vec<(String, String)>,
    at: Instant,
}

impl Seen {
    fn new(uri: &Uri) -> Self {
        let raw_query = uri.query().unwrap_or_default().to_string();
        let query = raw_query
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                (decode(name), decode(value))
            })
            .collect();
        Self {
            path: decode(uri.path()),
            raw_query,
            query,
            at: Instant::now(),
        }
    }

    fn param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn key(&self) -> &str {
        self.param("apikey").unwrap_or_default()
    }

    /// Path and query without the key, e.g. `/stable/quote?symbol=AAPL`.
    fn target(&self) -> String {
        let query: Vec<String> = self
            .query
            .iter()
            .filter(|(name, _)| name != "apikey")
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        if query.is_empty() {
            self.path.clone()
        } else {
            format!("{}?{}", self.path, query.join("&"))
        }
    }
}

fn decode(text: &str) -> String {
    urlencoding::decode(text)
        .map(|decoded| decoded.into_owned())
        .unwrap_or_else(|_| text.to_string())
}

/// What the mock answers.
struct Reply {
    status: u16,
    body: String,
    delay: Duration,
}

impl Reply {
    fn after(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

fn ok(body: impl Into<String>) -> Reply {
    fail(200, body)
}

fn fail(status: u16, body: impl Into<String>) -> Reply {
    Reply {
        status,
        body: body.into(),
        delay: Duration::ZERO,
    }
}

type Handler = dyn Fn(&Seen) -> Reply + Send + Sync;

struct MockState {
    handler: Box<Handler>,
    seen: Mutex<Vec<Seen>>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}

/// Decrements the in-flight count even when the client hangs up mid-request.
struct InFlight<'a>(&'a AtomicUsize);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn respond(State(state): State<Arc<MockState>>, uri: Uri) -> Response {
    let seen = Seen::new(&uri);
    state.seen.lock().unwrap().push(seen.clone());
    let reply = (state.handler)(&seen);
    let now = state.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    let _in_flight = InFlight(&state.in_flight);
    state.max_in_flight.fetch_max(now, Ordering::SeqCst);
    if !reply.delay.is_zero() {
        tokio::time::sleep(reply.delay).await;
    }
    let status = StatusCode::from_u16(reply.status).expect("valid status");
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        reply.body,
    )
        .into_response()
}

/// An in-process FMP stand-in that records every request.
struct MockFmp {
    url: String,
    state: Arc<MockState>,
}

impl MockFmp {
    async fn start(handler: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> Self {
        let state = Arc::new(MockState {
            handler: Box::new(handler),
            seen: Mutex::default(),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
        });
        let app = Router::new()
            .fallback(respond)
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the mock FMP server");
        let url = format!("http://{}", listener.local_addr().expect("mock address"));
        tokio::spawn(async move { axum::serve(listener, app).await.expect("mock FMP server") });
        Self { url, state }
    }

    fn seen(&self) -> Vec<Seen> {
        self.state.seen.lock().unwrap().clone()
    }

    fn targets(&self) -> Vec<String> {
        self.seen().iter().map(Seen::target).collect()
    }

    fn keys(&self) -> Vec<String> {
        self.seen()
            .iter()
            .map(|seen| seen.key().to_string())
            .collect()
    }

    fn max_in_flight(&self) -> usize {
        self.state.max_in_flight.load(Ordering::SeqCst)
    }

    /// Test settings: one key, no request spacing.
    fn config(&self, mode: ApiMode) -> FmpConfig {
        FmpConfig {
            api_keys: vec![KEY_A.to_string()],
            base_url: self.url.clone(),
            timeout_secs: 5,
            api_mode: mode,
            max_concurrency: 4,
            requests_per_minute: 0,
        }
    }

    fn client(&self, mode: ApiMode) -> FmpClient {
        FmpClient::new(self.config(mode)).expect("valid config")
    }

    fn client_with_keys(&self, mode: ApiMode, keys: &[&str]) -> FmpClient {
        FmpClient::new(FmpConfig {
            api_keys: keys.iter().map(|key| key.to_string()).collect(),
            ..self.config(mode)
        })
        .expect("valid config")
    }
}

/// Answers a quote request with one quote per requested symbol, priced 100.
fn echo_quotes(seen: &Seen) -> Reply {
    let list = seen
        .param("symbols")
        .or(seen.param("symbol"))
        .or(seen.path.strip_prefix("/api/v3/quote/"))
        .unwrap_or_default();
    let rows: Vec<Value> = list
        .split(',')
        .filter(|symbol| !symbol.is_empty())
        .map(|symbol| json!({"symbol": symbol, "price": 100.0, "previousClose": 99.0}))
        .collect();
    ok(Value::Array(rows).to_string())
}

/// Daily rows on the first and last day of the requested range (newest first, like FMP).
fn range_bars(seen: &Seen) -> Reply {
    let (from, to) = (
        seen.param("from").unwrap_or_default(),
        seen.param("to").unwrap_or_default(),
    );
    ok(json!([
        {"date": to, "open": 2.0, "high": 2.0, "low": 2.0, "close": 2.0, "volume": 10},
        {"date": from, "open": 1.0, "high": 1.0, "low": 1.0, "close": 1.0, "volume": 10},
    ])
    .to_string())
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn symbols(list: &[&str]) -> Vec<String> {
    list.iter().map(|symbol| symbol.to_string()).collect()
}

#[track_caller]
fn assert_close(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("a value");
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

// ---------------------------------------------------------------------------------------------
// Quotes
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn stable_batch_quotes_carry_fractional_changes() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/stable/batch-quote" => ok(fixture!("stable_batch_quote.json")),
        _ => fail(404, ""),
    })
    .await;
    let client = mock.client(ApiMode::Stable);
    assert_eq!(client.source(), DataSource::Fmp);

    let quotes = client
        .quotes(&symbols(&["AAPL", "msft", "DELIST"]))
        .await
        .unwrap();

    assert_eq!(
        mock.targets(),
        vec!["/stable/batch-quote?symbols=AAPL,MSFT,DELIST"]
    );
    assert_eq!(mock.keys(), vec![KEY_A]);
    let returned: Vec<&str> = quotes.iter().map(|quote| quote.symbol.as_str()).collect();
    assert_eq!(returned, vec!["AAPL", "MSFT"], "DELIST has no price");

    let aapl = &quotes[0];
    assert_eq!(aapl.price, 258.02);
    // Computed from the prices, not from the rounded `changePercentage: 0.65`.
    assert_close(aapl.change_pct, (258.02 - 256.35) / 256.35);
    assert_eq!(aapl.change, Some(1.67));
    assert_eq!(aapl.prev_close, Some(256.35));
    assert_eq!(
        (aapl.open, aapl.day_high, aapl.day_low),
        (Some(254.665), Some(259.24), Some(253.95))
    );
    assert_eq!(aapl.volume, Some(49_155_614.0));
    assert_eq!(aapl.avg_volume, None);
    assert_eq!(aapl.market_cap, Some(3_829_041_510_200.0));
    assert_eq!(aapl.timestamp, Some(utc("2025-10-03T20:00:01Z")));

    // Without a previous close the percent field is used, as a fraction.
    let msft = &quotes[1];
    assert_eq!(msft.prev_close, None);
    assert_close(msft.change_pct, 0.0031238);
}

#[tokio::test]
async fn a_single_symbol_uses_the_single_quote_endpoint() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/stable/quote" => ok(r#"[{"symbol":"AAPL","price":258.02,"previousClose":256.35}]"#),
        _ => fail(404, ""),
    })
    .await;
    let quotes = mock
        .client(ApiMode::Stable)
        .quotes(&symbols(&["aapl"]))
        .await
        .unwrap();
    assert_eq!(mock.targets(), vec!["/stable/quote?symbol=AAPL"]);
    assert_eq!(quotes.len(), 1);
}

#[tokio::test]
async fn legacy_quotes_use_the_legacy_field_names() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/api/v3/quote/AAPL,MSFT" => ok(fixture!("legacy_quote.json")),
        _ => fail(404, ""),
    })
    .await;
    let quotes = mock
        .client(ApiMode::Legacy)
        .quotes(&symbols(&["AAPL", "MSFT"]))
        .await
        .unwrap();
    assert_eq!(mock.targets(), vec!["/api/v3/quote/AAPL,MSFT"]);

    let (aapl, msft) = (&quotes[0], &quotes[1]);
    assert_close(aapl.change_pct, (258.02 - 256.35) / 256.35);
    assert_eq!(aapl.avg_volume, Some(54_987_245.0));
    // `previousClose: 0` is not a price: `changesPercentage` / 100 stands in.
    assert_eq!(msft.prev_close, None);
    assert_close(msft.change_pct, 0.0031238);
    assert_eq!(msft.avg_volume, Some(20_345_118.0));
}

#[tokio::test]
async fn quotes_are_requested_in_batches_of_at_most_25() {
    let mock = MockFmp::start(echo_quotes).await;
    let wanted: Vec<String> = (0..60).map(|i| format!("S{i:02}")).collect();

    let quotes = mock.client(ApiMode::Stable).quotes(&wanted).await.unwrap();

    let mut sizes: Vec<usize> = mock
        .seen()
        .iter()
        .map(|seen| seen.param("symbols").unwrap().split(',').count())
        .collect();
    sizes.sort_unstable();
    assert_eq!(sizes, vec![10, 25, 25]);
    let returned: Vec<String> = quotes.into_iter().map(|quote| quote.symbol).collect();
    assert_eq!(returned, wanted, "every symbol once, in request order");
}

#[tokio::test]
async fn a_refused_batch_falls_back_to_single_quotes() {
    // The plan has no batch endpoint, and one symbol is premium.
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/stable/batch-quote" => fail(402, EXCLUSIVE_ENDPOINT),
        "/stable/quote" if seen.param("symbol") == Some("PREM") => fail(402, PREMIUM_SYMBOL),
        "/stable/quote" => {
            let symbol = seen.param("symbol").unwrap_or_default();
            ok(json!([{"symbol": symbol, "price": 100.0, "timestamp": 1_759_680_000}]).to_string())
        }
        _ => fail(404, "{}"),
    })
    .await;
    let wanted = symbols(&["AAA", "PREM", "BBB"]);

    let quotes = mock.client(ApiMode::Stable).quotes(&wanted).await.unwrap();

    let returned: Vec<&str> = quotes.iter().map(|quote| quote.symbol.as_str()).collect();
    assert_eq!(returned, vec!["AAA", "BBB"]);
    let mut targets = mock.targets();
    targets.sort();
    assert_eq!(
        targets,
        vec![
            "/stable/batch-quote?symbols=AAA,PREM,BBB",
            "/stable/quote?symbol=AAA",
            "/stable/quote?symbol=BBB",
            "/stable/quote?symbol=PREM",
        ]
    );
}

#[tokio::test]
async fn invalid_symbols_never_reach_a_url() {
    let mock = MockFmp::start(echo_quotes).await;
    let client = mock.client(ApiMode::Stable);

    let quotes = client
        .quotes(&symbols(&[
            "aapl",
            "BRK B",
            "A&apikey=x",
            "",
            "^gspc",
            "AAPL",
            "brk-b",
            "../etc",
        ]))
        .await
        .unwrap();
    assert_eq!(
        mock.targets(),
        vec!["/stable/batch-quote?symbols=AAPL,^GSPC,BRK-B"]
    );
    assert_eq!(
        mock.seen()[0].raw_query,
        format!("symbols=AAPL,%5EGSPC,BRK-B&apikey={KEY_A}")
    );
    let returned: Vec<&str> = quotes.iter().map(|quote| quote.symbol.as_str()).collect();
    assert_eq!(returned, vec!["AAPL", "^GSPC", "BRK-B"]);

    assert!(
        client
            .quotes(&symbols(&["", "A B"]))
            .await
            .unwrap()
            .is_empty()
    );
    for result in [
        client
            .daily_bars("A/B", d(2025, 1, 2), d(2025, 1, 3))
            .await
            .map(|_| ()),
        client.splits("AAPL?x=1").await.map(|_| ()),
    ] {
        assert!(
            matches!(&result, Err(MarketError::Http(message)) if message.contains("invalid symbol")),
            "{result:?}"
        );
    }
    assert_eq!(mock.seen().len(), 1, "nothing else was requested");
}

// ---------------------------------------------------------------------------------------------
// Daily bars
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn stable_daily_bars_merge_dividend_adjusted_prices() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/stable/historical-price-eod/full" => ok(fixture!("stable_eod_full.json")),
        "/stable/historical-price-eod/dividend-adjusted" => {
            ok(fixture!("stable_eod_dividend_adjusted.json"))
        }
        _ => fail(404, ""),
    })
    .await;

    let bars = mock
        .client(ApiMode::Stable)
        .daily_bars("aapl", d(2025, 9, 29), d(2025, 10, 3))
        .await
        .unwrap();

    assert_eq!(
        mock.targets(),
        vec![
            "/stable/historical-price-eod/full?symbol=AAPL&from=2025-09-29&to=2025-10-03",
            "/stable/historical-price-eod/dividend-adjusted?symbol=AAPL&from=2025-09-29&to=2025-10-03",
        ]
    );
    // Oldest first; the duplicate 2025-10-01 row and the out-of-range 2025-09-26 row are gone.
    let dates: Vec<NaiveDate> = bars.iter().map(|bar| bar.date).collect();
    assert_eq!(
        dates,
        vec![
            d(2025, 9, 29),
            d(2025, 9, 30),
            d(2025, 10, 1),
            d(2025, 10, 2),
            d(2025, 10, 3)
        ]
    );
    assert!(bars.iter().all(|bar| bar.symbol == "AAPL"));
    let oct1 = &bars[2];
    assert_eq!(
        (oct1.open, oct1.high, oct1.low, oct1.close, oct1.volume),
        (255.04, 258.79, 254.93, 255.45, 48_713_940.0)
    );
    assert_eq!(
        (oct1.adj_open, oct1.adj_close),
        (Some(254.03), Some(254.4384))
    );
    // The adjusted series has no row for 2025-09-29.
    assert_eq!((bars[0].adj_open, bars[0].adj_close), (None, None));
}

#[tokio::test]
async fn daily_bars_survive_a_plan_without_dividend_adjusted_prices() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/stable/historical-price-eod/full" => ok(fixture!("stable_eod_full.json")),
        "/stable/historical-price-eod/dividend-adjusted" => fail(402, PREMIUM_SYMBOL),
        _ => fail(404, ""),
    })
    .await;
    let client = mock.client(ApiMode::Stable);

    for _ in 0..2 {
        let bars = client
            .daily_bars("AAPL", d(2025, 9, 29), d(2025, 10, 3))
            .await
            .unwrap();
        assert_eq!(bars.len(), 5);
        assert!(
            bars.iter()
                .all(|bar| bar.adj_open.is_none() && bar.adj_close.is_none())
        );
    }
    assert_eq!(mock.seen().len(), 4);
}

#[tokio::test]
async fn legacy_daily_bars_take_adj_close_from_the_same_rows() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/api/v3/historical-price-full/AAPL" => ok(fixture!("legacy_historical_price_full.json")),
        _ => fail(404, ""),
    })
    .await;

    let bars = mock
        .client(ApiMode::Legacy)
        .daily_bars("AAPL", d(2020, 8, 27), d(2020, 8, 31))
        .await
        .unwrap();

    assert_eq!(
        mock.targets(),
        vec!["/api/v3/historical-price-full/AAPL?from=2020-08-27&to=2020-08-31"],
        "no separate adjusted request"
    );
    let dates: Vec<NaiveDate> = bars.iter().map(|bar| bar.date).collect();
    assert_eq!(dates, vec![d(2020, 8, 27), d(2020, 8, 28), d(2020, 8, 31)]);
    let last = &bars[2];
    assert_eq!(
        (last.open, last.close, last.volume),
        (127.58, 129.04, 225_702_700.0)
    );
    assert_eq!(last.adj_close, Some(126.52));
    assert_close(last.adj_open, 127.58 * 126.52 / 129.04);
}

#[tokio::test]
async fn auto_mode_takes_adjusted_closes_from_legacy_when_stable_lacks_them() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/stable/historical-price-eod/full" => ok(fixture!("stable_eod_full.json")),
        "/stable/historical-price-eod/dividend-adjusted" => fail(402, PREMIUM_SYMBOL),
        "/api/v3/historical-price-full/AAPL" => ok(json!({"symbol": "AAPL", "historical": [
            {"date": "2025-10-03", "open": 254.665, "high": 259.24, "low": 253.95, "close": 258.02,
             "adjClose": 256.9982, "volume": 49_155_614}
        ]})
        .to_string()),
        _ => fail(404, ""),
    })
    .await;
    let client = mock.client(ApiMode::Auto);
    let (from, to) = (d(2025, 9, 29), d(2025, 10, 3));

    let bars = client.daily_bars("AAPL", from, to).await.unwrap();
    let friday = bars.last().unwrap();
    assert_eq!(friday.adj_close, Some(256.9982));
    assert_close(friday.adj_open, 254.665 * 256.9982 / 258.02);
    assert_eq!(bars[0].adj_close, None);

    client.daily_bars("AAPL", from, to).await.unwrap();
    let paths: Vec<String> = mock.seen().into_iter().map(|seen| seen.path).collect();
    assert_eq!(
        paths,
        vec![
            "/stable/historical-price-eod/full",
            "/stable/historical-price-eod/dividend-adjusted",
            "/api/v3/historical-price-full/AAPL",
            // Remembered: bars from stable, adjusted closes straight from legacy.
            "/stable/historical-price-eod/full",
            "/api/v3/historical-price-full/AAPL",
        ]
    );
}

#[tokio::test]
async fn long_daily_ranges_are_fetched_in_four_year_chunks() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/stable/historical-price-eod/full" => range_bars(seen),
        "/stable/historical-price-eod/dividend-adjusted" => {
            let (from, to) = (seen.param("from").unwrap(), seen.param("to").unwrap());
            ok(json!([
                {"date": to, "adjOpen": 1.9, "adjClose": 1.9},
                {"date": from, "adjOpen": 0.9, "adjClose": 0.9},
            ])
            .to_string())
        }
        _ => fail(404, ""),
    })
    .await;

    let bars = mock
        .client(ApiMode::Stable)
        .daily_bars("SPY", d(2010, 1, 1), d(2021, 6, 30))
        .await
        .unwrap();

    let chunks = [
        "from=2010-01-01&to=2013-12-31",
        "from=2014-01-01&to=2017-12-31",
        "from=2018-01-01&to=2021-06-30",
    ];
    let expected: Vec<String> = ["full", "dividend-adjusted"]
        .iter()
        .flat_map(|kind| {
            chunks
                .iter()
                .map(move |chunk| format!("/stable/historical-price-eod/{kind}?symbol=SPY&{chunk}"))
        })
        .collect();
    assert_eq!(mock.targets(), expected);
    let dates: Vec<NaiveDate> = bars.iter().map(|bar| bar.date).collect();
    assert_eq!(
        dates,
        vec![
            d(2010, 1, 1),
            d(2013, 12, 31),
            d(2014, 1, 1),
            d(2017, 12, 31),
            d(2018, 1, 1),
            d(2021, 6, 30)
        ]
    );
    let adjusted: Vec<Option<f64>> = bars.iter().map(|bar| bar.adj_close).collect();
    assert_eq!(adjusted, [Some(0.9), Some(1.9)].repeat(3));
}

// ---------------------------------------------------------------------------------------------
// Intraday bars
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn intraday_bars_are_regular_session_utc_across_a_dst_change() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/stable/historical-chart/5min" => ok(fixture!("stable_intraday_5min.json")),
        _ => fail(404, ""),
    })
    .await;

    let bars = mock
        .client(ApiMode::Stable)
        .intraday_bars("AAPL", Interval::FiveMin, d(2025, 10, 31), d(2025, 11, 3))
        .await
        .unwrap();

    assert_eq!(
        mock.targets(),
        vec!["/stable/historical-chart/5min?symbol=AAPL&from=2025-10-31&to=2025-11-03"]
    );
    // Friday 2025-10-31 is EDT (UTC-4); DST ended on 2025-11-02, so Monday is EST (UTC-5).
    // Pre-market, after-hours, the 16:00 print and the duplicate 09:30 row are dropped.
    let stamps: Vec<DateTime<Utc>> = bars.iter().map(|bar| bar.ts).collect();
    assert_eq!(
        stamps,
        vec![
            utc("2025-10-31T13:30:00Z"),
            utc("2025-10-31T13:35:00Z"),
            utc("2025-10-31T19:55:00Z"),
            utc("2025-11-03T14:30:00Z"),
            utc("2025-11-03T20:55:00Z"),
        ]
    );
    let open = &bars[0];
    assert_eq!(
        (open.open, open.high, open.low, open.close, open.volume),
        (276.99, 277.32, 275.9, 276.1, 2_754_897.0)
    );
}

#[tokio::test]
async fn legacy_intraday_puts_interval_and_symbol_in_the_path() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/api/v3/historical-chart/1hour/AAPL" => ok(fixture!("legacy_intraday_1hour.json")),
        _ => fail(404, ""),
    })
    .await;

    let bars = mock
        .client(ApiMode::Legacy)
        .intraday_bars("AAPL", Interval::OneHour, d(2025, 10, 3), d(2025, 10, 3))
        .await
        .unwrap();

    assert_eq!(
        mock.targets(),
        vec!["/api/v3/historical-chart/1hour/AAPL?from=2025-10-03&to=2025-10-03"]
    );
    let stamps: Vec<DateTime<Utc>> = bars.iter().map(|bar| bar.ts).collect();
    assert_eq!(
        stamps,
        vec![utc("2025-10-03T13:30:00Z"), utc("2025-10-03T19:30:00Z")]
    );
}

// ---------------------------------------------------------------------------------------------
// Splits and dividends
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn splits_from_both_generations_come_oldest_first() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/stable/splits" => ok(fixture!("stable_splits.json")),
        "/api/v3/historical-price-full/stock_split/AAPL" => ok(fixture!("legacy_splits.json")),
        _ => fail(404, ""),
    })
    .await;

    for mode in [ApiMode::Stable, ApiMode::Legacy] {
        let splits = mock.client(mode).splits("aapl").await.unwrap();
        let summary: Vec<(NaiveDate, f64)> = splits
            .iter()
            .map(|split| (split.date, split.ratio()))
            .collect();
        assert_eq!(
            summary,
            vec![
                (d(1987, 6, 16), 2.0),
                (d(2000, 6, 21), 2.0),
                (d(2005, 2, 28), 2.0),
                (d(2014, 6, 9), 7.0),
                (d(2020, 8, 31), 4.0),
            ],
            "{mode:?}"
        );
        assert!(splits.iter().all(|split| split.symbol == "AAPL"));
    }
    assert_eq!(
        mock.targets(),
        vec![
            "/stable/splits?symbol=AAPL",
            "/api/v3/historical-price-full/stock_split/AAPL"
        ]
    );
}

#[tokio::test]
async fn dividends_from_both_generations_come_oldest_first() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/stable/dividends" => ok(fixture!("stable_dividends.json")),
        "/api/v3/historical-price-full/stock_dividend/AAPL" => {
            ok(fixture!("legacy_dividends.json"))
        }
        _ => fail(404, ""),
    })
    .await;
    let summary = |dividends: Vec<Dividend>| -> Vec<(NaiveDate, f64, Option<NaiveDate>)> {
        dividends
            .into_iter()
            .map(|dividend| (dividend.ex_date, dividend.amount, dividend.pay_date))
            .collect()
    };

    let stable = mock
        .client(ApiMode::Stable)
        .dividends("AAPL")
        .await
        .unwrap();
    assert_eq!(
        summary(stable),
        vec![
            // `dividend` is null, so `adjDividend` stands in; the empty payment date is None.
            (d(1995, 11, 21), 0.00107, None),
            (d(2025, 2, 10), 0.25, Some(d(2025, 2, 13))),
            (d(2025, 5, 12), 0.26, Some(d(2025, 5, 15))),
            (d(2025, 8, 11), 0.26, Some(d(2025, 8, 14))),
        ]
    );
    let legacy = mock
        .client(ApiMode::Legacy)
        .dividends("AAPL")
        .await
        .unwrap();
    assert_eq!(
        summary(legacy),
        vec![
            // The declared amount, not the split-adjusted one.
            (d(2012, 8, 9), 2.65, Some(d(2012, 8, 16))),
            (d(2025, 5, 12), 0.26, Some(d(2025, 5, 15))),
            (d(2025, 8, 11), 0.26, Some(d(2025, 8, 14))),
        ]
    );
    assert_eq!(
        mock.targets(),
        vec![
            "/stable/dividends?symbol=AAPL",
            "/api/v3/historical-price-full/stock_dividend/AAPL"
        ]
    );
}

#[tokio::test]
async fn empty_responses_are_empty_results_not_errors() {
    let mock = MockFmp::start(|seen| {
        if seen.path.starts_with("/stable/") {
            ok("[]")
        } else {
            ok("{}")
        }
    })
    .await;
    let (from, to) = (d(2025, 10, 1), d(2025, 10, 3));

    for mode in [ApiMode::Stable, ApiMode::Legacy] {
        let client = mock.client(mode);
        let quotes = client.quotes(&symbols(&["AAPL", "MSFT"])).await.unwrap();
        assert!(quotes.is_empty());
        assert!(
            client
                .daily_bars("AAPL", from, to)
                .await
                .unwrap()
                .is_empty()
        );
        let intraday = client
            .intraday_bars("AAPL", Interval::FifteenMin, from, to)
            .await
            .unwrap();
        assert!(intraday.is_empty());
        assert!(client.splits("AAPL").await.unwrap().is_empty());
        assert!(client.dividends("AAPL").await.unwrap().is_empty());
        // An inverted range needs no request at all.
        assert!(
            client
                .daily_bars("AAPL", to, from)
                .await
                .unwrap()
                .is_empty()
        );
    }
    // Five requests per mode: no stable bars means no dividend-adjusted request either.
    assert_eq!(mock.seen().len(), 10);
}

// ---------------------------------------------------------------------------------------------
// Key pool
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn rejected_keys_fail_over_in_order_and_the_working_key_sticks() {
    let mock = MockFmp::start(|seen| match seen.key() {
        KEY_A => fail(429, LIMIT_REACHED),
        KEY_B => fail(401, INVALID_KEY),
        _ => ok(fixture!("stable_splits.json")),
    })
    .await;
    let client = mock.client_with_keys(ApiMode::Stable, &[KEY_A, KEY_B, KEY_C]);

    assert_eq!(client.splits("AAPL").await.unwrap().len(), 5);
    assert_eq!(mock.keys(), vec![KEY_A, KEY_B, KEY_C]);

    client.splits("AAPL").await.unwrap();
    assert_eq!(mock.keys()[3..], [KEY_C], "the key that worked goes first");
}

#[tokio::test]
async fn limit_messages_inside_http_200_rotate_keys() {
    let mock = MockFmp::start(|seen| match seen.key() {
        KEY_A => ok(LIMIT_REACHED),
        _ => ok(fixture!("stable_splits.json")),
    })
    .await;
    let client = mock.client_with_keys(ApiMode::Stable, &[KEY_A, KEY_B]);

    assert_eq!(client.splits("AAPL").await.unwrap().len(), 5);
    assert_eq!(mock.keys(), vec![KEY_A, KEY_B]);
}

#[tokio::test]
async fn exhausting_every_key_is_an_auth_error_that_names_no_key() {
    // The body echoes the key back, as some gateways do.
    let mock = MockFmp::start(|seen| {
        fail(
            403,
            format!("Forbidden: key {} is not authorised", seen.key()),
        )
    })
    .await;
    let client = mock.client_with_keys(ApiMode::Stable, &[KEY_A, KEY_B]);

    let error = client.splits("AAPL").await.unwrap_err();

    let message = error.to_string();
    assert!(matches!(error, MarketError::Auth(_)), "{message}");
    assert!(
        message.contains("key #1") && message.contains("key #2"),
        "{message}"
    );
    assert!(
        !message.contains(KEY_A) && !message.contains(KEY_B),
        "{message}"
    );
    assert_eq!(mock.keys(), vec![KEY_A, KEY_B]);
}

#[tokio::test]
async fn payment_required_is_plan_restricted_and_never_rotates_keys() {
    let mock = MockFmp::start(|_| fail(402, PREMIUM_SYMBOL)).await;
    let client = mock.client_with_keys(ApiMode::Stable, &[KEY_A, KEY_B]);

    let error = client.splits("AAPL").await.unwrap_err();

    assert!(
        matches!(&error, MarketError::PlanRestricted(message)
            if message.contains("HTTP 402") && message.contains("Premium Query Parameter")),
        "{error}"
    );
    assert_eq!(mock.keys(), vec![KEY_A]);
}

#[tokio::test]
async fn other_provider_errors_fail_without_rotating_keys() {
    let mock =
        MockFmp::start(|_| ok(r#"{"Error Message":"temporary upstream calculation failure"}"#))
            .await;
    let client = mock.client_with_keys(ApiMode::Stable, &[KEY_A, KEY_B]);

    let error = client.dividends("AAPL").await.unwrap_err();

    assert!(
        matches!(&error, MarketError::Http(message) if message.contains("temporary upstream")),
        "{error}"
    );
    assert_eq!(mock.keys(), vec![KEY_A]);
}

// ---------------------------------------------------------------------------------------------
// Auto mode
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn auto_mode_falls_back_to_legacy_on_402_and_404_and_remembers_it() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/stable/quote" => fail(402, PREMIUM_SYMBOL),
        "/api/v3/quote/AAPL" => echo_quotes(seen),
        "/stable/splits" => fail(404, r#"{"message":"Not Found"}"#),
        "/api/v3/historical-price-full/stock_split/AAPL" => ok(fixture!("legacy_splits.json")),
        _ => fail(404, ""),
    })
    .await;
    let client = mock.client(ApiMode::Auto);

    for _ in 0..2 {
        assert_eq!(client.quotes(&symbols(&["AAPL"])).await.unwrap().len(), 1);
        assert_eq!(client.splits("AAPL").await.unwrap().len(), 5);
    }

    assert_eq!(
        mock.targets(),
        vec![
            "/stable/quote?symbol=AAPL",
            "/api/v3/quote/AAPL",
            "/stable/splits?symbol=AAPL",
            "/api/v3/historical-price-full/stock_split/AAPL",
            // Second round: straight to the remembered legacy endpoints.
            "/api/v3/quote/AAPL",
            "/api/v3/historical-price-full/stock_split/AAPL",
        ]
    );
}

#[tokio::test]
async fn auto_mode_falls_back_on_a_plan_related_403_without_rotating_keys() {
    let mock = MockFmp::start(|seen| match seen.path.as_str() {
        "/stable/dividends" => fail(403, EXCLUSIVE_ENDPOINT),
        "/api/v3/historical-price-full/stock_dividend/AAPL" => {
            ok(fixture!("legacy_dividends.json"))
        }
        _ => fail(404, ""),
    })
    .await;
    let client = mock.client_with_keys(ApiMode::Auto, &[KEY_A, KEY_B]);

    assert_eq!(client.dividends("AAPL").await.unwrap().len(), 3);

    assert_eq!(
        mock.targets(),
        vec![
            "/stable/dividends?symbol=AAPL",
            "/api/v3/historical-price-full/stock_dividend/AAPL"
        ]
    );
    assert_eq!(mock.keys(), vec![KEY_A, KEY_A]);
}

#[tokio::test]
async fn auto_mode_goes_straight_to_stable_once_it_has_answered() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let mock = MockFmp::start(move |seen| match seen.path.as_str() {
        // The plan covers the first request but not the second (say, a symbol restriction).
        "/stable/splits" if counter.fetch_add(1, Ordering::SeqCst) == 0 => {
            ok(fixture!("stable_splits.json"))
        }
        "/stable/splits" => fail(402, PREMIUM_SYMBOL),
        _ => ok(fixture!("legacy_splits.json")),
    })
    .await;
    let client = mock.client(ApiMode::Auto);

    assert_eq!(client.splits("AAPL").await.unwrap().len(), 5);
    let error = client.splits("AAPL").await.unwrap_err();

    assert!(matches!(error, MarketError::PlanRestricted(_)), "{error}");
    assert_eq!(
        mock.targets(),
        vec!["/stable/splits?symbol=AAPL"; 2],
        "no legacy probe once stable is known to serve splits"
    );
}

#[tokio::test]
async fn auto_mode_keeps_probing_legacy_while_its_refusals_are_symbol_level() {
    // Both generations refuse the symbol: nothing answered, so nothing is remembered.
    let mock = MockFmp::start(|_| fail(402, PREMIUM_SYMBOL)).await;
    let client = mock.client(ApiMode::Auto);
    let (from, to) = (d(2025, 10, 1), d(2025, 10, 3));

    for _ in 0..2 {
        let error = client
            .intraday_bars("AAPL", Interval::FiveMin, from, to)
            .await
            .unwrap_err();
        let message = error.to_string();
        assert!(matches!(error, MarketError::PlanRestricted(_)), "{message}");
        assert!(
            message.contains("HTTP 402 for /stable/") && message.contains("legacy fallback"),
            "{message}"
        );
    }
    assert_eq!(mock.seen().len(), 4, "both generations probed every time");
}

#[tokio::test]
async fn auto_mode_stops_probing_legacy_once_the_account_has_no_legacy_access() {
    let mock = MockFmp::start(|seen| {
        if seen.path.starts_with("/api/v3/") {
            fail(403, LEGACY_ENDPOINT)
        } else {
            fail(402, PREMIUM_SYMBOL)
        }
    })
    .await;
    let client = mock.client(ApiMode::Auto);
    let (from, to) = (d(2025, 10, 1), d(2025, 10, 3));

    // The first failure reports the stable refusal and why the fallback failed too.
    let error = client
        .intraday_bars("AAPL", Interval::FiveMin, from, to)
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(matches!(error, MarketError::PlanRestricted(_)), "{message}");
    assert!(
        message.contains("HTTP 402")
            && message.contains("legacy fallback")
            && message.contains("Legacy Endpoint"),
        "{message}"
    );

    // From then on no family is re-tried on legacy.
    let error = client
        .intraday_bars("AAPL", Interval::FiveMin, from, to)
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("legacy fallback"), "{error}");
    assert!(client.splits("AAPL").await.is_err());
    assert_eq!(
        mock.targets(),
        vec![
            "/stable/historical-chart/5min?symbol=AAPL&from=2025-10-01&to=2025-10-03",
            "/api/v3/historical-chart/5min/AAPL?from=2025-10-01&to=2025-10-03",
            "/stable/historical-chart/5min?symbol=AAPL&from=2025-10-01&to=2025-10-03",
            "/stable/splits?symbol=AAPL",
        ]
    );
}

#[tokio::test]
async fn auto_mode_does_not_fall_back_on_key_failures() {
    let mock = MockFmp::start(|_| fail(401, INVALID_KEY)).await;
    let client = mock.client_with_keys(ApiMode::Auto, &[KEY_A, KEY_B]);

    assert!(matches!(
        client.splits("AAPL").await,
        Err(MarketError::Auth(_))
    ));
    assert_eq!(mock.targets(), vec!["/stable/splits?symbol=AAPL"; 2]);
}

// ---------------------------------------------------------------------------------------------
// Retries, transport failures and redaction
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_server_error_is_retried_once_on_the_same_key() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let mock = MockFmp::start(move |_| {
        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
            fail(500, "upstream hiccup")
        } else {
            ok(fixture!("stable_splits.json"))
        }
    })
    .await;
    let client = mock.client_with_keys(ApiMode::Stable, &[KEY_A, KEY_B]);
    let started = Instant::now();

    assert_eq!(client.splits("AAPL").await.unwrap().len(), 5);

    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "the retry waits about 300 ms"
    );
    assert_eq!(mock.keys(), vec![KEY_A, KEY_A]);
}

#[tokio::test]
async fn persistent_server_errors_fail_after_one_retry() {
    let mock = MockFmp::start(|_| fail(503, "<html>Service Unavailable</html>")).await;
    let client = mock.client_with_keys(ApiMode::Stable, &[KEY_A, KEY_B]);

    let error = client.splits("AAPL").await.unwrap_err();

    assert!(
        matches!(&error, MarketError::Http(message) if message.contains("HTTP 503")),
        "{error}"
    );
    assert_eq!(
        mock.keys(),
        vec![KEY_A, KEY_A],
        "server errors do not rotate keys"
    );
}

#[tokio::test]
async fn timeouts_are_retried_once() {
    let mock = MockFmp::start(|_| ok("[]").after(Duration::from_millis(1500))).await;
    let client = FmpClient::new(FmpConfig {
        timeout_secs: 1,
        ..mock.config(ApiMode::Stable)
    })
    .unwrap();

    let error = client.splits("AAPL").await.unwrap_err();

    assert!(matches!(error, MarketError::Http(_)), "{error}");
    assert_eq!(mock.seen().len(), 2);
}

#[tokio::test]
async fn connection_failures_are_retried_and_reported_without_the_key() {
    // Bind and release a port so that nothing listens on it.
    let address = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let client = FmpClient::new(FmpConfig {
        api_keys: vec![KEY_A.to_string()],
        base_url: format!("http://{address}/api/v3"),
        api_mode: ApiMode::Stable,
        requests_per_minute: 0,
        ..FmpConfig::default()
    })
    .unwrap();
    let started = Instant::now();

    let error = client.splits("AAPL").await.unwrap_err();

    let message = error.to_string();
    assert!(matches!(error, MarketError::Http(_)), "{message}");
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "retried once"
    );
    assert!(message.contains("/stable/splits?symbol=AAPL"), "{message}");
    assert!(
        !message.contains(KEY_A) && !message.contains("apikey"),
        "{message}"
    );
}

#[tokio::test]
async fn provider_text_echoing_the_url_is_redacted() {
    let mock = MockFmp::start(|seen| {
        fail(
            400,
            format!(
                "Bad request: https://financialmodelingprep.com{}?{}",
                seen.path, seen.raw_query
            ),
        )
    })
    .await;

    let error = mock
        .client(ApiMode::Stable)
        .splits("AAPL")
        .await
        .unwrap_err();

    let message = error.to_string();
    assert!(matches!(error, MarketError::Http(_)), "{message}");
    assert!(!message.contains(KEY_A), "{message}");
    assert!(message.contains("apikey=***"), "{message}");
}

// ---------------------------------------------------------------------------------------------
// Configuration and throttling
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn honeclaw_style_base_urls_reach_the_host_root() {
    for suffix in ["/api", "/api/v3/", "/"] {
        let mock = MockFmp::start(|_| ok(fixture!("stable_splits.json"))).await;
        let client = FmpClient::new(FmpConfig {
            base_url: format!("{}{suffix}", mock.url),
            ..mock.config(ApiMode::Stable)
        })
        .unwrap();
        assert_eq!(client.base_url(), mock.url);

        client.splits("AAPL").await.unwrap();

        assert_eq!(
            mock.targets(),
            vec!["/stable/splits?symbol=AAPL"],
            "{suffix}"
        );
    }
}

#[test]
fn configuration_is_validated_and_keys_stay_out_of_debug_output() {
    assert!(matches!(
        FmpClient::new(FmpConfig::default()),
        Err(MarketError::NotConfigured(_))
    ));
    let config = FmpConfig {
        api_keys: symbols(&[KEY_A, " ", KEY_B, KEY_A]),
        ..FmpConfig::default()
    };
    let debug = format!("{config:?}");
    assert!(!debug.contains(KEY_A) && !debug.contains(KEY_B), "{debug}");

    let client = FmpClient::new(config).unwrap();

    assert_eq!(client.key_count(), 2);
    assert_eq!(client.base_url(), "https://financialmodelingprep.com");
    let debug = format!("{client:?}");
    assert!(!debug.contains(KEY_A), "{debug}");
}

#[tokio::test]
async fn concurrency_is_capped_by_max_concurrency() {
    let mock = MockFmp::start(|seen| echo_quotes(seen).after(Duration::from_millis(150))).await;
    let client = FmpClient::new(FmpConfig {
        max_concurrency: 2,
        ..mock.config(ApiMode::Stable)
    })
    .unwrap();
    let wanted: Vec<String> = (0..150).map(|i| format!("C{i:03}")).collect();

    assert_eq!(client.quotes(&wanted).await.unwrap().len(), 150);

    assert_eq!(mock.seen().len(), 6);
    assert_eq!(mock.max_in_flight(), 2);
}

#[tokio::test]
async fn request_starts_are_spaced_by_the_rate_limit() {
    let mock = MockFmp::start(echo_quotes).await;
    let started = Instant::now();
    // 600 requests per minute: starts at least 100 ms apart.
    let client = FmpClient::new(FmpConfig {
        requests_per_minute: 600,
        ..mock.config(ApiMode::Stable)
    })
    .unwrap();
    let wanted: Vec<String> = (0..75).map(|i| format!("R{i:02}")).collect();

    client.quotes(&wanted).await.unwrap();

    // Three concurrent batches cannot all have started before two spacing intervals passed.
    assert!(started.elapsed() >= Duration::from_millis(200));
    let mut arrivals: Vec<Instant> = mock.seen().iter().map(|seen| seen.at).collect();
    arrivals.sort();
    assert_eq!(arrivals.len(), 3);
    for pair in arrivals.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(gap >= Duration::from_millis(50), "{gap:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// Diagnose
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn diagnose_probes_every_family_on_both_generations() {
    let mock = MockFmp::start(|seen| {
        if seen.path.starts_with("/api/v3/") {
            return fail(403, LEGACY_ENDPOINT);
        }
        match seen.path.as_str() {
            "/stable/quote" | "/stable/batch-quote" => echo_quotes(seen),
            "/stable/historical-price-eod/full" => range_bars(seen),
            "/stable/historical-price-eod/dividend-adjusted" => fail(402, PREMIUM_SYMBOL),
            "/stable/historical-chart/5min" => {
                let to = seen.param("to").unwrap_or_default();
                ok(json!([
                    {"date": format!("{to} 15:55:00"), "open": 1.0, "high": 1.0, "low": 1.0, "close": 1.0, "volume": 1},
                    {"date": format!("{to} 09:30:00"), "open": 1.0, "high": 1.0, "low": 1.0, "close": 1.0, "volume": 1},
                ])
                .to_string())
            }
            "/stable/splits" => ok(fixture!("stable_splits.json")),
            "/stable/dividends" => ok(fixture!("stable_dividends.json")),
            _ => fail(404, ""),
        }
    })
    .await;
    let client = mock.client_with_keys(ApiMode::Auto, &[KEY_A, KEY_B]);

    let checks = diagnose(&client, "aapl").await;

    let summary: Vec<(&str, &str, bool)> = checks
        .iter()
        .map(|check| (check.endpoint.as_str(), check.api.as_str(), check.ok))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("quote", "stable", true),
            ("quote", "legacy", false),
            ("batch quote", "stable", true),
            ("daily", "stable", true),
            ("daily", "legacy", false),
            ("daily dividend-adjusted", "stable", false),
            ("intraday 5min", "stable", true),
            ("intraday 5min", "legacy", false),
            ("splits", "stable", true),
            ("splits", "legacy", false),
            ("dividends", "stable", true),
            ("dividends", "legacy", false),
        ]
    );
    let detail = |endpoint: &str, api: &str| -> String {
        checks
            .iter()
            .find(|check| check.endpoint == endpoint && check.api == api)
            .map(|check| check.detail.clone())
            .unwrap()
    };
    assert_eq!(detail("quote", "stable"), "1 row, AAPL at 100");
    assert_eq!(detail("batch quote", "stable"), "2 rows");
    assert!(detail("daily", "stable").starts_with("2 rows, latest 20"));
    let intraday = detail("intraday 5min", "stable");
    assert!(
        intraday.starts_with("2 rows, latest 20") && intraday.ends_with(" UTC"),
        "{intraday}"
    );
    assert_eq!(detail("splits", "stable"), "5 rows, latest 2020-08-31");
    assert_eq!(detail("dividends", "stable"), "4 rows, latest 2025-08-11");
    assert!(
        detail("daily dividend-adjusted", "stable")
            .starts_with("not included in the market data plan: HTTP 402")
    );
    let legacy = detail("splits", "legacy");
    assert!(legacy.contains("Legacy Endpoint"), "{legacy}");

    // Plan-related 403s are not key problems: the second key was never needed.
    assert!(mock.keys().iter().all(|key| key == KEY_A));
    assert!(checks.iter().all(|check| !check.detail.contains(KEY_A)));
    // The report serialises as-is for the CLI's JSON output.
    assert_eq!(
        serde_json::to_value(&checks[0]).unwrap(),
        json!({"endpoint": "quote", "api": "stable", "ok": true, "detail": "1 row, AAPL at 100"})
    );
}

#[tokio::test]
async fn diagnose_refuses_an_invalid_symbol_without_requests() {
    let mock = MockFmp::start(|_| ok("[]")).await;

    let checks = diagnose(&mock.client(ApiMode::Auto), "A B").await;

    assert_eq!(checks.len(), 1);
    assert!(!checks[0].ok);
    assert!(checks[0].detail.contains("invalid symbol"));
    assert!(mock.seen().is_empty());
}
