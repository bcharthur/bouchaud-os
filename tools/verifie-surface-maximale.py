#!/usr/bin/env python3
"""Verifie qu'un client ring 3 peut etre maximise, et que sa surface suit.

LE DEFAUT
---------
Le bouton du milieu de la barre de titre du navigateur etait dessine,
survolable, et inerte. Le clic arrivait jusqu'a `route_window_command`, qui
refusait `Maximize` parce que la fenetre portait `WindowFlags::FIXED_SURFACE`.

Le drapeau n'etait pas arbitraire : la surface partagee etait allouee a la
taille exacte de la fenetre, et agrandir le cadre n'aurait agrandi que le
cadre. Ne rien faire etait la reponse honnete tant que la surface ne suivait
pas -- mais rien ne le DISAIT a l'ecran, et un bouton qui ne fait rien ne se
distingue pas d'un bouton casse.

LA REGLE
--------
Trois choses, et le defaut revient si l'une seule cede.

1. Aucune fenetre portant `App::Navigateur` n'est creee avec `FIXED_SURFACE`.
   C'est le refus lui-meme.

2. Tout `Client::lance` recoit `zone_maximale()` comme allocation. Allouer a la
   taille de depart ferait reapparaitre le probleme d'origine sous une forme
   pire : la fenetre accepterait de grandir, et le client peindrait hors de sa
   surface.

3. Le rectangle d'une fenetre maximisee n'est ecrit qu'UNE fois. Il l'etait
   trois fois -- `toggle_max`, le `Maximize` de `route_window_command`, le
   `Snap` juste dessous. La taille de la surface en depend desormais : une
   divergence d'un pixel se verrait comme une bande sale au bord d'une fenetre
   maximisee, et serait cherchee dans le compositeur.

CE QUE CE VERIFICATEUR NE PEUT PAS VOIR
---------------------------------------
Qu'un client peigne effectivement la zone qu'on lui annonce. C'est le travail
des tests de `Geometrie` dans `src/gui/protocole.rs`, qui exercent la borne que
le compositeur applique a la recopie.

Code de retour : 0 si les trois regles sont respectees.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
GUI = RACINE / "src" / "gui"
FENETRE = GUI / "window.rs"
GESTIONNAIRE = GUI / "window_manager.rs"

# Le rectangle maximise, tel qu'il etait recopie. Toute reapparition de cette
# forme ailleurs que dans `rect_maximise` est une deuxieme verite.
RECT_MAXIMISE = re.compile(
    r"Rect::new\(\s*0\s*,\s*BAR_H as i32\s*,[^)]*(?:WIDTH|fb::WIDTH)[^)]*\)",
    re.S,
)
DEFINITION = "pub(crate) const fn rect_maximise()"

LANCE = re.compile(r"Client::lance\((.*?)\)\s*\{", re.S)


def sans_commentaires(source):
    """Blanchit les commentaires de ligne : ils citent les formes interdites.

    Blanchir, et non retirer : les positions rendues par la recherche servent a
    numeroter des lignes du fichier D'ORIGINE. Un texte raccourci les aurait
    decalees, et le verificateur aurait accuse une ligne au hasard -- ce qu'il a
    fait a sa premiere execution.
    """
    return "\n".join(
        ligne if (position := ligne.find("//")) < 0
        else ligne[:position] + " " * (len(ligne) - position)
        for ligne in source.splitlines()
    )


def regle_drapeaux(sources, fautes):
    """1. Aucune fenetre de navigateur n'est creee en surface figee."""
    for nom, source in sources.items():
        for bloc in re.finditer(r"FIXED_SURFACE", source):
            # La fenetre et son application sont creees dans le meme appel a
            # `Win::new` : on regarde les lignes qui suivent immediatement.
            queue = source[bloc.end():bloc.end() + 200]
            if "App::Navigateur" in queue:
                ligne = source[: bloc.start()].count("\n") + 1
                fautes.append(
                    "%s:%d : une fenetre App::Navigateur est creee avec "
                    "FIXED_SURFACE ; son bouton plein ecran serait inerte."
                    % (nom, ligne)
                )


def regle_allocation(sources, fautes):
    """2. Toute surface de client est allouee a la zone maximale."""
    trouves = 0
    for nom, source in sources.items():
        for appel in LANCE.finditer(source):
            trouves += 1
            ligne = source[: appel.start()].count("\n") + 1
            if "zone_maximale()" not in appel.group(1):
                fautes.append(
                    "%s:%d : Client::lance n'alloue pas a zone_maximale(). "
                    "Une fenetre maximisable dont la surface ne l'est pas "
                    "ferait peindre le client hors de sa memoire."
                    % (nom, ligne)
                )
    if trouves == 0:
        fautes.append(
            "aucun appel a Client::lance trouve : le verificateur ne verifie "
            "plus rien."
        )


def regle_definition_unique(sources, fautes):
    """3. Le rectangle maximise n'est ecrit qu'une fois."""
    definitions = sum(source.count(DEFINITION) for source in sources.values())
    if definitions != 1:
        fautes.append(
            "`rect_maximise` est defini %d fois ; il en faut exactement une."
            % definitions
        )

    for nom, source in sources.items():
        propre = sans_commentaires(source)
        for copie in RECT_MAXIMISE.finditer(propre):
            # La definition elle-meme contient evidemment le rectangle.
            avant = propre[: copie.start()]
            if DEFINITION in avant[-400:]:
                continue
            ligne = avant.count("\n") + 1
            fautes.append(
                "%s:%d : le rectangle d'une fenetre maximisee est reecrit ici. "
                "La taille de la surface partagee en depend : appeler "
                "`window::rect_maximise()`." % (nom, ligne)
            )


def main():
    sources = {}
    for chemin in (FENETRE, GESTIONNAIRE):
        if not chemin.exists():
            print("ECHEC  fichier absent : %s" % chemin.relative_to(RACINE))
            return 1
        sources[chemin.relative_to(RACINE).as_posix()] = chemin.read_text(
            encoding="utf-8"
        )

    fautes = []
    regle_drapeaux(sources, fautes)
    regle_allocation(sources, fautes)
    regle_definition_unique(sources, fautes)

    if fautes:
        for faute in fautes:
            print("ECHEC  %s" % faute)
        return 1

    print("surface maximale : navigateur maximisable, allocation accordee, "
          "rectangle defini une fois")
    return 0


if __name__ == "__main__":
    sys.exit(main())
