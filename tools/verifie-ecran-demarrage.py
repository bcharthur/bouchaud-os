#!/usr/bin/env python3
"""Garde-fou : l'ecran de demarrage dit ce qui se passe, et ne peut pas tuer le boot.

# Ce qu'il remplace

Le demarrage n'affichait qu'un « B » bleu centre et immobile pendant tout
l'amorcage. Devant un ecran fige, on ne peut ni savoir si la machine travaille,
ni savoir OU elle est lente -- et c'est precisement la question posee par
l'utilisateur : « le demarrage est trop long ».

# LA CONTRAINTE QUI A DEJA COUTE UN BOOT

Le depot garde la trace d'une tentative precedente, dans
`stage2::point_de_controle` :

    « Drawing here caused a double fault on the Trigkey immediately after the
      network checkpoint. »

Dessiner au point de controle avec le rendu TrueType demande d'analyser une
police -- donc d'ALLOUER et de descendre profond dans la pile --, a un instant
ou les interruptions peuvent etre masquees et ou la pile d'amorcage fait
quatre-vingts kilo-octets.

Le seul moteur admissible ici est celui de l'ecran de FAUTE : atlas
pre-construit a la compilation, aucune allocation, aucun verrou, ecritures
volatiles et atomiques. C'est, par construction, le seul chemin d'affichage
dont on sait qu'il tient pendant une double faute.

# Ce qui est verifie

1. L'ecran vit dans le module de l'ecran de faute, avec son moteur.
2. Il n'alloue pas, ne verrouille pas, n'appelle pas la police TrueType.
3. Il est OUVERT au demarrage et FERME a l'arrivee au bureau.
4. Chaque point de controle porte son horodatage et son delta.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
ECRAN = RACINE / "src/platform/pc/ecran_faute.rs"
STAGE2 = RACINE / "src/platform/pc/stage2.rs"


def corps(source, entete):
    debut = source.find(entete)
    if debut < 0:
        return None
    i = source.find("{", debut)
    if i < 0:
        return None
    profondeur = 0
    for j in range(i, len(source)):
        if source[j] == "{":
            profondeur += 1
        elif source[j] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[i:j + 1]
    return None


def main():
    fautes = []
    for chemin in (ECRAN, STAGE2):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1
    ecran = ECRAN.read_text(encoding="utf-8")
    stage2 = STAGE2.read_text(encoding="utf-8")

    etape = corps(ecran, "pub fn demarrage_etape(")
    if etape is None:
        fautes.append(
            "ecran_faute.rs : l'ecran de demarrage a disparu. Le logo fige ne "
            "dit ni si la machine travaille, ni ou elle est lente."
        )
    else:
        # --- LA REGLE QUI A DEJA COUTE UN BOOT ------------------------------
        for interdit, raison in (
            ("Font::", "il analyserait une police TrueType"),
            ("fontdue", "il analyserait une police TrueType"),
            ("alloc::", "il allouerait"),
            ("Vec::", "il allouerait"),
            ("String", "il allouerait"),
            ("format!", "il allouerait pour formater"),
            ("smp_lock::enter", "il prendrait le gros verrou"),
            (".lock()", "il prendrait un verrou"),
        ):
            if interdit in etape:
                fautes.append(
                    "ecran_faute.rs : l'ecran de demarrage n'est plus sur au "
                    "point de controle (%s). Dessiner ainsi a deja produit une "
                    "double faute sur le Trigkey, juste apres le point reseau."
                    % raison
                )
        if "texte(" not in etape:
            fautes.append(
                "ecran_faute.rs : l'ecran de demarrage n'utilise plus le rendu "
                "de l'ecran de faute -- le seul dont on sait qu'il tient a "
                "n'importe quel moment de l'amorcage."
            )

    # --- 3. Ouvert au demarrage, FERME au bureau ----------------------------
    if "demarrage_ouvre()" not in stage2:
        fautes.append("stage2.rs : l'ecran de demarrage n'est plus ouvert.")
    if "demarrage_ferme()" not in stage2:
        fautes.append(
            "stage2.rs : l'ecran de demarrage n'est plus ferme a l'arrivee au "
            "bureau. Des points de controle existent encore apres : la barre "
            "de progression repeindrait par-dessus les fenetres."
        )
    else:
        rang_ferme = stage2.find("demarrage_ferme()")
        rang_bureau = stage2.find('point_de_controle("bureau")')
        if rang_bureau >= 0 and rang_ferme < rang_bureau:
            fautes.append(
                "stage2.rs : l'ecran est ferme AVANT le point bureau ; la "
                "derniere etape ne s'afficherait jamais."
            )

    # --- 4. Chaque point porte son temps ------------------------------------
    point = corps(ecran, "pub fn point(")
    if point is None:
        fautes.append("ecran_faute.rs : le point de controle a disparu.")
    else:
        for champ in ("t_ms=", "delta_ms="):
            if champ not in point:
                fautes.append(
                    "ecran_faute.rs : le point de controle ne publie plus "
                    "`%s`. « Le demarrage est trop long » ne se corrige pas "
                    "sans savoir OU part le temps." % champ
                )

    if fautes:
        print("ecran de demarrage : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "ecran de demarrage : rendu sans allocation ni verrou, ouvert puis "
        "ferme au bureau, chaque etape horodatee"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
