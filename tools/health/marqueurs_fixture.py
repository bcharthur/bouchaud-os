#!/usr/bin/env python3
"""Liste les marqueurs que la fixture plateforme doit imprimer.

Le verdict de la CI reclamait « passed=19/19 » alors que la fixture en emettait
21 depuis l'ajout des tests `https` et `reseau_echec` : deux listes tenues a la
main avaient diverge, et plus aucun run ne pouvait passer. La liste n'a donc
qu'une source, celle de la fixture, et ce script la lit.

Sortie : le NOMBRE de tests sur la premiere ligne, puis un marqueur par ligne.
"""

import pathlib
import re
import sys

FIXTURE = pathlib.Path(__file__).with_name("ladybird_platform_fixture.py")


def marqueurs(source: str) -> list[str]:
    bloc = re.search(r"const required = \[(.*?)\];", source, re.S)
    if bloc is None:
        raise SystemExit(
            "liste `required` introuvable dans la fixture : le verdict ne peut "
            "plus savoir ce qu'il doit exiger"
        )
    noms = re.findall(r'"([a-z_]+)"', bloc.group(1))
    if not noms:
        raise SystemExit("la liste `required` de la fixture est vide")
    return [f"PLATFORM_{nom.upper()} OK" for nom in noms]


def main() -> int:
    lignes = marqueurs(FIXTURE.read_text(encoding="utf-8"))
    print(len(lignes))
    for ligne in lignes:
        print(ligne)
    return 0


if __name__ == "__main__":
    sys.exit(main())
