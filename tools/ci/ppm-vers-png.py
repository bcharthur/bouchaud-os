#!/usr/bin/env python3
"""Convertit une capture PPM du moniteur QEMU en PNG regardable.

Sans dependance : `zlib` et `struct` suffisent, et une capture qu'on ne peut
pas ouvrir ne prouve rien.
"""
import struct
import sys
import zlib


def lit_ppm(chemin):
    with open(chemin, "rb") as fp:
        donnees = fp.read()
    if not donnees.startswith(b"P6"):
        raise SystemExit("capture inattendue : en-tete %r" % donnees[:2])
    champs = []
    place = 2
    while len(champs) < 3:
        while place < len(donnees) and donnees[place : place + 1].isspace():
            place += 1
        if donnees[place : place + 1] == b"#":
            while place < len(donnees) and donnees[place : place + 1] != b"\n":
                place += 1
            continue
        debut = place
        while place < len(donnees) and not donnees[place : place + 1].isspace():
            place += 1
        champs.append(int(donnees[debut:place]))
    place += 1
    largeur, hauteur, _maximum = champs
    pixels = donnees[place : place + largeur * hauteur * 3]
    return largeur, hauteur, pixels


def ecris_png(chemin, largeur, hauteur, pixels):
    brut = bytearray()
    for ligne in range(hauteur):
        brut.append(0)
        debut = ligne * largeur * 3
        brut.extend(pixels[debut : debut + largeur * 3])

    def morceau(nom, charge):
        entete = struct.pack(">I", len(charge)) + nom
        return entete + charge + struct.pack(
            ">I", zlib.crc32(nom + charge) & 0xFFFFFFFF
        )

    with open(chemin, "wb") as fp:
        fp.write(b"\x89PNG\r\n\x1a\n")
        fp.write(morceau(b"IHDR", struct.pack(">IIBBBBB", largeur, hauteur, 8, 2, 0, 0, 0)))
        fp.write(morceau(b"IDAT", zlib.compress(bytes(brut), 6)))
        fp.write(morceau(b"IEND", b""))


def main():
    if len(sys.argv) != 3:
        raise SystemExit("usage: ppm-vers-png.py ENTREE.ppm SORTIE.png")
    largeur, hauteur, pixels = lit_ppm(sys.argv[1])
    if len(pixels) != largeur * hauteur * 3:
        raise SystemExit(
            "capture tronquee : %d octets pour %dx%d" % (len(pixels), largeur, hauteur)
        )
    ecris_png(sys.argv[2], largeur, hauteur, pixels)
    print("capture %dx%d -> %s" % (largeur, hauteur, sys.argv[2]))


if __name__ == "__main__":
    main()
