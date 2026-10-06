//! Runtime configuration, read from the environment (and an optional `.env` in the working
//! directory for local development).
//!
//! Everything that is a deployment fact — where PostgreSQL is, which market data keys to use,
//! which port to bind — lives here. Everything an operator tunes while the app runs (plan times,
//! costs, notification channels, strategy) lives in the database and is edited from the UI.
//!
//! PostgreSQL resolution order: `HONE_QUANT_DATABASE_URL`, then discrete `HONE_QUANT_PG_*`
//! variables, then honeclaw's own `DATABASE_URL` and `HONE_POSTGRES_*` variables (sharing the
//! instance and database, isolated by schema). FMP keys: `HONE_QUANT_FMP_API_KEYS` / `HONE_QUANT_FMP_API_KEY`, or the
//! `fmp:` block of honeclaw's `config.yaml` via `HONE_QUANT_HONECLAW_CONFIG`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, NaiveDate, Utc};
use sha2::{Digest, Sha256};

use crate::market::DataSource;
use crate::market::fmp::{ApiMode, FmpConfig};

pub const DEFAULT_BIND: &str = "127.0.0.1:8090";
pub const DEFAULT_SCHEMA: &str = "hone_quant";
pub const DEFAULT_DEMO_SCHEMA: &str = "hone_quant_demo";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieSecurity {
    /// Secure when the request arrived over HTTPS (directly or via a proxy header).
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Text,
    Json,
}

#[derive(Clone)]
pub struct DbConfig {
    pub pg: tokio_postgres::Config,
    pub schema: String,
    pub pool_size: usize,
    /// Human-readable target without the password, for logs and the status page.
    pub display: String,
    /// True when the connection settings were borrowed from honeclaw's `HONE_POSTGRES_*`.
    pub borrowed_from_honeclaw: bool,
}

impl DbConfig {
    /// Demo clock: shifts `app_now()` on every pooled connection so database timestamps follow
    /// the simulated clock (see migration 0001).
    pub fn set_clock_offset(&mut self, offset: chrono::Duration) {
        let secs = offset.num_milliseconds() as f64 / 1000.0;
        self.pg.options(format!(
            "-c search_path={} -c hone_quant.clock_offset_secs={secs}",
            self.schema
        ));
    }
}

impl std::fmt::Debug for DbConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbConfig")
            .field("target", &self.display)
            .field("schema", &self.schema)
            .field("pool_size", &self.pool_size)
            .finish()
    }
}

#[derive(Clone)]
pub struct MarketConfig {
    pub source: DataSource,
    pub fmp: FmpConfig,
    pub demo_seed: u64,
    /// Where the FMP keys came from (for the status page; never the keys themselves).
    pub key_origin: String,
}

#[derive(Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub db: DbConfig,
    pub market: MarketConfig,
    pub state_dir: PathBuf,
    /// 32-byte key protecting notification-channel credentials stored in the database.
    pub secret_key: [u8; 32],
    pub cookie_security: CookieSecurity,
    /// Base URL used for links in outbound notifications (e.g. `http://localhost:8090`).
    pub public_url: Option<String>,
    pub bootstrap_admin: Option<(String, String)>,
    pub log_format: LogFormat,
    /// Optional replacement for the bundled universe file.
    pub universe_path: Option<PathBuf>,
    /// Serve the web UI from this directory instead of the embedded build (development).
    pub web_dir: Option<PathBuf>,
    pub scheduler_enabled: bool,
    /// Exchange closures that the rule-based calendar cannot derive.
    pub extra_closures: Vec<NaiveDate>,
    /// Demo-only: start the clock at this instant (and let it run), to demo market hours.
    pub dev_clock_start: Option<DateTime<Utc>>,
    /// `dev_clock_start` minus the wall clock at startup; shared by the scheduler's clock and the
    /// database connections so both agree.
    pub dev_clock_offset: Option<chrono::Duration>,
    pub initial_cash: f64,
    /// URL prefix the app is served under: "" at the root, or e.g. "/quant" behind
    /// hone-claw.com. The web UI must be built for the same prefix.
    pub base_path: String,
    pub auth: AuthMode,
    /// When set, every request except the health check must carry this value in
    /// `X-Hone-Quant-Origin-Token` (added by the Cloudflare Worker), so the origin cannot be
    /// used directly.
    pub origin_token: Option<String>,
}

/// How operators sign in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMode {
    /// hone-quant's own operator accounts (password and session cookie).
    Local,
    /// honeclaw's accounts: the browser's honeclaw session is checked against honeclaw's
    /// authentication endpoint on the server, and only honeclaw administrators are admitted.
    Honeclaw(HoneclawAuthConfig),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoneclawAuthConfig {
    /// `GET` endpoint that validates the session cookie and answers
    /// `{"user": {"user_id": …, "is_admin": …}}`.
    pub me_url: String,
    /// Name of honeclaw's session cookie.
    pub cookie: String,
    /// Where operators sign in to honeclaw (shown when no admin session is present).
    pub login_url: String,
    /// Seconds a successful check is reused; refusals are reused for at most 10 seconds.
    pub cache_secs: u64,
}

pub const DEFAULT_HONECLAW_AUTH_URL: &str = "https://hone-claw.com/api/public/auth/me";
pub const DEFAULT_HONECLAW_LOGIN_URL: &str = "https://hone-claw.com/";
pub const DEFAULT_HONECLAW_COOKIE: &str = "hone_web_session";

/// Normalises a URL prefix: "" for the root, otherwise "/segment[/segment…]" without a
/// trailing slash.
pub fn normalize_base_path(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let Some(rest) = trimmed.strip_prefix('/') else {
        bail!("HONE_QUANT_BASE_PATH must start with '/', e.g. /quant (got {raw:?})");
    };
    for segment in rest.split('/') {
        let ok = !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
        if !ok {
            bail!(
                "HONE_QUANT_BASE_PATH may only contain '/'-separated segments of letters, digits, '-', '_' and '.' (got {raw:?})"
            );
        }
    }
    Ok(trimmed.to_string())
}

/// The honeclaw endpoint receives operators' session cookies, so it must be HTTPS (plain HTTP
/// only on the loopback interface, e.g. to reach honeclaw on the same host).
pub fn check_auth_url(raw: &str) -> Result<()> {
    let url = reqwest::Url::parse(raw)
        .with_context(|| format!("HONE_QUANT_HONECLAW_AUTH_URL is not a URL: {raw:?}"))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match url.scheme() {
        "https" => Ok(()),
        "http" if loopback => Ok(()),
        _ => bail!(
            "HONE_QUANT_HONECLAW_AUTH_URL must use https (http only for localhost), got {raw:?}"
        ),
    }
}

fn auth_mode() -> Result<AuthMode> {
    match var("HONE_QUANT_AUTH_MODE").as_deref().unwrap_or("local") {
        "local" => Ok(AuthMode::Local),
        "honeclaw" => {
            let me_url = var("HONE_QUANT_HONECLAW_AUTH_URL")
                .unwrap_or_else(|| DEFAULT_HONECLAW_AUTH_URL.to_string());
            check_auth_url(&me_url)?;
            let cookie = var("HONE_QUANT_HONECLAW_SESSION_COOKIE")
                .unwrap_or_else(|| DEFAULT_HONECLAW_COOKIE.to_string());
            if !cookie
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            {
                bail!("HONE_QUANT_HONECLAW_SESSION_COOKIE is not a valid cookie name: {cookie:?}");
            }
            let login_url = var("HONE_QUANT_HONECLAW_LOGIN_URL")
                .unwrap_or_else(|| DEFAULT_HONECLAW_LOGIN_URL.to_string());
            let parsed = reqwest::Url::parse(&login_url).with_context(|| {
                format!("HONE_QUANT_HONECLAW_LOGIN_URL is not a URL: {login_url:?}")
            })?;
            if !matches!(parsed.scheme(), "https" | "http") {
                bail!("HONE_QUANT_HONECLAW_LOGIN_URL must be an http(s) URL");
            }
            Ok(AuthMode::Honeclaw(HoneclawAuthConfig {
                me_url,
                cookie,
                login_url,
                cache_secs: parse_num("HONE_QUANT_HONECLAW_CACHE_SECS", 30u64)?.min(300),
            }))
        }
        other => bail!("HONE_QUANT_AUTH_MODE must be 'local' or 'honeclaw', got {other:?}"),
    }
}

fn origin_token() -> Result<Option<String>> {
    match var("HONE_QUANT_ORIGIN_TOKEN") {
        None => Ok(None),
        Some(token) if token.len() >= 32 && token.chars().all(|c| c.is_ascii_graphic()) => {
            Ok(Some(token))
        }
        Some(_) => bail!(
            "HONE_QUANT_ORIGIN_TOKEN must be at least 32 printable characters (e.g. openssl rand -hex 32)"
        ),
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn parse_bool(name: &str, default: bool) -> Result<bool> {
    match var(name) {
        None => Ok(default),
        Some(v) => match v.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            other => bail!("{name} must be a boolean, got {other:?}"),
        },
    }
}

fn parse_num<T: FromStr>(name: &str, default: T) -> Result<T> {
    match var(name) {
        None => Ok(default),
        Some(v) => v
            .parse::<T>()
            .map_err(|_| anyhow!("{name} must be a number, got {v:?}")),
    }
}

/// Loads `KEY=VALUE` lines from `.env` in the working directory without overriding variables
/// that are already set. Intended for local development only.
pub fn load_dotenv() {
    let Ok(text) = std::fs::read_to_string(".env") else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if std::env::var_os(key).is_none() {
                // SAFETY: called once at startup before any threads are spawned.
                unsafe { std::env::set_var(key, value) };
            }
        }
    }
}

pub fn valid_schema_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && name.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn database_config(source: DataSource) -> Result<DbConfig> {
    let default_schema = match source {
        DataSource::Fmp => DEFAULT_SCHEMA,
        DataSource::Demo => DEFAULT_DEMO_SCHEMA,
    };
    let schema = var("HONE_QUANT_DB_SCHEMA").unwrap_or_else(|| default_schema.to_string());
    if !valid_schema_name(&schema) {
        bail!("HONE_QUANT_DB_SCHEMA must match ^[a-z_][a-z0-9_]*$ (got {schema:?})");
    }
    let pool_size = parse_num("HONE_QUANT_DB_POOL_SIZE", 8usize)?.clamp(2, 32);

    let mut borrowed = false;
    let mut pg = if let Some(url) = var("HONE_QUANT_DATABASE_URL") {
        tokio_postgres::Config::from_str(&url)
            .context("HONE_QUANT_DATABASE_URL is not a valid PostgreSQL URL")?
    } else if var("HONE_QUANT_PG_HOST").is_none()
        && let Some(url) = var("DATABASE_URL")
    {
        // honeclaw's primary setting: share its instance and database, isolated by schema.
        borrowed = true;
        tokio_postgres::Config::from_str(&url)
            .context("DATABASE_URL is not a valid PostgreSQL URL")?
    } else {
        let (prefix, is_borrowed) = if var("HONE_QUANT_PG_HOST").is_some() {
            ("HONE_QUANT_PG_", false)
        } else if var("HONE_POSTGRES_HOST").is_some() {
            ("HONE_POSTGRES_", true)
        } else {
            bail!(
                "PostgreSQL is not configured: set HONE_QUANT_DATABASE_URL or HONE_QUANT_PG_HOST/PORT/USER/PASSWORD/DATABASE (honeclaw's DATABASE_URL and HONE_POSTGRES_* are also accepted)"
            );
        };
        borrowed = is_borrowed;
        let get = |suffix: &str| var(&format!("{prefix}{suffix}"));
        let mut cfg = tokio_postgres::Config::new();
        cfg.host(get("HOST").unwrap_or_else(|| "127.0.0.1".into()));
        cfg.port(
            get("PORT")
                .map(|p| p.parse::<u16>())
                .transpose()
                .context("PostgreSQL port must be a number")?
                .unwrap_or(5432),
        );
        cfg.user(&get("USER").ok_or_else(|| anyhow!("{prefix}USER is required"))?);
        if let Some(password) = get("PASSWORD") {
            cfg.password(password);
        }
        cfg.dbname(&get("DATABASE").ok_or_else(|| anyhow!("{prefix}DATABASE is required"))?);
        cfg
    };
    pg.application_name("hone-quant");
    pg.connect_timeout(std::time::Duration::from_secs(10));
    // Pin every connection to the hone-quant schema; nothing is ever created in `public`.
    pg.options(format!("-c search_path={schema}"));

    let hosts: Vec<String> = pg
        .get_hosts()
        .iter()
        .map(|h| match h {
            tokio_postgres::config::Host::Tcp(h) => h.clone(),
            #[cfg(unix)]
            tokio_postgres::config::Host::Unix(p) => p.display().to_string(),
        })
        .collect();
    let display = format!(
        "postgres://{}@{}:{}/{} (schema {schema})",
        pg.get_user().unwrap_or("?"),
        hosts.join(","),
        pg.get_ports().first().copied().unwrap_or(5432),
        pg.get_dbname().unwrap_or("?"),
    );
    Ok(DbConfig {
        pg,
        schema,
        pool_size,
        display,
        borrowed_from_honeclaw: borrowed,
    })
}

/// Reads `fmp.api_key`, `fmp.api_keys`, `fmp.base_url` and `fmp.timeout` from a honeclaw
/// `config.yaml` (and its sibling `config.overrides.yaml`, which takes precedence).
pub fn fmp_from_honeclaw(path: &Path) -> Result<(Vec<String>, Option<String>, Option<u64>)> {
    fn read(path: &Path) -> Result<Option<serde_yaml::Value>> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                Ok(Some(serde_yaml::from_str(&text).with_context(|| {
                    format!("{} is not valid YAML", path.display())
                })?))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }
    let base = read(path)?.ok_or_else(|| anyhow!("{} does not exist", path.display()))?;
    let overrides = read(&path.with_file_name("config.overrides.yaml"))?;
    let pick = |key: &str| -> Option<serde_yaml::Value> {
        overrides
            .as_ref()
            .and_then(|o| o.get("fmp")?.get(key).cloned())
            .filter(|v| !v.is_null())
            .or_else(|| base.get("fmp")?.get(key).cloned())
    };
    let mut keys = Vec::new();
    if let Some(serde_yaml::Value::String(k)) = pick("api_key") {
        keys.push(k);
    }
    if let Some(serde_yaml::Value::Sequence(list)) = pick("api_keys") {
        keys.extend(
            list.into_iter()
                .filter_map(|v| v.as_str().map(str::to_string)),
        );
    }
    let base_url = pick("base_url").and_then(|v| v.as_str().map(str::to_string));
    let timeout = pick("timeout").and_then(|v| v.as_u64());
    Ok((dedupe_keys(keys), base_url, timeout))
}

fn dedupe_keys(keys: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for key in keys {
        let key = key.trim().to_string();
        if !key.is_empty() && !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

fn market_config() -> Result<MarketConfig> {
    let source = match var("HONE_QUANT_MARKET_DATA").as_deref().unwrap_or("fmp") {
        "fmp" => DataSource::Fmp,
        "demo" => DataSource::Demo,
        other => bail!("HONE_QUANT_MARKET_DATA must be 'fmp' or 'demo', got {other:?}"),
    };
    let mut fmp = FmpConfig::default();
    let mut key_origin = String::from("none");
    let mut keys = Vec::new();
    if let Some(list) = var("HONE_QUANT_FMP_API_KEYS") {
        keys.extend(list.split(',').map(str::to_string));
        key_origin = "HONE_QUANT_FMP_API_KEYS".into();
    }
    if let Some(key) = var("HONE_QUANT_FMP_API_KEY") {
        keys.insert(0, key);
        key_origin = "HONE_QUANT_FMP_API_KEY".into();
    }
    if keys.is_empty()
        && let Some(path) = var("HONE_QUANT_HONECLAW_CONFIG")
    {
        let (from_honeclaw, base_url, timeout) = fmp_from_honeclaw(Path::new(&path))?;
        keys = from_honeclaw;
        if let Some(url) = base_url {
            fmp.base_url = url;
        }
        if let Some(t) = timeout {
            fmp.timeout_secs = t.clamp(5, 120);
        }
        key_origin = format!("honeclaw config ({path})");
    }
    fmp.api_keys = dedupe_keys(keys);
    if let Some(url) = var("HONE_QUANT_FMP_BASE_URL") {
        fmp.base_url = url;
    }
    fmp.timeout_secs = parse_num("HONE_QUANT_FMP_TIMEOUT_SECS", fmp.timeout_secs)?.clamp(5, 120);
    fmp.api_mode = match var("HONE_QUANT_FMP_API").as_deref().unwrap_or("auto") {
        "auto" => ApiMode::Auto,
        "stable" => ApiMode::Stable,
        "legacy" => ApiMode::Legacy,
        other => bail!("HONE_QUANT_FMP_API must be auto, stable or legacy, got {other:?}"),
    };
    fmp.requests_per_minute =
        parse_num("HONE_QUANT_FMP_RPM", fmp.requests_per_minute)?.clamp(10, 3000);
    if source == DataSource::Fmp && fmp.api_keys.is_empty() {
        bail!(
            "No FMP API key configured. Set HONE_QUANT_FMP_API_KEYS (comma-separated) or HONE_QUANT_HONECLAW_CONFIG=/path/to/honeclaw/config.yaml. To evaluate without a key, run with HONE_QUANT_MARKET_DATA=demo (synthetic data, separate schema)."
        );
    }
    Ok(MarketConfig {
        source,
        fmp,
        demo_seed: parse_num("HONE_QUANT_DEMO_SEED", 20_260_101u64)?,
        key_origin,
    })
}

fn secret_key(state_dir: &Path) -> Result<[u8; 32]> {
    if let Some(material) = var("HONE_QUANT_SECRET_KEY") {
        return Ok(Sha256::digest(material.as_bytes()).into());
    }
    let path = state_dir.join("secret.key");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        return Ok(Sha256::digest(existing.trim().as_bytes()).into());
    }
    std::fs::create_dir_all(state_dir)
        .with_context(|| format!("cannot create state directory {}", state_dir.display()))?;
    let mut bytes = [0u8; 32];
    rand::Rng::fill(&mut rand::rngs::OsRng, &mut bytes);
    let material = hex::encode(bytes);
    write_private_file(&path, &material)?;
    tracing::info!(path = %path.display(), "generated a new secret key for stored credentials");
    Ok(Sha256::digest(material.as_bytes()).into())
}

fn write_private_file(path: &Path, contents: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("cannot create {}", path.display()))?;
        file.write_all(contents.as_bytes())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents).with_context(|| format!("cannot write {}", path.display()))
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let market = market_config()?;
        let mut db = database_config(market.source)?;
        let bind: SocketAddr = var("HONE_QUANT_BIND")
            .unwrap_or_else(|| DEFAULT_BIND.into())
            .parse()
            .context("HONE_QUANT_BIND must be host:port, e.g. 127.0.0.1:8090")?;
        let state_dir =
            PathBuf::from(var("HONE_QUANT_STATE_DIR").unwrap_or_else(|| "./data".into()));
        let secret_key = secret_key(&state_dir)?;
        let cookie_security = match var("HONE_QUANT_SECURE_COOKIE").as_deref().unwrap_or("auto") {
            "auto" => CookieSecurity::Auto,
            "true" | "1" | "always" => CookieSecurity::Always,
            "false" | "0" | "never" => CookieSecurity::Never,
            other => bail!("HONE_QUANT_SECURE_COOKIE must be auto, true or false, got {other:?}"),
        };
        let bootstrap_admin = match (
            var("HONE_QUANT_ADMIN_USER"),
            var("HONE_QUANT_ADMIN_PASSWORD"),
        ) {
            (Some(user), Some(password)) => Some((user, password)),
            (None, Some(password)) => Some(("admin".to_string(), password)),
            _ => None,
        };
        let log_format = match var("HONE_QUANT_LOG_FORMAT").as_deref().unwrap_or("text") {
            "json" => LogFormat::Json,
            _ => LogFormat::Text,
        };
        let extra_closures = var("HONE_QUANT_EXTRA_CLOSURES")
            .map(|list| {
                list.split(',')
                    .map(|d| {
                        NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d")
                            .with_context(|| format!("HONE_QUANT_EXTRA_CLOSURES: bad date {d:?}"))
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        let dev_clock_start = var("HONE_QUANT_DEV_CLOCK")
            .map(|v| {
                DateTime::parse_from_rfc3339(&v)
                    .map(|d| d.with_timezone(&Utc))
                    .context("HONE_QUANT_DEV_CLOCK must be RFC 3339")
            })
            .transpose()?;
        if dev_clock_start.is_some() && market.source != DataSource::Demo {
            bail!("HONE_QUANT_DEV_CLOCK is only allowed with HONE_QUANT_MARKET_DATA=demo");
        }
        let dev_clock_offset = dev_clock_start.map(|start| start - Utc::now());
        if let Some(offset) = dev_clock_offset {
            db.set_clock_offset(offset);
        }
        let initial_cash = parse_num("HONE_QUANT_INITIAL_CASH", 1_000_000.0f64)?;
        if !(1_000.0..=1e10).contains(&initial_cash) {
            bail!("HONE_QUANT_INITIAL_CASH must be between 1,000 and 10,000,000,000");
        }
        Ok(Self {
            bind,
            db,
            market,
            secret_key,
            state_dir,
            cookie_security,
            public_url: var("HONE_QUANT_PUBLIC_URL").map(|u| u.trim_end_matches('/').to_string()),
            bootstrap_admin,
            log_format,
            universe_path: var("HONE_QUANT_UNIVERSE_PATH").map(PathBuf::from),
            web_dir: var("HONE_QUANT_WEB_DIR").map(PathBuf::from),
            scheduler_enabled: parse_bool("HONE_QUANT_SCHEDULER", true)?,
            extra_closures,
            dev_clock_start,
            dev_clock_offset,
            initial_cash,
            base_path: normalize_base_path(&var("HONE_QUANT_BASE_PATH").unwrap_or_default())?,
            auth: auth_mode()?,
            origin_token: origin_token()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_paths_are_normalised() {
        assert_eq!(normalize_base_path("").unwrap(), "");
        assert_eq!(normalize_base_path("/").unwrap(), "");
        assert_eq!(normalize_base_path(" /quant/ ").unwrap(), "/quant");
        assert_eq!(normalize_base_path("/apps/quant").unwrap(), "/apps/quant");
        for bad in ["quant", "/a//b", "/../x", "/q?x=1", "/q t", "/quant/%2e%2e"] {
            assert!(
                normalize_base_path(bad).is_err(),
                "{bad} should be rejected"
            );
        }
    }

    #[test]
    fn honeclaw_auth_url_must_be_https_or_loopback() {
        assert!(check_auth_url("https://hone-claw.com/api/public/auth/me").is_ok());
        assert!(check_auth_url("http://127.0.0.1:8080/api/public/auth/me").is_ok());
        assert!(check_auth_url("http://localhost:8080/api/public/auth/me").is_ok());
        assert!(check_auth_url("http://hone-claw.com/api/public/auth/me").is_err());
        assert!(check_auth_url("ftp://hone-claw.com/").is_err());
        assert!(check_auth_url("not a url").is_err());
    }

    #[test]
    fn schema_names_are_validated() {
        assert!(valid_schema_name("hone_quant"));
        assert!(valid_schema_name("_x1"));
        assert!(!valid_schema_name("Hone"));
        assert!(!valid_schema_name("1abc"));
        assert!(!valid_schema_name("a;drop"));
        assert!(!valid_schema_name(""));
    }

    #[test]
    fn honeclaw_fmp_block_is_read_with_overrides() {
        let dir = std::env::temp_dir().join(format!("hq-cfg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.yaml");
        std::fs::write(
            &config,
            "fmp:\n  api_key: \"legacy-key\"\n  api_keys: [\"k2\", \"legacy-key\", \"\"]\n  base_url: \"https://financialmodelingprep.com/api\"\n  timeout: 60\n",
        )
        .unwrap();
        let (keys, url, timeout) = fmp_from_honeclaw(&config).unwrap();
        assert_eq!(keys, vec!["legacy-key".to_string(), "k2".to_string()]);
        assert_eq!(
            url.as_deref(),
            Some("https://financialmodelingprep.com/api")
        );
        assert_eq!(timeout, Some(60));
        std::fs::write(
            dir.join("config.overrides.yaml"),
            "fmp:\n  api_keys: [\"override\"]\n",
        )
        .unwrap();
        let (keys, _, _) = fmp_from_honeclaw(&config).unwrap();
        assert_eq!(keys, vec!["legacy-key".to_string(), "override".to_string()]);
        std::fs::remove_dir_all(dir).ok();
    }
}
