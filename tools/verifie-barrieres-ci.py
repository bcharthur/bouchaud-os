#!/usr/bin/env python3
"""Toute barriere agregee doit etre exigee sur `main`, et reciproquement.

# Ce que ce garde-fou protege

La protection de branche ne connait que les noms qu'on lui donne. Un job
`*-gate` ajoute dans un workflow n'y entre pas tout seul : il tourne, il peut
echouer, et la fusion passe quand meme. C'est la pire forme de barriere -- elle
existe, elle est verte ou rouge, et elle n'empeche rien.

Le defaut inverse est aussi reel : un contexte exige qui n'existe plus BLOQUE
toute fusion pour toujours, puisque GitHub attend un statut que personne ne
publiera jamais. Une barriere qu'on ne peut pas satisfaire finit par etre
retiree en urgence, et avec elle celles qui servaient.

# La regle, et la distinction qui la rend juste

Une barriere ne peut etre exigee que si elle PUBLIE un statut sur la tete d'une
pull request -- donc si son workflow se declenche sur `pull_request`. Exiger une
campagne planifiee (nightly, endurance) bloquerait toute fusion pour toujours :
GitHub attendrait un statut que personne ne publie sur ce commit.

Deux regles, donc :

  1. tout job `*-gate` d'un workflow declenche sur `pull_request` DOIT etre
     exige sur main ;
  2. tout contexte exige DOIT exister dans un tel workflow.

Une campagne planifiee garde ses barrieres -- elles servent a rendre le run
rouge -- et n'entre pas dans la protection de branche.

Ce script ne verifie pas GitHub -- il n'a pas les droits, et un depot ou la
protection n'a pas encore ete appliquee ne doit pas etre rouge pour autant. Il
verifie que la COMMANDE qu'on donnera a Arthur est a jour.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
WORKFLOWS = RACINE / ".github" / "workflows"
PROTECTION = RACINE / "tools" / "ci" / "configure_protection.ps1"

# `  nom-gate:` en tete de job, deux niveaux d'indentation.
JOB = re.compile(r"^  ([a-z0-9][a-z0-9-]*-gate):\s*$", re.M)
CONTEXTE = re.compile(r'"([a-z0-9][a-z0-9-]*-gate)"')


def main() -> int:
    if not WORKFLOWS.is_dir():
        print(f"introuvable : {WORKFLOWS}")
        return 1
    if not PROTECTION.exists():
        print(f"introuvable : {PROTECTION}")
        return 1

    barrieres: dict[str, str] = {}
    planifiees: dict[str, str] = {}
    for chemin in sorted(WORKFLOWS.glob("*.yml")):
        texte = chemin.read_text(encoding="utf-8")
        # Le declencheur, lu dans le bloc `on:` de tete.
        entete = texte.split("\njobs:", 1)[0]
        sur_pull_request = re.search(r"^\s*pull_request:", entete, re.M) is not None
        for nom in JOB.findall(texte):
            if sur_pull_request:
                barrieres[nom] = chemin.name
            else:
                planifiees[nom] = chemin.name

    exiges = set(CONTEXTE.findall(PROTECTION.read_text(encoding="utf-8")))

    fautes = []
    for nom, fichier in sorted(barrieres.items()):
        if nom not in exiges:
            fautes.append(
                f"  `{nom}` ({fichier}) n'est pas exigee sur main : elle "
                f"tourne, elle peut echouer, et la fusion passe quand meme"
            )
    for nom in sorted(exiges - set(barrieres)):
        ou = planifiees.get(nom)
        if ou:
            fautes.append(
                f"  `{nom}` est exigee sur main mais son workflow ({ou}) ne se "
                f"declenche pas sur `pull_request` : GitHub attendrait un "
                f"statut que personne ne publie sur la tete d'une PR, et toute "
                f"fusion serait bloquee pour toujours"
            )
        else:
            fautes.append(
                f"  `{nom}` est exigee sur main et n'existe dans aucun "
                f"workflow : meme consequence, toute fusion bloquee"
            )

    if fautes:
        print("barrieres de CI : regle violee")
        print("\n".join(fautes))
        return 1

    print(
        f"ok  {len(barrieres)} barriere(s) exigee(s) sur main ("
        + ", ".join(sorted(barrieres))
        + f") ; {len(planifiees)} barriere(s) de campagne planifiee hors "
        f"protection de branche"
        + (" (" + ", ".join(sorted(planifiees)) + ")" if planifiees else "")
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
