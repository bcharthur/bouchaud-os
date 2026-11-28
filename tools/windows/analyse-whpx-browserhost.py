#!/usr/bin/env python3
"""Report the scroll phase and the three paired differences in a WHPX campaign."""

import argparse
from collections import Counter
import json
from pathlib import Path
import re
from statistics import median, quantiles


JS = re.compile(r'(\d+\.\d+) WebContent\((\d+)\): \(js log\) "WHPX_P0_(BEGIN|SCROLL|CALLBACK)\b')
FRAME = re.compile(
    r"\[LB:FRAME\] onglet=(\d+) seq=(\d+) t=(\d+) .*?"
    r"publie=(-?\d+),(-?\d+) (\d+)x(\d+) copie_px=(\d+) complet=[01] present_us=(\d+)"
)


def stats(values):
    percentiles = quantiles(values, n=100, method="inclusive")
    return dict(p50=median(values), p95=percentiles[94], p99=percentiles[98], max=max(values))


def analyse(path, historical=False):
    raw = path.read_text(encoding="utf-8-sig", errors="replace")
    log = re.sub(r"\x1b\[[0-9;]*m", "", raw)
    markers = {}
    for line in log.splitlines():
        match = JS.search(line)
        if match:
            tick, pid, kind = match.groups()
            if kind in markers:
                raise ValueError(f"{path}: duplicate {kind}")
            data = json.JSONDecoder().raw_decode(line[match.end():].lstrip())[0] if kind != "BEGIN" else None
            markers[kind] = (round(float(tick) * 1000), pid, data)
    if not {"SCROLL", "CALLBACK"} <= markers.keys() or len({m[1] for m in markers.values()}) != 1:
        raise ValueError(f"{path}: missing markers or inconsistent WebContent PID")
    stop, _, scroll = markers["SCROLL"]
    _, _, callback = markers["CALLBACK"]
    raf = scroll["frames"]
    duration = scroll["end"] - scroll["click"]
    if (len(raf) != 240 or callback["count"] != 240 or callback["scrollOnly"]
            or not scroll["click"] <= raf[0] <= raf[-1] <= scroll["end"]
            or any(b < a for a, b in zip(raf, raf[1:])) or duration <= 0):
        raise ValueError(f"{path}: expected 240 callbacks with the DOM counter enabled")
    if "BEGIN" not in markers and not historical:
        raise ValueError(f"{path}: WHPX_P0_BEGIN missing")
    start = markers["BEGIN"][0] if "BEGIN" in markers else stop - duration
    if not 0 < start < stop:
        raise ValueError(f"{path}: invalid guest time window")

    frames = [tuple(map(int, match.groups())) for match in FRAME.finditer(log)]
    frames = [frame for frame in frames if start <= frame[2] < stop]
    sequences = [frame[1] for frame in frames]
    if (len(frames) < 2 or len({frame[0] for frame in frames}) != 1
            or sequences != list(range(sequences[0], sequences[-1] + 1))):
        raise ValueError(f"{path}: incomplete, duplicate, or multi-tab BrowserHost trace")
    raf_intervals = [b - a for a, b in zip(raf, raf[1:])]
    rects = Counter(f"{x},{y} {w}x{h}" for _, _, _, x, y, w, h, _, _ in frames)
    return {
        "phase": "guest-marker-window" if "BEGIN" in markers else "estimated-from-scroll-duration",
        "guest_window_ms": [start, stop],
        "scroll_ms": duration,
        "callbacks_per_s": len(raf) * 1000 / duration,
        "raf_ms": stats(raf_intervals),
        "raf_intervals_ge_35ms": sum(value >= 35 for value in raf_intervals),
        "browserhost_interval_ms": stats([b[2] - a[2] for a, b in zip(frames, frames[1:])]),
        "callback_total_ms": callback["callbackTotalMs"],
        "browserhost_count": len(frames),
        "present_us": stats([f[-1] for f in frames]),
        "published_rects": dict(rects),
        "published_pixels": sum(f[5] * f[6] for f in frames),
        "copied_pixels": sum(f[-2] for f in frames),
        "browserhost_seq": [sequences[0], sequences[-1]],
    }


def metrics(result):
    flat = {k: result[k] for k in ("scroll_ms", "callbacks_per_s", "callback_total_ms",
                                  "browserhost_count", "copied_pixels", "published_pixels", "raf_intervals_ge_35ms")}
    for group in ("raf_ms", "present_us", "browserhost_interval_ms"):
        flat.update({f"{group}.{k}": v for k, v in result[group].items()})
    return flat


ORDER = ("A", "B", "B", "A", "A", "B")
COMMON = ("boot_sha256", "fixture_sha256", "qemu_sha256", "launcher_sha256", "url", "profile")
ARM = ("checkout", "run_id", "browserhost_sha256", "webcontent_sha256", "manifest_sha256")


def campaign(directory):
    trials = []
    for i, arm in enumerate(ORDER, 1):
        trial = directory / f"{i:02d}-{arm}"
        info = json.loads((trial / "evidence.json").read_text(encoding="utf-8-sig"))
        if info["arm"] != arm:
            raise ValueError(f"{trial}: invalid arm")
        trials.append((arm, info, analyse(trial / "serial.log")))
    for keys, subset in [(COMMON, trials), *[(ARM, [t for t in trials if t[0] == arm]) for arm in "AB"]]:
        if any(len({t[1][key] for t in subset}) != 1 for key in keys):
            raise ValueError(f"campaign: inconsistent evidence for {', '.join(keys)}")
    if trials[0][1]["browserhost_sha256"] == trials[1][1]["browserhost_sha256"]:
        raise ValueError("campaign: identical BrowserHost across both arms")

    deltas = []
    for first, second in zip(trials[::2], trials[1::2]):
        a, b = sorted((first, second), key=lambda t: t[0])
        base, candidate = metrics(a[2]), metrics(b[2])
        deltas.append({k: (candidate[k] / v - 1) * 100 if v else None for k, v in base.items()})
    dispersion = {}
    for key in deltas[0]:
        values = [row[key] for row in deltas]
        if all(value is not None for value in values):
            mid = median(values)
            dispersion[key] = dict(median=mid, min=min(values), max=max(values),
                                   mad=median(abs(v - mid) for v in values))
    return {"trials": [{"arm": arm, "metrics": metrics} for arm, _, metrics in trials],
            "paired_candidate_relative_pct": deltas, "paired_dispersion_pct": dispersion}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--historical-log", type=Path)
    mode.add_argument("--campaign", type=Path)
    args = parser.parse_args()
    try:
        result = campaign(args.campaign) if args.campaign else analyse(args.historical_log, historical=True)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(1, f"Comparaison non qualifiable: {error}\n")
    print(json.dumps(result, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
