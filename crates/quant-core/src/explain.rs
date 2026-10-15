//! Why a plan differs from the plan before it.
//!
//! Every plan stores the engine's full output, but raw numbers do not say *why* a trade happened.
//! [`explain`] compares a plan with the account's previous plan and returns a structured
//! explanation made of codes and numbers, never prose, so the UI and the notifications can render
//! it in either language.
//!
//! - **Plan level**: market breadth and the exposure it implies (and which names crossed the
//!   breadth line), the cash gap against [`CASH_BAND`] and which side's bands it waived, the
//!   turnover and cash caps, trend breaks, the sector budget that moved most, frozen and excluded
//!   names, and, for a plan without orders, why nothing traded.
//! - **Order level**: the trade `target − before` is the model change (`target − previous
//!   target`) minus drift (`before − previous target`). The model change is split in log space
//!   into exposure, sector share, trend, momentum, volatility and a remainder (peers and caps);
//!   drift into the price move since the previous plan and what that plan left undone (a turnover
//!   or cash cap, an order that was rejected or did not fill). The dominant driver is the largest
//!   piece pushing in the direction of the trade; drift dominates when it is larger than the model
//!   change. Entries and exits are explained by the name's status change.
//!
//! Plans stored before these diagnostics existed are explained from what they did store: the band
//! waiver is derived from NAV, cash and the target invested weight, and the breadth line from the
//! trend signal when both use the same moving average.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::costs::Side;
use crate::rebalance::{BandWaiver, CASH_BAND, SkipReason};
use crate::strategy::{AssetStatus, StrategyParams};

/// Bumped when the explanation's shape or rules change.
pub const EXPLAIN_VERSION: u32 = 1;

/// A trend multiplier move larger than this counts as a trend break (or recovery).
pub const TREND_BREAK: f64 = 0.1;

const EPS: f64 = 1e-9;

// ---------------------------------------------------------------------------------------------
// Inputs: one stored plan
// ---------------------------------------------------------------------------------------------

/// The plan row's own columns.
#[derive(Debug, Clone, Default)]
pub struct PlanHeader {
    pub id: i64,
    pub status: String,
    pub strategy_version_id: Option<i64>,
    pub nav: f64,
    pub cash: f64,
    pub exposure_target: Option<f64>,
    pub invested_target: Option<f64>,
    pub breadth: Option<f64>,
    pub est_vol: Option<f64>,
}

/// One stored order and how it ended.
#[derive(Debug, Clone, Deserialize)]
pub struct OrderRecord {
    pub symbol: String,
    pub side: Side,
    pub qty: f64,
    pub weight_before: f64,
    pub weight_target: f64,
    pub weight_after: f64,
    /// `planned`, `filled`, `partially_filled`, `rejected`, `skipped`, `cancelled`, `expired`.
    pub status: String,
    #[serde(default)]
    pub status_reason: Option<String>,
    #[serde(default)]
    pub filled_qty: f64,
}

impl OrderRecord {
    fn filled_fraction(&self) -> f64 {
        if self.qty > 0.0 {
            (self.filled_qty / self.qty).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The name's weight right after this order: the planned weight scaled by how much filled.
    fn weight_left(&self) -> f64 {
        self.weight_before + (self.weight_after - self.weight_before) * self.filled_fraction()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flag {
    pub symbol: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SkippedRecord {
    pub symbol: String,
    pub reason: SkipReason,
    pub weight_before: f64,
    pub weight_target: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetRecord {
    pub symbol: String,
    pub sector: String,
    pub status: AssetStatus,
    #[serde(default)]
    pub price: Option<f64>,
    #[serde(default)]
    pub vol: Option<f64>,
    #[serde(default)]
    pub trend: Option<f64>,
    #[serde(default = "one")]
    pub momentum_multiplier: f64,
    #[serde(default = "one")]
    pub trend_multiplier: f64,
    #[serde(default)]
    pub weight: f64,
    #[serde(default)]
    pub above_breadth_line: Option<bool>,
}

fn one() -> f64 {
    1.0
}

/// Where the per-name breadth-line positions come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreadthSource {
    /// Stored by the engine.
    Recorded,
    /// Older plan: `trend > 0`, because the trend and breadth averages had the same length.
    Reconstructed,
    /// Older plan whose trend and breadth averages differ, or breadth scaling was off.
    Unavailable,
}

/// The portfolio-level gap that decides whether name-level bands are waived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CashGap {
    pub invested_now: f64,
    pub invested_target: f64,
    /// `invested_target − invested_now`.
    pub gap: f64,
    pub band: f64,
    pub waiver: Option<BandWaiver>,
    /// Derived from NAV, cash and the target invested weight (older plans), not recorded.
    pub derived: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct StoredDiagnostics {
    targets: Option<StoredTargets>,
    skipped: Vec<SkippedRecord>,
    frozen: Vec<Flag>,
    excluded: Vec<Flag>,
    quotes: BTreeMap<String, StoredQuote>,
    params: Option<StrategyParams>,
    rebalance: StoredRebalance,
}

#[derive(Debug, Clone, Deserialize)]
struct StoredTargets {
    assets: Vec<AssetRecord>,
    #[serde(default)]
    sectors: Vec<StoredSector>,
    #[serde(default)]
    vol_scale: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct StoredSector {
    sector: String,
    budget: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct StoredQuote {
    price: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct StoredRebalance {
    turnover_scale: Option<f64>,
    cash_scale: Option<f64>,
    invested_now: Option<f64>,
    invested_target: Option<f64>,
    cash_gap: Option<f64>,
    band_waiver: Option<BandWaiver>,
    band_waived: Option<Vec<String>>,
}

/// One plan as the explanation needs it.
#[derive(Debug, Clone)]
pub struct PlanSnapshot {
    pub header: PlanHeader,
    pub params: StrategyParams,
    pub assets: Vec<AssetRecord>,
    pub breadth_source: BreadthSource,
    /// Sector budgets before name-level pruning and volatility targeting.
    pub sector_budgets: BTreeMap<String, f64>,
    pub vol_scale: Option<f64>,
    pub quotes: BTreeMap<String, f64>,
    pub turnover_scale: Option<f64>,
    pub cash_scale: Option<f64>,
    pub cash: CashGap,
    pub band_waived: BTreeSet<String>,
    pub frozen: Vec<Flag>,
    pub excluded: Vec<Flag>,
    pub skipped: Vec<SkippedRecord>,
    pub orders: Vec<OrderRecord>,
}

impl PlanSnapshot {
    /// Builds a snapshot from a plan's columns, its stored diagnostics and its orders. `None` for
    /// a plan without engine output (a skipped slot).
    pub fn from_stored(
        header: PlanHeader,
        diagnostics: &Value,
        orders: Vec<OrderRecord>,
    ) -> Option<Self> {
        let stored: StoredDiagnostics = serde_json::from_value(diagnostics.clone()).ok()?;
        let targets = stored.targets?;
        let params = stored.params.unwrap_or_default();
        let mut assets = targets.assets;

        let breadth_source = if assets.iter().any(|a| a.above_breadth_line.is_some()) {
            BreadthSource::Recorded
        } else if params.exposure.breadth_scaling
            && params.asset.trend_sma == params.exposure.breadth_sma
            && params.asset.trend_sma > 0
        {
            // Both signals are SMA(n) of the same window (closes plus the decision price), and
            // `trend = price / SMA − 1`, so `trend > 0` is exactly `price > SMA`. Breadth was
            // measured before pruning, so names later pruned below the minimum weight count too.
            for a in assets.iter_mut() {
                if matches!(a.status, AssetStatus::Active | AssetStatus::BelowMinWeight) {
                    a.above_breadth_line = a.trend.map(|t| t > 0.0);
                }
            }
            BreadthSource::Reconstructed
        } else {
            BreadthSource::Unavailable
        };

        let r = &stored.rebalance;
        let cash = match (r.cash_gap, r.invested_now, r.invested_target) {
            (Some(gap), Some(invested_now), Some(invested_target)) => CashGap {
                invested_now,
                invested_target,
                gap,
                band: CASH_BAND,
                waiver: r.band_waiver,
                derived: false,
            },
            _ => {
                // The rebalancer values the book at the same prices as the plan's NAV, so the
                // invested weight before trading is everything but cash.
                let invested_now = if header.nav > 0.0 {
                    (header.nav - header.cash) / header.nav
                } else {
                    0.0
                };
                let invested_target = header
                    .invested_target
                    .unwrap_or_else(|| assets.iter().map(|a| a.weight.max(0.0)).sum());
                let gap = invested_target - invested_now;
                CashGap {
                    invested_now,
                    invested_target,
                    gap,
                    band: CASH_BAND,
                    waiver: waiver_for(gap),
                    derived: true,
                }
            }
        };

        let band_waived = match &r.band_waived {
            Some(list) => list.iter().cloned().collect(),
            // Older plans: a held name that traded inside its own band on the waived side.
            None => orders
                .iter()
                .filter(|o| {
                    let drift = o.weight_target - o.weight_before;
                    let band = params
                        .rebalance
                        .band_abs
                        .max(params.rebalance.band_rel * o.weight_target);
                    let side_waived = match cash.waiver {
                        Some(BandWaiver::Buys) => drift > 0.0,
                        Some(BandWaiver::Sells) => drift < 0.0,
                        None => false,
                    };
                    o.weight_before > 0.0
                        && o.weight_target > 0.0
                        && side_waived
                        && drift.abs() <= band
                })
                .map(|o| o.symbol.clone())
                .collect(),
        };

        let mut quotes: BTreeMap<String, f64> = stored
            .quotes
            .into_iter()
            .filter(|(_, q)| q.price > 0.0)
            .map(|(s, q)| (s, q.price))
            .collect();
        for a in &assets {
            if let Some(p) = a.price {
                quotes.entry(a.symbol.clone()).or_insert(p);
            }
        }

        Some(Self {
            header,
            params,
            assets,
            breadth_source,
            sector_budgets: targets
                .sectors
                .into_iter()
                .map(|s| (s.sector, s.budget))
                .collect(),
            vol_scale: targets.vol_scale,
            quotes,
            turnover_scale: r.turnover_scale,
            cash_scale: r.cash_scale,
            cash,
            band_waived,
            frozen: stored.frozen,
            excluded: stored.excluded,
            skipped: stored.skipped,
            orders,
        })
    }

    fn asset(&self, symbol: &str) -> Option<&AssetRecord> {
        self.assets.iter().find(|a| a.symbol == symbol)
    }

    fn target(&self, symbol: &str) -> f64 {
        self.asset(symbol).map(|a| a.weight.max(0.0)).unwrap_or(0.0)
    }

    /// Exposure the allocation was scaled to: the breadth target times any volatility scaling.
    fn effective_exposure(&self) -> Option<f64> {
        self.header
            .exposure_target
            .map(|e| e * self.vol_scale.unwrap_or(1.0))
    }

    /// (names above the line, names counted).
    fn breadth_counts(&self) -> Option<(usize, usize)> {
        if self.breadth_source == BreadthSource::Unavailable {
            return None;
        }
        let counted: Vec<bool> = self
            .assets
            .iter()
            .filter_map(|a| a.above_breadth_line)
            .collect();
        Some((counted.iter().filter(|&&x| x).count(), counted.len()))
    }

    fn universe(&self) -> usize {
        self.assets
            .iter()
            .filter(|a| {
                !self
                    .excluded
                    .iter()
                    .any(|f| f.symbol == a.symbol && f.reason == "not_in_universe")
            })
            .count()
    }

    fn count_status(&self, status: AssetStatus) -> usize {
        self.assets.iter().filter(|a| a.status == status).count()
    }
}

fn waiver_for(gap: f64) -> Option<BandWaiver> {
    if gap > CASH_BAND {
        Some(BandWaiver::Buys)
    } else if gap < -CASH_BAND {
        Some(BandWaiver::Sells)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------------------------

/// What moved one order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderDriver {
    /// No previous plan to compare with.
    FirstPlan,
    /// Entry: the name now has enough history to be eligible.
    NewlyEligible,
    /// Entry: the name was not in the previous plan's universe.
    AddedToUniverse,
    /// Entry: the name was pruned below the minimum weight last time.
    BackAboveMinWeight,
    /// Entry: an exclusion or lock was lifted.
    RestrictionLifted,
    /// Entry: the trend filter had removed the name entirely.
    TrendReentry,
    /// Exit: pruned below the minimum weight.
    BelowMinWeight,
    /// Exit: the trend filter removes the name (trend penalty 0).
    TrendExit,
    /// Exit: excluded by an operator.
    Excluded,
    /// Exit: no longer in the universe.
    NotInUniverse,
    /// Exit: no longer eligible (no price or not enough history).
    LostEligibility,
    /// Model: total exposure (market breadth, volatility targeting).
    Exposure,
    /// Model: the sector's share of the exposure.
    SectorBudget,
    /// Model: the name's trend multiplier.
    Trend,
    /// Model: the name's momentum multiplier.
    Momentum,
    /// Model: the name's volatility.
    Volatility,
    /// Model: the rest (peers' scores within the sector, caps, pruning).
    Peers,
    /// Model: the rest, in a sector that gained newly eligible names that now share its budget.
    NewPeers,
    /// Drift: the price moved since the previous plan.
    PriceDrift,
    /// Drift: the previous plan was cut short by the turnover cap.
    CarryOverTurnover,
    /// Drift: the previous plan's buys were cut short by cash.
    CarryOverCash,
    /// Drift: the previous plan's order for this name was rejected or did not fill.
    Retry,
}

/// One piece of an order's model change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub driver: OrderDriver,
    /// Contribution to ln(target / previous target).
    pub log: f64,
    /// The same contribution in weight points (shares of the model change).
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderExplanation {
    pub symbol: String,
    pub sector: Option<String>,
    pub side: Side,
    pub weight_before: f64,
    pub weight_target: f64,
    pub previous_target: Option<f64>,
    /// `weight_before − previous_target`.
    pub drift: Option<f64>,
    /// `weight_target − previous_target`.
    pub model: Option<f64>,
    pub driver: OrderDriver,
    pub status_before: Option<AssetStatus>,
    pub status_now: Option<AssetStatus>,
    /// Model change split by cause (only when both targets are positive).
    pub components: Vec<Component>,
    /// Part of the drift caused by price moves since the previous plan.
    pub price_drift: Option<f64>,
    /// Part of the drift the previous plan left undone, and why.
    pub left_over: Option<f64>,
    pub left_over_cause: Option<OrderDriver>,
    pub previous_order_status: Option<String>,
    pub previous_status_reason: Option<String>,
    /// The name's price change between the two plans' quotes.
    pub price_change: Option<f64>,
    pub trend_multiplier_before: Option<f64>,
    pub trend_multiplier_now: Option<f64>,
    /// Traded only because the portfolio-level gap waived the name's band.
    pub band_waived: bool,
    /// This plan's turnover or cash cap shortened the order.
    pub scaled: bool,
}

/// Orders whose dominant driver is `driver`, and the weight they trade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriverTally {
    pub driver: OrderDriver,
    pub orders: usize,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delta {
    pub before: Option<f64>,
    pub now: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreadthView {
    pub source: BreadthSource,
    pub before: Option<f64>,
    pub now: Option<f64>,
    pub above_before: Option<usize>,
    pub counted_before: Option<usize>,
    pub above_now: Option<usize>,
    pub counted_now: Option<usize>,
    pub crossed_below: Vec<String>,
    pub crossed_above: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendChange {
    pub symbol: String,
    pub before: f64,
    pub now: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorChange {
    pub sector: String,
    pub before: Option<f64>,
    pub now: f64,
    /// Budget change beyond what the change in exposure alone implies.
    pub share_effect: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoActionReason {
    /// Every name was within its drift band (and the cash gap within tolerance).
    WithinBands,
    /// The trades that were due were all below the minimum trade value.
    BelowMinTrade,
    /// The names that should trade are locked or have no usable price.
    NotTradable,
    /// Nothing held and nothing targeted.
    NothingToTrade,
}

/// How much of its band a held name has used: it trades when |drift| exceeds `band`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BandUse {
    pub symbol: String,
    /// `target − before`.
    pub drift: f64,
    pub band: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoAction {
    pub reason: NoActionReason,
    pub within_band: usize,
    pub below_min_trade: usize,
    pub not_tradable: usize,
    pub rounded: usize,
    /// The name closest to its band.
    pub closest: Option<BandUse>,
}

/// One reason a plan looks the way it does, in headline order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum PlanDriver {
    FirstPlan {
        eligible: usize,
        universe: usize,
    },
    StrategyChanged {
        from: Option<i64>,
        to: Option<i64>,
    },
    /// Many names lack the history to be eligible.
    HistoryLimited {
        insufficient: usize,
        universe: usize,
    },
    NoAction {
        reason: NoActionReason,
        gap: f64,
    },
    NewlyEligible {
        count: usize,
        orders: usize,
        weight: f64,
    },
    Breadth {
        /// Length of the breadth moving average (in observations, including the live price).
        line: usize,
        before: f64,
        now: f64,
        crossed_below: usize,
        crossed_above: usize,
        exposure_before: f64,
        exposure_now: f64,
        orders: usize,
        weight: f64,
    },
    TrendBreaks {
        count: usize,
        orders: usize,
        weight: f64,
    },
    TrendRecoveries {
        count: usize,
        orders: usize,
        weight: f64,
    },
    SectorBudget {
        sector: String,
        before: f64,
        now: f64,
        orders: usize,
        weight: f64,
    },
    /// Orders whose driver has no plan-level context of its own (momentum, volatility, peers,
    /// minimum weight, exclusions, a new universe member…).
    Orders {
        driver: OrderDriver,
        orders: usize,
        weight: f64,
    },
    PriceDrift {
        orders: usize,
        weight: f64,
    },
    CarryOver {
        previous_turnover_scale: Option<f64>,
        previous_cash_scale: Option<f64>,
        orders: usize,
        weight: f64,
    },
    Retry {
        orders: usize,
        weight: f64,
    },
    TurnoverCap {
        scale: f64,
        /// The one-way turnover ceiling.
        limit: f64,
    },
    CashCap {
        scale: f64,
    },
    BandWaiver {
        side: BandWaiver,
        gap: f64,
        invested_now: f64,
        invested_target: f64,
        orders: usize,
    },
    Frozen {
        count: usize,
    },
    Excluded {
        count: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanExplanation {
    pub version: u32,
    pub previous_plan_id: Option<i64>,
    pub drivers: Vec<PlanDriver>,
    pub breadth: BreadthView,
    pub exposure: Delta,
    pub est_vol: Delta,
    pub cash: CashGap,
    pub turnover_scale: Option<f64>,
    pub cash_scale: Option<f64>,
    pub universe: usize,
    pub insufficient_history: usize,
    pub newly_eligible: Vec<String>,
    pub trend_breaks: Vec<TrendChange>,
    pub trend_recoveries: Vec<TrendChange>,
    pub sectors: Vec<SectorChange>,
    pub frozen: Vec<Flag>,
    pub excluded: Vec<Flag>,
    pub no_action: Option<NoAction>,
    /// Orders by dominant driver, largest traded weight first.
    pub tally: Vec<DriverTally>,
    pub orders: Vec<OrderExplanation>,
}

/// A plan's headline in a few bytes, for lists and notifications.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Headline {
    pub previous_plan_id: Option<i64>,
    pub drivers: Vec<PlanDriver>,
}

impl PlanExplanation {
    /// The first `n` drivers.
    pub fn headline(&self, n: usize) -> Headline {
        Headline {
            previous_plan_id: self.previous_plan_id,
            drivers: self.drivers.iter().take(n).cloned().collect(),
        }
    }

    pub fn order(&self, symbol: &str) -> Option<&OrderExplanation> {
        self.orders.iter().find(|o| o.symbol == symbol)
    }
}

impl PlanDriver {
    pub fn code(&self) -> &'static str {
        match self {
            PlanDriver::FirstPlan { .. } => "first_plan",
            PlanDriver::StrategyChanged { .. } => "strategy_changed",
            PlanDriver::HistoryLimited { .. } => "history_limited",
            PlanDriver::NoAction { .. } => "no_action",
            PlanDriver::NewlyEligible { .. } => "newly_eligible",
            PlanDriver::Breadth { .. } => "breadth",
            PlanDriver::TrendBreaks { .. } => "trend_breaks",
            PlanDriver::TrendRecoveries { .. } => "trend_recoveries",
            PlanDriver::SectorBudget { .. } => "sector_budget",
            PlanDriver::Orders { .. } => "orders",
            PlanDriver::PriceDrift { .. } => "price_drift",
            PlanDriver::CarryOver { .. } => "carry_over",
            PlanDriver::Retry { .. } => "retry",
            PlanDriver::TurnoverCap { .. } => "turnover_cap",
            PlanDriver::CashCap { .. } => "cash_cap",
            PlanDriver::BandWaiver { .. } => "band_waiver",
            PlanDriver::Frozen { .. } => "frozen",
            PlanDriver::Excluded { .. } => "excluded",
        }
    }

    /// Traded weight attributed to this driver (0 for context-only drivers).
    fn weight(&self) -> f64 {
        match self {
            PlanDriver::NewlyEligible { weight, .. }
            | PlanDriver::Breadth { weight, .. }
            | PlanDriver::TrendBreaks { weight, .. }
            | PlanDriver::TrendRecoveries { weight, .. }
            | PlanDriver::SectorBudget { weight, .. }
            | PlanDriver::Orders { weight, .. }
            | PlanDriver::PriceDrift { weight, .. }
            | PlanDriver::CarryOver { weight, .. }
            | PlanDriver::Retry { weight, .. } => *weight,
            _ => 0.0,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Explanation
// ---------------------------------------------------------------------------------------------

/// Explains `plan` against the account's previous plan with engine output (`None` for the first).
pub fn explain(plan: &PlanSnapshot, previous: Option<&PlanSnapshot>) -> PlanExplanation {
    let orders: Vec<OrderExplanation> = plan
        .orders
        .iter()
        .map(|o| explain_order(plan, previous, o))
        .collect();

    // Tally orders by dominant driver.
    let mut tally_map: BTreeMap<OrderDriver, (usize, f64)> = BTreeMap::new();
    for o in &orders {
        let entry = tally_map.entry(o.driver).or_default();
        entry.0 += 1;
        entry.1 += (o.weight_target - o.weight_before).abs();
    }
    let mut tally: Vec<DriverTally> = tally_map
        .iter()
        .map(|(driver, (orders, weight))| DriverTally {
            driver: *driver,
            orders: *orders,
            weight: *weight,
        })
        .collect();
    tally.sort_by(|a, b| b.weight.total_cmp(&a.weight));

    // Breadth.
    let now_counts = plan.breadth_counts();
    let before_counts = previous.and_then(|p| p.breadth_counts());
    let mut crossed_below = Vec::new();
    let mut crossed_above = Vec::new();
    if let Some(prev) = previous {
        for a in &plan.assets {
            let (Some(now), Some(before)) = (
                a.above_breadth_line,
                prev.asset(&a.symbol).and_then(|p| p.above_breadth_line),
            ) else {
                continue;
            };
            if before && !now {
                crossed_below.push(a.symbol.clone());
            } else if !before && now {
                crossed_above.push(a.symbol.clone());
            }
        }
    }
    let breadth = BreadthView {
        source: plan.breadth_source,
        before: previous.and_then(|p| p.header.breadth),
        now: plan.header.breadth,
        above_before: before_counts.map(|c| c.0),
        counted_before: before_counts.map(|c| c.1),
        above_now: now_counts.map(|c| c.0),
        counted_now: now_counts.map(|c| c.1),
        crossed_below,
        crossed_above,
    };

    // Trend breaks and recoveries among names active in both plans.
    let mut trend_breaks = Vec::new();
    let mut trend_recoveries = Vec::new();
    let mut newly_eligible = Vec::new();
    if let Some(prev) = previous {
        for a in &plan.assets {
            // Newly eligible names that received weight (others were pruned straight away).
            let Some(p) = prev.asset(&a.symbol) else {
                if a.status == AssetStatus::Active && a.weight > 0.0 {
                    newly_eligible.push(a.symbol.clone());
                }
                continue;
            };
            if a.status == AssetStatus::Active
                && a.weight > 0.0
                && p.status == AssetStatus::InsufficientHistory
            {
                newly_eligible.push(a.symbol.clone());
            }
            if !(eligible(a.status) && eligible(p.status)) {
                continue;
            }
            let change = TrendChange {
                symbol: a.symbol.clone(),
                before: p.trend_multiplier,
                now: a.trend_multiplier,
            };
            if a.trend_multiplier < p.trend_multiplier - TREND_BREAK {
                trend_breaks.push(change);
            } else if a.trend_multiplier > p.trend_multiplier + TREND_BREAK {
                trend_recoveries.push(change);
            }
        }
    }

    // Sector budgets: the change beyond what the exposure change alone implies.
    let exposure_ratio = match (
        plan.effective_exposure(),
        previous.and_then(|p| p.effective_exposure()),
    ) {
        (Some(e1), Some(e0)) if e0 > 0.0 => Some(e1 / e0),
        _ => None,
    };
    let mut sectors: Vec<SectorChange> = plan
        .sector_budgets
        .iter()
        .map(|(sector, now)| {
            let before = previous.and_then(|p| p.sector_budgets.get(sector).copied());
            SectorChange {
                sector: sector.clone(),
                before,
                now: *now,
                share_effect: before.zip(exposure_ratio).map(|(b, r)| now - b * r),
            }
        })
        .collect();
    // Rounded so that float noise (a JSON round trip) cannot reorder sectors that did not move.
    sectors.sort_by(|a, b| {
        let key = |s: &SectorChange| (s.share_effect.unwrap_or(0.0).abs() * 1e9).round();
        key(b).total_cmp(&key(a)).then(a.sector.cmp(&b.sector))
    });

    let no_action = plan.orders.is_empty().then(|| no_action(plan));

    let mut explanation = PlanExplanation {
        version: EXPLAIN_VERSION,
        previous_plan_id: previous.map(|p| p.header.id),
        drivers: Vec::new(),
        breadth,
        exposure: Delta {
            before: previous.and_then(|p| p.header.exposure_target),
            now: plan.header.exposure_target,
        },
        est_vol: Delta {
            before: previous.and_then(|p| p.header.est_vol),
            now: plan.header.est_vol,
        },
        cash: plan.cash.clone(),
        turnover_scale: plan.turnover_scale,
        cash_scale: plan.cash_scale,
        universe: plan.universe(),
        insufficient_history: plan.count_status(AssetStatus::InsufficientHistory),
        newly_eligible,
        trend_breaks,
        trend_recoveries,
        sectors,
        frozen: plan.frozen.clone(),
        excluded: plan.excluded.clone(),
        no_action,
        tally,
        orders,
    };
    explanation.drivers = plan_drivers(plan, previous, &explanation);
    explanation
}

fn eligible(status: AssetStatus) -> bool {
    matches!(status, AssetStatus::Active | AssetStatus::BelowMinWeight)
}

fn no_action(plan: &PlanSnapshot) -> NoAction {
    let count = |reason: SkipReason| plan.skipped.iter().filter(|s| s.reason == reason).count();
    let within_band = count(SkipReason::WithinBand);
    let below_min_trade = count(SkipReason::BelowMinTrade);
    let not_tradable = count(SkipReason::NotTradable);
    let rounded = count(SkipReason::RoundedToZero);
    let rp = &plan.params.rebalance;
    let closest = plan
        .skipped
        .iter()
        .filter(|s| s.reason == SkipReason::WithinBand)
        .map(|s| BandUse {
            symbol: s.symbol.clone(),
            drift: s.weight_target - s.weight_before,
            band: rp.band_abs.max(rp.band_rel * s.weight_target),
        })
        .filter(|b| b.band > 0.0)
        .max_by(|a, b| (a.drift.abs() / a.band).total_cmp(&(b.drift.abs() / b.band)));
    let reason = if plan.skipped.is_empty() {
        NoActionReason::NothingToTrade
    } else if below_min_trade + rounded > within_band.max(not_tradable) {
        NoActionReason::BelowMinTrade
    } else if not_tradable > within_band {
        NoActionReason::NotTradable
    } else {
        NoActionReason::WithinBands
    };
    NoAction {
        reason,
        within_band,
        below_min_trade,
        not_tradable,
        rounded,
        closest,
    }
}

fn explain_order(
    plan: &PlanSnapshot,
    previous: Option<&PlanSnapshot>,
    order: &OrderRecord,
) -> OrderExplanation {
    let symbol = order.symbol.as_str();
    let now = plan.asset(symbol);
    let t1 = order.weight_target.max(0.0);
    let wb1 = order.weight_before.max(0.0);
    let scaled = order.weight_target > 0.0
        && (plan.turnover_scale.is_some()
            || (order.side == Side::Buy && plan.cash_scale.is_some()))
        && (order.weight_after - order.weight_target).abs() > 1e-4;
    let mut out = OrderExplanation {
        symbol: order.symbol.clone(),
        sector: now.map(|a| a.sector.clone()),
        side: order.side,
        weight_before: order.weight_before,
        weight_target: order.weight_target,
        previous_target: None,
        drift: None,
        model: None,
        driver: OrderDriver::FirstPlan,
        status_before: None,
        status_now: now.map(|a| a.status),
        components: Vec::new(),
        price_drift: None,
        left_over: None,
        left_over_cause: None,
        previous_order_status: None,
        previous_status_reason: None,
        price_change: None,
        trend_multiplier_before: None,
        trend_multiplier_now: now.map(|a| a.trend_multiplier),
        band_waived: plan.band_waived.contains(symbol),
        scaled,
    };
    let Some(prev) = previous else {
        return out;
    };
    let before = prev.asset(symbol);
    let t0 = prev.target(symbol);
    out.status_before = before.map(|a| a.status);
    out.trend_multiplier_before = before.map(|a| a.trend_multiplier);
    out.previous_target = Some(t0);
    out.drift = Some(wb1 - t0);
    out.model = Some(t1 - t0);
    out.price_change = match (prev.quotes.get(symbol), plan.quotes.get(symbol)) {
        (Some(p0), Some(p1)) if *p0 > 0.0 => Some(p1 / p0 - 1.0),
        _ => None,
    };

    // Drift = what the previous plan left undone + what prices did since.
    let previous_order = prev.orders.iter().find(|o| o.symbol == symbol);
    let left_weight = match previous_order {
        Some(o) => o.weight_left(),
        None => prev
            .skipped
            .iter()
            .find(|s| s.symbol == symbol)
            .map(|s| s.weight_before)
            .unwrap_or(t0),
    };
    let cause = previous_order.and_then(|o| {
        out.previous_order_status = Some(o.status.clone());
        out.previous_status_reason = o.status_reason.clone();
        if o.filled_fraction() < 1.0 - EPS {
            Some(OrderDriver::Retry)
        } else if o.side == Side::Buy && prev.cash_scale.is_some() {
            Some(OrderDriver::CarryOverCash)
        } else if prev.turnover_scale.is_some() {
            Some(OrderDriver::CarryOverTurnover)
        } else {
            None
        }
    });
    // A residual from a band, the minimum trade or rounding is just older price drift.
    let (price_drift, left_over) = match cause {
        Some(_) => (wb1 - left_weight, left_weight - t0),
        None => (wb1 - t0, 0.0),
    };
    out.price_drift = Some(price_drift);
    if cause.is_some() {
        out.left_over = Some(left_over);
        out.left_over_cause = cause;
    }
    let drift_driver = || {
        // The drift piece that pushes hardest in the direction of the trade (−drift).
        let direction = t1 - wb1;
        let mut best = (OrderDriver::PriceDrift, 0.0);
        for (driver, piece) in [
            (OrderDriver::PriceDrift, price_drift),
            (cause.unwrap_or(OrderDriver::PriceDrift), left_over),
        ] {
            let push = -piece * direction.signum();
            if push > best.1 {
                best = (driver, push);
            }
        }
        best.0
    };

    let zero = |w: f64| w <= 1e-12;
    out.driver = if zero(t0) && zero(t1) {
        // Neither plan wants the name: finishing an exit that did not complete.
        drift_driver()
    } else if zero(t1) {
        exit_driver(plan, now)
    } else if zero(t0) {
        if wb1 > 0.0 && previous_order.is_some() && cause == Some(OrderDriver::Retry) {
            OrderDriver::Retry
        } else {
            entry_driver(before)
        }
    } else {
        let drift = wb1 - t0;
        let model = t1 - t0;
        out.components = components(plan, prev, now, before, t0, t1);
        if drift.abs() > model.abs() {
            drift_driver()
        } else {
            let dominant = out
                .components
                .iter()
                .filter(|c| c.log * model > 0.0)
                .max_by(|a, b| a.log.abs().total_cmp(&b.log.abs()))
                .map(|c| c.driver)
                .unwrap_or(OrderDriver::Peers);
            let sector_gained_names = now.is_some_and(|a| {
                plan.assets.iter().any(|other| {
                    other.sector == a.sector
                        && eligible(other.status)
                        && prev.asset(&other.symbol).is_none_or(|p| {
                            matches!(
                                p.status,
                                AssetStatus::InsufficientHistory | AssetStatus::NoPrice
                            )
                        })
                })
            });
            if dominant == OrderDriver::Peers && model < 0.0 && sector_gained_names {
                OrderDriver::NewPeers
            } else {
                dominant
            }
        }
    };
    out
}

fn exit_driver(plan: &PlanSnapshot, now: Option<&AssetRecord>) -> OrderDriver {
    let Some(a) = now else {
        return OrderDriver::NotInUniverse;
    };
    match a.status {
        AssetStatus::Excluded => {
            let not_in_universe = plan
                .excluded
                .iter()
                .any(|f| f.symbol == a.symbol && f.reason == "not_in_universe");
            if not_in_universe {
                OrderDriver::NotInUniverse
            } else {
                OrderDriver::Excluded
            }
        }
        AssetStatus::BelowMinWeight => OrderDriver::BelowMinWeight,
        AssetStatus::InsufficientHistory | AssetStatus::NoPrice => OrderDriver::LostEligibility,
        AssetStatus::Active if a.trend_multiplier <= 0.0 => OrderDriver::TrendExit,
        AssetStatus::Active => OrderDriver::LostEligibility,
        // Frozen names never trade; listed for completeness.
        AssetStatus::Frozen => OrderDriver::Excluded,
    }
}

fn entry_driver(before: Option<&AssetRecord>) -> OrderDriver {
    let Some(b) = before else {
        return OrderDriver::AddedToUniverse;
    };
    match b.status {
        AssetStatus::InsufficientHistory | AssetStatus::NoPrice => OrderDriver::NewlyEligible,
        AssetStatus::BelowMinWeight => OrderDriver::BackAboveMinWeight,
        AssetStatus::Excluded | AssetStatus::Frozen => OrderDriver::RestrictionLifted,
        AssetStatus::Active if b.trend_multiplier <= 0.0 => OrderDriver::TrendReentry,
        AssetStatus::Active => OrderDriver::BackAboveMinWeight,
    }
}

/// Splits ln(t1 / t0) into exposure, sector share, trend, momentum, volatility and the rest.
fn components(
    plan: &PlanSnapshot,
    prev: &PlanSnapshot,
    now: Option<&AssetRecord>,
    before: Option<&AssetRecord>,
    t0: f64,
    t1: f64,
) -> Vec<Component> {
    let total = (t1 / t0).ln();
    let model = t1 - t0;
    let mut logs: Vec<(OrderDriver, f64)> = Vec::new();
    let ln_ratio = |a: f64, b: f64| {
        if a > 0.0 && b > 0.0 {
            Some((a / b).ln())
        } else {
            None
        }
    };
    let e1 = plan.effective_exposure();
    let e0 = prev.effective_exposure();
    if let (Some(e1), Some(e0)) = (e1, e0)
        && let Some(x) = ln_ratio(e1, e0)
    {
        logs.push((OrderDriver::Exposure, x));
    }
    if let (Some(a1), Some(a0)) = (now, before) {
        let b1 = plan.sector_budgets.get(&a1.sector).copied();
        let b0 = prev.sector_budgets.get(&a0.sector).copied();
        if let (Some(b1), Some(b0), Some(e1), Some(e0)) = (b1, b0, e1, e0)
            && e1 > 0.0
            && e0 > 0.0
            && let Some(x) = ln_ratio(b1 / e1, b0 / e0)
        {
            logs.push((OrderDriver::SectorBudget, x));
        }
        if let Some(x) = ln_ratio(a1.trend_multiplier, a0.trend_multiplier) {
            logs.push((OrderDriver::Trend, x));
        }
        if let Some(x) = ln_ratio(a1.momentum_multiplier, a0.momentum_multiplier) {
            logs.push((OrderDriver::Momentum, x));
        }
        if let (Some(v1), Some(v0)) = (a1.vol, a0.vol)
            && let Some(x) = ln_ratio(v1.max(0.01), v0.max(0.01))
        {
            logs.push((OrderDriver::Volatility, -plan.params.asset.vol_power * x));
        }
    }
    let explained: f64 = logs.iter().map(|(_, x)| x).sum();
    logs.push((OrderDriver::Peers, total - explained));
    logs.into_iter()
        .map(|(driver, log)| Component {
            driver,
            log,
            weight: if total.abs() > EPS {
                log / total * model
            } else {
                0.0
            },
        })
        .collect()
}

/// Plan-level drivers in headline order: context first, then the drivers behind the most
/// traded weight, the caps and the band waiver, and restrictions last.
fn plan_drivers(
    plan: &PlanSnapshot,
    previous: Option<&PlanSnapshot>,
    x: &PlanExplanation,
) -> Vec<PlanDriver> {
    let mut context = Vec::new();
    let Some(prev) = previous else {
        context.push(PlanDriver::FirstPlan {
            eligible: plan.assets.iter().filter(|a| eligible(a.status)).count(),
            universe: x.universe,
        });
        if x.insufficient_history * 10 >= x.universe.max(1) && x.insufficient_history > 0 {
            context.push(PlanDriver::HistoryLimited {
                insufficient: x.insufficient_history,
                universe: x.universe,
            });
        }
        context.extend(caps(plan));
        context.extend(restrictions(x));
        return context;
    };
    if plan.header.strategy_version_id != prev.header.strategy_version_id {
        context.push(PlanDriver::StrategyChanged {
            from: prev.header.strategy_version_id,
            to: plan.header.strategy_version_id,
        });
    }
    if x.insufficient_history * 10 >= x.universe.max(1) && x.insufficient_history > 0 {
        context.push(PlanDriver::HistoryLimited {
            insufficient: x.insufficient_history,
            universe: x.universe,
        });
    }
    if let Some(no_action) = &x.no_action {
        context.push(PlanDriver::NoAction {
            reason: no_action.reason,
            gap: x.cash.gap,
        });
    }

    // Tally by plan-level driver.
    let sum = |pred: &dyn Fn(&OrderExplanation) -> bool| {
        let hits: Vec<&OrderExplanation> = x.orders.iter().filter(|o| pred(o)).collect();
        (
            hits.len(),
            hits.iter()
                .map(|o| (o.weight_target - o.weight_before).abs())
                .fold(0.0, |acc, w| acc + w),
        )
    };
    let trend_down = |o: &OrderExplanation| {
        o.driver == OrderDriver::TrendExit
            || (o.driver == OrderDriver::Trend
                && o.components
                    .iter()
                    .any(|c| c.driver == OrderDriver::Trend && c.log < 0.0))
    };
    let mut linked: Vec<PlanDriver> = Vec::new();

    let (orders, weight) = sum(&|o| {
        matches!(
            o.driver,
            OrderDriver::NewlyEligible | OrderDriver::AddedToUniverse | OrderDriver::NewPeers
        )
    });
    if !x.newly_eligible.is_empty() || orders > 0 {
        linked.push(PlanDriver::NewlyEligible {
            count: x.newly_eligible.len(),
            orders,
            weight,
        });
    }
    if let (Some(before), Some(now), Some(e0), Some(e1)) = (
        x.breadth.before,
        x.breadth.now,
        x.exposure.before,
        x.exposure.now,
    ) {
        let (orders, weight) = sum(&|o| o.driver == OrderDriver::Exposure);
        let crossed = x.breadth.crossed_below.len() + x.breadth.crossed_above.len();
        if crossed > 0 || (e1 - e0).abs() >= 0.001 || orders > 0 {
            linked.push(PlanDriver::Breadth {
                line: plan.params.exposure.breadth_sma,
                before,
                now,
                crossed_below: x.breadth.crossed_below.len(),
                crossed_above: x.breadth.crossed_above.len(),
                exposure_before: e0,
                exposure_now: e1,
                orders,
                weight,
            });
        }
    }
    let (orders, weight) = sum(&trend_down);
    if !x.trend_breaks.is_empty() || orders > 0 {
        linked.push(PlanDriver::TrendBreaks {
            count: x.trend_breaks.len(),
            orders,
            weight,
        });
    }
    let (orders, weight) = sum(&|o| {
        o.driver == OrderDriver::TrendReentry || (o.driver == OrderDriver::Trend && !trend_down(o))
    });
    if !x.trend_recoveries.is_empty() || orders > 0 {
        linked.push(PlanDriver::TrendRecoveries {
            count: x.trend_recoveries.len(),
            orders,
            weight,
        });
    }
    // The sector whose budget moved the most traded weight, else the largest share change.
    let mut by_sector: BTreeMap<&str, (usize, f64)> = BTreeMap::new();
    for o in x
        .orders
        .iter()
        .filter(|o| o.driver == OrderDriver::SectorBudget)
    {
        let entry = by_sector
            .entry(o.sector.as_deref().unwrap_or(""))
            .or_default();
        entry.0 += 1;
        entry.1 += (o.weight_target - o.weight_before).abs();
    }
    let top_sector = by_sector
        .iter()
        .max_by(|a, b| a.1.1.total_cmp(&b.1.1).then(b.0.cmp(a.0)))
        .map(|(s, t)| (s.to_string(), *t))
        .or_else(|| {
            x.sectors
                .first()
                .filter(|s| s.share_effect.is_some_and(|e| e.abs() >= 0.005))
                .map(|s| (s.sector.clone(), (0, 0.0)))
        });
    if let Some((sector, (orders, weight))) = top_sector
        && let Some(change) = x.sectors.iter().find(|s| s.sector == sector)
    {
        linked.push(PlanDriver::SectorBudget {
            sector,
            before: change.before.unwrap_or(0.0),
            now: change.now,
            orders,
            weight,
        });
    }
    let (orders, weight) = sum(&|o| o.driver == OrderDriver::PriceDrift);
    if orders > 0 {
        linked.push(PlanDriver::PriceDrift { orders, weight });
    }
    let (orders, weight) = sum(&|o| {
        matches!(
            o.driver,
            OrderDriver::CarryOverTurnover | OrderDriver::CarryOverCash
        )
    });
    if orders > 0 {
        linked.push(PlanDriver::CarryOver {
            previous_turnover_scale: prev.turnover_scale,
            previous_cash_scale: prev.cash_scale,
            orders,
            weight,
        });
    }
    let (orders, weight) = sum(&|o| o.driver == OrderDriver::Retry);
    if orders > 0 {
        linked.push(PlanDriver::Retry { orders, weight });
    }
    for driver in [
        OrderDriver::Momentum,
        OrderDriver::Volatility,
        OrderDriver::Peers,
        OrderDriver::BackAboveMinWeight,
        OrderDriver::BelowMinWeight,
        OrderDriver::RestrictionLifted,
        OrderDriver::Excluded,
        OrderDriver::NotInUniverse,
        OrderDriver::LostEligibility,
    ] {
        let (orders, weight) = sum(&|o| o.driver == driver);
        if orders > 0 {
            linked.push(PlanDriver::Orders {
                driver,
                orders,
                weight,
            });
        }
    }
    // Largest traded weight first; ties (e.g. no orders) keep the order above.
    linked.sort_by(|a, b| b.weight().total_cmp(&a.weight()));

    let waived = x.orders.iter().filter(|o| o.band_waived).count();
    let waiver = plan
        .cash
        .waiver
        .filter(|_| waived > 0)
        .map(|side| PlanDriver::BandWaiver {
            side,
            gap: plan.cash.gap,
            invested_now: plan.cash.invested_now,
            invested_target: plan.cash.invested_target,
            orders: waived,
        });
    // A waiver caused by the exposure moving the same way (breadth fell and the book is now too
    // invested, or the reverse) makes breadth the root cause of the broad trade: it leads.
    let breadth_caused_waiver = match (plan.cash.waiver, x.exposure.before, x.exposure.now) {
        (Some(BandWaiver::Sells), Some(e0), Some(e1)) => waived > 0 && e1 < e0 - 0.001,
        (Some(BandWaiver::Buys), Some(e0), Some(e1)) => waived > 0 && e1 > e0 + 0.001,
        _ => false,
    };
    if breadth_caused_waiver
        && let Some(k) = linked
            .iter()
            .position(|d| matches!(d, PlanDriver::Breadth { .. }))
    {
        let breadth = linked.remove(k);
        linked.insert(0, breadth);
    }

    let mut drivers = context;
    let mut linked = linked.into_iter();
    if let Some(top) = linked.next() {
        drivers.push(top);
    }
    drivers.extend(caps(plan));
    drivers.extend(waiver);
    drivers.extend(linked);
    drivers.extend(restrictions(x));
    drivers
}

fn caps(plan: &PlanSnapshot) -> Vec<PlanDriver> {
    let mut out = Vec::new();
    if let Some(scale) = plan.turnover_scale {
        out.push(PlanDriver::TurnoverCap {
            scale,
            limit: plan.params.rebalance.max_turnover,
        });
    }
    if let Some(scale) = plan.cash_scale {
        out.push(PlanDriver::CashCap { scale });
    }
    out
}

fn restrictions(x: &PlanExplanation) -> Vec<PlanDriver> {
    let mut out = Vec::new();
    if !x.frozen.is_empty() {
        out.push(PlanDriver::Frozen {
            count: x.frozen.len(),
        });
    }
    let excluded = x.excluded.len();
    if excluded > 0 {
        out.push(PlanDriver::Excluded { count: excluded });
    }
    out
}

/// The breadth (share of counted names above the line) at which the portfolio-level gap would
/// pass [`CASH_BAND`] and waive every band on one side, holding today's invested weight and the
/// rest of the allocation fixed. A forward-looking reading of the exposure rule
/// `min + (max − min) × breadth`; `None` without breadth scaling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreadthTriggers {
    /// Invested weight the triggers are measured against.
    pub invested: f64,
    /// A broad trim happens if breadth falls below this (None: not reachable).
    pub trim_below: Option<f64>,
    /// A broad top-up happens if breadth rises above this (None: not reachable).
    pub top_up_above: Option<f64>,
    /// The same thresholds as counts of the names counted now.
    pub counted: Option<usize>,
    pub trim_below_count: Option<usize>,
    pub top_up_above_count: Option<usize>,
}

/// See [`BreadthTriggers`]. `invested` is the portfolio's invested weight now; `invested_target`
/// and `exposure_target` come from the latest plan (their difference — frozen names, caps,
/// pruning — is assumed to stay as it is).
pub fn breadth_triggers(
    params: &StrategyParams,
    invested: f64,
    exposure_target: f64,
    invested_target: f64,
    counted: Option<usize>,
) -> Option<BreadthTriggers> {
    let ep = &params.exposure;
    let span = ep.max_exposure - ep.min_exposure;
    if !ep.breadth_scaling || span <= 0.0 {
        return None;
    }
    let slack = invested_target - exposure_target;
    // Target invested at breadth b: min + span·b + slack. Trim when it is below invested − band.
    let breadth_at = |target: f64| (target - slack - ep.min_exposure) / span;
    let trim = breadth_at(invested - CASH_BAND);
    let top_up = breadth_at(invested + CASH_BAND);
    let trim_below = (trim > 0.0)
        .then_some(trim.min(1.0))
        .filter(|_| trim <= 1.0);
    let top_up_above = (top_up < 1.0)
        .then_some(top_up.max(0.0))
        .filter(|_| top_up >= 0.0);
    // Counts: strictly fewer than ⌈n·b⌉ names above the line means breadth below b.
    let trim_below_count = counted.zip(trim_below).map(|(n, b)| {
        let k = (n as f64 * b).ceil() as usize;
        k.saturating_sub(1)
    });
    let top_up_above_count = counted
        .zip(top_up_above)
        .map(|(n, b)| (n as f64 * b).floor() as usize + 1);
    Some(BreadthTriggers {
        invested,
        trim_below,
        top_up_above,
        counted,
        trim_below_count,
        top_up_above_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn asset(symbol: &str, sector: &str, weight: f64) -> Value {
        json!({
            "symbol": symbol, "sector": sector, "status": "active", "price": 100.0,
            "vol": 0.3, "trend": 0.05, "momentum_multiplier": 1.0, "trend_multiplier": 1.0,
            "weight": weight, "above_breadth_line": true,
        })
    }

    struct Builder {
        header: PlanHeader,
        diag: Value,
        orders: Vec<OrderRecord>,
    }

    impl Builder {
        fn new(id: i64, assets: Vec<Value>, exposure: f64, breadth: f64) -> Self {
            let invested: f64 = assets.iter().map(|a| a["weight"].as_f64().unwrap()).sum();
            let mut sectors: BTreeMap<String, f64> = BTreeMap::new();
            for a in &assets {
                *sectors
                    .entry(a["sector"].as_str().unwrap().to_string())
                    .or_default() += a["weight"].as_f64().unwrap();
            }
            let sectors: Vec<Value> = sectors
                .into_iter()
                .map(|(s, b)| json!({"sector": s, "budget": b, "weight": b}))
                .collect();
            let quotes: serde_json::Map<String, Value> = assets
                .iter()
                .map(|a| {
                    (
                        a["symbol"].as_str().unwrap().to_string(),
                        json!({"price": a["price"]}),
                    )
                })
                .collect();
            Self {
                header: PlanHeader {
                    id,
                    status: "executed".into(),
                    strategy_version_id: Some(1),
                    nav: 1_000_000.0,
                    cash: 1_000_000.0 * (1.0 - invested),
                    exposure_target: Some(exposure),
                    invested_target: Some(invested),
                    breadth: Some(breadth),
                    est_vol: Some(0.2),
                },
                diag: json!({
                    "targets": {"assets": assets, "sectors": sectors, "vol_scale": null},
                    "skipped": [], "frozen": [], "excluded": [], "quotes": quotes,
                    "params": StrategyParams::default(),
                    "rebalance": {"turnover_scale": null, "cash_scale": null},
                }),
                orders: Vec::new(),
            }
        }

        fn nav_cash(mut self, invested_now: f64) -> Self {
            self.header.cash = self.header.nav * (1.0 - invested_now);
            self
        }

        fn order(
            mut self,
            symbol: &str,
            before: f64,
            target: f64,
            status: &str,
            fill: f64,
        ) -> Self {
            self.orders.push(OrderRecord {
                symbol: symbol.into(),
                side: if target > before {
                    Side::Buy
                } else {
                    Side::Sell
                },
                qty: 100.0,
                weight_before: before,
                weight_target: target,
                weight_after: target,
                status: status.into(),
                status_reason: (status == "rejected").then(|| "price_moved".into()),
                filled_qty: 100.0 * fill,
            });
            self
        }

        fn set(mut self, path: &[&str], value: Value) -> Self {
            let mut node = &mut self.diag;
            for key in &path[..path.len() - 1] {
                node = &mut node[*key];
            }
            node[path[path.len() - 1]] = value;
            self
        }

        fn build(self) -> PlanSnapshot {
            PlanSnapshot::from_stored(self.header, &self.diag, self.orders).unwrap()
        }
    }

    fn with(mut a: Value, key: &str, value: Value) -> Value {
        a[key] = value;
        a
    }

    #[test]
    fn a_first_plan_is_explained_as_such() {
        let plan = Builder::new(1, vec![asset("A", "s1", 0.05)], 0.9, 1.0)
            .nav_cash(0.0)
            .order("A", 0.0, 0.05, "filled", 1.0)
            .build();
        let x = explain(&plan, None);
        assert_eq!(x.previous_plan_id, None);
        assert!(matches!(x.drivers[0], PlanDriver::FirstPlan { .. }));
        assert_eq!(x.orders[0].driver, OrderDriver::FirstPlan);
    }

    #[test]
    fn falling_breadth_lists_crossings_and_attributes_cuts_to_exposure() {
        let prev = Builder::new(
            1,
            vec![asset("A", "s1", 0.04), asset("B", "s2", 0.04)],
            0.98,
            1.0,
        )
        .build();
        let plan = Builder::new(
            2,
            vec![
                asset("A", "s1", 0.03),
                with(
                    with(asset("B", "s2", 0.03), "above_breadth_line", json!(false)),
                    "trend",
                    json!(-0.01),
                ),
            ],
            0.79,
            0.5,
        )
        .nav_cash(0.08)
        .order("A", 0.04, 0.03, "planned", 0.0)
        .build();
        let x = explain(&plan, Some(&prev));
        assert_eq!(x.breadth.crossed_below, vec!["B".to_string()]);
        assert_eq!(x.breadth.above_now, Some(1));
        assert_eq!(x.breadth.counted_now, Some(2));
        let a = x.order("A").unwrap();
        assert_eq!(a.driver, OrderDriver::Exposure);
        // Model change −1pt; exposure explains ln(0.79/0.98) of ln(0.03/0.04).
        let exposure = a
            .components
            .iter()
            .find(|c| c.driver == OrderDriver::Exposure)
            .unwrap();
        assert!((exposure.log - (0.79f64 / 0.98).ln()).abs() < 1e-12);
        let total: f64 = a.components.iter().map(|c| c.weight).sum();
        assert!((total - (-0.01)).abs() < 1e-12);
        match &x.drivers[0] {
            PlanDriver::Breadth {
                crossed_below,
                orders,
                ..
            } => {
                assert_eq!(*crossed_below, 1);
                assert_eq!(*orders, 1);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn trend_cut_dominates_when_the_multiplier_falls() {
        // Z absorbs A's cut so the sector budget is unchanged.
        let prev = Builder::new(
            1,
            vec![asset("A", "s1", 0.04), asset("Z", "s1", 0.02)],
            0.9,
            1.0,
        )
        .build();
        let plan = Builder::new(
            2,
            vec![
                with(asset("A", "s1", 0.02), "trend_multiplier", json!(0.5)),
                asset("Z", "s1", 0.04),
            ],
            0.9,
            1.0,
        )
        .order("A", 0.04, 0.02, "planned", 0.0)
        .build();
        let x = explain(&plan, Some(&prev));
        let a = x.order("A").unwrap();
        assert_eq!(a.driver, OrderDriver::Trend);
        assert!(matches!(
            x.drivers[0],
            PlanDriver::TrendBreaks { count: 1, .. }
        ));
    }

    #[test]
    fn price_drift_dominates_when_larger_than_the_model_change() {
        let prev = Builder::new(1, vec![asset("A", "s1", 0.040)], 0.9, 1.0).build();
        let plan = Builder::new(
            2,
            vec![with(asset("A", "s1", 0.039), "price", json!(125.0))],
            0.9,
            1.0,
        )
        .order("A", 0.050, 0.039, "planned", 0.0)
        .build();
        let x = explain(&plan, Some(&prev));
        let a = x.order("A").unwrap();
        assert_eq!(a.driver, OrderDriver::PriceDrift);
        assert!((a.price_change.unwrap() - 0.25).abs() < 1e-12);
        assert!((a.drift.unwrap() - 0.01).abs() < 1e-12);
    }

    #[test]
    fn a_capped_previous_plan_is_carried_over_and_a_rejected_order_retried() {
        let prev = Builder::new(
            1,
            vec![asset("A", "s1", 0.05), asset("B", "s1", 0.05)],
            0.9,
            1.0,
        )
        .set(&["rebalance", "turnover_scale"], json!(0.5))
        .order("A", 0.0, 0.05, "filled", 1.0)
        .order("B", 0.0, 0.05, "rejected", 0.0)
        .build();
        // A's order went half way (weight_after 2.5%).
        let mut prev = prev;
        prev.orders[0].weight_after = 0.025;
        let plan = Builder::new(
            2,
            vec![asset("A", "s1", 0.05), asset("B", "s1", 0.05)],
            0.9,
            1.0,
        )
        .order("A", 0.025, 0.05, "planned", 0.0)
        .order("B", 0.0, 0.05, "planned", 0.0)
        .build();
        let x = explain(&plan, Some(&prev));
        assert_eq!(x.order("A").unwrap().driver, OrderDriver::CarryOverTurnover);
        let b = x.order("B").unwrap();
        assert_eq!(b.driver, OrderDriver::Retry);
        assert_eq!(b.previous_status_reason.as_deref(), Some("price_moved"));
    }

    #[test]
    fn entries_and_exits_follow_status_transitions() {
        let prev = Builder::new(
            1,
            vec![
                with(
                    asset("NEW", "s1", 0.0),
                    "status",
                    json!("insufficient_history"),
                ),
                asset("OLD", "s1", 0.01),
                asset("GONE", "s2", 0.02),
            ],
            0.9,
            1.0,
        )
        .build();
        let plan = Builder::new(
            2,
            vec![
                asset("NEW", "s1", 0.02),
                with(asset("OLD", "s1", 0.0), "status", json!("below_min_weight")),
                with(asset("GONE", "s2", 0.0), "status", json!("excluded")),
            ],
            0.9,
            1.0,
        )
        .set(
            &["excluded"],
            json!([{"symbol": "GONE", "reason": "not_in_universe"}]),
        )
        .order("NEW", 0.0, 0.02, "planned", 0.0)
        .order("OLD", 0.01, 0.0, "planned", 0.0)
        .order("GONE", 0.02, 0.0, "planned", 0.0)
        .build();
        let x = explain(&plan, Some(&prev));
        assert_eq!(x.order("NEW").unwrap().driver, OrderDriver::NewlyEligible);
        assert_eq!(x.order("OLD").unwrap().driver, OrderDriver::BelowMinWeight);
        assert_eq!(x.order("GONE").unwrap().driver, OrderDriver::NotInUniverse);
        assert_eq!(x.newly_eligible, vec!["NEW".to_string()]);
    }

    #[test]
    fn a_cut_to_make_room_for_newly_eligible_names_is_new_peers() {
        let prev = Builder::new(
            1,
            vec![
                asset("A", "s1", 0.05),
                with(
                    asset("N", "s1", 0.0),
                    "status",
                    json!("insufficient_history"),
                ),
            ],
            0.9,
            1.0,
        )
        .build();
        // Same sector budget, now shared with N.
        let plan = Builder::new(
            2,
            vec![asset("A", "s1", 0.025), asset("N", "s1", 0.025)],
            0.9,
            1.0,
        )
        .order("A", 0.05, 0.025, "planned", 0.0)
        .order("N", 0.0, 0.025, "planned", 0.0)
        .build();
        let x = explain(&plan, Some(&prev));
        assert_eq!(x.order("A").unwrap().driver, OrderDriver::NewPeers);
        assert_eq!(x.order("N").unwrap().driver, OrderDriver::NewlyEligible);
        assert!(matches!(
            x.drivers[0],
            PlanDriver::NewlyEligible {
                count: 1,
                orders: 2,
                ..
            }
        ));
    }

    #[test]
    fn the_band_waiver_is_derived_for_older_plans() {
        // 90% invested against an 80% target: bands are waived for sells, and a 0.2pt trim
        // inside a 1pt band is attributed to the waiver.
        let prev = Builder::new(1, vec![asset("A", "s1", 0.04)], 0.9, 1.0).build();
        let plan = Builder::new(2, vec![asset("A", "s1", 0.038)], 0.8, 0.5)
            .nav_cash(0.9)
            .order("A", 0.040, 0.038, "planned", 0.0)
            .build();
        assert!(plan.cash.derived);
        assert_eq!(plan.cash.waiver, Some(BandWaiver::Sells));
        let x = explain(&plan, Some(&prev));
        assert!(x.order("A").unwrap().band_waived);
        assert!(x.drivers.iter().any(|d| matches!(
            d,
            PlanDriver::BandWaiver {
                side: BandWaiver::Sells,
                orders: 1,
                ..
            }
        )));
    }

    #[test]
    fn breadth_is_reconstructed_from_the_trend_when_the_averages_match() {
        let mut a = asset("A", "s1", 0.04);
        a.as_object_mut().unwrap().remove("above_breadth_line");
        let mut b = with(asset("B", "s1", 0.02), "trend", json!(-0.02));
        b.as_object_mut().unwrap().remove("above_breadth_line");
        let plan = Builder::new(1, vec![a.clone(), b.clone()], 0.79, 0.5).build();
        assert_eq!(plan.breadth_source, BreadthSource::Reconstructed);
        assert_eq!(plan.breadth_counts(), Some((1, 2)));
        // With different averages it is not guessed.
        let mut params = StrategyParams::default();
        params.asset.trend_sma = 100;
        let plan = Builder::new(1, vec![a, b], 0.79, 0.5)
            .set(&["params"], serde_json::to_value(&params).unwrap())
            .build();
        assert_eq!(plan.breadth_source, BreadthSource::Unavailable);
        assert_eq!(plan.breadth_counts(), None);
    }

    #[test]
    fn a_no_action_plan_says_why_nothing_traded() {
        let prev = Builder::new(1, vec![asset("A", "s1", 0.04)], 0.9, 1.0).build();
        let plan = Builder::new(2, vec![asset("A", "s1", 0.041)], 0.88, 0.9)
            .nav_cash(0.04)
            .set(
                &["skipped"],
                json!([{"symbol": "A", "reason": "within_band", "weight_before": 0.04, "weight_target": 0.041}]),
            )
            .build();
        let x = explain(&plan, Some(&prev));
        let na = x.no_action.as_ref().unwrap();
        assert_eq!(na.reason, NoActionReason::WithinBands);
        let closest = na.closest.as_ref().unwrap();
        assert_eq!(closest.symbol, "A");
        // Band = max(0.25pt, 25% × 4.1%).
        assert!((closest.band - 0.01025).abs() < 1e-12);
        assert!(matches!(
            x.drivers[0],
            PlanDriver::NoAction {
                reason: NoActionReason::WithinBands,
                ..
            }
        ));
    }

    #[test]
    fn breadth_triggers_follow_the_exposure_rule() {
        let params = StrategyParams::default();
        // Invested 82.4%, exposure = invested target = 0.6 + 0.38 b.
        let t = breadth_triggers(&params, 0.824, 0.748, 0.748, Some(64)).unwrap();
        // Trim below b where 0.6 + 0.38 b = 0.774.
        assert!((t.trim_below.unwrap() - (0.774 - 0.6) / 0.38).abs() < 1e-12);
        assert!((t.top_up_above.unwrap() - (0.874 - 0.6) / 0.38).abs() < 1e-12);
        let trim = t.trim_below_count.unwrap();
        assert!((trim as f64) / 64.0 < t.trim_below.unwrap());
        assert!(((trim + 1) as f64) / 64.0 >= t.trim_below.unwrap());
        // Fully invested at the ceiling: no top-up is reachable.
        let t = breadth_triggers(&params, 0.98, 0.98, 0.98, None).unwrap();
        assert_eq!(t.top_up_above, None);
    }
}
