#!/usr/bin/env python3
"""BOUCHAUD_COMPOSITORD_PRESENTATION_V1 : la mire posee par `composited`
(ring 3, `bo_display_present`) est-elle a l'ecran, au pixel ?

  verifie_capture_composited.py CAPTURE.ppm X Y L H

CAPTURE est un `screendump` du moniteur QEMU (PPM P6). Chaque pixel de la
zone doit etre EXACTEMENT celui de la mire (meme formule que
`tools/userland/mire-gpu.c` et `composited-slice.c`) ; une capture absente,
illisible ou trop petite est un ECHEC.
"""
import sys


def lit_ppm(chemin):
    with open(chemin, "rb") as f:
        donnees = f.read()
    champs, i = [], 0
    while len(champs) < 4:
        while donnees[i:i + 1].isspace():
            i += 1
        if donnees[i:i + 1] == b"#":
            while donnees[i:i + 1] not in (b"\n", b""):
                i += 1
            continue
        j = i
        while not donnees[j:j + 1].isspace():
            j += 1
        champs.append(donnees[i:j])
        i = j
    if champs[0] != b"P6" or int(champs[3]) != 255:
        raise ValueError("PPM P6 8 bits attendu")
    return int(champs[1]), int(champs[2]), donnees[i + 1:]


def attendu(x, y):
    return (y & 0xFF, x & 0xFF, 0x80 | ((y >> 8) << 3) | (x >> 8))


def main():
    chemin = sys.argv[1]
    ox, oy, l, h = (int(v) for v in sys.argv[2:6])
    try:
        w, hh, px = lit_ppm(chemin)
    except (OSError, ValueError, IndexError) as e:
        print(f"COMPOSITED_CAPTURE_ECHEC capture illisible : {e}")
        return 1
    if ox + l > w or oy + h > hh:
        print(f"COMPOSITED_CAPTURE_ECHEC ecran {w}x{hh} trop petit")
        return 1
    exacts = faux = 0
    for y in range(h):
        for x in range(l):
            o = ((oy + y) * w + ox + x) * 3
            if tuple(px[o:o + 3]) == attendu(x, y):
                exacts += 1
            else:
                faux += 1
    ok = faux == 0 and exacts == l * h
    print(f"COMPOSITED_CAPTURE_{'OK' if ok else 'ECHEC'} ecran={w}x{hh} zone={l}x{h}@{ox},{oy} exacts={exacts} faux={faux}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
