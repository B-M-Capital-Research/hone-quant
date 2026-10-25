//! Forming the AI sector view, choosing the one a plan may use, and measuring what it did.
//!
//! - [`generate`] gathers the briefing (ontology digest, market statistics, earnings against
//!   consensus), asks the model, validates the answer and stores the attempt, successful or not.
//!   The scheduler runs it once per trading day before the open, and only when some active
//!   portfolio's strategy consults the view; administrators can also run it by hand. Attempts
//!   are capped per day because the model's quota is shared with honeclaw.
//! - [`usable`] is the newest successful view within the strategy's age limit.
//! - [`apply`] runs the engine with and without the view and records both, whichever is traded.
//! - [`evaluate`] measures, forward, what trading with the view would have changed.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use futures::StreamExt;
use quant_core::calendar::MarketCalendar;
use quant_core::strategy::{AiMode, AiParams, EngineInput, SectorView, TargetResult};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ai::material::{self, SectorMembers};
use crate::ai::ontology;
use crate::ai::view::{self, PROMPT_VERSION, ViewOutput};
use crate::market::{EarningsReport, MarketError};
use crate::state::{AppState, ServerEvent};
use crate::store::ai::{self as store, NewView, StoredView};
use crate::store::{market, portfolios, strategy as strategy_store};
use crate::universe;

/// Model calls allowed in any 24 hours (scheduled and manual together).
pub const MAX_ATTEMPTS_PER_DAY: i64 = 4;
/// Calendar days of prices the market statistics need (a year of sessions plus slack).
const PRICE_DAYS: i64 = 400;

/// Only one view is formed at a time.
static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Serialize)]
pub struct Generated {
    pub id: i64,
    pub status: String,
    pub as_of: NaiveDate,
    pub error: Option<String>,
}

/// Whether any active portfolio's strategy consults the view.
pub async fn needed(state: &AppState) -> Result<bool> {
    let client = state.pool.get().await?;
    for book in portfolios::active_books(&client).await? {
        if let Some(version) = strategy_store::active_version(&client, book.account.id).await?
            && version.strategy_params()?.ai.mode != AiMode::Off
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether the model was asked [`MAX_ATTEMPTS_PER_DAY`] times in the last 24 hours.
pub async fn capped(state: &AppState) -> Result<bool> {
    let client = state.pool.get().await?;
    let since = state.now() - Duration::hours(24);
    Ok(store::attempts_since(&client, since).await? >= MAX_ATTEMPTS_PER_DAY)
}

/// Whether a view is being formed right now.
pub fn running() -> bool {
    RUNNING.try_lock().is_err()
}

/// Forms and stores a view from the material up to the last close before today. Errors only
/// when nothing could be attempted (no model, a view already in progress, the daily cap, no
/// prices); a failed model call or an unusable answer is stored as a failed view.
pub async fn generate(state: &Arc<AppState>, actor: &str) -> Result<Generated> {
    let model = state
        .llm
        .clone()
        .ok_or_else(|| anyhow!("no model is configured for the AI sector view"))?;
    let _running = RUNNING
        .try_lock()
        .map_err(|_| anyhow!("an AI sector view is already being formed"))?;
    let now = state.now();
    let client = state.pool.get().await?;
    // Nothing else can be running (the lock above): older `running` rows were interrupted.
    store::fail_interrupted(&client, now).await?;
    let attempts = store::attempts_since(&client, now - Duration::hours(24)).await?;
    if attempts >= MAX_ATTEMPTS_PER_DAY {
        bail!("the model was already asked {attempts} times in the last 24 hours");
    }
    let today = MarketCalendar::local_date(now);
    let sectors = universe_sectors(&client).await?;
    let symbols: Vec<String> = sectors.iter().flat_map(|s| s.symbols.clone()).collect();
    let benchmarks: Vec<String> = universe::bundled()
        .benchmarks
        .into_iter()
        .map(|b| b.symbol)
        .collect();
    let all: Vec<String> = symbols.iter().chain(&benchmarks).cloned().collect();
    let panel = market::adjusted_panel(
        &client,
        &all,
        today - Duration::days(PRICE_DAYS),
        Some(today - Duration::days(1)),
    )
    .await?;
    drop(client);
    let as_of = panel
        .iter()
        .filter(|(symbol, _)| symbols.contains(symbol))
        .filter_map(|(_, rows)| rows.last().map(|(d, _, _)| *d))
        .max()
        .ok_or_else(|| anyhow!("no prices before {today}: sync market data first"))?;
    let closes: HashMap<String, Vec<(NaiveDate, f64)>> = panel
        .into_iter()
        .map(|(symbol, rows)| {
            let series = rows
                .into_iter()
                .filter(|(d, _, c)| *d <= as_of && c.is_finite() && *c > 0.0)
                .map(|(d, _, c)| (d, c))
                .collect();
            (symbol, series)
        })
        .collect();

    let mut notes = Vec::new();
    let ontology = match universe::read_json(&state.config.ai.ontology).await {
        Ok(mut ontology) => {
            // honeclaw's live edits carry the newest research; without them the base file is
            // still complete, only older.
            let mut applied = None;
            if let Some(location) = &state.config.ai.ontology_edits {
                match universe::read_json(location).await {
                    Ok(edits) => applied = Some(ontology::apply_edits(&mut ontology, &edits)),
                    Err(error) => notes.push(format!("ontology edits unavailable: {error:#}")),
                }
            }
            let mut digest = material::ontology_digest(&ontology, &sectors, as_of);
            digest["edits_applied"] = json!(applied);
            digest
        }
        Err(error) => {
            notes.push(format!("ontology unavailable: {error:#}"));
            Value::Null
        }
    };
    let (reports, failed) = earnings(state, &symbols).await;
    if failed > 0 {
        notes.push(format!(
            "earnings unavailable for {failed} of {} companies",
            symbols.len()
        ));
    }
    let briefing = json!({
        "as_of": as_of,
        "ontology": ontology,
        "market": material::market_stats(&sectors, &closes, &benchmarks),
        "earnings": material::earnings_summary(&sectors, &reports, as_of),
        "notes": notes,
    });
    let hash = hex::encode(Sha256::digest(briefing.to_string().as_bytes()));
    let sector_ids: Vec<String> = sectors.iter().map(|s| s.id.clone()).collect();
    let system = view::system_prompt(AiParams::default().max_tilt);
    let user = view::user_prompt(as_of, &sector_ids, &briefing);

    // The attempt is recorded before the call, so it counts toward the cap whatever happens next.
    let client = state.pool.get().await?;
    let id = store::insert_running(
        &client,
        &NewView {
            as_of,
            prompt_version: PROMPT_VERSION,
            model: model.model(),
            material: &briefing,
            material_hash: &hash,
            created_by: actor,
        },
    )
    .await?;
    drop(client);
    state.emit(ServerEvent::AiView {
        id,
        status: "running".into(),
    });

    let answer = model.complete_json(&system, &user).await.map(|mut c| {
        // PostgreSQL text and jsonb cannot hold NUL.
        c.content = c.content.replace('\0', "");
        c
    });
    let (status, output, raw, error, usage, latency, reported_model) = match answer {
        Ok(completion) => match view::parse_view(&completion.content, &sector_ids, as_of) {
            Ok(parsed) => (
                "ok",
                Some(serde_json::to_value(&parsed)?),
                Some(completion.content),
                None,
                completion.usage,
                Some(completion.latency_ms),
                completion.model,
            ),
            Err(problem) => (
                "failed",
                None,
                Some(completion.content),
                Some(format!("unusable answer: {problem}")),
                completion.usage,
                Some(completion.latency_ms),
                completion.model,
            ),
        },
        Err(error) => (
            "failed",
            None,
            None,
            Some(error.to_string()),
            None,
            None,
            String::new(),
        ),
    };
    let model_name = if reported_model.is_empty() || reported_model == model.model() {
        model.model().to_string()
    } else {
        format!("{} ({reported_model})", model.model())
    };
    let client = state.pool.get().await?;
    store::finish(
        &client,
        id,
        &store::Outcome {
            status,
            model: &model_name,
            output,
            raw_response: raw.as_deref(),
            error: error.as_deref(),
            usage: usage.as_ref(),
            latency_ms: latency.map(|ms| ms.min(i32::MAX as u64) as i32),
        },
    )
    .await?;
    drop(client);
    match &error {
        None => tracing::info!(view = id, %as_of, model = %model_name, "AI sector view formed"),
        Some(error) => tracing::warn!(view = id, %as_of, %error, "AI sector view failed"),
    }
    state.emit(ServerEvent::AiView {
        id,
        status: status.into(),
    });
    Ok(Generated {
        id,
        status: status.into(),
        as_of,
        error,
    })
}

/// The universe's active sectors with their active members, in display order.
async fn universe_sectors(
    client: &impl deadpool_postgres::GenericClient,
) -> Result<Vec<SectorMembers>> {
    let assets = market::assets(client, false).await?;
    Ok(market::sectors(client)
        .await?
        .into_iter()
        .map(|s| SectorMembers {
            symbols: assets
                .iter()
                .filter(|a| a.sector_id == s.id)
                .map(|a| a.symbol.clone())
                .collect(),
            id: s.id,
            name_zh: s.name_zh,
            name_en: s.name_en,
        })
        .filter(|s| !s.symbols.is_empty())
        .collect())
}

/// A company's reports, or why not (`true`: the data plan refuses earnings altogether).
type Reports = Result<Vec<EarningsReport>, (bool, String)>;

/// One company's reports, bounded in time.
async fn earnings_of(
    market: Arc<dyn crate::market::MarketData>,
    symbol: String,
) -> (String, Reports) {
    let reports =
        tokio::time::timeout(std::time::Duration::from_secs(60), market.earnings(&symbol)).await;
    let result = match reports {
        Ok(Ok(list)) => Ok(list),
        Ok(Err(error)) => Err((
            matches!(
                error,
                MarketError::PlanRestricted(_) | MarketError::NotConfigured(_)
            ),
            error.to_string(),
        )),
        Err(_) => Err((false, "timed out".to_string())),
    };
    (symbol, result)
}

/// Recent and scheduled reports per company, best effort; also how many companies failed. The
/// first company is asked alone: when the data plan does not include earnings, that one refusal
/// stands for all of them instead of one refused request per company. Any other failure (a
/// timeout, a rate limit) costs only that company.
async fn earnings(
    state: &AppState,
    symbols: &[String],
) -> (HashMap<String, Vec<EarningsReport>>, usize) {
    let Some((first, rest)) = symbols.split_first() else {
        return (HashMap::new(), 0);
    };
    let mut reports = HashMap::new();
    let mut failed = 0;
    match earnings_of(state.market.clone(), first.clone()).await {
        (symbol, Ok(list)) => {
            reports.insert(symbol, list);
        }
        (symbol, Err((true, error))) => {
            tracing::info!(%symbol, %error, "earnings not in the data plan; skipping the rest");
            return (reports, symbols.len());
        }
        (symbol, Err((false, error))) => {
            tracing::debug!(%symbol, %error, "earnings unavailable");
            failed += 1;
        }
    }
    // Owned symbols and market handle: a stream of futures borrowing from the closure's
    // argument would not be `Send` for every lifetime, and the scheduler spawns this.
    let results: Vec<(String, Reports)> = futures::stream::iter(rest.to_vec())
        .map(|symbol| earnings_of(state.market.clone(), symbol))
        .buffer_unordered(4)
        .collect()
        .await;
    for (symbol, result) in results {
        match result {
            Ok(list) => {
                reports.insert(symbol, list);
            }
            Err((_, error)) => {
                tracing::debug!(%symbol, %error, "earnings unavailable");
                failed += 1;
            }
        }
    }
    (reports, failed)
}

/// The newest successful view a plan on `trade_date` may use under `params`.
pub async fn usable(
    client: &impl deadpool_postgres::GenericClient,
    params: &AiParams,
    trade_date: NaiveDate,
) -> Result<Option<(StoredView, ViewOutput)>> {
    let oldest = trade_date - Duration::days(params.max_age_days as i64);
    Ok(store::latest_ok(client, oldest)
        .await?
        .filter(|v| v.as_of < trade_date)
        .and_then(|v| v.view().map(|view| (v, view))))
}

// ---------------------------------------------------------------------------------------------
// Applying a view to a plan
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AiSector {
    pub sector: String,
    pub stance: i8,
    pub confidence: f64,
    pub multiplier: f64,
    /// The sector's final weight with and without the view.
    pub weight_with: f64,
    pub weight_without: f64,
}

/// What the AI sector view did to one plan; stored as `diagnostics.ai`.
#[derive(Debug, Clone, Serialize)]
pub struct AiApplied {
    pub mode: AiMode,
    /// `live` (traded with the view), `shadow` (recorded only) or `unavailable` (no usable
    /// view: the plan is the engine's alone).
    pub status: &'static str,
    pub view_id: Option<i64>,
    pub as_of: Option<NaiveDate>,
    pub model: Option<String>,
    pub formed_at: Option<DateTime<Utc>>,
    pub max_tilt: f64,
    pub min_confidence: f64,
    pub max_age_days: u32,
    pub sectors: Vec<AiSector>,
    /// Target weights (non-zero) with and without the view.
    pub weights_with: BTreeMap<String, f64>,
    pub weights_without: BTreeMap<String, f64>,
    /// Σ|with − without| / 2: how much of the portfolio the view moves.
    pub active_share: f64,
}

fn nonzero(symbols: &[String], weights: &[f64]) -> BTreeMap<String, f64> {
    symbols
        .iter()
        .zip(weights)
        .filter(|(_, w)| **w > 0.0)
        .map(|(s, w)| (s.clone(), *w))
        .collect()
}

/// Runs the engine for a plan: with the view in live mode, without it otherwise. When a view is
/// usable, the other run is recorded next to it, so both are always comparable.
pub fn apply(
    params: &quant_core::strategy::StrategyParams,
    input: &EngineInput,
    symbols: &[String],
    view: Option<&(StoredView, ViewOutput)>,
) -> (TargetResult, Option<AiApplied>) {
    let run = |views: &BTreeMap<String, SectorView>| {
        quant_core::strategy::compute_targets(
            params,
            &EngineInput {
                sector_views: views,
                ..*input
            },
        )
    };
    let ai = &params.ai;
    let none = BTreeMap::new();
    if ai.mode == AiMode::Off {
        return (run(&none), None);
    }
    let record = |status,
                  view: Option<&(StoredView, ViewOutput)>,
                  sectors,
                  with: &TargetResult,
                  without: &TargetResult| AiApplied {
        mode: ai.mode,
        status,
        view_id: view.map(|(v, _)| v.id),
        as_of: view.map(|(v, _)| v.as_of),
        model: view.map(|(v, _)| v.model.clone()),
        formed_at: view.map(|(v, _)| v.created_at),
        max_tilt: ai.max_tilt,
        min_confidence: ai.min_confidence,
        max_age_days: ai.max_age_days,
        sectors,
        weights_with: nonzero(symbols, &with.weights),
        weights_without: nonzero(symbols, &without.weights),
        active_share: with
            .weights
            .iter()
            .zip(&without.weights)
            .map(|(a, b)| (a - b).abs())
            .sum::<f64>()
            / 2.0,
    };
    let Some(found) = view else {
        let targets = run(&none);
        let applied = record("unavailable", None, Vec::new(), &targets, &targets);
        return (targets, Some(applied));
    };
    let views = found.1.sector_views();
    let with = run(&views);
    let without = run(&none);
    let sectors = with
        .sectors
        .iter()
        .map(|s| AiSector {
            sector: s.sector.clone(),
            stance: s.ai_stance.unwrap_or(0),
            confidence: views.get(&s.sector).map_or(0.0, |v| v.confidence),
            multiplier: s.ai_multiplier,
            weight_with: s.weight,
            weight_without: without
                .sectors
                .iter()
                .find(|o| o.sector == s.sector)
                .map_or(0.0, |o| o.weight),
        })
        .collect();
    if ai.mode == AiMode::Live {
        let applied = record("live", Some(found), sectors, &with, &without);
        (with, Some(applied))
    } else {
        let applied = record("shadow", Some(found), sectors, &with, &without);
        (without, Some(applied))
    }
}

// ---------------------------------------------------------------------------------------------
// Forward evaluation
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct OverlayDay {
    pub date: NaiveDate,
    /// Return of (with − without) target weights over the day, in basis points of NAV.
    pub bps: f64,
    pub cumulative_bps: f64,
    pub plan_id: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evaluation {
    pub plans: usize,
    pub days: usize,
    pub first_plan: Option<NaiveDate>,
    pub cumulative_bps: f64,
    pub mean_bps: Option<f64>,
    /// Share of days the view added return (days with no difference excluded).
    pub hit_rate: Option<f64>,
    /// Mean over its standard error; meaningless below a few dozen days.
    pub t_stat: Option<f64>,
    pub mean_active_share: Option<f64>,
    /// Cumulative contribution per sector, in basis points.
    pub sectors: BTreeMap<String, f64>,
    pub series: Vec<OverlayDay>,
}

fn weights_of(ai: &Value, key: &str) -> BTreeMap<String, f64> {
    ai.get(key)
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_f64()?)))
                .collect()
        })
        .unwrap_or_default()
}

/// What trading with the view instead of without it would have earned, day by day, measured on
/// adjusted closes. Each plan's two target vectors are held from the close of its trade date
/// until the next plan; a later plan without a view ends the holding, and days held under such a
/// plan are not counted. The view's target is first rescaled to the invested weight of the
/// target without it, so the overlay is a pure reallocation: with a volatility ceiling the view
/// can also move total exposure, and that market exposure is not skill. Costs of the extra trades
/// are not included.
pub fn evaluate_from(
    plans: &[store::PlanAi],
    closes: &HashMap<String, Vec<(NaiveDate, f64)>>,
    sector_of: &HashMap<String, String>,
    through: NaiveDate,
) -> Evaluation {
    // The last plan of each trade date wins.
    let mut by_date: BTreeMap<NaiveDate, &store::PlanAi> = BTreeMap::new();
    for plan in plans {
        by_date.insert(plan.trade_date, plan);
    }
    let returns: HashMap<&str, BTreeMap<NaiveDate, f64>> = closes
        .iter()
        .map(|(symbol, rows)| {
            let r = rows
                .windows(2)
                .filter(|w| w[0].1 > 0.0)
                .map(|w| (w[1].0, w[1].1 / w[0].1 - 1.0))
                .collect();
            (symbol.as_str(), r)
        })
        .collect();
    let mut dates: Vec<NaiveDate> = returns
        .values()
        .flat_map(|r| r.keys().copied())
        .filter(|d| *d <= through)
        .collect();
    dates.sort();
    dates.dedup();

    let mut series = Vec::new();
    let mut sectors: BTreeMap<String, f64> = BTreeMap::new();
    let mut cumulative = 0.0;
    for date in dates {
        // Weights decided on an earlier trade date are held over this day's close-to-close move.
        let Some((_, plan)) = by_date.range(..date).next_back() else {
            continue;
        };
        let Some(ai) = plan.ai.as_ref().filter(|_| plan.has_view()) else {
            continue;
        };
        let mut with = weights_of(ai, "weights_with");
        let without = weights_of(ai, "weights_without");
        let invested_with: f64 = with.values().sum();
        let invested_without: f64 = without.values().sum();
        if invested_with > 0.0 {
            let scale = invested_without / invested_with;
            with.values_mut().for_each(|w| *w *= scale);
        }
        let mut bps = 0.0;
        let mut seen = std::collections::BTreeSet::new();
        for symbol in with.keys().chain(without.keys()) {
            if !seen.insert(symbol) {
                continue;
            }
            let delta = with.get(symbol).unwrap_or(&0.0) - without.get(symbol).unwrap_or(&0.0);
            let r = returns
                .get(symbol.as_str())
                .and_then(|r| r.get(&date))
                .copied()
                .unwrap_or(0.0);
            let contribution = delta * r * 10_000.0;
            bps += contribution;
            if let Some(sector) = sector_of.get(symbol) {
                *sectors.entry(sector.clone()).or_default() += contribution;
            }
        }
        cumulative += bps;
        series.push(OverlayDay {
            date,
            bps,
            cumulative_bps: cumulative,
            plan_id: plan.plan_id,
        });
    }
    let n = series.len();
    let mean = (n > 0).then(|| series.iter().map(|d| d.bps).sum::<f64>() / n as f64);
    let t_stat = match (mean, n) {
        (Some(m), n) if n > 2 => {
            let var = series.iter().map(|d| (d.bps - m).powi(2)).sum::<f64>() / (n - 1) as f64;
            (var > 0.0).then(|| m / (var / n as f64).sqrt())
        }
        _ => None,
    };
    let decided: Vec<&OverlayDay> = series.iter().filter(|d| d.bps.abs() > 1e-9).collect();
    let hit_rate = (!decided.is_empty())
        .then(|| decided.iter().filter(|d| d.bps > 0.0).count() as f64 / decided.len() as f64);
    let with_view: Vec<&store::PlanAi> = plans.iter().filter(|p| p.has_view()).collect();
    let shares: Vec<f64> = with_view
        .iter()
        .filter_map(|p| p.ai.as_ref()?.get("active_share")?.as_f64())
        .collect();
    Evaluation {
        plans: with_view.len(),
        days: n,
        first_plan: with_view.first().map(|p| p.trade_date),
        cumulative_bps: cumulative,
        mean_bps: mean,
        hit_rate,
        t_stat,
        mean_active_share: (!shares.is_empty())
            .then(|| shares.iter().sum::<f64>() / shares.len() as f64),
        sectors,
        series,
    }
}

/// [`evaluate_from`] for an account, through the last close.
pub async fn evaluate(state: &AppState, account_id: i64) -> Result<Evaluation> {
    let client = state.pool.get().await?;
    let plans = store::plans_since_first_view(&client, account_id).await?;
    let mut symbols: Vec<String> = plans
        .iter()
        .filter_map(|p| p.ai.as_ref())
        .flat_map(|ai| {
            weights_of(ai, "weights_with")
                .into_keys()
                .chain(weights_of(ai, "weights_without").into_keys())
        })
        .collect();
    symbols.sort();
    symbols.dedup();
    let today = MarketCalendar::local_date(state.now());
    let closes: HashMap<String, Vec<(NaiveDate, f64)>> = match plans.first() {
        Some(first) => market::adjusted_panel(
            &client,
            &symbols,
            first.trade_date - Duration::days(7),
            Some(today),
        )
        .await?
        .into_iter()
        .map(|(s, rows)| (s, rows.into_iter().map(|(d, _, c)| (d, c)).collect()))
        .collect(),
        None => HashMap::new(),
    };
    let sector_of: HashMap<String, String> = market::assets(&client, true)
        .await?
        .into_iter()
        .map(|a| (a.symbol, a.sector_id))
        .collect();
    Ok(evaluate_from(&plans, &closes, &sector_of, today))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    fn plan(id: i64, day: u32, with: Value, without: Value) -> store::PlanAi {
        store::PlanAi {
            plan_id: id,
            trade_date: d(day),
            slot: "open".into(),
            generated_at: Utc::now(),
            ai: Some(
                json!({"view_id": 1, "weights_with": with, "weights_without": without, "active_share": 0.1}),
            ),
        }
    }

    fn plan_without_view(id: i64, day: u32) -> store::PlanAi {
        store::PlanAi {
            plan_id: id,
            trade_date: d(day),
            slot: "open".into(),
            generated_at: Utc::now(),
            ai: None,
        }
    }

    #[test]
    fn the_overlay_earns_the_weight_difference_times_the_next_returns() {
        // Plan on the 5th: 10 points more in A, 10 fewer in B. On the 6th A gains 2%, B loses 1%:
        // 0.1 × 2% + 0.1 × 1% = 30 bp. A plan on the 6th removes the difference.
        let plans = vec![
            plan(
                1,
                5,
                json!({"A": 0.3, "B": 0.1}),
                json!({"A": 0.2, "B": 0.2}),
            ),
            plan(2, 6, json!({"A": 0.2}), json!({"A": 0.2})),
        ];
        let closes = HashMap::from([
            (
                "A".to_string(),
                vec![(d(2), 100.0), (d(5), 100.0), (d(6), 102.0), (d(7), 110.0)],
            ),
            (
                "B".to_string(),
                vec![(d(2), 50.0), (d(5), 50.0), (d(6), 49.5), (d(7), 40.0)],
            ),
        ]);
        let sectors = HashMap::from([
            ("A".to_string(), "power".to_string()),
            ("B".to_string(), "storage".to_string()),
        ]);
        let evaluation = evaluate_from(&plans, &closes, &sectors, d(7));
        assert_eq!(evaluation.days, 2);
        assert!((evaluation.series[0].bps - 30.0).abs() < 1e-9);
        assert_eq!(evaluation.series[0].plan_id, 1);
        assert_eq!(evaluation.series[1].bps, 0.0);
        assert!((evaluation.cumulative_bps - 30.0).abs() < 1e-9);
        assert!((evaluation.sectors["power"] - 20.0).abs() < 1e-9);
        assert!((evaluation.sectors["storage"] - 10.0).abs() < 1e-9);
        assert_eq!(evaluation.hit_rate, Some(1.0));
        assert_eq!(evaluation.mean_active_share, Some(0.1));
        // Nothing before the first plan counts.
        let evaluation = evaluate_from(&plans, &closes, &sectors, d(5));
        assert_eq!(evaluation.days, 0);

        // A later plan without a view ends the holding: its days are not counted at all.
        let plans = vec![
            plan(
                1,
                5,
                json!({"A": 0.3, "B": 0.1}),
                json!({"A": 0.2, "B": 0.2}),
            ),
            plan_without_view(2, 6),
        ];
        let evaluation = evaluate_from(&plans, &closes, &sectors, d(7));
        assert_eq!(evaluation.days, 1);
        assert_eq!(evaluation.plans, 1);
        assert!((evaluation.cumulative_bps - 30.0).abs() < 1e-9);
    }

    #[test]
    fn a_view_that_moves_exposure_is_measured_at_equal_exposure() {
        // With a volatility ceiling the view's target invests 50% instead of 40%; rescaled, it is
        // the same 50/50 split as without it, so the extra exposure earns nothing.
        let plans = vec![plan(
            1,
            5,
            json!({"A": 0.25, "B": 0.25}),
            json!({"A": 0.2, "B": 0.2}),
        )];
        let closes = HashMap::from([
            ("A".to_string(), vec![(d(5), 100.0), (d(6), 110.0)]),
            ("B".to_string(), vec![(d(5), 100.0), (d(6), 105.0)]),
        ]);
        let evaluation = evaluate_from(&plans, &closes, &HashMap::new(), d(6));
        assert_eq!(evaluation.days, 1);
        assert!(evaluation.cumulative_bps.abs() < 1e-9);
    }
}
