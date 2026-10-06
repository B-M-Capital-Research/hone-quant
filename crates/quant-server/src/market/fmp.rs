//! Financial Modeling Prep (FMP) market data client.
//!
//! FMP runs two API generations side by side: the current `/stable/*` API and the legacy
//! `/api/v3/*` API, which only subscriptions opened before August 2025 can still call. Plans also
//! differ per endpoint, so one key may get daily bars from one generation and intraday bars only
//! from the other. [`ApiMode::Auto`] therefore asks `/stable` first, falls back to legacy when an
//! endpoint is missing (404) or outside the plan, and remembers per endpoint family which
//! generation answered, so steady-state calls cost a single request.
//!
//! Keys form an ordered pool: a key that is invalid, exhausted or rate limited hands the request
//! to the next one, plan restrictions are the same for every key and are reported at once, and
//! transport failures and 5xx responses are retried once on the same key. Keys never appear in
//! errors or logs.
//!
//! Data is normalised on the way in: `change_pct` is a fraction, bars are oldest first,
//! de-duplicated and clipped to the requested range, and intraday bars are converted from New
//! York wall-clock time to UTC and limited to the regular session.
//!
//! Internals: `endpoints` (URLs of both generations, batching and chunking rules), `transport`
//! (key pool, throttling, retries, error classification, redaction) and `wire` (response shapes
//! and their mapping onto the domain types).

mod endpoints;
mod transport;
mod wire;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use chrono::{Duration, NaiveDate, Utc};
use quant_core::calendar::MARKET_TZ;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::endpoints::{ApiKind, Family, Request};
use self::transport::{FetchError, Transport};
use self::wire::AdjustedPrices;
use super::{
    DailyBar, DataSource, Dividend, Interval, IntradayBar, MarketData, MarketError, Quote, Split,
};

/// The public FMP host.
const DEFAULT_BASE_URL: &str = "https://financialmodelingprep.com";

/// Which FMP API generation(s) the client may call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApiMode {
    /// Try `/stable` first and fall back to legacy `/api/v3` when an endpoint is unavailable.
    #[default]
    Auto,
    /// Only `/stable` (every current subscription).
    Stable,
    /// Only legacy `/api/v3` (subscriptions opened before August 2025).
    Legacy,
}

/// FMP client settings. `Debug` output never shows the keys.
#[derive(Clone)]
pub struct FmpConfig {
    /// Ordered key pool; blank and duplicate entries are ignored.
    pub api_keys: Vec<String>,
    /// Host root, e.g. `https://financialmodelingprep.com`. A trailing `/api` or `/api/v3`
    /// (honeclaw's config style) is accepted and stripped.
    pub base_url: String,
    /// Timeout of each HTTP request.
    pub timeout_secs: u64,
    /// Which API generation(s) to call.
    pub api_mode: ApiMode,
    /// Most requests in flight at once.
    pub max_concurrency: usize,
    /// Request starts are spaced `60s / requests_per_minute` apart; 0 disables spacing.
    pub requests_per_minute: u32,
}

impl Default for FmpConfig {
    fn default() -> Self {
        Self {
            api_keys: Vec::new(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_secs: 20,
            api_mode: ApiMode::Auto,
            max_concurrency: 4,
            requests_per_minute: 240,
        }
    }
}

impl fmt::Debug for FmpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FmpConfig")
            .field(
                "api_keys",
                &format_args!("<{} redacted>", self.api_keys.len()),
            )
            .field("base_url", &self.base_url)
            .field("timeout_secs", &self.timeout_secs)
            .field("api_mode", &self.api_mode)
            .field("max_concurrency", &self.max_concurrency)
            .field("requests_per_minute", &self.requests_per_minute)
            .finish()
    }
}

/// [`MarketData`] backed by FMP. All state is internally synchronised; share it behind an `Arc`.
pub struct FmpClient {
    transport: Transport,
    mode: ApiMode,
    /// `Auto` mode: the API generation that answered each endpoint family.
    routes: Mutex<HashMap<Family, ApiKind>>,
    /// `Auto` mode: set once FMP says this account has no legacy access at all, after which
    /// failing stable endpoints are no longer re-tried on legacy.
    legacy_closed: AtomicBool,
    /// Whether the "no dividend-adjusted prices" warning has been logged.
    adjusted_warned: AtomicBool,
}

impl fmt::Debug for FmpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FmpClient")
            .field("base_url", &self.transport.base_url())
            .field("keys", &self.transport.key_count())
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

impl FmpClient {
    /// Validates the configuration (at least one non-blank key, an http(s) base URL) and builds
    /// the HTTP client.
    pub fn new(config: FmpConfig) -> Result<Self, MarketError> {
        Ok(Self {
            transport: Transport::new(&config)?,
            mode: config.api_mode,
            routes: Mutex::new(HashMap::new()),
            legacy_closed: AtomicBool::new(false),
            adjusted_warned: AtomicBool::new(false),
        })
    }

    /// The normalised host root that requests are sent to.
    pub fn base_url(&self) -> &str {
        self.transport.base_url()
    }

    /// Number of distinct keys in the pool.
    pub fn key_count(&self) -> usize {
        self.transport.key_count()
    }

    /// Runs `op` on the API generation the mode selects for `family` and returns the generation
    /// that answered along with the value.
    ///
    /// In [`ApiMode::Auto`] a family without a remembered generation tries stable first and, if
    /// the endpoint is missing or outside the plan, legacy; whichever succeeds is remembered so
    /// later calls go straight to it. Nothing is remembered while both fail, except that an
    /// account FMP reports as having no legacy access stops being probed on legacy at all.
    async fn route<T, F, Fut>(&self, family: Family, op: F) -> Result<(ApiKind, T), FetchError>
    where
        F: Fn(ApiKind) -> Fut,
        Fut: Future<Output = Result<T, FetchError>>,
    {
        let remembered = self.remembered(family);
        let first = match self.mode {
            ApiMode::Stable => ApiKind::Stable,
            ApiMode::Legacy => ApiKind::Legacy,
            ApiMode::Auto => remembered.unwrap_or(ApiKind::Stable),
        };
        let probing = self.mode == ApiMode::Auto && remembered.is_none();
        match op(first).await {
            Ok(value) => {
                if probing {
                    self.remember(family, first);
                }
                Ok((first, value))
            }
            Err(error)
                if probing
                    && error.allows_fallback()
                    && !self.legacy_closed.load(Ordering::Relaxed) =>
            {
                tracing::info!(
                    family = family.as_str(),
                    %error,
                    "FMP /stable endpoint unavailable; trying the legacy API"
                );
                match op(ApiKind::Legacy).await {
                    Ok(value) => {
                        tracing::info!(
                            family = family.as_str(),
                            "using the legacy FMP API for this endpoint family"
                        );
                        self.remember(family, ApiKind::Legacy);
                        Ok((ApiKind::Legacy, value))
                    }
                    Err(legacy) => {
                        if legacy.closes_legacy()
                            && !self.legacy_closed.swap(true, Ordering::Relaxed)
                        {
                            tracing::info!(
                                "this FMP account has no legacy API access; Auto mode now only uses /stable"
                            );
                        }
                        Err(error.with_note(format_args!("legacy fallback: {legacy}")))
                    }
                }
            }
            Err(error) => Err(error),
        }
    }

    fn remembered(&self, family: Family) -> Option<ApiKind> {
        self.routes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&family)
            .copied()
    }

    fn remember(&self, family: Family, api: ApiKind) {
        self.routes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(family, api);
    }

    fn note_adjusted_unavailable(&self, symbol: &str, error: &FetchError) {
        if self.adjusted_warned.swap(true, Ordering::Relaxed) {
            tracing::debug!(symbol, %error, "FMP dividend-adjusted prices unavailable");
        } else {
            tracing::warn!(
                symbol,
                %error,
                "FMP dividend-adjusted prices are unavailable; daily bars will lack total-return prices"
            );
        }
    }

    /// GETs `request` and parses the body, naming the request in parse errors.
    async fn get<T>(
        &self,
        request: Request,
        parse: impl FnOnce(Value) -> Result<T, FetchError>,
    ) -> Result<T, FetchError> {
        let value = self.transport.get_json(&request).await?;
        parse(value).map_err(|error| error.with_note(format_args!("from {request}")))
    }

    async fn fetch_quotes(
        &self,
        api: ApiKind,
        symbols: &[String],
    ) -> Result<Vec<Quote>, FetchError> {
        self.get(endpoints::quotes(api, symbols), wire::quotes)
            .await
    }

    /// Single-quote requests for each symbol; symbols outside the plan are skipped (and logged).
    async fn quotes_one_by_one(&self, symbols: &[String]) -> Result<Vec<Quote>, FetchError> {
        let results = futures::future::join_all(symbols.iter().map(|symbol| async move {
            let one = std::slice::from_ref(symbol);
            (
                symbol,
                self.route(Family::Quote, move |api| self.fetch_quotes(api, one))
                    .await,
            )
        }))
        .await;
        let mut quotes = Vec::new();
        let mut refused = Vec::new();
        for (symbol, result) in results {
            match result {
                Ok((_, mut found)) => quotes.append(&mut found),
                Err(FetchError::Plan(_)) => refused.push(symbol.as_str()),
                Err(error) => return Err(error),
            }
        }
        if !refused.is_empty() {
            tracing::warn!(symbols = ?refused, "quotes for these symbols are not included in the FMP plan");
        }
        Ok(quotes)
    }

    /// Daily bars for consecutive, ascending `chunks`; concatenating the per-chunk results (each
    /// sorted and clipped) keeps the whole oldest first and free of duplicates.
    async fn fetch_daily(
        &self,
        api: ApiKind,
        symbol: &str,
        chunks: &[(NaiveDate, NaiveDate)],
    ) -> Result<Vec<DailyBar>, FetchError> {
        let mut bars = Vec::new();
        for &(from, to) in chunks {
            let request = endpoints::daily(api, symbol, from, to);
            bars.extend(
                self.get(request, |value| wire::daily_bars(value, symbol, from, to))
                    .await?,
            );
        }
        Ok(bars)
    }

    async fn fetch_adjusted(
        &self,
        api: ApiKind,
        symbol: &str,
        chunks: &[(NaiveDate, NaiveDate)],
    ) -> Result<Vec<AdjustedPrices>, FetchError> {
        let mut prices = Vec::new();
        for &(from, to) in chunks {
            let request = endpoints::adjusted(api, symbol, from, to);
            prices.extend(self.get(request, wire::adjusted_prices).await?);
        }
        Ok(prices)
    }

    async fn fetch_intraday(
        &self,
        api: ApiKind,
        symbol: &str,
        interval: Interval,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<IntradayBar>, FetchError> {
        let request = endpoints::intraday(api, symbol, interval, from, to);
        self.get(request, |value| wire::intraday_bars(value, from, to))
            .await
    }

    async fn fetch_splits(&self, api: ApiKind, symbol: &str) -> Result<Vec<Split>, FetchError> {
        self.get(endpoints::splits(api, symbol), |value| {
            wire::splits(value, symbol)
        })
        .await
    }

    async fn fetch_dividends(
        &self,
        api: ApiKind,
        symbol: &str,
    ) -> Result<Vec<Dividend>, FetchError> {
        self.get(endpoints::dividends(api, symbol), |value| {
            wire::dividends(value, symbol)
        })
        .await
    }
}

/// Valid, upper-cased, de-duplicated symbols in request order; invalid ones are skipped.
fn quote_symbols(symbols: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut valid = Vec::new();
    for raw in symbols {
        let Some(symbol) = endpoints::normalize_symbol(raw) else {
            tracing::debug!(symbol = ?raw, "skipping invalid symbol");
            continue;
        };
        if seen.insert(symbol.clone()) {
            valid.push(symbol);
        }
    }
    valid
}

/// The upper-cased symbol of a single-symbol request. Text that cannot be a ticker is refused
/// before it can reach a URL.
fn single_symbol(raw: &str) -> Result<String, MarketError> {
    endpoints::normalize_symbol(raw).ok_or_else(|| {
        MarketError::Http(format!(
            "invalid symbol {raw:?}: expected letters, digits and . - _ ^"
        ))
    })
}

#[async_trait]
impl MarketData for FmpClient {
    fn source(&self) -> DataSource {
        DataSource::Fmp
    }

    /// Requests at most 25 symbols (and 700 encoded characters) at a time; invalid symbols are
    /// skipped. A batch the subscription refuses (the batch endpoint is not in the plan, or one
    /// symbol is premium) is retried symbol by symbol on the single-quote endpoint, so one
    /// restricted symbol cannot blank out the others; symbols still refused are left out of the
    /// result. Any other failure fails the call.
    async fn quotes(&self, symbols: &[String]) -> Result<Vec<Quote>, MarketError> {
        let batches = endpoints::quote_batches(&quote_symbols(symbols));
        let results = futures::future::try_join_all(batches.iter().map(|batch| async move {
            match self
                .route(Family::Quote, move |api| self.fetch_quotes(api, batch))
                .await
            {
                Ok((_, quotes)) => Ok(quotes),
                Err(FetchError::Plan(message)) if batch.len() > 1 => {
                    tracing::debug!(%message, "batch quote refused by the plan; quoting one by one");
                    self.quotes_one_by_one(batch).await
                }
                Err(error) => Err(error),
            }
        }))
        .await?;
        let mut seen = HashSet::new();
        Ok(results
            .into_iter()
            .flatten()
            .filter(|quote| seen.insert(quote.symbol.clone()))
            .collect())
    }

    /// Split-adjusted OHLCV with total-return `adj_open`/`adj_close`. On the stable API those
    /// come from the separate dividend-adjusted series; when the plan lacks it the bars are
    /// still returned, without `adj_*`.
    async fn daily_bars(
        &self,
        symbol: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<DailyBar>, MarketError> {
        let symbol = single_symbol(symbol)?;
        if from > to {
            return Ok(Vec::new());
        }
        let chunks = endpoints::date_chunks(from, to);
        let (api, mut bars) = self
            .route(Family::Daily, |api| self.fetch_daily(api, &symbol, &chunks))
            .await?;
        // Legacy rows carry `adjClose` already.
        if api == ApiKind::Stable && !bars.is_empty() {
            let adjusted = self
                .route(Family::Adjusted, |api| {
                    self.fetch_adjusted(api, &symbol, &chunks)
                })
                .await;
            match adjusted {
                Ok((_, prices)) => wire::merge_adjusted(&mut bars, &prices),
                Err(error) if error.allows_fallback() => {
                    self.note_adjusted_unavailable(&symbol, &error);
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(bars)
    }

    async fn intraday_bars(
        &self,
        symbol: &str,
        interval: Interval,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<IntradayBar>, MarketError> {
        let symbol = single_symbol(symbol)?;
        if from > to {
            return Ok(Vec::new());
        }
        let (_, bars) = self
            .route(Family::Intraday, |api| {
                self.fetch_intraday(api, &symbol, interval, from, to)
            })
            .await?;
        Ok(bars)
    }

    async fn splits(&self, symbol: &str) -> Result<Vec<Split>, MarketError> {
        let symbol = single_symbol(symbol)?;
        let (_, splits) = self
            .route(Family::Splits, |api| self.fetch_splits(api, &symbol))
            .await?;
        Ok(splits)
    }

    async fn dividends(&self, symbol: &str) -> Result<Vec<Dividend>, MarketError> {
        let symbol = single_symbol(symbol)?;
        let (_, dividends) = self
            .route(Family::Dividends, |api| self.fetch_dividends(api, &symbol))
            .await?;
        Ok(dividends)
    }
}

/// Result of probing one endpoint family on one API generation with the configured keys.
#[derive(Debug, Clone, Serialize)]
pub struct EndpointCheck {
    /// Endpoint family, e.g. `"quote"` or `"intraday 5min"`.
    pub endpoint: String,
    /// `"stable"` or `"legacy"`.
    pub api: String,
    pub ok: bool,
    /// What came back (`"12 rows, latest 2026-10-02"`) or the (redacted) error.
    pub detail: String,
}

impl EndpointCheck {
    fn probe<T>(
        endpoint: &str,
        api: ApiKind,
        outcome: Result<T, FetchError>,
        describe: impl FnOnce(&T) -> String,
    ) -> Self {
        let (ok, detail) = match outcome {
            Ok(value) => (true, describe(&value)),
            Err(error) => (false, error.to_string()),
        };
        Self {
            endpoint: endpoint.to_string(),
            api: api.as_str().to_string(),
            ok,
            detail,
        }
    }
}

/// Probes quote, batch quote, daily, dividend-adjusted daily (stable only), 5-minute intraday
/// (last five days), splits and dividends for `symbol` on both API generations — the report
/// behind `hone-quant fmp check`.
///
/// Probes run one after another, bypass `Auto` routing and leave its memory untouched, so the
/// report shows what each generation can serve with the configured keys.
pub async fn diagnose(client: &FmpClient, symbol: &str) -> Vec<EndpointCheck> {
    const BOTH: [ApiKind; 2] = [ApiKind::Stable, ApiKind::Legacy];
    let Some(symbol) = endpoints::normalize_symbol(symbol) else {
        return vec![EndpointCheck {
            endpoint: "symbol".to_string(),
            api: "-".to_string(),
            ok: false,
            detail: format!("invalid symbol {symbol:?}"),
        }];
    };
    let today = Utc::now().with_timezone(&MARKET_TZ).date_naive();
    let recent = [(today - Duration::days(14), today)];
    let intraday_from = today - Duration::days(5);
    let single = [symbol.clone()];
    let companion = if symbol == "SPY" { "QQQ" } else { "SPY" };
    let pair = [symbol.clone(), companion.to_string()];

    let mut checks = Vec::new();
    for api in BOTH {
        let outcome = client.fetch_quotes(api, &single).await;
        checks.push(EndpointCheck::probe(
            "quote",
            api,
            outcome,
            |quotes| match quotes.first() {
                Some(quote) => format!(
                    "{}, {} at {}",
                    rows(quotes.len()),
                    quote.symbol,
                    quote.price
                ),
                None => rows(0),
            },
        ));
    }
    let outcome = client.fetch_quotes(ApiKind::Stable, &pair).await;
    checks.push(EndpointCheck::probe(
        "batch quote",
        ApiKind::Stable,
        outcome,
        |quotes| rows(quotes.len()),
    ));
    for api in BOTH {
        let outcome = client.fetch_daily(api, &symbol, &recent).await;
        checks.push(EndpointCheck::probe("daily", api, outcome, |bars| {
            latest(bars.len(), bars.last().map(|bar| bar.date))
        }));
    }
    let outcome = client
        .fetch_adjusted(ApiKind::Stable, &symbol, &recent)
        .await;
    checks.push(EndpointCheck::probe(
        "daily dividend-adjusted",
        ApiKind::Stable,
        outcome,
        |prices| latest(prices.len(), prices.iter().map(|day| day.date).max()),
    ));
    for api in BOTH {
        let outcome = client
            .fetch_intraday(api, &symbol, Interval::FiveMin, intraday_from, today)
            .await;
        checks.push(EndpointCheck::probe(
            "intraday 5min",
            api,
            outcome,
            |bars| {
                latest(
                    bars.len(),
                    bars.last().map(|bar| bar.ts.format("%Y-%m-%d %H:%M UTC")),
                )
            },
        ));
    }
    for api in BOTH {
        let outcome = client.fetch_splits(api, &symbol).await;
        checks.push(EndpointCheck::probe("splits", api, outcome, |splits| {
            latest(splits.len(), splits.last().map(|split| split.date))
        }));
    }
    for api in BOTH {
        let outcome = client.fetch_dividends(api, &symbol).await;
        checks.push(EndpointCheck::probe(
            "dividends",
            api,
            outcome,
            |dividends| {
                latest(
                    dividends.len(),
                    dividends.last().map(|dividend| dividend.ex_date),
                )
            },
        ));
    }
    checks
}

fn rows(count: usize) -> String {
    if count == 1 {
        "1 row".to_string()
    } else {
        format!("{count} rows")
    }
}

fn latest(count: usize, last: Option<impl fmt::Display>) -> String {
    match last {
        Some(last) => format!("{}, latest {last}", rows(count)),
        None => rows(count),
    }
}
