//! Operator authentication.
//!
//! Mirrors honeclaw's session design: a random 256-bit token in an HttpOnly, SameSite=Strict
//! cookie, stored server-side only as its SHA-256. Passwords are Argon2id (PHC strings).
//! Mutating requests must also carry the `X-Hone-Quant-Action` header, which a cross-site form
//! cannot send — together with SameSite=Strict this closes CSRF without tokens in the page.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::FromRequestParts;
use axum::http::HeaderMap;
use axum::http::request::Parts;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::api::error::ApiError;
use crate::config::CookieSecurity;
use crate::honeclaw_auth::{HoneclawVerifier, Verdict};
use crate::state::SharedState;
use crate::store::system;

pub const SESSION_COOKIE: &str = "hone_quant_session";
pub const SESSION_TTL_DAYS: i64 = 30;
pub const ACTION_HEADER: &str = "x-hone-quant-action";
pub const MIN_PASSWORD_LEN: usize = 10;

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow!("password hashing failed: {e}"))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// A real hash of a throw-away password, so failed logins for unknown users cost the same
/// Argon2 work as for known ones.
pub fn dummy_hash() -> &'static str {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| hash_password("timing-equalisation-only").expect("argon2 works"))
}

pub fn check_password_strength(password: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        ));
    }
    if password.chars().all(|c| c.is_ascii_digit()) {
        return Err("password must not be only digits".into());
    }
    Ok(())
}

pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// In-memory brute-force protection: 8 failures within 10 minutes block the key for 15 minutes.
#[derive(Default)]
pub struct LoginLimiter {
    failures: Mutex<HashMap<String, Vec<Instant>>>,
    blocked: Mutex<HashMap<String, Instant>>,
}

const WINDOW: Duration = Duration::from_secs(600);
const MAX_FAILURES: usize = 8;
const BLOCK: Duration = Duration::from_secs(900);

impl LoginLimiter {
    /// `Err(remaining)` while the key is blocked.
    pub fn check(&self, key: &str) -> Result<(), Duration> {
        let mut blocked = self.blocked.lock().expect("limiter lock");
        if let Some(until) = blocked.get(key) {
            let now = Instant::now();
            if *until > now {
                return Err(*until - now);
            }
            blocked.remove(key);
        }
        Ok(())
    }

    pub fn record_failure(&self, key: &str) {
        let now = Instant::now();
        let mut failures = self.failures.lock().expect("limiter lock");
        if failures.len() > 10_000 {
            failures.retain(|_, v| v.iter().any(|t| now.duration_since(*t) < WINDOW));
        }
        let entry = failures.entry(key.to_string()).or_default();
        entry.retain(|t| now.duration_since(*t) < WINDOW);
        entry.push(now);
        if entry.len() >= MAX_FAILURES {
            entry.clear();
            self.blocked
                .lock()
                .expect("limiter lock")
                .insert(key.to_string(), now + BLOCK);
        }
    }

    pub fn record_success(&self, key: &str) {
        self.failures.lock().expect("limiter lock").remove(key);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    Viewer,
}

impl Role {
    pub fn parse(value: &str) -> Self {
        if value == "admin" {
            Role::Admin
        } else {
            Role::Viewer
        }
    }
}

#[derive(Debug, Clone)]
pub struct CurrentUser {
    /// Local operator id; 0 for honeclaw administrators (no local account).
    pub id: i64,
    /// Audit identity: the local username, or `honeclaw:<user id>`.
    pub username: String,
    /// Name shown in the UI.
    pub display_name: String,
    pub role: Role,
    pub token_hash: String,
    pub ip: String,
    /// Signed in through honeclaw rather than a hone-quant session.
    pub external: bool,
}

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

/// Client address for the audit log: Cloudflare's `CF-Connecting-IP` when present, else the
/// first `X-Forwarded-For` hop when behind a proxy, else the peer.
pub fn client_ip(parts_headers: &HeaderMap, peer: Option<std::net::SocketAddr>) -> String {
    let header = |name: &str| {
        parts_headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|v| v.trim().chars().take(64).collect::<String>())
            .filter(|v| !v.is_empty())
    };
    header("cf-connecting-ip")
        .or_else(|| header("x-forwarded-for"))
        .or_else(|| peer.map(|p| p.ip().to_string()))
        .unwrap_or_default()
}

pub fn wants_secure_cookie(security: CookieSecurity, headers: &HeaderMap) -> bool {
    match security {
        CookieSecurity::Always => true,
        CookieSecurity::Never => false,
        CookieSecurity::Auto => headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("https")),
    }
}

pub fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        SESSION_TTL_DAYS * 86_400,
        if secure { "; Secure" } else { "" }
    )
}

pub fn clear_cookie(secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

impl FromRequestParts<SharedState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0);
        if let Some(verifier) = &state.honeclaw {
            return honeclaw_user(verifier, &parts.headers, client_ip(&parts.headers, peer)).await;
        }
        let token = cookie_value(&parts.headers, SESSION_COOKIE).ok_or(ApiError::Unauthorized)?;
        if token.len() != 64 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ApiError::Unauthorized);
        }
        let hash = token_hash(&token);
        let client = state
            .pool
            .get()
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        let user = system::session_user(&client, &hash)
            .await?
            .ok_or(ApiError::Unauthorized)?;
        Ok(CurrentUser {
            id: user.id,
            display_name: user.username.clone(),
            username: user.username,
            role: Role::parse(&user.role),
            token_hash: hash,
            ip: client_ip(&parts.headers, peer),
            external: false,
        })
    }
}

/// honeclaw sign-in: the browser's honeclaw session must belong to an administrator, as
/// confirmed by honeclaw itself. Fails closed when honeclaw cannot be asked.
async fn honeclaw_user(
    verifier: &HoneclawVerifier,
    headers: &HeaderMap,
    ip: String,
) -> Result<CurrentUser, ApiError> {
    let cookie = cookie_value(headers, verifier.cookie_name()).ok_or(ApiError::Unauthorized)?;
    match verifier.verify(&cookie).await {
        Ok(Verdict::Admin { user_id, display }) => Ok(CurrentUser {
            id: 0,
            username: format!("honeclaw:{user_id}"),
            display_name: display,
            role: Role::Admin,
            token_hash: String::new(),
            ip,
            external: true,
        }),
        Ok(Verdict::NotAdmin) => Err(ApiError::Forbidden(
            "hone-quant is limited to hone-claw.com administrators".into(),
        )),
        Ok(Verdict::SignedOut) => Err(ApiError::Unauthorized),
        Err(error) => {
            tracing::warn!(error = %format!("{error:#}"), "honeclaw sign-in check failed; request refused");
            Err(ApiError::Unavailable(
                "the hone-claw.com sign-in cannot be verified right now".into(),
            ))
        }
    }
}

/// An operator with the admin role (required for anything that changes trading behaviour).
#[derive(Debug, Clone)]
pub struct AdminUser(pub CurrentUser);

impl FromRequestParts<SharedState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        if user.role != Role::Admin {
            return Err(ApiError::Forbidden(
                "this action requires the admin role".into(),
            ));
        }
        Ok(AdminUser(user))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_round_trip() {
        let hash = hash_password("correct horse battery").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &hash));
        assert!(!verify_password("wrong", &hash));
        assert!(!verify_password("x", "not a hash"));
    }

    #[test]
    fn password_strength() {
        assert!(check_password_strength("short").is_err());
        assert!(check_password_strength("1234567890").is_err());
        assert!(check_password_strength("a-long-passphrase").is_ok());
    }

    #[test]
    fn tokens_are_random_and_hashed() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(token_hash(&a).len(), 64);
        assert_ne!(token_hash(&a), a);
    }

    #[test]
    fn limiter_blocks_after_repeated_failures() {
        let limiter = LoginLimiter::default();
        for _ in 0..7 {
            limiter.record_failure("admin");
        }
        assert!(limiter.check("admin").is_ok());
        limiter.record_failure("admin");
        assert!(limiter.check("admin").is_err());
        assert!(limiter.check("other").is_ok());
    }

    #[tokio::test]
    async fn honeclaw_sessions_map_to_admins_or_refusals() {
        use crate::honeclaw_auth::testkit::{mock_honeclaw, verifier};
        let (url, _) = mock_honeclaw().await;
        let verifier = verifier(url, 30);
        let with_cookie = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::COOKIE,
                format!("theme=dark; hone_web_session={value}; lang=zh")
                    .parse()
                    .unwrap(),
            );
            headers
        };
        let Ok(user) = honeclaw_user(&verifier, &with_cookie("admin"), "203.0.113.9".into()).await
        else {
            panic!("an administrator session must be admitted");
        };
        assert_eq!(user.username, "honeclaw:adm-1");
        assert_eq!(user.display_name, "ad***@hone-claw.com");
        assert_eq!(user.role, Role::Admin);
        assert!(user.external);
        assert_eq!(user.ip, "203.0.113.9");
        assert!(matches!(
            honeclaw_user(&verifier, &with_cookie("member"), String::new()).await,
            Err(ApiError::Forbidden(_))
        ));
        assert!(matches!(
            honeclaw_user(&verifier, &with_cookie("expired"), String::new()).await,
            Err(ApiError::Unauthorized)
        ));
        assert!(matches!(
            honeclaw_user(&verifier, &HeaderMap::new(), String::new()).await,
            Err(ApiError::Unauthorized)
        ));
        // A browser cannot assert anything itself: a forged admin flag is just another cookie.
        let mut forged = with_cookie("expired");
        forged.insert("x-hone-admin", "true".parse().unwrap());
        assert!(matches!(
            honeclaw_user(&verifier, &forged, String::new()).await,
            Err(ApiError::Unauthorized)
        ));
        // honeclaw unavailable: refused, never admitted.
        assert!(matches!(
            honeclaw_user(&verifier, &with_cookie("broken"), String::new()).await,
            Err(ApiError::Unavailable(_))
        ));
    }

    #[test]
    fn audit_ip_prefers_cloudflare_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "172.68.1.1, 10.0.0.1".parse().unwrap());
        assert_eq!(client_ip(&headers, None), "172.68.1.1");
        headers.insert("cf-connecting-ip", "198.51.100.7".parse().unwrap());
        assert_eq!(client_ip(&headers, None), "198.51.100.7");
    }

    #[test]
    fn cookie_parsing() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            "a=1; hone_quant_session=abc; b=2".parse().unwrap(),
        );
        assert_eq!(
            cookie_value(&headers, SESSION_COOKIE).as_deref(),
            Some("abc")
        );
        assert_eq!(cookie_value(&headers, "missing"), None);
        assert!(session_cookie("t", true).contains("Secure"));
        assert!(!session_cookie("t", false).contains("Secure"));
    }
}
