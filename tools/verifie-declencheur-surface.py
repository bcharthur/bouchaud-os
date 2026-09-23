#!/usr/bin/env python3
"""Garde-fou : la capture de la mire se declenche-t-elle au bon moment ?

BOUCHAUD_C41_CAPTURE_VIVANTE

La logique vit dans `tools/ci/surface_declencheur.py`, avec ses journaux
synthetiques. Ce fichier n'existe que pour la faire entrer dans la barriere
d'architecture, qui decouvre les gardes par le motif `verifie-*.py`.

Sans cela, l'autotest ne tournerait qu'a la main -- c'est-a-dire jamais, ce
qui est la facon habituelle dont un test cesse de proteger quoi que ce soit.
"""
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent / "ci"))

import surface_declencheur  # noqa: E402

if __name__ == "__main__":
    sys.exit(surface_declencheur.autotest())
