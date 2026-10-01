#!/usr/bin/env python3
"""Echantillons ou plusieurs vCPU attendent le MEME futex (verrou global de QEMU ?)."""
import sys
from collections import Counter
f = sys.argv[1]; mini = int(sys.argv[2]) if len(sys.argv) > 2 else 4
ech = []
for l in open(f):
    if l.startswith('#'): continue
    p = l.split(); t = int(p[0]); fl = {}
    for x in p[2:]:
        c = x.split(':')
        if len(c) < 5: continue
        fl[c[0]] = (c[1], int(c[2]), int(c[3]), c[4])
    ech.append((t, int(p[1]), fl))
maxrun = {}
for _, _, fl in ech:
    for k, v in fl.items(): maxrun[k] = max(maxrun.get(k, 0), v[1])
top = max(maxrun.values()); pid_principal = min(maxrun, key=int)
vcpu = sorted([k for k, v in maxrun.items() if v > 0.1 * top and k != pid_principal], key=int)
print(f"{f.split('/')[-1]} vcpu={len(vcpu)} principal={pid_principal}")
serie = []
for t, taille, fl in ech:
    adr = Counter(fl[k][3] for k in vcpu if k in fl and fl[k][3].startswith('202@'))
    commun = adr.most_common(1)
    n = commun[0][1] if commun else 0
    if n >= mini:
        hors = {k: (fl[k][0], fl[k][3]) for k in vcpu if k in fl and fl[k][3] != commun[0][0]}
        serie.append((t, n, commun[0][0], hors, fl.get(pid_principal, ('?', 0, 0, '?'))))
    elif serie:
        d = serie[-1][0] - serie[0][0]
        if d >= 60:
            t0, n0, a0, hors0, pr = serie[len(serie) // 2]
            print(f"  t={serie[0][0]} duree>={d}ms vcpu_sur_meme_futex={max(s[1] for s in serie)} adresse={a0} "
                  f"principal={pr[0]}:{pr[3]} hors_groupe={hors0}")
        serie = []
