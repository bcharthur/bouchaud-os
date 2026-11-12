#!/usr/bin/env python3
"""BOUCHAUD_BUREAU_RING3_2A/2B -- l'ecran compose par `composited`, decode au pixel.

    verifie_capture_bureau_ring3.py CAPTURE.ppm FENETRE...
    verifie_capture_bureau_ring3.py CAPTURE.ppm vide

FENETRE = `X,Y,L,H` : la zone de CONTENU d'une fenetre de `mire-gpu` (ce que
`composited` publie), du FOND vers le PREMIER PLAN. Chaque pixel de l'ecran
est classe selon la fenetre la plus haute qui le couvre :

  - contenu : le pixel EXACT de la mire du client (`mire-gpu` : chaque pixel
    encode sa position dans SA surface) ou son carre blanc mobile (borne) ;
  - cadre : bordure, barre de titre active (premier plan) ou inactive ;
  - ailleurs : le fond du bureau.
Aucun autre pixel n'est admis : pas de zone vide (contenu non recopie), pas de
pixel decale, pas de fenetre du dessous qui deborde sur celle du dessus, pas de
trace d'un autre compositeur. `vide` : tout l'ecran est le fond (les fenetres
des clients morts ont disparu).

`X,Y,L,H,o` : fenetre OPAQUE (DOOM, Ladybird : contenu non decodable). Son
cadre, sa barre et sa place dans la pile sont juges comme les autres ; de son
contenu, on exige qu'il soit reellement peint -- moins de la moitie de ses
pixels visibles a la couleur du fond.

Rend 0 et `BUREAU_RING3_CAPTURE_OK ...`, ou 1 et `BUREAU_RING3_CAPTURE_ECHEC ...`.
"""
from __future__ import annotations

import sys
from pathlib import Path

FOND = 0x1E3A5F
BORD = 0x0B1A2A
BARRE = 0x2F6FB0
BARRE_INACTIVE = 0x46607A
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


def classe(largeur: int, hauteur: int, pixels: bytes, fenetres: list) -> dict:
    """`fenetres` : (x, y, l, h) du fond vers le premier plan."""
    haut = len(fenetres) - 1
    opaques = [len(f) == 5 for f in fenetres]
    fenetres = [f[:4] for f in fenetres]
    cadres = [(x - BORD_L, y - BARRE_H - BORD_L, x + l + BORD_L, y + h + BORD_L) for x, y, l, h in fenetres]
    fond_dans = [0] * len(fenetres)
    comptes = {"exacts": 0, "carre": 0, "opaques": 0, "cadre": 0, "fond": 0, "autres": 0}
    par_fenetre = [0] * len(fenetres)
    premier_autre = None
    for y in range(hauteur):
        ligne = memoryview(pixels)[y * largeur * 3 : (y + 1) * largeur * 3]
        for x in range(largeur):
            o = 3 * x
            v = (ligne[o] << 16) | (ligne[o + 1] << 8) | ligne[o + 2]
            juge = None
            for k in range(haut, -1, -1):
                cx0, cy0, cx1, cy1 = cadres[k]
                if not (cx0 <= x < cx1 and cy0 <= y < cy1):
                    continue
                fx, fy, fl, fh = fenetres[k]
                if fx <= x < fx + fl and fy <= y < fy + fh and opaques[k]:
                    juge = "opaques"
                    par_fenetre[k] += 1
                    fond_dans[k] += v == FOND
                elif fx <= x < fx + fl and fy <= y < fy + fh:
                    if v == motif(x - fx, y - fy):
                        juge = "exacts"
                        par_fenetre[k] += 1
                    elif v == BLANC:
                        juge = "carre"
                        par_fenetre[k] += 1
                elif fy - BARRE_H <= y < fy and fx <= x < fx + fl:
                    if v == (BARRE if k == haut else BARRE_INACTIVE):
                        juge = "cadre"
                elif v == BORD:
                    juge = "cadre"
                break
            else:
                if v == FOND:
                    juge = "fond"
            if juge is None:
                comptes["autres"] += 1
                if premier_autre is None:
                    premier_autre = (x, y, v)
            else:
                comptes[juge] += 1
    comptes["premier_autre"] = premier_autre
    comptes["par_fenetre"] = par_fenetre
    comptes["fond_dans"] = fond_dans
    comptes["opaques_k"] = opaques
    return comptes


def visible(fenetres: list, k: int, largeur: int, hauteur: int) -> int:
    """Pixels de contenu de la fenetre k que rien au-dessus ne cache."""
    x, y, l, h = fenetres[k]
    n = 0
    for py in range(max(0, y), min(hauteur, y + h)):
        for px in range(max(0, x), min(largeur, x + l)):
            cache = False
            for j in range(k + 1, len(fenetres)):
                fx, fy, fl, fh = fenetres[j]
                if fx - BORD_L <= px < fx + fl + BORD_L and fy - BARRE_H - BORD_L <= py < fy + fh + BORD_L:
                    cache = True
                    break
            n += not cache
    return n


def juge(comptes: dict, fenetres: list, largeur: int, hauteur: int) -> list:
    raisons = []
    if comptes["autres"]:
        x, y, v = comptes["premier_autre"]
        raisons.append(f"autres={comptes['autres']} premier=({x},{y})={v:#08x}")
    if comptes["carre"] > CARRE_MAX * max(1, len(fenetres)):
        raisons.append(f"carre={comptes['carre']}")
    geometries = [f[:4] for f in fenetres]
    for k in range(len(fenetres)):
        attendu = visible(geometries, k, largeur, hauteur)
        if comptes["par_fenetre"][k] != attendu:
            raisons.append(f"fenetre{k}={comptes['par_fenetre'][k]}!={attendu}")
        if comptes["opaques_k"][k] and attendu and 2 * comptes["fond_dans"][k] >= attendu:
            raisons.append(f"fenetre{k}-non-peinte fond={comptes['fond_dans'][k]}/{attendu}")
    if fenetres and comptes["cadre"] == 0:
        raisons.append("cadre-absent")
    return raisons


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    largeur, hauteur, pixels = lit_ppm(Path(argv[1]))
    if argv[2:] == ["vide"]:
        fenetres = []
    else:
        try:
            fenetres = []
            for a in argv[2:]:
                champs = a.split(",")
                opaque = len(champs) == 5 and champs[4] == "o"
                f = tuple(int(v) for v in champs[:4])
                fenetres.append(f + ("o",) if opaque else f)
                if len(champs) not in (4, 5) or (len(champs) == 5 and not opaque):
                    raise ValueError(a)
        except ValueError:
            print(__doc__, file=sys.stderr)
            return 2
        if any(len(f) not in (4, 5) for f in fenetres):
            print(__doc__, file=sys.stderr)
            return 2
    comptes = classe(largeur, hauteur, pixels, fenetres)
    raisons = juge(comptes, fenetres, largeur, hauteur)
    resume = " ".join(f"{k}={v}" for k, v in comptes.items()
                      if k not in ("premier_autre", "par_fenetre", "fond_dans", "opaques_k"))
    resume += " par_fenetre=" + ",".join(map(str, comptes["par_fenetre"]))
    etat = "vide" if not fenetres else f"fenetres={len(fenetres)}"
    if raisons:
        print(f"BUREAU_RING3_CAPTURE_ECHEC {etat} ecran={largeur}x{hauteur} {resume} raisons={';'.join(raisons)}")
        return 1
    print(f"BUREAU_RING3_CAPTURE_OK {etat} ecran={largeur}x{hauteur} {resume}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
