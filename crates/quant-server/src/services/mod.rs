//! Business logic on top of the store: market data sync, planning, the paper broker,
//! valuation, plan explanations, the AI sector view, the automation loop, backtests and reminders.
pub mod ai_view;
#[cfg(test)]
mod ai_view_tests;
pub mod backtests;
pub mod bootstrap;
pub mod broker;
pub mod explain;
#[cfg(test)]
mod explain_tests;
pub mod marketdata;
pub mod planner;
pub mod portfolio;
#[cfg(test)]
pub(crate) mod portfolio_tests;
pub mod reminders;
pub mod scheduler;
