//! Trading-plan generation.
//!
//! A plan is a complete, immutable record of one decision: the strategy version, the prices and
//! signals it saw, which names were frozen or excluded and why, the target weights, and the
//! orders needed to reach them. The same [`build_proposal`] powers the strategy page's preview,
//! so what an operator previews is exactly what the scheduler would trade.
//!
//! Safety rails applied before the engine runs:
//! - operator restrictions (`exclude` → target zero, `lock` → hold, never trade);
//! - names no longer in the universe are excluded (sold);
//! - a stale quote, a price far from the last close (bad print or unprocessed split), or a held
//!   name with missing recent history freezes that name for this plan — a data problem must
//!   never cause a trade.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use quant_core::rebalance::{RebalanceInput, RebalancePlan, rebalance};
use quant_core::strategy::{AssetMeta, EngineInput, StrategyParams, TargetResult, compute_targets};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Value, json};

use crate::notify::{self, Event};
use crate::services::marketdata;
use crate::state::{AppState, ServerEvent};
use crate::store::settings::{self, AutomationMode, ExecutionSettings};
use crate::store::strategy::{self as strategy_store, StrategyVersion};
use crate::store::trading::{self, Account, NewPlan, dec, f};
use crate::store::{market, system};

#[derive(Debug, Clone, Serialize)]
pub struct QuoteInfo {
    pub price: f64,
    pub quote_ts: Option<DateTime<Utc>>,
    pub age_secs: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Flag {
    pub symbol: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedView {
    pub symbol: String,
    pub reason: quant_core::rebalance::SkipReason,
    pub weight_before: f64,
    pub weight_target: f64,
}

pub struct Proposal {
    pub account: Account,
    pub nav: f64,
    pub cash: f64,
    pub symbols: Vec<String>,
    pub assets: Vec<AssetMeta>,
    pub prices: Vec<f64>,
    pub targets: TargetResult,
    pub plan: RebalancePlan,
    pub frozen: Vec<Flag>,
    pub excluded: Vec<Flag>,
    pub quotes: BTreeMap<String, QuoteInfo>,
    pub history_from: Option<NaiveDate>,
    pub history_to: Option<NaiveDate>,
}

impl Proposal {
    pub fn skipped(&self) -> Vec<SkippedView> {
        self.plan
            .skipped
            .iter()
            .map(|s| SkippedView {
                symbol: self.symbols[s.asset].clone(),
                reason: s.reason,
                weight_before: s.weight_before,
                weight_target: s.weight_target,
            })
            .collect()
    }

    pub fn orders_json(&self) -> Vec<Value> {
        self.plan
            .orders
            .iter()
            .map(|o| {
                json!({
                    "symbol": self.symbols[o.asset],
                    "side": o.side,
                    "reason": o.reason,
                    "qty": o.qty,
                    "price": o.price,
                    "notional": o.notional,
                    "weight_before": o.weight_before,
                    "weight_target": o.weight_target,
                    "weight_after": o.weight_after,
                })
            })
            .collect()
    }

    pub fn diagnostics(&self, params: &StrategyParams, version: Option<&StrategyVersion>) -> Value {
        json!({
            "targets": self.targets,
            "skipped": self.skipped(),
            "frozen": self.frozen,
            "excluded": self.excluded,
            "quotes": self.quotes,
            "params": params,
            "strategy": version.map(|v| json!({"id": v.id, "name": v.name, "preset_id": v.preset_id})),
            "rebalance": {
                "nav": self.plan.nav,
                "turnover": self.plan.turnover,
                "buy_value": self.plan.buy_value,
                "sell_value": self.plan.sell_value,
                "est_commission": self.plan.est_commission,
                "est_slippage": self.plan.est_slippage,
                "est_fees": self.plan.est_fees,
                "cash_after": self.plan.cash_after,
                "turnover_scale": self.plan.turnover_scale,
                "cash_scale": self.plan.cash_scale,
            },
            "history": {"from": self.history_from, "to": self.history_to},
        })
    }
}

/// Builds targets and orders for `trade_date` using live quotes and history strictly before it.
pub async fn build_proposal(
    state: &AppState,
    params: &StrategyParams,
    trade_date: NaiveDate,
) -> Result<Proposal> {
    params
        .validate()
        .map_err(|errors| anyhow!("invalid strategy parameters: {}", errors[0].message))?;
    let client = state.pool.get().await?;
    let account = trading::require_active_account(&client).await?;
    let execution: ExecutionSettings = settings::get(&client, settings::EXECUTION).await?;
    let universe = market::assets(&client, true).await?;
    let positions = trading::positions(&client, account.id, false).await?;
    let restrictions = strategy_store::active_restrictions(&client, trade_date).await?;
    drop(client);

    let held: HashMap<String, Decimal> = positions
        .iter()
        .map(|p| (p.symbol.clone(), p.qty))
        .collect();
    let mut assets: Vec<AssetMeta> = Vec::new();
    let mut in_universe: Vec<bool> = Vec::new();
    for asset in &universe {
        if asset.is_active || held.contains_key(&asset.symbol) {
            assets.push(AssetMeta {
                symbol: asset.symbol.clone(),
                sector: asset.sector_id.clone(),
            });
            in_universe.push(asset.is_active);
        }
    }
    // Holdings that are not in the asset table at all (should not happen, but never strand them).
    for symbol in held.keys() {
        if !assets.iter().any(|a| &a.symbol == symbol) {
            assets.push(AssetMeta {
                symbol: symbol.clone(),
                sector: "unclassified".into(),
            });
            in_universe.push(false);
        }
    }
    let symbols: Vec<String> = assets.iter().map(|a| a.symbol.clone()).collect();
    let n = symbols.len();

    // Quotes.
    let now = state.now();
    let session_open = state
        .calendar
        .session(trade_date)
        .is_some_and(|s| s.contains(now));
    let live = marketdata::fresh_quotes(state, &symbols).await?;
    let mut quotes = BTreeMap::new();
    let mut prices = vec![f64::NAN; n];
    for (i, symbol) in symbols.iter().enumerate() {
        if let Some((price, quote_ts, fetched_at)) = live.get(symbol) {
            let reference = quote_ts.unwrap_or(*fetched_at);
            quotes.insert(
                symbol.clone(),
                QuoteInfo {
                    price: *price,
                    quote_ts: *quote_ts,
                    age_secs: (now - reference).num_seconds().max(0),
                },
            );
            if *price > 0.0 {
                prices[i] = *price;
            }
        }
    }

    // History strictly before the trade date.
    let lookback_days = (params.max_lookback() as f64 * 1.6) as i64 + 30;
    let from = trade_date - Duration::days(lookback_days);
    let client = state.pool.get().await?;
    let panel = market::adjusted_panel(
        &client,
        &symbols,
        from,
        Some(trade_date - Duration::days(1)),
    )
    .await?;
    let all_dates: BTreeSet<NaiveDate> = panel.values().flatten().map(|(d, _, _)| *d).collect();
    let dates: Vec<NaiveDate> = all_dates.into_iter().collect();
    let date_index: HashMap<NaiveDate, usize> =
        dates.iter().enumerate().map(|(i, d)| (*d, i)).collect();
    let mut history = vec![vec![f64::NAN; dates.len()]; n];
    for (i, symbol) in symbols.iter().enumerate() {
        if let Some(series) = panel.get(symbol) {
            for (d, _, close) in series {
                history[i][date_index[d]] = *close;
            }
        }
    }

    // Guards.
    let mut excluded_flags = vec![false; n];
    let mut frozen_flags = vec![false; n];
    let mut frozen = Vec::new();
    let mut excluded = Vec::new();
    for (i, symbol) in symbols.iter().enumerate() {
        if let Some(r) = restrictions.iter().find(|r| &r.symbol == symbol) {
            if r.mode == "lock" {
                frozen_flags[i] = true;
                frozen.push(Flag {
                    symbol: symbol.clone(),
                    reason: "locked".into(),
                });
            } else {
                excluded_flags[i] = true;
                excluded.push(Flag {
                    symbol: symbol.clone(),
                    reason: "excluded".into(),
                });
            }
            continue;
        }
        if !in_universe[i] {
            excluded_flags[i] = true;
            excluded.push(Flag {
                symbol: symbol.clone(),
                reason: "not_in_universe".into(),
            });
            continue;
        }
        let is_held = held.get(symbol).is_some_and(|q| *q > Decimal::ZERO);
        if let Some(q) = quotes.get(symbol)
            && session_open
            && q.age_secs > execution.max_quote_age_secs as i64
        {
            frozen_flags[i] = true;
            frozen.push(Flag {
                symbol: symbol.clone(),
                reason: "stale_quote".into(),
            });
            continue;
        }
        let last_close = history[i].iter().rev().find(|p| p.is_finite()).copied();
        if let (Some(close), true) = (last_close, prices[i].is_finite())
            && close > 0.0
            && (prices[i] / close - 1.0).abs() > execution.max_daily_move
        {
            frozen_flags[i] = true;
            frozen.push(Flag {
                symbol: symbol.clone(),
                reason: "price_anomaly".into(),
            });
            continue;
        }
        let recent_ok = history[i].iter().rev().take(5).any(|p| p.is_finite());
        if is_held && !recent_ok {
            frozen_flags[i] = true;
            frozen.push(Flag {
                symbol: symbol.clone(),
                reason: "missing_history".into(),
            });
        }
    }

    // Current weights.
    let cash = f(account.cash);
    let qty: Vec<f64> = symbols
        .iter()
        .map(|s| held.get(s).map(|q| f(*q)).unwrap_or(0.0))
        .collect();
    let valuation_price: Vec<f64> = (0..n)
        .map(|i| {
            if prices[i].is_finite() {
                prices[i]
            } else {
                history[i]
                    .iter()
                    .rev()
                    .find(|p| p.is_finite())
                    .copied()
                    .unwrap_or(f64::NAN)
            }
        })
        .collect();
    let invested: f64 = (0..n)
        .filter(|&i| valuation_price[i].is_finite())
        .map(|i| qty[i] * valuation_price[i])
        .sum();
    let nav = cash + invested;
    let current_weights: Vec<f64> = (0..n)
        .map(|i| {
            if nav > 0.0 && valuation_price[i].is_finite() {
                qty[i] * valuation_price[i] / nav
            } else {
                0.0
            }
        })
        .collect();

    let history_refs: Vec<&[f64]> = history.iter().map(|h| h.as_slice()).collect();
    let targets = compute_targets(
        params,
        &EngineInput {
            assets: &assets,
            history: &history_refs,
            current: &prices,
            current_weights: &current_weights,
            excluded: &excluded_flags,
            frozen: &frozen_flags,
        },
    );
    let tradable: Vec<bool> = (0..n)
        .map(|i| !frozen_flags[i] && prices[i].is_finite())
        .collect();
    let plan = rebalance(
        &RebalanceInput {
            cash,
            qty: &qty,
            prices: &valuation_price,
            targets: &targets.weights,
            tradable: &tradable,
        },
        &params.rebalance,
        &execution.costs,
    );
    Ok(Proposal {
        account,
        nav,
        cash,
        symbols,
        assets,
        prices,
        targets,
        plan,
        frozen,
        excluded,
        quotes,
        history_from: dates.first().copied(),
        history_to: dates.last().copied(),
    })
}

pub struct PlanRequest {
    /// `open`, `close` or `manual`.
    pub slot: String,
    pub trade_date: NaiveDate,
    pub deadline: DateTime<Utc>,
    pub actor: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeneratedPlan {
    pub plan_id: i64,
    pub status: String,
    pub orders: usize,
}

/// Generates and persists a plan. For scheduled slots this is idempotent: if a plan for the
/// slot already exists it is returned unchanged.
pub async fn generate(state: &Arc<AppState>, request: PlanRequest) -> Result<GeneratedPlan> {
    let _guard = state.trading_lock.lock().await;
    let client = state.pool.get().await?;
    let account = trading::require_active_account(&client).await?;
    if request.slot != "manual"
        && let Some(existing) =
            trading::plan_for_slot(&client, account.id, request.trade_date, &request.slot).await?
    {
        return Ok(GeneratedPlan {
            plan_id: existing.id,
            status: existing.status,
            orders: existing.order_count as usize,
        });
    }
    let version = strategy_store::active_version(&client, account.id)
        .await?
        .ok_or_else(|| anyhow!("no active strategy"))?;
    let params = version.strategy_params()?;
    let automation: settings::AutomationSettings =
        settings::get(&client, settings::AUTOMATION).await?;
    let schedule = settings::schedule(&client).await?;
    drop(client);

    let mode = automation.effective_mode(state.now());
    let proposal = build_proposal(state, &params, request.trade_date).await?;
    let now = state.now();
    let has_orders = !proposal.plan.orders.is_empty();
    let status = if has_orders { "pending" } else { "no_action" };
    let execute_after = (has_orders && mode == AutomationMode::Auto)
        .then(|| (now + Duration::minutes(schedule.review_minutes as i64)).min(request.deadline));
    let buys = proposal
        .plan
        .orders
        .iter()
        .filter(|o| o.side == quant_core::costs::Side::Buy)
        .count();
    let sells = proposal.plan.orders.len() - buys;
    let est_costs =
        proposal.plan.est_commission + proposal.plan.est_fees + proposal.plan.est_slippage;
    let summary = json!({
        "buys": buys,
        "sells": sells,
        "buy_value": proposal.plan.buy_value,
        "sell_value": proposal.plan.sell_value,
        "frozen": proposal.frozen.len(),
        "excluded": proposal.excluded.len(),
        "strategy_name": version.name,
        "cash_after": proposal.plan.cash_after,
    });

    let mut client = state.pool.get().await?;
    let tx = client.transaction().await?;
    let plan_id = trading::insert_plan(
        &tx,
        &NewPlan {
            account_id: proposal.account.id,
            trade_date: request.trade_date,
            slot: &request.slot,
            status,
            strategy_version_id: Some(version.id),
            automation_mode: mode.as_str(),
            execute_after,
            deadline: request.deadline,
            nav: dec(proposal.nav, 2),
            cash: dec(proposal.cash, 2),
            exposure_target: Some(proposal.targets.exposure_target),
            invested_target: Some(proposal.targets.invested),
            breadth: proposal.targets.breadth,
            est_vol: proposal.targets.est_vol,
            turnover: proposal.plan.turnover,
            est_costs: dec(est_costs, 2),
            order_count: proposal.plan.orders.len() as i32,
            diagnostics: proposal.diagnostics(&params, Some(&version)),
            summary,
            error: None,
            created_by: &request.actor,
        },
    )
    .await?
    .ok_or_else(|| anyhow!("a plan for this slot already exists"))?;
    for (seq, order) in proposal.plan.orders.iter().enumerate() {
        let qty_dp = if params.rebalance.fractional_shares {
            4
        } else {
            0
        };
        trading::insert_order(
            &tx,
            plan_id,
            proposal.account.id,
            &proposal.symbols[order.asset],
            order.side.as_str(),
            order.reason.as_str(),
            dec(order.qty, qty_dp),
            order.price,
            (order.weight_before, order.weight_target, order.weight_after),
            seq as i32,
        )
        .await?;
    }
    system::audit(
        &tx,
        &request.actor,
        "plan.generated",
        "plan",
        &plan_id.to_string(),
        json!({"slot": request.slot, "trade_date": request.trade_date, "orders": proposal.plan.orders.len(), "strategy_version_id": version.id, "mode": mode.as_str()}),
        "",
    )
    .await?;
    tx.commit().await?;
    drop(client);
    tracing::info!(
        plan = plan_id,
        trade_date = %request.trade_date,
        slot = %request.slot,
        status,
        orders = proposal.plan.orders.len(),
        turnover = format!("{:.3}", proposal.plan.turnover),
        execute_after = ?execute_after,
        "plan generated"
    );

    state.emit(ServerEvent::Plan {
        plan_id,
        status: status.into(),
    });
    let event = if has_orders {
        Event::PlanGenerated {
            plan_id,
            trade_date: request.trade_date,
            slot: request.slot.clone(),
            orders: proposal.plan.orders.len(),
            buys,
            sells,
            turnover: proposal.plan.turnover,
            execute_after,
            deadline: request.deadline,
            strategy: version.name.clone(),
            preset_id: version.preset_id.clone(),
        }
    } else {
        Event::PlanNoAction {
            plan_id,
            trade_date: request.trade_date,
            slot: request.slot.clone(),
        }
    };
    if let Err(error) = notify::notify(state, event).await {
        tracing::warn!(%error, "plan notification failed");
    }
    Ok(GeneratedPlan {
        plan_id,
        status: status.into(),
        orders: proposal.plan.orders.len(),
    })
}

/// Records that a scheduled slot produced no plan (paused, cancelled by an operator, or missed
/// while the service was down) so the history has no silent gaps.
pub async fn record_skipped_slot(
    state: &Arc<AppState>,
    trade_date: NaiveDate,
    slot: &str,
    reason: &str,
    deadline: DateTime<Utc>,
) -> Result<Option<i64>> {
    let client = state.pool.get().await?;
    let account = trading::require_active_account(&client).await?;
    let automation: settings::AutomationSettings =
        settings::get(&client, settings::AUTOMATION).await?;
    let version = strategy_store::active_version(&client, account.id).await?;
    let plan_id = trading::insert_plan(
        &client,
        &NewPlan {
            account_id: account.id,
            trade_date,
            slot,
            status: "skipped",
            strategy_version_id: version.map(|v| v.id),
            automation_mode: automation.effective_mode(state.now()).as_str(),
            execute_after: None,
            deadline,
            nav: Decimal::ZERO,
            cash: account.cash,
            exposure_target: None,
            invested_target: None,
            breadth: None,
            est_vol: None,
            turnover: 0.0,
            est_costs: Decimal::ZERO,
            order_count: 0,
            diagnostics: json!({}),
            summary: json!({"skip_reason": reason}),
            error: None,
            created_by: "system",
        },
    )
    .await?;
    drop(client);
    if let Some(id) = plan_id {
        state.emit(ServerEvent::Plan {
            plan_id: id,
            status: "skipped".into(),
        });
        let _ = notify::notify(
            state,
            Event::SlotSkipped {
                trade_date,
                slot: slot.to_string(),
                reason: reason.to_string(),
            },
        )
        .await;
    }
    Ok(plan_id)
}
