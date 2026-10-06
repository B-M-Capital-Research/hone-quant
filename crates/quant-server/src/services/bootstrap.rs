//! First-run and every-start initialisation: universe sync, operator account, paper account,
//! default strategy and built-in reminders. Everything here is idempotent.

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
use crate::store::{strategy as strategy_store, system, trading};
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

    // Paper account.
    let account = match trading::active_account(&client).await? {
        Some(account) => account,
        None => {
            let (first_session, base_date) = portfolio::inception_dates(state, state.now());
            let cash = Decimal::from_f64_retain(state.config.initial_cash)
                .unwrap_or(Decimal::ONE_THOUSAND)
                .round_dp(2);
            let tx = client.transaction().await?;
            let account = trading::create_account(&tx, "Paper", cash, first_session).await?;
            trading::upsert_nav(&tx, account.id, base_date, cash, cash, Decimal::ZERO).await?;
            system::audit(
                &tx,
                "system",
                "account.created",
                "account",
                &account.id.to_string(),
                json!({"initial_cash": cash}),
                "",
            )
            .await?;
            tx.commit().await?;
            tracing::info!(initial_cash = %cash, "created the paper account");
            account
        }
    };

    // Strategy.
    if strategy_store::active_version(&client, account.id)
        .await?
        .is_none()
    {
        let p = preset(DEFAULT_PRESET).expect("default preset exists");
        let version = strategy_store::insert_version(
            &client,
            p.name_en,
            p.id,
            &p.params,
            "default strategy",
            "system",
        )
        .await?;
        strategy_store::activate(
            &client,
            account.id,
            version.id,
            "system",
            "initial activation",
        )
        .await?;
        system::audit(
            &client,
            "system",
            "strategy.activated",
            "strategy_version",
            &version.id.to_string(),
            json!({"preset": p.id}),
            "",
        )
        .await?;
    }

    reminders::seed_builtins(&client).await?;
    system::fail_interrupted_backtests(&client).await?;
    Ok(())
}
