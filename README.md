# agentop

> **Naming note:** `agentop` is a temporary internal codename. The package,
> binary, and docs are structured for a clean rename (see `src/branding.rs`).

A fast, local-first terminal dashboard for monitoring **Codex CLI** and
**Claude Code** usage — `btop`, but for AI coding agents.

Built for people on **flat-rate ChatGPT/Codex and Claude subscriptions**
who need capacity planning, not billing: How much of my 5-hour window is
left? Will my weekly quota survive until it resets? Which model is eating
my week?

```text
┌─ CODEX ────────────────────────────┬─ CLAUDE ───────────────────────────┐
│ 5h      ██████████░░░  67.0% ↻2h05m ⚠ empty ~1h40m                      │
│ Weekly  █████░░░░░░░░  39.2% ↻3d 4h ✓ lasts                             │
│ Credits 188                        │ Weekly (Fable) ████████░░░  52%    │
│ Observed session 1.2M · week 45.6M │ Observed session 890k · week 12.3M │
├─ TOKEN RATE (tokens/min, observed) ┴────────────────────────────────────┤
│        ⢀⣴⣧⡀       Codex in/out · Claude in/out                          │
├─ SESSIONS ──────────────────────────────────────────────────────────────┤
│ Provider  Model      Context  Tok/min  Tokens  Project     Age   State  │
├─ WEEKLY USAGE (observed) ──────────┬─ MODEL BREAKDOWN (week) ───────────┤
│ in / cached / cache-write / out    │ Fable 5   ███████  45%             │
└────────────────────────────────────┴────────────────────────────────────┘
```

*(screenshot placeholder — real capture pending)*

## What it shows

Three concepts, kept strictly separate:

| Concept | Source | Meaning |
|---|---|---|
| **Observed tokens** | local session metadata | what your agents actually consumed, by session / calendar week / model |
| **Provider quota** | provider-reported percentages | authoritative subscription window usage (5-hour, weekly), with reset times |
| **Estimated API cost** | — | not implemented; would be hypothetical only |

Flat-rate providers apply hidden weights, caching rules, and model
multipliers — so observed tokens are **never** converted into quota
percentages, and quota percentages are never converted into token counts.

On top of the provider-reported quota trend it computes **burn velocity**
(percentage points per hour) and a **projected exhaustion time**, and
answers the question that matters: *will this window run out before it
resets?* (`✓ lasts` / `⚠ empty ~1h40m` / `✗ exhausted`).

It also distinguishes **calendar weeks** (Monday-start by default,
configurable, local timezone) from provider **rolling quota windows** — a
"weekly" quota window is a rolling 7 days, not your calendar week, and is
never labeled as one.

## Supported providers and capabilities

| Capability | Codex | Claude Code |
|---|---|---|
| local_token_usage | ✅ | ✅ |
| active_session | ✅ | ✅ |
| calendar_week_aggregation | ✅ | ✅ |
| model_breakdown | ✅ | ✅ |
| provider_quota | ✅ (from local rate-limit snapshots) | ✅ (from Claude Code's local quota cache) |
| credits | ✅ (when reported) | ❌ |
| reset_times | ✅ | ✅ |
| model-scoped limits | ❌ (not reported) | ✅ (e.g. a per-model weekly cap) |
| history | ✅ | ✅ |

Unavailable capabilities are shown as *unavailable*, never invented. See
`docs/data-sources.md` for exactly where each number comes from and
`docs/privacy.md` for what is never read.

## Install / build

Stable Rust required ([rustup](https://rustup.rs)):

```bash
git clone <repo-url>
cd agentop
cargo build --release
./target/release/agentop
```

## Usage

```bash
agentop                     # combined dashboard
agentop --provider codex    # Codex only
agentop --provider claude   # Claude only
agentop --offline           # never touch the network
agentop --refresh 5         # rescan every 5 seconds
agentop --ascii             # ASCII bars/charts
agentop snapshot            # one-shot text summary (non-interactive)
agentop snapshot --json     # machine-readable snapshot
agentop doctor              # discovery, parse health, capabilities
agentop --version
```

## Keyboard shortcuts

```text
1        Codex view              w      Focus weekly usage
2        Claude view             h      Focus history chart
3        Combined view           j/k ↓↑ Scroll sessions
Tab      Change focused panel    r      Refresh now
s        Focus sessions          p      Pause / resume collectors
m        Focus model breakdown   ?      Help
                                 q/Esc  Quit
```

## Flat-rate subscription limitations

Honesty section — read this before trusting any number:

- **Codex quota** comes from rate-limit snapshots that the Codex CLI writes
  into its own session logs. They are authoritative but only as fresh as
  your last Codex activity; if you haven't used Codex for hours, the quota
  shown is hours old (freshness is displayed).
- **Claude quota** comes from Claude Code's own cached quota view in
  `~/.claude.json`, refreshed only while Claude Code itself is running —
  same staleness caveat as above, and the age is displayed. If the cache is
  missing, quota is marked *unavailable* rather than guessed.
- **Observed tokens ≠ quota consumption.** Providers weight models, cached
  tokens, and request overhead differently and don't publish the formula.
- **Burn velocity is an extrapolation** of the provider's own recent
  percentages (a linear trend of the current monotonic run). It's a
  planning aid, not a promise.
- Quota windows are classified by their reported duration (~300 min → 5h,
  ~10 080 min → weekly). Windows with unrecognized durations are shown as
  `Window (Nm)` — unknown, but visible.

## Privacy

Local-first: no network calls, no telemetry, no API keys, no credential
reads, no prompt content — collectors parse token counts and identifiers
from session metadata and nothing else. Full model: `docs/privacy.md`.

## Platform status

| Platform | Status |
|---|---|
| Linux | primary — developed and smoke-tested here |
| macOS | expected to work (crossterm + platform dirs); untested |
| Windows | expected to work; untested |
| WSL / SSH | works — plain terminal I/O, ASCII fallback available |

## Documentation

- `docs/architecture.md` — layers, data flow, design decisions
- `docs/data-sources.md` — exact provider schemas consumed
- `docs/privacy.md` — the privacy contract
- `docs/configuration.md` — config file reference
- `CONTRIBUTING.md` — development guide

## License

MIT — see `LICENSE`.
