# PRD: OpenCode collector

| | |
|---|---|
| **Status** | Draft, ready for review |
| **Author** | Edward Wijaya |
| **Date** | 2026-11-20 |
| **Target version** | lmtop 0.4.0 (minor bump: new provider) |
| **Verified against** | OpenCode v2.3.2 (`opencode.db`, 62 migrations, latest `20261116213417_automation_active_index`), Claude Code transcripts on the same machine |

---

## 1. Summary

lmtop watches two coding agents today: Codex CLI and Claude Code. OpenCode
is a third agent that talks to many model backends: OpenAI (via ChatGPT
subscription OAuth or API key), Anthropic, OpenRouter, local models, and
Claude through the Claude Code harness (`claude-agent`). Most of that
usage is invisible to lmtop.

This PRD adds a read-only **OpenCode collector**. It reads OpenCode's
SQLite database, normalizes each assistant message's token usage into
lmtop's domain types, and shows OpenCode as a first-class provider:
sessions, calendar-week totals, model breakdown, rate history. It does
this without double counting usage that Claude Code's transcripts already
record.

## 2. Problem

### 2.1 What is broken today

Observed on this machine (2026-11-20):

| OpenCode backend (`providerID`) | Where the usage lands | Visible in lmtop? |
|---|---|---|
| `claude-agent` (Claude via the Claude Code harness) | `~/.claude/projects/<slug>/<uuid>.jsonl` **and** `opencode.db` | ✅ through the Claude collector |
| `openai` (ChatGPT OAuth, e.g. `gpt-6.2-sol`) | `opencode.db` only | ❌ tokens and sessions missing |
| Anything else (OpenRouter, Anthropic API key, local) | `opencode.db` only | ❌ missing |

Evidence:

- The gpt-6.2-sol response `resp_0839312e…` (2026-11-19 18:13 JST) is in
  `opencode.db`. A search of `~/.codex/sessions` and
  `~/.codex/archived_sessions` for its response and message ids returns
  nothing, and no Codex rollout directory exists for that day. OpenCode
  calls OpenAI directly; Codex never sees the traffic.
- The OpenAI credential row in `opencode.db` is `type: oauth`,
  `methodID: chatgpt…`, so this traffic draws on the same ChatGPT plan
  as the Codex CLI.

### 2.2 Why it matters

lmtop is a capacity-planning tool. Today a user who works partly in
OpenCode on a ChatGPT plan sees:

- **Quota (with `--live`): correct.** The ChatGPT usage endpoint is
  account-level.
- **Quota (without `--live`): stale.** The local snapshots refresh only
  when the Codex CLI runs.
- **Observed tokens, sessions, week totals, model mix: silently
  incomplete.** Nothing on screen marks the gap.

The last point violates the working agreement "surface the gap rather
than papering over it". The dashboard looks complete when it is not.

## 3. Goals and non-goals

### Goals

- **G1.** Every token OpenCode records against a non-mirrored backend
  appears in lmtop's observed-token views: sessions table, calendar
  week, model breakdown, rate chart, `snapshot --json`, `line`.
- **G2.** Nothing is counted twice. Usage already captured by another
  collector (today: `claude-agent` → Claude Code transcripts) is
  excluded by default, and the exclusion is visible in `doctor`.
- **G3.** The token semantics match the domain contract in
  `src/domain/tokens.rs` exactly (§6.3).
- **G4.** The collector only reads, never blocks OpenCode, never reads
  conversation content, and degrades to *unavailable* on schema drift
  instead of guessing.
- **G5.** The collector has the same health, capability and doctor
  surface as the existing collectors.

### Non-goals

- **Quota for OpenCode itself.** OpenCode has no quota of its own. Plan
  quota stays with the Codex/Claude panels (§7.4 covers a pointer only).
- **Cost.** OpenCode stores a `cost` field. lmtop still does not do cost
  (README: "not implemented"), and this PRD does not change that.
- **Legacy OpenCode storage.** Pre-SQLite JSON storage
  (`~/.local/share/opencode/storage/…`) is out of scope.
- **Writing to OpenCode, the OpenCode server API, or SSE event streams.**
  File-based reads only, like the other collectors.
- **Reattributing OpenCode tokens into the Codex or Claude panels.**
  See §5 for why.

## 4. Users and scenarios

- **S1. Mixed workflow on one ChatGPT plan.** Edward uses Codex CLI in
  one terminal and gpt-6.2-sol in OpenCode in another. He wants this
  week's GPT tokens from both, side by side, while the Codex panel's
  quota bar tells him whether the plan will last.
- **S2. OpenCode with Claude through the harness.** Usage must keep
  showing under Claude exactly once. Adding the collector must not
  inflate the Claude numbers.
- **S3. OpenCode with a third-party backend** (OpenRouter, local Ollama).
  Tokens and sessions appear under OpenCode with the upstream named.
  Quota shows as unavailable.
- **S4. OpenCode not installed.** The collector reports *unavailable*
  ("no OpenCode database found"). No errors, no empty panel noise beyond
  what Codex/Claude show in the same situation.

## 5. Key product decision: OpenCode is its own provider

`Provider` is documented as "an AI coding agent product whose usage we
monitor" (`src/domain/mod.rs`). OpenCode is an agent product, so it gets
`Provider::OpenCode`.

Rejected alternative: fold OpenCode's `openai` traffic into the Codex
panel and its Anthropic traffic into the Claude panel. This is tempting
because they share a plan, but:

- The Codex and Claude panels' observed tokens are defined as "what the
  Codex CLI / Claude Code recorded". Mixing sources makes each number
  harder to trace back to a file, which is the codebase's main promise.
- Upstream ≠ plan. `openai` may be an API key, not ChatGPT OAuth. Folding
  would mean guessing which plan a request billed against.
- A per-upstream split is still available inside the OpenCode panel
  (model breakdown, session upstream column), and the combined view
  already shows all providers side by side.

The plan relationship is surfaced as a pointer instead of a merge (§7.4).

## 6. Data source specification

### 6.1 Location and discovery

1. `providers.opencode.db_path`, if configured.
2. `$XDG_DATA_HOME/opencode/opencode.db`, if `XDG_DATA_HOME` is set.
3. `~/.local/share/opencode/opencode.db` (Linux default, observed).

> **Verify before release:** OpenCode uses XDG paths on macOS too
> (`~/.local/share/opencode`) rather than `~/Library/Application
> Support`. Confirm on a Mac. If wrong, add the macOS path as a
> candidate. Windows: `%LOCALAPPDATA%\opencode` (unverified, best
> effort).

The database runs in WAL mode (`opencode.db-wal`, `opencode.db-shm`
present). Open it exactly the way `codex_db.rs` opens the Codex state DB:
`SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`, `busy_timeout(100 ms)`.
Every failure degrades to `None` → health *degraded*, never a panic and
never a wait on OpenCode's write lock.

### 6.2 Tables and fields consumed

Only the fields below are read. **Conversation content (`content`,
`session_part.data`, `summary_diffs`, `title`) is never selected.** JSON
fields are extracted with `json_extract` *inside SQL*, so prompt and
response bodies never enter lmtop's memory. The same discipline applies
to the `credential` table (§7.4): only `integration_id` and
`json_extract(value,'$.type')` / `'$.methodID'` are selected. Token
material is never read.

**`session_v2`** (one row per session)

| Column | Use |
|---|---|
| `id` | session id (`ses_…`) |
| `parent_id` | roll child sessions into their root (§6.5) |
| `directory` | project name = basename only (`store::project_name`) |
| `time_created` (ms) | `started_at` |
| `time_updated` (ms) | fallback `last_activity` |
| `time_archived` | diagnostics only; archived sessions still count (§6.5) |
| `version` | OpenCode version for `doctor` |
| `tokens_input`, `tokens_output`, `tokens_reasoning`, `tokens_cache_read`, `tokens_cache_write` | reconciliation only (§6.6), never the primary source |

**`session_message`** with `type = 'assistant'` (one row per model response)

| JSON path in `data` | Use |
|---|---|
| row `id` | dedup key |
| row `session_id` | session |
| row `time_created` / `time_updated` (ms) | incremental cursor |
| `$.model.providerID` | upstream (`openai`, `claude-agent`, …); exclusion filter |
| `$.model.id` | raw model id → `ModelIdentity::normalize` |
| `$.model.variant` | reasoning effort candidate (§6.4) |
| `$.time.completed` (ms) | event timestamp; message is counted only once set |
| `$.finish` | `stop` / `tool-calls` / `error` / null (in flight) |
| `$.tokens.input` / `.output` / `.reasoning` / `.cache.read` / `.cache.write` | usage |

Other message types observed (`user`, `idle`, `model-switched`) are not
needed. Model switches are already reflected per assistant message.
`session_part` is empty on this install and is not read.

**Schema fingerprint.** Before scanning, check that `session_v2` and
`session_message` exist with the columns above (`PRAGMA table_info`). On
mismatch, report health *unavailable*:
`unsupported OpenCode schema (latest migration <id>)`. Don't partially
parse. The latest `migration.id` is reported in `doctor` either way.

### 6.3 Token semantics (verified)

These were verified request by request: 11 consecutive `claude-agent`
messages from this session in `opencode.db` were matched against the
same API requests in the Claude Code transcript
(`~/.claude/projects/-home-ubuntu-projects-lmtop/31d6ea37-….jsonl`):

| OpenCode | Claude transcript | Relationship |
|---|---|---|
| `input: 4` | `input_tokens: 4` | equal → OpenCode `input` **excludes** cache |
| `cache.read: 34435` | `cache_read_input_tokens: 34435` | equal |
| `cache.write: 997` | `cache_creation_input_tokens: 997` | equal |
| `output: 405`, `reasoning: 222` | `output_tokens: 627` | **405 + 222 = 627** → OpenCode `output` **excludes** reasoning |

The same identity held for all 11 (e.g. 727+555=1282, 258+311=569,
194+226=420, 765+7=772).

Domain mapping (`TokenCounts`):

| Domain field | From OpenCode |
|---|---|
| `input` | `tokens.input` |
| `cached_input` | `tokens.cache.read` |
| `cache_creation` | `tokens.cache.write` |
| `output` | `tokens.output + tokens.reasoning` (domain `output` includes reasoning) |
| `reasoning` | `tokens.reasoning` (informational subset) |
| `unattributed` | 0 per message (see §6.6 for the session residual) |
| `other` | unknown keys under `tokens` ending in a number, preserved (forward compatibility) |

The mapping matches the domain contract. Getting `output` wrong would
undercount reasoning-heavy models by up to ~2× (e.g. 258 vs 569 above).
That is why it is pinned by a test (§10).

> **Verify per upstream before release:** the identity above was proven
> for `claude-agent`. For `openai`, the only samples have `reasoning: 0`
> and `cache.read: 0`, so the exclusion semantics are inferred
> (OpenCode normalizes AI SDK usage the same way for all providers) but
> not yet proven. Capture one reasoning-heavy, cache-hitting gpt-6.2-sol
> turn and cross-check it against the `codex/usage` delta or the OpenAI
> response `usage` before shipping. Record the result in
> `docs/data-sources.md`.

### 6.4 Session field mapping

| `SessionUsage` field | Source |
|---|---|
| `provider` | `Provider::OpenCode` |
| `id` | root session id |
| `model` | model of the latest counted assistant message |
| `reasoning_effort` | `$.model.variant` when it is one of `minimal`/`low`/`medium`/`high`/`xhigh`; `default` or absent → unavailable (never inherited from an earlier message, same rule as Codex) |
| `project` | basename of `session_v2.directory` |
| `started_at` | `session_v2.time_created` |
| `last_activity` | max `time.completed` of counted messages, else `session_v2.time_updated` |
| `tokens`, `tokens_by_model` | sum of counted messages |
| `context_tokens` | latest completed message: `input + cache.read + cache.write` |
| `context_window` | **unavailable.** `opencode.db` stores no model limits and `~/.cache/opencode` has no models catalog on this install. Shown as unavailable, never guessed. |
| `tokens_per_minute` | from events, as for other providers |
| **new** `upstream: Option<String>` | `providerID` of the latest counted message (e.g. `openai`). Added to `SessionUsage` with `#[serde(default)]`. `None` for Codex/Claude. |

### 6.5 Inclusion rules

- **Completed only.** A message is counted once `time.completed` is set
  and `tokens` is present. In-flight messages (`finish: null`, no
  tokens) are skipped and picked up on a later scan.
- **Errors.** `finish: "error"` messages are counted if they carry
  non-zero tokens (the request was billed), otherwise skipped and
  counted in `doctor` as `failed_messages`.
- **Excluded upstreams.** Messages whose `providerID` is in
  `skip_upstreams` (default `["claude-agent"]`) are not counted. The
  number and token total skipped per upstream are reported in `doctor`.
  A session whose messages are all skipped is not shown in the sessions
  table. Its usage is shown by the Claude collector instead.
- **Archived sessions count.** `time_archived` only hides a session in
  OpenCode's UI; the tokens were spent. (Observed:
  `ses_e0cebcd09…` was archived 0.1 s after creation and holds 13,632
  real input tokens.)
- **Child sessions roll up.** Messages of a session with `parent_id` are
  attributed to the root session, matching how Claude subagent
  transcripts roll into their parent. (0 child sessions on this install;
  covered by fixture.)
- **Mixed-upstream sessions.** A session may switch models mid-way
  (`model-switched` messages). Rules apply per message, so only the
  non-skipped part of a session is counted.

### 6.6 Session-total reconciliation (auxiliary calls)

`session_v2.tokens_*` is consistently **larger** than the sum of its
assistant messages:

| Session | `session_v2` in/out | Σ messages in/out | Residual |
|---|---|---|---|
| `ses_e0ceaaacb…` (openai) | 14052 / 18 | 13615 / 13 | 437 / 5 |
| `ses_e09b94583…` (claude-agent) | 40 / 6104 | 38 / 6088 | 2 / 16 |
| `ses_e0d10f2b1…` (claude-agent) | 4 / 20 | 2 / 14 | 2 / 6 |

The likely cause is auxiliary model calls (session title generation,
compaction) that OpenCode bills to the session without writing an
assistant message. Their model and upstream are not recorded.

Rule (keeps both "no guesses" and "no double counting"):

- If **every** message in the session belongs to counted upstreams, the
  residual (per category, saturating at zero) is recorded as one
  session-level event with `model = None`, in its known directions. It
  appears in the session total and the week total, and in the model
  breakdown as **"auxiliary (model unknown)"**.
- If the session contains **any** skipped upstream, the residual is
  **not counted**, because it may already be in the Claude transcripts.
  It is reported in `doctor` and in the session detail popup as
  "N auxiliary tokens not attributed (may be counted under Claude)".
- The residual event's timestamp is `session_v2.time_updated`.

This is open question Q1. If review prefers, the simpler v1 is "report
but never count".

### 6.7 Incremental scanning

The DB is small but grows without bound, so scans must stay cheap:

- **Query:** assistant messages with
  `time_created >= retention_cutoff` (uses
  `session_message_time_created_idx`), selecting only the extracted
  columns from §6.2.
- **Per-message ledger:** `HashMap<message_id, TokenCounts>` of the last
  counted value. A message seen again with different tokens records only
  `saturating_delta(new, old)`, so a message whose tokens are filled in
  late is handled. Unchanged messages are a no-op.
- **High-water mark:** `max(time_updated)` seen. A scan whose result set
  has no `time_updated` above it skips the rebuild entirely.
- **Session rows:** fetched only for session ids that appear in the
  result set, plus their roots.
- **Ledger trimming:** entries older than the retention cutoff are
  dropped together with `UsageStore::trim`.

Scans run at the normal refresh interval on the blocking pool, like the
other collectors.

## 7. Product surface

### 7.1 Provider plumbing

- `Provider::OpenCode` (serde `"opencode"`, display `"OpenCode"`),
  added to `Provider::ALL` and `FromStr`.
- `--provider opencode` works. The `--provider` filter in
  `src/cli.rs::run` gains the OpenCode line.
- Theme: `opencode` / `opencode_dim` colors in every palette
  (`src/tui/theme.rs`), ANSI color in `line` output (`src/cli.rs`).
- `persist.rs`: history lines gain a new `p` value. Old files without
  it still parse.

### 7.2 Capabilities declared

| Capability | Declared |
|---|---|
| `local_token_usage` | ✅ |
| `active_session` | ✅ |
| `calendar_week_aggregation` | ✅ |
| `model_breakdown` | ✅ |
| `history` | ✅ |
| `provider_quota` | ❌ (no quota of its own) |
| `reset_times` | ❌ |
| `credits` | ❌ |

### 7.3 Views

- **Combined (`3`)** and **Planner (`4`)** include OpenCode when it is
  enabled and available. Planner has no quota windows for OpenCode, so it
  shows only the week-pacing row, or omits OpenCode if that row needs
  quota.
- **Dedicated view:** add `View::OpenCode` on key **`5`**. Keys `1`–`4`
  keep their current meanings, so existing muscle memory is unaffected.
  Footer hints and help gain one entry.
- **Sessions table:** OpenCode rows use the provider color. The detail
  popup shows `upstream`, and the auxiliary-token note from §6.6 when it
  applies.
- **Header freshness:** OpenCode appears alongside Codex/Claude.

### 7.4 Plan pointer (P1)

When a counted message's upstream is `openai` **and** the `credential`
row for `integration_id = 'openai'` has `type = 'oauth'` with a ChatGPT
`methodID`, the OpenCode panel's quota area reads:

> Quota: shared ChatGPT plan, see Codex panel (`--live` for freshness)

Same for `anthropic` + OAuth → "see Claude panel". In every other case it
reads "Quota: unavailable (no plan quota for this backend)". Nothing is
merged, and no percentages are moved between panels.

### 7.5 `doctor`

New OpenCode block:

```
OpenCode
  database:          ~/.local/share/opencode/opencode.db (found, read-only OK)
  opencode version:  2.3.2   schema: 20261116213417_automation_active_index
  sessions (48 h):   16
  messages counted:  3 (openai: 3)
  messages skipped:  44 (claude-agent: 44, mirrored in Claude Code transcripts)
  failed messages:   1
  auxiliary tokens:  counted 442 · not counted 18 (mixed with skipped upstream)
  quota:             none (see Codex panel for ChatGPT plan)
```

Paths go through `diagnostics::redacted_dirs`, like the existing blocks.

### 7.6 `snapshot --json` and `line`

- `providers.opencode` appears with the normalized snapshot. Sessions
  carry `upstream`.
- `line` adds an OpenCode segment (week tokens) only when the collector
  is available, so status bars without OpenCode stay unchanged.

## 8. Configuration

New section (its own struct, since OpenCode has no `session_paths` or
`network_quota`):

```toml
[providers.opencode]
enabled = true
# db_path = "~/.local/share/opencode/opencode.db"
skip_upstreams = ["claude-agent"]
```

- `enabled` defaults to `true`. The collector reports *unavailable*
  when no database exists, so this is free for non-users.
- `skip_upstreams`: upstreams whose usage another collector already
  records. Set it to `[]` to count everything (e.g. Claude Code
  collector disabled).
- If `providers.claude.enabled = false`, `claude-agent` is **not**
  auto-removed from the skip list. lmtop should not quietly change what
  it counts; `doctor` prints a hint instead.
- `docs/configuration.md` reference block updated. The
  `documented_reference_config_matches_the_code` test gains assertions
  for the new defaults.

## 9. Privacy and safety

`docs/privacy.md` gains an OpenCode section stating:

- The database is opened read-only, with a 100 ms busy timeout, and is
  never written to.
- Only the columns and JSON paths listed in §6.2 are selected, via
  `json_extract` in SQL. Message content, parts, diffs, titles, and
  credential values never enter process memory.
- From `credential`, only `integration_id`, the auth `type` and
  `methodID` are read (P1, §7.4). Tokens and refresh tokens are never
  selected.
- `directory` is reduced to its basename before it is stored.

## 10. Testing

Fixtures are built in-test as temporary SQLite files (pattern:
`codex_db.rs` tests). No real `opencode.db` is committed.

Unit tests (`src/collectors/opencode.rs`):

1. **Token mapping identity:** `{input 4, output 405, reasoning 222,
   cache {read 34435, write 997}}` → `output == 627`, `reasoning ==
   222`, `total == 4 + 34435 + 997 + 627`.
2. `claude-agent` messages are skipped by default and counted when
   `skip_upstreams = []`.
3. In-flight message (no `time.completed`, no tokens) is ignored, then
   counted once completed on the next scan, with no double count.
4. A message whose tokens change between scans records only the delta.
5. `finish: error` with zero tokens is skipped; with tokens it is counted.
6. Archived session still counts toward the week.
7. Child session (`parent_id`) rolls into its root.
8. Mid-session model switch: `tokens_by_model` splits correctly, and
   `model` is the latest one.
9. Residual rule (§6.6): counted when all messages are counted, not
   counted when any upstream is skipped.
10. Schema drift: a missing `session_message` table or column gives
    *unavailable* with a message and no panic. A non-SQLite file gives
    *degraded*.
11. DB locked by a writer (open a write transaction in the test) →
    scan returns within the timeout and degrades.
12. `variant` → `reasoning_effort` only for known effort values.
13. `content` is never selected (assert on the SQL text constant).

Integration (`tests/collectors.rs`): a whole collector over a fixture DB
with a deterministic `ScanContext`, checking the week aggregate, history
buckets and sessions ordering.

Gate: `cargo test && cargo clippy --all-targets && cargo fmt --check`.

## 11. Rollout

| Milestone | Scope | Exit criteria |
|---|---|---|
| **M1. Collector core** | `opencode.rs` reader, mapping, ledger, exclusion, health, `Provider::OpenCode`, config | Tests 1–8, 10–13 pass. `snapshot --json` shows correct OpenCode totals on this machine. |
| **M2. Surface** | Views (`5`), theme, sessions `upstream`, header, `line`, `doctor` block | Manual check in the TUI. Footer width tests still pass. |
| **M3. Reconciliation + plan pointer** | §6.6 residual, §7.4 pointer | Test 9. Doctor numbers match a hand SQL query. |
| **M4. Docs** | `data-sources.md`, `privacy.md`, `architecture.md`, `configuration.md`, README capability table + OpenCode column | Config doc test passes. |

Version: **0.4.0** (minor bump for a new provider) in the M1 commit, with
patch bumps for each later `src/` change, per CLAUDE.md. No tag until
Edward decides to release.

## 12. Acceptance criteria

1. On this machine, `lmtop snapshot --json` reports OpenCode week
   tokens equal to a hand query of the sum of non-`claude-agent`
   completed assistant messages since week start, plus the counted
   residual. Exact equality, no tolerance.
2. Claude week totals are **identical** before and after enabling the
   OpenCode collector.
3. With OpenCode uninstalled (DB path absent), lmtop starts with no
   errors and OpenCode shows *unavailable*.
4. Running a gpt-6.2-sol turn in OpenCode makes it appear in the lmtop
   sessions table within one refresh interval after the response
   completes.
5. Corrupting or locking the DB never stalls or crashes lmtop and never
   blocks OpenCode.
6. `rg -n "content" src/collectors/opencode.rs` shows no SQL selecting
   content.

## 13. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| OpenCode schema churn (62 migrations already, v2 tables) | High | Fingerprint check, report the migration id, degrade to *unavailable*. Pin the verified version in `data-sources.md`. |
| Token semantics differ for some upstreams | Medium | Per-upstream verification note (§6.3). `other` map keeps unknown keys. |
| `claude-agent` stops writing Claude transcripts in a future harness | Low | `doctor` compares skipped claude-agent message count with Claude collector sessions in the same window and warns on mismatch. Config lets the user flip `skip_upstreams`. |
| Same Claude usage also reached through `anthropic` OAuth with Claude Code running | Low | Not mirrored: OpenCode calls the API directly, so it is counted once, under OpenCode. Documented. |
| WAL read-only open fails when the directory is not writable | Low | Degrade with a clear message, per G4. |
| UI crowding with 3–4 providers | Medium | Combined layout already handles Custom. Verify at 80×24 in M2. |

## 14. Open questions

- **Q1.** Count the auxiliary residual (§6.6) in v1, or report only?
  *Recommendation: count it under the safe rule; report-only is the
  fallback.*
- **Q2.** Should `View::OpenCode` exist, or is the combined view enough?
  *Recommendation: add it on `5`. It is cheap and consistent with
  `1`/`2`.*
- **Q3.** Should an `upstream` filter (`/upstream:openai`) join the
  sessions filter? *Recommendation: defer to a follow-up.*
- **Q4.** macOS and Windows database paths need confirmation on real
  installs (§6.1).
- **Q5.** Should OpenCode's `cost` field ever be shown? It is provider
  pricing, not plan consumption. *Recommendation: no, consistent with
  the README's stance on cost.*
