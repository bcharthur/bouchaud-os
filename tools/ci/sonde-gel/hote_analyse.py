#!/usr/bin/env python3
"""Fenetres ou les vCPU de QEMU ne tournent pas, vues de l'hote.
usage: hote_analyse.py FICHIER.hote [fenetre_ms] [seuil_fraction]
Pour chaque fenetre glissante, fraction du temps ou chaque vCPU a tourne
(schedstat run) et a attendu un CPU hote (schedstat wait), et ses etats."""
import sys
f = sys.argv[1]; fen = int(sys.argv[2]) if len(sys.argv) > 2 else 500
seuil = float(sys.argv[3]) if len(sys.argv) > 3 else 0.3
ech = []
for l in open(f):
    if l.startswith('#'): continue
    p = l.split(); t, taille = int(p[0]), int(p[1]); fils = {}
    for x in p[2:]:
        c = x.split(':'); tid, etat, run, att = c[:4]; fils[tid] = (etat, int(run), int(att), c[4] if len(c) > 4 else '')
    ech.append((t, taille, fils))
maxrun = {}
for _, _, fl in ech:
    for k, v in fl.items(): maxrun[k] = max(maxrun.get(k, 0), v[1])
top = max(maxrun.values())
vcpu = sorted([k for k, v in maxrun.items() if v > 0.2 * top], key=int)
# ne garder que la periode ou tous les vCPU existent et tournent (apres l'amorcage)
actifs = [i for i, e in enumerate(ech) if all(k in e[2] for k in vcpu)]
print(f"{f.split('/')[-1]} echantillons={len(ech)} vcpu={vcpu}")
if not actifs: sys.exit()
i0, i1 = actifs[0], actifs[-1]
j = i0
trous = []
for i in range(i0, i1):
    while j < i1 and ech[j][0] - ech[i][0] < fen: j += 1
    if j >= i1: break
    d = ech[j][0] - ech[i][0]
    runs = [(ech[j][2][k][1] - ech[i][2][k][1]) / 1000 / d for k in vcpu]
    atts = [(ech[j][2][k][2] - ech[i][2][k][2]) / 1000 / d for k in vcpu]
    if max(runs) < seuil:
        trous.append((ech[i][0], d, runs, atts, i, j))
# fusionner les fenetres qui se chevauchent
fus = []
for t in trous:
    if fus and t[0] <= fus[-1][0] + fus[-1][1]:
        if min(t[2]) < min(fus[-1][2]): fus[-1] = (fus[-1][0], t[0] + t[1] - fus[-1][0], t[2], t[3], fus[-1][4], t[5])
        else: fus[-1] = (fus[-1][0], t[0] + t[1] - fus[-1][0], fus[-1][2], fus[-1][3], fus[-1][4], t[5])
    else: fus.append(t)
for t0, d, runs, atts, i, j in fus:
    etats = {k: ''.join(sorted(set(e[2][k][0] for e in ech[i:j + 1] if k in e[2]))) for k in vcpu}
    autres = {k: (ech[j][2][k][1] - ech[i][2][k][1]) // 1000 for k in ech[i][2] if k not in vcpu and k in ech[j][2]}
    autres = {k: v for k, v in autres.items() if v > 5}
    print(f"  VCPU-ARRETES t_hote={t0} duree~{d}ms run={['%.2f' % r for r in runs]} attente_cpu={['%.2f' % a for a in atts]} etats={etats} autres_fils_run_ms={autres}")
print(f"  fenetres_vcpu_arretes={len(fus)}")
