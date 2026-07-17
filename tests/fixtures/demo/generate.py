#!/usr/bin/env python3
"""Regenerate the demo fixture used for the README screenshot.

The other fixture sets exist to pin collector behaviour and are deliberately
lopsided (one session dwarfs the rest), which makes the rate chart render as a
single spike. This set instead models a plausible working hour: several
overlapping sessions of comparable magnitude across both providers.

Timestamps are anchored relative to a --now argument so the capture can place
the activity inside the chart's 60-minute retention window. Content is fully
synthetic: no real projects, paths, or usage.

Usage:
    python3 tests/fixtures/demo/generate.py --now 2026-07-17T12:00:00Z
"""
import argparse
import json
import math
import pathlib
import random
import shutil
from datetime import datetime, timedelta, timezone

ROOT = pathlib.Path(__file__).parent


def ts(t):
    return t.strftime("%Y-%m-%dT%H:%M:%S.") + f"{t.microsecond // 1000:03d}Z"


def burst(minute, centre, width, height):
    """A smooth activity hump: agents ramp up, peak, then wind down."""
    return height * math.exp(-((minute - centre) ** 2) / (2 * width**2))


# Each session: a project, a model, and a burst shape (centre minute, width,
# peak tokens/min). Magnitudes are within ~3x of each other so no single
# session flattens the chart's y-axis.
CLAUDE_SESSIONS = [
    ("gamma", "claude-fable-5", 12, 6, 52_000),
    ("delta", "claude-opus-4-8", 26, 7, 78_000),
    ("lambda", "claude-sonnet-5", 41, 6, 45_000),
    ("hop", "claude-fable-5", 50, 5, 61_000),
]

CODEX_SESSIONS = [
    ("alpha", "gpt-5.6-terra", 18, 7, 40_000),
    ("beta", "gpt-5.5", 34, 6, 33_000),
]


def gen_claude(now, rng):
    out = ROOT / "claude"
    for proj, model, centre, width, height in CLAUDE_SESSIONS:
        d = out / f"-home-user-projects-{proj}"
        d.mkdir(parents=True, exist_ok=True)
        sid = f"sess-{proj}"
        lines = []
        start = now - timedelta(minutes=58)
        lines.append(json.dumps({
            "parentUuid": None, "isSidechain": False, "userType": "external",
            "cwd": f"/home/user/projects/{proj}", "sessionId": sid,
            "version": "2.1.212", "gitBranch": "main", "type": "user",
            "uuid": f"u-{proj}-0", "timestamp": ts(start + timedelta(minutes=centre - width)),
            "message": {"role": "user", "content": "[synthetic demo fixture]"},
        }))
        # One API request per minute across the burst; usage is per-request.
        for i, m in enumerate(range(max(centre - width * 2, 0), min(centre + width * 2, 57))):
            rate = burst(m, centre, width, height)
            if rate < height * 0.04:
                continue
            jitter = rng.uniform(0.82, 1.18)
            total = int(rate * jitter)
            out_tok = max(int(total * 0.06), 40)
            cache_read = int(total * 0.78)
            cache_create = int(total * 0.10)
            inp = max(total - out_tok - cache_read - cache_create, 10)
            at = start + timedelta(minutes=m, seconds=rng.randint(0, 45))
            rid = f"req-{proj}-{i}"
            lines.append(json.dumps({
                "parentUuid": f"u-{proj}-{i}", "isSidechain": False,
                "userType": "external", "cwd": f"/home/user/projects/{proj}",
                "sessionId": sid, "version": "2.1.212", "gitBranch": "main",
                "type": "assistant", "uuid": f"u-{proj}-msg-{i}",
                "timestamp": ts(at), "requestId": rid,
                "message": {
                    "id": f"msg-{proj}-{i}", "type": "message", "role": "assistant",
                    "model": model,
                    "usage": {
                        "input_tokens": inp,
                        "cache_creation_input_tokens": cache_create,
                        "cache_read_input_tokens": cache_read,
                        "output_tokens": out_tok,
                        "service_tier": "standard",
                    },
                },
            }))
        (d / f"{sid}.jsonl").write_text("\n".join(lines) + "\n")


def gen_codex(now, rng):
    day = now.strftime("%Y/%m/%d")
    d = ROOT / "codex" / "sessions" / day
    d.mkdir(parents=True, exist_ok=True)
    start = now - timedelta(minutes=58)
    for proj, model, centre, width, height in CODEX_SESSIONS:
        lines = []
        s0 = start + timedelta(minutes=max(centre - width * 2, 0))
        lines.append(json.dumps({
            "timestamp": ts(s0), "type": "session_meta",
            "payload": {"id": f"codex-{proj}", "timestamp": ts(s0),
                        "cwd": f"/home/user/projects/{proj}",
                        "originator": "codex_cli_rs", "cli_version": "0.99.0",
                        "source": "cli", "thread_source": "user"},
        }))
        lines.append(json.dumps({
            "timestamp": ts(s0 + timedelta(seconds=20)), "type": "turn_context",
            "payload": {"model": model, "effort": "high", "summary": "auto",
                        "cwd": f"/home/user/projects/{proj}"},
        }))
        # Codex counters are CUMULATIVE; the collector diffs consecutive
        # snapshots, so these must only ever increase.
        cum = {"input_tokens": 0, "cached_input_tokens": 0, "output_tokens": 0,
               "reasoning_output_tokens": 0}
        for m in range(max(centre - width * 2, 0), min(centre + width * 2, 57)):
            rate = burst(m, centre, width, height)
            if rate < height * 0.04:
                continue
            total = int(rate * rng.uniform(0.82, 1.18))
            cum["input_tokens"] += max(int(total * 0.18), 5)
            cum["cached_input_tokens"] += int(total * 0.74)
            cum["output_tokens"] += max(int(total * 0.08), 20)
            cum["reasoning_output_tokens"] += int(total * 0.01)
            at = start + timedelta(minutes=m, seconds=rng.randint(0, 45))
            tot = dict(cum, total_tokens=cum["input_tokens"] + cum["output_tokens"])
            lines.append(json.dumps({
                "timestamp": ts(at), "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {"total_token_usage": tot, "last_token_usage": tot,
                             "model_context_window": 258400},
                    "rate_limits": {
                        "limit_id": "codex", "limit_name": None,
                        "primary": {"used_percent": 42.0, "window_minutes": 10080,
                                    "resets_at": int((now + timedelta(days=3, hours=4)).timestamp())},
                        "secondary": {"used_percent": round(18 + m * 0.85, 1),
                                      "window_minutes": 300,
                                      "resets_at": int((now + timedelta(hours=3, minutes=12)).timestamp())},
                        "credits": {"balance": 188.5}, "individual_limit": None,
                        "plan_type": "plus", "rate_limit_reached_type": None,
                    },
                },
            }))
        stamp = s0.strftime("%Y-%m-%dT%H-%M-%S")
        (d / f"rollout-{stamp}-{proj}.jsonl").write_text("\n".join(lines) + "\n")


def gen_quota(now):
    """Claude's local quota cache (~/.claude.json shape).

    The per-model weekly cap comes from the `limits` array (weekly_scoped +
    scope.model.display_name), not from `seven_day_opus`.
    """
    five = (now + timedelta(hours=3, minutes=11)).isoformat(timespec="microseconds")
    week = (now + timedelta(days=3, hours=5)).isoformat(timespec="microseconds")
    data = {
        "cachedUsageUtilization": {
            "fetchedAtMs": int((now - timedelta(minutes=2)).timestamp() * 1000),
            "utilization": {
                "five_hour": {"utilization": 61, "resets_at": five,
                              "limit_dollars": None, "used_dollars": None},
                "seven_day": {"utilization": 35, "resets_at": week},
                "seven_day_opus": None,
                "limits": [
                    {"kind": "session", "group": "session", "percent": 61,
                     "severity": "ok", "resets_at": five, "scope": None,
                     "is_active": True},
                    {"kind": "weekly_all", "group": "weekly", "percent": 35,
                     "severity": "ok", "resets_at": week, "scope": None,
                     "is_active": False},
                    {"kind": "weekly_scoped", "group": "weekly", "percent": 52,
                     "severity": "warning", "resets_at": week,
                     "scope": {"model": {"id": None, "display_name": "Fable"},
                               "surface": None},
                     "is_active": True},
                ],
            },
        }
    }
    (ROOT / "claude.json").write_text(json.dumps(data, indent=1) + "\n")


def clean():
    """Remove previously generated data so re-runs cannot leave stale files.

    Rollout filenames embed the anchor time, so a second run with a different
    --now would otherwise add duplicates alongside the first run's output.
    """
    for sub in ("claude", "codex"):
        shutil.rmtree(ROOT / sub, ignore_errors=True)
    (ROOT / "claude.json").unlink(missing_ok=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--now", required=True, help="anchor time, e.g. 2026-07-17T12:00:00Z")
    args = ap.parse_args()
    now = datetime.fromisoformat(args.now.replace("Z", "+00:00")).astimezone(timezone.utc)
    rng = random.Random(20260717)  # fixed seed: regenerating is reproducible
    clean()
    gen_claude(now, rng)
    gen_codex(now, rng)
    gen_quota(now)
    print(f"demo fixture regenerated, anchored at {ts(now)}")


if __name__ == "__main__":
    main()
