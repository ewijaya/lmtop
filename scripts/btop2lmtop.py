#!/usr/bin/env python3
"""Convert btop .theme files into lmtop theme TOML.

Usage: btop2lmtop.py FILE.theme [FILE.theme ...] -o OUTDIR

btop themes define terminal-monitor roles (cpu gradients, box outlines);
lmtop needs semantic roles (good/warn/bad, per-provider and per-model
accents) that btop does not have. Structural roles map directly; semantic
roles are picked from the theme's accent pool by nearest hue, so the
result keeps the theme's character but is derived, not designed. The
generated header says so.

btop color syntax: "#RRGGBB", "#XX" (grayscale), or "" (terminal default).
"""

import argparse
import colorsys
import re
import sys
from pathlib import Path

LINE = re.compile(r'theme\[([a-z_]+)\]\s*=\s*"([^"]*)"')

# lmtop semantic roles -> target hue in degrees. The converter picks the
# pool color closest to the target; duplicates across roles are fine (the
# curated builtin palettes reuse colors the same way).
HUE_TARGETS = {
    "good": 120,   # green
    "warn": 40,    # amber
    "bad": 0,      # red
    "codex": 210,  # blue
    "claude": 30,  # orange
    "custom": 165, # teal
    "fable": 285,  # purple
    "opus": 30,
    "sonnet": 55,
    "haiku": 110,
    "gpt": 210,
}

# Keys whose colors render on the main background, so they are safe accent
# candidates. selected_bg and the box outlines are excluded: backgrounds
# and near-background outlines have no contrast guarantee.
POOL_KEYS = [
    "hi_fg", "title", "proc_misc",
    "cpu_start", "cpu_mid", "cpu_end",
    "temp_start", "temp_mid", "temp_end",
    "free_start", "free_mid", "free_end",
    "used_start", "used_mid", "used_end",
    "cached_start", "cached_mid", "cached_end",
    "available_start", "available_mid", "available_end",
    "download_start", "download_mid", "download_end",
    "upload_start", "upload_mid", "upload_end",
]


def parse_color(value):
    value = value.strip()
    if not value.startswith("#"):
        return None
    hex_part = value[1:]
    if len(hex_part) == 6:
        return tuple(int(hex_part[i : i + 2], 16) for i in (0, 2, 4))
    if len(hex_part) == 2:  # btop grayscale shorthand
        v = int(hex_part, 16)
        return (v, v, v)
    return None


def parse_theme(path):
    colors = {}
    for match in LINE.finditer(path.read_text()):
        rgb = parse_color(match.group(2))
        if rgb is not None:
            colors[match.group(1)] = rgb
    return colors


def luminance(rgb):
    return (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]) / 255.0


def hue_sat(rgb):
    h, _l, s = colorsys.rgb_to_hls(*(c / 255.0 for c in rgb))
    return h * 360.0, s


def hue_distance(a, b):
    d = abs(a - b) % 360.0
    return min(d, 360.0 - d)


def score(rgb, target_hue, bg, text):
    """Hue distance to target, with penalties for anything that would make
    the accent fail at its job: weak saturation (a hard gate at gray,
    graded above it — near-white "colors" like #f8f8f2 must not win a
    semantic role), weak luminance contrast against the background (a
    right-hued but invisible accent is no accent), and closeness to the
    theme's text color (a warn that looks like body text warns nobody)."""
    hue, sat = hue_sat(rgb)
    penalty = 1000.0 if sat < 0.2 else max(0.0, 0.45 - sat) * 400.0
    penalty += max(0.0, 0.25 - abs(luminance(rgb) - luminance(bg))) * 300.0
    if max(abs(a - b) for a, b in zip(rgb, text)) < 24:
        penalty += 150.0
    return hue_distance(hue, target_hue) + penalty


def pick(pool, target_hue, bg, text):
    return min(pool, key=lambda rgb: score(rgb, target_hue, bg, text))


def pick_trio(pool, roles, bg, text):
    """Jointly choose colors for a trio of sibling roles (good/warn/bad,
    the provider accents). Near-duplicate hues within a trio would make
    the roles indistinguishable, so closeness costs more than a worse hue
    match — and the assignment is optimized globally rather than greedily,
    so a theme's only red lands on `bad`, not whichever role picked first.
    """
    import itertools

    def cost(combo):
        total = sum(score(rgb, HUE_TARGETS[role], bg, text) for rgb, role in zip(combo, roles))
        hues = [hue_sat(rgb)[0] for rgb in combo]
        for a, b in itertools.combinations(hues, 2):
            if hue_distance(a, b) < 18.0:
                total += 200.0
        return total

    return min(itertools.product(pool, repeat=len(roles)), key=cost)


def mix(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def to_hex(rgb):
    return "#{:02x}{:02x}{:02x}".format(*rgb)


def convert(colors, name):
    for key in ("main_fg", "inactive_fg", "div_line", "hi_fg"):
        if key not in colors:
            raise ValueError(f"{name}: missing required key {key}")
    bg = colors.get("main_bg")
    light = luminance(bg) > 0.5 if bg else False
    if bg is None:  # transparent theme: assume a dark terminal
        bg = (30, 30, 30)

    pool = list({colors[k] for k in POOL_KEYS if k in colors})
    if not pool:
        raise ValueError(f"{name}: no accent colors found")

    out = {"light": light}
    out["text"] = colors["main_fg"]
    out["dim"] = colors["inactive_fg"]
    out["border"] = colors["div_line"]
    out["border_focused"] = colors["hi_fg"]
    text = colors["main_fg"]
    for trio in [("good", "warn", "bad"), ("codex", "claude", "custom")]:
        for role, rgb in zip(trio, pick_trio(pool, trio, bg, text)):
            out[role] = rgb
    for role, hue in HUE_TARGETS.items():
        if role not in out:  # model families may reuse hues freely
            out[role] = pick(pool, hue, bg, text)
    for provider in ("codex", "claude", "custom"):
        out[f"{provider}_dim"] = mix(out[provider], bg, 0.55)
    out["other_model"] = mix(colors["main_fg"], colors["inactive_fg"], 0.5)
    return out


FIELD_ORDER = [
    "text", "dim", "border", "border_focused",
    "good", "warn", "bad",
    "codex", "codex_dim", "claude", "claude_dim", "custom", "custom_dim",
    "fable", "opus", "sonnet", "haiku", "gpt", "other_model",
]


def emit_toml(out, name):
    lines = [
        f'# Converted from the btop theme "{name}"',
        "# (https://github.com/aristocratos/btop/tree/main/themes).",
        "# Structural colors map directly; the good/warn/bad and",
        "# provider/model accents are derived mechanically by nearest hue,",
        "# not hand-curated. Tune freely.",
        f"light = {'true' if out['light'] else 'false'}",
    ]
    lines += [f'{field} = "{to_hex(out[field])}"' for field in FIELD_ORDER]
    return "\n".join(lines) + "\n"


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("themes", nargs="+", type=Path)
    ap.add_argument("-o", "--outdir", type=Path, required=True)
    args = ap.parse_args()
    args.outdir.mkdir(parents=True, exist_ok=True)
    failures = 0
    for path in args.themes:
        name = path.stem
        try:
            toml = emit_toml(convert(parse_theme(path), name), name)
        except ValueError as err:
            print(f"skip: {err}", file=sys.stderr)
            failures += 1
            continue
        (args.outdir / f"{name}.toml").write_text(toml)
        print(f"{name}.toml")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
