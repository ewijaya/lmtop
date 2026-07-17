//! Product naming, kept in one place: everything user-visible derives
//! from the Cargo package name. The product is `lmtop` ("language-model
//! top"), styled lowercase like `top`, `htop`, and `btop`.

/// Binary / product name (from Cargo.toml `name`).
pub const APP_NAME: &str = env!("CARGO_PKG_NAME");

/// Version string.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Directory name used for config/cache locations.
pub const APP_DIR: &str = APP_NAME;

/// Environment variable enabling diagnostic logging (env vars cannot be
/// derived from the package name at compile time).
pub const LOG_ENV: &str = "LMTOP_LOG";

/// Application identifier (`lmtop/<version>`), for any future
/// network-facing use such as a user agent. Nothing in the current build
/// makes network calls.
pub fn app_ident() -> String {
    format!("{APP_NAME}/{APP_VERSION}")
}
