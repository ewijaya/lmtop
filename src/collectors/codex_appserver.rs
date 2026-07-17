//! Live Codex rate limits via the Codex CLI's own app-server.
//!
//! `codex app-server` speaks JSON-RPC over stdio and exposes
//! `account/rateLimits/read` — the exact call the Codex TUI uses for the
//! usage block in its own status panel. Asking it is strictly better than
//! calling the HTTP usage endpoint ourselves: the CLI's network stack is
//! not challenged by the endpoint's bot protection (ours increasingly is),
//! and lmtop never touches a credential — the subprocess authenticates
//! with its own stored auth.
//!
//! The subprocess is short-lived: spawn, initialize, one read, kill.
//! Every failure degrades to an error string so the caller can fall back
//! to the HTTP endpoint and, past that, to local files.
//!
//! Response shape (verified against codex-cli 0.144.5):
//!
//! ```json
//! {"id":2,"result":{"rateLimits":{
//!   "primary":{"usedPercent":79,"windowDurationMins":10080,"resetsAt":1784861444},
//!   "secondary":null,
//!   "credits":{"hasCredits":true,"unlimited":false,"balance":"187.5097062500"},
//!   "planType":"plus"}}}
//! ```

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Overall budget for spawn + initialize + read. The fetch runs on the
/// collector's blocking task, so overrunning delays one Codex refresh,
/// never the UI.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Fetch current rate limits through `codex app-server`. Returns the raw
/// `rateLimits` object from the RPC response.
pub fn fetch_rate_limits() -> Result<Value, String> {
    let mut child = Command::new("codex")
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("codex CLI unavailable: {e}"))?;
    let result = converse(&mut child);
    // Always reap: the server runs until told otherwise, and an orphaned
    // one would keep polling the account in the background.
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn converse(child: &mut std::process::Child) -> Result<Value, String> {
    let mut stdin = child.stdin.take().ok_or("no stdin pipe")?;
    let stdout = child.stdout.take().ok_or("no stdout pipe")?;
    let (tx, rx) = mpsc::channel::<String>();
    // Reader thread ends when the pipe closes (i.e. when the child is
    // killed by the caller); the channel disconnect then surfaces here.
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + TIMEOUT;

    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "clientInfo": {
            "name": crate::branding::APP_NAME,
            "title": crate::branding::APP_NAME,
            "version": crate::branding::APP_VERSION,
        }},
    });
    writeln!(stdin, "{initialize}").map_err(|_| "app-server closed stdin")?;
    wait_for_response(&rx, 1, deadline)?;

    let read = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "account/rateLimits/read",
        "params": {},
    });
    writeln!(stdin, "{read}").map_err(|_| "app-server closed stdin")?;
    let response = wait_for_response(&rx, 2, deadline)?;

    if let Some(error) = response.get("error") {
        let msg = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!("app-server error: {msg}"));
    }
    response
        .get("result")
        .and_then(|r| r.get("rateLimits"))
        .cloned()
        .ok_or_else(|| "app-server response had no rateLimits".into())
}

/// Read lines until the response with the given id arrives, skipping
/// server-initiated notifications (no `id`) and unparsable lines.
fn wait_for_response(
    rx: &mpsc::Receiver<String>,
    id: i64,
    deadline: Instant,
) -> Result<Value, String> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("codex app-server timed out")?;
        let line = rx.recv_timeout(remaining).map_err(|e| match e {
            mpsc::RecvTimeoutError::Timeout => "codex app-server timed out".to_string(),
            mpsc::RecvTimeoutError::Disconnected => "codex app-server exited early".to_string(),
        })?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            return Ok(value);
        }
    }
}

/// Map the app-server `rateLimits` object onto the `rate_limits` shape the
/// rollout files carry, so the collector ingests every quota source
/// through one tested path. Field mapping: `usedPercent` →
/// `used_percent`, `windowDurationMins` → `window_minutes`, `resetsAt`
/// (unix seconds) → `resets_at`; `credits.balance` (decimal string)
/// passes through for the existing string-tolerant credits parser.
pub fn normalize_app_server_rate_limits(rate_limits: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for key in ["primary", "secondary"] {
        let Some(window) = rate_limits.get(key).filter(|w| !w.is_null()) else {
            continue;
        };
        let mut mapped = serde_json::Map::new();
        if let Some(pct) = window.get("usedPercent") {
            mapped.insert("used_percent".into(), pct.clone());
        }
        if let Some(mins) = window.get("windowDurationMins") {
            mapped.insert("window_minutes".into(), mins.clone());
        }
        if let Some(reset) = window.get("resetsAt").filter(|v| v.is_i64() || v.is_u64()) {
            mapped.insert("resets_at".into(), reset.clone());
        }
        out.insert(key.into(), Value::Object(mapped));
    }
    if let Some(balance) = rate_limits
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

    /// The exact response captured from codex-cli 0.144.5.
    #[test]
    fn normalizes_real_app_server_response() {
        let rate_limits = json!({
            "limitId": "codex",
            "limitName": null,
            "primary": {
                "usedPercent": 79,
                "windowDurationMins": 10080,
                "resetsAt": 1784861444_i64
            },
            "secondary": null,
            "credits": {
                "hasCredits": true,
                "unlimited": false,
                "balance": "187.5097062500"
            },
            "planType": "plus",
            "rateLimitReachedType": null
        });
        let normalized = normalize_app_server_rate_limits(&rate_limits);
        assert_eq!(normalized["primary"]["used_percent"], json!(79));
        assert_eq!(normalized["primary"]["window_minutes"], json!(10_080));
        assert_eq!(normalized["primary"]["resets_at"], json!(1784861444_i64));
        assert!(normalized.get("secondary").is_none());
        assert_eq!(normalized["credits"]["balance"], json!("187.5097062500"));
    }

    #[test]
    fn normalizes_two_windows_and_missing_credits() {
        let rate_limits = json!({
            "primary": { "usedPercent": 56.5, "windowDurationMins": 300 },
            "secondary": { "usedPercent": 42, "windowDurationMins": 10080,
                           "resetsAt": 1784861444_i64 },
            "credits": { "balance": null }
        });
        let normalized = normalize_app_server_rate_limits(&rate_limits);
        assert_eq!(normalized["primary"]["used_percent"], json!(56.5));
        assert!(normalized["primary"].get("resets_at").is_none());
        assert_eq!(normalized["secondary"]["window_minutes"], json!(10_080));
        assert!(normalized.get("credits").is_none());
    }
}
