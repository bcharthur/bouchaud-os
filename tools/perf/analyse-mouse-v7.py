#!/usr/bin/env python3
from __future__ import annotations
import re
import sys
from pathlib import Path

RE_MOUSE = re.compile(
    r"\[MOUSE-IRQ\] phase=(\d+)\(([^)]+)\) entries=(\d+) bytes=(\d+) "
    r"eoi=(\d+) exit=(\d+) packets=(\d+) changed=(\d+) deferred=(\d+) "
    r"irq_signals=(\d+) irq_flushes=(\d+) irq_woken=(\d+) pending=(\d+) "
    r"last_irq_age_ns=(\d+) last_packet_age_ns=(\d+)"
)
RE_SNAPSHOT = re.compile(r"\[SMP-SNAPSHOT\].*t=(\d+)")
RE_STALL = re.compile(r"\[SMP-STALL\]")
RE_WM = re.compile(r"\[sched-watchdog\].*desktop sans heartbeat depuis (\d+) ms")
RE_IDLE = re.compile(r"\[IDLE-DIAG\].*pit_ticks=(\d+).*pit_age_ns=(\d+)")

def main() -> int:
    if len(sys.argv) != 2:
        print("usage: analyse-mouse-v7.py <log>")
        return 2

    text = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
    last_mouse = None
    last_snapshot = 0
    stalls = 0
    wm_max = 0
    last_idle = None

    for line in text.splitlines():
        if m := RE_MOUSE.search(line):
            last_mouse = m
        if m := RE_SNAPSHOT.search(line):
            last_snapshot = max(last_snapshot, int(m.group(1)))
        if RE_STALL.search(line):
            stalls += 1
        if m := RE_WM.search(line):
            wm_max = max(wm_max, int(m.group(1)))
        if m := RE_IDLE.search(line):
            last_idle = (int(m.group(1)), int(m.group(2)))

    print("=== Bouchaud Mouse IRQ V7 ===")
    print(f"last SMP snapshot t      : {last_snapshot}")
    print(f"SMP-STALL                : {stalls}")
    print(f"desktop heartbeat max ms : {wm_max}")

    if last_idle:
        print(f"PIT ticks / age ns       : {last_idle[0]} / {last_idle[1]}")

    if not last_mouse:
        print("aucun [MOUSE-IRQ] trouvé")
        return 1

    g = last_mouse.groups()
    phase_name = g[1]
    entries, byte_count, eoi, exits = map(int, g[2:6])
    packets, changed, deferred = map(int, g[6:9])
    signals, flushes, woken, pending = map(int, g[9:13])
    irq_age, packet_age = map(int, g[13:15])

    print(f"phase                    : {phase_name}")
    print(f"IRQ entries/bytes        : {entries}/{byte_count}")
    print(f"EOI/exits                : {eoi}/{exits}")
    print(f"packets/changed          : {packets}/{changed}")
    print(f"deferred signals         : {deferred}")
    print(f"IRQ signal/flush/woken   : {signals}/{flushes}/{woken}")
    print(f"pending                  : {pending}")
    print(f"last IRQ age ns          : {irq_age}")
    print(f"last packet age ns       : {packet_age}")

    if entries > eoi or entries > exits:
        print("verdict: hard IRQ incomplet; regarder phase/status/EOI")
    elif signals > flushes + 100 and pending:
        print("verdict: bottom-half ne vide plus les réveils; regarder PIT/BKL try_enter")
    elif changed > 0 and flushes > 0 and stalls == 0 and wm_max < 2000:
        print("verdict: chaîne IRQ12 -> bottom-half progresse sans stall global observé")
    elif changed > 0 and (stalls or wm_max >= 2000):
        print("verdict: freeze persistant après déport IRQ; regarder consommateur GUI/scheduler")
    else:
        print("verdict: données insuffisantes")

    return 0

if __name__ == "__main__":
    raise SystemExit(main())
