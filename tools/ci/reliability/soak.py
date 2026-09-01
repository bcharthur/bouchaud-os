#!/usr/bin/env python3
"""Repeated QEMU boot campaign for bounded or long-running soak tests."""

from __future__ import annotations

import argparse
import json
import time
from pathlib import Path

import qemu_matrix

def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("bootimage", type=Path)
    parser.add_argument("--cpus", type=int, default=4)
    parser.add_argument("--duration-seconds", type=int, required=True)
    parser.add_argument("--cycle-seconds", type=int, default=120)
    parser.add_argument("--memory-mb", type=int, default=4096)
    parser.add_argument("--out-dir", type=Path, default=Path("reliability-soak"))
    parser.add_argument("--require", action="append", default=[])
    args = parser.parse_args()

    if args.duration_seconds <= 0 or args.cycle_seconds <= 0:
        raise SystemExit("les durees doivent etre positives")

    qemu = qemu_matrix.qemu_binary()
    started = time.monotonic()
    deadline = started + args.duration_seconds
    cycles = []
    index = 0

    while time.monotonic() < deadline:
        index += 1
        cycle_dir = args.out_dir / f"cycle-{index:04d}"
        remaining = max(1, int(deadline - time.monotonic()))
        seconds = min(args.cycle_seconds, remaining)
        result = qemu_matrix.run_one(
            qemu, args.bootimage, args.cpus, seconds, args.memory_mb, cycle_dir, args.require
        )
        cycles.append(result)
        print(f"cycle {index}: {'OK' if result['ok'] else 'ECHEC'}")
        if not result["ok"]:
            break

    payload = {
        "schema": 1,
        "requested_seconds": args.duration_seconds,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "cycles": cycles,
        "ok": bool(cycles) and all(c["ok"] for c in cycles),
    }
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "summary.json").write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return 0 if payload["ok"] else 1

if __name__ == "__main__":
    raise SystemExit(main())
