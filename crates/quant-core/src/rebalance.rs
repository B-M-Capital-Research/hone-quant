//! From target weights to orders.
//!
//! Rules, in order:
//! 1. Names that cannot be traded (locked, no price) are never touched.
//! 2. A full exit (target zero while holding) always trades and is never scaled.
//! 3. Otherwise a name trades only when its drift exceeds the larger of the absolute and
//!    relative bands; when it trades it goes all the way to target. Bands are waived in the
//!    direction that closes the gap when the portfolio as a whole is more than
//!    [`CASH_BAND`] away from its target invested weight (e.g. a portfolio still being built
//!    after a turnover-capped first plan), so small shortfalls cannot stay open indefinitely.
//! 4. If one-way turnover would exceed the cap, every non-exit trade is scaled down uniformly.
//! 5. Quantities are rounded to whole shares (or 1e-4 for fractional), buys rounded down.
//! 6. Trades below the minimum notional are dropped (exits excepted).
//! 7. Buys are scaled down if sale proceeds plus cash cannot fund them after costs; the account
//!    never borrows.

use serde::{Deserialize, Serialize};

use crate::costs::{CostModel, Side};
use crate::strategy::RebalanceParams;

const FRACTIONAL_STEP: f64 = 1e-4;

/// When invested weight differs from the target invested weight by more than this, name-level
/// bands are waived for trades that close the gap.
pub const CASH_BAND: f64 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderReason {
    Entry,
    Exit,
    Increase,
    Decrease,
}

impl OrderReason {
    pub fn as_str(self) -> &'static str {
        match self {
            OrderReason::Entry => "entry",
            OrderReason::Exit => "exit",
            OrderReason::Increase => "increase",
            OrderReason::Decrease => "decrease",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    WithinBand,
    BelowMinTrade,
    NotTradable,
    RoundedToZero,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderIntent {
    pub asset: usize,
    pub side: Side,
    pub reason: OrderReason,
    pub qty: f64,
    /// Reference price used for sizing.
    pub price: f64,
    pub notional: f64,
    pub weight_before: f64,
    pub weight_target: f64,
    pub weight_after: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedTrade {
    pub asset: usize,
    pub reason: SkipReason,
    pub weight_before: f64,
    pub weight_target: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RebalancePlan {
    pub nav: f64,
    pub orders: Vec<OrderIntent>,
    pub skipped: Vec<SkippedTrade>,
    /// One-way turnover of the final orders (Σ|notional| / 2 / NAV).
    pub turnover: f64,
    pub buy_value: f64,
    pub sell_value: f64,
    pub est_commission: f64,
    pub est_slippage: f64,
    pub est_fees: f64,
    pub cash_after: f64,
    pub turnover_scale: Option<f64>,
    pub cash_scale: Option<f64>,
}

pub struct RebalanceInput<'a> {
    pub cash: f64,
    pub qty: &'a [f64],
    pub prices: &'a [f64],
    pub targets: &'a [f64],
    pub tradable: &'a [bool],
}

fn round_qty(qty: f64, fractional: bool, round_down: bool) -> f64 {
    if fractional {
        let steps = qty / FRACTIONAL_STEP;
        let steps = if round_down {
            steps.floor()
        } else {
            steps.round()
        };
        steps * FRACTIONAL_STEP
    } else if round_down {
        qty.floor()
    } else {
        qty.round()
    }
}

struct Candidate {
    asset: usize,
    delta_weight: f64,
    exit: bool,
}

pub fn rebalance(
    input: &RebalanceInput,
    params: &RebalanceParams,
    costs: &CostModel,
) -> RebalancePlan {
    let n = input.qty.len();
    assert!(
        input.prices.len() == n && input.targets.len() == n && input.tradable.len() == n,
        "rebalance inputs must align"
    );
    let price_ok = |i: usize| input.prices[i].is_finite() && input.prices[i] > 0.0;
    let positions_value: f64 = (0..n)
        .filter(|&i| price_ok(i))
        .map(|i| input.qty[i] * input.prices[i])
        .sum();
    let nav = input.cash + positions_value;
    let weight = |i: usize| {
        if nav > 0.0 && price_ok(i) {
            input.qty[i] * input.prices[i] / nav
        } else {
            0.0
        }
    };

    // Portfolio-level gap: far from the target invested weight, every name that is under (or
    // over) target trades, whatever its own band.
    let invested_now: f64 = (0..n).map(weight).sum();
    let invested_target: f64 = input.targets.iter().map(|t| t.max(0.0)).sum();
    let gap = invested_target - invested_now;
    let waive_for_buys = gap > CASH_BAND;
    let waive_for_sells = gap < -CASH_BAND;

    let mut skipped = Vec::new();
    let mut candidates = Vec::new();
    for i in 0..n {
        let current = weight(i);
        let target = input.targets[i].max(0.0);
        let drift = target - current;
        let holding = input.qty[i] > 0.0;
        if !holding && target <= 0.0 {
            continue;
        }
        if !input.tradable[i] || !price_ok(i) {
            if drift.abs() > 1e-9 {
                skipped.push(SkippedTrade {
                    asset: i,
                    reason: SkipReason::NotTradable,
                    weight_before: current,
                    weight_target: target,
                });
            }
            continue;
        }
        if holding && target <= 1e-12 {
            candidates.push(Candidate {
                asset: i,
                delta_weight: -current,
                exit: true,
            });
            continue;
        }
        let band = params.band_abs.max(params.band_rel * target);
        let waived = (drift > 0.0 && waive_for_buys) || (drift < 0.0 && waive_for_sells);
        if holding && !waived && drift.abs() <= band {
            skipped.push(SkippedTrade {
                asset: i,
                reason: SkipReason::WithinBand,
                weight_before: current,
                weight_target: target,
            });
            continue;
        }
        candidates.push(Candidate {
            asset: i,
            delta_weight: drift,
            exit: false,
        });
    }

    // Turnover ceiling: exits are mandatory, the rest share what is left.
    let exit_turnover: f64 = candidates
        .iter()
        .filter(|c| c.exit)
        .map(|c| c.delta_weight.abs())
        .sum();
    let other_turnover: f64 = candidates
        .iter()
        .filter(|c| !c.exit)
        .map(|c| c.delta_weight.abs())
        .sum();
    let mut turnover_scale = None;
    if (exit_turnover + other_turnover) / 2.0 > params.max_turnover && other_turnover > 0.0 {
        let room = (params.max_turnover * 2.0 - exit_turnover).max(0.0);
        let scale = (room / other_turnover).clamp(0.0, 1.0);
        turnover_scale = Some(scale);
        for candidate in candidates.iter_mut().filter(|c| !c.exit) {
            candidate.delta_weight *= scale;
        }
    }

    // Quantities.
    let mut sells: Vec<OrderIntent> = Vec::new();
    let mut buys: Vec<OrderIntent> = Vec::new();
    for candidate in &candidates {
        let i = candidate.asset;
        let price = input.prices[i];
        let current = weight(i);
        let target = input.targets[i].max(0.0);
        if candidate.exit {
            sells.push(OrderIntent {
                asset: i,
                side: Side::Sell,
                reason: OrderReason::Exit,
                qty: input.qty[i],
                price,
                notional: input.qty[i] * price,
                weight_before: current,
                weight_target: 0.0,
                weight_after: 0.0,
            });
            continue;
        }
        let raw_qty = candidate.delta_weight * nav / price;
        let (side, qty) = if raw_qty < 0.0 {
            (
                Side::Sell,
                round_qty(-raw_qty, params.fractional_shares, false).min(input.qty[i]),
            )
        } else {
            (
                Side::Buy,
                round_qty(raw_qty, params.fractional_shares, true),
            )
        };
        if qty <= 0.0 {
            skipped.push(SkippedTrade {
                asset: i,
                reason: SkipReason::RoundedToZero,
                weight_before: current,
                weight_target: target,
            });
            continue;
        }
        let notional = qty * price;
        if notional < params.min_trade_value {
            skipped.push(SkippedTrade {
                asset: i,
                reason: SkipReason::BelowMinTrade,
                weight_before: current,
                weight_target: target,
            });
            continue;
        }
        let selling_everything = side == Side::Sell && (input.qty[i] - qty).abs() < 1e-9;
        let reason = match side {
            Side::Sell if selling_everything => OrderReason::Exit,
            Side::Sell => OrderReason::Decrease,
            Side::Buy if input.qty[i] > 0.0 => OrderReason::Increase,
            Side::Buy => OrderReason::Entry,
        };
        let order = OrderIntent {
            asset: i,
            side,
            reason,
            qty,
            price,
            notional,
            weight_before: current,
            weight_target: target,
            weight_after: 0.0,
        };
        match side {
            Side::Sell => sells.push(order),
            Side::Buy => buys.push(order),
        }
    }

    // Cash feasibility: never spend more than cash plus net sale proceeds.
    let sale_cash: f64 = sells
        .iter()
        .map(|o| {
            costs.cash_delta(
                Side::Sell,
                o.qty,
                costs.execution_price(Side::Sell, o.price),
            )
        })
        .sum();
    let available = input.cash + sale_cash;
    let buy_cost = |orders: &[OrderIntent]| -> f64 {
        orders
            .iter()
            .map(|o| -costs.cash_delta(Side::Buy, o.qty, costs.execution_price(Side::Buy, o.price)))
            .sum()
    };
    let mut cash_scale = None;
    let required = buy_cost(&buys);
    if required > available && required > 0.0 {
        let scale = (available / required).clamp(0.0, 1.0);
        cash_scale = Some(scale);
        for order in buys.iter_mut() {
            order.qty = round_qty(order.qty * scale, params.fractional_shares, true);
        }
        // Commission minimums can still leave a small shortfall: trim the largest buys.
        let step = if params.fractional_shares {
            FRACTIONAL_STEP
        } else {
            1.0
        };
        while buy_cost(&buys) > available + 1e-9 {
            let Some(largest) = buys
                .iter_mut()
                .filter(|o| o.qty > 0.0)
                .max_by(|a, b| (a.qty * a.price).total_cmp(&(b.qty * b.price)))
            else {
                break;
            };
            largest.qty = (largest.qty - step).max(0.0);
        }
        buys.retain(|o| {
            let keep = o.qty > 0.0 && o.qty * o.price >= params.min_trade_value;
            if !keep {
                skipped.push(SkippedTrade {
                    asset: o.asset,
                    reason: SkipReason::BelowMinTrade,
                    weight_before: o.weight_before,
                    weight_target: o.weight_target,
                });
            }
            keep
        });
        for order in buys.iter_mut() {
            order.notional = order.qty * order.price;
        }
    }

    // Estimates and post-trade weights.
    let mut est_commission = 0.0;
    let mut est_slippage = 0.0;
    let mut est_fees = 0.0;
    let mut cash_after = input.cash;
    let mut qty_after = input.qty.to_vec();
    for order in sells.iter().chain(buys.iter()) {
        let exec = costs.execution_price(order.side, order.price);
        est_commission += costs.commission(order.qty, exec);
        est_fees += costs.fees(order.side, order.qty * exec);
        est_slippage += (exec - order.price).abs() * order.qty;
        cash_after += costs.cash_delta(order.side, order.qty, exec);
        qty_after[order.asset] += order.side.sign() * order.qty;
    }
    let nav_after = nav - est_commission - est_fees - est_slippage;
    for order in sells.iter_mut().chain(buys.iter_mut()) {
        order.weight_after = if nav_after > 0.0 {
            qty_after[order.asset] * order.price / nav_after
        } else {
            0.0
        };
    }

    sells.sort_by(|a, b| b.notional.total_cmp(&a.notional));
    buys.sort_by(|a, b| b.notional.total_cmp(&a.notional));
    let sell_value: f64 = sells.iter().map(|o| o.notional).sum();
    let buy_value: f64 = buys.iter().map(|o| o.notional).sum();
    let mut orders = sells;
    orders.extend(buys);
    skipped.sort_by_key(|s| s.asset);

    RebalancePlan {
        nav,
        turnover: if nav > 0.0 {
            (buy_value + sell_value) / 2.0 / nav
        } else {
            0.0
        },
        orders,
        skipped,
        buy_value,
        sell_value,
        est_commission,
        est_slippage,
        est_fees,
        cash_after,
        turnover_scale,
        cash_scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> RebalanceParams {
        RebalanceParams {
            band_abs: 0.01,
            band_rel: 0.25,
            min_trade_value: 100.0,
            max_turnover: 1.0,
            fractional_shares: false,
        }
    }

    fn run(
        cash: f64,
        qty: &[f64],
        prices: &[f64],
        targets: &[f64],
        p: &RebalanceParams,
    ) -> RebalancePlan {
        let tradable = vec![true; qty.len()];
        rebalance(
            &RebalanceInput {
                cash,
                qty,
                prices,
                targets,
                tradable: &tradable,
            },
            p,
            &CostModel::default(),
        )
    }

    #[test]
    fn initial_build_buys_to_target_with_whole_shares() {
        let plan = run(
            100_000.0,
            &[0.0, 0.0],
            &[100.0, 33.0],
            &[0.5, 0.4],
            &params(),
        );
        assert_eq!(plan.orders.len(), 2);
        let a = plan.orders.iter().find(|o| o.asset == 0).unwrap();
        assert_eq!(a.side, Side::Buy);
        assert_eq!(a.reason, OrderReason::Entry);
        assert_eq!(a.qty, 500.0);
        let b = plan.orders.iter().find(|o| o.asset == 1).unwrap();
        assert_eq!(b.qty, (40_000.0f64 / 33.0).floor());
        assert!(plan.cash_after >= 0.0);
    }

    #[test]
    fn drift_inside_band_is_left_alone() {
        // 52% vs 50% target: drift 2pt < max(1pt, 12.5pt) band.
        let plan = run(48_000.0, &[520.0], &[100.0], &[0.5], &params());
        assert!(plan.orders.is_empty());
        assert_eq!(plan.skipped[0].reason, SkipReason::WithinBand);
    }

    #[test]
    fn a_portfolio_far_below_target_tops_up_names_inside_their_bands() {
        // Ten names targeted at 9% (90% invested) held at 7.5% each (75% invested): each drift of
        // 1.5pt is inside its 2.25pt band, but the portfolio is 15pt short of target.
        let qty = [75.0; 10];
        let prices = [100.0; 10];
        let targets = [0.09; 10];
        let plan = run(25_000.0, &qty, &prices, &targets, &params());
        assert_eq!(plan.orders.len(), 10);
        assert!(
            plan.orders
                .iter()
                .all(|o| o.side == Side::Buy && o.qty == 15.0)
        );

        // Within the cash band (86% vs 90%), the name bands apply again.
        let qty = [86.0; 10];
        let plan = run(14_000.0, &qty, &prices, &targets, &params());
        assert!(plan.orders.is_empty());
        assert!(
            plan.skipped
                .iter()
                .all(|s| s.reason == SkipReason::WithinBand)
        );
    }

    #[test]
    fn exits_sell_everything_regardless_of_size() {
        let mut p = params();
        p.min_trade_value = 1_000_000.0;
        let plan = run(10_000.0, &[3.0], &[50.0], &[0.0], &p);
        assert_eq!(plan.orders.len(), 1);
        assert_eq!(plan.orders[0].reason, OrderReason::Exit);
        assert_eq!(plan.orders[0].qty, 3.0);
    }

    #[test]
    fn not_tradable_names_are_skipped() {
        let tradable = [false];
        let plan = rebalance(
            &RebalanceInput {
                cash: 10_000.0,
                qty: &[10.0],
                prices: &[100.0],
                targets: &[0.0],
                tradable: &tradable,
            },
            &params(),
            &CostModel::default(),
        );
        assert!(plan.orders.is_empty());
        assert_eq!(plan.skipped[0].reason, SkipReason::NotTradable);
    }

    #[test]
    fn turnover_cap_scales_non_exit_trades() {
        let mut p = params();
        p.max_turnover = 0.1;
        let plan = run(100_000.0, &[0.0, 0.0], &[100.0, 100.0], &[0.5, 0.5], &p);
        assert!(plan.turnover_scale.unwrap() < 1.0);
        assert!(plan.turnover <= 0.1 + 1e-9);
    }

    #[test]
    fn buys_never_exceed_available_cash() {
        // Targets sum above 100% of NAV → cash constraint binds.
        let plan = run(10_000.0, &[0.0, 0.0], &[10.0, 10.0], &[0.8, 0.8], &params());
        assert!(plan.cash_scale.is_some());
        assert!(plan.cash_after >= -1e-9, "cash_after {}", plan.cash_after);
    }

    #[test]
    fn sells_fund_buys_and_orders_are_sells_first() {
        // Rotate from asset 0 to asset 1 with no spare cash.
        let plan = run(0.0, &[100.0, 0.0], &[100.0, 50.0], &[0.0, 0.98], &params());
        assert_eq!(plan.orders[0].side, Side::Sell);
        assert_eq!(plan.orders[1].side, Side::Buy);
        assert!(plan.cash_after >= -1e-9);
        assert!(plan.orders[1].qty > 150.0);
    }

    #[test]
    fn small_trades_are_dropped() {
        let mut p = params();
        p.band_abs = 0.0;
        p.band_rel = 0.0;
        p.min_trade_value = 5_000.0;
        // Need +1% of 100k = $1,000 < $5,000 minimum.
        let plan = run(51_000.0, &[490.0], &[100.0], &[0.5], &p);
        assert!(plan.orders.is_empty());
        assert_eq!(plan.skipped[0].reason, SkipReason::BelowMinTrade);
    }

    #[test]
    fn fractional_shares_round_to_step() {
        let mut p = params();
        p.fractional_shares = true;
        let plan = run(1_000.0, &[0.0], &[333.0], &[0.5], &p);
        let qty = plan.orders[0].qty;
        assert!((qty * 10_000.0 - (qty * 10_000.0).round()).abs() < 1e-6);
        assert!(qty > 1.0 && qty < 1.51);
    }

    #[test]
    fn partial_decrease_is_labelled() {
        let plan = run(
            0.0,
            &[1_000.0, 0.0],
            &[100.0, 100.0],
            &[0.5, 0.48],
            &params(),
        );
        let sell = plan.orders.iter().find(|o| o.side == Side::Sell).unwrap();
        assert_eq!(sell.reason, OrderReason::Decrease);
        assert_eq!(sell.qty, 500.0);
    }
}
