//! The paper broker: executes plans against live quotes and keeps the account's books.
//!
//! hone-quant has no live-broker integration at all — this module *is* the broker. Fills use
//! the same cost model as the backtester (commission, regulatory fee, slippage). Every fill,
//! position change and cash movement of a plan is written in a single database transaction, so
//! a failure leaves the books exactly as they were.
//!
//! Per-order checks at execution time: the quote must be fresh, must not have moved more than
//! the configured deviation from the price the plan was sized with, sells cannot exceed the
//! position, and buys cannot exceed available cash (they are reduced rather than borrowed for).

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use quant_core::costs::Side;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use serde::Serialize;
use serde_json::json;

use crate::notify::{self, Event};
use crate::services::marketdata;
use crate::state::{AppState, ServerEvent};
use crate::store::settings::{self, ExecutionSettings};
use crate::store::strategy as strategy_store;
use crate::store::system;
use crate::store::trading::{self, NewFill, Position, dec, f};

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionReport {
    pub plan_id: i64,
    pub status: String,
    pub filled: usize,
    pub partial: usize,
    pub rejected: usize,
    pub bought: f64,
    pub sold: f64,
    pub costs: f64,
}

fn round_qty(qty: f64, fractional: bool) -> f64 {
    if fractional {
        (qty * 10_000.0).floor() / 10_000.0
    } else {
        qty.floor()
    }
}

/// Executes a pending plan now. The session must be open and the plan before its deadline.
pub async fn execute_plan(
    state: &Arc<AppState>,
    plan_id: i64,
    actor: &str,
) -> Result<ExecutionReport> {
    let _guard = state.trading_lock.lock().await;
    let now = state.now();
    let mut client = state.pool.get().await?;

    // Claim the plan.
    let plan = {
        let tx = client.transaction().await?;
        let plan = trading::plan_for_update(&tx, plan_id)
            .await?
            .ok_or_else(|| anyhow!("plan {plan_id} not found"))?;
        if plan.status != "pending" {
            bail!("plan {plan_id} is {}, not pending", plan.status);
        }
        if now >= plan.deadline {
            tx.execute(
                "UPDATE plans SET status = 'expired' WHERE id = $1",
                &[&plan_id],
            )
            .await?;
            tx.execute(
                "UPDATE orders SET status = 'expired', updated_at = app_now() WHERE plan_id = $1 AND status = 'planned'",
                &[&plan_id],
            )
            .await?;
            tx.commit().await?;
            bail!("plan {plan_id} passed its deadline and expired");
        }
        let session_open = state
            .calendar
            .session(plan.trade_date)
            .is_some_and(|s| s.contains(now));
        if !session_open {
            bail!("the market is closed; plans execute only during the regular session");
        }
        trading::set_plan_status(&tx, plan_id, "executing", None).await?;
        tx.commit().await?;
        plan
    };
    state.emit(ServerEvent::Plan {
        plan_id,
        status: "executing".into(),
    });

    let outcome = execute_claimed(state, &mut client, &plan, actor).await;
    match outcome {
        Ok(report) => {
            drop(client);
            tracing::info!(
                plan = plan_id,
                status = %report.status,
                filled = report.filled,
                partial = report.partial,
                rejected = report.rejected,
                bought = format!("{:.2}", report.bought),
                sold = format!("{:.2}", report.sold),
                costs = format!("{:.2}", report.costs),
                actor,
                "plan executed"
            );
            state.emit(ServerEvent::Plan {
                plan_id,
                status: report.status.clone(),
            });
            state.emit(ServerEvent::Account {
                reason: "execution".into(),
            });
            let _ = notify::notify(
                state,
                Event::PlanExecuted {
                    plan_id,
                    trade_date: plan.trade_date,
                    slot: plan.slot.clone(),
                    filled: report.filled + report.partial,
                    rejected: report.rejected,
                    bought: report.bought,
                    sold: report.sold,
                    costs: report.costs,
                },
            )
            .await;
            Ok(report)
        }
        Err(error) => {
            let message = format!("{error:#}");
            tracing::error!(plan = plan_id, error = %message, "plan execution failed");
            trading::set_plan_status(&client, plan_id, "failed", Some(&message)).await?;
            system::audit(
                &client,
                actor,
                "plan.failed",
                "plan",
                &plan_id.to_string(),
                json!({"error": message}),
                "",
            )
            .await?;
            drop(client);
            state.emit(ServerEvent::Plan {
                plan_id,
                status: "failed".into(),
            });
            let _ = notify::notify(
                state,
                Event::PlanFailed {
                    plan_id,
                    slot: plan.slot.clone(),
                    error: message.clone(),
                },
            )
            .await;
            Err(error)
        }
    }
}

async fn execute_claimed(
    state: &AppState,
    client: &mut deadpool_postgres::Client,
    plan: &trading::Plan,
    actor: &str,
) -> Result<ExecutionReport> {
    let execution: ExecutionSettings = settings::get(&*client, settings::EXECUTION).await?;
    let fractional = match plan.strategy_version_id {
        Some(id) => strategy_store::version(&*client, id)
            .await?
            .and_then(|v| v.strategy_params().ok())
            .is_some_and(|p| p.rebalance.fractional_shares),
        None => false,
    };
    let orders: Vec<_> = trading::orders_for_plan(&*client, plan.id)
        .await?
        .into_iter()
        .filter(|o| o.status == "planned")
        .collect();
    let symbols: Vec<String> = orders.iter().map(|o| o.symbol.clone()).collect();
    let quotes = marketdata::fresh_quotes(state, &symbols).await?;
    let now = state.now();
    let costs = &execution.costs;

    let tx = client.transaction().await?;
    let account = trading::lock_account(&tx, plan.account_id).await?;
    let mut cash = account.cash;
    let mut report = ExecutionReport {
        plan_id: plan.id,
        status: String::new(),
        filled: 0,
        partial: 0,
        rejected: 0,
        bought: 0.0,
        sold: 0.0,
        costs: 0.0,
    };

    // Orders were stored sells-first; keep that order so sales fund purchases.
    for order in &orders {
        let side = Side::parse(&order.side).ok_or_else(|| anyhow!("bad side {}", order.side))?;
        let reject = |reason: &'static str| reason;
        let quote = quotes.get(&order.symbol);
        let verdict: Result<(f64, Option<chrono::DateTime<chrono::Utc>>), &str> = match quote {
            None => Err(reject("no_quote")),
            Some((price, quote_ts, fetched_at)) => {
                let age = (now - quote_ts.unwrap_or(*fetched_at)).num_seconds();
                if *price <= 0.0 {
                    Err(reject("no_quote"))
                } else if age > execution.max_quote_age_secs as i64 {
                    Err(reject("stale_quote"))
                } else if order.ref_price > 0.0
                    && (price / order.ref_price - 1.0).abs() > execution.max_price_deviation
                {
                    Err(reject("price_moved"))
                } else {
                    Ok((*price, *quote_ts))
                }
            }
        };
        let (quote_price, quote_ts) = match verdict {
            Ok(v) => v,
            Err(reason) => {
                trading::set_order_status(&tx, order.id, "rejected", Some(reason), None).await?;
                report.rejected += 1;
                continue;
            }
        };

        let mut position = trading::position_for_update(&tx, plan.account_id, &order.symbol)
            .await?
            .unwrap_or(Position {
                symbol: order.symbol.clone(),
                qty: Decimal::ZERO,
                avg_cost: Decimal::ZERO,
                realized_pnl: Decimal::ZERO,
                dividends: Decimal::ZERO,
                opened_at: None,
                updated_at: now,
            });
        let exec_price = costs.execution_price(side, quote_price);
        let wanted = f(order.qty);
        let mut qty = wanted;
        match side {
            Side::Sell => qty = qty.min(f(position.qty)),
            Side::Buy => {
                let available = f(cash);
                let cost_of = |q: f64| q * exec_price + costs.commission(q, exec_price);
                if cost_of(qty) > available {
                    qty = round_qty(available / (exec_price * (1.0 + 1e-9)), fractional);
                    while qty > 0.0 && cost_of(qty) > available {
                        qty -= if fractional { 0.0001 } else { 1.0 };
                    }
                    qty = qty.max(0.0);
                }
            }
        }
        if qty <= 0.0 {
            let reason = if side == Side::Sell {
                "no_position"
            } else {
                "insufficient_cash"
            };
            trading::set_order_status(&tx, order.id, "rejected", Some(reason), None).await?;
            report.rejected += 1;
            continue;
        }

        let qty_d = Decimal::from_f64(qty).unwrap_or_default().round_dp(6);
        let price_d = dec(exec_price, 6);
        let notional = (qty_d * price_d).round_dp(2);
        let commission = dec(costs.commission(qty, exec_price), 2);
        let fees = dec(costs.fees(side, f(notional)), 2);
        let slippage = dec((exec_price - quote_price).abs() * qty, 2);
        let mut realized = None;
        let cash_delta = match side {
            Side::Buy => {
                let total_cost = position.qty * position.avg_cost + notional + commission + fees;
                position.qty += qty_d;
                position.avg_cost = if position.qty > Decimal::ZERO {
                    (total_cost / position.qty).round_dp(6)
                } else {
                    Decimal::ZERO
                };
                if position.opened_at.is_none() {
                    position.opened_at = Some(now);
                }
                -(notional + commission + fees)
            }
            Side::Sell => {
                let proceeds = notional - commission - fees;
                let pnl = (proceeds - qty_d * position.avg_cost).round_dp(2);
                position.realized_pnl += pnl;
                position.qty -= qty_d;
                if position.qty <= Decimal::ZERO {
                    position.qty = Decimal::ZERO;
                    position.opened_at = None;
                }
                realized = Some(pnl);
                proceeds
            }
        };
        if cash + cash_delta < Decimal::ZERO {
            // Rounding can leave a buy a few cents short; never let cash go negative.
            trading::set_order_status(&tx, order.id, "rejected", Some("insufficient_cash"), None)
                .await?;
            report.rejected += 1;
            continue;
        }
        trading::save_position(&tx, plan.account_id, &position).await?;
        let fill_id = trading::insert_fill(
            &tx,
            &NewFill {
                order_id: order.id,
                account_id: plan.account_id,
                symbol: order.symbol.clone(),
                side: order.side.clone(),
                qty: qty_d,
                price: price_d,
                quote_price,
                quote_ts,
                notional,
                commission,
                fees,
                slippage,
                realized_pnl: realized,
            },
        )
        .await?;
        cash = trading::post_cash(
            &tx,
            plan.account_id,
            "trade",
            cash_delta,
            Some(&order.symbol),
            Some("fill"),
            Some(fill_id.to_string()),
            &format!("{} {} @ {}", order.side, qty_d, price_d),
        )
        .await?;
        let complete = (qty - wanted).abs() < 1e-9;
        trading::set_order_status(
            &tx,
            order.id,
            if complete {
                "filled"
            } else {
                "partially_filled"
            },
            (!complete).then_some(if side == Side::Sell {
                "reduced_to_position"
            } else {
                "reduced_for_cash"
            }),
            Some(qty_d),
        )
        .await?;
        if complete {
            report.filled += 1;
        } else {
            report.partial += 1;
        }
        match side {
            Side::Buy => report.bought += f(notional),
            Side::Sell => report.sold += f(notional),
        }
        report.costs += f(commission + fees + slippage);
    }

    report.status = if report.filled + report.partial == 0 {
        "failed".into()
    } else if report.rejected > 0 || report.partial > 0 {
        "partially_executed".into()
    } else {
        "executed".into()
    };
    let error = (report.status == "failed").then_some("no order could be filled");
    trading::set_plan_status(&tx, plan.id, &report.status, error).await?;
    trading::update_plan_summary(
        &tx,
        plan.id,
        &json!({"executed": {"filled": report.filled, "partial": report.partial, "rejected": report.rejected,
                              "bought": report.bought, "sold": report.sold, "costs": report.costs}}),
    )
    .await?;
    system::audit(
        &tx,
        actor,
        "plan.executed",
        "plan",
        &plan.id.to_string(),
        serde_json::to_value(&report)?,
        "",
    )
    .await?;
    tx.commit().await?;
    Ok(report)
}

/// Cancels a pending plan (operator intervention).
pub async fn cancel_plan(
    state: &Arc<AppState>,
    plan_id: i64,
    actor: &str,
    reason: &str,
    ip: &str,
) -> Result<()> {
    let _guard = state.trading_lock.lock().await;
    let mut client = state.pool.get().await?;
    let tx = client.transaction().await?;
    let plan = trading::plan_for_update(&tx, plan_id)
        .await?
        .ok_or_else(|| anyhow!("plan {plan_id} not found"))?;
    if plan.status != "pending" {
        bail!(
            "only pending plans can be cancelled (this one is {})",
            plan.status
        );
    }
    trading::cancel_plan(&tx, plan_id, actor, reason).await?;
    system::audit(
        &tx,
        actor,
        "plan.cancelled",
        "plan",
        &plan_id.to_string(),
        json!({"reason": reason}),
        ip,
    )
    .await?;
    tx.commit().await?;
    drop(client);
    tracing::info!(plan = plan_id, actor, reason, "plan cancelled");
    state.emit(ServerEvent::Plan {
        plan_id,
        status: "cancelled".into(),
    });
    let _ = notify::notify(
        state,
        Event::PlanCancelled {
            plan_id,
            slot: plan.slot,
            actor: actor.to_string(),
            reason: reason.to_string(),
        },
    )
    .await;
    Ok(())
}

/// Removes one order from a pending plan before it executes.
pub async fn skip_order(
    state: &Arc<AppState>,
    plan_id: i64,
    order_id: i64,
    actor: &str,
    ip: &str,
) -> Result<()> {
    let _guard = state.trading_lock.lock().await;
    let mut client = state.pool.get().await?;
    let tx = client.transaction().await?;
    let plan = trading::plan_for_update(&tx, plan_id)
        .await?
        .ok_or_else(|| anyhow!("plan {plan_id} not found"))?;
    if plan.status != "pending" {
        bail!("orders can only be removed from pending plans");
    }
    let order = trading::order(&tx, order_id)
        .await?
        .filter(|o| o.plan_id == plan_id)
        .ok_or_else(|| anyhow!("order {order_id} not found in plan {plan_id}"))?;
    if order.status != "planned" {
        bail!("order {order_id} is {}", order.status);
    }
    trading::set_order_status(&tx, order_id, "skipped", Some("operator"), None).await?;
    let remaining: i64 = tx
        .query_one(
            "SELECT count(*) FROM orders WHERE plan_id = $1 AND status = 'planned'",
            &[&plan_id],
        )
        .await?
        .get(0);
    if remaining == 0 {
        trading::cancel_plan(&tx, plan_id, actor, "all orders removed").await?;
    }
    system::audit(
        &tx,
        actor,
        "order.skipped",
        "order",
        &order_id.to_string(),
        json!({"plan_id": plan_id, "symbol": order.symbol, "side": order.side, "qty": order.qty}),
        ip,
    )
    .await?;
    tx.commit().await?;
    state.emit(ServerEvent::Plan {
        plan_id,
        status: if remaining == 0 {
            "cancelled".into()
        } else {
            "pending".into()
        },
    });
    Ok(())
}

/// Marks pending plans past their deadline as expired.
pub async fn expire_overdue(state: &Arc<AppState>) -> Result<usize> {
    let _guard = state.trading_lock.lock().await;
    let client = state.pool.get().await?;
    let now = state.now();
    let rows = client
        .query(
            "UPDATE plans SET status = 'expired' WHERE status = 'pending' AND deadline <= $1 RETURNING id, slot",
            &[&now],
        )
        .await?;
    for row in &rows {
        let id: i64 = row.get(0);
        client
            .execute(
                "UPDATE orders SET status = 'expired', updated_at = app_now() WHERE plan_id = $1 AND status = 'planned'",
                &[&id],
            )
            .await?;
    }
    drop(client);
    for row in &rows {
        let plan_id: i64 = row.get(0);
        tracing::warn!(plan = plan_id, "plan expired before execution");
        state.emit(ServerEvent::Plan {
            plan_id,
            status: "expired".into(),
        });
        let _ = notify::notify(
            state,
            Event::PlanExpired {
                plan_id,
                slot: row.get(1),
            },
        )
        .await;
    }
    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::round_qty;

    #[test]
    fn quantities_round_down() {
        assert_eq!(round_qty(10.9, false), 10.0);
        assert_eq!(round_qty(1.23456, true), 1.2345);
    }
}
