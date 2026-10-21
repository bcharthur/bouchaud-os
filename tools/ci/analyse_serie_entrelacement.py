#!/usr/bin/env python3
"""Chaque ligne de serie-entrelacement-probe est-elle sortie entiere ?

BOUCHAUD_SERIE_ECRITURE_ATOMIQUE_V1

    analyse_serie_entrelacement.py JOURNAL_SERIE ECRIVAINS LIGNES [CHARGE] [--perdues-admises]

Une ligne est INTACTE si, apres le prefixe de journal eventuel, elle est
exactement celle que la sonde a ecrite (charge utile recalculee, numero repete
en fin de ligne). Tout le reste est une ligne coupee ou melangee.
Sortie 0 si les ECRIVAINS*LIGNES lignes sont toutes intactes, 1 sinon.

BOUCHAUD_SERIE_JAMAIS_SANS_JETON_V1 : `--perdues-admises` (detenteur suspendu
plus longtemps que les anneaux ne tiennent) admet des lignes MANQUANTES, a
condition que le noyau les ait comptees perdues (`lignes_perdues` du bilan) ;
une ligne coupee n'est jamais admise.

BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1 : une ligne DOUBLEE (deux fois la meme
(ecrivain, numero)) ou une ligne qui sort AVANT une ligne plus ancienne du meme
ecrivain est un echec, perdues admises ou non -- la console garde l'ordre de
chaque ecrivain et n'emet une ligne qu'une fois.
"""
import re
import sys

def attendue(e, n, octets):
    charge = "".join(chr(ord("a") + (n * 7 + e * 3 + i) % 26) for i in range(octets))
    return f"SERIE_LIGNE e={e} n={n:04d} {charge} FIN_{n:04d}"


def main(argv):
    perdues_admises = "--perdues-admises" in argv
    argv = [a for a in argv if a != "--perdues-admises"]
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
    doublees = 0
    inversions = []
    dernier = {}
    abimees = []
    for brute in texte.replace("\r", "").split("\n"):
        i = brute.find("SERIE_LIGNE")
        if i < 0:
            continue
        ligne = brute[i:]
        m = re.match(r"SERIE_LIGNE e=(\d+) n=(\d{4}) ", ligne)
        if m and ligne == attendue(int(m.group(1)), int(m.group(2)), octets):
            e, n = int(m.group(1)), int(m.group(2))
            if (e, n) in vues:
                doublees += 1
            vues.add((e, n))
            if e in dernier and n < dernier[e]:
                inversions.append((e, dernier[e], n))
            dernier[e] = max(n, dernier.get(e, -1))
        else:
            abimees.append(brute)
    total = ecrivains * lignes
    intactes = len({(e, n) for (e, n) in vues if e < ecrivains and n < lignes})
    bilan = re.findall(r"BOUCHAUD_SERIE_BILAN emissions_a_la_borne=(\d+) emissions_reentrees=(\d+)", texte)
    bornes = bilan[-1] if bilan else ("?", "?")
    perdues = re.findall(r"BOUCHAUD_SERIE_BILAN .*? lignes_differees=(\d+) lignes_perdues=(\d+) lignes_vidangees=(\d+)", texte)
    differees, perdues_n, vidangees = (int(x) for x in perdues[-1]) if perdues else (-1, -1, -1)
    manquantes = total - intactes
    if perdues_admises:
        complet = perdues_n >= 0 and manquantes <= perdues_n
    else:
        complet = intactes == total
    verdict = "OK" if complet and not abimees and not doublees and not inversions else "ECHEC"
    print(f"SERIE_ENTRELACEMENT_{verdict} intactes={intactes}/{total} abimees={len(abimees)} "
          f"doublees={doublees} inversions={len(inversions)} "
          f"emissions_a_la_borne={bornes[0]} emissions_reentrees={bornes[1]} "
          f"lignes_differees={differees} lignes_perdues={perdues_n} lignes_vidangees={vidangees}")
    for e, avant, n in inversions[:3]:
        print(f"  inversion : ecrivain {e}, ligne {n:04d} apres {avant:04d}")
    for brute in abimees[:3]:
        print("  abimee : " + brute[:200].encode("unicode_escape").decode("ascii"))
    return 0 if verdict == "OK" else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
