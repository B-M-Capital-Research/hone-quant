//! # quant-core
//!
//! Pure, deterministic domain logic for hone-quant. Nothing in this crate performs I/O, reads the
//! clock or touches the network: every function is a function of its inputs, which is what lets
//! the live paper-trading path and the backtester share one implementation and lets every plan be
//! reproduced from its stored inputs.
//!
//! - [`calendar`]: NYSE sessions, holidays, early closes, DST.
//! - [`schedule`]: the two daily plan slots (first three hours / three hours before the close).
//! - [`stats`]: volatility, moving averages, momentum, ranks, covariance.
//! - [`alloc`]: capped proportional ("water-filling") allocation.
//! - [`strategy`]: parameters, presets and the target-weight engine.
//! - [`rebalance`]: drift bands, lot rounding, turnover and cash constraints → orders.
//! - [`costs`]: commissions, slippage and fees.
//! - [`backtest`]: daily-bar simulation using all of the above.
//! - [`metrics`]: performance analytics.

// Numeric panels are indexed by asset and time; index loops read better than zipped iterators.
#![allow(clippy::needless_range_loop)]
// `!(x > 0.0)` is deliberate: it also rejects NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod alloc;
pub mod backtest;
pub mod calendar;
pub mod costs;
pub mod metrics;
pub mod rebalance;
pub mod schedule;
pub mod stats;
pub mod strategy;
