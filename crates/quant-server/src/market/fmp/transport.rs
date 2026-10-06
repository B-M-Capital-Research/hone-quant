//! HTTP transport for FMP: the key pool, throttling, retries and error classification.
//!
//! FMP reports failures in several ways — status codes, plain-text bodies, and HTTP 200 with an
//! `"Error Message"` object — and each calls for a different reaction: a rejected, exhausted or
//! rate-limited key hands the request to the next key; a plan restriction is the same for every
//! key and ends the request; a transport failure or 5xx earns one retry on the same key. `judge`
//! reduces every exchange to one of those verdicts, so the whole policy is one pure function.
//!
//! Keys only ever appear in the URL handed to reqwest. Provider text is redacted before it is
//! truncated into an error, and every message is redacted again on its way out, in case FMP
//! echoes the URL or the key back.

use std::fmt;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{Mutex, Semaphore, SemaphorePermit};
use tokio::time::Instant;

use super::endpoints::Request;
use super::{DEFAULT_BASE_URL, FmpConfig};
use crate::market::MarketError;

const USER_AGENT: &str = concat!("hone-quant/", env!("CARGO_PKG_VERSION"));
/// Pause before the single retry of a transient failure.
const RETRY_DELAY: Duration = Duration::from_millis(300);
/// Longest provider text quoted in an error message.
const MAX_DETAIL_CHARS: usize = 240;

/// Why a request failed. Unlike [`MarketError`] it keeps 404 apart, because "this API generation
/// has no such endpoint" is one of the triggers for the legacy fallback.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum FetchError {
    /// HTTP 404: the endpoint does not exist on this API generation.
    NotFound(String),
    /// Outside the subscription plan: HTTP 402, or a plan-related 403 or error message.
    Plan(String),
    /// Every key in the pool was rejected (invalid, exhausted or rate limited).
    Auth(String),
    /// Transport failure, timeout, a 5xx that survived its retry, or another provider error.
    Http(String),
    /// The response did not have the expected shape.
    Parse(String),
}

impl FetchError {
    /// Whether the other API generation might serve the request.
    pub(super) fn allows_fallback(&self) -> bool {
        matches!(self, FetchError::NotFound(_) | FetchError::Plan(_))
    }

    /// Whether FMP refused the legacy API for the whole account ("Legacy Endpoint : … only
    /// available for legacy users …") rather than for one endpoint or symbol.
    pub(super) fn closes_legacy(&self) -> bool {
        matches!(self, FetchError::Plan(message)
            if message.to_ascii_lowercase().contains("legacy endpoint"))
    }

    /// Appends context to the message, keeping the kind.
    pub(super) fn with_note(self, note: impl fmt::Display) -> Self {
        self.map_message(|message| format!("{message} ({note})"))
    }

    fn map_message(self, f: impl FnOnce(String) -> String) -> Self {
        match self {
            FetchError::NotFound(message) => FetchError::NotFound(f(message)),
            FetchError::Plan(message) => FetchError::Plan(f(message)),
            FetchError::Auth(message) => FetchError::Auth(f(message)),
            FetchError::Http(message) => FetchError::Http(f(message)),
            FetchError::Parse(message) => FetchError::Parse(f(message)),
        }
    }
}

impl From<FetchError> for MarketError {
    fn from(error: FetchError) -> Self {
        match error {
            FetchError::NotFound(message) | FetchError::Http(message) => MarketError::Http(message),
            FetchError::Plan(message) => MarketError::PlanRestricted(message),
            FetchError::Auth(message) => MarketError::Auth(message),
            FetchError::Parse(message) => MarketError::Parse(message),
        }
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Same wording as `MarketError`, so nested notes read like top-level errors.
        fmt::Display::fmt(&MarketError::from(self.clone()), f)
    }
}

/// Sends FMP requests through the key pool with throttling and retries.
pub(super) struct Transport {
    http: reqwest::Client,
    base_url: String,
    keys: Vec<String>,
    /// The key to try first: the one that last succeeded after a failover.
    preferred_key: AtomicUsize,
    throttle: Throttle,
}

/// How a request ended on one key.
enum KeyOutcome {
    /// This key cannot serve requests right now; the next key may.
    Rejected(String),
    /// Final, whichever key is used.
    Failed(FetchError),
}

impl Transport {
    /// Validates keys and base URL and builds the HTTP client.
    pub(super) fn new(config: &FmpConfig) -> Result<Self, MarketError> {
        let keys = key_pool(&config.api_keys);
        if keys.is_empty() {
            return Err(MarketError::NotConfigured(
                "no FMP API key configured".into(),
            ));
        }
        let base_url = normalize_base_url(&config.base_url);
        let parsed = reqwest::Url::parse(&base_url)
            .ok()
            .filter(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some()
                    && url.query().is_none()
                    && url.fragment().is_none()
            })
            .ok_or_else(|| {
                MarketError::NotConfigured(format!(
                    "invalid FMP base_url {:?}: expected an http(s) URL without a query",
                    redact_secrets(&base_url)
                ))
            })?;

        let timeout = Duration::from_secs(config.timeout_secs.max(1));
        let mut builder = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .gzip(true)
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(10)));
        if is_loopback(&parsed) {
            // Mock servers and local adapters must not be sent through a workstation proxy.
            builder = builder.no_proxy();
        }
        let http = builder.build().map_err(|error| {
            MarketError::NotConfigured(format!("cannot build the FMP HTTP client: {error}"))
        })?;

        Ok(Self {
            http,
            base_url,
            keys,
            preferred_key: AtomicUsize::new(0),
            throttle: Throttle::new(config.max_concurrency, config.requests_per_minute),
        })
    }

    pub(super) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(super) fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// GETs `request` and returns its JSON body.
    ///
    /// Keys are tried in pool order, starting from the last one that worked; only key-level
    /// failures move on to the next key, and when every key is rejected the result is
    /// [`FetchError::Auth`].
    pub(super) async fn get_json(&self, request: &Request) -> Result<Value, FetchError> {
        let count = self.keys.len();
        let first = self.preferred_key.load(Ordering::Relaxed) % count;
        let mut rejections = Vec::new();
        for offset in 0..count {
            let index = (first + offset) % count;
            match self.get_with_key(request, index).await {
                Ok(value) => {
                    if index != first {
                        self.preferred_key.store(index, Ordering::Relaxed);
                    }
                    return Ok(value);
                }
                Err(KeyOutcome::Rejected(reason)) => {
                    let reason = self.redact(&reason);
                    tracing::warn!(
                        key = index + 1,
                        keys = count,
                        request = %request,
                        %reason,
                        "FMP rejected the API key"
                    );
                    rejections.push(format!("key #{}: {reason}", index + 1));
                }
                Err(KeyOutcome::Failed(error)) => {
                    return Err(error.map_message(|message| self.redact(&message)));
                }
            }
        }
        Err(FetchError::Auth(format!(
            "all {count} key(s) rejected for {request} ({})",
            rejections.join("; ")
        )))
    }

    /// One request on one key, retrying a transient failure once.
    async fn get_with_key(&self, request: &Request, index: usize) -> Result<Value, KeyOutcome> {
        let url = request.url(&self.base_url, &self.keys[index]);
        let what = request.to_string();
        let mut verdict = self.attempt(&url, &what).await;
        if let Verdict::Transient(reason) = &verdict {
            tracing::debug!(request = %what, %reason, "transient FMP failure; retrying once");
            tokio::time::sleep(RETRY_DELAY).await;
            verdict = self.attempt(&url, &what).await;
        }
        match verdict {
            Verdict::Data(value) => Ok(value),
            Verdict::KeyRejected(reason) => Err(KeyOutcome::Rejected(reason)),
            Verdict::Transient(reason) => Err(KeyOutcome::Failed(FetchError::Http(reason))),
            Verdict::Fail(error) => Err(KeyOutcome::Failed(error)),
        }
    }

    /// One throttled HTTP exchange. `url` carries the key; only `what` may be shown.
    async fn attempt(&self, url: &str, what: &str) -> Verdict {
        let redact = |text: &str| self.redact(text);
        let _permit = self.throttle.acquire().await;
        let started = Instant::now();
        let response = match self.http.get(url).send().await {
            Ok(response) => response,
            Err(error) => return transport_failure(what, error, &redact),
        };
        let status = response.status().as_u16();
        match response.bytes().await {
            Ok(body) => {
                tracing::debug!(
                    request = what,
                    status,
                    bytes = body.len(),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "FMP response"
                );
                judge(status, &body, what, &redact)
            }
            Err(error) => transport_failure(what, error, &redact),
        }
    }

    /// Masks API keys in `text`: every `apikey`-style parameter value and every configured key
    /// that appears verbatim.
    pub(super) fn redact(&self, text: &str) -> String {
        let mut redacted = redact_secrets(text);
        for key in &self.keys {
            if redacted.contains(key.as_str()) {
                redacted = redacted.replace(key.as_str(), "***");
            }
        }
        redacted
    }
}

/// Trimmed, non-empty keys in configured order, without duplicates.
fn key_pool(keys: &[String]) -> Vec<String> {
    let mut pool: Vec<String> = Vec::new();
    for key in keys
        .iter()
        .map(|key| key.trim())
        .filter(|key| !key.is_empty())
    {
        if !pool.iter().any(|existing| existing == key) {
            pool.push(key.to_string());
        }
    }
    pool
}

/// Reduces a configured base URL to the host root that FMP paths hang off: trims whitespace and
/// trailing slashes and strips an `/api/v3`, `/api` or `/stable` suffix (honeclaw configures
/// `…/api`). An empty value means the public FMP host.
pub(super) fn normalize_base_url(raw: &str) -> String {
    let mut url = raw.trim().trim_end_matches('/');
    for suffix in ["/api/v3", "/api", "/stable"] {
        if let Some(stripped) = strip_suffix_ignore_ascii_case(url, suffix) {
            url = stripped.trim_end_matches('/');
            break;
        }
    }
    if url.is_empty() {
        DEFAULT_BASE_URL.to_string()
    } else {
        url.to_string()
    }
}

fn strip_suffix_ignore_ascii_case<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = text.len().checked_sub(suffix.len())?;
    // The suffix is ASCII, so a match guarantees `cut` is a char boundary.
    text.as_bytes()[cut..]
        .eq_ignore_ascii_case(suffix.as_bytes())
        .then(|| &text[..cut])
}

fn is_loopback(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Caps concurrent requests and spaces request starts to stay under FMP's per-minute limit.
struct Throttle {
    permits: Semaphore,
    /// Minimum gap between request starts; zero disables spacing.
    spacing: Duration,
    /// Earliest start of the next request.
    next_start: Mutex<Instant>,
}

impl Throttle {
    fn new(max_concurrency: usize, requests_per_minute: u32) -> Self {
        let spacing = match requests_per_minute {
            0 => Duration::ZERO,
            per_minute => Duration::from_secs(60) / per_minute,
        };
        Self {
            // `Semaphore::new` panics above `MAX_PERMITS`; zero would deadlock.
            permits: Semaphore::new(max_concurrency.clamp(1, Semaphore::MAX_PERMITS)),
            spacing,
            next_start: Mutex::new(Instant::now()),
        }
    }

    /// Waits for a concurrency slot, then for this request's turn to start.
    async fn acquire(&self) -> SemaphorePermit<'_> {
        let permit = self
            .permits
            .acquire()
            .await
            .expect("the throttle semaphore is never closed");
        if !self.spacing.is_zero() {
            let start = {
                let mut next = self.next_start.lock().await;
                let start = (*next).max(Instant::now());
                *next = start + self.spacing;
                start
            };
            tokio::time::sleep_until(start).await;
        }
        permit
    }
}

/// What one HTTP exchange means for the request.
#[derive(Debug, PartialEq)]
enum Verdict {
    Data(Value),
    /// Worth one more attempt on the same key.
    Transient(String),
    /// This key cannot serve the request (invalid, exhausted or rate limited).
    KeyRejected(String),
    /// Final, whichever key is used.
    Fail(FetchError),
}

/// Who a provider error message blames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blame {
    /// The key: invalid, exhausted or rate limited.
    Key,
    /// The subscription plan: the endpoint or symbol is not included.
    Plan,
    Other,
}

/// FMP's wording overlaps — "Limit Reach. Please upgrade your plan" is about the key's quota,
/// while "Exclusive Endpoint: … not available under your current subscription … upgrade your
/// plan" is about the plan — so unambiguous key phrases win first, then plan phrases, and only
/// then the weaker key hints.
fn blame(message: &str) -> Blame {
    const KEY: &[&str] = &[
        "invalid api key",
        "limit reach",
        "rate limit",
        "too many requests",
        "quota",
    ];
    const PLAN: &[&str] = &["premium", "subscription", "not available", "exclusive"];
    const KEY_HINTS: &[&str] = &["key", "limit", "upgrade"];
    let lower = message.to_ascii_lowercase();
    let mentions = |phrases: &[&str]| phrases.iter().any(|phrase| lower.contains(phrase));
    if mentions(KEY) {
        Blame::Key
    } else if mentions(PLAN) {
        Blame::Plan
    } else if mentions(KEY_HINTS) {
        Blame::Key
    } else {
        Blame::Other
    }
}

/// Classifies one response. `what` names the request without its key; `redact` is applied to
/// provider text before it is truncated into a message.
fn judge(status: u16, body: &[u8], what: &str, redact: &dyn Fn(&str) -> String) -> Verdict {
    if (200..=299).contains(&status) {
        return judge_success(body, what, redact);
    }
    // Error bodies are plain text or an `{"Error Message": …}` object; quote the message itself.
    let message = serde_json::from_slice::<Value>(body)
        .ok()
        .as_ref()
        .and_then(provider_error)
        .unwrap_or_else(|| String::from_utf8_lossy(body).into_owned());
    let detailed = |prefix: String| {
        let detail = excerpt(&redact(&message));
        if detail.is_empty() {
            prefix
        } else {
            format!("{prefix}: {detail}")
        }
    };
    match status {
        401 | 429 => Verdict::KeyRejected(detailed(format!("HTTP {status}"))),
        402 => Verdict::Fail(FetchError::Plan(detailed(format!("HTTP 402 for {what}")))),
        403 if blame(&message) == Blame::Plan => {
            Verdict::Fail(FetchError::Plan(detailed(format!("HTTP 403 for {what}"))))
        }
        403 => Verdict::KeyRejected(detailed("HTTP 403".to_string())),
        404 => Verdict::Fail(FetchError::NotFound(format!("HTTP 404 for {what}"))),
        408 | 500..=599 => Verdict::Transient(detailed(format!("HTTP {status} for {what}"))),
        _ => Verdict::Fail(FetchError::Http(detailed(format!(
            "HTTP {status} for {what}"
        )))),
    }
}

/// A 2xx response is data unless it is an error object (or a non-JSON text) in disguise.
fn judge_success(body: &[u8], what: &str, redact: &dyn Fn(&str) -> String) -> Verdict {
    let (message, parse_error) = match serde_json::from_slice::<Value>(body) {
        Ok(value) => match provider_error(&value) {
            Some(message) => (message, None),
            None => return Verdict::Data(value),
        },
        // FMP occasionally answers in plain text; that can still be a key or plan problem.
        Err(error) => (String::from_utf8_lossy(body).into_owned(), Some(error)),
    };
    let detail = excerpt(&redact(&message));
    let is_html = message.trim_start().starts_with('<');
    match (blame(&message), parse_error) {
        (Blame::Key, _) if !is_html => Verdict::KeyRejected(detail),
        (Blame::Plan, _) if !is_html => {
            Verdict::Fail(FetchError::Plan(format!("{what}: {detail}")))
        }
        (_, Some(error)) => Verdict::Fail(FetchError::Parse(format!(
            "{what} returned invalid JSON ({error}): {detail}"
        ))),
        (_, None) => Verdict::Fail(FetchError::Http(format!("FMP error for {what}: {detail}"))),
    }
}

/// The provider's message when the body is an `{"Error Message": …}` or `{"error": …}` object.
fn provider_error(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    ["Error Message", "error"]
        .iter()
        .find_map(|field| match object.get(*field)? {
            Value::String(text) => Some(text.trim().to_string()).filter(|text| !text.is_empty()),
            Value::Null | Value::Bool(false) => None,
            other => Some(other.to_string()),
        })
}

/// Connect failures, timeouts and truncated bodies are transient; a request that could not even
/// be built is not. reqwest's message would include the URL (and so the key), hence `without_url`.
fn transport_failure(
    what: &str,
    error: reqwest::Error,
    redact: &dyn Fn(&str) -> String,
) -> Verdict {
    let error = error.without_url();
    let message = format!("{what}: {}", redact(&error_chain(&error)));
    if error.is_builder() {
        Verdict::Fail(FetchError::Http(message))
    } else {
        Verdict::Transient(message)
    }
}

/// An error followed by its sources, e.g. `error sending request: … tcp connect error: …`.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    message
}

/// Provider text for an error message: whitespace collapsed, at most [`MAX_DETAIL_CHARS`].
fn excerpt(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_DETAIL_CHARS {
        collapsed
    } else {
        collapsed.chars().take(MAX_DETAIL_CHARS).collect::<String>() + "…"
    }
}

/// Masks the value of every `apikey` / `api_key` parameter in `text`, whatever the case and
/// whether written as a query parameter (`apikey=…`), a label (`apikey: …`) or a JSON field
/// (`"apikey":"…"`).
pub(super) fn redact_secrets(text: &str) -> String {
    // ASCII lower-casing keeps byte offsets identical, so positions found in `lower` index `text`.
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut redacted = String::with_capacity(text.len());
    let mut copied = 0;
    let mut search = 0;
    while let Some((name_start, name_len)) = find_key_name(&lower, search) {
        let mut cursor = name_start + name_len;
        search = cursor;
        if bytes.get(cursor) == Some(&b'"') {
            cursor += 1;
        }
        while bytes.get(cursor) == Some(&b' ') {
            cursor += 1;
        }
        if !matches!(bytes.get(cursor), Some(b'=' | b':')) {
            continue;
        }
        cursor += 1;
        while bytes.get(cursor) == Some(&b' ') {
            cursor += 1;
        }
        if bytes.get(cursor) == Some(&b'"') {
            cursor += 1;
        }
        let value_start = cursor;
        while cursor < bytes.len() && !ends_secret(bytes[cursor]) {
            cursor += 1;
        }
        if cursor > value_start {
            redacted.push_str(&text[copied..value_start]);
            redacted.push_str("***");
            copied = cursor;
        }
        search = cursor;
    }
    redacted.push_str(&text[copied..]);
    redacted
}

fn find_key_name(lower: &str, from: usize) -> Option<(usize, usize)> {
    ["apikey", "api_key"]
        .iter()
        .filter_map(|name| lower[from..].find(name).map(|at| (from + at, name.len())))
        .min_by_key(|(at, _)| *at)
}

fn ends_secret(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'&' | b'"' | b'\'' | b')' | b',' | b';' | b'}' | b']' | b'#' | b'<' | b'>'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> String {
        text.to_string()
    }

    fn judged(status: u16, body: &str) -> Verdict {
        judge(status, body.as_bytes(), "/stable/quote?symbol=AAPL", &plain)
    }

    fn config(keys: &[&str], base_url: &str) -> FmpConfig {
        FmpConfig {
            api_keys: keys.iter().map(|key| key.to_string()).collect(),
            base_url: base_url.to_string(),
            ..FmpConfig::default()
        }
    }

    #[test]
    fn blame_tells_key_problems_from_plan_problems() {
        let cases = [
            (
                "Invalid API KEY. Feel free to create a Free API Key",
                Blame::Key,
            ),
            (
                "Limit Reach . Please upgrade your plan or visit our documentation",
                Blame::Key,
            ),
            ("Too Many Requests", Blame::Key),
            ("You have exhausted your daily quota", Blame::Key),
            ("Please upgrade", Blame::Key),
            (
                "Exclusive Endpoint : This endpoint is not available under your current \
                 subscription agreement, please visit our subscription page to upgrade your plan",
                Blame::Plan,
            ),
            (
                "Premium Query Parameter: 'Special Endpoint : This value set for 'symbol' is not \
                 available under your current subscription",
                Blame::Plan,
            ),
            (
                "Legacy Endpoint : Due to Legacy endpoints being no longer supported - This \
                 endpoint is only available for legacy users who have valid subscriptions prior \
                 August 31, 2025.",
                Blame::Plan,
            ),
            ("temporary upstream calculation failure", Blame::Other),
            ("", Blame::Other),
        ];
        for (message, expected) in cases {
            assert_eq!(blame(message), expected, "{message}");
        }
    }

    #[test]
    fn judge_success_bodies() {
        assert_eq!(
            judged(200, r#"[{"symbol":"AAPL"}]"#),
            Verdict::Data(serde_json::json!([{"symbol": "AAPL"}]))
        );
        assert_eq!(judged(200, "[]"), Verdict::Data(serde_json::json!([])));
        // An empty error field is not an error.
        assert_eq!(
            judged(200, r#"{"symbol":"AAPL","error":null}"#),
            Verdict::Data(serde_json::json!({"symbol": "AAPL", "error": null}))
        );
        assert!(matches!(
            judged(200, r#"{"Error Message":"Limit Reach . Please upgrade your plan"}"#),
            Verdict::KeyRejected(reason) if reason.contains("Limit Reach")
        ));
        assert!(matches!(
            judged(200, r#"{"Error Message":"Invalid API KEY."}"#),
            Verdict::KeyRejected(_)
        ));
        assert!(matches!(
            judged(200, r#"{"error":"Exclusive Endpoint : not available under your current subscription"}"#),
            Verdict::Fail(FetchError::Plan(message)) if message.starts_with("/stable/quote")
        ));
        assert!(matches!(
            judged(200, r#"{"Error Message":"temporary upstream calculation failure"}"#),
            Verdict::Fail(FetchError::Http(message)) if message.contains("temporary upstream")
        ));
        assert!(matches!(
            judged(200, "Limit Reach"),
            Verdict::KeyRejected(_)
        ));
        assert!(matches!(
            judged(200, "<html>rate limit exceeded</html>"),
            Verdict::Fail(FetchError::Parse(_))
        ));
        assert!(matches!(
            judged(200, ""),
            Verdict::Fail(FetchError::Parse(_))
        ));
    }

    #[test]
    fn judge_status_codes() {
        // JSON error bodies are quoted by their message, plain-text bodies as they are.
        assert_eq!(
            judged(401, r#"{"Error Message":"Invalid API KEY."}"#),
            Verdict::KeyRejected("HTTP 401: Invalid API KEY.".to_string())
        );
        assert_eq!(
            judged(429, "Too Many Requests"),
            Verdict::KeyRejected("HTTP 429: Too Many Requests".to_string())
        );
        assert!(matches!(judged(429, ""), Verdict::KeyRejected(reason) if reason == "HTTP 429"));
        assert!(matches!(judged(403, "Forbidden"), Verdict::KeyRejected(_)));
        assert!(matches!(
            judged(
                403,
                r#"{"Error Message":"Exclusive Endpoint : This endpoint is not available under your current subscription"}"#
            ),
            Verdict::Fail(FetchError::Plan(_))
        ));
        assert_eq!(
            judged(
                402,
                "Premium Query Parameter: 'Special Endpoint : This value set for 'symbol' is not available'"
            ),
            Verdict::Fail(FetchError::Plan(
                "HTTP 402 for /stable/quote?symbol=AAPL: Premium Query Parameter: 'Special \
                 Endpoint : This value set for 'symbol' is not available'"
                    .to_string()
            ))
        );
        assert_eq!(
            judged(404, "Not Found"),
            Verdict::Fail(FetchError::NotFound(
                "HTTP 404 for /stable/quote?symbol=AAPL".into()
            ))
        );
        assert!(matches!(judged(500, "oops"), Verdict::Transient(_)));
        assert!(matches!(judged(503, ""), Verdict::Transient(_)));
        assert!(matches!(judged(408, ""), Verdict::Transient(_)));
        assert!(
            matches!(judged(400, "bad from"), Verdict::Fail(FetchError::Http(message)) if message.contains("HTTP 400"))
        );
    }

    #[test]
    fn judge_redacts_before_truncating() {
        // The echoed key straddles the truncation point: truncating first would leave a prefix
        // of the key that verbatim replacement no longer recognises.
        let key = "SECRET-KEY-0123456789";
        let body = format!("{} {key}", "x".repeat(MAX_DETAIL_CHARS - 10));
        let redact = |text: &str| text.replace(key, "***");
        let Verdict::Transient(message) = judge(502, body.as_bytes(), "/q", &redact) else {
            panic!("502 is transient");
        };
        assert!(!message.contains("SECRET"), "{message}");
        assert!(message.ends_with(" ***"), "{message}");
    }

    #[test]
    fn excerpt_collapses_whitespace_and_truncates() {
        assert_eq!(excerpt("  a\n  b\tc "), "a b c");
        let long = "é".repeat(MAX_DETAIL_CHARS + 5);
        let cut = excerpt(&long);
        assert_eq!(cut.chars().count(), MAX_DETAIL_CHARS + 1);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn redact_secrets_masks_every_apikey_form() {
        let cases = [
            (
                "error sending request for url (https://fmp.test/api/v3/quote/AAPL?apikey=abc123)",
                "error sending request for url (https://fmp.test/api/v3/quote/AAPL?apikey=***)",
            ),
            (
                "https://fmp.test/stable/quote?symbol=AAPL&apikey=abc123&x=1",
                "https://fmp.test/stable/quote?symbol=AAPL&apikey=***&x=1",
            ),
            (
                "?api_key=one&apiKey=two&APIKEY=three",
                "?api_key=***&apiKey=***&APIKEY=***",
            ),
            (
                r#"{"apikey":"abc","apiKey" : "def"}"#,
                r#"{"apikey":"***","apiKey" : "***"}"#,
            ),
            ("apikey: abc123 is invalid", "apikey: *** is invalid"),
            (
                "no secrets here, apikey alone",
                "no secrets here, apikey alone",
            ),
            ("apikey=", "apikey="),
            ("ünïcode apikey=ß€cret tail", "ünïcode apikey=*** tail"),
        ];
        for (input, expected) in cases {
            assert_eq!(redact_secrets(input), expected, "{input}");
        }
    }

    #[test]
    fn transport_redacts_configured_keys_verbatim() {
        let transport = Transport::new(&config(
            &["alpha-key-0001", "beta-key-0002"],
            "https://fmp.test",
        ))
        .unwrap();
        assert_eq!(
            transport.redact("key beta-key-0002 is invalid; apikey=alpha-key-0001"),
            "key *** is invalid; apikey=***"
        );
    }

    #[test]
    fn base_urls_normalize_to_the_host_root() {
        let cases = [
            (
                "https://financialmodelingprep.com",
                "https://financialmodelingprep.com",
            ),
            (
                "https://financialmodelingprep.com/",
                "https://financialmodelingprep.com",
            ),
            (
                "https://financialmodelingprep.com/api",
                "https://financialmodelingprep.com",
            ),
            (
                "https://financialmodelingprep.com/api/",
                "https://financialmodelingprep.com",
            ),
            (
                "https://financialmodelingprep.com/api/v3",
                "https://financialmodelingprep.com",
            ),
            (
                " https://financialmodelingprep.com/API/V3/ ",
                "https://financialmodelingprep.com",
            ),
            (
                "https://financialmodelingprep.com/stable",
                "https://financialmodelingprep.com",
            ),
            ("http://127.0.0.1:8080/fmp/api", "http://127.0.0.1:8080/fmp"),
            ("", DEFAULT_BASE_URL),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_base_url(input), expected, "{input:?}");
        }
    }

    #[test]
    fn key_pool_trims_dedupes_and_keeps_order() {
        let keys: Vec<String> = ["b", " a ", "", "b", "  ", "c", "a"]
            .iter()
            .map(|key| key.to_string())
            .collect();
        assert_eq!(key_pool(&keys), vec!["b", "a", "c"]);
    }

    #[test]
    fn construction_validates_keys_and_base_url() {
        assert!(matches!(
            Transport::new(&config(&["", "  "], DEFAULT_BASE_URL)),
            Err(MarketError::NotConfigured(_))
        ));
        for bad in [
            "ftp://fmp.test",
            "not a url",
            "https://fmp.test/?apikey=SECRET",
        ] {
            match Transport::new(&config(&["k"], bad)) {
                Err(MarketError::NotConfigured(message)) => {
                    assert!(!message.contains("SECRET"), "{message}")
                }
                Err(other) => panic!("unexpected error for {bad:?}: {other}"),
                Ok(_) => panic!("{bad:?} must be rejected"),
            }
        }
        let transport = Transport::new(&config(&["k", "k"], "https://fmp.test/api/v3")).unwrap();
        assert_eq!(transport.base_url(), "https://fmp.test");
        assert_eq!(transport.key_count(), 1);
    }

    #[test]
    fn loopback_hosts_are_detected() {
        let loopback = |url: &str| is_loopback(&reqwest::Url::parse(url).unwrap());
        assert!(loopback("http://127.0.0.1:8080"));
        assert!(loopback("http://127.1.2.3"));
        assert!(loopback("http://localhost:3000/api"));
        assert!(loopback("http://[::1]:8080"));
        assert!(!loopback("https://financialmodelingprep.com"));
        assert!(!loopback("http://10.0.0.1"));
    }

    #[test]
    fn fetch_errors_map_onto_market_errors() {
        assert!(matches!(
            MarketError::from(FetchError::NotFound("x".into())),
            MarketError::Http(_)
        ));
        assert!(matches!(
            MarketError::from(FetchError::Plan("x".into())),
            MarketError::PlanRestricted(_)
        ));
        assert!(matches!(
            MarketError::from(FetchError::Auth("x".into())),
            MarketError::Auth(_)
        ));
        assert!(matches!(
            MarketError::from(FetchError::Parse("x".into())),
            MarketError::Parse(_)
        ));
        let noted = FetchError::Plan("HTTP 402".into()).with_note("legacy fallback: HTTP 403");
        assert_eq!(
            noted.to_string(),
            "not included in the market data plan: HTTP 402 (legacy fallback: HTTP 403)"
        );
        assert!(noted.allows_fallback());
        assert!(!FetchError::Auth("x".into()).allows_fallback());

        let closed = FetchError::Plan(
            "HTTP 403 for /api/v3/quote/AAPL: Legacy Endpoint : Due to Legacy endpoints being no \
             longer supported"
                .into(),
        );
        assert!(closed.closes_legacy());
        assert!(!FetchError::Plan("HTTP 402: Premium Query Parameter".into()).closes_legacy());
        assert!(!FetchError::Http("Legacy Endpoint".into()).closes_legacy());
    }

    #[tokio::test]
    async fn throttle_tolerates_extreme_settings() {
        for max_concurrency in [0, usize::MAX] {
            let throttle = Throttle::new(max_concurrency, u32::MAX);
            drop(throttle.acquire().await);
            drop(throttle.acquire().await);
        }
    }

    #[tokio::test]
    async fn throttle_spaces_request_starts() {
        let throttle = Throttle::new(4, 1200); // 50 ms apart
        let started = Instant::now();
        for _ in 0..3 {
            drop(throttle.acquire().await);
        }
        assert!(started.elapsed() >= Duration::from_millis(100));
    }
}
