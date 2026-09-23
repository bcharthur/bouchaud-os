#!/usr/bin/env python3
"""La mire est-elle arrivee jusqu'a la FRAME PRESENTEE ?

# La question que le banc de page ne peut pas poser

Tout le banc d'images lit ses pixels dans un `<canvas>`. Cela prouve que le
decodeur rend les bons octets. Cela ne prouve PAS qu'ils arrivent a l'ecran.

Le defaut observe sur la machine physique est exactement celui-la :

    la ressource arrive en HTTP 200, l'ImageDecoder tourne, `onload` part
    -- et l'image reste BLANCHE.

Entre le bitmap decode et le pixel affiche il reste le display list, le
peintre, le compositeur et la presentation. Aucune interface de page ne
permet de relire ce qui en sort. On regarde donc l'ECRAN DE LA MACHINE,
capture par le moniteur QEMU.

# Pourquoi compter des couleurs plutot que regarder des coordonnees

La mire pourrait etre cherchee a une position connue. Ce serait fragile :
la hauteur de la barre d'adresse, un decalage de fenetre ou un facteur
d'echelle suffiraient a faire echouer un rendu pourtant juste, et l'on
passerait la journee a ajuster des offsets.

Les trois aplats sont donc de couleur PURE et SATUREE. Le chrome d'un
navigateur est fait de gris, de blancs et de bleus d'accentuation : un
#FF0000 ou un #00FF00 exact n'y apparait pas par accident. Compter les
pixels exacts repond a la question sans rien savoir de la mise en page.

# Ce que chaque couleur accuse

    rouge   absent  -> `<img>` ne peint pas
    vert    absent  -> `background-image` ne peint pas (chemin CSS)
    bleu    absent  -> la mise a l'echelle d'un `<img>` ne peint pas

Les distinguer importe : un port peut faire marcher HTMLImageElement et pas
le chemin du style calcule, et la page parait alors « a moitie » illustree.

    python3 tools/ci/verifie-surface-mire.py <capture.ppm|png> [--min 2048]
"""
import struct
import sys
import zlib

# Les aplats sont produits par `tools/health/browser_host_fixture.py`.
COULEURS = {
    "rouge": (255, 0, 0),
    "vert": (0, 255, 0),
    "bleu": (0, 0, 255),
}

# LE SEUIL, ET D'OU IL VIENT.
#
# Le plus petit aplat fait 64x64, soit 4096 pixels. La moitie suffit a
# conclure : un aplat a moitie recouvert par le chrome reste un aplat peint.
# Exiger les 4096 rendrait le banc sensible a la mise en page, ce que le
# comptage existe justement pour eviter.
MIN_PAR_DEFAUT = 2048


def lis_ppm(octets):
    """P6 binaire, le format que `screendump` de QEMU produit."""
    if not octets.startswith(b"P6"):
        return None
    champs, pos = [], 2
    while len(champs) < 3:
        while pos < len(octets) and octets[pos : pos + 1].isspace():
            pos += 1
        if octets[pos : pos + 1] == b"#":
            while pos < len(octets) and octets[pos] != 0x0A:
                pos += 1
            continue
        debut = pos
        while pos < len(octets) and not octets[pos : pos + 1].isspace():
            pos += 1
        champs.append(int(octets[debut:pos]))
    pos += 1
    largeur, hauteur, maxi = champs
    if maxi != 255:
        raise SystemExit(f"mire : PPM a {maxi} niveaux, 255 attendu")
    return largeur, hauteur, octets[pos : pos + largeur * hauteur * 3]


def lis_png(octets):
    """PNG RGB/RGBA 8 bits, non entrelace : ce que produit notre convertisseur."""
    if octets[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    pos, idat, entete = 8, bytearray(), None
    while pos < len(octets):
        taille = int.from_bytes(octets[pos : pos + 4], "big")
        genre = octets[pos + 4 : pos + 8]
        charge = octets[pos + 8 : pos + 8 + taille]
        if genre == b"IHDR":
            entete = (
                int.from_bytes(charge[0:4], "big"),
                int.from_bytes(charge[4:8], "big"),
                charge[8],
                charge[9],
                charge[12],
            )
        elif genre == b"IDAT":
            idat += charge
        elif genre == b"IEND":
            break
        pos += 12 + taille
    largeur, hauteur, profondeur, couleur, entrelace = entete
    if profondeur != 8 or couleur not in (2, 6) or entrelace != 0:
        raise SystemExit(
            f"mire : PNG profondeur={profondeur} couleur={couleur} "
            f"entrelace={entrelace} -- non gere"
        )
    canaux = 3 if couleur == 2 else 4
    brut = zlib.decompress(bytes(idat))
    par_ligne = largeur * canaux
    sortie = bytearray()
    precedente = bytearray(par_ligne)
    pos = 0
    for _ in range(hauteur):
        filtre = brut[pos]
        ligne = bytearray(brut[pos + 1 : pos + 1 + par_ligne])
        # Les cinq filtres PNG. Une capture d'ecran les utilise tous.
        for i in range(par_ligne):
            a = ligne[i - canaux] if i >= canaux else 0
            b = precedente[i]
            c = precedente[i - canaux] if i >= canaux else 0
            if filtre == 1:
                ligne[i] = (ligne[i] + a) & 0xFF
            elif filtre == 2:
                ligne[i] = (ligne[i] + b) & 0xFF
            elif filtre == 3:
                ligne[i] = (ligne[i] + ((a + b) >> 1)) & 0xFF
            elif filtre == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                ligne[i] = (ligne[i] + pred) & 0xFF
        precedente = ligne
        if canaux == 3:
            sortie += ligne
        else:
            for i in range(0, par_ligne, 4):
                sortie += ligne[i : i + 3]
        pos += 1 + par_ligne
    return largeur, hauteur, bytes(sortie)


def compte(pixels, cible):
    r, v, b = cible
    total = 0
    for i in range(0, len(pixels) - 2, 3):
        if pixels[i] == r and pixels[i + 1] == v and pixels[i + 2] == b:
            total += 1
    return total


def principal(argv):
    if len(argv) < 2:
        raise SystemExit(__doc__)
    chemin = argv[1]
    minimum = MIN_PAR_DEFAUT
    if "--min" in argv:
        minimum = int(argv[argv.index("--min") + 1])

    with open(chemin, "rb") as f:
        octets = f.read()
    lu = lis_ppm(octets) or lis_png(octets)
    if lu is None:
        raise SystemExit(f"mire : {chemin} n'est ni un PPM P6 ni un PNG")
    largeur, hauteur, pixels = lu
    print(f"mire : capture {largeur}x{hauteur}, {len(pixels) // 3} pixels")

    manquants = []
    for nom, couleur in COULEURS.items():
        n = compte(pixels, couleur)
        etat = "OK" if n >= minimum else "ABSENT"
        print(f"  {nom:<6} #{couleur[0]:02X}{couleur[1]:02X}{couleur[2]:02X}"
              f"  {n:>7} pixels  {etat}")
        if n < minimum:
            manquants.append((nom, n))

    if manquants:
        print("mire : la frame presentee ne porte pas la mire.", file=sys.stderr)
        for nom, n in manquants:
            quoi = {
                "rouge": "`<img>` ne peint pas",
                "vert": "`background-image` ne peint pas (chemin CSS)",
                "bleu": "la mise a l'echelle d'un `<img>` ne peint pas",
            }[nom]
            print(f"       {nom} : {n} pixels (< {minimum}) -- {quoi}", file=sys.stderr)
        return 1

    print("SURFACE_MIRE_OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(principal(sys.argv))
