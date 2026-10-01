#!/usr/bin/env python3
"""Compare des campagnes scheduler-ng-banc : attentes vives, D p99, releves IRQ."""
import re
import sys
from pathlib import Path
from statistics import median

KERNEL = {}


def lit(log: Path):
    t = log.read_bytes().decode("latin1")
    noms = {}
    for m in re.finditer(r"\[SMP-TASK\] idx=\d+ tid=(\d+) pid=\d+ .*?kernel=(true|false) prio=\w+ name=(\S+)", t):
        noms[int(m.group(1))] = (m.group(2) == "true", m.group(3))
    vives = []
    for m in re.finditer(r"\[SCHED-NG-ATTENTE-VIVE\] tid=(\d+) .*?attente_ms=(\d+) .*?cur_tid=(\d+) cur_pid=\d+ cur_noyau=(\d) .*?syscall=(\d+)", t):
        cur = int(m.group(3))
        k, nom = noms.get(cur, (False, "?"))
        vives.append((int(m.group(2)), nom if k else ("sys%s" % m.group(5) if cur else "aucune")))
    veille = re.findall(r"\[SCHED-NG-VEILLE\] episodes=(\d+) pire_ms=(\d+)", t)
    ep, pire = (int(veille[-1][0]), int(veille[-1][1])) if veille else (0, 0)
    d = re.search(r"sec=D cas=fork-exit-wait4 .*?p99_us=(\d+)", t)
    c = re.search(r"sec=C .*?reveil_p99_us=(\d+)", t)
    q = re.findall(r"preempt_noyau_quantum=(\d+)", t)
    irq = [int(x) for x in re.findall(r"\[SONDE-IRQ-DUREE\] complet=1 duree_ms=(\d+)", t)]
    fin = re.search(r"sec=FIN .*?echecs=(\d+) perdus=(\d+) ressuscites=(\d+)", t)
    return dict(ep=ep, pire=pire, vives=vives, d99=int(d.group(1)) if d else -1,
                c99=int(c.group(1)) if c else -1, q=int(q[-1]) if q else -1,
                irq=irq, fin=fin.groups() if fin else None)


def main():
    for camp in sys.argv[1:]:
        par_smp = {}
        for log in sorted(Path(camp).glob("smp*-*.log")):
            smp = log.name.split("-")[0]
            par_smp.setdefault(smp, []).append(lit(log))
        for smp, rs in sorted(par_smp.items(), key=lambda kv: int(kv[0][3:])):
            causes = {}
            for r in rs:
                for ms, c in r["vives"]:
                    causes.setdefault(c, []).append(ms)
            irq = sorted(x for r in rs for x in r["irq"])
            print(f"{Path(camp).name:8} {smp:5} n={len(rs)} episodes={[r['ep'] for r in rs]} "
                  f"pire_ms={[r['pire'] for r in rs]} D.fork-exit p99_us med={median(r['d99'] for r in rs):.0f} "
                  f"max={max(r['d99'] for r in rs)} q={[r['q'] for r in rs]} "
                  f"irq_complet n={len(irq)} max_ms={irq[-1] if irq else 0} >100ms={sum(x > 100 for x in irq)} "
                  f"fin={[r['fin'] for r in rs]}")
            for c, l in sorted(causes.items(), key=lambda kv: -max(kv[1])):
                print(f"           coeur tenu par {c:24} rapports={len(l):3} max_ms={max(l)}")


main()
