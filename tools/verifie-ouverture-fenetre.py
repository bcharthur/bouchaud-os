#!/usr/bin/env python3
"""Verifie qu'aucune fenetre n'est ouverte sans annoncer son apparition.

LE DEFAUT
---------
Ouvrir une application par double-clic sur une icone du bureau poussait la
fenetre dans `wins` et n'annoncait RIEN : ni degat, ni `sale`. Le chemin par le
menu Demarrer, lui, appelait `degats.tout()`.

La fenetre et son bouton de barre des taches n'existaient alors que dans
l'etat. Ils n'apparaissaient qu'au moment ou un AUTRE degat passait par la --
le curseur qu'on promene, par exemple. C'est ce qui s'est vu comme « la barre
des taches n'affiche Fichiers que si je passe la souris dessus ».

LA REGLE
--------
Une fenetre entre dans `wins` par `ouvre_fenetre`, qui pousse ET annonce. Un
`wins.push` ailleurs est refuse.

CE QUE CE VERIFICATEUR NE PEUT PAS VOIR
---------------------------------------
Un appelant qui reconstruit la liste autrement (`insert`, `splice`,
`extend`). Il attrape la forme qui a produit le defaut, pas toutes les formes
imaginables. Les deplacements internes -- `remove` puis `push` pour remonter
une fenetre -- sont legitimes et signales comme tels par un commentaire dedie.

Code de retour : 0 si la regle est respectee.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
CIBLE = RACINE / "src" / "gui" / "window_manager.rs"

POUSSE = re.compile(r"\bwins\s*\.\s*push\s*\(")
# Les deux seules fonctions autorisees a pousser, et leurs signatures.
#
#   * `ouvre_fenetre` : une fenetre APPARAIT, et annonce le plein ecran ;
#   * `remonte_fenetre` : une fenetre DEJA dessinee remonte, et c'est le degat
#     de focus qui l'annonce.
#
# Les distinguer n'est pas cosmetique : confondre les deux, c'est soit repeindre
# tout l'ecran pour un simple changement de plan, soit ne rien repeindre pour
# une fenetre qui vient d'exister.
OUVERTURE = "fn ouvre_fenetre(wins: &mut Vec<Win>, fenetre: Win, degats: &mut Degats)"
REMONTEE = "fn remonte_fenetre(wins: &mut Vec<Win>, index: usize) -> usize"
AUTORISEES = (OUVERTURE, REMONTEE)


def sans_commentaires(ligne: str) -> str:
    position = ligne.find("//")
    return ligne if position < 0 else ligne[:position]


def verifie(chemin: Path) -> list[str]:
    lignes = chemin.read_text(encoding="utf-8").splitlines()
    manquantes = [
        signature
        for signature in AUTORISEES
        if not any(signature in ligne for ligne in lignes)
    ]
    if manquantes:
        return [
            f"{chemin.name} : `{signature}` est introuvable.\n"
            "           C'est une des deux portes d'entree d'une fenetre dans "
            "le bureau ;\n"
            "           sans elle, la regle n'a plus de sens."
            for signature in manquantes
        ]

    fautes: list[str] = []
    dans_helper = False
    profondeur = 0

    for numero, brute in enumerate(lignes, 1):
        ligne = sans_commentaires(brute)

        if any(signature in ligne for signature in AUTORISEES):
            dans_helper = True
            profondeur = 0

        if dans_helper:
            profondeur += ligne.count("{") - ligne.count("}")

        if POUSSE.search(ligne):
            if not dans_helper:
                fautes.append(
                    f"{chemin.name}:{numero} `wins.push` hors de `ouvre_fenetre`.\n"
                    f"           {brute.strip()}\n"
                    "           Une fenetre poussee directement apparait sans "
                    "que personne ne l'annonce :\n"
                    "           elle n'est peinte qu'au prochain degat qui "
                    "passe par la.\n"
                    "           Passe par `ouvre_fenetre` (apparition) ou "
                    "`remonte_fenetre` (reordonnancement)."
                )

        if dans_helper and profondeur <= 0 and "{" in "".join(lignes[:numero]):
            if ligne.strip().startswith("}") or (profondeur == 0 and "}" in ligne):
                dans_helper = False

    return fautes


def main() -> int:
    if not CIBLE.exists():
        print(f"ECHEC  fichier introuvable : {CIBLE}")
        return 1
    fautes = verifie(CIBLE)
    if fautes:
        print("ECHEC  ouverture de fenetre sans degat\n")
        for faute in fautes:
            print(f"  {faute}\n")
        return 1
    print(f"ok  {CIBLE.relative_to(RACINE)} : toute fenetre passe par ouvre_fenetre")
    return 0


if __name__ == "__main__":
    sys.exit(main())
