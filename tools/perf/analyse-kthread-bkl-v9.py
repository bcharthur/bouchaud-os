#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path

RE_KT = re.compile(
    r"\[KTHREAD-BKL\] mode=([^ ]+) task=([^ ]+) depth=(\d+) owner=(\d+) "
    r"owner_cpu=(\d+) parked=(0x[0-9a-fA-F]+) resume=(0x[0-9a-fA-F]+) "
    r"checks=(\d+) checkpoints=(\d+) scopes=(\d+) releases=(\d+) contended=(\d+) "
    r"gap_current_ns=(\d+) gap_max_ns=(\d+) unlocked_ns=(\d+) unlocked_max_ns=(\d+) "
    r"reacquire_ns=(\d+) reacquire_max_ns=(\d+) release_window_ns=(\d+) "
    r"release_window_max_ns=(\d+) handoff_spins=(\d+)"
)
RE_SITE = re.compile(
    r"\[KTHREAD-BKL-SITE\] site=([^ ]+) releases=(\d+) unlocked_ns=(\d+)"
)
RE_STALL = re.compile(r"\[SMP-STALL\]")
RE_POLL = re.compile(r"\[SMP-POLL\].*tenue=(\d+)ms")
RE_WM = re.compile(r"\[sched-watchdog\].*desktop sans heartbeat depuis (\d+) ms")
RE_SCHED = re.compile(r"\[BKL-SCHED\].*wait_max_ns=(\d+)")
RE_MOUSE = re.compile(r"\[MOUSE-IRQ\].*entries=(\d+).*eoi=(\d+).*exit=(\d+).*pending=(\d+)")

def main() -> int:
    if len(sys.argv) != 2:
        print("usage: analyse-kthread-bkl-v9.py <log>")
        return 2

    text = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
    last = None
    sites = {}
    stalls = 0
    hold_max_ms = 0
    wm_max = 0
    resume_max = 0
    mouse = None

    for line in text.splitlines():
        if m := RE_KT.search(line):
            last = m.groups()
        if m := RE_SITE.search(line):
            sites[m.group(1)] = (int(m.group(2)), int(m.group(3)))
        if RE_STALL.search(line):
            stalls += 1
        if m := RE_POLL.search(line):
            hold_max_ms = max(hold_max_ms, int(m.group(1)))
        if m := RE_WM.search(line):
            wm_max = max(wm_max, int(m.group(1)))
        if m := RE_SCHED.search(line):
            resume_max = max(resume_max, int(m.group(1)))
        if m := RE_MOUSE.search(line):
            mouse = tuple(int(x) for x in m.groups())

    print("=== Bouchaud Desktop BKL V9 ===")
    print(f"SMP-STALL                : {stalls}")
    print(f"BKL hold max log ms      : {hold_max_ms}")
    print(f"desktop heartbeat max ms : {wm_max}")
    print(f"resume wait max ns       : {resume_max}")

    if mouse:
        print(f"mouse irq/eoi/exit/pending: {mouse[0]}/{mouse[1]}/{mouse[2]}/{mouse[3]}")

    if not last:
        print("aucun [KTHREAD-BKL] trouvé")
        return 1

    (
        mode, task, depth, owner, owner_cpu, parked, resume,
        checks, checkpoints, scopes, releases, contended,
        gap_current, gap_max, unlocked, unlocked_max,
        reacq, reacq_max, window, window_max, spins
    ) = last

    print(f"mode/task                : {mode}/{task}")
    print(f"last depth/owner/cpu     : {depth}/{owner}/{owner_cpu}")
    print(f"checks/checkpoints/scopes: {checks}/{checkpoints}/{scopes}")
    print(f"releases/contended       : {releases}/{contended}")
    print(f"gap current/max ns       : {gap_current}/{gap_max}")
    print(f"unlocked total/max ns    : {unlocked}/{unlocked_max}")
    print(f"reacquire total/max ns   : {reacq}/{reacq_max}")
    print(f"release window max ns    : {window_max}")
    print(f"handoff spins            : {spins}")

    for name, (count, ns) in sorted(sites.items()):
        print(f"site {name:14s}: releases={count} unlocked_ns={ns}")

    mode_scoped = mode == "scoped"
    meaningful = int(releases) > 0

    if mode_scoped and meaningful and hold_max_ms < 250 and wm_max < 2000:
        print("verdict: safe points actifs; plus de long hold desktop observe")
    elif mode_scoped and meaningful and hold_max_ms >= 1000:
        print("verdict: long hold persiste; trouver la phase GUI sans safe point")
    elif mode_scoped and not meaningful:
        print("verdict: V9 actif mais aucun safe point eligible (depth/task/IF a verifier)")
    else:
        print("verdict: donnees insuffisantes")

    return 0

if __name__ == "__main__":
    raise SystemExit(main())
