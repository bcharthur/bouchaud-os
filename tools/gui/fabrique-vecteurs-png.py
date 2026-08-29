#!/usr/bin/env python3
"""Fabrique les vecteurs de test du decodeur PNG.

La MEME image de reference, encodee de toutes les facons legales que le
decodeur du noyau doit savoir lire : les cinq filtres de ligne, un fichier qui
les melange comme le fait un vrai encodeur, et les cinq types de couleur.

Un decodeur qui ne gere que le filtre « None » lit parfaitement les fichiers
que `fabrique-icones.py` produit et rate tous les autres. Ces vecteurs sont la
pour que le premier PNG venu d'ailleurs -- une icone changee, une image
telechargee -- ne soit pas la premiere occasion de s'en apercevoir.

Aucune dependance : `zlib` et `struct` suffisent.
"""

import struct
import sys
import zlib
from pathlib import Path

RACINE = Path(__file__).resolve().parent
SORTIE = RACINE / "vecteurs-png"

COTE = 24


def reference():
    """L'image de reference, en (r, v, b, a). Doit rester identique a
    `reference()` de `tools/gui/test_png.rs`."""
    pixels = []
    for y in range(COTE):
        for x in range(COTE):
            r = x * 255 // COTE
            v = y * 255 // COTE
            b = 0xD0 if (x // 3 + y // 3) % 2 == 0 else 0x20
            a = (x + y) * 255 // (2 * COTE)
            pixels.append((r, v, b, a))
    return pixels


def morceau(genre, contenu):
    return (
        struct.pack(">I", len(contenu))
        + genre
        + contenu
        + struct.pack(">I", zlib.crc32(genre + contenu) & 0xFFFFFFFF)
    )


def paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    return b if pb <= pc else c


def filtre_ligne(numero, ligne, precedente, composantes):
    """Applique le filtre `numero` a une ligne d'octets deja brute."""
    sortie = bytearray()
    for i, x in enumerate(ligne):
        a = ligne[i - composantes] if i >= composantes else 0
        b = precedente[i] if precedente else 0
        c = precedente[i - composantes] if precedente and i >= composantes else 0
        if numero == 0:
            sortie.append(x)
        elif numero == 1:
            sortie.append((x - a) & 0xFF)
        elif numero == 2:
            sortie.append((x - b) & 0xFF)
        elif numero == 3:
            sortie.append((x - (a + b) // 2) & 0xFF)
        else:
            sortie.append((x - paeth(a, b, c)) & 0xFF)
    return sortie


def encode(chemin, lignes_brutes, composantes, type_couleur, filtres,
           palette=None, trns=None):
    """Ecrit un PNG. `filtres` donne le numero de filtre de chaque ligne."""
    flux = bytearray()
    precedente = None
    for index, ligne in enumerate(lignes_brutes):
        numero = filtres[index % len(filtres)]
        flux.append(numero)
        flux += filtre_ligne(numero, ligne, precedente, composantes)
        precedente = ligne

    fichier = b"\x89PNG\r\n\x1a\n" + morceau(
        b"IHDR", struct.pack(">IIBBBBB", COTE, COTE, 8, type_couleur, 0, 0, 0)
    )
    if palette is not None:
        fichier += morceau(b"PLTE", palette)
    if trns is not None:
        fichier += morceau(b"tRNS", trns)
    fichier += morceau(b"IDAT", zlib.compress(bytes(flux), 9))
    fichier += morceau(b"IEND", b"")
    chemin.write_bytes(fichier)


def lignes(pixels, extrait):
    """Decoupe l'image en lignes d'octets, `extrait` donnant les composantes."""
    resultat = []
    for y in range(COTE):
        ligne = bytearray()
        for x in range(COTE):
            ligne += bytes(extrait(pixels[y * COTE + x]))
        resultat.append(ligne)
    return resultat


def main():
    SORTIE.mkdir(parents=True, exist_ok=True)
    px = reference()

    rgba = lignes(px, lambda p: p)
    for numero in range(5):
        encode(SORTIE / f"filtre{numero}.png", rgba, 4, 6, [numero])
    # Un vrai encodeur change de filtre a chaque ligne.
    encode(SORTIE / "melange.png", rgba, 4, 6, [0, 1, 2, 3, 4, 2, 4, 1])

    encode(SORTIE / "rvb.png", lignes(px, lambda p: p[:3]), 3, 2, [0, 1, 4])

    # Gris : la composante rouge de la reference, pour que le test puisse
    # comparer sans ambiguite.
    encode(SORTIE / "gris.png", lignes(px, lambda p: (p[0],)), 1, 0, [0, 2, 3])
    encode(SORTIE / "gris-alpha.png", lignes(px, lambda p: (p[0], p[3])), 2, 4,
           [1, 4])

    # Palette de seize entrees, une sur deux translucide.
    palette = bytearray()
    trns = bytearray()
    for index in range(16):
        palette += bytes((index * 16, 255 - index * 16, 0x40))
        trns.append(255 if index % 2 == 0 else 96)
    indices = []
    for y in range(COTE):
        ligne = bytearray()
        for x in range(COTE):
            ligne.append((x + y) % 16)
        indices.append(ligne)
    encode(SORTIE / "palette.png", indices, 1, 3, [0, 1, 2], bytes(palette),
           bytes(trns))

    # Un vecteur dedie aux EGALITES du predicteur de Paeth.
    #
    # Le damier de reference n'en produit aucune : departager `pa`, `pb` et `pc`
    # dans le mauvais ordre le decodait parfaitement. Or c'est exactement le
    # genre d'erreur qui froisse une vraie photo sans rien casser d'autre.
    #
    # Une suite pseudo-aleatoire deterministe en produit des milliers. La graine
    # est fixe : le fichier doit etre reproductible a l'octet pres.
    etat = 0x2545F491
    bruit = []
    for _ in range(COTE * COTE):
        etat = (etat * 1103515245 + 12345) & 0xFFFFFFFF
        r = (etat >> 16) & 0xFF
        etat = (etat * 1103515245 + 12345) & 0xFFFFFFFF
        v = (etat >> 16) & 0xFF
        etat = (etat * 1103515245 + 12345) & 0xFFFFFFFF
        b = (etat >> 16) & 0xFF
        etat = (etat * 1103515245 + 12345) & 0xFFFFFFFF
        a = (etat >> 16) & 0xFF
        bruit.append((r, v, b, a))
    encode(SORTIE / "paeth.png", lignes(bruit, lambda p: p), 4, 6, [4])
    (SORTIE / "paeth.txt").write_bytes(
        b"".join(bytes(p) for p in bruit)
    )

    for fichier in sorted(SORTIE.glob("*.png")):
        print(f"  ecrit  {fichier.relative_to(RACINE.parent.parent)}  "
              f"{fichier.stat().st_size} octets")
    return 0


if __name__ == "__main__":
    sys.exit(main())
