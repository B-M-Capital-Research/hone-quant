//! Data access, grouped by domain. Functions take any `deadpool_postgres::GenericClient`, so the
//! same code runs on a pooled client or inside a transaction.
pub mod market;
pub mod settings;
pub mod strategy;
pub mod system;
pub mod trading;
