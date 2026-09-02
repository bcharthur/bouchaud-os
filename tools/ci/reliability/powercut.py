#!/usr/bin/env python3
"""Power-cut / crash-recovery campaign for a raw persistence disk.

The tool never mutates the seed image.  Every iteration copies it, boots with
writeback caching, kills QEMU at a randomized instant, then reboots the exact
same mutated copy and scans the recovery journal.

A filesystem-specific autorun may additionally expose a recovery marker through
--require-recovery.  Without that marker the campaign still proves the minimum
contract: the recovered kernel emits serial output and does not panic/fault.
"""

from __future__ import annotations

import argparse
import json
import random
import shutil
import subprocess
import time
from pathlib import Path

from logscan import scan_file
from qemu_matrix import qemu_binary

def spawn(qemu: str, bootimage: Path, disk: Path, cpus: int, memory_mb: int, log: Path):
    command = [
        qemu,
        "-drive", f"format=raw,file={bootimage}",
        "-drive", f"format=raw,file={disk},cache=writeback",
        "-m", str(memory_mb),
        "-smp", str(cpus),
        "-cpu", "max",
        "-display", "none",
        "-serial", f"file:{log}",
        "-no-reboot",
    ]
    return subprocess.Popen(command)

def kill_after(proc: subprocess.Popen, seconds: float):
    deadline = time.monotonic() + seconds
    while proc.poll() is None and time.monotonic() < deadline:
        time.sleep(0.05)
    if proc.poll() is None:
        proc.kill()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.terminate()
        proc.wait(timeout=5)

def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("bootimage", type=Path)
    parser.add_argument("seed_disk", type=Path)
    parser.add_argument("--iterations", type=int, default=8)
    parser.add_argument("--cut-min-ms", type=int, default=250)
    parser.add_argument("--cut-max-ms", type=int, default=3000)
    parser.add_argument("--recovery-seconds", type=int, default=20)
    parser.add_argument("--cpus", type=int, default=4)
    parser.add_argument("--memory-mb", type=int, default=2048)
    parser.add_argument("--seed", type=int, default=0xB0C4A0D)
    parser.add_argument("--require-recovery", action="append", default=[])
    parser.add_argument("--out-dir", type=Path, default=Path("reliability-powercut"))
    args = parser.parse_args()

    if not args.bootimage.is_file() or not args.seed_disk.is_file():
        raise SystemExit("bootimage ou seed disk absent")
    if args.cut_min_ms < 1 or args.cut_max_ms < args.cut_min_ms:
        raise SystemExit("fenetre de coupure invalide")

    qemu = qemu_binary()
    rng = random.Random(args.seed)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    results = []

    for iteration in range(1, args.iterations + 1):
        work = args.out_dir / f"disk-{iteration:03d}.img"
        shutil.copyfile(args.seed_disk, work)
        crash_log = args.out_dir / f"crash-{iteration:03d}.log"
        recovery_log = args.out_dir / f"recovery-{iteration:03d}.log"

        cut_ms = rng.randint(args.cut_min_ms, args.cut_max_ms)
        proc = spawn(qemu, args.bootimage, work, args.cpus, args.memory_mb, crash_log)
        kill_after(proc, cut_ms / 1000.0)

        proc = spawn(qemu, args.bootimage, work, args.cpus, args.memory_mb, recovery_log)
        kill_after(proc, float(args.recovery_seconds))

        scan = scan_file(recovery_log, required_markers=args.require_recovery)
        item = {
            "iteration": iteration,
            "cut_ms": cut_ms,
            "disk": str(work),
            "recovery_log": str(recovery_log),
            "recovery_log_bytes": scan.bytes,
            "fatal_findings": [
                {"kind": f.kind, "line": f.line, "text": f.text} for f in scan.findings
            ],
            "required_markers_missing": list(scan.required_markers_missing),
            "ok": scan.ok,
        }
        results.append(item)
        print(f"{'OK' if item['ok'] else 'ECHEC'} powercut {iteration}/{args.iterations} @ {cut_ms}ms")
        if not item["ok"]:
            break

    payload = {"schema": 1, "seed": args.seed, "ok": bool(results) and all(r["ok"] for r in results), "runs": results}
    (args.out_dir / "summary.json").write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return 0 if payload["ok"] else 1

if __name__ == "__main__":
    raise SystemExit(main())
