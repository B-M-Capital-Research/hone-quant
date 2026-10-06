//! Shared application state.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use quant_core::calendar::MarketCalendar;
use serde::Serialize;
use tokio::sync::{Mutex, broadcast};

use crate::auth::LoginLimiter;
use crate::config::Config;
use crate::crypto::SecretBox;
use crate::honeclaw_auth::HoneclawVerifier;
use crate::market::MarketData;

pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// Real-time events pushed to connected browsers over SSE. Payloads are small hints; the UI
/// refetches the affected resources, so there is a single source of truth (the API).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Quotes {
        at: DateTime<Utc>,
    },
    Plan {
        plan_id: i64,
        status: String,
    },
    Account {
        reason: String,
    },
    Notification {
        id: i64,
        severity: String,
        category: String,
        title_zh: String,
        title_en: String,
    },
    Backtest {
        id: i64,
        status: String,
    },
    Settings {
        key: String,
    },
    Strategy {
        version_id: i64,
    },
    Universe,
}

pub struct AppState {
    pub config: Arc<Config>,
    pub pool: Pool,
    pub market: Arc<dyn MarketData>,
    pub calendar: Arc<MarketCalendar>,
    pub clock: Clock,
    pub events: broadcast::Sender<ServerEvent>,
    /// Outbound HTTP for notification channels.
    pub http: reqwest::Client,
    pub secrets: SecretBox,
    pub limiter: LoginLimiter,
    /// Present in honeclaw sign-in mode: checks honeclaw sessions for every protected request.
    pub honeclaw: Option<Arc<HoneclawVerifier>>,
    /// Serialises everything that mutates the paper account (plan generation, execution,
    /// corporate actions, resets) so two of them can never interleave.
    pub trading_lock: Mutex<()>,
    pub started_at: DateTime<Utc>,
}

pub type SharedState = Arc<AppState>;

impl AppState {
    pub fn now(&self) -> DateTime<Utc> {
        (self.clock)()
    }

    pub fn emit(&self, event: ServerEvent) {
        // No subscribers is fine.
        let _ = self.events.send(event);
    }
}

pub fn system_clock() -> Clock {
    Arc::new(Utc::now)
}

/// The wall clock shifted by a fixed offset, advancing in real time (demo mode only).
pub fn offset_clock(offset: chrono::Duration) -> Clock {
    Arc::new(move || Utc::now() + offset)
}
