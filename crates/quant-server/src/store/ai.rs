//! AI sector views.

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use deadpool_postgres::GenericClient;
use serde::Serialize;
use serde_json::Value;
use tokio_postgres::Row;

use crate::ai::view::ViewOutput;

#[derive(Debug, Clone, Serialize)]
pub struct StoredView {
    pub id: i64,
    pub as_of: NaiveDate,
    pub status: String,
    pub prompt_version: String,
    pub model: String,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub usage: Option<Value>,
    pub latency_ms: Option<i32>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    /// Only on the detail endpoint: what the model read and what it answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_response: Option<String>,
}

impl StoredView {
    /// The validated view (status `ok` only).
    pub fn view(&self) -> Option<ViewOutput> {
        (self.status == "ok")
            .then(|| serde_json::from_value(self.output.clone()?).ok())
            .flatten()
    }
}

const COLUMNS: &str = "id, as_of, status, prompt_version, model, output, error, usage, latency_ms, created_by, created_at";

fn row(r: &Row, detail: bool) -> StoredView {
    StoredView {
        id: r.get("id"),
        as_of: r.get("as_of"),
        status: r.get("status"),
        prompt_version: r.get("prompt_version"),
        model: r.get("model"),
        output: r.get("output"),
        error: r.get("error"),
        usage: r.get("usage"),
        latency_ms: r.get("latency_ms"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        material: detail.then(|| r.get("material")),
        raw_response: if detail { r.get("raw_response") } else { None },
    }
}

pub struct NewView<'a> {
    pub as_of: NaiveDate,
    pub prompt_version: &'a str,
    pub model: &'a str,
    pub material: &'a Value,
    pub material_hash: &'a str,
    pub created_by: &'a str,
}

/// Records an attempt as `running`, before the model is called.
pub async fn insert_running(client: &impl GenericClient, view: &NewView<'_>) -> Result<i64> {
    let r = client
        .query_one(
            "INSERT INTO ai_views (as_of, status, prompt_version, model, material, material_hash, created_by)
             VALUES ($1, 'running', $2, $3, $4, $5, $6) RETURNING id",
            &[
                &view.as_of,
                &view.prompt_version,
                &view.model,
                &view.material,
                &view.material_hash,
                &view.created_by,
            ],
        )
        .await?;
    Ok(r.get(0))
}

pub struct Outcome<'a> {
    /// `ok` or `failed`.
    pub status: &'a str,
    pub model: &'a str,
    pub output: Option<Value>,
    pub raw_response: Option<&'a str>,
    pub error: Option<&'a str>,
    pub usage: Option<&'a Value>,
    pub latency_ms: Option<i32>,
}

pub async fn finish(client: &impl GenericClient, id: i64, outcome: &Outcome<'_>) -> Result<()> {
    client
        .execute(
            "UPDATE ai_views SET status = $2, model = $3, output = $4, raw_response = $5, error = $6,
                                 usage = $7, latency_ms = $8
             WHERE id = $1",
            &[
                &id,
                &outcome.status,
                &outcome.model,
                &outcome.output,
                &outcome.raw_response,
                &outcome.error,
                &outcome.usage,
                &outcome.latency_ms,
            ],
        )
        .await?;
    Ok(())
}

/// Attempts left `running` by a process that stopped mid-call.
pub async fn fail_interrupted(client: &impl GenericClient, before: DateTime<Utc>) -> Result<u64> {
    Ok(client
        .execute(
            "UPDATE ai_views SET status = 'failed', error = 'interrupted'
             WHERE status = 'running' AND created_at < $1",
            &[&before],
        )
        .await?)
}

pub async fn list(client: &impl GenericClient, limit: i64) -> Result<Vec<StoredView>> {
    let rows = client
        .query(
            &format!("SELECT {COLUMNS} FROM ai_views ORDER BY id DESC LIMIT $1"),
            &[&limit],
        )
        .await?;
    Ok(rows.iter().map(|r| row(r, false)).collect())
}

pub async fn get(client: &impl GenericClient, id: i64) -> Result<Option<StoredView>> {
    let r = client
        .query_opt(
            &format!("SELECT {COLUMNS}, material, raw_response FROM ai_views WHERE id = $1"),
            &[&id],
        )
        .await?;
    Ok(r.map(|r| row(&r, true)))
}

/// The newest successful view whose material is no older than `oldest_as_of`.
pub async fn latest_ok(
    client: &impl GenericClient,
    oldest_as_of: NaiveDate,
) -> Result<Option<StoredView>> {
    let r = client
        .query_opt(
            &format!(
                "SELECT {COLUMNS} FROM ai_views WHERE status = 'ok' AND as_of >= $1
                 ORDER BY as_of DESC, id DESC LIMIT 1"
            ),
            &[&oldest_as_of],
        )
        .await?;
    Ok(r.map(|r| row(&r, false)))
}

/// Attempts (successful or not) made since `since`, to cap how often the model is called.
pub async fn attempts_since(client: &impl GenericClient, since: DateTime<Utc>) -> Result<i64> {
    let r = client
        .query_one(
            "SELECT count(*) FROM ai_views WHERE created_at >= $1",
            &[&since],
        )
        .await?;
    Ok(r.get(0))
}

/// One plan's AI record (`diagnostics.ai`), oldest first.
#[derive(Debug, Clone)]
pub struct PlanAi {
    pub plan_id: i64,
    pub trade_date: NaiveDate,
    pub slot: String,
    pub generated_at: DateTime<Utc>,
    /// `None` (or without a `view_id`) when the plan used no view.
    pub ai: Option<Value>,
}

impl PlanAi {
    /// Whether the plan recorded a view (applied or in shadow).
    pub fn has_view(&self) -> bool {
        self.ai
            .as_ref()
            .is_some_and(|ai| ai.get("view_id").is_some_and(|v| !v.is_null()))
    }
}

/// Every plan of an account from the first one that recorded an AI sector view on: later plans
/// without a view end the period the previous view's targets were held.
pub async fn plans_since_first_view(
    client: &impl GenericClient,
    account_id: i64,
) -> Result<Vec<PlanAi>> {
    let rows = client
        .query(
            "SELECT id, trade_date, slot, generated_at, diagnostics->'ai' FROM plans
             WHERE account_id = $1 AND status <> 'skipped'
               AND generated_at >= (SELECT min(generated_at) FROM plans
                                    WHERE account_id = $1 AND status <> 'skipped'
                                      AND diagnostics->'ai'->>'view_id' IS NOT NULL)
             ORDER BY generated_at, id",
            &[&account_id],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| PlanAi {
            plan_id: r.get(0),
            trade_date: r.get(1),
            slot: r.get(2),
            generated_at: r.get(3),
            ai: r.get(4),
        })
        .collect())
}
