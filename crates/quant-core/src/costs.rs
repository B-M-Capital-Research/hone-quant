//! Transaction cost model shared by the paper broker and the backtester, so simulated history
//! and simulated live trading pay exactly the same frictions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Buy => "buy",
            Side::Sell => "sell",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "buy" => Some(Side::Buy),
            "sell" => Some(Side::Sell),
            _ => None,
        }
    }

    /// +1 for buys, −1 for sells.
    pub fn sign(self) -> f64 {
        match self {
            Side::Buy => 1.0,
            Side::Sell => -1.0,
        }
    }
}

/// Defaults approximate a US retail-professional broker's fixed tier: $0.005/share with a $1.00
/// minimum and a 1% cap, 5 bps of slippage against the reference price, and the SEC Section 31
/// fee on sales.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CostModel {
    pub commission_per_share: f64,
    pub commission_min: f64,
    /// Upper bound on the per-share commission as a fraction of notional (0 disables the cap).
    pub commission_max_rate: f64,
    /// Additional commission as a fraction of notional.
    pub commission_rate: f64,
    /// Adverse execution versus the reference price, in basis points.
    pub slippage_bps: f64,
    /// Regulatory fee on sale proceeds, as a fraction of notional.
    pub sell_fee_rate: f64,
}

impl Default for CostModel {
    fn default() -> Self {
        Self {
            commission_per_share: 0.005,
            commission_min: 1.0,
            commission_max_rate: 0.01,
            commission_rate: 0.0,
            slippage_bps: 5.0,
            sell_fee_rate: 0.000_027_8,
        }
    }
}

impl CostModel {
    pub fn zero() -> Self {
        Self {
            commission_per_share: 0.0,
            commission_min: 0.0,
            commission_max_rate: 0.0,
            commission_rate: 0.0,
            slippage_bps: 0.0,
            sell_fee_rate: 0.0,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let checks = [
            ("commission_per_share", self.commission_per_share, 0.0, 1.0),
            ("commission_min", self.commission_min, 0.0, 100.0),
            ("commission_max_rate", self.commission_max_rate, 0.0, 0.05),
            ("commission_rate", self.commission_rate, 0.0, 0.01),
            ("slippage_bps", self.slippage_bps, 0.0, 200.0),
            ("sell_fee_rate", self.sell_fee_rate, 0.0, 0.001),
        ];
        for (name, value, min, max) in checks {
            if !(value.is_finite() && value >= min && value <= max) {
                return Err(format!("{name} must be between {min} and {max}"));
            }
        }
        Ok(())
    }

    /// Execution price after slippage.
    pub fn execution_price(&self, side: Side, reference: f64) -> f64 {
        reference * (1.0 + side.sign() * self.slippage_bps / 10_000.0)
    }

    /// Commission for one fill.
    pub fn commission(&self, qty: f64, price: f64) -> f64 {
        if qty <= 0.0 || price <= 0.0 {
            return 0.0;
        }
        let notional = qty * price;
        let mut per_share = (qty * self.commission_per_share).max(self.commission_min);
        if self.commission_max_rate > 0.0 {
            per_share = per_share.min(notional * self.commission_max_rate);
        }
        per_share + notional * self.commission_rate
    }

    /// Regulatory fees (sales only).
    pub fn fees(&self, side: Side, notional: f64) -> f64 {
        match side {
            Side::Sell => notional.max(0.0) * self.sell_fee_rate,
            Side::Buy => 0.0,
        }
    }

    /// Signed cash impact of a fill at `execution_price` (negative for buys).
    pub fn cash_delta(&self, side: Side, qty: f64, execution_price: f64) -> f64 {
        let notional = qty * execution_price;
        let costs = self.commission(qty, execution_price) + self.fees(side, notional);
        match side {
            Side::Buy => -(notional + costs),
            Side::Sell => notional - costs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commission_has_minimum_and_cap() {
        let m = CostModel::default();
        assert!((m.commission(10.0, 100.0) - 1.0).abs() < 1e-12); // min $1
        assert!((m.commission(1_000.0, 100.0) - 5.0).abs() < 1e-12); // $0.005 × 1000
        // 1% cap: 100 shares at $0.50 = $50 notional → cap $0.50 beats the $1 minimum.
        assert!((m.commission(100.0, 0.5) - 0.5).abs() < 1e-12);
        assert_eq!(m.commission(0.0, 100.0), 0.0);
    }

    #[test]
    fn slippage_is_adverse() {
        let m = CostModel::default();
        assert!(m.execution_price(Side::Buy, 100.0) > 100.0);
        assert!(m.execution_price(Side::Sell, 100.0) < 100.0);
        assert!((m.execution_price(Side::Buy, 100.0) - 100.05).abs() < 1e-12);
    }

    #[test]
    fn cash_delta_includes_costs() {
        let m = CostModel::default();
        let buy = m.cash_delta(Side::Buy, 100.0, 50.0);
        assert!((buy + 5_001.0).abs() < 1e-9);
        let sell = m.cash_delta(Side::Sell, 100.0, 50.0);
        assert!((sell - (5_000.0 - 1.0 - 5_000.0 * 0.000_027_8)).abs() < 1e-9);
    }

    #[test]
    fn zero_model_is_frictionless() {
        let m = CostModel::zero();
        assert_eq!(m.cash_delta(Side::Buy, 10.0, 10.0), -100.0);
        assert!(m.validate().is_ok());
        assert!(CostModel::default().validate().is_ok());
        let bad = CostModel {
            slippage_bps: -1.0,
            ..CostModel::default()
        };
        assert!(bad.validate().is_err());
    }
}
