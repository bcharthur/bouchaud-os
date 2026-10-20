#!/usr/bin/env python3
"""Garde hôte KVM pour campagne P13 dédiée. Refuse toute preuve simulée.

Vérifie le nombre de CPU alloués, quota cgroup, droits KVM et outils. Ne
prétend pas mesurer le temps volé : seul le journal invité peut le faire.
"""
from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path


def quota_cpus(text: str) -> float | None:
    words = text.strip().split()
    if len(words) != 2:
        return 0.0
    if words[0] == "max":
        return None
    try:
        quota, period = map(int, words)
        return quota / period if period > 0 else 0.0
    except ValueError:
        return 0.0


def controle(allowed: int, quota: float | None, min_cpus: int, kvm: bool, tools: list[str], dedicated: bool, load: float, max_load_ratio: float = 1.5) -> list[str]:
    errors = []
    if not dedicated:
        errors.append("hôte non explicitement dédié (BOUCHAUD_DEDICATED_HOST=YES requis)")
    if allowed < min_cpus:
        errors.append(f"affinité CPU insuffisante : {allowed} < {min_cpus}")
    if quota is not None and quota < min_cpus:
        errors.append(f"quota CPU insuffisant : {quota:g} < {min_cpus}")
    if not kvm:
        errors.append("/dev/kvm inexistant ou inaccessible en lecture/écriture")
    if tools:
        errors.append("outils absents : " + ", ".join(tools))
    if allowed > 0 and load > allowed * max_load_ratio:
        errors.append(f"charge hôte élevée au contrôle : {load:.2f} > {allowed * max_load_ratio:.2f}")
    return errors


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--min-cpus", type=int, default=8)
    ap.add_argument("--output", type=Path, default=Path("p13-host-preflight.json"))
    ns = ap.parse_args()
    if ns.min_cpus < 1:
        ap.error("--min-cpus doit être positif")
    affinity = sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else []
    p = Path("/sys/fs/cgroup/cpu.max")
    quota = quota_cpus(p.read_text()) if p.exists() else None
    required = ["qemu-system-x86_64", "python3", "bash", "gcc", "git"]
    missing = [t for t in required if not shutil.which(t)]
    kvm = os.path.exists("/dev/kvm") and os.access("/dev/kvm", os.R_OK | os.W_OK)
    load = os.getloadavg()[0] if hasattr(os, "getloadavg") else 0.0
    dedicated = os.environ.get("BOUCHAUD_DEDICATED_HOST") == "YES"
    errors = controle(len(affinity), quota, ns.min_cpus, kvm, missing, dedicated, load)
    try:
        sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True, stderr=subprocess.DEVNULL).strip()
    except (OSError, subprocess.CalledProcessError):
        sha = None
        errors.append("Git HEAD absent, impossible de relier les preuves à un SHA")
    info = {"sha": sha, "uname": platform.uname()._asdict(), "allowed_cpu_ids": affinity,
            "allowed_count": len(affinity), "cpu_quota": quota, "host_load_1m": load,
            "kvm_accessible": kvm, "dedicated_confirmed_by_operator": dedicated,
            "minimum_cpu": ns.min_cpus, "errors": errors, "status": "PRECHECK_OK" if not errors else "PRECHECK_REFUSED"}
    ns.output.parent.mkdir(parents=True, exist_ok=True)
    ns.output.write_text(json.dumps(info, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(info["status"] + " sha=" + str(sha) + " allowed_cpus=" + str(len(affinity)) + " quota=" + str(quota))
    for error in errors:
        print("  - " + error)
    return 0 if not errors else 2


if __name__ == "__main__":
    sys.exit(main())
