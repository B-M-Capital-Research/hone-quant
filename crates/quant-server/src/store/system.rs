//! Audit trail, scheduler job runs, operators and sessions, notifications, reminders and
//! backtest records.

use anyhow::Result;
use chrono::{DateTime, Utc};
use deadpool_postgres::GenericClient;
use serde::Serialize;
use serde_json::Value;
use tokio_postgres::Row;

// ---------------------------------------------------------------------------------------------
// Audit
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub id: i64,
    pub ts: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub entity_type: String,
    pub entity_id: String,
    pub detail: Value,
    pub ip: String,
}

pub async fn audit(
    client: &impl GenericClient,
    actor: &str,
    action: &str,
    entity_type: &str,
    entity_id: &str,
    detail: Value,
    ip: &str,
) -> Result<()> {
    client
        .execute(
            "INSERT INTO audit_log (actor, action, entity_type, entity_id, detail, ip) VALUES ($1, $2, $3, $4, $5, $6)",
            &[&actor, &action, &entity_type, &entity_id, &detail, &ip],
        )
        .await?;
    Ok(())
}

pub struct AuditFilter {
    pub actor: Option<String>,
    pub action: Option<String>,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub before: Option<i64>,
    pub limit: i64,
}

pub async fn audit_entries(
    client: &impl GenericClient,
    filter: &AuditFilter,
) -> Result<Vec<AuditEntry>> {
    let rows = client
        .query(
            "SELECT id, ts, actor, action, entity_type, entity_id, detail, ip FROM audit_log
             WHERE ($1::text IS NULL OR actor = $1) AND ($2::text IS NULL OR action LIKE $2 || '%')
               AND ($3::text IS NULL OR entity_type = $3) AND ($4::text IS NULL OR entity_id = $4)
               AND ($5::bigint IS NULL OR id < $5)
             ORDER BY id DESC LIMIT $6",
            &[
                &filter.actor,
                &filter.action,
                &filter.entity_type,
                &filter.entity_id,
                &filter.before,
                &filter.limit,
            ],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| AuditEntry {
            id: r.get(0),
            ts: r.get(1),
            actor: r.get(2),
            action: r.get(3),
            entity_type: r.get(4),
            entity_id: r.get(5),
            detail: r.get(6),
            ip: r.get(7),
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Job runs (idempotency + operational history for the scheduler)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct JobRun {
    pub id: i64,
    pub job: String,
    pub run_key: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub detail: Value,
    pub error: Option<String>,
}

/// Claims `(job, run_key)`. Returns `None` if it already ran (or is running).
pub async fn claim_job(
    client: &impl GenericClient,
    job: &str,
    run_key: &str,
) -> Result<Option<i64>> {
    let row = client
        .query_opt(
            "INSERT INTO job_runs (job, run_key, status) VALUES ($1, $2, 'running') ON CONFLICT (job, run_key) DO NOTHING RETURNING id",
            &[&job, &run_key],
        )
        .await?;
    Ok(row.map(|r| r.get(0)))
}

/// Re-claims a failed run so it can be retried.
pub async fn reclaim_failed_job(
    client: &impl GenericClient,
    job: &str,
    run_key: &str,
) -> Result<Option<i64>> {
    let row = client
        .query_opt(
            "UPDATE job_runs SET status = 'running', started_at = app_now(), finished_at = NULL, error = NULL
             WHERE job = $1 AND run_key = $2 AND status = 'failed' RETURNING id",
            &[&job, &run_key],
        )
        .await?;
    Ok(row.map(|r| r.get(0)))
}

pub async fn finish_job(
    client: &impl GenericClient,
    id: i64,
    status: &str,
    detail: Value,
    error: Option<&str>,
) -> Result<()> {
    client
        .execute(
            "UPDATE job_runs SET status = $2, finished_at = app_now(), detail = $3, error = $4 WHERE id = $1",
            &[&id, &status, &detail, &error],
        )
        .await?;
    Ok(())
}

pub async fn job_runs(
    client: &impl GenericClient,
    job: Option<&str>,
    limit: i64,
) -> Result<Vec<JobRun>> {
    let rows = client
        .query(
            "SELECT id, job, run_key, status, started_at, finished_at, detail, error FROM job_runs
             WHERE ($1::text IS NULL OR job = $1) ORDER BY started_at DESC LIMIT $2",
            &[&job, &limit],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| JobRun {
            id: r.get(0),
            job: r.get(1),
            run_key: r.get(2),
            status: r.get(3),
            started_at: r.get(4),
            finished_at: r.get(5),
            detail: r.get(6),
            error: r.get(7),
        })
        .collect())
}

pub async fn last_job(client: &impl GenericClient, job: &str) -> Result<Option<JobRun>> {
    Ok(job_runs(client, Some(job), 1).await?.into_iter().next())
}

/// Marks runs left `running` by a crashed process as failed.
pub async fn fail_stale_jobs(client: &impl GenericClient) -> Result<u64> {
    Ok(client
        .execute(
            "UPDATE job_runs SET status = 'failed', finished_at = app_now(), error = 'interrupted (process restarted)'
             WHERE status = 'running' AND started_at < app_now() - interval '15 minutes'",
            &[],
        )
        .await?)
}

// ---------------------------------------------------------------------------------------------
// Users and sessions
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[serde(skip)]
    pub password_hash: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

fn user_from_row(r: &Row) -> User {
    User {
        id: r.get("id"),
        username: r.get("username"),
        password_hash: r.get("password_hash"),
        role: r.get("role"),
        created_at: r.get("created_at"),
        last_login_at: r.get("last_login_at"),
    }
}

pub async fn user_count(client: &impl GenericClient) -> Result<i64> {
    Ok(client
        .query_one("SELECT count(*) FROM users", &[])
        .await?
        .get(0))
}

pub async fn user_by_name(client: &impl GenericClient, username: &str) -> Result<Option<User>> {
    let row = client
        .query_opt(
            "SELECT id, username, password_hash, role, created_at, last_login_at FROM users WHERE lower(username) = lower($1)",
            &[&username],
        )
        .await?;
    Ok(row.as_ref().map(user_from_row))
}

pub async fn users(client: &impl GenericClient) -> Result<Vec<User>> {
    let rows = client
        .query(
            "SELECT id, username, password_hash, role, created_at, last_login_at FROM users ORDER BY id",
            &[],
        )
        .await?;
    Ok(rows.iter().map(user_from_row).collect())
}

pub async fn create_user(
    client: &impl GenericClient,
    username: &str,
    hash: &str,
    role: &str,
) -> Result<User> {
    let row = client
        .query_one(
            "INSERT INTO users (username, password_hash, role) VALUES ($1, $2, $3)
             RETURNING id, username, password_hash, role, created_at, last_login_at",
            &[&username, &hash, &role],
        )
        .await?;
    Ok(user_from_row(&row))
}

pub async fn set_password(client: &impl GenericClient, user_id: i64, hash: &str) -> Result<()> {
    client
        .execute(
            "UPDATE users SET password_hash = $2, password_changed_at = now() WHERE id = $1",
            &[&user_id, &hash],
        )
        .await?;
    Ok(())
}

pub async fn touch_login(client: &impl GenericClient, user_id: i64) -> Result<()> {
    client
        .execute(
            "UPDATE users SET last_login_at = now() WHERE id = $1",
            &[&user_id],
        )
        .await?;
    Ok(())
}

pub async fn create_session(
    client: &impl GenericClient,
    token_hash: &str,
    user_id: i64,
    expires_at: DateTime<Utc>,
    user_agent: &str,
    ip: &str,
) -> Result<()> {
    client
        .execute(
            "INSERT INTO sessions (token_hash, user_id, expires_at, user_agent, ip) VALUES ($1, $2, $3, $4, $5)",
            &[&token_hash, &user_id, &expires_at, &user_agent, &ip],
        )
        .await?;
    Ok(())
}

/// The session's user if the session is valid; refreshes `last_seen_at` at most once a minute.
pub async fn session_user(client: &impl GenericClient, token_hash: &str) -> Result<Option<User>> {
    let row = client
        .query_opt(
            "WITH s AS (
                 UPDATE sessions SET last_seen_at = now()
                 WHERE token_hash = $1 AND expires_at > now()
                 RETURNING user_id
             )
             SELECT u.id, u.username, u.password_hash, u.role, u.created_at, u.last_login_at
             FROM users u JOIN s ON s.user_id = u.id",
            &[&token_hash],
        )
        .await?;
    Ok(row.as_ref().map(user_from_row))
}

pub async fn delete_session(client: &impl GenericClient, token_hash: &str) -> Result<()> {
    client
        .execute("DELETE FROM sessions WHERE token_hash = $1", &[&token_hash])
        .await?;
    Ok(())
}

pub async fn delete_user_sessions(
    client: &impl GenericClient,
    user_id: i64,
    except: Option<&str>,
) -> Result<()> {
    client
        .execute(
            "DELETE FROM sessions WHERE user_id = $1 AND ($2::text IS NULL OR token_hash <> $2)",
            &[&user_id, &except],
        )
        .await?;
    Ok(())
}

pub async fn purge_expired_sessions(client: &impl GenericClient) -> Result<u64> {
    Ok(client
        .execute("DELETE FROM sessions WHERE expires_at < now()", &[])
        .await?)
}

// ---------------------------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct NotificationRow {
    pub id: i64,
    pub ts: DateTime<Utc>,
    pub kind: String,
    pub category: String,
    pub severity: String,
    pub title_zh: String,
    pub title_en: String,
    pub body_zh: String,
    pub body_en: String,
    pub params: Value,
    pub link: Option<String>,
    pub read_at: Option<DateTime<Utc>>,
    pub deliveries: Value,
    pub deferred: bool,
}

const NOTIFICATION_COLUMNS: &str = "id, ts, kind, category, severity, title_zh, title_en, body_zh, body_en, params, link, read_at, deliveries, deferred";

fn notification_from_row(r: &Row) -> NotificationRow {
    NotificationRow {
        id: r.get(0),
        ts: r.get(1),
        kind: r.get(2),
        category: r.get(3),
        severity: r.get(4),
        title_zh: r.get(5),
        title_en: r.get(6),
        body_zh: r.get(7),
        body_en: r.get(8),
        params: r.get(9),
        link: r.get(10),
        read_at: r.get(11),
        deliveries: r.get(12),
        deferred: r.get(13),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_notification(
    client: &impl GenericClient,
    kind: &str,
    category: &str,
    severity: &str,
    title: (&str, &str),
    body: (&str, &str),
    params: &Value,
    link: Option<&str>,
) -> Result<NotificationRow> {
    let row = client
        .query_one(
            &format!(
                "INSERT INTO notifications (kind, category, severity, title_zh, title_en, body_zh, body_en, params, link)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING {NOTIFICATION_COLUMNS}"
            ),
            &[&kind, &category, &severity, &title.0, &title.1, &body.0, &body.1, params, &link],
        )
        .await?;
    Ok(notification_from_row(&row))
}

pub async fn set_deliveries(
    client: &impl GenericClient,
    id: i64,
    deliveries: &Value,
    deferred: bool,
) -> Result<()> {
    client
        .execute(
            "UPDATE notifications SET deliveries = $2, deferred = $3 WHERE id = $1",
            &[&id, deliveries, &deferred],
        )
        .await?;
    Ok(())
}

pub async fn deferred_notifications(client: &impl GenericClient) -> Result<Vec<NotificationRow>> {
    let rows = client
        .query(
            &format!("SELECT {NOTIFICATION_COLUMNS} FROM notifications WHERE deferred ORDER BY ts LIMIT 200"),
            &[],
        )
        .await?;
    Ok(rows.iter().map(notification_from_row).collect())
}

pub struct NotificationFilter {
    pub unread_only: bool,
    pub category: Option<String>,
    pub before: Option<i64>,
    pub limit: i64,
}

pub async fn notifications(
    client: &impl GenericClient,
    filter: &NotificationFilter,
) -> Result<Vec<NotificationRow>> {
    let rows = client
        .query(
            &format!(
                "SELECT {NOTIFICATION_COLUMNS} FROM notifications
                 WHERE (NOT $1 OR read_at IS NULL) AND ($2::text IS NULL OR category = $2) AND ($3::bigint IS NULL OR id < $3)
                 ORDER BY id DESC LIMIT $4"
            ),
            &[&filter.unread_only, &filter.category, &filter.before, &filter.limit],
        )
        .await?;
    Ok(rows.iter().map(notification_from_row).collect())
}

pub async fn unread_count(client: &impl GenericClient) -> Result<i64> {
    Ok(client
        .query_one(
            "SELECT count(*) FROM notifications WHERE read_at IS NULL",
            &[],
        )
        .await?
        .get(0))
}

pub async fn mark_read(client: &impl GenericClient, id: Option<i64>) -> Result<u64> {
    Ok(client
        .execute(
            "UPDATE notifications SET read_at = app_now() WHERE read_at IS NULL AND ($1::bigint IS NULL OR id = $1)",
            &[&id],
        )
        .await?)
}

// ---------------------------------------------------------------------------------------------
// Reminders
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Reminder {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub note: String,
    pub schedule: Value,
    pub enabled: bool,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub next_fire_at: Option<DateTime<Utc>>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const REMINDER_COLUMNS: &str = "id, kind, title, note, schedule, enabled, last_fired_at, next_fire_at, created_by, created_at, updated_at";

fn reminder_from_row(r: &Row) -> Reminder {
    Reminder {
        id: r.get(0),
        kind: r.get(1),
        title: r.get(2),
        note: r.get(3),
        schedule: r.get(4),
        enabled: r.get(5),
        last_fired_at: r.get(6),
        next_fire_at: r.get(7),
        created_by: r.get(8),
        created_at: r.get(9),
        updated_at: r.get(10),
    }
}

pub async fn reminders(client: &impl GenericClient) -> Result<Vec<Reminder>> {
    let rows = client
        .query(
            &format!("SELECT {REMINDER_COLUMNS} FROM reminders ORDER BY (kind = 'custom'), id"),
            &[],
        )
        .await?;
    Ok(rows.iter().map(reminder_from_row).collect())
}

pub async fn reminder(client: &impl GenericClient, id: i64) -> Result<Option<Reminder>> {
    let row = client
        .query_opt(
            &format!("SELECT {REMINDER_COLUMNS} FROM reminders WHERE id = $1"),
            &[&id],
        )
        .await?;
    Ok(row.as_ref().map(reminder_from_row))
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_builtin_reminder(
    client: &impl GenericClient,
    kind: &str,
    schedule: &Value,
    enabled: bool,
    next_fire_at: Option<DateTime<Utc>>,
) -> Result<()> {
    client
        .execute(
            "INSERT INTO reminders (kind, schedule, enabled, next_fire_at, created_by) VALUES ($1, $2, $3, $4, 'system')
             ON CONFLICT (kind) WHERE kind <> 'custom' DO NOTHING",
            &[&kind, schedule, &enabled, &next_fire_at],
        )
        .await?;
    Ok(())
}

pub async fn insert_reminder(
    client: &impl GenericClient,
    title: &str,
    note: &str,
    schedule: &Value,
    enabled: bool,
    next_fire_at: Option<DateTime<Utc>>,
    actor: &str,
) -> Result<Reminder> {
    let row = client
        .query_one(
            &format!(
                "INSERT INTO reminders (kind, title, note, schedule, enabled, next_fire_at, created_by)
                 VALUES ('custom', $1, $2, $3, $4, $5, $6) RETURNING {REMINDER_COLUMNS}"
            ),
            &[&title, &note, schedule, &enabled, &next_fire_at, &actor],
        )
        .await?;
    Ok(reminder_from_row(&row))
}

pub async fn update_reminder(
    client: &impl GenericClient,
    id: i64,
    title: &str,
    note: &str,
    schedule: &Value,
    enabled: bool,
    next_fire_at: Option<DateTime<Utc>>,
) -> Result<Option<Reminder>> {
    let row = client
        .query_opt(
            &format!(
                "UPDATE reminders SET title = $2, note = $3, schedule = $4, enabled = $5, next_fire_at = $6, updated_at = app_now()
                 WHERE id = $1 RETURNING {REMINDER_COLUMNS}"
            ),
            &[&id, &title, &note, schedule, &enabled, &next_fire_at],
        )
        .await?;
    Ok(row.as_ref().map(reminder_from_row))
}

pub async fn mark_reminder_fired(
    client: &impl GenericClient,
    id: i64,
    fired_at: DateTime<Utc>,
    next_fire_at: Option<DateTime<Utc>>,
) -> Result<()> {
    client
        .execute(
            "UPDATE reminders SET last_fired_at = $2, next_fire_at = $3, enabled = enabled AND $3 IS NOT NULL WHERE id = $1",
            &[&id, &fired_at, &next_fire_at],
        )
        .await?;
    Ok(())
}

pub async fn delete_reminder(client: &impl GenericClient, id: i64) -> Result<bool> {
    Ok(client
        .execute(
            "DELETE FROM reminders WHERE id = $1 AND kind = 'custom'",
            &[&id],
        )
        .await?
        > 0)
}

// ---------------------------------------------------------------------------------------------
// Backtests
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct BacktestRow {
    pub id: i64,
    pub name: String,
    pub status: String,
    pub config: Value,
    pub strategy_version_id: Option<i64>,
    pub summary: Option<Value>,
    pub error: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

const BACKTEST_COLUMNS: &str = "id, name, status, config, strategy_version_id, summary, error, created_by, created_at, started_at, finished_at";

fn backtest_from_row(r: &Row) -> BacktestRow {
    BacktestRow {
        id: r.get(0),
        name: r.get(1),
        status: r.get(2),
        config: r.get(3),
        strategy_version_id: r.get(4),
        summary: r.get(5),
        error: r.get(6),
        created_by: r.get(7),
        created_at: r.get(8),
        started_at: r.get(9),
        finished_at: r.get(10),
    }
}

pub async fn insert_backtest(
    client: &impl GenericClient,
    name: &str,
    config: &Value,
    strategy_version_id: Option<i64>,
    actor: &str,
) -> Result<BacktestRow> {
    let row = client
        .query_one(
            &format!(
                "INSERT INTO backtests (name, status, config, strategy_version_id, created_by)
                 VALUES ($1, 'queued', $2, $3, $4) RETURNING {BACKTEST_COLUMNS}"
            ),
            &[&name, config, &strategy_version_id, &actor],
        )
        .await?;
    Ok(backtest_from_row(&row))
}

pub async fn backtests(client: &impl GenericClient, limit: i64) -> Result<Vec<BacktestRow>> {
    let rows = client
        .query(
            &format!("SELECT {BACKTEST_COLUMNS} FROM backtests ORDER BY id DESC LIMIT $1"),
            &[&limit],
        )
        .await?;
    Ok(rows.iter().map(backtest_from_row).collect())
}

pub async fn backtest(
    client: &impl GenericClient,
    id: i64,
) -> Result<Option<(BacktestRow, Option<Value>)>> {
    let row = client
        .query_opt(
            &format!("SELECT {BACKTEST_COLUMNS}, result FROM backtests WHERE id = $1"),
            &[&id],
        )
        .await?;
    Ok(row.map(|r| (backtest_from_row(&r), r.get(11))))
}

pub async fn start_backtest(client: &impl GenericClient, id: i64) -> Result<()> {
    client
        .execute(
            "UPDATE backtests SET status = 'running', started_at = app_now() WHERE id = $1",
            &[&id],
        )
        .await?;
    Ok(())
}

pub async fn finish_backtest(
    client: &impl GenericClient,
    id: i64,
    summary: Option<&Value>,
    result: Option<&Value>,
    error: Option<&str>,
) -> Result<()> {
    let status = if error.is_some() {
        "failed"
    } else {
        "succeeded"
    };
    client
        .execute(
            "UPDATE backtests SET status = $2, summary = $3, result = $4, error = $5, finished_at = app_now() WHERE id = $1",
            &[&id, &status, &summary, &result, &error],
        )
        .await?;
    Ok(())
}

pub async fn delete_backtest(client: &impl GenericClient, id: i64) -> Result<bool> {
    Ok(client
        .execute(
            "DELETE FROM backtests WHERE id = $1 AND status <> 'running'",
            &[&id],
        )
        .await?
        > 0)
}

pub async fn fail_interrupted_backtests(client: &impl GenericClient) -> Result<u64> {
    Ok(client
        .execute(
            "UPDATE backtests SET status = 'failed', error = 'interrupted (process restarted)', finished_at = app_now()
             WHERE status IN ('queued', 'running')",
            &[],
        )
        .await?)
}
