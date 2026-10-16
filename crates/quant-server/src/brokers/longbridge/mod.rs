//! Longbridge, through its open-source `longbridge` command-line client
//! (<https://github.com/longbridge/longbridge-terminal>).
//!
//! Trading and account data go through `longbridge serve`, the client's interface for other
//! software: JSON-RPC 2.0 over standard input and output carrying Longbridge OpenAPI's raw
//! payloads (exact decimals, documented shapes), unlike `--format json`, which the client
//! reshapes for people and AI agents and may change. Two one-shot commands complete it:
//! `--version` and `auth status` (which account is signed in). Shapes are parsed in [`wire`].
//!
//! The client keeps its own OAuth sign-in (an encrypted token under `$HOME/.longbridge`, bound
//! to the machine), so hone-quant never sees Longbridge credentials: an operator signs in once
//! on the server with `hone-quant longbridge auth login`, which runs the client with exactly
//! the environment the server gives it.
//!
//! Safety:
//! - A paper client runs with `LONGBRIDGE_PAPERTRADING=true`. The Longbridge SDK then marks
//!   every trading request as paper trading, and Longbridge's servers refuse it when the
//!   signed-in account is a real-money one: a paper portfolio cannot reach real money even if
//!   someone signs the client in to a live account. The account channel reported with the
//!   positions and by `auth status` is checked as well.
//! - Placing or cancelling an order is two-step in this client: a dry run returns the order
//!   with a confirmation code, and only the same request with that code acts. The dry run is
//!   checked against the intended order before the code is used.
//! - Calls are serialised and spaced a second apart (Longbridge allows 30 trade calls in 30 s),
//!   the client runs in an empty working directory (its SDK would read a `.env` there), and
//!   analytics are off for these automated calls (`DO_NOT_TRACK=1`).

mod serve;
pub mod wire;

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use quant_core::costs::Side;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::Mutex;

use super::{
    AccountMode, Broker, BrokerError, BrokerOrder, ConnectionStatus, Execution, Identity,
    OrderRequest, Snapshot,
};

/// How long a one-shot command may take (near token expiry the client can wait minutes for a
/// browser that will never come; it is killed instead).
const ONE_SHOT_TIMEOUT: Duration = Duration::from_secs(60);
/// The signed-in account is re-read at most this often.
const IDENTITY_TTL: Duration = Duration::from_secs(600);

/// Where the client is and which home directory (sign-in) it uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongbridgeConfig {
    /// The `longbridge` executable: a path, or a name looked up on `PATH`.
    pub bin: PathBuf,
    /// `HOME` for the client (its sign-in lives in `$HOME/.longbridge`); `None` keeps the
    /// server's own.
    pub home: Option<PathBuf>,
}

impl Default for LongbridgeConfig {
    fn default() -> Self {
        Self {
            bin: PathBuf::from("longbridge"),
            home: None,
        }
    }
}

pub(crate) fn spawn_error(error: std::io::Error) -> BrokerError {
    match error.kind() {
        std::io::ErrorKind::NotFound => BrokerError::Unavailable(
            "the Longbridge command-line client is not installed (see https://github.com/longbridge/longbridge-terminal; set HONE_QUANT_LONGBRIDGE_BIN if it is not on PATH)".into(),
        ),
        _ => BrokerError::Unavailable(format!("cannot start the Longbridge client: {error}")),
    }
}

pub struct LongbridgeCli {
    config: LongbridgeConfig,
    mode: AccountMode,
    serve: serve::Serve,
    /// Serialises one-shot commands with each other (concurrent token refreshes would race on
    /// the same token file).
    one_shot: Mutex<()>,
    identity: Mutex<Option<(Identity, Instant)>>,
    workdir: PathBuf,
}

impl LongbridgeCli {
    pub fn new(config: LongbridgeConfig, mode: AccountMode) -> Self {
        let workdir = std::env::temp_dir().join(format!("hone-quant-longbridge-{}", mode.as_str()));
        Self {
            config,
            mode,
            serve: serve::Serve::default(),
            one_shot: Mutex::new(()),
            identity: Mutex::new(None),
            workdir,
        }
    }

    /// The client with the environment every call gets (also used by `hone-quant longbridge`
    /// to run it interactively).
    pub fn command(config: &LongbridgeConfig, mode: AccountMode) -> Command {
        let mut command = Command::new(&config.bin);
        if let Some(home) = &config.home {
            command.env("HOME", home);
        }
        command.env_remove("LONGBRIDGE_PAPERTRADING");
        command.env_remove("LONGPORT_PAPERTRADING");
        if mode == AccountMode::Paper {
            command.env("LONGBRIDGE_PAPERTRADING", "true");
        }
        command.env("DO_NOT_TRACK", "1").env("NO_COLOR", "1");
        command
    }

    fn workdir(&self) -> Result<&PathBuf, BrokerError> {
        std::fs::create_dir_all(&self.workdir).map_err(|e| {
            BrokerError::Unavailable(format!("cannot create {}: {e}", self.workdir.display()))
        })?;
        Ok(&self.workdir)
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value, BrokerError> {
        let workdir = self.workdir()?.clone();
        let _one_at_a_time = self.one_shot.lock().await;
        self.serve
            .call(
                || Self::command(&self.config, self.mode),
                &workdir,
                method,
                params,
            )
            .await
    }

    /// Runs a one-shot command and returns its standard output.
    async fn one_shot(&self, args: &[&str]) -> Result<String, BrokerError> {
        let workdir = self.workdir()?.clone();
        let _one_at_a_time = self.one_shot.lock().await;
        let mut command = Self::command(&self.config, self.mode);
        command
            .args(args)
            .current_dir(&workdir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = command.spawn().map_err(spawn_error)?;
        let shown = format!("longbridge {}", args.join(" "));
        let output = tokio::time::timeout(ONE_SHOT_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| {
                BrokerError::Failed(format!(
                    "`{shown}` did not finish within {}s",
                    ONE_SHOT_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|error| BrokerError::Failed(format!("`{shown}`: {error}")))?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if output.status.success() {
            Ok(stdout)
        } else {
            Err(wire::classify_failure(
                &String::from_utf8_lossy(&output.stderr),
                &stdout,
            ))
        }
    }

    async fn auth_status(&self) -> Result<wire::AuthStatus, BrokerError> {
        let stdout = self
            .one_shot(&["auth", "status", "--format", "json"])
            .await?;
        wire::AuthStatus::parse(&wire::extract_json(&stdout)?)
    }

    /// The account the client is signed in to (re-read every [`IDENTITY_TTL`]).
    async fn identity(&self) -> Result<Identity, BrokerError> {
        if let Some((identity, at)) = self.identity.lock().await.as_ref()
            && at.elapsed() < IDENTITY_TTL
        {
            return Ok(identity.clone());
        }
        let identity = self.auth_status().await?.identity(self.mode)?;
        *self.identity.lock().await = Some((identity.clone(), Instant::now()));
        Ok(identity)
    }

    /// Sends a gated request as a dry run, checks the dry run with `check`, then sends it again
    /// with the confirmation code.
    async fn confirmed(
        &self,
        method: &str,
        params: Value,
        check: impl FnOnce(&Value) -> Result<(), BrokerError>,
    ) -> Result<Value, BrokerError> {
        let dry_run = self.rpc(method, params.clone()).await?;
        let code = wire::confirmation_code(&dry_run)?;
        check(&dry_run)?;
        let mut params = params;
        params["execute"] = Value::String(code);
        self.rpc(method, params).await
    }
}

#[async_trait]
impl Broker for LongbridgeCli {
    fn name(&self) -> &'static str {
        super::LONGBRIDGE
    }

    fn mode(&self) -> AccountMode {
        self.mode
    }

    fn broker_symbol(&self, symbol: &str) -> String {
        format!("{symbol}.US")
    }

    async fn status(&self) -> ConnectionStatus {
        let mut status = ConnectionStatus {
            broker: super::LONGBRIDGE,
            mode: self.mode,
            client: None,
            signed_in: false,
            identity: None,
            error: None,
            checked_at: Utc::now(),
        };
        match self.one_shot(&["--version"]).await {
            Ok(version) => status.client = Some(version.trim().to_string()),
            Err(error) => {
                status.error = Some(error.to_string());
                return status;
            }
        }
        match self.auth_status().await {
            Ok(auth) => {
                status.signed_in = auth.signed_in();
                match auth.identity(self.mode) {
                    Ok(identity) => {
                        *self.identity.lock().await = Some((identity.clone(), Instant::now()));
                        status.identity = Some(identity);
                    }
                    Err(error) => status.error = Some(error.to_string()),
                }
            }
            Err(error) => status.error = Some(error.to_string()),
        }
        status
    }

    async fn snapshot(&self) -> Result<Snapshot, BrokerError> {
        let identity = self.identity().await?;
        let balances = self
            .rpc("trade.account_balance", json!({"currency": "USD"}))
            .await?;
        let positions = self.rpc("trade.stock_positions", json!({})).await?;
        let (channels, holdings) = wire::parse_positions(&positions)?;
        for channel in channels {
            let mode = wire::channel_mode(&channel);
            if mode != self.mode {
                *self.identity.lock().await = None;
                return Err(BrokerError::WrongAccount(format!(
                    "the Longbridge account reports channel {channel} ({} account), not a {} account",
                    mode.as_str(),
                    self.mode.as_str()
                )));
            }
        }
        Ok(Snapshot {
            identity,
            cash: wire::parse_balances(&balances)?,
            holdings,
        })
    }

    async fn submit(&self, order: &OrderRequest) -> Result<String, BrokerError> {
        if order.qty <= Decimal::ZERO || order.qty.fract() != Decimal::ZERO {
            return Err(BrokerError::Refused(format!(
                "Longbridge orders are for whole shares (got {})",
                order.qty
            )));
        }
        let params = json!({
            "symbol": order.broker_symbol,
            "side": match order.side { Side::Buy => "buy", Side::Sell => "sell" },
            "order_type": "LO",
            "quantity": order.qty.trunc().normalize().to_string(),
            "price": order.limit_price.normalize().to_string(),
            "time_in_force": "day",
            "outside_rth": "RTH_ONLY",
            "remark": order.remark,
        });
        let placed = self
            .confirmed("trade.submit_order", params, |dry_run| {
                wire::check_order_preview(dry_run, order)
            })
            .await?;
        wire::placed_order_id(&placed)
    }

    async fn cancel(&self, order_id: &str) -> Result<(), BrokerError> {
        self.confirmed(
            "trade.cancel_order",
            json!({"order_id": order_id}),
            |dry_run| wire::check_cancel_preview(dry_run, order_id),
        )
        .await?;
        Ok(())
    }

    async fn today_orders(&self) -> Result<Vec<BrokerOrder>, BrokerError> {
        wire::parse_orders(&self.rpc("trade.today_orders", json!({})).await?)
    }

    async fn order(&self, order_id: &str) -> Result<BrokerOrder, BrokerError> {
        wire::parse_order(
            &self
                .rpc("trade.order_detail", json!({"order_id": order_id}))
                .await?,
        )
    }

    async fn today_executions(&self) -> Result<Vec<Execution>, BrokerError> {
        wire::parse_executions(&self.rpc("trade.today_executions", json!({})).await?)
    }
}

#[cfg(test)]
mod tests;
