#!/usr/bin/env python3
from pathlib import Path
import re
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: analyse-p0-ng1.py <serial.log>")

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8", errors="replace")

patterns = {
    "lockdep": r"\[LOCKDEP\].*",
    "preempt": r"\[SCHED-NG-PREEMPT\].*",
    "latency": r"\[SCHED-NG-LAT\].*",
    "heap": r"\[MEM-NG-HEAP\].*",
    "frames": r"\[MEM-NG-FRAMECACHE\].*",
    "pressure": r"\[MEM-NG-PRESSURE\].*",
    "pagecache": r"\[MEM-NG-PAGECACHE\].*",
}

missing = []
for name, pat in patterns.items():
    hits = re.findall(pat, text)
    print(f"{name}: {hits[-1] if hits else 'no sample'}")
    if not hits:
        missing.append(name)

fatal_patterns = [
    r"\*\*\* KERNEL PANIC \*\*\*",
    r"(?i)double fault",
    r"(?i)LOCKDEP inversion",
    r"(?i)deadlock",
    r"\[BKL-FR\]\s+VIOLATION",
    r"(?i)smp_lock: release par un CPU non proprietaire",
    r"(?i)smp_lock: release par une continuation non proprietaire",
]
fatal_hits = []
for pat in fatal_patterns:
    fatal_hits.extend(re.findall(pat, text))
print(f"fatal_markers={len(fatal_hits)}")

# Print the last panic assertion/violation lines so a failed capture is
# actionable without manually opening a multi-megabyte serial log.
panic_lines = [
    line for line in text.splitlines()
    if ("KERNEL PANIC" in line
        or "panicked at " in line
        or "smp_lock:" in line
        or "[BKL-FR] VIOLATION" in line
        or "LOCKDEP inversion" in line)
]
if panic_lines:
    print("fatal_context:")
    for line in panic_lines[-16:]:
        print("  " + line)

holds = [int(x) for x in re.findall(r"\[BKL-MAX-HOLD\].*?ns=(\d+)", text)]
if holds:
    print(f"bkl_max_hold_ns={max(holds)}")
else:
    print("bkl_max_hold_ns=no sample")

lat = [int(x) for x in re.findall(r"\[SCHED-NG-LAT\].*?interactive_max_ns=(\d+)", text)]
if lat:
    print(f"interactive_ready_to_run_max_ns={max(lat)}")

# `max_defer_ns` est le report SUBI : du premier refus d'un point sur au
# service. `attente_service_max_ns` est l'ecart demande->service, inactivite
# comprise -- diagnostic seul, il grandit sur un coeur au repos.
pre = [int(x) for x in re.findall(r"\[SCHED-NG-PREEMPT\].*?max_defer_ns=(\d+)", text)]
if pre:
    print(f"kernel_preempt_defer_max_ns={max(pre)}")

att = [int(x) for x in
       re.findall(r"\[SCHED-NG-PREEMPT\].*?attente_service_max_ns=(\d+)", text)]
if att:
    print(f"kernel_preempt_attente_service_max_ns={max(att)}")

repairs = [int(x) for x in re.findall(r"\[BKL-NG1\.1\].*?stale_self_repairs=(\d+)", text)]
mismatches = [int(x) for x in re.findall(r"\[BKL-NG1\.1\].*?identity_mismatches=(\d+)", text)]
if repairs:
    print(f"bkl_stale_self_repairs={max(repairs)}")
if mismatches:
    print(f"bkl_identity_mismatches={max(mismatches)}")

if fatal_hits:
    print("verdict=FAIL")
    sys.exit(2)
if missing:
    print("missing_samples=" + ",".join(missing))
    print("verdict=OBSERVE")
    sys.exit(1)
print("verdict=PASS-RUNTIME-NO-FATAL")
