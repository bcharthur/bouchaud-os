#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path

RE_DIAG = re.compile(
    r"\[IDLE-DIAG\].*bsp_safe=(\d+).*pit_ticks=(\d+).*pit_age_ns=(\d+).*idle_mask=(0x[0-9a-fA-F]+)"
)
RE_CPU = re.compile(
    r"\[IDLE-CPU\] cpu=(\d+) phase=(\d+)\(([^)]+)\) phase_age_ns=(\d+) "
    r"idle=(\d+) idle_age_ns=(\d+) seq=(\d+) "
    r"sched=(\d+)/(\d+)/(\d+)/safe(\d+) "
    r"lock=(\d+)/(\d+)/(\d+)/safe(\d+) "
    r"wfi=(\d+)/(\d+)/safe(\d+) sleep_max_ns=(\d+)"
)
RE_STALL = re.compile(r"\[SMP-STALL\]")
RE_WATCHDOG = re.compile(r"\[sched-watchdog\].*desktop sans heartbeat depuis (\d+) ms")

def main() -> int:
    if len(sys.argv) != 2:
        print("usage: analyse-idle-v6.py <log>")
        return 2

    text = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
    last_diag = None
    cpus = {}
    stalls = 0
    wm_max = 0

    for line in text.splitlines():
        if m := RE_DIAG.search(line):
            last_diag = tuple(m.groups())
        if m := RE_CPU.search(line):
            cpus[int(m.group(1))] = m.groups()
        if RE_STALL.search(line):
            stalls += 1
        if m := RE_WATCHDOG.search(line):
            wm_max = max(wm_max, int(m.group(1)))

    print("=== Bouchaud Idle/IRQ V6 ===")
    print(f"SMP-STALL                : {stalls}")
    print(f"desktop heartbeat max ms : {wm_max}")

    if last_diag:
        safe, ticks, age, mask = last_diag
        print(f"BSP safe no-HLT          : {safe}")
        print(f"last PIT ticks            : {ticks}")
        print(f"last PIT age ns           : {age}")
        print(f"last idle mask            : {mask}")
    else:
        print("aucun [IDLE-DIAG] trouvé")

    for cpu in sorted(cpus):
        g = cpus[cpu]
        print(
            f"cpu{cpu}: phase={g[2]} age_ns={g[3]} idle={g[4]} "
            f"sched_safe={g[10]} lock_safe={g[14]} wfi_safe={g[17]} "
            f"sleep_max_ns={g[18]}"
        )

    if last_diag and last_diag[0] == "1" and stalls == 0 and wm_max < 2000:
        print("verdict: BSP no-HLT reste vivant sur la fenêtre observée")
    elif last_diag and last_diag[0] == "1" and (stalls or wm_max >= 2000):
        print("verdict: le freeze/stall persiste malgré BSP no-HLT; regarder PIT/IRQ ou boucle noyau")
    else:
        print("verdict: données insuffisantes")

    return 0

if __name__ == "__main__":
    raise SystemExit(main())
