#!/usr/bin/env python3
"""Résume les lots de relecture Skia dans un journal série d'un démarrage."""

import argparse
import json
import re
from pathlib import Path

MARQUEUR = "[LB:SKIA:READBACK] end_us:wall_us:bytes"
MARQUEUR_GPU = "[LB:SKIA:GPU] stage="
ECHANTILLON = re.compile(r"(\d+):(\d+):(\d+)")


def centiles(durees):
    durees.sort()
    nombre = len(durees)
    return {f"p{p}": durees[(p * nombre + 99) // 100 - 1] if nombre else None for p in (50, 95, 99)}


def resume(journal):
    durees = []
    octets = 0
    echecs = 0
    lots_incomplets = 0
    gpu = {"flush_submit": [], "completion_observed": []}
    for ligne in journal.splitlines():
        if MARQUEUR_GPU in ligne:
            nom, _, valeurs = ligne.split(MARQUEUR_GPU, 1)[1].partition(" wall_us")
            lot = valeurs.split()
            if nom in gpu and len(lot) == 16 and all(re.fullmatch(r"[0-9]+", valeur) for valeur in lot):
                gpu[nom].extend(map(int, lot))
            else:
                lots_incomplets += 1
            continue
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

    return {
        "lectures_reussies": len(durees),
        "lectures_en_echec": echecs,
        "lots_incomplets": lots_incomplets,
        "octets_relus": octets,
        "duree_murale_us": centiles(durees),
        "gpu": {nom: {"echantillons": len(valeurs), "duree_murale_us": centiles(valeurs)}
                for nom, valeurs in gpu.items()},
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
