#!/usr/bin/env python3
"""Preuve de vie cumulative, independante d'un evenement serie perdu.

Un snapshot apres FIN doit couvrir TOUT le run (created=1, removed=0),
avec le meme PID vivant avant le workload et apres sa fin. Un redemarrage
entre deux releves reste visible dans les compteurs, meme avec PID recycle.
Les lignes abimees ne sont pas reconstruites : sans preuve complete, rouge.
"""
import re
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
STATE = re.compile(r"\[LB:COMPOSITOR_STATE\] seq=(\d+) created=(\d+) removed=(\d+) live=(\d+) pid=(\d+) END")


def verify(text):
    lines = ANSI.sub("", text).splitlines()
    start = next((i for i, l in enumerate(lines) if "HOST_ENDURANCE cycle=1 " in l), None)
    end = next((i for i, l in enumerate(lines) if "HOST_ENDURANCE_FIN cycles=" in l), None)
    samples = [(i, tuple(map(int, m.groups()))) for i, l in enumerate(lines) if (m := STATE.search(l))]
    if start is None or end is None or start >= end or not samples:
        return False, "workload ou releves absents"
    active = [(i, v) for i, v in samples if v[1] > 0]
    if not active or active[0][0] >= start or active[-1][0] <= end:
        return False, "preuve manquante avant workload ou apres FIN"
    pid = active[0][1][4]
    if not pid or any(v[1:] != (1, 0, 1, pid) for _, v in active):
        return False, "creation multiple, retrait, mort ou PID change"
    if any(a[1][0] >= b[1][0] for a, b in zip(samples, samples[1:])):
        return False, "sequence non monotone (logs melanges)"
    if any(v[1] == 0 and i > active[0][0] for i, v in samples):
        return False, "compteur remis a zero"
    return True, f"created=1 removed=0 live=1 pid={pid} samples={len(active)} avant_et_apres=oui"


if __name__ == "__main__":
    ok, detail = verify(Path(sys.argv[1]).read_text(errors="replace"))
    print(f"COMPOSITOR_PREUVE {'OK' if ok else 'ECHEC'} {detail}")
    sys.exit(0 if ok else 1)
