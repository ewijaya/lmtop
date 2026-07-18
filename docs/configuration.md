# Configuration

The application works with **zero configuration**. Every setting below is
optional; the file only needs to exist if you want to change a default.

## Location

| Platform | Path |
|---|---|
| Linux / WSL | `~/.config/lmtop/config.toml` |
| macOS | `~/Library/Application Support/lmtop/config.toml` |
| Windows | `%APPDATA%\lmtop\config\config.toml` |

(Run `lmtop doctor` to see the exact path on your machine.) An alternate file can be passed with
`--config <path>`.

## Full reference (defaults shown)

```toml
[providers.codex]
enabled = true
# Extra directories to scan for Codex rollout files, in addition to
# ~/.codex/sessions.
session_paths = []
# Fetch live quota from the provider's own usage endpoint, authenticated
# with the access token the provider's CLI already stores locally
# (~/.codex/auth.json). Off by default: the application is local-first.
# --offline always wins over this setting. See docs/privacy.md.
network_quota = false

[providers.claude]
enabled = true
# Extra directories to scan for Claude Code project logs, in addition to
# ~/.claude/projects.
session_paths = []
# Same as above, using ~/.claude/.credentials.json.
network_quota = false

# A user-defined provider (Gemini, Ollama, OpenRouter, …) fed by JSON you
# produce. See "Custom provider" below for the schema.
[providers.custom]
enabled = false
# Display name used across the UI.
name = "Custom"
# EITHER a JSON file lmtop re-reads each refresh…
#source = "/path/to/usage.json"
# …OR a command whose stdout is that JSON (ignored when source is set).
# The command runs via bash each refresh, so keep it fast.
#command = "my-usage-exporter"

[ui]
# Seconds between filesystem rescans. Rendering is independent of this.
refresh_secs = 5
# Color palette. 45 ship in the binary — dark | light | catppuccin |
# gruvbox | nord plus btop's theme set (dracula, tokyo-night, onedark,
# solarized_dark, …; full list in the repo's themes/ directory). Your own
# ~/.config/lmtop/themes/<name>.toml files are also picked up by name and
# may override shipped ones; the t/T keys cycle themes at runtime.
# Unknown names fall back to dark. 16-color terminals ignore this.
theme = "dark"
# Chart drawing symbol for the token-rate / quota chart:
#   braille — highest resolution
#   block   — medium resolution, broader font compatibility
#   tty     — lowest resolution, maximum terminal compatibility
# Unknown values fall back to braille; ascii = true overrides this.
graph_symbol = "braille"
# ASCII-only bars/charts for terminals without good unicode support.
ascii = false
# Never touch the network (also available as --offline).
offline = false
# Redraw once per second instead of 4x/second, and disable the
# active-session pulse.
reduced_motion = false

[time]
# First day of the calendar week: monday | tuesday | ... | sunday.
week_start = "monday"
# "local" (system timezone) or a fixed UTC offset in hours: "+9", "-5".
# Affects calendar-week boundaries only; provider quota windows are
# provider-defined and unaffected.
timezone = "local"

[history]
# Minutes of token-rate history kept for the live chart window.
retention_minutes = 60
# Persist rate and quota history across runs (JSONL in the data dir, e.g.
# ~/.local/share/lmtop/history.jsonl). Powers the pannable history view
# and the quota timeline. Contains timestamps, token counts, and quota
# percentages only — never session ids, project names, or content.
persist = true
# Days of persisted history kept; older entries are pruned at startup.
retention_days = 30

[alerts]
enabled = true
# Fire once when a window's used percentage crosses each threshold
# (re-armed when the window resets). Only the highest crossed threshold
# fires per reading.
quota_thresholds = [80.0, 95.0]
# Fire when projected exhaustion is within this many minutes and before
# the window's reset.
exhaustion_warn_minutes = 30
# Ring the terminal bell.
bell = true
# Desktop notification via notify-send (Linux) or osascript (macOS),
# best effort — silently skipped if unavailable.
desktop = true
# Optional command run on every alert (via bash) with these env vars:
# LMTOP_ALERT_TITLE, LMTOP_ALERT_BODY, LMTOP_ALERT_SEVERITY,
# LMTOP_ALERT_PROVIDER.
#command = "my-alert-hook"
```

Unknown keys are rejected with an error message naming the key, so typos
fail loudly instead of being silently ignored.

Alerts are evaluated against provider-reported quota only, and only when
the underlying data is less than an hour old — stale local snapshots never
alert. Alerts fire from the interactive dashboard, not from `snapshot` or
`line`.

## Custom provider

`[providers.custom]` wires in any provider lmtop has no built-in collector
for. Point `source` at a JSON file (or `command` at a program printing the
same JSON) with this shape — every field optional except session `id`s:

```json
{
  "captured_at": "2026-07-17T10:00:00Z",
  "quota_windows": [
    {"used_percent": 41.5, "window_minutes": 300,
     "resets_at": "2026-07-17T14:00:00Z", "scope": null}
  ],
  "credits": 12.5,
  "sessions": [
    {"id": "abc", "model": "gemini-2.5-pro", "project": "myapp",
     "started_at": "2026-07-17T08:00:00Z",
     "last_activity": "2026-07-17T09:59:00Z",
     "tokens": {"input": 100, "cached_input": 0, "cache_creation": 0,
                "output": 50, "reasoning": 0},
     "context_tokens": 40000, "context_window": 1000000}
  ]
}
```

Session token counts are **cumulative**; lmtop computes deltas between
refreshes (feeding the week aggregate and rate chart) and ignores shrinking
counters. Quota windows are classified by `window_minutes` (~300 → 5h,
~10080 → weekly) and get the same burn-trend estimation as the built-in
providers. Capabilities are inferred from which fields you provide.

## CLI flags

Flags override the config file:

```text
--provider codex|claude|custom   monitor one provider only
--offline                        disable all network access
--live                           network_quota = true for all enabled providers
--refresh <secs>                 collector refresh interval
--ascii                          ASCII bars and charts
--config <path>                  alternate config file
```

## Subcommands

```text
lmtop                 interactive dashboard
lmtop snapshot        one-shot text summary
lmtop snapshot --json machine-readable snapshot
lmtop line            one-line colored summary for status bars
lmtop line --plain    …without ANSI colors
lmtop doctor          discovery, parse health, capabilities
```

`lmtop line` is built for embedding: tmux `status-right`, starship custom
commands, waybar `custom` modules, or a Claude Code statusline. Colors are
emitted only when stdout is a terminal (force with `--color`).
