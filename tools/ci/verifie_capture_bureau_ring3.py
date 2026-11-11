#!/usr/bin/env python3
"""BOUCHAUD_BUREAU_RING3_2A -- l'ecran compose par `composited`, decode au pixel.

    verifie_capture_bureau_ring3.py CAPTURE.ppm X Y L H vivant|mort

`X Y L H` : la zone de CONTENU de la fenetre (ce que `composited` publie en
`fenetre_x/fenetre_y`, 640x400 en 2A). Chaque pixel de l'ecran est classe :

  vivant  - contenu : le pixel EXACT de la mire du client (`mire-gpu` : chaque
            pixel encode sa position) ou le carre blanc mobile (borne) ;
          - cadre : bordure et barre de titre, couleurs du bureau ;
          - ailleurs : le fond du bureau.
          Aucun autre pixel n'est admis : pas de zone vide (contenu non
          recopie), pas de pixel decale, pas de trace d'un autre compositeur.
  mort    - tout l'ecran est le fond : la fenetre d'un client mort a disparu.

Rend 0 et `BUREAU_RING3_CAPTURE_OK ...`, ou 1 et `BUREAU_RING3_CAPTURE_ECHEC ...`.
"""
from __future__ import annotations

import sys
from pathlib import Path

FOND = 0x1E3A5F
BORD = 0x0B1A2A
BARRE = 0x2F6FB0
BLANC = 0xFFFFFF
BARRE_H = 24
BORD_L = 2
# Carre mobile de 32x32 : au plus deux positions visibles dans une capture
# prise entre la restauration de l'ancien et la pose du nouveau.
CARRE_MAX = 2 * 32 * 32


def motif(x: int, y: int) -> int:
    return ((y & 0xFF) << 16) | ((x & 0xFF) << 8) | 0x80 | ((y >> 8) << 3) | (x >> 8)


def lit_ppm(chemin: Path) -> tuple[int, int, bytes]:
    data = chemin.read_bytes()
    jetons: list[bytes] = []
    i = 0
    while len(jetons) < 4:
        while i < len(data) and data[i : i + 1].isspace():
            i += 1
        if i < len(data) and data[i : i + 1] == b"#":
            while i < len(data) and data[i : i + 1] != b"\n":
                i += 1
            continue
        debut = i
        while i < len(data) and not data[i : i + 1].isspace():
            i += 1
        if debut == i:
            raise ValueError("en-tete PPM tronque")
        jetons.append(data[debut:i])
    if jetons[0] != b"P6" or int(jetons[3]) != 255:
        raise ValueError("PPM P6 8 bits attendu")
    largeur, hauteur = int(jetons[1]), int(jetons[2])
    pixels = data[i + 1 :]
    if len(pixels) < largeur * hauteur * 3:
        raise ValueError(f"PPM tronque : {len(pixels)} octets pour {largeur}x{hauteur}")
    return largeur, hauteur, pixels


def classe(largeur: int, hauteur: int, pixels: bytes, fen: tuple[int, int, int, int], etat: str) -> dict:
    x0, y0, l, h = fen
    cadre = (x0 - BORD_L, y0 - BARRE_H - BORD_L, x0 + l + BORD_L, y0 + h + BORD_L)
    comptes = {"exacts": 0, "carre": 0, "cadre": 0, "fond": 0, "autres": 0}
    premier_autre = None
    for y in range(hauteur):
        ligne = memoryview(pixels)[y * largeur * 3 : (y + 1) * largeur * 3]
        for x in range(largeur):
            o = 3 * x
            v = (ligne[o] << 16) | (ligne[o + 1] << 8) | ligne[o + 2]
            if etat == "vivant" and x0 <= x < x0 + l and y0 <= y < y0 + h:
                if v == motif(x - x0, y - y0):
                    comptes["exacts"] += 1
                    continue
                if v == BLANC:
                    comptes["carre"] += 1
                    continue
            elif etat == "vivant" and cadre[0] <= x < cadre[2] and cadre[1] <= y < cadre[3]:
                barre = cadre[1] + BORD_L <= y < y0 and x0 <= x < x0 + l
                if v == (BARRE if barre else BORD):
                    comptes["cadre"] += 1
                    continue
            elif v == FOND:
                comptes["fond"] += 1
                continue
            comptes["autres"] += 1
            if premier_autre is None:
                premier_autre = (x, y, v)
    comptes["premier_autre"] = premier_autre
    return comptes


def juge(comptes: dict, fen: tuple[int, int, int, int], etat: str) -> list[str]:
    _, _, l, h = fen
    raisons = []
    if comptes["autres"]:
        x, y, v = comptes["premier_autre"]
        raisons.append(f"autres={comptes['autres']} premier=({x},{y})={v:#08x}")
    if etat == "vivant":
        if comptes["carre"] > CARRE_MAX:
            raisons.append(f"carre={comptes['carre']}>{CARRE_MAX}")
        if comptes["exacts"] + comptes["carre"] != l * h:
            raisons.append(f"contenu={comptes['exacts'] + comptes['carre']}!={l * h}")
        if comptes["cadre"] == 0:
            raisons.append("cadre-absent")
    else:
        if comptes["exacts"] or comptes["cadre"]:
            raisons.append("fenetre-restee")
    return raisons


def main(argv: list[str]) -> int:
    if len(argv) != 7 or argv[6] not in ("vivant", "mort"):
        print(__doc__, file=sys.stderr)
        return 2
    largeur, hauteur, pixels = lit_ppm(Path(argv[1]))
    fen = tuple(int(a) for a in argv[2:6])
    comptes = classe(largeur, hauteur, pixels, fen, argv[6])
    raisons = juge(comptes, fen, argv[6])
    resume = " ".join(f"{k}={v}" for k, v in comptes.items() if k != "premier_autre")
    if raisons:
        print(f"BUREAU_RING3_CAPTURE_ECHEC etat={argv[6]} ecran={largeur}x{hauteur} {resume} raisons={';'.join(raisons)}")
        return 1
    print(f"BUREAU_RING3_CAPTURE_OK etat={argv[6]} ecran={largeur}x{hauteur} {resume}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
