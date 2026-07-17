# Demo fixture

Synthetic session data used to produce the dashboard screenshot in the
top-level `README.md`. **Not used by any test** — the sets in `../claude/`
and `../codex/` pin collector behaviour and should not be changed for
cosmetic reasons.

Those test fixtures are deliberately lopsided (one session's token counts
dwarf the rest, which is exactly what some of them exist to check), so the
rate chart renders as a single spike. This set instead models a plausible
working hour: six overlapping sessions across both providers, at comparable
magnitudes, so the chart shows real variation.

Everything here is invented: no real projects, paths, or usage.

## Regenerating

Timestamps are relative to an anchor time, because the rate chart only keeps
the last 60 minutes — fixtures with fixed timestamps render an empty chart.
Regenerate immediately before capturing:

```bash
python3 tests/fixtures/demo/generate.py --now "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
```

The generator uses a fixed random seed, so the same anchor time always
produces identical output.

## Capturing the screenshot

Point a real run at this data via an isolated `HOME`, so the collectors'
default paths (`~/.codex/sessions`, `~/.claude/projects`) resolve here and
never touch your real session logs:

```bash
DEMO=$(mktemp -d)
mkdir -p "$DEMO/.codex" "$DEMO/.claude"
cp -r tests/fixtures/demo/codex/sessions "$DEMO/.codex/sessions"
cp -r tests/fixtures/demo/claude        "$DEMO/.claude/projects"
cp    tests/fixtures/demo/claude.json   "$DEMO/.claude.json"

env HOME="$DEMO" COLORTERM=truecolor ./target/release/lmtop
```

Note that `session_paths` in `config.toml` *adds* to the default locations
rather than replacing them, so an isolated `HOME` is the reliable way to keep
real usage out of a published capture.

Capture at **140x40** or wider: the footer's keybind help and the
right-aligned freshness readout collide on narrower terminals.
