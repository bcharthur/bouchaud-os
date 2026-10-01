#!/usr/bin/env python3
"""Montre les echantillons hote autour du moment ou le journal brut atteint la ligne t~TICKS."""
import re, sys
hote, brut, cible = sys.argv[1], sys.argv[2], int(sys.argv[3])
data = open(brut, 'rb').read()
off = None
for m in re.finditer(rb' t=(\d+) ', data):
    if int(m.group(1)) >= cible:
        off = m.start(); break
ech = [l.split() for l in open(hote) if not l.startswith('#')]
idx = next(i for i, p in enumerate(ech) if int(p[1]) >= off)
print(f"offset={off} t_hote={ech[idx][0]}")
prev = None
for p in ech[max(0, idx - 25): idx + 20]:
    fl = {x.split(':')[0]: x.split(':') for x in p[2:]}
    if prev:
        dt = int(p[0]) - int(prev[0])
        cols = []
        for k in sorted(fl, key=int):
            if k in prevfl:
                run = (int(fl[k][2]) - int(prevfl[k][2])) // 1000
                att = (int(fl[k][3]) - int(prevfl[k][3])) // 1000
                sc = fl[k][4] if len(fl[k]) > 4 else ''
                cols.append(f"{fl[k][1]}{run:>3}/{att:<2}{sc[:10]:>10}")
        print(f"{p[0]} dt={dt:>3} taille={p[1]:>7} | " + ' '.join(cols))
    prev = p; prevfl = fl
