//! Opt-in live quota fetchers (`network_quota = true` or `--live`).
//!
//! The file-based quota caches are only as fresh as the last agent activity
//! on this machine — usage from other devices, or simply time passing, is
//! invisible to them. These fetchers ask each provider's own usage endpoint
//! (the same one the provider's CLI queries for its status screen) using the
//! access token the CLI already stores locally.
//!
//! Privacy contract (see docs/privacy.md):
//! - Disabled by default; `--offline` always wins over `network_quota`.
//! - Only the access token needed for the `Authorization` header is read
//!   from the CLI's credential file. It is kept in memory for the request,
//!   never logged, persisted, displayed, or sent anywhere but the
//!   provider's own usage endpoint.
//! - Only usage endpoints are contacted; nothing is uploaded beyond the
//!   request itself.
//!
//! Endpoints (verified 2026-07-17):
//! - Claude: `GET https://api.anthropic.com/api/oauth/usage` with the OAuth
//!   token from `~/.claude/.credentials.json`. The response body is exactly
//!   the `utilization` subtree Claude Code caches in `~/.claude.json`.
//! - Codex: `GET https://chatgpt.com/backend-api/codex/usage` with the
//!   token (and account id header) from `~/.codex/auth.json`. The response
//!   carries `rate_limit.primary_window` / `secondary_window`
//!   (`used_percent`, `limit_window_seconds`, `reset_at`) plus
//!   `credits.balance`.

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Minimum spacing between network fetches. Collector scans run every few
/// seconds; hitting the provider that often would be rude and pointless.
const CLAUDE_FETCH_INTERVAL_SECS: i64 = 60;

/// The Codex endpoint sits behind aggressive bot protection that
/// temporarily blocks clients making bursts of requests — poll it gently.
const CODEX_FETCH_INTERVAL_SECS: i64 = 300;

/// HTTP timeout. Fetches run on the collector's blocking task, so a slow
/// endpoint delays one provider's refresh, never the UI.
const HTTP_TIMEOUT_SECS: u64 = 8;

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/codex/usage";

/// `None` if the platform TLS backend fails to initialize (effectively
/// never; surfaced as a fetch error rather than a panic).
fn agent() -> Option<ureq::Agent> {
    let connector = native_tls::TlsConnector::new().ok()?;
    Some(
        ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
            .user_agent(concat!(
                "lmtop/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/ewijaya/lmtop)"
            ))
            .tls_connector(std::sync::Arc::new(connector))
            .build(),
    )
}

/// Shared fetch pacing + last-result cache. Errors are stored, not
/// propagated: a failed live fetch must degrade to the file cache, and its
/// message must never contain token material (only our own words and HTTP
/// status codes are used).
pub struct LiveQuota {
    creds_path: Option<PathBuf>,
    agent: Option<ureq::Agent>,
    interval_secs: i64,
    last_attempt: Option<DateTime<Utc>>,
    /// Last successful response and when it was fetched.
    last_value: Option<(Value, DateTime<Utc>)>,
    /// Human-readable reason the last attempt failed, for collector health.
    pub last_error: Option<String>,
}

impl LiveQuota {
    fn new(creds_path: Option<PathBuf>, interval_secs: i64) -> Self {
        LiveQuota {
            creds_path,
            agent: agent(),
            interval_secs,
            last_attempt: None,
            last_value: None,
            last_error: None,
        }
    }

    pub fn for_claude() -> Self {
        Self::new(
            directories::UserDirs::new()
                .map(|d| d.home_dir().join(".claude").join(".credentials.json")),
            CLAUDE_FETCH_INTERVAL_SECS,
        )
    }

    pub fn for_codex() -> Self {
        Self::new(
            directories::UserDirs::new().map(|d| d.home_dir().join(".codex").join("auth.json")),
            CODEX_FETCH_INTERVAL_SECS,
        )
    }

    fn due(&self, now: DateTime<Utc>) -> bool {
        self.last_attempt
            .is_none_or(|t| now - t >= Duration::seconds(self.interval_secs))
    }

    fn fetch(&mut self, now: DateTime<Utc>, provider: LiveProvider) -> bool {
        self.last_attempt = Some(now);
        let Some(agent) = &self.agent else {
            self.last_error = Some("TLS initialization failed".into());
            return false;
        };
        match provider.request(agent, self.creds_path.as_deref()) {
            Ok(value) => {
                self.last_value = Some((value, now));
                self.last_error = None;
                true
            }
            Err(msg) => {
                self.last_error = Some(msg);
                false
            }
        }
    }

    /// The freshest Claude utilization JSON, fetching when due. Between
    /// fetches (and after failures) the last successful response is reused.
    pub fn claude_utilization(&mut self, now: DateTime<Utc>) -> Option<(Value, DateTime<Utc>)> {
        if self.due(now) {
            self.fetch(now, LiveProvider::Claude);
        }
        self.last_value.clone()
    }

    /// A newly fetched Codex usage response, normalized into the same
    /// `rate_limits` shape the rollout files carry. Returns `Some` only when
    /// a fetch actually happened, so callers ingest each sample exactly once.
    ///
    /// The Codex CLI's own app-server is asked first: it reports the same
    /// numbers as the CLI's status panel, authenticates itself, and its
    /// requests are not challenged by the usage endpoint's bot protection
    /// the way third-party clients are. The direct HTTP endpoint remains
    /// as a fallback for machines where the `codex` binary is missing.
    pub fn codex_rate_limits(&mut self, now: DateTime<Utc>) -> Option<Value> {
        if !self.due(now) {
            return None;
        }
        self.last_attempt = Some(now);
        let app_server_error = match super::codex_appserver::fetch_rate_limits() {
            Ok(rate_limits) => {
                self.last_value = Some((rate_limits.clone(), now));
                self.last_error = None;
                return Some(super::codex_appserver::normalize_app_server_rate_limits(
                    &rate_limits,
                ));
            }
            Err(e) => e,
        };
        if !self.fetch(now, LiveProvider::Codex) {
            // Both routes failed; report the app-server one first — it is
            // the one the user can usually do something about.
            let http_error = self.last_error.take().unwrap_or_default();
            self.last_error = Some(format!("{app_server_error}; endpoint: {http_error}"));
            return None;
        }
        self.last_value
            .as_ref()
            .map(|(v, _)| normalize_codex_usage(v))
    }
}

enum LiveProvider {
    Claude,
    Codex,
}

impl LiveProvider {
    fn request(
        &self,
        agent: &ureq::Agent,
        creds_path: Option<&std::path::Path>,
    ) -> Result<Value, String> {
        let creds_path = creds_path.ok_or("no home directory")?;
        let creds = read_creds(creds_path)?;
        let request = match self {
            LiveProvider::Claude => {
                let token = claude_token(&creds)?;
                agent
                    .get(CLAUDE_USAGE_URL)
                    .set("Authorization", &format!("Bearer {token}"))
                    .set("anthropic-beta", "oauth-2025-04-20")
            }
            LiveProvider::Codex => {
                let (token, account_id) = codex_token(&creds)?;
                let mut request = agent
                    .get(CODEX_USAGE_URL)
                    .set("Authorization", &format!("Bearer {token}"));
                if let Some(account_id) = account_id {
                    request = request.set("chatgpt-account-id", &account_id);
                }
                request
            }
        };
        match request.call() {
            Ok(response) => response
                .into_json::<Value>()
                .map_err(|_| "unparsable usage response".to_string()),
            Err(ureq::Error::Status(401, _)) => Err(format!(
                "token rejected; run {} to refresh it",
                match self {
                    LiveProvider::Claude => "Claude Code",
                    LiveProvider::Codex => "Codex",
                }
            )),
            // The Codex endpoint's bot protection answers 403 to clients it
            // dislikes or that poll too fast. A rejected token answers 401
            // (handled above), so this is not an auth problem and must not
            // send anyone off to re-authenticate.
            Err(ureq::Error::Status(403, _)) => {
                Err("endpoint refused the request (bot protection, not auth)".to_string())
            }
            Err(ureq::Error::Status(code, _)) => Err(format!("usage endpoint returned {code}")),
            Err(ureq::Error::Transport(_)) => Err("network unreachable".to_string()),
        }
    }
}

/// Read the credential file. Only the token fields extracted below ever
/// leave this module, and only inside the `Authorization` header.
fn read_creds(path: &std::path::Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|_| format!("no readable credentials at {}", redact_home(path)))?;
    serde_json::from_str(&text).map_err(|_| "unparsable credential file".to_string())
}

fn redact_home(path: &std::path::Path) -> String {
    let shown = path.display().to_string();
    match directories::UserDirs::new() {
        Some(dirs) => shown.replace(&dirs.home_dir().display().to_string(), "~"),
        None => shown,
    }
}

/// `~/.claude/.credentials.json` → `claudeAiOauth.accessToken`, honoring
/// the stored expiry (Claude Code refreshes the token while it runs).
fn claude_token(creds: &Value) -> Result<String, String> {
    let oauth = creds
        .get("claudeAiOauth")
        .ok_or("credential file has no OAuth section")?;
    if let Some(expires_ms) = oauth.get("expiresAt").and_then(Value::as_i64)
        && let Some(expiry) = DateTime::from_timestamp_millis(expires_ms)
        && expiry <= Utc::now()
    {
        return Err("OAuth token expired; open Claude Code to refresh it".into());
    }
    oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "credential file has no access token".into())
}

/// `~/.codex/auth.json` → `tokens.access_token` (+ `tokens.account_id` for
/// the `chatgpt-account-id` header).
fn codex_token(creds: &Value) -> Result<(String, Option<String>), String> {
    let tokens = creds
        .get("tokens")
        .ok_or("auth file has no tokens section")?;
    let access = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or("auth file has no access token")?;
    let account_id = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((access, account_id))
}

/// Map the Codex usage endpoint's response onto the `rate_limits` shape the
/// rollout files already carry, so the collector ingests both through one
/// tested path. Endpoint fields: `rate_limit.primary_window` /
/// `secondary_window` with `used_percent`, `limit_window_seconds`,
/// `reset_at` (unix seconds) or `reset_after_seconds`; `credits.balance`
/// is a decimal string.
pub fn normalize_codex_usage(response: &Value) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(rate_limit) = response.get("rate_limit") {
        for (src, dst) in [
            ("primary_window", "primary"),
            ("secondary_window", "secondary"),
        ] {
            let Some(window) = rate_limit.get(src).filter(|w| !w.is_null()) else {
                continue;
            };
            let mut mapped = serde_json::Map::new();
            if let Some(pct) = window.get("used_percent") {
                mapped.insert("used_percent".into(), pct.clone());
            }
            if let Some(secs) = window.get("limit_window_seconds").and_then(Value::as_u64) {
                mapped.insert("window_minutes".into(), json!(secs / 60));
            }
            if let Some(reset) = window.get("reset_at").filter(|v| v.is_i64() || v.is_u64()) {
                mapped.insert("resets_at".into(), reset.clone());
            } else if let Some(relative) = window.get("reset_after_seconds") {
                mapped.insert("resets_in_seconds".into(), relative.clone());
            }
            out.insert(dst.into(), Value::Object(mapped));
        }
    }
    if let Some(balance) = response
        .get("credits")
        .and_then(|c| c.get("balance"))
        .filter(|v| !v.is_null())
    {
        out.insert("credits".into(), json!({ "balance": balance.clone() }));
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_codex_usage_response() {
        let response = json!({
            "plan_type": "plus",
            "rate_limit": {
                "allowed": true,
                "primary_window": {
                    "used_percent": 60,
                    "limit_window_seconds": 604800,
                    "reset_after_seconds": 586425,
                    "reset_at": 1784861444_i64
                },
                "secondary_window": null
            },
            "credits": {
                "has_credits": true,
                "unlimited": false,
                "balance": "187.5097062500"
            }
        });
        let normalized = normalize_codex_usage(&response);
        let primary = &normalized["primary"];
        assert_eq!(primary["used_percent"], json!(60));
        assert_eq!(primary["window_minutes"], json!(10_080));
        assert_eq!(primary["resets_at"], json!(1784861444_i64));
        assert!(normalized.get("secondary").is_none());
        assert_eq!(normalized["credits"]["balance"], json!("187.5097062500"));
    }

    #[test]
    fn normalizes_relative_reset_and_missing_credits() {
        let response = json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 12.5,
                    "limit_window_seconds": 18000,
                    "reset_after_seconds": 900
                }
            },
            "credits": { "balance": null }
        });
        let normalized = normalize_codex_usage(&response);
        let primary = &normalized["primary"];
        assert_eq!(primary["window_minutes"], json!(300));
        assert_eq!(primary["resets_in_seconds"], json!(900));
        assert!(normalized.get("credits").is_none());
    }

    #[test]
    fn claude_token_rejects_expired() {
        let creds = json!({
            "claudeAiOauth": { "accessToken": "tok", "expiresAt": 1000 }
        });
        assert!(claude_token(&creds).is_err());
    }

    #[test]
    fn claude_token_reads_unexpired() {
        let future = (Utc::now() + Duration::hours(1)).timestamp_millis();
        let creds = json!({
            "claudeAiOauth": { "accessToken": "tok", "expiresAt": future }
        });
        assert_eq!(claude_token(&creds).unwrap(), "tok");
    }

    #[test]
    fn codex_token_reads_access_and_account() {
        let creds = json!({
            "tokens": { "access_token": "tok", "account_id": "acct" }
        });
        assert_eq!(
            codex_token(&creds).unwrap(),
            ("tok".to_string(), Some("acct".to_string()))
        );
    }
}
