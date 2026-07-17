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
# Allow a future network-backed quota collector (off: local-first).
network_quota = false

[providers.claude]
enabled = true
# Extra directories to scan for Claude Code project logs, in addition to
# ~/.claude/projects.
session_paths = []
network_quota = false

[ui]
# Seconds between filesystem rescans. Rendering is independent of this.
refresh_secs = 5
theme = "dark"
# ASCII-only bars/charts for terminals without good unicode support.
ascii = false
# Never touch the network (also available as --offline).
offline = false
# Redraw once per second instead of 4x/second.
reduced_motion = false

[time]
# First day of the calendar week: monday | tuesday | ... | sunday.
week_start = "monday"
# "local" (system timezone) or a fixed UTC offset in hours: "+9", "-5".
# Affects calendar-week boundaries only; provider quota windows are
# provider-defined and unaffected.
timezone = "local"

[history]
# Minutes of token-rate history kept for the chart.
retention_minutes = 60
```

Unknown keys are rejected with an error message naming the key, so typos
fail loudly instead of being silently ignored.

## CLI flags

Flags override the config file:

```text
--provider codex|claude   monitor one provider only
--offline                 disable all network access
--refresh <secs>          collector refresh interval
--ascii                   ASCII bars and charts
--config <path>           alternate config file
```
