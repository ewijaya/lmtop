//! The `doctor` subcommand and privacy-safe diagnostics.

use crate::domain::{Capability, Provider, ProviderSnapshot};
use serde::Serialize;
use std::path::PathBuf;

/// Replace the user's home directory with `~` in any user-visible path or
/// message. Diagnostics must never leak absolute home paths, and nothing in
/// this module ever reads credential file contents.
pub fn redact_path(text: &str) -> String {
    match std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        Ok(home) if !home.is_empty() => text.replace(&home, "~"),
        _ => text.to_string(),
    }
}

/// What a collector discovered about its provider installation. Presence
/// booleans only — no credential contents, no file bodies.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryInfo {
    pub provider: Provider,
    pub installed: bool,
    /// Redacted (`~`-relative) session directories that exist.
    pub session_dirs: Vec<String>,
    pub session_files: u64,
    /// Whether an authentication artifact exists (presence only).
    pub auth_present: bool,
    pub cli_version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub version: String,
    pub config_path: Option<String>,
    pub providers: Vec<ProviderDoctor>,
}

#[derive(Debug, Serialize)]
pub struct ProviderDoctor {
    pub discovery: DiscoveryInfo,
    pub enabled: bool,
    pub status: String,
    pub message: Option<String>,
    pub files_scanned: u64,
    pub parse_errors: u64,
    pub last_data_at: Option<String>,
    pub capabilities: Vec<String>,
}

impl ProviderDoctor {
    pub fn from_snapshot(
        discovery: DiscoveryInfo,
        enabled: bool,
        snapshot: &ProviderSnapshot,
    ) -> Self {
        let last_data_at = snapshot
            .sessions
            .first()
            .and_then(|s| s.last_activity)
            .map(|t| t.to_rfc3339());
        ProviderDoctor {
            discovery,
            enabled,
            status: format!("{:?}", snapshot.health.status).to_lowercase(),
            message: snapshot.health.message.clone(),
            files_scanned: snapshot.health.files_scanned,
            parse_errors: snapshot.health.parse_errors,
            last_data_at,
            capabilities: snapshot
                .capabilities
                .iter()
                .map(|c| c.label().to_string())
                .collect(),
        }
    }
}

impl DoctorReport {
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("{} {}\n", crate::branding::APP_NAME, self.version));
        out.push_str(&format!(
            "config: {}\n\n",
            self.config_path.as_deref().unwrap_or("(defaults, no file)")
        ));
        for p in &self.providers {
            let d = &p.discovery;
            out.push_str(&format!("[{}]\n", d.provider.display_name()));
            out.push_str(&format!("  enabled:        {}\n", p.enabled));
            out.push_str(&format!("  installed:      {}\n", d.installed));
            if let Some(v) = &d.cli_version {
                out.push_str(&format!("  cli version:    {v}\n"));
            }
            if d.session_dirs.is_empty() {
                out.push_str("  session dirs:   none found\n");
            } else {
                for dir in &d.session_dirs {
                    out.push_str(&format!("  session dir:    {dir}\n"));
                }
            }
            out.push_str(&format!("  session files:  {}\n", d.session_files));
            out.push_str(&format!(
                "  auth:           {}\n",
                if d.auth_present {
                    "present (not read)"
                } else {
                    "not found"
                }
            ));
            out.push_str(&format!("  status:         {}\n", p.status));
            if let Some(m) = &p.message {
                out.push_str(&format!("  note:           {m}\n"));
            }
            out.push_str(&format!(
                "  parse health:   {} files scanned, {} parse errors\n",
                p.files_scanned, p.parse_errors
            ));
            out.push_str(&format!(
                "  last data:      {}\n",
                p.last_data_at.as_deref().unwrap_or("none")
            ));
            out.push_str(&format!(
                "  capabilities:   {}\n\n",
                if p.capabilities.is_empty() {
                    "none".to_string()
                } else {
                    p.capabilities.join(", ")
                }
            ));
        }
        out
    }
}

/// All capabilities as label strings, for reporting.
pub fn capability_labels(caps: &[Capability]) -> Vec<String> {
    caps.iter().map(|c| c.label().to_string()).collect()
}

/// Redacted display form of a directory list.
pub fn redacted_dirs(dirs: &[PathBuf]) -> Vec<String> {
    dirs.iter()
        .map(|d| redact_path(&d.display().to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_home_directory() {
        let home = std::env::var("HOME").unwrap();
        let input = format!("{home}/.codex/sessions");
        assert_eq!(redact_path(&input), "~/.codex/sessions");
        assert!(!redact_path(&input).contains(&home));
    }

    #[test]
    fn leaves_other_paths_alone() {
        assert_eq!(redact_path("/var/log/thing"), "/var/log/thing");
    }
}
