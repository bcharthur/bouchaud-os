#!/usr/bin/env python3
"""Ce que l'ecran virtio-gpu montre vraiment (BOUCHAUD_VIRTIO_GPU_2D_V1).

Les captures viennent du moniteur QEMU (`screendump`), c'est-a-dire de ce que
le peripherique PRESENTE, pas de ce que le noyau croit avoir ecrit.

    analyse_scanout.py mire CAPTURE.ppm
        La mire de l'autotest : 64x64, quatre quadrants purs (rouge, vert,
        bleu, blanc), exacts au pixel pres -- marge de 2 pixels autour des
        frontieres. Prouve memoire -> TRANSFER_TO_HOST_2D -> RESOURCE_FLUSH ->
        SET_SCANOUT.

    analyse_scanout.py compare VIRTIO.ppm REFERENCE.ppm [--min-identiques 0.90]
        Le bureau presente par virtio-gpu contre le meme bureau presente par
        BGA : meme taille, et au moins 90 % des pixels identiques (l'horloge et
        le curseur bougent entre deux demarrages).

    analyse_scanout.py vivant AVANT.ppm APRES.ppm
        Deux captures du meme ecran a quelques secondes : l'horloge a change,
        donc les presentations partielles atteignent l'ecran.

    analyse_scanout.py taille CAPTURE.ppm LARGEUR HAUTEUR

Sortie 0 si la verification passe, 1 sinon ; une ligne de verdict.
"""
import sys

MIRE = [(0xFF, 0x00, 0x00), (0x00, 0xFF, 0x00), (0x00, 0x00, 0xFF), (0xFF, 0xFF, 0xFF)]
COTE = 64


def lit_ppm(chemin):
    d = open(chemin, "rb").read()
    champs, i = [], 0
    while len(champs) < 4:
        while d[i:i + 1].isspace():
            i += 1
        if d[i:i + 1] == b"#":
            while d[i:i + 1] not in (b"\n", b""):
                i += 1
            continue
        j = i
        while not d[j:j + 1].isspace():
            j += 1
        champs.append(d[i:j])
        i = j
    if champs[0] != b"P6" or champs[3] != b"255":
        raise ValueError(f"{chemin} : pas un PPM P6 8 bits")
    largeur, hauteur = int(champs[1]), int(champs[2])
    pixels = d[i + 1:i + 1 + largeur * hauteur * 3]
    if len(pixels) != largeur * hauteur * 3:
        raise ValueError(f"{chemin} : capture tronquee")
    return largeur, hauteur, pixels


def pixel(px, largeur, x, y):
    o = (y * largeur + x) * 3
    return px[o], px[o + 1], px[o + 2]


def mire(chemin):
    largeur, hauteur, px = lit_ppm(chemin)
    if (largeur, hauteur) != (COTE, COTE):
        print(f"SCANOUT_MIRE_ECHEC taille={largeur}x{hauteur} attendue={COTE}x{COTE}")
        return 1
    fautes, verifies = 0, 0
    for y in range(COTE):
        for x in range(COTE):
            if abs(x - COTE // 2) < 2 or abs(y - COTE // 2) < 2:
                continue
            q = (y >= COTE // 2) * 2 + (x >= COTE // 2)
            verifies += 1
            if pixel(px, largeur, x, y) != MIRE[q]:
                fautes += 1
    if fautes:
        coins = [pixel(px, largeur, x, y) for x, y in ((8, 8), (56, 8), (8, 56), (56, 56))]
        print(f"SCANOUT_MIRE_ECHEC pixels_faux={fautes}/{verifies} coins={coins}")
        return 1
    print(f"SCANOUT_MIRE_OK {COTE}x{COTE} pixels_exacts={verifies}/{verifies} quadrants=rouge,vert,bleu,blanc")
    return 0


def compare(virtio, reference, minimum):
    lv, hv, pv = lit_ppm(virtio)
    lr, hr, pr = lit_ppm(reference)
    if (lv, hv) != (lr, hr):
        print(f"SCANOUT_BUREAU_ECHEC taille_virtio={lv}x{hv} taille_reference={lr}x{hr}")
        return 1
    total = lv * hv
    identiques = sum(1 for k in range(total) if pv[3 * k:3 * k + 3] == pr[3 * k:3 * k + 3])
    # Un ecran uni (tout noir, tout de la meme couleur) serait « identique » a
    # un autre ecran uni : on exige une image, pas un aplat.
    couleurs = len({pv[3 * k:3 * k + 3] for k in range(0, total, 7)})
    part = identiques / total
    verdict = "OK" if part >= minimum and couleurs >= 16 else "ECHEC"
    print(f"SCANOUT_BUREAU_{verdict} {lv}x{hv} pixels_identiques={identiques}/{total} ({part:.3f}, seuil {minimum}) couleurs_distinctes>={couleurs}")
    return 0 if verdict == "OK" else 1


def vivant(avant, apres):
    """Deux captures de l'ecran virtio a quelques secondes d'intervalle.

    L'horloge de la barre du haut (secondes affichees) DOIT avoir change : la
    seule facon qu'un pixel change a l'ecran virtio est un TRANSFER_TO_HOST_2D
    suivi d'un RESOURCE_FLUSH. Une premiere trame figee, puis plus rien,
    echoue ici.
    """
    la, ha, pa = lit_ppm(avant)
    lb, hb, pb = lit_ppm(apres)
    if (la, ha) != (lb, hb):
        print(f"SCANOUT_VIVANT_ECHEC tailles {la}x{ha} {lb}x{hb}")
        return 1
    horloge = differents = 0
    for y in range(ha):
        for x in range(la):
            o = 3 * (y * la + x)
            if pa[o:o + 3] != pb[o:o + 3]:
                differents += 1
                if y < 30 and x >= la - 120:
                    horloge += 1
    verdict = "OK" if horloge > 0 else "ECHEC"
    print(f"SCANOUT_VIVANT_{verdict} pixels_changes={differents} dont_horloge={horloge}")
    return 0 if verdict == "OK" else 1


def taille(chemin, largeur, hauteur):
    l, h, _ = lit_ppm(chemin)
    ok = (l, h) == (largeur, hauteur)
    print(f"SCANOUT_TAILLE_{'OK' if ok else 'ECHEC'} {l}x{h} attendue={largeur}x{hauteur}")
    return 0 if ok else 1


def main(argv):
    try:
        return verifie(argv)
    except (OSError, ValueError) as e:
        # Une capture absente ou illisible est un echec, jamais un saut.
        print(f"SCANOUT_CAPTURE_ECHEC {e}")
        return 1


def verifie(argv):
    if len(argv) >= 3 and argv[1] == "mire":
        return mire(argv[2])
    if len(argv) >= 4 and argv[1] == "compare":
        minimum = 0.90
        if "--min-identiques" in argv:
            minimum = float(argv[argv.index("--min-identiques") + 1])
        return compare(argv[2], argv[3], minimum)
    if len(argv) == 4 and argv[1] == "vivant":
        return vivant(argv[2], argv[3])
    if len(argv) == 5 and argv[1] == "taille":
        return taille(argv[2], int(argv[3]), int(argv[4]))
    print(__doc__, file=sys.stderr)
    sys.exit(2)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
