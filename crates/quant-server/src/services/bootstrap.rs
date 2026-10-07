//! First-run and every-start initialisation: universe sync, operator account, the first
//! portfolio, default strategy and built-in reminders. Everything here is idempotent.

use std::sync::Arc;

use anyhow::Result;
use quant_core::strategy::{DEFAULT_PRESET, preset};
use rust_decimal::Decimal;
use serde_json::json;

use crate::auth;
use crate::notify::{self, Event};
use crate::services::portfolio;
use crate::services::reminders;
use crate::state::AppState;
use crate::store::portfolios::{self, NewPortfolio};
use crate::store::settings::AutomationSettings;
use crate::store::{strategy as strategy_store, system};
use crate::universe;

pub async fn run(state: &Arc<AppState>) -> Result<()> {
    // Universe from the bundled honeclaw snapshot (or the configured replacement).
    let file = universe::load(state.config.universe_path.as_deref())?;
    let mut client = state.pool.get().await?;
    let changes = universe::sync_to_db(&mut client, &file, "system").await?;
    if !changes.first_load && !changes.is_empty() {
        tracing::info!(?changes, "universe changed");
        system::audit(
            &client,
            "system",
            "universe.synced",
            "universe",
            "",
            serde_json::to_value(&changes)?,
            "",
        )
        .await?;
        let _ = notify::notify(
            state,
            Event::UniverseChanged {
                added: changes.added.clone(),
                removed: changes.removed.clone(),
            },
        )
        .await;
    }

    // Operator. In honeclaw mode honeclaw's administrators sign in; no local account is made.
    if state.honeclaw.is_some() {
        if state.config.bootstrap_admin.is_some() {
            tracing::warn!(
                "HONE_QUANT_ADMIN_PASSWORD is ignored: sign-in goes through honeclaw (HONE_QUANT_AUTH_MODE=honeclaw)"
            );
        }
    } else if system::user_count(&client).await? == 0 {
        match &state.config.bootstrap_admin {
            Some((username, password)) => {
                auth::check_password_strength(password).map_err(anyhow::Error::msg)?;
                let hash = auth::hash_password(password)?;
                system::create_user(&client, username, &hash, "admin").await?;
                system::audit(
                    &client,
                    "system",
                    "user.created",
                    "user",
                    username,
                    json!({"role": "admin", "source": "bootstrap"}),
                    "",
                )
                .await?;
                tracing::info!(%username, "created the first administrator from HONE_QUANT_ADMIN_PASSWORD");
            }
            None => tracing::warn!(
                "no operator account exists: set HONE_QUANT_ADMIN_PASSWORD (and optionally HONE_QUANT_ADMIN_USER) or run `hone-quant user add <name>`"
            ),
        }
    }

    // Default strategy version: the default preset, created once.
    let default_version = match strategy_store::versions(&client)
        .await?
        .into_iter()
        .find(|v| v.preset_id == DEFAULT_PRESET)
    {
        Some(version) => version,
        None => {
            let p = preset(DEFAULT_PRESET).expect("default preset exists");
            strategy_store::insert_version(
                &client,
                p.name_en,
                p.id,
                &p.params,
                "default strategy",
                "system",
            )
            .await?
        }
    };

    // First start: one shared portfolio.
    if portfolios::list(&client, true).await?.is_empty() {
        let cash = Decimal::from_f64_retain(state.config.initial_cash)
            .unwrap_or(Decimal::ONE_THOUSAND)
            .round_dp(2);
        let tx = client.transaction().await?;
        portfolio::open(
            state,
            &tx,
            &portfolio::OpenPortfolio {
                portfolio: NewPortfolio {
                    name: "Main",
                    description: "",
                    owner: None,
                    owner_name: "",
                    automation: &AutomationSettings::default(),
                    created_by: "system",
                },
                initial_cash: cash,
                strategy_version_id: default_version.id,
            },
        )
        .await?;
        tx.commit().await?;
        tracing::info!(initial_cash = %cash, "created the Main portfolio");
    }

    // Every active portfolio trades some strategy.
    for book in portfolios::active_books(&client).await? {
        if strategy_store::active_version(&client, book.account.id)
            .await?
            .is_none()
        {
            strategy_store::activate(
                &client,
                book.account.id,
                default_version.id,
                "system",
                "initial activation",
            )
            .await?;
            system::audit(
                &client,
                "system",
                "strategy.activated",
                "strategy_version",
                &default_version.id.to_string(),
                json!({"preset": DEFAULT_PRESET, "portfolio_id": book.portfolio.id}),
                "",
            )
            .await?;
        }
    }

    reminders::seed_builtins(&client).await?;
    system::fail_interrupted_backtests(&client).await?;
    Ok(())
}
