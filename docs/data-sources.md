# Data sources

Everything below was verified against live installations on 2026-07-17
(Codex CLI 0.x rollout format; Claude Code 2.1.x). Field lists show only
what the collectors consume — both formats contain far more, including
conversation content, which is deliberately never read into memory
structures (see `docs/privacy.md`).

## Codex CLI

**Location:** `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`
(plus `~/.codex/archived_sessions/` when it exists, deduplicated against the
live copies by session id).

Each line is `{"timestamp": "<RFC3339>", "type": "...", "payload": {...}}`.
Consumed line types:

### `session_meta`

```json
{"type":"session_meta","payload":{
  "id":"<session-uuid>", "timestamp":"<RFC3339>",
  "cwd":"<redacted>", "cli_version":"0.99.0", "source":"cli"}}
```

Used for: session identity, start time, project name (basename of `cwd`
only), CLI version (doctor). `instructions` and git metadata are ignored.

### `turn_context`

```json
{"type":"turn_context","payload":{"model":"gpt-5.6-terra","effort":"high"}}
```

Used for: attributing subsequent token deltas to the correct model,
including mid-session model switches.

### `event_msg` with `payload.type == "token_count"`

```json
{"type":"event_msg","payload":{"type":"token_count",
  "info":{
    "total_token_usage":{"input_tokens":55239,"cached_input_tokens":37632,
      "output_tokens":483,"reasoning_output_tokens":45,"total_tokens":55722},
    "last_token_usage":{...same shape, per turn...},
    "model_context_window":258400},
  "rate_limits":{
    "primary":{"used_percent":42,"window_minutes":10080,"resets_at":1784516428},
    "secondary":null,
    "credits":null,"plan_type":"plus"}}}
```

Semantics the collector depends on (verified empirically):

- `total_token_usage` is **cumulative per session**; the collector diffs
  consecutive snapshots to get deltas, saturating at zero per category.
- Codex `input_tokens` **includes** `cached_input_tokens`; the domain's
  `input` excludes cached tokens, so the mapping subtracts.
- `total_tokens = input_tokens + output_tokens`; `reasoning_output_tokens`
  is a subset of output.
- **Fork/inheritance heuristic:** in every fresh session observed, the
  first `token_count` has `total_token_usage == last_token_usage`. A first
  counter where they differ means the cumulative total contains inherited
  history (fork/resume) that another file already accounted for; only the
  `last_token_usage` part is counted and the cumulative becomes the
  baseline.
- `rate_limits` is account-level, provider-authoritative data. Windows are
  classified **by `window_minutes`** (240–360 → five-hour, 9000–11500 →
  weekly, else unknown), never by `primary`/`secondary` position — live
  data shows `primary` carrying the *weekly* window with `secondary`
  absent. `resets_at` is unix seconds; an alternative
  `resets_in_seconds` (relative) form is also accepted for older formats.
- `credits` is parsed when present as a number or `{balance: ...}`; on the
  observed plan it is `null`, so this path is fixture-tested only.

### Not used

- `~/.codex/history.jsonl` (prompt history — content, no token data).
- `~/.codex/session_index.jsonl` (index; sessions are discovered directly).
- `auth.json` (never opened; `doctor` reports existence only).
- The `codex app-server` JSON-RPC interface could provide push-based
  account usage; not used in the MVP to stay read-only and process-free.

### Codex state database (`~/.codex/state_N.sqlite`)

Newer Codex CLIs (observed with 0.144.x) run an in-process app-server and
record sessions as rows in a `threads` table — sometimes without writing a
rollout file at all. lmtop opens the highest-numbered `state_N.sqlite`
read-only (100 ms busy timeout, never blocking the CLI's writes) and reads
per thread: `id`, `cwd` (basename only, for the project column), `model`,
cumulative `tokens_used`, `created_at`, `updated_at`. Sessions already
owned by a rollout file are skipped, so nothing is double counted.

Limits of this source, surfaced rather than papered over: `tokens_used`
is a single total with no input/cached/output split (deltas are recorded
as *unattributed* tokens, which count toward totals but never pretend to
have a direction), and the table carries no rate-limit snapshots — quota
freshness still depends on rollout files or `--live`.

`logs_N.sqlite` (tracing output) is not read: it contains log text, not
usage data.

## Claude Code

**Location:** `~/.claude/projects/<project-slug>/<session-uuid>.jsonl`,
plus nested subagent transcripts under
`<slug>/<session-uuid>/subagents/agent-*.jsonl` and workflow-spawned agents
another two levels down (`subagents/workflows/wf_*/agent-*.jsonl`).
Discovery walks the tree recursively — fixed-depth globs silently miss the
workflow tier, which carries real usage.

Each line is a self-describing record. Only `type == "assistant"` lines are
consumed (other types — `user`, `attachment`, `file-history-delta`,
`summary`, etc. — are skipped):

```json
{"type":"assistant","uuid":"...","sessionId":"<session-uuid>",
 "timestamp":"2026-07-17T05:39:15.613Z","cwd":"<redacted>",
 "version":"2.1.212","isSidechain":false,"requestId":"req_...",
 "message":{"id":"msg_...","model":"claude-fable-5",
   "usage":{"input_tokens":2,"cache_creation_input_tokens":21833,
            "cache_read_input_tokens":20546,"output_tokens":681, "...":"..."}}}
```

Semantics the collector depends on (verified empirically):

- Usage is **per API request**, not cumulative.
- Claude Code writes **one line per content block**, repeating the same
  `message.id` and identical `usage` on consecutive lines. Usage is counted
  **once per unique `message.id`** (`requestId`, then line `uuid`, as
  fallbacks). The same global dedup key also collapses forked/resumed
  sessions that inherit history lines, and archived copies.
- Claude `input_tokens` **excludes** cache tokens; `cache_read_input_tokens`
  maps to the domain's cached input, `cache_creation_input_tokens` to
  cache-creation input.
- `model == "<synthetic>"` marks client-generated error placeholders with
  no real API usage; skipped.
- Sidechain lines (`isSidechain: true`, subagent traffic) carry real usage
  and are counted toward the session and week.
- Unknown top-level `usage` keys ending in `_tokens` are preserved in the
  domain's `other` map for forward compatibility.
- `usage.iterations[]`, when non-empty, is the per-turn truth for
  responses that involved multiple server-side turns: the top-level
  counters silently omit the largest turn (verified empirically:
  `top_output == sum(iterations) - max(iterations)` on every multi-turn
  line inspected, an undercount of up to 12x on single messages). For
  single-turn lines `iterations[0]` equals the top level. The collector
  therefore sums `iterations[]` whenever present and falls back to the
  top-level counters otherwise.
- Model ids observed: `claude-fable-5`, `claude-opus-4-8`,
  `claude-sonnet-5`, `claude-haiku-4-5-*`, `<synthetic>`. Normalization
  keeps raw ids and classifies unknown families as `Other`.

### Quota

Claude Code caches its own subscription-quota view in **`~/.claude.json`**
under `cachedUsageUtilization`:

```json
{"cachedUsageUtilization":{"fetchedAtMs":1784075794565,
  "utilization":{
    "five_hour":{"utilization":27,"resets_at":"2026-07-15T02:40:00+00:00"},
    "seven_day":{"utilization":35,"resets_at":"2026-07-20T10:00:00+00:00"},
    "limits":[{"kind":"weekly_scoped","group":"weekly","percent":52,
               "resets_at":"...","scope":{"model":{"display_name":"Fable"}},
               "is_active":true}, "..."]}}}
```

The collector extracts exactly this subtree and nothing else from the file
(which also holds account identifiers and OAuth metadata — never read into
program state):

- `five_hour.utilization` / `seven_day.utilization` → the 5-hour and
  weekly windows (percent + `resets_at`).
- `limits[]` entries **with a model scope** → model-specific limits,
  displayed as e.g. `Weekly (Fable)`. Unscoped entries duplicate the named
  windows and are skipped.
- `fetchedAtMs` → the snapshot's capture time. This is a **cache** that
  Claude Code refreshes while in use; the dashboard shows its age and
  refuses burn projections from stale samples.

If `~/.claude.json` is absent the `provider_quota` and `reset_times`
capabilities are not declared and the UI shows quota as unavailable —
unless the live collector below is enabled.
Claude exposes no local credit balance, so `credits` is never declared.

### Not used

- `settings.json`, `.credentials.json` (presence of the credentials file
  is reported by `doctor`; contents are never read), `stats-cache.json`
  (Claude Code's own all-time totals; useful only as a manual cross-check).
- Everything in `~/.claude.json` other than `cachedUsageUtilization`.
- Sidechain agent transcripts beyond their usage fields.

## Live quota (opt-in, `network_quota = true` / `--live`)

For Codex, live quota is fetched by asking the Codex CLI itself first:
lmtop spawns a short-lived `codex app-server` subprocess (JSON-RPC over
stdio) and calls `account/rateLimits/read` — the same call behind the
usage block in Codex's own status panel, so the numbers match it exactly
(used percent, window duration, reset time, credits). The subprocess
authenticates with its own stored credentials; lmtop reads no token on
this path. It is killed as soon as the response arrives. This became the
primary route when the HTTP usage endpoint's bot protection started
challenging third-party TLS fingerprints outright (`cf-mitigated:
challenge`, verified 2026-07-17) — impersonating a browser would be an
arms race; asking the CLI is not.

The direct HTTP endpoints below remain as fallbacks (for Codex, only
when the `codex` binary is unavailable).

## Live quota endpoints (fallback)

The file-based sources above update only while the CLI runs *on this
machine* — usage from another device, or simply time passing, is invisible
to them, and a cached window can even outlive its own reset. The opt-in
live collector closes that gap by querying the endpoint each CLI's own
status screen uses, authenticated with the token that CLI already stores
(verified 2026-07-17):

- **Claude:** `GET https://api.anthropic.com/api/oauth/usage` with the
  OAuth token from `~/.claude/.credentials.json` and header
  `anthropic-beta: oauth-2025-04-20`. The response body is exactly the
  `utilization` subtree cached in `~/.claude.json`, so it flows through
  the same parser. Fetched at most once per minute.
- **Codex:** `GET https://chatgpt.com/backend-api/codex/usage` with the
  token and `chatgpt-account-id` from `~/.codex/auth.json`. Response
  carries `rate_limit.primary_window` / `secondary_window`
  (`used_percent`, `limit_window_seconds`, `reset_at`) plus
  `credits.balance`; it is normalized into the rollout `rate_limits`
  shape and ingested through the same path. Fetched at most once per
  five minutes — the endpoint sits behind bot protection that answers
  403 to clients it dislikes or that poll too fast. (It also rejects
  rustls and curl TLS fingerprints outright, which is why lmtop uses
  native-tls/OpenSSL, the same stack as the Codex CLI itself.)

A failed live fetch degrades to the file-based sources and says so in the
collector health line; expired tokens are reported ("run Claude Code /
Codex to refresh"), never refreshed by lmtop. See `docs/privacy.md` for
the exact credential-handling contract.

## Freshness caveat

The default collectors are file-based: quota snapshots and token counts
are only as fresh as the most recent agent activity on this machine. The
dashboard surfaces this — quota lines older than 10 minutes carry an age
marker, burn/exhaustion projections are suppressed entirely when the
newest quota sample is older than 30 minutes rather than extrapolating
stale trends, and a window whose reported reset time has already passed
is rendered as *stale* (the percentage describes a finished window, not
the current one). Enabling the live collector removes the staleness at
the cost of two authenticated GETs per refresh interval.

## Schema drift

Both formats are undocumented and change between CLI versions. The
collectors parse defensively (missing fields degrade to "unknown", never
panic), count parse errors into collector health, and unknown quota-window
durations render as unknown windows instead of being force-fitted into the
five-hour/weekly buckets.
