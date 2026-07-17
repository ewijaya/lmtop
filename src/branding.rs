//! Product naming, isolated for a clean rename. The current name is a
//! temporary internal codename; everything user-visible derives from the
//! Cargo package name, so renaming the package renames the product.

/// Binary / product name (from Cargo.toml `name`).
pub const APP_NAME: &str = env!("CARGO_PKG_NAME");

/// Version string.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Directory name used for config/cache locations.
pub const APP_DIR: &str = APP_NAME;

/// Environment variable enabling diagnostic logging. Update alongside the
/// package name on rename (env vars cannot be derived at compile time).
pub const LOG_ENV: &str = "AGENTOP_LOG";
