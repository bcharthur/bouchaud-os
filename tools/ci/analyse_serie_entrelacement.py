#!/usr/bin/env python3
"""Chaque ligne de serie-entrelacement-probe est-elle sortie entiere ?

BOUCHAUD_SERIE_ECRITURE_ATOMIQUE_V1

    analyse_serie_entrelacement.py JOURNAL_SERIE ECRIVAINS LIGNES [CHARGE]

Une ligne est INTACTE si, apres le prefixe de journal eventuel, elle est
exactement celle que la sonde a ecrite (charge utile recalculee, numero repete
en fin de ligne). Tout le reste est une ligne coupee ou melangee.
Sortie 0 si les ECRIVAINS*LIGNES lignes sont toutes intactes, 1 sinon.
"""
import re
import sys

def attendue(e, n, octets):
    charge = "".join(chr(ord("a") + (n * 7 + e * 3 + i) % 26) for i in range(octets))
    return f"SERIE_LIGNE e={e} n={n:04d} {charge} FIN_{n:04d}"


def main(argv):
    if len(argv) not in (4, 5):
        print(__doc__, file=sys.stderr)
        return 2
    journal, ecrivains, lignes = argv[1], int(argv[2]), int(argv[3])
    octets = int(argv[4]) if len(argv) == 5 else 160
    try:
        texte = open(journal, "rb").read().decode("latin-1")
    except OSError as e:
        print(f"SERIE_ENTRELACEMENT_ECHEC journal illisible : {e}")
        return 1
    vues = set()
    abimees = []
    for brute in texte.replace("\r", "").split("\n"):
        i = brute.find("SERIE_LIGNE")
        if i < 0:
            continue
        ligne = brute[i:]
        m = re.match(r"SERIE_LIGNE e=(\d+) n=(\d{4}) ", ligne)
        if m and ligne == attendue(int(m.group(1)), int(m.group(2)), octets):
            vues.add((int(m.group(1)), int(m.group(2))))
        else:
            abimees.append(brute)
    total = ecrivains * lignes
    intactes = len({(e, n) for (e, n) in vues if e < ecrivains and n < lignes})
    bilan = re.findall(r"BOUCHAUD_SERIE_BILAN emissions_a_la_borne=(\d+) emissions_reentrees=(\d+)", texte)
    bornes = bilan[-1] if bilan else ("?", "?")
    verdict = "OK" if intactes == total and not abimees else "ECHEC"
    print(f"SERIE_ENTRELACEMENT_{verdict} intactes={intactes}/{total} abimees={len(abimees)} "
          f"emissions_a_la_borne={bornes[0]} emissions_reentrees={bornes[1]}")
    for brute in abimees[:3]:
        print("  abimee : " + brute[:200].encode("unicode_escape").decode("ascii"))
    return 0 if verdict == "OK" else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
