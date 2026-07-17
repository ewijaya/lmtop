# Architecture

`agentop` is a local-first terminal dashboard that monitors Codex CLI and
Claude Code usage. The design has one central rule: **provider collection is
completely separate from rendering.** The UI depends only on normalized
domain types; nothing outside a collector ever sees a provider schema.

## Layers

```text
┌────────────────────────────────────────────────────────────┐
│ tui/            rendering, layout, theme, key handling     │
│ app.rs          central UI state (latest snapshots)        │
├────────────────────────────────────────────────────────────┤
│ domain/         normalized types (the only shared language)│
├────────────────────────────────────────────────────────────┤
│ aggregation/    week windows, history buckets, rates       │
│ collectors/     codex.rs, claude.rs, incremental JSONL     │
│ diagnostics/    doctor, redaction                          │
│ config.rs       TOML config with full defaults             │
└────────────────────────────────────────────────────────────┘
```

### Domain (`src/domain/`)

Normalized types shared by every layer:

- `Provider` — Codex or Claude.
- `ModelIdentity` — raw model id (always preserved) + normalized display
  name + `ModelFamily` (Fable, Opus, Sonnet, Haiku, GPT, Other). Unknown and
  future models are grouped under `Other`, never dropped.
- `TokenCounts` — input, cached input, cache-creation input, output,
  reasoning, plus a map of unknown categories kept for forward
  compatibility. **Displayed totals include cached input**
  (`total = input + cached_input + cache_creation + output`); reasoning is
  an informational subset of output.
- `SessionUsage` — per-session usage metadata (never prompt bodies).
- `WeekAggregate` — calendar-week totals with a per-model breakdown.
- `QuotaWindow` / `QuotaWindowKind` — provider-reported rolling quota
  windows, classified by duration (~300 min → five-hour, ~10 080 min →
  weekly, anything else → Unknown). Classification never relies on array
  order, and a missing window never causes another window to be relabeled.
- `Credits`, `CollectorHealth`, `Freshness`, `HistorySample`, `Capability`.
- `UsageSnapshot` / `ProviderSnapshot` — the complete state handed to the
  UI and to `snapshot --json`.

Three concepts are kept deliberately distinct and labeled distinctly in the
UI:

1. **Observed tokens** — computed from local session metadata.
2. **Provider quota** — percentages reported by the provider itself.
3. **Estimated API cost** — intentionally not implemented in the MVP; it
   would be an optional, clearly-hypothetical add-on.

Quota percentages are never converted into token counts, and observed
tokens are never presented as quota consumption.

### Collectors (`src/collectors/`)

`CodexCollector` and `ClaudeCollector` implement one trait:

```rust
pub trait Collector: Send {
    fn provider(&self) -> Provider;
    fn scan(&mut self, ctx: &ScanContext) -> ProviderSnapshot;
}
```

`ScanContext` carries the clock (`now`), the calendar-week bounds, and the
history retention — time is always injected, so tests run on deterministic
clocks.

Each collector owns:

- **Discovery** — finding session files under the provider's data
  directory (plus configured extra paths), filtered by mtime so only files
  that can contribute to the current week or session view are read.
- **Incremental ingestion** — `JsonlTail` tracks a byte offset per file;
  a grown file has only its new bytes parsed, truncation/rotation resets
  the cursor, and a partial trailing line is left unconsumed until the
  writer finishes it. Corrupt lines increment a parse-error counter and are
  skipped; they never abort a scan.
- **Cumulative→delta conversion** (Codex) — Codex token counters are
  cumulative per session; the collector diffs consecutive snapshots,
  saturating at zero per category.
- **Deduplication** (Claude) — Claude Code writes one line per content
  block and duplicates `usage` across them; usage is counted once per
  unique API request. Forked/resumed sessions that inherit history lines
  are deduplicated by the same key, and active/archived copies of the same
  session collapse together.
- **Capability reporting** — each collector declares exactly what it can
  provide (`local_token_usage`, `provider_quota`, `credits`, …). The UI
  renders missing capabilities as "unavailable", not as errors.

Both collectors feed a shared `UsageStore`, which accumulates deduplicated
delta events plus per-session records and assembles `ProviderSnapshot`s via
the aggregation layer.

### Aggregation (`src/aggregation/`)

Pure functions over normalized events:

- `week_bounds` — calendar week in the user's display timezone
  (configurable week start; "local" or a fixed UTC offset). A provider's
  rolling 7-day quota window is *not* a calendar week and is never labeled
  as one.
- `build_week` — totals plus per-model breakdown for events inside the
  week bounds.
- `build_history` — per-minute throughput buckets for the rate chart, with
  explicit zero samples so gaps render honestly.
- `tokens_per_minute` — recent per-session rate.

### Runtime

```text
main ──> tokio runtime ──> collector loop (one task per provider)
  │                            │  scan() on spawn_blocking, every N secs
  │                            ▼
  │                    mpsc channel of ProviderSnapshot
  ▼                            │
TUI thread (synchronous) ◄─────┘
  event::poll(tick) → keys/resize → App
  terminal.draw(App) several times per second
```

- The UI may redraw several times per second, but **filesystem scans run
  only at the configured refresh interval** (default 5 s) on a blocking
  thread, decoupled from render frequency.
- Pause (`p`) stops collector scans via a shared atomic; refresh (`r`)
  wakes them early via a `Notify`.
- A collector failure degrades only that provider's panel
  (`CollectorHealth`), never the application.
- Terminal state is restored on normal exit, quit keys, panics (ratatui's
  panic hook), and errors.

### Storage decision

The MVP keeps history **in memory only**. The rate chart needs the last
hour, which is reconstructed on startup from event timestamps already
present in the session files, so a database adds no user-visible value yet.
`rusqlite` was deliberately left out; long-horizon persistent history is a
possible follow-up and would live behind `storage/`.

Filesystem watching (`notify`) was also left out deliberately: interval
scanning with per-file offsets is cheap (only grown files are re-read),
works over network mounts and WSL, and keeps the collector loop simple.

## Testing strategy

- Sanitized synthetic fixtures under `tests/fixtures/` for both providers —
  no real session data is committed.
- Unit tests co-located with each module (week math, DST, normalization,
  quota classification, incremental reads, dedup).
- Integration tests drive whole collectors over fixture directories with a
  deterministic `ScanContext` clock.
