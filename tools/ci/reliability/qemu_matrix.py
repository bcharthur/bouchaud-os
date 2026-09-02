#!/usr/bin/env python3
"""Boot one Bouchaud OS image under several SMP topologies and scan every journal."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

from logscan import scan_file

def qemu_binary() -> str:
    for name in ("qemu-system-x86_64", "qemu-system-x86_64.exe"):
        found = shutil.which(name)
        if found:
            return found
    raise SystemExit("qemu-system-x86_64 introuvable dans PATH")

def run_one(qemu: str, bootimage: Path, cpus: int, seconds: int, memory_mb: int,
            out_dir: Path, required: list[str]) -> dict:
    out_dir.mkdir(parents=True, exist_ok=True)
    log = out_dir / f"smp{cpus}.log"
    if log.exists():
        log.unlink()

    command = [
        qemu,
        "-drive", f"format=raw,file={bootimage}",
        "-m", str(memory_mb),
        "-smp", str(cpus),
        "-cpu", "max",
        "-display", "none",
        "-serial", f"file:{log}",
        "-no-reboot",
        "-netdev", "user,id=net0",
        "-device", "e1000,netdev=net0",
        "-audiodev", "none,id=muet",
        "-device", "AC97,audiodev=muet",
    ]

    timed_out = False
    returncode = None
    try:
        completed = subprocess.run(command, timeout=seconds, check=False)
        returncode = completed.returncode
    except subprocess.TimeoutExpired:
        timed_out = True

    scan = scan_file(log, required_markers=required)
    qemu_exit_ok = timed_out or returncode in (0, None)
    ok = qemu_exit_ok and scan.ok
    return {
        "cpus": cpus,
        "seconds": seconds,
        "timed_out": timed_out,
        "returncode": returncode,
        "qemu_exit_ok": qemu_exit_ok,
        "log": str(log),
        "log_bytes": scan.bytes,
        "fatal_findings": [
            {"kind": f.kind, "line": f.line, "text": f.text}
            for f in scan.findings
        ],
        "required_markers_missing": list(scan.required_markers_missing),
        "ok": ok,
    }

def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("bootimage", type=Path)
    parser.add_argument("--cpus", nargs="+", type=int, default=[1, 2, 4, 8])
    parser.add_argument("--seconds", type=int, default=45)
    parser.add_argument("--memory-mb", type=int, default=2048)
    parser.add_argument("--out-dir", type=Path, default=Path("reliability-qemu"))
    parser.add_argument("--require", action="append", default=[])
    args = parser.parse_args()

    if not args.bootimage.is_file():
        print(f"bootimage absent: {args.bootimage}", file=sys.stderr)
        return 2
    if any(c < 1 or c > 64 for c in args.cpus):
        print("nombre de CPU invalide", file=sys.stderr)
        return 2

    qemu = qemu_binary()
    results = [
        run_one(qemu, args.bootimage, cpus, args.seconds, args.memory_mb, args.out_dir, args.require)
        for cpus in args.cpus
    ]
    payload = {"schema": 1, "bootimage": str(args.bootimage), "ok": all(r["ok"] for r in results), "runs": results}
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "summary.json").write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    for result in results:
        state = "OK" if result["ok"] else "ECHEC"
        print(f"{state} SMP{result['cpus']} log={result['log_bytes']} octets timeout={result['timed_out']} rc={result['returncode']}")
        for finding in result["fatal_findings"]:
            print(f"  {finding['kind']}:{finding['line']}: {finding['text']}")
        for marker in result["required_markers_missing"]:
            print(f"  marqueur absent: {marker}")

    return 0 if payload["ok"] else 1

if __name__ == "__main__":
    raise SystemExit(main())
