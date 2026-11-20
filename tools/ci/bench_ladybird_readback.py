#!/usr/bin/env python3
"""Résume les lots de relecture Skia dans un journal série d'un démarrage."""

import argparse
import json
import re
from pathlib import Path

MARQUEUR = "[LB:SKIA:READBACK] end_us:wall_us:bytes"
ECHANTILLON = re.compile(r"(\d+):(\d+):(\d+)")


def resume(journal):
    durees = []
    octets = 0
    echecs = 0
    lots_incomplets = 0
    for ligne in journal.splitlines():
        if MARQUEUR not in ligne:
            continue
        lot = ligne.split(MARQUEUR, 1)[1].split()
        if len(lot) != 16 or not all(ECHANTILLON.fullmatch(valeur) for valeur in lot):
            lots_incomplets += 1
            continue
        for valeur in lot:
            _, duree, volume = map(int, valeur.split(":"))
            if volume:
                durees.append(duree)
                octets += volume
            else:
                echecs += 1

    durees.sort()
    nombre = len(durees)
    return {
        "lectures_reussies": nombre,
        "lectures_en_echec": echecs,
        "lots_incomplets": lots_incomplets,
        "octets_relus": octets,
        "duree_murale_us": {
            f"p{p}": durees[(p * nombre + 99) // 100 - 1] if nombre else None
            for p in (50, 95, 99)
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("journal", type=Path)
    args = parser.parse_args()
    bilan = resume(args.journal.read_text(errors="replace"))
    print(json.dumps(bilan, indent=2))
    return 0 if bilan["lectures_reussies"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
