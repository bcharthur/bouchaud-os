#!/usr/bin/env python3
"""BOUCHAUD_BUREAU_RING3_2B -- l'etat des fenetres de `composited`, lu dans son journal.

    fenetres_bureau_ring3.py JOURNAL [--jusqua MARQUEUR] [--opaques PID,PID...]

Rejoue ce que le compositeur a publie -- apparition d'une fenetre
(`COMPOSITED_BUREAU_FENETRE indice=I`), fin du scenario
(`COMPOSITED_BUREAU_SCENARIO_FIN fenetre=I`), mort d'un client
(`COMPOSITED_BUREAU_CLIENT_MORT pid=P`) -- jusqu'a la fin du journal, ou
jusqu'au bloc du premier MARQUEUR inclus. Ecrit les fenetres visibles du FOND
vers le PREMIER PLAN, `X,Y,L,H` separees par des espaces (ou `vide`) : les
arguments de `verifie_capture_bureau_ring3.py`. `--opaques` : les fenetres de
ces processus (DOOM, Ladybird : contenu non decodable) sont marquees `,o`.
"""
from __future__ import annotations

import re
import sys

ANSI = re.compile(r"\x1b\[[0-9;]*m")
DEBUT = re.compile(r"COMPOSITED_BUREAU_(?:FENETRE indice|SCENARIO_FIN fenetre)=(\d+)")
MORT = re.compile(r"COMPOSITED_BUREAU_CLIENT_MORT pid=(\d+)")
CHAMP = re.compile(r"\[COMPOSITED\] (pid|x|y|l|h|rang)=(-?\d+)$")


def etat(lignes: list[str], jusqua: str | None, opaques: frozenset = frozenset()) -> list[tuple]:
    fen: dict[int, dict] = {}
    courant = None
    vu = False  # le marqueur est passe : on finit son bloc, puis on s'arrete
    for brut in lignes:
        ligne = ANSI.sub("", brut)
        c = CHAMP.search(ligne)
        if c:
            if courant is not None:
                courant[c.group(1)] = int(c.group(2))
            continue
        if "COMPOSITED_BUREAU_" not in ligne:
            continue  # une ligne du noyau intercalee ne ferme pas le bloc
        if vu:
            break
        courant = None
        m = DEBUT.search(ligne)
        if m:
            courant = fen.setdefault(int(m.group(1)), {})
            courant["visible"] = True
        m = MORT.search(ligne)
        if m:
            for f in fen.values():
                if f.get("pid") == int(m.group(1)):
                    f["visible"] = False
        if jusqua and jusqua in ligne:
            vu = True
    visibles = [f for f in fen.values() if f.get("visible") and all(k in f for k in "xylh")]
    visibles.sort(key=lambda f: f.get("rang", 0))
    return [(f["x"], f["y"], f["l"], f["h"]) + (("o",) if f.get("pid") in opaques else ())
            for f in visibles]


def main(argv: list[str]) -> int:
    args = argv[1:]
    jusqua, opaques = None, frozenset()
    try:
        journal = args.pop(0)
        while args:
            option = args.pop(0)
            if option == "--jusqua":
                jusqua = args.pop(0)
            elif option == "--opaques":
                opaques = frozenset(int(p) for p in args.pop(0).split(",") if p)
            else:
                raise ValueError(option)
    except (IndexError, ValueError):
        print(__doc__, file=sys.stderr)
        return 2
    with open(journal, "rb") as f:
        lignes = f.read().decode("utf-8", "replace").splitlines()
    fenetres = etat(lignes, jusqua, opaques)
    print(" ".join(",".join(map(str, f)) for f in fenetres) or "vide")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
