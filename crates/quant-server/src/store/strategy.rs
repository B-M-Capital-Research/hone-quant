//! Strategy versions and activations, plus the operator's trading controls (per-symbol
//! restrictions and cancelled plan slots).

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use deadpool_postgres::GenericClient;
use quant_core::strategy::StrategyParams;
use serde::Serialize;
use serde_json::Value;
use tokio_postgres::Row;

#[derive(Debug, Clone, Serialize)]
pub struct StrategyVersion {
    pub id: i64,
    pub name: String,
    pub preset_id: String,
    pub params: Value,
    pub note: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

impl StrategyVersion {
    fn from_row(r: &Row) -> Self {
        Self {
            id: r.get("id"),
            name: r.get("name"),
            preset_id: r.get("preset_id"),
            params: r.get("params"),
            note: r.get("note"),
            created_by: r.get("created_by"),
            created_at: r.get("created_at"),
        }
    }

    /// Stored params are lenient-parsed so a version written by an older build still loads.
    pub fn strategy_params(&self) -> Result<StrategyParams> {
        let params: StrategyParams = serde_json::from_value(self.params.clone())
            .context("stored strategy parameters are unreadable")?;
        params.validate().map_err(|errors| {
            anyhow::anyhow!(
                "stored strategy parameters are invalid: {}",
                errors[0].message
            )
        })?;
        Ok(params)
    }
}

pub async fn insert_version(
    client: &impl GenericClient,
    name: &str,
    preset_id: &str,
    params: &StrategyParams,
    note: &str,
    actor: &str,
) -> Result<StrategyVersion> {
    let row = client
        .query_one(
            "INSERT INTO strategy_versions (name, preset_id, params, note, created_by) VALUES ($1, $2, $3, $4, $5)
             RETURNING id, name, preset_id, params, note, created_by, created_at",
            &[&name, &preset_id, &serde_json::to_value(params)?, &note, &actor],
        )
        .await?;
    Ok(StrategyVersion::from_row(&row))
}

pub async fn version(client: &impl GenericClient, id: i64) -> Result<Option<StrategyVersion>> {
    let row = client
        .query_opt(
            "SELECT id, name, preset_id, params, note, created_by, created_at FROM strategy_versions WHERE id = $1",
            &[&id],
        )
        .await?;
    Ok(row.as_ref().map(StrategyVersion::from_row))
}

pub async fn versions(client: &impl GenericClient) -> Result<Vec<StrategyVersion>> {
    let rows = client
        .query(
            "SELECT id, name, preset_id, params, note, created_by, created_at FROM strategy_versions ORDER BY id DESC",
            &[],
        )
        .await?;
    Ok(rows.iter().map(StrategyVersion::from_row).collect())
}

pub async fn activate(
    client: &impl GenericClient,
    account_id: i64,
    version_id: i64,
    actor: &str,
    note: &str,
) -> Result<()> {
    client
        .execute(
            "INSERT INTO strategy_activations (account_id, strategy_version_id, activated_by, note) VALUES ($1, $2, $3, $4)",
            &[&account_id, &version_id, &actor, &note],
        )
        .await?;
    Ok(())
}

pub async fn active_version(
    client: &impl GenericClient,
    account_id: i64,
) -> Result<Option<StrategyVersion>> {
    let row = client
        .query_opt(
            "SELECT v.id, v.name, v.preset_id, v.params, v.note, v.created_by, v.created_at
             FROM strategy_activations a JOIN strategy_versions v ON v.id = a.strategy_version_id
             WHERE a.account_id = $1 ORDER BY a.activated_at DESC, a.id DESC LIMIT 1",
            &[&account_id],
        )
        .await?;
    Ok(row.as_ref().map(StrategyVersion::from_row))
}

#[derive(Debug, Clone, Serialize)]
pub struct Activation {
    pub id: i64,
    pub strategy_version_id: i64,
    pub version_name: String,
    pub activated_by: String,
    pub note: String,
    pub activated_at: DateTime<Utc>,
}

pub async fn activations(client: &impl GenericClient, account_id: i64) -> Result<Vec<Activation>> {
    let rows = client
        .query(
            "SELECT a.id, a.strategy_version_id, v.name, a.activated_by, a.note, a.activated_at
             FROM strategy_activations a JOIN strategy_versions v ON v.id = a.strategy_version_id
             WHERE a.account_id = $1 ORDER BY a.activated_at DESC, a.id DESC",
            &[&account_id],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| Activation {
            id: r.get(0),
            strategy_version_id: r.get(1),
            version_name: r.get(2),
            activated_by: r.get(3),
            note: r.get(4),
            activated_at: r.get(5),
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Restrictions
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Restriction {
    pub id: i64,
    /// `None`: applies to every portfolio.
    pub portfolio_id: Option<i64>,
    pub symbol: String,
    pub mode: String,
    pub reason: String,
    pub starts_on: NaiveDate,
    pub ends_on: Option<NaiveDate>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_by: Option<String>,
}

fn restriction_from_row(r: &Row) -> Restriction {
    Restriction {
        id: r.get(0),
        symbol: r.get(1),
        mode: r.get(2),
        reason: r.get(3),
        starts_on: r.get(4),
        ends_on: r.get(5),
        created_by: r.get(6),
        created_at: r.get(7),
        revoked_at: r.get(8),
        revoked_by: r.get(9),
        portfolio_id: r.get(10),
    }
}

const RESTRICTION_COLUMNS: &str = "id, symbol, mode, reason, starts_on, ends_on, created_by, created_at, revoked_at, revoked_by, portfolio_id";

/// Restrictions in force on `date` for a portfolio: the universe-wide ones and its own. When a
/// symbol has both, the universe-wide one comes first, so the planner's first match (an
/// administrator's decision for every portfolio) wins.
pub async fn active_restrictions(
    client: &impl GenericClient,
    date: NaiveDate,
    portfolio_id: i64,
) -> Result<Vec<Restriction>> {
    let rows = client
        .query(
            &format!(
                "SELECT {RESTRICTION_COLUMNS} FROM trading_restrictions
                 WHERE revoked_at IS NULL AND starts_on <= $1 AND (ends_on IS NULL OR ends_on >= $1)
                   AND (portfolio_id IS NULL OR portfolio_id = $2)
                 ORDER BY symbol, portfolio_id NULLS FIRST, id"
            ),
            &[&date, &portfolio_id],
        )
        .await?;
    Ok(rows.iter().map(restriction_from_row).collect())
}

/// The latest restrictions (any state) that concern a portfolio.
pub async fn all_restrictions(
    client: &impl GenericClient,
    portfolio_id: i64,
) -> Result<Vec<Restriction>> {
    let rows = client
        .query(
            &format!(
                "SELECT {RESTRICTION_COLUMNS} FROM trading_restrictions
                 WHERE portfolio_id IS NULL OR portfolio_id = $1 ORDER BY created_at DESC LIMIT 500"
            ),
            &[&portfolio_id],
        )
        .await?;
    Ok(rows.iter().map(restriction_from_row).collect())
}

pub async fn restriction(client: &impl GenericClient, id: i64) -> Result<Option<Restriction>> {
    let row = client
        .query_opt(
            &format!("SELECT {RESTRICTION_COLUMNS} FROM trading_restrictions WHERE id = $1"),
            &[&id],
        )
        .await?;
    Ok(row.as_ref().map(restriction_from_row))
}

#[allow(clippy::too_many_arguments)]
pub async fn add_restriction(
    client: &impl GenericClient,
    portfolio_id: Option<i64>,
    symbol: &str,
    mode: &str,
    reason: &str,
    starts_on: NaiveDate,
    ends_on: Option<NaiveDate>,
    actor: &str,
) -> Result<Restriction> {
    let row = client
        .query_one(
            &format!(
                "INSERT INTO trading_restrictions (portfolio_id, symbol, mode, reason, starts_on, ends_on, created_by)
                 VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING {RESTRICTION_COLUMNS}"
            ),
            &[&portfolio_id, &symbol, &mode, &reason, &starts_on, &ends_on, &actor],
        )
        .await?;
    Ok(restriction_from_row(&row))
}

pub async fn revoke_restriction(
    client: &impl GenericClient,
    id: i64,
    actor: &str,
) -> Result<Option<Restriction>> {
    let row = client
        .query_opt(
            &format!(
                "UPDATE trading_restrictions SET revoked_at = app_now(), revoked_by = $2
                 WHERE id = $1 AND revoked_at IS NULL RETURNING {RESTRICTION_COLUMNS}"
            ),
            &[&id, &actor],
        )
        .await?;
    Ok(row.as_ref().map(restriction_from_row))
}

// ---------------------------------------------------------------------------------------------
// Cancelled slots
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SkippedSlot {
    pub trade_date: NaiveDate,
    pub slot: String,
    pub reason: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

pub async fn skipped_slots(
    client: &impl GenericClient,
    portfolio_id: i64,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<SkippedSlot>> {
    let rows = client
        .query(
            "SELECT trade_date, slot, reason, created_by, created_at FROM skipped_slots
             WHERE portfolio_id = $1 AND trade_date BETWEEN $2 AND $3 ORDER BY trade_date, slot",
            &[&portfolio_id, &from, &to],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| SkippedSlot {
            trade_date: r.get(0),
            slot: r.get(1),
            reason: r.get(2),
            created_by: r.get(3),
            created_at: r.get(4),
        })
        .collect())
}

pub async fn is_slot_skipped(
    client: &impl GenericClient,
    portfolio_id: i64,
    date: NaiveDate,
    slot: &str,
) -> Result<Option<SkippedSlot>> {
    Ok(skipped_slots(client, portfolio_id, date, date)
        .await?
        .into_iter()
        .find(|s| s.slot == slot))
}

pub async fn skip_slot(
    client: &impl GenericClient,
    portfolio_id: i64,
    date: NaiveDate,
    slot: &str,
    reason: &str,
    actor: &str,
) -> Result<bool> {
    let n = client
        .execute(
            "INSERT INTO skipped_slots (portfolio_id, trade_date, slot, reason, created_by) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT DO NOTHING",
            &[&portfolio_id, &date, &slot, &reason, &actor],
        )
        .await?;
    Ok(n > 0)
}

pub async fn unskip_slot(
    client: &impl GenericClient,
    portfolio_id: i64,
    date: NaiveDate,
    slot: &str,
) -> Result<bool> {
    let n = client
        .execute(
            "DELETE FROM skipped_slots WHERE portfolio_id = $1 AND trade_date = $2 AND slot = $3",
            &[&portfolio_id, &date, &slot],
        )
        .await?;
    Ok(n > 0)
}
