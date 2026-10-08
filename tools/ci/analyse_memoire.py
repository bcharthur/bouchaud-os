#!/usr/bin/env python3
"""Ce qui reste apres des onglets fermes : fuite ou cache borne ?

BOUCHAUD_MEMOIRE_ONGLETS_V1

    tools/ci/analyse_memoire.py JOURNAL_SERIE

La page `memoire.html` publie trois reperes (`HOST_MEMOIRE_REPERE m=0|1|2`) :
avant, apres une premiere phase de n onglets autre site fermes, apres une
seconde phase IDENTIQUE. Le noyau publie le RSS de chaque processus toutes
les 5 s (`[PERF-PROC] t= pid= image= rss_kio=`), le Compositor ses objets
vivants a chaque contexte cree ou detruit (`[LB:MEM]`, BOUCHAUD_LB_MEM_V1).

Les deux sont alignes sur l'horodatage de la ligne serie. Pour chaque
processus qui vit du debut a la fin (un service, pas un onglet), le RSS au
plus pres de chaque repere, et la croissance de chaque phase : G1 = M1 - M0,
G2 = M2 - M1. Une fuite croit autant en phase 2 qu'en phase 1 ; un cache
borne croit en phase 1 puis plus. Verdict par processus :

    MEMOIRE_PROCESSUS image= m0_kio= m1_kio= m2_kio= g1_kio= g2_kio= max_kio= verdict=borne|croissance

`croissance` si G2 > 1024 + G1 / 2 (Kio) : la seconde phase a retenu plus de
la moitie de la premiere, plus un Mio de bruit. Et pour le Compositor, les
contextes vivants et les octets des surfaces aux trois reperes : apres la
fermeture des onglets, ils doivent retomber a leur niveau de M0.

Sortie : MEMOIRE_BORNEE ou MEMOIRE_CROISSANCE n= ; rend 0 / 1. Sans les
trois reperes : MEMOIRE_INCONCLUSIF, rend 2.
"""
import re
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
HEURE = re.compile(r"^\[(\d\d):(\d\d):(\d\d)\]")
REPERE = re.compile(r"HOST_MEMOIRE_REPERE m=(\d)")
PROC = re.compile(r"\[PERF-PROC\] t=\d+ pid=(\d+) image=(\S+) rss_kio=(\d+)")
MEM = re.compile(r"\[LB:MEM\] ev=\S+ ctx=\d+ contexts_live=(\d+) backing_stores_live=(\d+) backing_store_octets=(\d+)")
ALIAS = {"bo-navigateur": "BouchaudBrowserHost"}


def secondes(ligne: str):
    m = HEURE.match(ligne)
    return int(m[1]) * 3600 + int(m[2]) * 60 + int(m[3]) if m else None


def main(argv) -> int:
    if len(argv) != 2:
        print(__doc__.strip().splitlines()[4], file=sys.stderr)
        return 2
    reperes, rss, mem = {}, {}, []
    jour = 0
    derniere = None
    for brut in Path(argv[1]).read_text(errors="replace").splitlines():
        ligne = ANSI.sub("", brut).replace("\r", "")
        t = secondes(ligne)
        if t is None:
            continue
        # Passage de minuit : le journal reste monotone.
        if derniere is not None and t + jour < derniere - 3600:
            jour += 86400
        t += jour
        derniere = t
        if m := REPERE.search(ligne):
            reperes.setdefault(int(m[1]), t)
        if m := PROC.search(ligne):
            image = Path(m[2]).name
            image = ALIAS.get(image, image)
            rss.setdefault((int(m[1]), image), []).append((t, int(m[3])))
        if m := MEM.search(ligne):
            mem.append((t, int(m[1]), int(m[2]), int(m[3])))
    if not all(k in reperes for k in (0, 1, 2)):
        print(f"MEMOIRE_INCONCLUSIF reperes={sorted(reperes)}")
        return 2

    def au_repere(points, k):
        """La derniere valeur relevee au plus tard au repere k."""
        avant = [v for t, *v in points if t <= reperes[k]]
        return avant[-1] if avant else None

    croissances = 0
    print("== memoire apres des onglets fermes (deux phases identiques) ==")
    for (pid, image), points in sorted(rss.items(), key=lambda kv: kv[0][1]):
        # Un service vit avant M0 et apres M2 ; un onglet non.
        if points[0][0] > reperes[0] or points[-1][0] < reperes[2]:
            continue
        m0, m1, m2 = (au_repere(points, k) for k in (0, 1, 2))
        if None in (m0, m1, m2):
            continue
        m0, m1, m2 = m0[0], m1[0], m2[0]
        g1, g2 = m1 - m0, m2 - m1
        dans = [r for t, r in points if reperes[0] <= t <= reperes[2]]
        verdict = "croissance" if g2 > 1024 + max(g1, 0) // 2 else "borne"
        croissances += verdict == "croissance"
        print(f"MEMOIRE_PROCESSUS pid={pid} image={image} m0_kio={m0} m1_kio={m1} m2_kio={m2} "
              f"g1_kio={g1} g2_kio={g2} max_kio={max(dans) if dans else m2} verdict={verdict}")
    if mem:
        lignes = [(t, *v) for t, *v in mem]
        for k in (0, 1, 2):
            v = au_repere([(t, a, b, c) for t, a, b, c in lignes], k)
            if v:
                print(f"MEMOIRE_COMPOSITOR m={k} contexts_live={v[0]} backing_stores_live={v[1]} backing_store_kio={v[2] // 1024}")
        v0 = au_repere([(t, a, b, c) for t, a, b, c in lignes], 0)
        v2 = au_repere([(t, a, b, c) for t, a, b, c in lignes], 2)
        if v0 and v2 and (v2[0] > v0[0] or v2[2] > v0[2]):
            croissances += 1
            print(f"MEMOIRE_COMPOSITOR_OBJETS_RESTANTS contexts {v0[0]}->{v2[0]} backing_store_kio {v0[2] // 1024}->{v2[2] // 1024}")
    else:
        print("MEMOIRE_COMPOSITOR absent (aucune ligne [LB:MEM])")
    if croissances:
        print(f"MEMOIRE_CROISSANCE n={croissances}")
        return 1
    print("MEMOIRE_BORNEE")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
