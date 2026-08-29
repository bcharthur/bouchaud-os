#!/usr/bin/env python3
from __future__ import annotations
import re, sys
from pathlib import Path

PHASES = {
    700: "desktop: début de tour",
    730: "desktop: composition/culling",
    740: "desktop: trame terminée",
    745: "desktop: trame différée",
    750: "desktop: present complet avant scope",
    751: "desktop: present complet terminé",
    752: "desktop: present_rect avant scope",
    753: "desktop: present_rect terminé",
    760: "desktop: rapport périodique",
    770: "desktop: préparation attente interface",
    771: "desktop: sommeil interface détaché",
    772: "desktop: reprise BKL après sommeil",
    773: "desktop: attente terminée",
}

def read_log(path: Path) -> str:
    raw = path.read_bytes()
    if raw.startswith(b"\xff\xfe"):
        return raw[2:].decode("utf-16-le", errors="replace")
    sample = raw[:4096]
    pairs = max(1, len(sample)//2)
    if sample[1::2].count(0) > pairs//3 and sample[0::2].count(0) < pairs//10:
        return raw.decode("utf-16-le", errors="replace")
    return raw.decode("utf-8-sig", errors="replace")

RE_IDLE = re.compile(r"\[IDLE-DIAG\].*bsp_safe=(\d+).*bsp_hlt=(\d+)")
RE_CPU0 = re.compile(
    r"\[IDLE-CPU\] cpu=0 phase=(\d+)\(([^)]+)\).*sched=(\d+)/(\d+)/(\d+)/safe(\d+).*lock=(\d+)/(\d+)/(\d+)/safe(\d+).*wfi=(\d+)/(\d+)/safe(\d+).*sleep_max_ns=(\d+)"
)
RE_IFACE = re.compile(
    r"\[INTERFACE-WAIT\] phase=(\d+)\(([^)]+)\).*detached=(\d+).*sleep_max_ns=(\d+).*resume_max_ns=(\d+).*depth_violations=(\d+)"
)
RE_PROV = re.compile(r"\[SMP-PROV\].*held=(\d+)ms.*task=(\d+).*site=(\d+):")
RE_WATCH = re.compile(r"desktop sans heartbeat depuis (\d+) ms")
RE_BKL = re.compile(r"\[BKL-SCHED\].*wait_max_ns=(\d+)")

def main() -> int:
    if len(sys.argv) != 2:
        print("usage: analyse-event-driven-core.py <log>")
        return 2

    text = read_log(Path(sys.argv[1]))
    last_idle = last_cpu0 = last_iface = None
    max_hold = heartbeat = resume_max = 0
    worst = None

    for line in text.splitlines():
        if m := RE_IDLE.search(line):
            last_idle = tuple(map(int, m.groups()))
        if m := RE_CPU0.search(line):
            g = m.groups()
            last_cpu0 = (
                int(g[0]), g[1], *map(int, g[2:])
            )
        if m := RE_IFACE.search(line):
            g = m.groups()
            last_iface = (int(g[0]), g[1], *map(int, g[2:]))
        if m := RE_PROV.search(line):
            held, task, site = map(int, m.groups())
            max_hold = max(max_hold, held)
            if worst is None or held > worst[0]:
                worst = (held, task, site, line)
        if m := RE_WATCH.search(line):
            heartbeat = max(heartbeat, int(m.group(1)))
        if m := RE_BKL.search(line):
            resume_max = max(resume_max, int(m.group(1)))

    print("=== Bouchaud Event-Driven Core ===")
    if last_idle:
        print(f"BSP safe / HLT          : {last_idle[0]} / {last_idle[1]}")
    else:
        print("IDLE-DIAG               : absent")

    if last_cpu0:
        phase, name, sp, sc, sw, ss, lp, lc, lw, ls, we, ww, ws, sleepmax = last_cpu0
        print(f"CPU0 phase              : {name}")
        print(f"CPU0 sched prep/commit/wake/safe : {sp}/{sc}/{sw}/{ss}")
        print(f"CPU0 lock prep/commit/wake/safe  : {lp}/{lc}/{lw}/{ls}")
        print(f"CPU0 wfi enter/wake/safe         : {we}/{ww}/{ws}")
        print(f"CPU0 sleep max          : {sleepmax/1e6:.3f} ms")

    if last_iface:
        phase, name, detached, sleepmax, resumemax, violations = last_iface
        print(f"interface phase         : {name}")
        print(f"desktop detached waits  : {detached}")
        print(f"interface sleep max     : {sleepmax/1e6:.3f} ms")
        print(f"interface resume max    : {resumemax/1e6:.3f} ms")
        print(f"interface depth violations: {violations}")
    else:
        print("INTERFACE-WAIT          : absent")

    print(f"max BKL hold            : {max_hold} ms")
    print(f"BKL resume max          : {resume_max/1e6:.3f} ms")
    print(f"desktop heartbeat max   : {heartbeat} ms")

    if worst:
        held, task, site, line = worst
        print(f"worst site              : {site} — {PHASES.get(site, 'autre domaine')}")
        print("worst provenance:")
        print("  " + line)

    print("verdict:")
    if last_iface and last_iface[-1] != 0:
        print("- ECHEC invariant: attente desktop détachée a rendu une mauvaise profondeur BKL")
    elif last_idle and last_idle[1] != 1:
        print("- BSP HLT non actif: vérifier politique idle appliquée")
    elif max_hold < 500 and heartbeat < 2000:
        print("- objectif principal atteint sur ce run")
    elif worst and worst[1] == 0:
        print("- longue tenue restante côté desktop/kernel; le site 700..773 donne maintenant la phase")
    else:
        print("- amélioration à comparer au V12; utiliser le worst site pour le prochain ciblage")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
