//! Notifications, reminders, settings, notification channels, audit trail, jobs and data
//! status.

use axum::Json;
use axum::extract::{Path, Query, State};
use quant_core::schedule::ScheduleSettings;
use serde::Deserialize;
use serde_json::{Value, json};

use super::error::{ApiError, ApiResult};
use crate::auth::{AdminUser, CurrentUser};
use crate::notify::{self, channels};
use crate::services::marketdata;
use crate::services::reminders::{self, Schedule};
use crate::state::{ServerEvent, SharedState};
use crate::store::settings::{
    self, AutomationSettings, BenchmarkSettings, ChannelMap, DisplaySettings, ExecutionSettings,
    NotificationSettings, RiskSettings, StoredChannel,
};
use crate::store::{market, system};

#[derive(Deserialize)]
pub struct NotificationQuery {
    unread: Option<bool>,
    category: Option<String>,
    before: Option<i64>,
    limit: Option<i64>,
}

pub async fn notifications(
    State(state): State<SharedState>,
    _user: CurrentUser,
    Query(q): Query<NotificationQuery>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let items = system::notifications(
        &client,
        &system::NotificationFilter {
            unread_only: q.unread.unwrap_or(false),
            category: q.category.filter(|c| !c.is_empty()),
            before: q.before,
            limit: q.limit.unwrap_or(50).clamp(1, 200),
        },
    )
    .await?;
    Ok(Json(
        json!({"notifications": items, "unread": system::unread_count(&client).await?}),
    ))
}

pub async fn read_one(
    State(state): State<SharedState>,
    _user: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    system::mark_read(&client, Some(id)).await?;
    Ok(Json(
        json!({"unread": system::unread_count(&client).await?}),
    ))
}

pub async fn read_all(
    State(state): State<SharedState>,
    _user: CurrentUser,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    system::mark_read(&client, None).await?;
    Ok(Json(json!({"unread": 0})))
}

// ---------------------------------------------------------------------------------------------
// Reminders
// ---------------------------------------------------------------------------------------------

pub async fn reminders(
    State(state): State<SharedState>,
    _user: CurrentUser,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    Ok(Json(
        json!({"reminders": system::reminders(&client).await?}),
    ))
}

#[derive(Deserialize)]
pub struct ReminderBody {
    #[serde(default)]
    title: String,
    #[serde(default)]
    note: String,
    schedule: Schedule,
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

pub async fn create_reminder(
    State(state): State<SharedState>,
    AdminUser(user): AdminUser,
    Json(body): Json<ReminderBody>,
) -> ApiResult<Json<Value>> {
    body.schedule
        .validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    if !body.schedule.is_custom() {
        return Err(ApiError::bad(
            "custom reminders need a once, daily, trading_days or weekly schedule",
        ));
    }
    let title = body.title.trim();
    if title.is_empty() || title.chars().count() > 120 {
        return Err(ApiError::bad("title must be 1–120 characters"));
    }
    let next = reminders::next_fire(&body.schedule, state.now(), &state.calendar);
    let client = state.pool.get().await?;
    let reminder = system::insert_reminder(
        &client,
        title,
        body.note.trim(),
        &serde_json::to_value(&body.schedule).map_err(anyhow::Error::from)?,
        body.enabled && next.is_some(),
        next,
        &user.username,
    )
    .await?;
    system::audit(
        &client,
        &user.username,
        "reminder.created",
        "reminder",
        &reminder.id.to_string(),
        json!(reminder),
        &user.ip,
    )
    .await?;
    Ok(Json(json!({"reminder": reminder})))
}

pub async fn update_reminder(
    State(state): State<SharedState>,
    AdminUser(user): AdminUser,
    Path(id): Path<i64>,
    Json(body): Json<ReminderBody>,
) -> ApiResult<Json<Value>> {
    body.schedule
        .validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let client = state.pool.get().await?;
    let existing = system::reminder(&client, id)
        .await?
        .ok_or_else(|| ApiError::not_found("reminder"))?;
    let builtin = existing.kind != "custom";
    if builtin {
        let same_kind = std::mem::discriminant(&body.schedule)
            == std::mem::discriminant(
                &serde_json::from_value::<Schedule>(existing.schedule.clone())
                    .map_err(anyhow::Error::from)?,
            );
        if !same_kind {
            return Err(ApiError::bad("built-in reminders keep their schedule type"));
        }
    } else if !body.schedule.is_custom() {
        return Err(ApiError::bad(
            "custom reminders need a once, daily, trading_days or weekly schedule",
        ));
    }
    let next = if builtin {
        None
    } else {
        reminders::next_fire(&body.schedule, state.now(), &state.calendar)
    };
    let title = if builtin {
        existing.title.clone()
    } else {
        body.title.trim().to_string()
    };
    let updated = system::update_reminder(
        &client,
        id,
        &title,
        body.note.trim(),
        &serde_json::to_value(&body.schedule).map_err(anyhow::Error::from)?,
        body.enabled,
        next,
    )
    .await?
    .ok_or_else(|| ApiError::not_found("reminder"))?;
    system::audit(
        &client,
        &user.username,
        "reminder.updated",
        "reminder",
        &id.to_string(),
        json!({"before": existing, "after": updated}),
        &user.ip,
    )
    .await?;
    Ok(Json(json!({"reminder": updated})))
}

pub async fn delete_reminder(
    State(state): State<SharedState>,
    AdminUser(user): AdminUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    if !system::delete_reminder(&client, id).await? {
        return Err(ApiError::bad(
            "only custom reminders can be deleted (switch built-ins off instead)",
        ));
    }
    system::audit(
        &client,
        &user.username,
        "reminder.deleted",
        "reminder",
        &id.to_string(),
        json!({}),
        &user.ip,
    )
    .await?;
    Ok(Json(json!({"ok": true})))
}

// ---------------------------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------------------------

pub async fn settings(
    State(state): State<SharedState>,
    _user: CurrentUser,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let schedule: ScheduleSettings = settings::schedule(&client).await?;
    let automation: AutomationSettings = settings::get(&client, settings::AUTOMATION).await?;
    let execution: ExecutionSettings = settings::get(&client, settings::EXECUTION).await?;
    let risk: RiskSettings = settings::get(&client, settings::RISK).await?;
    let notifications: NotificationSettings =
        settings::get(&client, settings::NOTIFICATIONS).await?;
    let display: DisplaySettings = settings::get(&client, settings::DISPLAY).await?;
    let benchmarks: BenchmarkSettings = settings::get(&client, settings::BENCHMARKS).await?;
    Ok(Json(json!({
        "schedule": schedule,
        "automation": automation,
        "execution": execution,
        "risk": risk,
        "notifications": notifications,
        "display": display,
        "benchmarks": benchmarks,
        "defaults": {
            "schedule": ScheduleSettings::default(),
            "execution": ExecutionSettings::default(),
            "risk": RiskSettings::default(),
            "notifications": NotificationSettings::default(),
            "display": DisplaySettings::default(),
            "benchmarks": BenchmarkSettings::default(),
        },
    })))
}

async fn store_section<T: serde::Serialize + serde::de::DeserializeOwned + Default>(
    state: &SharedState,
    user: &crate::auth::CurrentUser,
    key: &str,
    value: &T,
) -> ApiResult<()> {
    let client = state.pool.get().await?;
    let before: T = settings::get(&client, key).await?;
    settings::put(&client, key, value, &user.username).await?;
    system::audit(
        &client,
        &user.username,
        &format!("settings.{key}"),
        "settings",
        key,
        json!({"before": before, "after": value}),
        &user.ip,
    )
    .await?;
    state.emit(ServerEvent::Settings {
        key: key.to_string(),
    });
    Ok(())
}

pub async fn put_settings(
    State(state): State<SharedState>,
    AdminUser(user): AdminUser,
    Path(section): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    fn parse<T: serde::de::DeserializeOwned>(body: Value) -> ApiResult<T> {
        serde_json::from_value(body).map_err(|e| ApiError::BadRequest(e.to_string()))
    }
    match section.as_str() {
        "schedule" => {
            let v: ScheduleSettings = parse(body)?;
            v.validate().map_err(ApiError::BadRequest)?;
            store_section(&state, &user, settings::SCHEDULE, &v).await?;
        }
        "execution" => {
            let v: ExecutionSettings = parse(body)?;
            v.validate()
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            store_section(&state, &user, settings::EXECUTION, &v).await?;
        }
        "risk" => {
            let v: RiskSettings = parse(body)?;
            v.validate()
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            store_section(&state, &user, settings::RISK, &v).await?;
        }
        "notifications" => {
            let v: NotificationSettings = parse(body)?;
            v.quiet_hours
                .validate()
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            store_section(&state, &user, settings::NOTIFICATIONS, &v).await?;
        }
        "display" => {
            let v: DisplaySettings = parse(body)?;
            v.timezone
                .parse::<chrono_tz::Tz>()
                .map_err(|_| ApiError::bad("unknown time zone"))?;
            store_section(&state, &user, settings::DISPLAY, &v).await?;
        }
        "benchmarks" => {
            let mut v: BenchmarkSettings = parse(body)?;
            v.symbols = v
                .symbols
                .iter()
                .map(|s| s.trim().to_ascii_uppercase())
                .collect();
            v.primary = v.primary.trim().to_ascii_uppercase();
            v.validate()
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            store_section(&state, &user, settings::BENCHMARKS, &v).await?;
            let s = state.clone();
            tokio::spawn(async move {
                let _ = marketdata::sync_daily(&s, 7, false).await;
            });
        }
        _ => return Err(ApiError::not_found("settings section")),
    }
    Ok(Json(json!({"ok": true})))
}

// ---------------------------------------------------------------------------------------------
// Notification channels
// ---------------------------------------------------------------------------------------------

pub async fn channels(
    State(state): State<SharedState>,
    _user: CurrentUser,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let stored: ChannelMap = settings::get(&client, settings::CHANNELS).await?;
    let mut out = Vec::new();
    for (name, channel) in stored {
        let config = state
            .secrets
            .open(&channel.sealed)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<channels::ChannelConfig>(&bytes).ok())
            .map(|c| channels::redacted(&c));
        out.push(json!({
            "name": name,
            "kind": channel.kind,
            "label": channel.label,
            "enabled": channel.enabled,
            "config": config,
            "readable": config.is_some(),
            "updated_at": channel.updated_at,
        }));
    }
    Ok(Json(json!({"channels": out})))
}

#[derive(Deserialize)]
pub struct ChannelBody {
    label: String,
    enabled: bool,
    config: channels::ChannelConfig,
}

fn valid_channel_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub async fn put_channel(
    State(state): State<SharedState>,
    AdminUser(user): AdminUser,
    Path(name): Path<String>,
    Json(body): Json<ChannelBody>,
) -> ApiResult<Json<Value>> {
    if !valid_channel_name(&name) {
        return Err(ApiError::bad(
            "channel names use letters, digits, '-' and '_' (max 40)",
        ));
    }
    let client = state.pool.get().await?;
    let mut stored: ChannelMap = settings::get(&client, settings::CHANNELS).await?;
    let existing = stored
        .get(&name)
        .and_then(|c| state.secrets.open(&c.sealed).ok())
        .and_then(|bytes| serde_json::from_slice::<channels::ChannelConfig>(&bytes).ok());
    let config = channels::merge_secrets(body.config, existing.as_ref());
    channels::validate(&config).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let sealed = state
        .secrets
        .seal(&serde_json::to_vec(&config).map_err(anyhow::Error::from)?);
    stored.insert(
        name.clone(),
        StoredChannel {
            kind: channels::kind(&config).to_string(),
            label: body.label.trim().chars().take(60).collect(),
            enabled: body.enabled,
            sealed,
            updated_at: state.now(),
        },
    );
    settings::put(&client, settings::CHANNELS, &stored, &user.username).await?;
    system::audit(
        &client,
        &user.username,
        "channel.saved",
        "channel",
        &name,
        json!({"kind": channels::kind(&config), "enabled": body.enabled, "config": channels::redacted(&config)}),
        &user.ip,
    )
    .await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn delete_channel(
    State(state): State<SharedState>,
    AdminUser(user): AdminUser,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let mut stored: ChannelMap = settings::get(&client, settings::CHANNELS).await?;
    if stored.remove(&name).is_none() {
        return Err(ApiError::not_found("channel"));
    }
    settings::put(&client, settings::CHANNELS, &stored, &user.username).await?;
    system::audit(
        &client,
        &user.username,
        "channel.deleted",
        "channel",
        &name,
        json!({}),
        &user.ip,
    )
    .await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn test_channel(
    State(state): State<SharedState>,
    AdminUser(user): AdminUser,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let stored: ChannelMap = settings::get(&client, settings::CHANNELS).await?;
    let prefs: NotificationSettings = settings::get(&client, settings::NOTIFICATIONS).await?;
    let channel = stored
        .get(&name)
        .ok_or_else(|| ApiError::not_found("channel"))?;
    let config: channels::ChannelConfig = serde_json::from_slice(
        &state
            .secrets
            .open(&channel.sealed)
            .map_err(|e| ApiError::BadRequest(e.to_string()))?,
    )
    .map_err(anyhow::Error::from)?;
    drop(client);
    let result = notify::send_test(&state, &config, prefs.language).await;
    let client = state.pool.get().await?;
    system::audit(
        &client,
        &user.username,
        "channel.tested",
        "channel",
        &name,
        json!({"ok": result.is_ok(), "error": result.as_ref().err()}),
        &user.ip,
    )
    .await?;
    Ok(Json(json!({"ok": result.is_ok(), "error": result.err()})))
}

// ---------------------------------------------------------------------------------------------
// Audit, jobs, data
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AuditQuery {
    actor: Option<String>,
    action: Option<String>,
    entity_type: Option<String>,
    entity_id: Option<String>,
    before: Option<i64>,
    limit: Option<i64>,
}

pub async fn audit(
    State(state): State<SharedState>,
    _user: CurrentUser,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let entries = system::audit_entries(
        &client,
        &system::AuditFilter {
            actor: q.actor.filter(|s| !s.is_empty()),
            action: q.action.filter(|s| !s.is_empty()),
            entity_type: q.entity_type.filter(|s| !s.is_empty()),
            entity_id: q.entity_id.filter(|s| !s.is_empty()),
            before: q.before,
            limit: q.limit.unwrap_or(100).clamp(1, 500),
        },
    )
    .await?;
    Ok(Json(json!({"entries": entries})))
}

#[derive(Deserialize)]
pub struct JobsQuery {
    job: Option<String>,
    limit: Option<i64>,
}

pub async fn jobs(
    State(state): State<SharedState>,
    _user: CurrentUser,
    Query(q): Query<JobsQuery>,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let runs = system::job_runs(
        &client,
        q.job.as_deref().filter(|s| !s.is_empty()),
        q.limit.unwrap_or(100).clamp(1, 500),
    )
    .await?;
    Ok(Json(json!({"jobs": runs})))
}

pub async fn data_status(
    State(state): State<SharedState>,
    _user: CurrentUser,
) -> ApiResult<Json<Value>> {
    let client = state.pool.get().await?;
    let coverage = market::coverage(&client).await?;
    let quotes = market::quotes(&client).await?;
    let mut last_jobs = serde_json::Map::new();
    for job in ["preopen_sync", "postclose_sync", "eod_snapshot", "plan"] {
        last_jobs.insert(job.into(), json!(system::last_job(&client, job).await?));
    }
    let cfg = &state.config.market;
    Ok(Json(json!({
        "source": state.market.source(),
        "fmp": {
            "keys": cfg.fmp.api_keys.len(),
            "key_origin": cfg.key_origin,
            "base_url": cfg.fmp.base_url,
            "api_mode": cfg.fmp.api_mode,
            "requests_per_minute": cfg.fmp.requests_per_minute,
        },
        "history_years": marketdata::history_years(),
        "coverage": coverage,
        "quotes": {
            "count": quotes.len(),
            "newest": quotes.values().map(|q| q.fetched_at).max(),
            "oldest": quotes.values().map(|q| q.fetched_at).min(),
        },
        "jobs": last_jobs,
        "database": state.config.db.display,
        "database_borrowed_from_honeclaw": state.config.db.borrowed_from_honeclaw,
    })))
}

#[derive(Deserialize)]
pub struct SyncBody {
    kind: String,
}

pub async fn data_sync(
    State(state): State<SharedState>,
    AdminUser(user): AdminUser,
    Json(body): Json<SyncBody>,
) -> ApiResult<Json<Value>> {
    let kind = body.kind.clone();
    if !["quotes", "daily", "full", "corporate_actions"].contains(&kind.as_str()) {
        return Err(ApiError::bad(
            "kind must be quotes, daily, full or corporate_actions",
        ));
    }
    {
        let client = state.pool.get().await?;
        system::audit(
            &client,
            &user.username,
            "data.sync_requested",
            "data",
            &kind,
            json!({}),
            &user.ip,
        )
        .await?;
    }
    let s = state.clone();
    tokio::spawn(async move {
        let result = match kind.as_str() {
            "quotes" => marketdata::poll_quotes(&s)
                .await
                .map(|n| json!({"quotes": n})),
            "daily" => marketdata::sync_daily(&s, 10, false)
                .await
                .map(|r| json!(r)),
            "full" => marketdata::sync_daily(&s, 0, true).await.map(|r| json!(r)),
            _ => marketdata::sync_corporate_actions(&s, &[])
                .await
                .map(|n| json!({"actions": n})),
        };
        match result {
            Ok(detail) => tracing::info!(%kind, %detail, "manual data sync finished"),
            Err(error) => {
                tracing::warn!(%kind, error = %format!("{error:#}"), "manual data sync failed")
            }
        }
        s.emit(ServerEvent::Quotes { at: s.now() });
    });
    Ok(Json(json!({"accepted": true})))
}

pub async fn fmp_check(
    State(state): State<SharedState>,
    _admin: AdminUser,
) -> ApiResult<Json<Value>> {
    if state.market.source() != crate::market::DataSource::Fmp {
        return Err(ApiError::bad(
            "the server runs on demo data; there is no FMP key to check",
        ));
    }
    let client = crate::market::fmp::FmpClient::new(state.config.market.fmp.clone())
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let checks = crate::market::fmp::diagnose(&client, "NVDA").await;
    Ok(Json(json!({"checks": checks})))
}
