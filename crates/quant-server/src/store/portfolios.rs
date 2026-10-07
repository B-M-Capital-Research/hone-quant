//! Portfolios: named, isolated paper books.
//!
//! A portfolio owns a series of paper accounts (exactly one active while the portfolio is
//! active), its automation mode, and — through its accounts — its holdings, plans, fills and
//! strategy activations. Everything else (universe, strategy library, market data, settings) is
//! shared by all portfolios.

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use deadpool_postgres::GenericClient;
use serde::Serialize;
use tokio_postgres::Row;

use crate::store::settings::AutomationSettings;
use crate::store::trading::{self, Account};

#[derive(Debug, Clone, Serialize)]
pub struct Portfolio {
    pub id: i64,
    pub name: String,
    pub description: String,
    /// Audit identity of the owner; `None` = shared, managed by administrators.
    pub owner: Option<String>,
    pub owner_name: String,
    pub automation: AutomationSettings,
    pub status: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub archived_by: Option<String>,
}

impl Portfolio {
    fn from_row(row: &Row) -> Self {
        let automation: serde_json::Value = row.get("automation");
        Self {
            id: row.get("id"),
            name: row.get("name"),
            description: row.get("description"),
            owner: row.get("owner"),
            owner_name: row.get("owner_name"),
            automation: serde_json::from_value(automation).unwrap_or_default(),
            status: row.get("status"),
            created_by: row.get("created_by"),
            created_at: row.get("created_at"),
            archived_at: row.get("archived_at"),
            archived_by: row.get("archived_by"),
        }
    }

    pub fn is_active(&self) -> bool {
        self.status == "active"
    }

    pub fn tag(&self) -> PortfolioRef {
        PortfolioRef {
            id: self.id,
            name: self.name.clone(),
        }
    }
}

/// What notifications and events need to say which portfolio they are about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PortfolioRef {
    pub id: i64,
    pub name: String,
}

/// A portfolio together with its active paper account.
#[derive(Debug, Clone)]
pub struct Book {
    pub portfolio: Portfolio,
    pub account: Account,
}

const COLUMNS: &str = "id, name, description, owner, owner_name, automation, status, created_by, created_at, archived_at, archived_by";

pub async fn get(client: &impl GenericClient, id: i64) -> Result<Option<Portfolio>> {
    let row = client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM portfolios WHERE id = $1"),
            &[&id],
        )
        .await?;
    Ok(row.as_ref().map(Portfolio::from_row))
}

/// Locks the portfolio row for the rest of the transaction.
pub async fn lock(client: &impl GenericClient, id: i64) -> Result<Portfolio> {
    let row = client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM portfolios WHERE id = $1 FOR UPDATE"),
            &[&id],
        )
        .await?
        .ok_or_else(|| anyhow!("portfolio {id} not found"))?;
    Ok(Portfolio::from_row(&row))
}

/// Shared portfolios first, then by creation.
pub async fn list(client: &impl GenericClient, include_archived: bool) -> Result<Vec<Portfolio>> {
    let rows = client
        .query(
            &format!(
                "SELECT {COLUMNS} FROM portfolios WHERE $1 OR status = 'active'
                 ORDER BY status, owner IS NOT NULL, id"
            ),
            &[&include_archived],
        )
        .await?;
    Ok(rows.iter().map(Portfolio::from_row).collect())
}

pub async fn active_count(client: &impl GenericClient) -> Result<i64> {
    Ok(client
        .query_one(
            "SELECT count(*) FROM portfolios WHERE status = 'active'",
            &[],
        )
        .await?
        .get(0))
}

pub struct NewPortfolio<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub owner: Option<&'a str>,
    pub owner_name: &'a str,
    pub automation: &'a AutomationSettings,
    pub created_by: &'a str,
}

/// `None` when the owner already has an active portfolio with that name.
pub async fn insert(
    client: &impl GenericClient,
    p: &NewPortfolio<'_>,
) -> Result<Option<Portfolio>> {
    let row = client
        .query_opt(
            &format!(
                "INSERT INTO portfolios (name, description, owner, owner_name, automation, created_by)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT DO NOTHING RETURNING {COLUMNS}"
            ),
            &[
                &p.name,
                &p.description,
                &p.owner,
                &p.owner_name,
                &serde_json::to_value(p.automation)?,
                &p.created_by,
            ],
        )
        .await?;
    Ok(row.as_ref().map(Portfolio::from_row))
}

/// `None` when the name is taken by another of the owner's active portfolios.
pub async fn rename(
    client: &impl GenericClient,
    id: i64,
    name: &str,
    description: &str,
) -> Result<Option<Portfolio>> {
    let taken: bool = client
        .query_one(
            "SELECT EXISTS (
                 SELECT 1 FROM portfolios p JOIN portfolios me ON me.id = $1
                 WHERE p.id <> $1 AND p.status = 'active'
                   AND COALESCE(p.owner, '') = COALESCE(me.owner, '')
                   AND lower(btrim(p.name)) = lower(btrim($2)))",
            &[&id, &name],
        )
        .await?
        .get(0);
    if taken {
        return Ok(None);
    }
    let row = client
        .query_one(
            &format!(
                "UPDATE portfolios SET name = $2, description = $3 WHERE id = $1 RETURNING {COLUMNS}"
            ),
            &[&id, &name, &description],
        )
        .await?;
    Ok(Some(Portfolio::from_row(&row)))
}

pub async fn set_automation(
    client: &impl GenericClient,
    id: i64,
    automation: &AutomationSettings,
) -> Result<()> {
    client
        .execute(
            "UPDATE portfolios SET automation = $2 WHERE id = $1",
            &[&id, &serde_json::to_value(automation)?],
        )
        .await?;
    Ok(())
}

pub async fn archive(client: &impl GenericClient, id: i64, actor: &str) -> Result<Portfolio> {
    let row = client
        .query_one(
            &format!(
                "UPDATE portfolios SET status = 'archived', archived_at = app_now(), archived_by = $2
                 WHERE id = $1 RETURNING {COLUMNS}"
            ),
            &[&id, &actor],
        )
        .await?;
    Ok(Portfolio::from_row(&row))
}

/// The portfolio and its active account, for an active portfolio.
pub async fn book(client: &impl GenericClient, portfolio_id: i64) -> Result<Book> {
    let portfolio = get(client, portfolio_id)
        .await?
        .filter(Portfolio::is_active)
        .ok_or_else(|| anyhow!("portfolio {portfolio_id} is not active"))?;
    let account = trading::active_account(client, portfolio_id)
        .await?
        .ok_or_else(|| anyhow!("portfolio {portfolio_id} has no active paper account"))?;
    Ok(Book { portfolio, account })
}

/// Every active portfolio with its active account (what the scheduler trades).
pub async fn active_books(client: &impl GenericClient) -> Result<Vec<Book>> {
    let mut books = Vec::new();
    for portfolio in list(client, false).await? {
        if let Some(account) = trading::active_account(client, portfolio.id).await? {
            books.push(Book { portfolio, account });
        }
    }
    Ok(books)
}

/// The portfolio an account (active or archived) belongs to.
pub async fn for_account(client: &impl GenericClient, account_id: i64) -> Result<Portfolio> {
    let row = client
        .query_opt(
            &format!(
                "SELECT {} FROM portfolios p JOIN accounts a ON a.portfolio_id = p.id WHERE a.id = $1",
                COLUMNS
                    .split(", ")
                    .map(|c| format!("p.{c}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            &[&account_id],
        )
        .await?
        .ok_or_else(|| anyhow!("account {account_id} not found"))?;
    Ok(Portfolio::from_row(&row))
}
