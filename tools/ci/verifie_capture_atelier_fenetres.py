#!/usr/bin/env python3
"""Refuse les fausses captures Atelier (pointeur seul, zero nouvelle fenetre).

Analyse les PPM reels du moniteur QEMU, pas des images generees. Pas de
dependance tierce ni de seuil relache dans les CI existantes.
"""
from __future__ import annotations

from pathlib import Path
import runpy
import sys

LARGEUR_MAX = 4096
HAUTEUR_MAX = 2160
MIN_PIXELS = 12_000
PPM = runpy.run_path(str(Path(__file__).with_name("ppm-vers-png.py")))
LIT_PPM = PPM["lit_ppm"]


def pixels_differents(a: Path, b: Path) -> int:
    aw, ah, ar = LIT_PPM(a)
    bw, bh, br = LIT_PPM(b)
    if (aw, ah) != (bw, bh) or aw > LARGEUR_MAX or ah > HAUTEUR_MAX:
        raise ValueError("resolution absente, variable ou trop grande")
    if len(ar) != aw * ah * 3 or len(br) != aw * ah * 3:
        raise ValueError("capture tronquee")
    # Le pointeur change des dizaines de pixels, pas 12 000. On exclut la
    # barre d'horloge pour ne pas compter son rafraichissement.
    start = min(32, ah) * aw * 3
    count = 0
    for p in range(start, len(ar), 3):
        if (abs(ar[p] - br[p]) + abs(ar[p + 1] - br[p + 1])
                + abs(ar[p + 2] - br[p + 2])) >= 45:
            count += 1
    return count


def verifie(dossier: Path) -> None:
    # Comparer AVANT/APRES sur trois interactions ou une fenetre DOIT changer.
    checks = (
        ("Terminal deplace", "bureau.ppm", "atelier-terminal.ppm"),
        ("Notes ouverte", "atelier-doom.ppm", "atelier-notes.ppm"),
        ("Fichiers ouverte", "atelier-notes.ppm", "atelier-fichiers.ppm"),
    )
    for nom, avant, apres in checks:
        diff = pixels_differents(dossier / avant, dossier / apres)
        print(f"ATELIER_CAPTURE_VERIFICATION {nom} pixels_changes={diff}")
        if diff < MIN_PIXELS:
            raise ValueError(f"fausse capture: {nom}: {diff} pixels (<{MIN_PIXELS})")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: verifie_capture_atelier_fenetres.py SORTIE")
    try:
        verifie(Path(sys.argv[1]))
    except (OSError, ValueError) as exc:
        print(f"ATELIER_CAPTURE_FAIL {exc}", file=sys.stderr)
        raise SystemExit(1)
