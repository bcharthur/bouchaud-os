#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path

RE_MAIN = re.compile(
    r"\[PREEMPT-IRQ\] bsp_defer=(\d+) requests=(\d+) direct=(\d+)/(\d+) "
    r"bsp_deferred=(\d+) site_clears=(\d+) continuation_max_ns=(\d+) "
    r"bkl_owner=(\d+) bkl_cpu=(\d+) bkl_site=(\d+) bkl_kind=(\d+)"
)
RE_CPU = re.compile(
    r"\[PREEMPT-CPU\] cpu=(\d+) active=(\d+) source=([^ ]+) "
    r"active_age_ns=(\d+) last_return_age_ns=(\d+)"
)
RE_STALL = re.compile(r"\[SMP-STALL\]")
RE_POLL = re.compile(r"\[SMP-POLL\].*tenue=(\d+)ms")
RE_PROV = re.compile(
    r"\[SMP-PROV\].*owner=(\d+).*cpu=(\d+).*held=(\d+)ms.*kind=(\d+).*task=(\d+).*live_site=(\d+):"
)
RE_WM = re.compile(r"\[sched-watchdog\].*desktop sans heartbeat depuis (\d+) ms")

def main() -> int:
    if len(sys.argv) != 2:
        print("usage: analyse-preempt-v8.py <log>")
        return 2

    text = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
    last = None
    cpus = {}
    stalls = 0
    hold_max = 0
    wm_max = 0
    site41_stall = False

    for line in text.splitlines():
        if m := RE_MAIN.search(line):
            last = tuple(int(x) for x in m.groups())
        if m := RE_CPU.search(line):
            cpus[int(m.group(1))] = m.groups()
        if RE_STALL.search(line):
            stalls += 1
        if m := RE_POLL.search(line):
            hold_max = max(hold_max, int(m.group(1)))
        if m := RE_WM.search(line):
            wm_max = max(wm_max, int(m.group(1)))
        if m := RE_PROV.search(line):
            if int(m.group(3)) >= 1000 and int(m.group(6)) == 41:
                site41_stall = True

    print("=== Bouchaud Preempt IRQ V8 ===")
    print(f"SMP-STALL                 : {stalls}")
    print(f"BKL hold max observed ms  : {hold_max}")
    print(f"desktop heartbeat max ms  : {wm_max}")
    print(f"site41 >=1s observed      : {int(site41_stall)}")

    if last is None:
        print("aucun [PREEMPT-IRQ] trouvé")
        return 1

    (bsp_defer, requests, direct_calls, direct_returns, bsp_deferred,
     site_clears, continuation_max, owner, owner_cpu, bkl_site, bkl_kind) = last

    print(f"BSP deferred mode         : {bsp_defer}")
    print(f"requests                  : {requests}")
    print(f"direct calls/returns      : {direct_calls}/{direct_returns}")
    print(f"BSP deferred              : {bsp_deferred}")
    print(f"site clears               : {site_clears}")
    print(f"continuation max ns       : {continuation_max}")
    print(f"last BKL owner/cpu/site   : {owner}/{owner_cpu}/{bkl_site}")

    for cpu in sorted(cpus):
        g = cpus[cpu]
        print(
            f"cpu{cpu}: active={g[1]} source={g[2]} "
            f"active_age_ns={g[3]} last_return_age_ns={g[4]}"
        )

    if bsp_defer and not site41_stall and stalls == 0 and wm_max < 2000:
        print("verdict: BSP hard-IRQ direct preemption removed; no global stall in observed window")
    elif bsp_defer and site41_stall:
        print("verdict: site41 persists despite BSP defer; inspect AP direct preemption or stale task-layer site")
    elif bsp_defer and (stalls or wm_max >= 2000):
        print("verdict: freeze persists with BSP direct IRQ preemption disabled; site41 was not the root cause")
    else:
        print("verdict: diagnostic mode not active or data insufficient")

    return 0

if __name__ == "__main__":
    raise SystemExit(main())
