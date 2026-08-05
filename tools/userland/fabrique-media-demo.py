#!/usr/bin/env python3
"""Fabrique la video et le son de la demonstration media.

    ./fabrique-media-demo.py <destination>

Produit `demo-media.avi` (MJPEG) et `demo-media.wav` (PCM), que
`navigateur/demo-media.html` lit avec `<video>` et `<audio>`.

## Pourquoi fabriquer plutot que telecharger

Une video d'exemple prise sur le web ferait dependre la demonstration du reseau
et de droits d'usage, pour un contenu qu'on ne maitrise pas. Trente images
generees ici pesent quelques dizaines de kilo-octets, ne dependent de rien, et
montrent exactement ce qu'on veut montrer : que les trames se succedent au bon
rythme.

MJPEG plutot que H.264 parce que ce script tourne sur la machine de
construction, ou `cjpeg` est disponible mais pas d'encodeur H.264 — et parce que
le chemin exerce sur l'OS est le meme : demultiplexage, decodeur, mise a
l'echelle, conversion. Le decodeur H.264 est lie et verifie a part
(`media-probe.py`).
"""

import math
import os
import struct
import subprocess
import sys

LARGEUR = 480
HAUTEUR = 270
IMAGES = 60
IMAGES_PAR_SECONDE = 15


def image(numero):
    """Une image du film : un disque qui traverse un fond en degrade."""
    avance = numero / float(IMAGES)
    centre_x = int(LARGEUR * (0.15 + 0.7 * avance))
    centre_y = int(HAUTEUR * (0.5 + 0.25 * math.sin(avance * 2 * math.pi)))
    rayon = 34

    pixels = bytearray(LARGEUR * HAUTEUR * 3)
    for y in range(HAUTEUR):
        # Fond : un degrade vertical sombre, qui glisse lentement.
        fond_r = 18 + (y * 30) // HAUTEUR
        fond_v = 32 + (y * 60) // HAUTEUR
        fond_b = 60 + (y * 90) // HAUTEUR + int(20 * avance)
        base = y * LARGEUR * 3
        for x in range(LARGEUR):
            dx = x - centre_x
            dy = y - centre_y
            position = base + x * 3
            if dx * dx + dy * dy <= rayon * rayon:
                pixels[position] = 250
                pixels[position + 1] = 190
                pixels[position + 2] = 60
            else:
                pixels[position] = fond_r
                pixels[position + 1] = fond_v
                pixels[position + 2] = min(255, fond_b)

    # Une barre de progression, pour qu'on voie d'un coup d'œil ou en est la
    # lecture meme sur une capture fixe.
    largeur_barre = int(LARGEUR * avance)
    for y in range(HAUTEUR - 12, HAUTEUR - 4):
        base = y * LARGEUR * 3
        for x in range(largeur_barre):
            position = base + x * 3
            pixels[position] = 120
            pixels[position + 1] = 220
            pixels[position + 2] = 150

    ppm = b"P6\n%d %d\n255\n" % (LARGEUR, HAUTEUR) + bytes(pixels)
    return subprocess.run(["cjpeg", "-quality", "75"], input=ppm,
                          capture_output=True, check=True).stdout


def avi(trames):
    """Assemble des JPEG en AVI. Le conteneur le plus simple a ecrire a la main."""
    def bloc(tag, charge):
        rembourrage = b"\x00" if len(charge) % 2 else b""
        return tag + struct.pack("<I", len(charge)) + charge + rembourrage

    def liste(tag, charge):
        return b"LIST" + struct.pack("<I", len(charge) + 4) + tag + charge

    plus_grande = max(len(t) for t in trames)
    avih = struct.pack("<IIIIIIIIIIIIIIII",
                       1000000 // IMAGES_PAR_SECONDE, 0, 0, 0x10, len(trames),
                       0, 1, plus_grande, LARGEUR, HAUTEUR, 0, 0, 0, 0, 0, 0)
    strh = (b"vids" + b"MJPG"
            + struct.pack("<IHHIIIIIIIIi", 0, 0, 0, 0, 1, IMAGES_PAR_SECONDE,
                          0, len(trames), plus_grande, 0, 0, 0))
    strf = struct.pack("<IiiHHIIiiII", 40, LARGEUR, HAUTEUR, 1, 24,
                       0x47504A4D, LARGEUR * HAUTEUR * 3, 0, 0, 0, 0)

    entetes = liste(b"hdrl", bloc(b"avih", avih)
                    + liste(b"strl", bloc(b"strh", strh) + bloc(b"strf", strf)))
    corps = liste(b"movi", b"".join(bloc(b"00dc", t) for t in trames))
    return (b"RIFF" + struct.pack("<I", len(entetes) + len(corps) + 4) + b"AVI "
            + entetes + corps)


def wav(secondes=4.0, frequence=44100):
    """Un arpege, en PCM 16 bits stereo."""
    notes = [261.63, 329.63, 392.00, 523.25, 392.00, 329.63]
    par_note = int(frequence * secondes / len(notes))
    echantillons = bytearray()
    for index, hauteur in enumerate(notes):
        for i in range(par_note):
            # Une enveloppe simple, pour que les notes ne claquent pas.
            avance = i / float(par_note)
            enveloppe = min(1.0, avance * 12.0) * (1.0 - avance) ** 0.6
            valeur = int(11000 * enveloppe * math.sin(2 * math.pi * hauteur * i / frequence))
            echantillons += struct.pack("<hh", valeur, valeur)

    donnees = bytes(echantillons)
    return (b"RIFF" + struct.pack("<I", 36 + len(donnees)) + b"WAVE"
            + b"fmt " + struct.pack("<IHHIIHH", 16, 1, 2, frequence,
                                    frequence * 4, 4, 16)
            + b"data" + struct.pack("<I", len(donnees)) + donnees)


def principal():
    destination = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(destination, exist_ok=True)

    trames = [image(n) for n in range(IMAGES)]
    film = avi(trames)
    son = wav()

    chemin_film = os.path.join(destination, "demo-media.avi")
    chemin_son = os.path.join(destination, "demo-media.wav")
    with open(chemin_film, "wb") as f:
        f.write(film)
    with open(chemin_son, "wb") as f:
        f.write(son)

    print("  demo-media.avi : %d images %dx%d, %d ko"
          % (IMAGES, LARGEUR, HAUTEUR, len(film) // 1024))
    print("  demo-media.wav : %.1f s, %d ko" % (4.0, len(son) // 1024))
    return 0


if __name__ == "__main__":
    sys.exit(principal())
