#!/usr/bin/env python3
"""Garde-fou : l'audit des douze chantiers dit ce que le code dit.

`docs/AUDIT_12_CHANTIERS.md` a peri une premiere fois exactement comme
perissent tous les documents qui portent des chiffres a la main : le code a
avance, le document est reste. Il affirmait « futex reste sous BKL » et « pas
de W^X » plusieurs commits apres que le code eut dit l'inverse.

Un document faux est pire qu'un document absent : on lui fait confiance, et
chaque decision qu'on en tire est fausse d'un cran.

Ce garde-fou delegue la mesure a `tools/mesure-chantiers.py --verifie`, qui
compte sur l'arbre et compare. La mise a jour du document devient une condition
de la barriere, et non une intention.
"""

import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]

if __name__ == "__main__":
    sys.exit(
        subprocess.call(
            [sys.executable, str(RACINE / "tools/mesure-chantiers.py"), "--verifie"]
        )
    )
