#!/usr/bin/env python3
"""Un workflow publie-t-il ses checks sur la tete d'une pull request ?

BOUCHAUD_CI_TROIS_NIVEAUX_V1. Un check exige par la protection de `main` doit
etre PUBLIE sur le commit de tete de chaque PR, sinon GitHub l'attend pour
toujours. Deux declencheurs le garantissent :

  * `pull_request` -- le run porte le commit de tete ;
  * `push` sur TOUTES les branches (`branches: ['**']`) SANS filtre de chemins
    ni d'exclusion -- chaque commit pousse sur la branche d'une PR (du depot)
    recoit le check, quel que soit le fichier modifie.

Un `push` filtre (chemins, liste de branches) ne le garantit pas : une PR
documentaire, ou une branche hors liste, n'aurait jamais le check. Le mot
`pull_request` dans un COMMENTAIRE ne compte pas : `verifie-protection-main.py`
le croyait (recherche de sous-chaine), et l'aurait cru d'un workflow qui ne se
declenchait plus du tout sur PR.

Lancer ce fichier execute ses temoins.
"""
from __future__ import annotations

import re
import sys


def _lignes_on(entete: str) -> list[str]:
    """Les lignes du bloc `on:` de tete, commentaires retires."""
    lignes = []
    dedans = False
    for brute in entete.split("\n"):
        ligne = brute.split(" #", 1)[0].rstrip() if not brute.lstrip().startswith("#") else ""
        if not ligne:
            continue
        if re.match(r"^(on|\"on\"|'on'):\s*$", ligne):
            dedans = True
            continue
        if dedans and not ligne.startswith(" "):
            break
        if dedans:
            lignes.append(ligne)
    return lignes


def _bloc(lignes: list[str], cle: str) -> list[str] | None:
    """Le contenu d'un declencheur (`  cle:`), ou None s'il est absent."""
    for i, ligne in enumerate(lignes):
        if re.match(rf"^  {cle}:\s*$", ligne):
            fin = i + 1
            while fin < len(lignes) and lignes[fin].startswith("   "):
                fin += 1
            return lignes[i + 1:fin]
    return None


def publie_sur_tete_pr(entete: str) -> bool:
    lignes = _lignes_on(entete)
    if _bloc(lignes, "pull_request") is not None:
        return True
    push = _bloc(lignes, "push")
    if push is None:
        return False
    texte = "\n".join(push)
    if re.search(r"^\s+(paths|paths-ignore|branches-ignore|tags|tags-ignore):", texte, re.M):
        return False
    branches = re.search(r"^\s+branches:\s*\[(.*)\]\s*$", texte, re.M)
    if not branches:
        return False
    motifs = [m.strip().strip("'\"") for m in branches.group(1).split(",")]
    return "**" in motifs


def _temoins() -> int:
    cas = [
        ("on:\n  pull_request:\n  push:\n    branches: [main]\n", True),
        ("on:\n  push:\n    branches: ['**']\n  workflow_dispatch:\n", True),
        ("on:\n  push:\n    branches: [\"**\"]\n", True),
        # Negatifs : chacun aurait laisse une PR sans check.
        ("on:\n  push:\n    branches: ['**']\n    paths:\n      - 'src/**'\n", False),
        ("on:\n  push:\n    branches: [main, 'claude/**']\n", False),
        ("on:\n  push:\n    branches: ['**']\n    branches-ignore: [docs]\n", False),
        ("on:\n  # pull_request: retire\n  push:\n    branches: [main]\n", False),
        ("on:\n  push:\n    branches: [main]  # pull_request ailleurs\n", False),
        ("on:\n  schedule:\n    - cron: '0 1 * * *'\n  workflow_dispatch:\n", False),
    ]
    fautes = 0
    for entete, attendu in cas:
        obtenu = publie_sur_tete_pr(entete)
        if obtenu != attendu:
            fautes += 1
            print(f"TEMOIN FAUX attendu={attendu} obtenu={obtenu} :\n{entete}")
    print(f"declencheurs : {len(cas) - fautes}/{len(cas)} temoins")
    return 1 if fautes else 0


if __name__ == "__main__":
    sys.exit(_temoins())
