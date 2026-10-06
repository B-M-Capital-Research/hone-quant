//! `hone-quant` — paper-trading quant app for the honeclaw AI-infrastructure universe.
//!
//! `hone-quant serve` (the default) runs the web UI/API, the scheduler and the paper broker in
//! one process. The other subcommands are operational tools.

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use quant_core::calendar::MarketCalendar;
use quant_server::config::{Config, LogFormat};
use quant_server::market::demo::{DemoInstrument, DemoMarket};
use quant_server::market::fmp::FmpClient;
use quant_server::market::{DataSource, MarketData};
use quant_server::services::{bootstrap, marketdata, scheduler};
use quant_server::state::{AppState, offset_clock, system_clock};
use quant_server::store::system;
use quant_server::{api, auth, config, crypto, db, honeclaw_auth, universe};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "hone-quant",
    version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("HONE_QUANT_REVISION_OR_DEV"), ")"),
    about = "Paper-trading quant app for the honeclaw AI-infrastructure universe (US equities)."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the web UI, API, scheduler and paper broker (default).
    Serve,
    /// Apply database migrations and exit.
    Migrate,
    /// Manage operator accounts.
    User {
        #[command(subcommand)]
        command: UserCommand,
    },
    /// Universe maintenance (derived from the honeclaw ontology).
    Universe {
        #[command(subcommand)]
        command: UniverseCommand,
    },
    /// Probe the configured FMP keys against every endpoint hone-quant uses.
    FmpCheck {
        #[arg(long, default_value = "NVDA")]
        symbol: String,
    },
    /// Fetch market data now.
    Sync {
        /// Re-download the full history window instead of topping it up.
        #[arg(long)]
        full: bool,
    },
}

#[derive(Subcommand)]
enum UserCommand {
    /// Create an operator (password read from the terminal or stdin).
    Add {
        username: String,
        #[arg(long, default_value = "admin", value_parser = ["admin", "viewer"])]
        role: String,
    },
    /// Set a new password for an operator and sign out their sessions.
    Passwd { username: String },
    /// List operators.
    List,
}

#[derive(Subcommand)]
enum UniverseCommand {
    /// Build a universe file from the honeclaw ontology (and optionally its live edit log).
    Build {
        /// Path or URL of honeclaw's skills/industry-map/references/industry-map.json.
        #[arg(long, default_value = universe::ONTOLOGY_URL)]
        ontology: String,
        /// Path or URL of honeclaw's data/industry_map/edits.json.
        #[arg(long)]
        edits: Option<String>,
        #[arg(long, default_value = "config/universe.json")]
        out: PathBuf,
    },
    /// Compare the ontology with the database and optionally apply the changes.
    Sync {
        #[arg(long, default_value = universe::ONTOLOGY_URL)]
        ontology: String,
        #[arg(long)]
        edits: Option<String>,
        #[arg(long)]
        apply: bool,
    },
}

fn init_logging(format: LogFormat) {
    let filter = EnvFilter::try_from_env("HONE_QUANT_LOG").unwrap_or_else(|_| {
        EnvFilter::new("info,tower_http=warn,deadpool=warn,tokio_postgres=warn")
    });
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false);
    match format {
        LogFormat::Json => builder.json().init(),
        LogFormat::Text => builder.init(),
    }
}

fn read_password(prompt: &str) -> Result<String> {
    if std::io::stdin().is_terminal() {
        let first = rpassword::prompt_password(prompt)?;
        let second = rpassword::prompt_password("Repeat: ")?;
        if first != second {
            bail!("passwords do not match");
        }
        Ok(first)
    } else {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }
}

async fn connect(config: &Config) -> Result<deadpool_postgres::Pool> {
    let pool = db::create_pool(&config.db)?;
    let applied = db::migrate(&pool, &config.db.schema, config.market.source).await?;
    if !applied.is_empty() {
        tracing::info!(?applied, schema = %config.db.schema, "applied migrations");
    }
    Ok(pool)
}

fn market_source(
    config: &Config,
    clock: quant_server::state::Clock,
) -> Result<Arc<dyn MarketData>> {
    Ok(match config.market.source {
        DataSource::Fmp => Arc::new(FmpClient::new(config.market.fmp.clone())?),
        DataSource::Demo => {
            let file = universe::load(config.universe_path.as_deref())?;
            let mut instruments: Vec<DemoInstrument> = file
                .assets
                .iter()
                .map(|a| DemoInstrument {
                    symbol: a.symbol.clone(),
                    sector: a.sector.clone(),
                })
                .collect();
            instruments.extend(file.benchmarks.iter().map(|b| DemoInstrument {
                symbol: b.symbol.clone(),
                sector: "benchmark".into(),
            }));
            Arc::new(DemoMarket::new(instruments, config.market.demo_seed, clock))
        }
    })
}

async fn build_state(config: Config) -> Result<Arc<AppState>> {
    let pool = connect(&config).await?;
    let mut calendar = MarketCalendar::nyse();
    for date in &config.extra_closures {
        calendar.add_closure(*date);
    }
    let clock = match (config.dev_clock_start, config.dev_clock_offset) {
        (Some(start), Some(offset)) => {
            tracing::warn!(%start, "demo clock offset active");
            offset_clock(offset)
        }
        _ => system_clock(),
    };
    let market = market_source(&config, clock.clone())?;
    let (events, _) = tokio::sync::broadcast::channel(256);
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(concat!("hone-quant/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let started_at = clock();
    let honeclaw = match &config.auth {
        config::AuthMode::Local => None,
        config::AuthMode::Honeclaw(settings) => {
            tracing::info!(endpoint = %settings.me_url, "sign-in through honeclaw: administrators only");
            Some(Arc::new(honeclaw_auth::HoneclawVerifier::new(
                settings.clone(),
            )?))
        }
    };
    Ok(Arc::new(AppState {
        honeclaw,
        secrets: crypto::SecretBox::new(&config.secret_key),
        config: Arc::new(config),
        pool,
        market,
        calendar: Arc::new(calendar),
        clock,
        events,
        http,
        limiter: auth::LoginLimiter::default(),
        trading_lock: tokio::sync::Mutex::new(()),
        started_at,
    }))
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutting down");
}

async fn serve(config: Config) -> Result<()> {
    let bind = config.bind;
    let source = config.market.source;
    let schema = config.db.schema.clone();
    let target = config.db.display.clone();
    if config.db.borrowed_from_honeclaw {
        tracing::info!(
            "using honeclaw's HONE_POSTGRES_* settings; hone-quant tables are isolated in schema {schema}"
        );
    }
    let state = build_state(config).await?;
    bootstrap::run(&state).await?;
    if state.config.scheduler_enabled {
        scheduler::spawn(state.clone());
    } else {
        tracing::warn!(
            "scheduler disabled (HONE_QUANT_SCHEDULER=false): no plans will be generated or executed"
        );
    }
    quant_server::web::check_build(&state.config)?;
    let app = api::app(state.clone());
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("cannot bind {bind}"))?;
    tracing::info!(%bind, source = source.as_str(), database = %target, "hone-quant is listening (paper trading only)");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    config::load_dotenv();
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Serve);

    // Offline commands that need no configuration.
    if let Command::Universe {
        command:
            UniverseCommand::Build {
                ontology,
                edits,
                out,
            },
    } = &command
    {
        let doc = universe::read_json(ontology).await?;
        let edits_doc = match edits {
            Some(location) => Some(universe::read_json(location).await?),
            None => None,
        };
        let built = universe::build_from_ontology(
            &doc,
            edits_doc.as_ref(),
            &universe::overlay(),
            ontology,
        )?;
        std::fs::write(out, serde_json::to_string_pretty(&built)? + "\n")?;
        println!(
            "wrote {} ({} sectors, {} companies, {} edits applied)",
            out.display(),
            built.sectors.len(),
            built.assets.len(),
            built.source.edits_applied
        );
        return Ok(());
    }

    let config = Config::from_env()?;
    init_logging(config.log_format);
    match command {
        Command::Serve => serve(config).await,
        Command::Migrate => {
            let schema = config.db.schema.clone();
            connect(&config).await?;
            println!("schema {schema} is up to date");
            Ok(())
        }
        Command::User { command } => {
            let pool = connect(&config).await?;
            let client = pool.get().await?;
            match command {
                UserCommand::Add { username, role } => {
                    if system::user_by_name(&client, &username).await?.is_some() {
                        bail!("user {username} already exists");
                    }
                    let password = read_password(&format!("Password for {username}: "))?;
                    auth::check_password_strength(&password).map_err(anyhow::Error::msg)?;
                    system::create_user(
                        &client,
                        &username,
                        &auth::hash_password(&password)?,
                        &role,
                    )
                    .await?;
                    system::audit(
                        &client,
                        "cli",
                        "user.created",
                        "user",
                        &username,
                        serde_json::json!({"role": role}),
                        "",
                    )
                    .await?;
                    println!("created {role} {username}");
                }
                UserCommand::Passwd { username } => {
                    let user = system::user_by_name(&client, &username)
                        .await?
                        .with_context(|| format!("no user {username}"))?;
                    let password = read_password(&format!("New password for {username}: "))?;
                    auth::check_password_strength(&password).map_err(anyhow::Error::msg)?;
                    system::set_password(&client, user.id, &auth::hash_password(&password)?)
                        .await?;
                    system::delete_user_sessions(&client, user.id, None).await?;
                    system::audit(
                        &client,
                        "cli",
                        "auth.password_reset",
                        "user",
                        &username,
                        serde_json::json!({}),
                        "",
                    )
                    .await?;
                    println!("password updated; existing sessions signed out");
                }
                UserCommand::List => {
                    for user in system::users(&client).await? {
                        println!(
                            "{:<24} {:<7} last login {:?}",
                            user.username, user.role, user.last_login_at
                        );
                    }
                }
            }
            Ok(())
        }
        Command::Universe {
            command:
                UniverseCommand::Sync {
                    ontology,
                    edits,
                    apply,
                },
        } => {
            let pool = connect(&config).await?;
            let doc = universe::read_json(&ontology).await?;
            let edits_doc = match &edits {
                Some(location) => Some(universe::read_json(location).await?),
                None => None,
            };
            let built = universe::build_from_ontology(
                &doc,
                edits_doc.as_ref(),
                &universe::overlay(),
                &ontology,
            )?;
            let mut client = pool.get().await?;
            let changes = universe::diff(&client, &built).await?;
            println!("{}", serde_json::to_string_pretty(&changes)?);
            if apply {
                universe::sync_to_db(&mut client, &built, "cli").await?;
                system::audit(
                    &client,
                    "cli",
                    "universe.applied",
                    "universe",
                    "",
                    serde_json::json!({"changes": changes}),
                    "",
                )
                .await?;
                println!("applied");
            } else {
                println!("dry run; pass --apply to write these changes");
            }
            Ok(())
        }
        Command::Universe {
            command: UniverseCommand::Build { .. },
        } => unreachable!("handled above"),
        Command::FmpCheck { symbol } => {
            let client = FmpClient::new(config.market.fmp.clone())?;
            println!(
                "keys: {} (from {})",
                config.market.fmp.api_keys.len(),
                config.market.key_origin
            );
            let checks = quant_server::market::fmp::diagnose(&client, &symbol).await;
            let mut failures = 0;
            for check in &checks {
                if !check.ok {
                    failures += 1;
                }
                println!(
                    "{:<4} {:<7} {:<32} {}",
                    if check.ok { "ok" } else { "FAIL" },
                    check.api,
                    check.endpoint,
                    check.detail
                );
            }
            if failures > 0 && checks.iter().all(|c| !c.ok) {
                bail!("no endpoint worked with the configured keys");
            }
            Ok(())
        }
        Command::Sync { full } => {
            let state = build_state(config).await?;
            bootstrap::run(&state).await?;
            let report = marketdata::sync_daily(&state, 10, full).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            let quotes = marketdata::poll_quotes(&state).await?;
            println!("quotes: {quotes}");
            Ok(())
        }
    }
}
