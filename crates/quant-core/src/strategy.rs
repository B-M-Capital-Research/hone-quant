//! Price-based portfolio construction over the fixed honeclaw universe.
//!
//! The engine follows the house allocation discipline from honeclaw's `hari-invest` frameworks:
//!
//! 1. **Market state sets total exposure** (framework 3/4): the share of the universe trading
//!    above its long moving average ("breadth") scales equity exposure between a floor and a
//!    ceiling; the remainder is held as cash, the stable end of the barbell.
//! 2. **Sector budget first** (framework 5): each sector gets a budget from its risk (inverse
//!    volatility of its equal-weight index), its relative momentum and the share of its members
//!    in an uptrend, bounded by per-sector floors and caps.
//! 3. **Then names within the sector**: inverse-volatility weights tilted by cross-sectional
//!    momentum and penalised below the trend line, bounded by a single-name cap.
//!
//! Only prices are used — no fundamentals, no stock selection. Every intermediate number is
//! returned in the diagnostics so a plan can always be explained after the fact.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::alloc::{capped_proportional, floored_capped_proportional};
use crate::stats;

// ---------------------------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectorMethod {
    /// Score = sector-index volatility ^ (−vol_power).
    Risk,
    /// Score = number of eligible members (with equal asset weights this is equal weight per name).
    MemberCount,
    /// Score = user-provided relative budget per sector.
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UniverseParams {
    /// Consecutive daily closes required before a name can receive weight.
    pub min_history_days: usize,
}

impl Default for UniverseParams {
    fn default() -> Self {
        Self {
            min_history_days: 126,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SectorParams {
    pub method: SectorMethod,
    /// Exponent on sector volatility for `risk` (1 = inverse vol, 2 = inverse variance).
    pub vol_power: f64,
    pub vol_lookback: usize,
    /// 0 disables; 0.5 means the strongest sector gets ×1.5 and the weakest ×0.5.
    pub momentum_tilt: f64,
    pub momentum_lookback: usize,
    pub momentum_skip: usize,
    /// Scale a sector's score by the average trend multiplier of its members.
    pub trend_aware: bool,
    /// Per-sector floor and cap, as fractions of the whole portfolio.
    pub min_weight: f64,
    pub max_weight: f64,
    /// Relative budgets for `custom`, keyed by sector id.
    pub custom_budgets: BTreeMap<String, f64>,
}

impl Default for SectorParams {
    fn default() -> Self {
        Self {
            method: SectorMethod::Risk,
            vol_power: 1.0,
            vol_lookback: 63,
            momentum_tilt: 0.3,
            momentum_lookback: 126,
            momentum_skip: 21,
            trend_aware: true,
            min_weight: 0.03,
            max_weight: 0.22,
            custom_budgets: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssetParams {
    /// Exponent on asset volatility (0 = equal weight, 1 = inverse vol, 2 = inverse variance).
    pub vol_power: f64,
    pub vol_lookback: usize,
    pub momentum_tilt: f64,
    pub momentum_lookback: usize,
    pub momentum_skip: usize,
    /// Moving-average length of the trend filter; 0 disables it.
    pub trend_sma: usize,
    /// Score multiplier when the price is well below its trend line (0 = exit, 1 = ignore trend).
    pub trend_penalty: f64,
    /// How far below the trend line the multiplier reaches `trend_penalty`: it fades linearly
    /// from 1 at the line (0 = a hard step at the line). A fade keeps names that hover around
    /// their average from flipping between full and penalised weight at every plan.
    pub trend_ramp: f64,
    /// Names whose final weight would fall below this are dropped and their budget reassigned.
    pub min_weight: f64,
    /// Single-name cap as a fraction of the whole portfolio.
    pub max_weight: f64,
}

impl Default for AssetParams {
    fn default() -> Self {
        Self {
            vol_power: 1.0,
            vol_lookback: 63,
            momentum_tilt: 0.4,
            momentum_lookback: 126,
            momentum_skip: 21,
            trend_sma: 200,
            trend_penalty: 0.5,
            trend_ramp: 0.1,
            min_weight: 0.005,
            max_weight: 0.05,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExposureParams {
    /// Equity exposure when every eligible name is above its trend line (the rest is cash).
    pub max_exposure: f64,
    /// Equity exposure when none is.
    pub min_exposure: f64,
    pub breadth_scaling: bool,
    pub breadth_sma: usize,
    /// Optional ex-ante annualised volatility ceiling; exposure is scaled down to respect it.
    pub target_vol: Option<f64>,
    pub vol_lookback: usize,
}

impl Default for ExposureParams {
    fn default() -> Self {
        Self {
            max_exposure: 0.98,
            min_exposure: 0.6,
            breadth_scaling: true,
            breadth_sma: 200,
            target_vol: None,
            vol_lookback: 63,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RebalanceParams {
    /// Absolute drift band in weight points (0.01 = one percentage point).
    pub band_abs: f64,
    /// Relative drift band as a fraction of the target weight; the larger band applies.
    pub band_rel: f64,
    /// Trades smaller than this (USD) are skipped, except full exits.
    pub min_trade_value: f64,
    /// One-way turnover ceiling per plan (Σ|Δw| / 2).
    pub max_turnover: f64,
    pub fractional_shares: bool,
}

impl Default for RebalanceParams {
    fn default() -> Self {
        Self {
            band_abs: 0.0025,
            band_rel: 0.25,
            min_trade_value: 1_000.0,
            max_turnover: 0.30,
            fractional_shares: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StrategyParams {
    pub universe: UniverseParams,
    pub sector: SectorParams,
    pub asset: AssetParams,
    pub exposure: ExposureParams,
    pub rebalance: RebalanceParams,
}

/// A structured validation problem; `path` is a dotted field path the UI can map to a control.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FieldError {
    pub path: String,
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    pub message: String,
}

impl FieldError {
    fn range(path: &str, min: f64, max: f64) -> Self {
        Self {
            path: path.to_string(),
            code: "out_of_range",
            min: Some(min),
            max: Some(max),
            message: format!("{path} must be between {min} and {max}"),
        }
    }

    fn rule(path: &str, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            path: path.to_string(),
            code,
            min: None,
            max: None,
            message: message.into(),
        }
    }
}

struct Checker(Vec<FieldError>);

impl Checker {
    fn f(&mut self, path: &str, value: f64, min: f64, max: f64) {
        if !(value.is_finite() && value >= min && value <= max) {
            self.0.push(FieldError::range(path, min, max));
        }
    }

    fn u(&mut self, path: &str, value: usize, min: usize, max: usize) {
        if value < min || value > max {
            self.0.push(FieldError::range(path, min as f64, max as f64));
        }
    }
}

impl StrategyParams {
    /// Longest history window any signal needs (in observations, excluding the current price).
    pub fn max_lookback(&self) -> usize {
        let mut lookbacks = vec![
            self.universe.min_history_days,
            self.sector.vol_lookback + 1,
            self.sector.momentum_lookback + 1,
            self.asset.vol_lookback + 1,
            self.asset.momentum_lookback + 1,
            self.asset.trend_sma,
            self.exposure.vol_lookback + 1,
        ];
        if self.exposure.breadth_scaling {
            lookbacks.push(self.exposure.breadth_sma);
        }
        lookbacks.into_iter().max().unwrap_or(1)
    }

    pub fn validate(&self) -> Result<(), Vec<FieldError>> {
        let mut c = Checker(Vec::new());
        c.u(
            "universe.min_history_days",
            self.universe.min_history_days,
            20,
            756,
        );

        let s = &self.sector;
        c.f("sector.vol_power", s.vol_power, 0.0, 3.0);
        c.u("sector.vol_lookback", s.vol_lookback, 10, 504);
        c.f("sector.momentum_tilt", s.momentum_tilt, 0.0, 0.95);
        c.u("sector.momentum_lookback", s.momentum_lookback, 10, 504);
        c.u("sector.momentum_skip", s.momentum_skip, 0, 63);
        c.f("sector.min_weight", s.min_weight, 0.0, 0.2);
        c.f("sector.max_weight", s.max_weight, 0.02, 1.0);
        if s.momentum_skip >= s.momentum_lookback {
            c.0.push(FieldError::rule(
                "sector.momentum_skip",
                "skip_not_below_lookback",
                "sector.momentum_skip must be shorter than sector.momentum_lookback",
            ));
        }
        if s.min_weight > s.max_weight {
            c.0.push(FieldError::rule(
                "sector.min_weight",
                "min_above_max",
                "sector.min_weight cannot exceed sector.max_weight",
            ));
        }
        if s.method == SectorMethod::Custom {
            if s.custom_budgets.is_empty() {
                c.0.push(FieldError::rule(
                    "sector.custom_budgets",
                    "required",
                    "custom sector budgets are required when sector.method is custom",
                ));
            }
            for (sector, budget) in &s.custom_budgets {
                c.f(
                    &format!("sector.custom_budgets.{sector}"),
                    *budget,
                    0.0,
                    100.0,
                );
            }
        }

        let a = &self.asset;
        c.f("asset.vol_power", a.vol_power, 0.0, 3.0);
        c.u("asset.vol_lookback", a.vol_lookback, 10, 504);
        c.f("asset.momentum_tilt", a.momentum_tilt, 0.0, 0.95);
        c.u("asset.momentum_lookback", a.momentum_lookback, 10, 504);
        c.u("asset.momentum_skip", a.momentum_skip, 0, 63);
        if a.trend_sma != 0 {
            c.u("asset.trend_sma", a.trend_sma, 10, 400);
        }
        c.f("asset.trend_penalty", a.trend_penalty, 0.0, 1.0);
        c.f("asset.trend_ramp", a.trend_ramp, 0.0, 0.5);
        c.f("asset.min_weight", a.min_weight, 0.0, 0.05);
        c.f("asset.max_weight", a.max_weight, 0.005, 0.5);
        if a.momentum_skip >= a.momentum_lookback {
            c.0.push(FieldError::rule(
                "asset.momentum_skip",
                "skip_not_below_lookback",
                "asset.momentum_skip must be shorter than asset.momentum_lookback",
            ));
        }
        if a.min_weight > a.max_weight {
            c.0.push(FieldError::rule(
                "asset.min_weight",
                "min_above_max",
                "asset.min_weight cannot exceed asset.max_weight",
            ));
        }

        let e = &self.exposure;
        c.f("exposure.max_exposure", e.max_exposure, 0.0, 1.0);
        c.f("exposure.min_exposure", e.min_exposure, 0.0, 1.0);
        c.u("exposure.breadth_sma", e.breadth_sma, 10, 400);
        c.u("exposure.vol_lookback", e.vol_lookback, 10, 504);
        if let Some(target) = e.target_vol {
            c.f("exposure.target_vol", target, 0.03, 1.5);
        }
        if e.min_exposure > e.max_exposure {
            c.0.push(FieldError::rule(
                "exposure.min_exposure",
                "min_above_max",
                "exposure.min_exposure cannot exceed exposure.max_exposure",
            ));
        }

        let r = &self.rebalance;
        c.f("rebalance.band_abs", r.band_abs, 0.0, 0.2);
        c.f("rebalance.band_rel", r.band_rel, 0.0, 1.0);
        c.f(
            "rebalance.min_trade_value",
            r.min_trade_value,
            0.0,
            1_000_000.0,
        );
        c.f("rebalance.max_turnover", r.max_turnover, 0.01, 2.0);

        if c.0.is_empty() { Ok(()) } else { Err(c.0) }
    }

    /// Parses user input, rejecting unknown fields (typos) as well as out-of-range values.
    pub fn from_json_strict(value: &Value) -> Result<Self, Vec<FieldError>> {
        let params: StrategyParams = serde_json::from_value(value.clone())
            .map_err(|error| vec![FieldError::rule("", "invalid_json", error.to_string())])?;
        let canonical = serde_json::to_value(&params).expect("params serialize");
        let mut errors = Vec::new();
        collect_unknown_fields(value, &canonical, "", &mut errors);
        if let Err(mut problems) = params.validate() {
            errors.append(&mut problems);
        }
        if errors.is_empty() {
            Ok(params)
        } else {
            Err(errors)
        }
    }
}

fn collect_unknown_fields(
    input: &Value,
    canonical: &Value,
    prefix: &str,
    out: &mut Vec<FieldError>,
) {
    let (Value::Object(input), Value::Object(canonical)) = (input, canonical) else {
        return;
    };
    for (key, value) in input {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match canonical.get(key) {
            None => out.push(FieldError::rule(
                &path,
                "unknown_field",
                format!("{path} is not a strategy parameter"),
            )),
            // `custom_budgets` is a free-form map keyed by sector id.
            Some(inner) if key != "custom_budgets" => {
                collect_unknown_fields(value, inner, &path, out)
            }
            Some(_) => {}
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Preset {
    pub id: &'static str,
    pub name_zh: &'static str,
    pub name_en: &'static str,
    pub summary_zh: &'static str,
    pub summary_en: &'static str,
    pub params: StrategyParams,
}

pub const DEFAULT_PRESET: &str = "sector_risk_budget";

pub fn presets() -> Vec<Preset> {
    vec![
        Preset {
            id: "sector_risk_budget",
            name_zh: "板块优先 · 风险预算",
            name_en: "Sector-first risk budget",
            summary_zh: "先按板块风险、动量与趋势定板块预算，再在板块内按波动率倒数配置并做动量倾斜；市场宽度决定总仓位，其余留现金。",
            summary_en: "Sector budgets from risk, momentum and trend; inverse-volatility names with a momentum tilt inside each sector; market breadth sets total exposure and the rest stays in cash.",
            params: StrategyParams::default(),
        },
        Preset {
            id: "equal_weight",
            name_zh: "等权基准",
            name_en: "Equal-weight baseline",
            summary_zh: "每家公司同等权重、几乎满仓，只做漂移再平衡。用来衡量其它策略是否真的创造了价值。",
            summary_en: "Every company at the same weight, nearly fully invested, rebalanced only on drift. The yardstick for whether other strategies add value.",
            params: StrategyParams {
                sector: SectorParams {
                    method: SectorMethod::MemberCount,
                    momentum_tilt: 0.0,
                    trend_aware: false,
                    min_weight: 0.0,
                    max_weight: 1.0,
                    ..SectorParams::default()
                },
                asset: AssetParams {
                    vol_power: 0.0,
                    momentum_tilt: 0.0,
                    trend_sma: 0,
                    trend_penalty: 1.0,
                    min_weight: 0.0,
                    max_weight: 0.1,
                    ..AssetParams::default()
                },
                exposure: ExposureParams {
                    max_exposure: 0.98,
                    min_exposure: 0.98,
                    breadth_scaling: false,
                    ..ExposureParams::default()
                },
                ..StrategyParams::default()
            },
        },
        Preset {
            id: "momentum_rotation",
            name_zh: "动量轮动",
            name_en: "Momentum rotation",
            summary_zh: "强势板块与公司拿更多预算，跌破长期均线的公司逐步减仓直至清仓；仓位随市场宽度大幅升降，换手更高。",
            summary_en: "Strong sectors and names take more of the budget, and names below their long trend line are scaled out to zero; exposure swings with breadth and turnover is higher.",
            params: StrategyParams {
                sector: SectorParams {
                    method: SectorMethod::MemberCount,
                    // Above ~0.6, swaps between neighbouring sector ranks move whole budgets
                    // back and forth from plan to plan (35x annual turnover at 0.9 in tests).
                    momentum_tilt: 0.5,
                    min_weight: 0.0,
                    max_weight: 0.25,
                    ..SectorParams::default()
                },
                asset: AssetParams {
                    momentum_tilt: 0.8,
                    trend_penalty: 0.0,
                    min_weight: 0.01,
                    max_weight: 0.08,
                    ..AssetParams::default()
                },
                exposure: ExposureParams {
                    min_exposure: 0.4,
                    ..ExposureParams::default()
                },
                rebalance: RebalanceParams {
                    band_abs: 0.004,
                    band_rel: 0.3,
                    max_turnover: 0.4,
                    ..RebalanceParams::default()
                },
                ..StrategyParams::default()
            },
        },
        Preset {
            id: "defensive_low_vol",
            name_zh: "低波防御",
            name_en: "Defensive low volatility",
            summary_zh: "按波动率平方倒数配置，压低高 Beta 板块；组合预估波动率上限 20%，弱市时仓位可降到四成。",
            summary_en: "Inverse-variance weights that lean away from high-beta sectors, a 20% ex-ante volatility ceiling, and exposure that can fall to 40% in weak markets.",
            params: StrategyParams {
                sector: SectorParams {
                    vol_power: 2.0,
                    momentum_tilt: 0.2,
                    max_weight: 0.2,
                    ..SectorParams::default()
                },
                asset: AssetParams {
                    vol_power: 2.0,
                    momentum_tilt: 0.2,
                    max_weight: 0.05,
                    ..AssetParams::default()
                },
                exposure: ExposureParams {
                    max_exposure: 0.9,
                    min_exposure: 0.4,
                    target_vol: Some(0.20),
                    ..ExposureParams::default()
                },
                rebalance: RebalanceParams {
                    max_turnover: 0.25,
                    ..RebalanceParams::default()
                },
                ..StrategyParams::default()
            },
        },
    ]
}

/// A version's name in Chinese and English. Versions still named after their preset (in either
/// language) show the preset's name in each language; any other name is the operator's own.
pub fn version_display_names(name: &str, preset_id: &str) -> (String, String) {
    match preset(preset_id) {
        Some(p) if name == p.name_en || name == p.name_zh => {
            (p.name_zh.to_string(), p.name_en.to_string())
        }
        _ => (name.to_string(), name.to_string()),
    }
}

pub fn preset(id: &str) -> Option<Preset> {
    presets().into_iter().find(|preset| preset.id == id)
}

// ---------------------------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetMeta {
    pub symbol: String,
    /// Primary sector id; an asset belongs to exactly one sector for budgeting.
    pub sector: String,
}

/// Everything the engine needs at one decision instant.
pub struct EngineInput<'a> {
    pub assets: &'a [AssetMeta],
    /// Adjusted daily closes per asset (aligned, oldest → newest), excluding the decision price.
    pub history: &'a [&'a [f64]],
    /// Decision price per asset (live quote, or the session open/close in a backtest).
    pub current: &'a [f64],
    /// Current portfolio weights (needed to hold frozen names in place).
    pub current_weights: &'a [f64],
    /// Names the operator excluded: target weight zero.
    pub excluded: &'a [bool],
    /// Names the operator locked (or with suspicious data): keep the current weight, never trade.
    pub frozen: &'a [bool],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetStatus {
    Active,
    Excluded,
    Frozen,
    NoPrice,
    InsufficientHistory,
    BelowMinWeight,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetDiagnostics {
    pub symbol: String,
    pub sector: String,
    pub status: AssetStatus,
    pub price: Option<f64>,
    pub vol: Option<f64>,
    pub momentum: Option<f64>,
    pub momentum_rank: Option<f64>,
    /// price / SMA(trend) − 1.
    pub trend: Option<f64>,
    pub momentum_multiplier: f64,
    pub trend_multiplier: f64,
    pub score: f64,
    pub capped: bool,
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SectorDiagnostics {
    pub sector: String,
    pub active_members: usize,
    pub vol: Option<f64>,
    pub momentum: Option<f64>,
    pub momentum_rank: Option<f64>,
    pub trend_factor: f64,
    pub score: f64,
    pub cap: f64,
    pub budget: f64,
    /// Final weight after name-level pruning and volatility targeting (including frozen names).
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetResult {
    pub weights: Vec<f64>,
    /// Exposure implied by market breadth, before volatility targeting.
    pub exposure_target: f64,
    /// Sum of the final weights.
    pub invested: f64,
    pub cash_weight: f64,
    pub breadth: Option<f64>,
    pub est_vol: Option<f64>,
    pub vol_scale: Option<f64>,
    pub sectors: Vec<SectorDiagnostics>,
    pub assets: Vec<AssetDiagnostics>,
}

/// The last `len` returns of an equal-weight index of `members`, using only members with a
/// complete window. `None` when no member has one.
fn ew_index_returns(series: &[&[f64]], len: usize) -> Option<Vec<f64>> {
    let windows: Vec<&[f64]> = series
        .iter()
        .filter_map(|s| stats::tail(s, len + 1))
        .collect();
    if windows.is_empty() {
        return None;
    }
    let mut out = vec![0.0; len];
    for window in &windows {
        for (t, slot) in out.iter_mut().enumerate() {
            *slot += window[t + 1] / window[t] - 1.0;
        }
    }
    let n = windows.len() as f64;
    out.iter_mut().for_each(|r| *r /= n);
    Some(out)
}

/// Share of `asset.min_weight` below which a position that is already held is dropped.
pub const HELD_MIN_WEIGHT_FRACTION: f64 = 0.5;

/// Multiplier for a price `trend` (price / average − 1, negative) below its trend line: 1 at the
/// line, falling linearly to `penalty` at `ramp` below it (a step when `ramp` is 0).
fn trend_fade(trend: f64, penalty: f64, ramp: f64) -> f64 {
    if ramp <= 0.0 {
        return penalty;
    }
    let depth = (-trend / ramp).clamp(0.0, 1.0);
    1.0 - (1.0 - penalty) * depth
}

fn tilt(rank: Option<f64>, strength: f64) -> f64 {
    match rank {
        Some(pct) => (1.0 + strength * (2.0 * pct - 1.0)).max(0.0),
        None => 1.0,
    }
}

/// Computes target weights for one decision instant.
pub fn compute_targets(params: &StrategyParams, input: &EngineInput) -> TargetResult {
    let n = input.assets.len();
    assert!(
        input.history.len() == n
            && input.current.len() == n
            && input.current_weights.len() == n
            && input.excluded.len() == n
            && input.frozen.len() == n,
        "engine inputs must align with assets"
    );
    let ap = &params.asset;
    let sp = &params.sector;
    let ep = &params.exposure;

    // Short windows: the last `max_lookback` closes plus the decision price.
    let keep = params.max_lookback();
    let series: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            let history = input.history[i];
            let start = history.len().saturating_sub(keep);
            let mut window = Vec::with_capacity(history.len() - start + 1);
            window.extend_from_slice(&history[start..]);
            window.push(input.current[i]);
            window
        })
        .collect();

    let required_history = params.universe.min_history_days.max(ap.vol_lookback + 1);
    let mut status = vec![AssetStatus::Active; n];
    for i in 0..n {
        let price_ok = input.current[i].is_finite() && input.current[i] > 0.0;
        status[i] = if input.frozen[i] {
            AssetStatus::Frozen
        } else if input.excluded[i] {
            AssetStatus::Excluded
        } else if !price_ok {
            AssetStatus::NoPrice
        } else if stats::tail(input.history[i], required_history).is_none() {
            AssetStatus::InsufficientHistory
        } else {
            AssetStatus::Active
        };
    }

    // Signals for active names.
    let mut vol = vec![None; n];
    let mut momentum = vec![None; n];
    let mut trend = vec![None; n];
    let mut above_breadth_line = Vec::new();
    for i in 0..n {
        if status[i] != AssetStatus::Active {
            continue;
        }
        let s = &series[i];
        let price = input.current[i];
        vol[i] = stats::annualized_vol(s, ap.vol_lookback);
        momentum[i] = stats::momentum(s, ap.momentum_lookback, ap.momentum_skip);
        if ap.trend_sma > 0 {
            trend[i] = stats::sma(s, ap.trend_sma).map(|ma| price / ma - 1.0);
        }
        if ep.breadth_scaling
            && let Some(ma) = stats::sma(s, ep.breadth_sma)
        {
            above_breadth_line.push(if price > ma { 1.0 } else { 0.0 });
        }
    }

    // Market state → exposure.
    let breadth = stats::mean(&above_breadth_line);
    let exposure_target = match (ep.breadth_scaling, breadth) {
        (true, Some(b)) => ep.min_exposure + (ep.max_exposure - ep.min_exposure) * b,
        _ => ep.max_exposure,
    };

    // Frozen names (and held names without a price) keep their current weight.
    let mut weights = vec![0.0; n];
    let mut reserved = 0.0;
    for i in 0..n {
        let held = input.current_weights[i].max(0.0);
        let hold_in_place =
            status[i] == AssetStatus::Frozen || (status[i] == AssetStatus::NoPrice && held > 0.0);
        if hold_in_place {
            weights[i] = held;
            reserved += held;
        }
    }
    let available = (exposure_target - reserved).max(0.0);

    // Name-level multipliers (cross-sectional momentum ranks over the active universe).
    let momentum_ranks = stats::percentile_ranks(&momentum);
    let mut momentum_multiplier = vec![1.0; n];
    let mut trend_multiplier = vec![1.0; n];
    let mut score = vec![0.0; n];
    for i in 0..n {
        if status[i] != AssetStatus::Active {
            continue;
        }
        momentum_multiplier[i] = tilt(momentum_ranks[i], ap.momentum_tilt);
        trend_multiplier[i] = match trend[i] {
            Some(t) if t < 0.0 => trend_fade(t, ap.trend_penalty, ap.trend_ramp),
            _ => 1.0,
        };
        let base = match vol[i] {
            Some(v) => v.max(0.01).powf(-ap.vol_power),
            None => 0.0,
        };
        score[i] = base * momentum_multiplier[i] * trend_multiplier[i];
    }

    // Sector order follows first appearance so output is stable.
    let mut sector_ids: Vec<String> = Vec::new();
    for asset in input.assets {
        if !sector_ids.contains(&asset.sector) {
            sector_ids.push(asset.sector.clone());
        }
    }

    let mut capped = vec![false; n];
    let mut sector_diag: Vec<SectorDiagnostics>;
    // Prune names that would end up below the minimum weight, then re-allocate.
    let mut iteration = 0;
    loop {
        iteration += 1;
        sector_diag = Vec::with_capacity(sector_ids.len());
        let mut sector_scores = Vec::with_capacity(sector_ids.len());
        let mut sector_caps = Vec::with_capacity(sector_ids.len());
        let mut sector_momentum = Vec::with_capacity(sector_ids.len());
        let mut members_by_sector: Vec<Vec<usize>> = Vec::with_capacity(sector_ids.len());
        for sector in &sector_ids {
            let members: Vec<usize> = (0..n)
                .filter(|&i| {
                    &input.assets[i].sector == sector
                        && status[i] == AssetStatus::Active
                        && score[i] > 0.0
                })
                .collect();
            let windows: Vec<&[f64]> = members.iter().map(|&i| series[i].as_slice()).collect();
            let index_vol = ew_index_returns(&windows, sp.vol_lookback)
                .and_then(|r| stats::stdev(&r))
                .map(|s| s * stats::TRADING_DAYS_PER_YEAR.sqrt());
            let index_momentum = ew_index_returns(&windows, sp.momentum_lookback).map(|r| {
                let keep = r.len() - sp.momentum_skip;
                r[..keep].iter().fold(1.0, |acc, x| acc * (1.0 + x)) - 1.0
            });
            let trend_factor = if sp.trend_aware && !members.is_empty() {
                members.iter().map(|&i| trend_multiplier[i]).sum::<f64>() / members.len() as f64
            } else {
                1.0
            };
            let base = if members.is_empty() {
                0.0
            } else {
                match sp.method {
                    SectorMethod::Risk => match index_vol {
                        Some(v) => v.max(0.01).powf(-sp.vol_power),
                        // Fall back to the members' average volatility.
                        None => {
                            let vols: Vec<f64> = members.iter().filter_map(|&i| vol[i]).collect();
                            stats::mean(&vols)
                                .map(|v| v.max(0.01).powf(-sp.vol_power))
                                .unwrap_or(0.0)
                        }
                    },
                    SectorMethod::MemberCount => members.len() as f64,
                    SectorMethod::Custom => sp.custom_budgets.get(sector).copied().unwrap_or(0.0),
                }
            };
            let cap = sp
                .max_weight
                .min(members.len() as f64 * ap.max_weight)
                .min(available);
            sector_momentum.push(if members.is_empty() {
                None
            } else {
                index_momentum
            });
            sector_scores.push(base * trend_factor);
            sector_caps.push(cap);
            sector_diag.push(SectorDiagnostics {
                sector: sector.clone(),
                active_members: members.len(),
                vol: index_vol,
                momentum: index_momentum,
                momentum_rank: None,
                trend_factor,
                score: 0.0,
                cap,
                budget: 0.0,
                weight: 0.0,
            });
            members_by_sector.push(members);
        }
        let sector_ranks = stats::percentile_ranks(&sector_momentum);
        for (k, diag) in sector_diag.iter_mut().enumerate() {
            diag.momentum_rank = sector_ranks[k];
            sector_scores[k] *= tilt(sector_ranks[k], sp.momentum_tilt);
            diag.score = sector_scores[k];
        }
        let budgets =
            floored_capped_proportional(&sector_scores, &sector_caps, sp.min_weight, available);

        for i in 0..n {
            if status[i] == AssetStatus::Active {
                weights[i] = 0.0;
                capped[i] = false;
            }
        }
        for (k, members) in members_by_sector.iter().enumerate() {
            sector_diag[k].budget = budgets[k];
            if members.is_empty() || budgets[k] <= 0.0 {
                continue;
            }
            let member_scores: Vec<f64> = members.iter().map(|&i| score[i]).collect();
            let caps = vec![ap.max_weight; members.len()];
            let allocation = capped_proportional(&member_scores, &caps, budgets[k]);
            for (j, &i) in members.iter().enumerate() {
                weights[i] = allocation[j];
                capped[i] = allocation[j] >= ap.max_weight - 1e-12;
            }
        }

        // Hysteresis: a new position needs the full minimum weight, but one already held is kept
        // down to half of it, so a name hovering around the minimum is not sold and bought back
        // plan after plan.
        let threshold = |i: usize| {
            if input.current_weights[i] > 0.0 {
                ap.min_weight * HELD_MIN_WEIGHT_FRACTION
            } else {
                ap.min_weight
            }
        };
        let too_small: Vec<usize> = (0..n)
            .filter(|&i| {
                status[i] == AssetStatus::Active && weights[i] > 0.0 && weights[i] < threshold(i)
            })
            .collect();
        if too_small.is_empty() || iteration >= 12 {
            break;
        }
        for i in too_small {
            status[i] = AssetStatus::BelowMinWeight;
            weights[i] = 0.0;
        }
    }

    // Optional ex-ante volatility ceiling (scales only the freely allocated names).
    let mut est_vol = estimate_portfolio_vol(&series, &weights, ep.vol_lookback);
    let mut vol_scale = None;
    if let (Some(target), Some(estimate)) = (ep.target_vol, est_vol)
        && estimate > target
        && estimate > 0.0
    {
        let scale = target / estimate;
        for i in 0..n {
            if status[i] == AssetStatus::Active {
                weights[i] *= scale;
            }
        }
        vol_scale = Some(scale);
        est_vol = estimate_portfolio_vol(&series, &weights, ep.vol_lookback);
    }

    for diag in sector_diag.iter_mut() {
        diag.weight = (0..n)
            .filter(|&i| input.assets[i].sector == diag.sector)
            .map(|i| weights[i])
            .sum();
    }
    let invested: f64 = weights.iter().sum();
    let assets = (0..n)
        .map(|i| AssetDiagnostics {
            symbol: input.assets[i].symbol.clone(),
            sector: input.assets[i].sector.clone(),
            status: status[i],
            price: (input.current[i].is_finite() && input.current[i] > 0.0)
                .then_some(input.current[i]),
            vol: vol[i],
            momentum: momentum[i],
            momentum_rank: momentum_ranks[i],
            trend: trend[i],
            momentum_multiplier: momentum_multiplier[i],
            trend_multiplier: trend_multiplier[i],
            score: score[i],
            capped: capped[i],
            weight: weights[i],
        })
        .collect();

    TargetResult {
        weights,
        exposure_target,
        invested,
        cash_weight: (1.0 - invested).max(0.0),
        breadth,
        est_vol,
        vol_scale,
        sectors: sector_diag,
        assets,
    }
}

/// Ex-ante annualised volatility of a weight vector from the sample covariance of daily log
/// returns. Names without a complete window are left out of the estimate.
pub fn estimate_portfolio_vol(
    series: &[Vec<f64>],
    weights: &[f64],
    lookback: usize,
) -> Option<f64> {
    let held: Vec<(f64, Vec<f64>)> = weights
        .iter()
        .zip(series)
        .filter(|(w, _)| **w > 0.0)
        .filter_map(|(w, s)| stats::tail(s, lookback + 1).map(|win| (*w, stats::log_returns(win))))
        .collect();
    if held.is_empty() {
        return None;
    }
    let mut variance = 0.0;
    for i in 0..held.len() {
        for j in i..held.len() {
            let cov = stats::covariance(&held[i].1, &held[j].1)?;
            let factor = if i == j { 1.0 } else { 2.0 };
            variance += factor * held[i].0 * held[j].0 * cov;
        }
    }
    Some((variance.max(0.0) * stats::TRADING_DAYS_PER_YEAR).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic price path: geometric drift plus a periodic wiggle of a given amplitude.
    fn path(len: usize, drift: f64, wiggle: f64, phase: f64) -> Vec<f64> {
        (0..len)
            .map(|t| {
                let t = t as f64;
                100.0 * (drift * t).exp() * (1.0 + wiggle * (t * 0.7 + phase).sin())
            })
            .collect()
    }

    struct Fixture {
        assets: Vec<AssetMeta>,
        history: Vec<Vec<f64>>,
        current: Vec<f64>,
    }

    impl Fixture {
        fn new(specs: &[(&str, &str, f64, f64)]) -> Self {
            let len = 400;
            let mut assets = Vec::new();
            let mut history = Vec::new();
            let mut current = Vec::new();
            for (k, (symbol, sector, drift, wiggle)) in specs.iter().enumerate() {
                let p = path(len + 1, *drift, *wiggle, k as f64);
                assets.push(AssetMeta {
                    symbol: symbol.to_string(),
                    sector: sector.to_string(),
                });
                current.push(p[len]);
                history.push(p[..len].to_vec());
            }
            Self {
                assets,
                history,
                current,
            }
        }

        fn run(&self, params: &StrategyParams) -> TargetResult {
            self.run_with(params, &[], &[], &[])
        }

        fn run_with(
            &self,
            params: &StrategyParams,
            excluded: &[&str],
            frozen: &[&str],
            weights: &[f64],
        ) -> TargetResult {
            let n = self.assets.len();
            let history: Vec<&[f64]> = self.history.iter().map(|h| h.as_slice()).collect();
            let excluded: Vec<bool> = self
                .assets
                .iter()
                .map(|a| excluded.contains(&a.symbol.as_str()))
                .collect();
            let frozen: Vec<bool> = self
                .assets
                .iter()
                .map(|a| frozen.contains(&a.symbol.as_str()))
                .collect();
            let current_weights = if weights.is_empty() {
                vec![0.0; n]
            } else {
                weights.to_vec()
            };
            compute_targets(
                params,
                &EngineInput {
                    assets: &self.assets,
                    history: &history,
                    current: &self.current,
                    current_weights: &current_weights,
                    excluded: &excluded,
                    frozen: &frozen,
                },
            )
        }
    }

    fn universe() -> Fixture {
        Fixture::new(&[
            ("NVDA", "ai-chip", 0.0015, 0.02),
            ("AMD", "ai-chip", 0.0010, 0.03),
            ("AVGO", "ai-chip", 0.0012, 0.015),
            ("MU", "storage", 0.0008, 0.03),
            ("WDC", "storage", -0.0005, 0.025),
            ("VST", "power", 0.0006, 0.01),
            ("CEG", "power", 0.0004, 0.012),
            ("RKLB", "space", 0.0020, 0.06),
        ])
    }

    fn sum(xs: &[f64]) -> f64 {
        xs.iter().sum()
    }

    /// Defaults with a single-name cap that suits the eight-name fixture: at the production
    /// 5% cap the fixture could hold at most 40%, so every name would sit at its cap.
    fn fixture_params() -> StrategyParams {
        let mut params = StrategyParams::default();
        params.asset.max_weight = 0.2;
        params
    }

    #[test]
    fn default_params_validate_and_presets_validate() {
        assert!(StrategyParams::default().validate().is_ok());
        for preset in presets() {
            assert!(
                preset.params.validate().is_ok(),
                "preset {} invalid",
                preset.id
            );
        }
        assert_eq!(presets()[0].id, DEFAULT_PRESET);
    }

    #[test]
    fn strict_parsing_rejects_unknown_fields_and_bad_ranges() {
        let input = serde_json::json!({
            "asset": { "max_weight": 0.9, "max_wieght": 0.1 },
            "exposure": { "min_exposure": 0.9, "max_exposure": 0.5 },
            "colour": "blue"
        });
        let errors = StrategyParams::from_json_strict(&input).unwrap_err();
        let paths: Vec<&str> = errors.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"asset.max_wieght"));
        assert!(paths.contains(&"colour"));
        assert!(paths.contains(&"asset.max_weight"));
        assert!(paths.contains(&"exposure.min_exposure"));
    }

    #[test]
    fn strict_parsing_accepts_partial_input_with_defaults() {
        let input = serde_json::json!({
            "sector": { "method": "custom", "custom_budgets": { "ai-chip": 2.0, "power": 1.0 } }
        });
        let params = StrategyParams::from_json_strict(&input).unwrap();
        assert_eq!(params.sector.method, SectorMethod::Custom);
        assert_eq!(params.asset, AssetParams::default());
    }

    #[test]
    fn weights_respect_caps_and_exposure() {
        let fixture = universe();
        let params = fixture_params();
        let result = fixture.run(&params);
        let total = sum(&result.weights);
        assert!((total - result.invested).abs() < 1e-12);
        assert!(total <= params.exposure.max_exposure + 1e-9);
        assert!(total >= params.exposure.min_exposure - 1e-9);
        for w in &result.weights {
            assert!(*w <= params.asset.max_weight + 1e-9);
        }
        for sector in &result.sectors {
            assert!(sector.weight <= params.sector.max_weight + 1e-9);
        }
        assert!((result.cash_weight - (1.0 - total)).abs() < 1e-12);
    }

    #[test]
    fn single_member_sector_is_bounded_by_the_name_cap() {
        let fixture = universe();
        let result = fixture.run(&StrategyParams::default());
        let space = result.sectors.iter().find(|s| s.sector == "space").unwrap();
        assert!(space.cap <= StrategyParams::default().asset.max_weight + 1e-12);
    }

    #[test]
    fn equal_weight_preset_is_equal_per_name() {
        let fixture = universe();
        let mut params = preset("equal_weight").unwrap().params;
        // Eight test names at 12.25% each would breach the 10% single-name cap.
        params.asset.max_weight = 0.2;
        let result = fixture.run(&params);
        let expected = 0.98 / fixture.assets.len() as f64;
        for w in &result.weights {
            assert!((w - expected).abs() < 1e-9, "{w} vs {expected}");
        }
    }

    #[test]
    fn excluded_names_get_zero_and_frozen_names_keep_their_weight() {
        let fixture = universe();
        let mut current = vec![0.0; fixture.assets.len()];
        current[0] = 0.07; // NVDA held at 7%
        let result = fixture.run_with(&StrategyParams::default(), &["AMD"], &["NVDA"], &current);
        assert_eq!(result.weights[1], 0.0);
        assert_eq!(result.assets[1].status, AssetStatus::Excluded);
        assert_eq!(result.weights[0], 0.07);
        assert_eq!(result.assets[0].status, AssetStatus::Frozen);
        assert!(sum(&result.weights) <= result.exposure_target + 1e-9);
    }

    #[test]
    fn short_history_is_ineligible() {
        let mut fixture = universe();
        for p in fixture.history[7].iter_mut().take(300) {
            *p = f64::NAN; // RKLB "listed" 100 sessions ago
        }
        let result = fixture.run(&StrategyParams::default());
        assert_eq!(result.assets[7].status, AssetStatus::InsufficientHistory);
        assert_eq!(result.weights[7], 0.0);
    }

    #[test]
    fn missing_price_holds_existing_position() {
        let mut fixture = universe();
        fixture.current[3] = f64::NAN;
        let mut current = vec![0.0; fixture.assets.len()];
        current[3] = 0.05;
        let result = fixture.run_with(&StrategyParams::default(), &[], &[], &current);
        assert_eq!(result.assets[3].status, AssetStatus::NoPrice);
        assert_eq!(result.weights[3], 0.05);
    }

    #[test]
    fn downtrend_is_penalised_and_trend_exit_removes_the_name() {
        let fixture = universe();
        let mut params = StrategyParams::default();
        params.asset.trend_ramp = 0.0;
        let base = fixture.run(&params);
        let wdc = &base.assets[4];
        assert!(wdc.trend.unwrap() < 0.0);
        assert_eq!(wdc.trend_multiplier, 0.5);
        params.asset.trend_penalty = 0.0;
        let exit = fixture.run(&params);
        assert_eq!(exit.weights[4], 0.0);
    }

    #[test]
    fn the_trend_penalty_fades_in_below_the_line() {
        assert_eq!(trend_fade(-0.0, 0.5, 0.1), 1.0);
        assert!((trend_fade(-0.05, 0.5, 0.1) - 0.75).abs() < 1e-12);
        assert_eq!(trend_fade(-0.2, 0.5, 0.1), 0.5);
        assert!((trend_fade(-0.02, 0.0, 0.1) - 0.8).abs() < 1e-12);
        // A step when the ramp is zero.
        assert_eq!(trend_fade(-0.001, 0.5, 0.0), 0.5);
        // In the engine: a name just below its average keeps most of its weight.
        let fixture = universe();
        let result = fixture.run(&StrategyParams::default());
        let wdc = &result.assets[4];
        let expected = trend_fade(wdc.trend.unwrap(), 0.5, 0.1);
        assert!((wdc.trend_multiplier - expected).abs() < 1e-12);
        assert!(wdc.trend_multiplier >= 0.5 && wdc.trend_multiplier <= 1.0);
    }

    #[test]
    fn breadth_scales_exposure_between_bounds() {
        let fixture = Fixture::new(&[
            ("A", "x", -0.002, 0.01),
            ("B", "x", -0.002, 0.01),
            ("C", "y", -0.002, 0.01),
        ]);
        let params = StrategyParams::default();
        let result = fixture.run(&params);
        assert_eq!(result.breadth, Some(0.0));
        assert!((result.exposure_target - params.exposure.min_exposure).abs() < 1e-12);
    }

    #[test]
    fn vol_target_scales_down_exposure() {
        let fixture = universe();
        let unconstrained = fixture.run(&fixture_params()).est_vol.unwrap();
        let target = unconstrained / 2.0;
        let mut params = fixture_params();
        params.exposure.target_vol = Some(target.max(0.03));
        let result = fixture.run(&params);
        let scale = result.vol_scale.expect("vol target binds");
        assert!(scale < 1.0);
        assert!(result.est_vol.unwrap() <= target.max(0.03) + 1e-6);
        assert!(result.invested < fixture.run(&fixture_params()).invested);
    }

    #[test]
    fn min_weight_prunes_tiny_positions() {
        let fixture = universe();
        let baseline = fixture.run(&fixture_params());
        let smallest = baseline
            .weights
            .iter()
            .copied()
            .filter(|w| *w > 0.0)
            .fold(f64::INFINITY, f64::min);
        let mut params = fixture_params();
        params.asset.min_weight = (smallest + 1e-4).min(params.asset.max_weight);
        let result = fixture.run(&params);
        for (i, w) in result.weights.iter().enumerate() {
            assert!(
                *w == 0.0 || *w >= params.asset.min_weight - 1e-12,
                "asset {i} weight {w}"
            );
        }
        assert!(
            result
                .assets
                .iter()
                .any(|a| a.status == AssetStatus::BelowMinWeight)
        );
    }

    #[test]
    fn held_positions_get_a_lower_minimum_than_new_ones() {
        let fixture = universe();
        let baseline = fixture.run(&fixture_params());
        let (smallest_i, smallest) = baseline
            .weights
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, w)| *w > 0.0)
            .fold(
                (0, f64::INFINITY),
                |acc, (i, w)| if w < acc.1 { (i, w) } else { acc },
            );
        let mut params = fixture_params();
        // Above the smallest weight, but less than twice it.
        params.asset.min_weight = smallest * 1.2;
        let fresh = fixture.run(&params);
        assert_eq!(
            fresh.weights[smallest_i], 0.0,
            "a new position needs the full minimum"
        );
        let mut current = vec![0.0; fixture.assets.len()];
        current[smallest_i] = smallest;
        let held = fixture.run_with(&params, &[], &[], &current);
        assert!(
            held.weights[smallest_i] > 0.0,
            "a held position is kept down to half the minimum"
        );
    }

    #[test]
    fn custom_sector_budgets_are_followed() {
        let fixture = universe();
        let mut params = StrategyParams::default();
        params.sector.method = SectorMethod::Custom;
        params.sector.momentum_tilt = 0.0;
        params.sector.trend_aware = false;
        params.sector.min_weight = 0.0;
        params.sector.max_weight = 1.0;
        params.asset.max_weight = 0.5;
        params.exposure.breadth_scaling = false;
        params.sector.custom_budgets =
            BTreeMap::from([("ai-chip".to_string(), 3.0), ("power".to_string(), 1.0)]);
        let result = fixture.run(&params);
        let chip = result
            .sectors
            .iter()
            .find(|s| s.sector == "ai-chip")
            .unwrap();
        let power = result.sectors.iter().find(|s| s.sector == "power").unwrap();
        let storage = result
            .sectors
            .iter()
            .find(|s| s.sector == "storage")
            .unwrap();
        assert!((chip.weight / power.weight - 3.0).abs() < 1e-9);
        assert_eq!(storage.weight, 0.0);
    }

    #[test]
    fn portfolio_vol_estimate_matches_single_asset_vol() {
        let p = path(100, 0.001, 0.02, 0.0);
        let series = vec![p.clone()];
        let est = estimate_portfolio_vol(&series, &[1.0], 63).unwrap();
        let direct = stats::annualized_vol(&p, 63).unwrap();
        assert!((est - direct).abs() < 1e-9);
    }
}
