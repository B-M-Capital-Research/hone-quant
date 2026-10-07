//! Business logic on top of the store: market data sync, planning, the paper broker,
//! valuation, the automation loop, backtests and reminders.
pub mod backtests;
pub mod bootstrap;
pub mod broker;
pub mod marketdata;
pub mod planner;
pub mod portfolio;
#[cfg(test)]
pub(crate) mod portfolio_tests;
pub mod reminders;
pub mod scheduler;
