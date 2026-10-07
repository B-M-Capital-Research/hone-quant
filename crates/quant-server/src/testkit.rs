//! Integration-test helpers: an [`AppState`] on a throw-away schema with the demo market and a
//! clock frozen at a chosen instant.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use quant_core::calendar::MarketCalendar;

use crate::auth::LoginLimiter;
use crate::config::{AuthMode, Config, CookieSecurity, DbConfig, LogFormat, MarketConfig};
use crate::crypto::SecretBox;
use crate::db::testing::TestDb;
use crate::market::DataSource;
use crate::market::demo::{DemoInstrument, DemoMarket};
use crate::state::{AppState, Clock};
use crate::universe;

/// A demo-market state whose clock always reads `now` (and whose database clock, `app_now()`,
/// starts there too); `None` when no test database is set.
pub async fn state_at(now: DateTime<Utc>) -> Option<(TestDb, Arc<AppState>)> {
    let db = TestDb::new(DataSource::Demo).await?;
    let clock: Clock = Arc::new(move || now);
    let url = std::env::var("HONE_QUANT_TEST_DATABASE_URL").ok()?;
    let mut db_config = DbConfig {
        pg: url.parse().expect("valid test URL"),
        schema: db.schema.clone(),
        pool_size: 4,
        display: "test".into(),
        borrowed_from_honeclaw: false,
    };
    db_config.set_clock_offset(now - Utc::now());
    let pool = crate::db::create_pool(&db_config).expect("pool");
    let file = universe::load(None).expect("bundled universe");
    let instruments: Vec<DemoInstrument> = file
        .assets
        .iter()
        .map(|a| DemoInstrument {
            symbol: a.symbol.clone(),
            sector: a.sector.clone(),
        })
        .chain(file.benchmarks.iter().map(|b| DemoInstrument {
            symbol: b.symbol.clone(),
            sector: "benchmark".into(),
        }))
        .collect();
    let config = Config {
        bind: "127.0.0.1:0".parse().expect("address"),
        db: db_config,
        market: MarketConfig {
            source: DataSource::Demo,
            fmp: Default::default(),
            demo_seed: 7,
            key_origin: String::new(),
        },
        state_dir: std::env::temp_dir(),
        secret_key: [7; 32],
        cookie_security: CookieSecurity::Never,
        public_url: None,
        bootstrap_admin: None,
        log_format: LogFormat::Text,
        universe_path: None,
        web_dir: None,
        scheduler_enabled: false,
        extra_closures: Vec::new(),
        dev_clock_start: None,
        dev_clock_offset: None,
        initial_cash: 1_000_000.0,
        base_path: String::new(),
        auth: AuthMode::Local,
        origin_token: None,
    };
    let (events, _) = tokio::sync::broadcast::channel(256);
    let state = Arc::new(AppState {
        secrets: SecretBox::new(&config.secret_key),
        config: Arc::new(config),
        pool,
        market: Arc::new(DemoMarket::new(instruments, 7, clock.clone())),
        calendar: Arc::new(MarketCalendar::nyse()),
        clock,
        events,
        http: reqwest::Client::new(),
        limiter: LoginLimiter::default(),
        honeclaw: None,
        trading_lock: tokio::sync::Mutex::new(()),
        started_at: now,
    });
    Some((db, state))
}
