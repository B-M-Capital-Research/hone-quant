//! Typed operator settings stored as JSON documents in the `settings` table.
//!
//! Each section has a Rust type with defaults, so a missing or partially-written row always
//! resolves to a complete, valid configuration, and every write is validated before it lands.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use chrono::{DateTime, NaiveTime, Utc};
use deadpool_postgres::GenericClient;
use quant_core::costs::CostModel;
use quant_core::schedule::ScheduleSettings;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::notify::channels::Severity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationMode {
    /// Plans execute automatically after the review window.
    Auto,
    /// Plans wait for an operator's approval until their deadline.
    Approval,
    /// No plans are generated.
    Paused,
}

impl AutomationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AutomationMode::Auto => "auto",
            AutomationMode::Approval => "approval",
            AutomationMode::Paused => "paused",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomationSettings {
    pub mode: AutomationMode,
    /// While set and in the future, automation behaves as paused; afterwards `mode` applies.
    pub paused_until: Option<DateTime<Utc>>,
    pub note: String,
}

impl Default for AutomationSettings {
    fn default() -> Self {
        Self {
            mode: AutomationMode::Auto,
            paused_until: None,
            note: String::new(),
        }
    }
}

impl AutomationSettings {
    pub fn effective_mode(&self, now: DateTime<Utc>) -> AutomationMode {
        match self.paused_until {
            Some(until) if until > now => AutomationMode::Paused,
            _ => self.mode,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecutionSettings {
    pub costs: CostModel,
    /// Quotes older than this (during the session) are treated as stale.
    pub max_quote_age_secs: u64,
    /// An order is rejected when the execution quote moved more than this from its plan price.
    pub max_price_deviation: f64,
    /// A name is frozen for the plan when its price moved more than this from the last close
    /// (protects against bad prints and unprocessed splits).
    pub max_daily_move: f64,
    /// Quote polling interval during the regular session.
    pub quote_poll_secs: u64,
}

impl Default for ExecutionSettings {
    fn default() -> Self {
        Self {
            costs: CostModel::default(),
            max_quote_age_secs: 300,
            max_price_deviation: 0.03,
            max_daily_move: 0.40,
            quote_poll_secs: 60,
        }
    }
}

impl ExecutionSettings {
    pub fn validate(&self) -> Result<()> {
        self.costs.validate().map_err(anyhow::Error::msg)?;
        if !(30..=3600).contains(&self.max_quote_age_secs) {
            bail!("max_quote_age_secs must be between 30 and 3600");
        }
        if !(0.002..=0.2).contains(&self.max_price_deviation) {
            bail!("max_price_deviation must be between 0.2% and 20%");
        }
        if !(0.05..=0.9).contains(&self.max_daily_move) {
            bail!("max_daily_move must be between 5% and 90%");
        }
        if !(15..=900).contains(&self.quote_poll_secs) {
            bail!("quote_poll_secs must be between 15 and 900");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RiskSettings {
    /// Alert when NAV is this far below its running peak.
    pub drawdown_alert: f64,
    /// Alert when NAV is down this much since the previous close.
    pub daily_loss_alert: f64,
}

impl Default for RiskSettings {
    fn default() -> Self {
        Self {
            drawdown_alert: 0.10,
            daily_loss_alert: 0.03,
        }
    }
}

impl RiskSettings {
    pub fn validate(&self) -> Result<()> {
        if !(0.01..=0.9).contains(&self.drawdown_alert)
            || !(0.005..=0.5).contains(&self.daily_loss_alert)
        {
            bail!("risk thresholds out of range");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    Zh,
    En,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuietHours {
    pub enabled: bool,
    /// Local wall-clock "HH:MM".
    pub start: String,
    pub end: String,
    pub timezone: String,
}

impl Default for QuietHours {
    fn default() -> Self {
        Self {
            enabled: false,
            start: "23:30".into(),
            end: "07:30".into(),
            timezone: "Asia/Singapore".into(),
        }
    }
}

impl QuietHours {
    pub fn validate(&self) -> Result<()> {
        NaiveTime::parse_from_str(&self.start, "%H:%M")?;
        NaiveTime::parse_from_str(&self.end, "%H:%M")?;
        self.timezone
            .parse::<chrono_tz::Tz>()
            .map_err(|_| anyhow::anyhow!("unknown time zone {}", self.timezone))?;
        Ok(())
    }

    /// Whether `now` falls inside the quiet window (which may wrap past midnight).
    pub fn contains(&self, now: DateTime<Utc>) -> bool {
        if !self.enabled {
            return false;
        }
        let (Ok(start), Ok(end), Ok(tz)) = (
            NaiveTime::parse_from_str(&self.start, "%H:%M"),
            NaiveTime::parse_from_str(&self.end, "%H:%M"),
            self.timezone.parse::<chrono_tz::Tz>(),
        ) else {
            return false;
        };
        let local = now.with_timezone(&tz).time();
        if start <= end {
            local >= start && local < end
        } else {
            local >= start || local < end
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationSettings {
    /// Language of outbound messages.
    pub language: Lang,
    pub quiet_hours: QuietHours,
    /// Critical notifications always go out, even inside quiet hours.
    pub critical_bypasses_quiet_hours: bool,
    /// Outbound delivery per category (plan, execution, risk, system, reminder, report, data).
    pub categories: BTreeMap<String, bool>,
    pub min_severity: Severity,
    /// Browser notifications in the web UI while it is open.
    pub browser: bool,
}

pub const CATEGORIES: [&str; 7] = [
    "plan",
    "execution",
    "risk",
    "system",
    "reminder",
    "report",
    "data",
];

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            language: Lang::Zh,
            quiet_hours: QuietHours::default(),
            critical_bypasses_quiet_hours: true,
            categories: CATEGORIES.iter().map(|c| (c.to_string(), true)).collect(),
            min_severity: Severity::Info,
            browser: true,
        }
    }
}

impl NotificationSettings {
    pub fn category_enabled(&self, category: &str) -> bool {
        self.categories.get(category).copied().unwrap_or(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpColor {
    /// US convention: green up, red down.
    GreenUp,
    /// China/HK convention: red up, green down.
    RedUp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaySettings {
    /// Time zone shown next to exchange time (New York) throughout the UI.
    pub timezone: String,
    pub up_color: UpColor,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            timezone: "Asia/Singapore".into(),
            up_color: UpColor::GreenUp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BenchmarkSettings {
    pub symbols: Vec<String>,
    pub primary: String,
    pub risk_free_rate: f64,
}

impl Default for BenchmarkSettings {
    fn default() -> Self {
        Self {
            symbols: vec!["SPY".into(), "QQQ".into(), "SMH".into()],
            primary: "QQQ".into(),
            risk_free_rate: 0.04,
        }
    }
}

impl BenchmarkSettings {
    pub fn validate(&self) -> Result<()> {
        if self.symbols.is_empty() || self.symbols.len() > 6 {
            bail!("choose between one and six benchmarks");
        }
        if !self.symbols.contains(&self.primary) {
            bail!("the primary benchmark must be one of the benchmarks");
        }
        if !(0.0..=0.2).contains(&self.risk_free_rate) {
            bail!("risk_free_rate must be between 0% and 20%");
        }
        Ok(())
    }
}

/// A configured outbound channel; the channel config itself is stored encrypted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredChannel {
    pub kind: String,
    pub label: String,
    pub enabled: bool,
    /// `SecretBox::seal` of the JSON-encoded `ChannelConfig`.
    pub sealed: String,
    pub updated_at: DateTime<Utc>,
}

pub type ChannelMap = BTreeMap<String, StoredChannel>;

pub const SCHEDULE: &str = "schedule";
pub const AUTOMATION: &str = "automation";
pub const EXECUTION: &str = "execution";
pub const RISK: &str = "risk";
pub const NOTIFICATIONS: &str = "notifications";
pub const DISPLAY: &str = "display";
pub const BENCHMARKS: &str = "benchmarks";
pub const CHANNELS: &str = "channels";

pub async fn get<T: DeserializeOwned + Default>(
    client: &impl GenericClient,
    key: &str,
) -> Result<T> {
    let row = client
        .query_opt("SELECT value FROM settings WHERE key = $1", &[&key])
        .await?;
    Ok(match row {
        Some(row) => {
            let value: serde_json::Value = row.get(0);
            serde_json::from_value(value).unwrap_or_else(|error| {
                tracing::warn!(key, %error, "stored settings unreadable; using defaults");
                T::default()
            })
        }
        None => T::default(),
    })
}

pub async fn put<T: Serialize>(
    client: &impl GenericClient,
    key: &str,
    value: &T,
    actor: &str,
) -> Result<()> {
    client
        .execute(
            "INSERT INTO settings (key, value, updated_at, updated_by) VALUES ($1, $2, app_now(), $3)
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = app_now(), updated_by = EXCLUDED.updated_by",
            &[&key, &serde_json::to_value(value)?, &actor],
        )
        .await?;
    Ok(())
}

pub async fn schedule(client: &impl GenericClient) -> Result<ScheduleSettings> {
    let value: ScheduleSettings = get(client, SCHEDULE).await?;
    Ok(if value.validate().is_ok() {
        value
    } else {
        ScheduleSettings::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_hours_wrap_midnight_in_singapore() {
        let quiet = QuietHours {
            enabled: true,
            ..QuietHours::default()
        };
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        // 01:00 SGT = 17:00 UTC (previous day).
        assert!(quiet.contains(at("2026-10-05T17:00:00Z")));
        // 12:00 SGT = 04:00 UTC.
        assert!(!quiet.contains(at("2026-10-05T04:00:00Z")));
        // 23:45 SGT = 15:45 UTC.
        assert!(quiet.contains(at("2026-10-05T15:45:00Z")));
        let disabled = QuietHours::default();
        assert!(!disabled.contains(at("2026-10-05T17:00:00Z")));
    }

    #[test]
    fn paused_until_overrides_mode() {
        let now = Utc::now();
        let settings = AutomationSettings {
            mode: AutomationMode::Auto,
            paused_until: Some(now + chrono::Duration::hours(1)),
            note: String::new(),
        };
        assert_eq!(settings.effective_mode(now), AutomationMode::Paused);
        assert_eq!(
            settings.effective_mode(now + chrono::Duration::hours(2)),
            AutomationMode::Auto
        );
    }

    #[test]
    fn defaults_validate() {
        assert!(ExecutionSettings::default().validate().is_ok());
        assert!(RiskSettings::default().validate().is_ok());
        assert!(BenchmarkSettings::default().validate().is_ok());
        assert!(QuietHours::default().validate().is_ok());
    }
}
