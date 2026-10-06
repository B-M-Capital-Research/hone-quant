//! hone-quant server library. The binary in `main.rs` is a thin CLI over these modules.

// `!(x > 0.0)` is deliberate: it also rejects NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod api;
pub mod auth;
pub mod config;
pub mod crypto;
pub mod db;
pub mod honeclaw_auth;
pub mod market;
pub mod notify;
pub mod services;
pub mod state;
pub mod store;
pub mod universe;
pub mod web;

/// Git revision baked in by the release build (`HONE_QUANT_REVISION`, see build.rs), or "dev".
pub const REVISION: &str = env!("HONE_QUANT_REVISION_OR_DEV");
