#!/usr/bin/env python3
"""L'atlas du chrome contient-il de vraies lettres, accents compris ?

# Ce que ce test protege

Le chrome dessinait sa barre d'adresse, ses boutons et son titre avec une
bitmap 8x8 d'un bit par pixel, agrandie par un facteur entier : un escalier a
chaque diagonale, et strictement aucune lettre accentuee -- la table s'arretait
a `0x7e`.

Il melange desormais un atlas de DejaVu Sans rasterise a la construction. Le
gain n'existe que si l'atlas est REELLEMENT bon, et un atlas faux ne se voit
pas ici : il se voit vingt minutes plus tard, dans une fenetre de navigateur.

Les pieges sont precis, et ce sont eux qu'on verifie :

  * un glyphe COMPOSITE mal resolu -- « e accent aigu » est une composition de
    deux glyphes -- rend la lettre nue, sans son accent ;
  * un decalage hors bornes lit la couverture d'un autre glyphe ;
  * une avance nulle empile toutes les lettres au meme endroit ;
  * une couverture entierement opaque ou entierement vide veut dire que la
    rasterisation n'a rien produit d'utile.

Lance par `tools/ladybird/chrome/test-atlas.sh`.
"""

import re
import sys
from pathlib import Path

ICI = Path(__file__).resolve().parent
ATLAS = ICI / "BouchaudAtlas.h"

GLYPHE = re.compile(
    r"\{ 0x([0-9a-f]{4}), (-?\d+), (-?\d+), (-?\d+), (\d+), (\d+), (\d+) \}")


def charge():
    source = ATLAS.read_text(encoding="utf-8")
    glyphes = {}
    for trouve in GLYPHE.finditer(source):
        point, avance, gauche, haut, largeur, hauteur, decalage = trouve.groups()
        glyphes[int(point, 16)] = {
            "avance": int(avance), "gauche": int(gauche), "haut": int(haut),
            "largeur": int(largeur), "hauteur": int(hauteur),
            "decalage": int(decalage),
        }
    brut = re.search(r"couverture\[\] = \{(.*?)\n\};", source, re.S).group(1)
    couverture = [int(v) for v in brut.replace("\n", "").split(",") if v.strip()]
    return glyphes, couverture, source


def pixels(glyphes, couverture, point):
    g = glyphes[point]
    debut = g["decalage"]
    return couverture[debut : debut + g["largeur"] * g["hauteur"]]


def main() -> int:
    if not ATLAS.exists():
        print(f"atlas absent : {ATLAS}")
        return 1
    glyphes, couverture, source = charge()
    fautes = []

    declare = int(re.search(r"nombre = (\d+)", source).group(1))
    if declare != len(glyphes):
        fautes.append(f"  `nombre` vaut {declare} pour {len(glyphes)} glyphes lus")

    # --- L'ASCII imprimable au complet -------------------------------------
    for point in range(0x20, 0x7F):
        if point not in glyphes:
            fautes.append(f"  U+{point:04X} ({chr(point)!r}) absent de l'atlas")

    # --- Les accents, la raison d'etre du changement ------------------------
    accents = "àâäçèéêëîïôöùûüÀÂÄÇÈÉÊËÎÏÔÖÙÛÜ"
    for caractere in accents:
        point = ord(caractere)
        if point not in glyphes:
            fautes.append(f"  {caractere!r} absent : c'est le carre vide qu'on "
                          f"voyait a l'ecran")

    # --- Un composite doit differer de sa base ------------------------------
    #
    # C'est LE test du resolveur de glyphes composites. « e » et « e accent
    # aigu » partagent leur contour de base ; si la composition n'a pas ete
    # resolue, l'atlas contient deux fois la meme lettre nue.
    for accentue, base in [("é", "e"), ("è", "e"), ("à", "a"), ("ç", "c"),
                           ("ô", "o"), ("ü", "u"), ("É", "E")]:
        pa, pb = ord(accentue), ord(base)
        if pa not in glyphes or pb not in glyphes:
            continue
        if glyphes[pa]["hauteur"] <= glyphes[pb]["hauteur"]:
            fautes.append(
                f"  {accentue!r} n'est pas plus haut que {base!r} : le glyphe "
                f"composite n'a pas ete resolu, l'accent manque"
            )
        if pixels(glyphes, couverture, pa) == pixels(glyphes, couverture, pb):
            fautes.append(f"  {accentue!r} a exactement les pixels de {base!r}")

    # --- Bornes, avances, couvertures --------------------------------------
    for point, g in sorted(glyphes.items()):
        fin = g["decalage"] + g["largeur"] * g["hauteur"]
        if fin > len(couverture):
            fautes.append(
                f"  U+{point:04X} deborde la couverture ({fin} > "
                f"{len(couverture)}) : il lirait les pixels d'un autre glyphe")
            continue
        if g["avance"] <= 0 and point != 0x20:
            fautes.append(f"  U+{point:04X} a une avance de {g['avance']} : "
                          f"les lettres s'empileraient")
        if g["largeur"] == 0:
            continue
        valeurs = pixels(glyphes, couverture, point)
        if point == 0x20:
            continue
        if all(v == 0 for v in valeurs):
            fautes.append(f"  U+{point:04X} est entierement vide")
        elif all(v == 255 for v in valeurs):
            fautes.append(f"  U+{point:04X} est un rectangle plein : la "
                          f"rasterisation a rate le contour")

    # --- L'antialiassage, la seconde raison d'etre --------------------------
    #
    # Une couverture qui ne vaudrait que 0 ou 255 serait une bitmap de plus.
    intermediaires = sum(1 for v in couverture if 0 < v < 255)
    if intermediaires < len(couverture) // 10:
        fautes.append(
            f"  seulement {intermediaires} octets de couverture partielle sur "
            f"{len(couverture)} : l'atlas n'est pas antialiase")

    if fautes:
        print("atlas du chrome : regle violee")
        print("\n".join(fautes[:20]))
        if len(fautes) > 20:
            print(f"  ... et {len(fautes) - 20} autre(s)")
        return 1

    print(
        f"ok  BouchaudAtlas.h : {len(glyphes)} glyphes, accents composes "
        f"resolus, {intermediaires} octets de couverture partielle"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
