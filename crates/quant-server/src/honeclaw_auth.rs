//! Sign-in through honeclaw (`HONE_QUANT_AUTH_MODE=honeclaw`).
//!
//! hone-quant runs under `hone-claw.com/quant`, so the browser sends honeclaw's session cookie
//! (`hone_web_session`, `Path=/`) with every request. hone-quant never trusts anything the
//! browser says about the user: for each request it asks honeclaw's own authentication
//! endpoint (`GET /api/public/auth/me`) whether that cookie is a live session and admits the
//! request only when the answer is `200` with `user.is_admin == true`.
//!
//! - `401`/`403` from honeclaw mean "not signed in"; a valid session whose user is not an
//!   administrator is refused with `403`.
//! - Anything else — a network error, a timeout, a redirect, another status or an answer that
//!   does not parse — fails closed: the request is refused with `503` and nothing is cached.
//! - Verdicts are cached briefly under the SHA-256 of the cookie (never the cookie itself):
//!   admissions for `cache_secs` (30 s by default), refusals for at most 10 s. A session revoked
//!   in honeclaw therefore stops working here within that window.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::StatusCode;
use reqwest::header;
use serde_json::Value;

use crate::auth::token_hash;
use crate::config::HoneclawAuthConfig;

/// Largest honeclaw answer read; `auth/me` is a few hundred bytes.
const MAX_BODY: usize = 256 * 1024;
/// Most cached verdicts kept; expired ones are dropped first, then everything.
const MAX_CACHED: usize = 4096;
/// Refusals are re-checked at least this often, so signing in takes effect quickly.
const REFUSAL_TTL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A live honeclaw session of an administrator.
    Admin { user_id: String, display: String },
    /// A live session of a user who is not an administrator.
    NotAdmin,
    /// No live session (missing, expired or signed out).
    SignedOut,
}

pub struct HoneclawVerifier {
    config: HoneclawAuthConfig,
    http: reqwest::Client,
    cache: Mutex<HashMap<String, (Instant, Verdict)>>,
}

impl HoneclawVerifier {
    pub fn new(config: HoneclawAuthConfig) -> Result<Self> {
        Self::with_timeout(config, Duration::from_secs(5))
    }

    pub fn with_timeout(config: HoneclawAuthConfig, timeout: Duration) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("hone-quant/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            config,
            http,
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn cookie_name(&self) -> &str {
        &self.config.cookie
    }

    pub fn login_url(&self) -> &str {
        &self.config.login_url
    }

    /// Checks a honeclaw session cookie. `Err` means the check could not be completed; callers
    /// must refuse the request.
    pub async fn verify(&self, cookie: &str) -> Result<Verdict> {
        if !valid_cookie_value(cookie) {
            return Ok(Verdict::SignedOut);
        }
        let key = token_hash(cookie);
        if let Some(verdict) = self.cached(&key) {
            return Ok(verdict);
        }
        let verdict = self.ask(cookie).await?;
        self.remember(key, &verdict);
        Ok(verdict)
    }

    async fn ask(&self, cookie: &str) -> Result<Verdict> {
        let response = self
            .http
            .get(&self.config.me_url)
            .header(header::COOKIE, format!("{}={cookie}", self.config.cookie))
            .header(header::ACCEPT, "application/json")
            .header(header::CACHE_CONTROL, "no-store")
            .send()
            .await
            .context("honeclaw authentication endpoint unreachable")?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => return Ok(Verdict::SignedOut),
            other => bail!("honeclaw authentication endpoint answered {other}"),
        }
        if response
            .content_length()
            .is_some_and(|n| n as usize > MAX_BODY)
        {
            bail!("honeclaw authentication answer is too large");
        }
        let bytes = response
            .bytes()
            .await
            .context("reading the honeclaw authentication answer")?;
        if bytes.len() > MAX_BODY {
            bail!("honeclaw authentication answer is too large");
        }
        let body: Value =
            serde_json::from_slice(&bytes).context("honeclaw authentication answer is not JSON")?;
        interpret(&body)
    }

    fn cached(&self, key: &str) -> Option<Verdict> {
        let cache = self.cache.lock().expect("honeclaw cache lock");
        cache
            .get(key)
            .filter(|(until, _)| *until > Instant::now())
            .map(|(_, verdict)| verdict.clone())
    }

    fn remember(&self, key: String, verdict: &Verdict) {
        let ttl = match verdict {
            Verdict::Admin { .. } => Duration::from_secs(self.config.cache_secs),
            _ => Duration::from_secs(self.config.cache_secs).min(REFUSAL_TTL),
        };
        if ttl.is_zero() {
            return;
        }
        let now = Instant::now();
        let mut cache = self.cache.lock().expect("honeclaw cache lock");
        if cache.len() >= MAX_CACHED {
            cache.retain(|_, (until, _)| *until > now);
            if cache.len() >= MAX_CACHED {
                cache.clear();
            }
        }
        cache.insert(key, (now + ttl, verdict.clone()));
    }
}

/// Reads honeclaw's `auth/me` answer: `{"user": {"user_id": "…", "is_admin": true, …}}`. Only
/// the JSON boolean `true` counts as an administrator; a malformed answer is an error.
pub fn interpret(body: &Value) -> Result<Verdict> {
    let user = body
        .get("user")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("honeclaw answer has no user object"))?;
    let user_id = user
        .get("user_id")
        .and_then(Value::as_str)
        .filter(|id| valid_user_id(id))
        .ok_or_else(|| anyhow!("honeclaw answer has no valid user_id"))?;
    if user.get("is_admin") != Some(&Value::Bool(true)) {
        return Ok(Verdict::NotAdmin);
    }
    let display = user
        .get("email_hint")
        .and_then(Value::as_str)
        .map(|hint| {
            hint.chars()
                .filter(|c| !c.is_control())
                .take(80)
                .collect::<String>()
        })
        .filter(|hint| !hint.trim().is_empty())
        .unwrap_or_else(|| user_id.chars().take(24).collect());
    Ok(Verdict::Admin {
        user_id: user_id.to_string(),
        display,
    })
}

/// RFC 6265 cookie-octets only (no spaces, quotes, commas, semicolons or backslashes), so the
/// value can be forwarded in a `Cookie` header verbatim.
pub fn valid_cookie_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && value
            .bytes()
            .all(|b| (0x21..=0x7e).contains(&b) && !matches!(b, b'"' | b',' | b';' | b'\\'))
}

/// honeclaw user ids become hone-quant audit actors (`honeclaw:<id>`): printable ASCII only.
fn valid_user_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// A stand-in for honeclaw's `GET /api/public/auth/me`, keyed by the `hone_web_session` value:
/// `admin` (administrator), `member` (signed in, not an administrator), `garbled` (unexpected
/// JSON), `broken` (500), `slow` (800 ms), anything else 401.
#[cfg(test)]
pub(crate) mod testkit {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use axum::Router;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::get;
    use serde_json::{Value, json};

    use super::HoneclawVerifier;
    use crate::config::HoneclawAuthConfig;

    pub async fn mock_honeclaw() -> (String, Arc<AtomicUsize>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let app = Router::new().route(
            "/api/public/auth/me",
            get(move |headers: HeaderMap| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    let cookie = headers
                        .get("cookie")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let ok = |body: Value| (StatusCode::OK, axum::Json(body));
                    match cookie.as_str() {
                        "hone_web_session=admin" => ok(json!({"user": {"user_id": "adm-1", "is_admin": true, "email_hint": "ad***@hone-claw.com"}})),
                        "hone_web_session=member" => ok(json!({"user": {"user_id": "usr-2", "is_admin": false}})),
                        "hone_web_session=garbled" => ok(json!({"unexpected": true})),
                        "hone_web_session=broken" => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            axum::Json(json!({"error": "boom"})),
                        ),
                        "hone_web_session=slow" => {
                            tokio::time::sleep(Duration::from_millis(800)).await;
                            ok(json!({"user": {"user_id": "adm-1", "is_admin": true}}))
                        }
                        _ => (
                            StatusCode::UNAUTHORIZED,
                            axum::Json(json!({"error": "unauthorized"})),
                        ),
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/api/public/auth/me"), hits)
    }

    pub fn verifier(me_url: String, cache_secs: u64) -> HoneclawVerifier {
        HoneclawVerifier::with_timeout(
            HoneclawAuthConfig {
                me_url,
                cookie: "hone_web_session".into(),
                login_url: "https://hone-claw.com/".into(),
                cache_secs,
            },
            Duration::from_millis(300),
        )
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use serde_json::json;

    use super::testkit::{mock_honeclaw, verifier};
    use super::*;

    #[test]
    fn only_a_json_true_admits() {
        let admin =
            json!({"user": {"user_id": "u-1", "is_admin": true, "email_hint": "a***@x.com"}});
        assert_eq!(
            interpret(&admin).unwrap(),
            Verdict::Admin {
                user_id: "u-1".into(),
                display: "a***@x.com".into()
            }
        );
        for not_admin in [
            json!({"user": {"user_id": "u-1", "is_admin": false}}),
            json!({"user": {"user_id": "u-1", "is_admin": "true"}}),
            json!({"user": {"user_id": "u-1", "is_admin": 1}}),
            json!({"user": {"user_id": "u-1"}}),
        ] {
            assert_eq!(interpret(&not_admin).unwrap(), Verdict::NotAdmin);
        }
        for malformed in [
            json!({}),
            json!({"user": null}),
            json!({"user": {"is_admin": true}}),
            json!({"user": {"user_id": "", "is_admin": true}}),
            json!({"user": {"user_id": "has space", "is_admin": true}}),
        ] {
            assert!(
                interpret(&malformed).is_err(),
                "{malformed} should be an error"
            );
        }
        let no_hint =
            json!({"user": {"user_id": "0123456789abcdef0123456789abcdef", "is_admin": true}});
        let Verdict::Admin { display, .. } = interpret(&no_hint).unwrap() else {
            panic!("admin expected");
        };
        assert_eq!(display, "0123456789abcdef01234567");
    }

    #[test]
    fn cookie_values_are_restricted_to_cookie_octets() {
        assert!(valid_cookie_value("abcDEF0123-_.~+/="));
        for bad in ["", "a b", "a;b", "a,b", "a\"b", "a\\b", "a\nb", "é"] {
            assert!(!valid_cookie_value(bad), "{bad:?}");
        }
        assert!(!valid_cookie_value(&"a".repeat(1025)));
    }

    #[tokio::test]
    async fn verdicts_follow_honeclaw_and_fail_closed() {
        let (url, _) = mock_honeclaw().await;
        let v = verifier(url, 0);
        assert!(matches!(
            v.verify("admin").await.unwrap(),
            Verdict::Admin { .. }
        ));
        assert_eq!(v.verify("member").await.unwrap(), Verdict::NotAdmin);
        assert_eq!(v.verify("expired").await.unwrap(), Verdict::SignedOut);
        // Not a cookie-octet string: refused without asking honeclaw.
        assert_eq!(v.verify("a;b").await.unwrap(), Verdict::SignedOut);
        // Errors, timeouts and malformed answers are errors, never admissions.
        assert!(v.verify("broken").await.is_err());
        assert!(v.verify("garbled").await.is_err());
        assert!(v.verify("slow").await.is_err());
        let unreachable = verifier("http://127.0.0.1:9/api/public/auth/me".into(), 30);
        assert!(unreachable.verify("admin").await.is_err());
    }

    #[tokio::test]
    async fn admissions_are_cached_but_errors_are_not() {
        let (url, hits) = mock_honeclaw().await;
        let v = verifier(url, 30);
        for _ in 0..3 {
            assert!(matches!(
                v.verify("admin").await.unwrap(),
                Verdict::Admin { .. }
            ));
        }
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "admission reused from the cache"
        );
        for _ in 0..2 {
            assert!(v.verify("broken").await.is_err());
        }
        assert_eq!(
            hits.load(Ordering::SeqCst),
            3,
            "errors are asked again every time"
        );
        assert_eq!(v.verify("member").await.unwrap(), Verdict::NotAdmin);
        assert_eq!(v.verify("member").await.unwrap(), Verdict::NotAdmin);
        assert_eq!(hits.load(Ordering::SeqCst), 4, "refusal reused briefly");
    }
}
