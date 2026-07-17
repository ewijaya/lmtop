//! Library crate: everything lives here so integration tests can drive
//! collectors and aggregation directly; `main.rs` is a thin shim.

pub mod aggregation;
pub mod alerts;
pub mod app;
pub mod branding;
pub mod cli;
pub mod collectors;
pub mod config;
pub mod diagnostics;
pub mod domain;
pub mod persist;
pub mod tui;
