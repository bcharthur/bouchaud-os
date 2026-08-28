#!/usr/bin/env python3
"""Fabrique les icones du bureau, en PNG, sans aucune dependance.

# Pourquoi un generateur et pas des fichiers poses la

Les icones etaient DESSINEES dans le noyau : des disques et des rectangles
empiles par `widgets.rs`, un peintre par application. Le resultat se voyait sur
la premiere capture d'ecran venue.

Passer a de vraies images demandait des images. Les prendre a un jeu d'icones
existant, c'est heriter de sa licence pour tout le depot ; les dessiner a la
main, c'est un binaire qu'aucune ligne du depot n'explique. Un generateur les
rend REPRODUCTIBLES : le dessin est du code lisible, revu comme le reste, et
`python3 tools/assets/fabrique-icones.py` refait a l'octet pres les fichiers
que le noyau embarque.

Seule exception, et elle est voulue : le logo de Ladybird. C'est le sien, il
vient de son depot, et le bureau doit afficher le vrai -- pas une coccinelle
approchee. Voir `src/assets/icons/LISEZMOI.md`.

# Comment

Pas de Pillow -- il n'est pas garanti sur la machine de personne. Un PNG est un
en-tete, des morceaux, et un flux zlib de lignes precedees d'un octet de
filtre ; `zlib` est dans la bibliotheque standard, donc tout tient ici.

Le dessin se fait a QUATRE fois la taille finale, puis se reduit par moyenne de
blocs. C'est ce qui donne des bords lisses sans code d'antialiassage : un
quart de pixel couvert donne un quart d'opacite, exactement.
"""

import math
import struct
import sys
import zlib
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent.parent
SORTIE = RACINE / "src" / "assets" / "icons"

# Taille finale et facteur de suréchantillonnage.
TAILLE = 128
ECHELLE = 4
BRUT = TAILLE * ECHELLE


# --- Toile ------------------------------------------------------------------


class Toile:
    """RGBA non premultiplie, un octet par composante, a `BRUT` x `BRUT`."""

    def __init__(self):
        self.px = [(0, 0, 0, 0)] * (BRUT * BRUT)

    def pose(self, x, y, couleur):
        """Compose `couleur` (r, v, b, a) sur le pixel, alpha classique."""
        if not (0 <= x < BRUT and 0 <= y < BRUT):
            return
        r, v, b, a = couleur
        if a == 0:
            return
        index = y * BRUT + x
        dr, dv, db, da = self.px[index]
        ia = 255 - a
        na = a + da * ia // 255
        if na == 0:
            self.px[index] = (0, 0, 0, 0)
            return
        self.px[index] = (
            (r * a + dr * da * ia // 255) // na,
            (v * a + dv * da * ia // 255) // na,
            (b * a + db * da * ia // 255) // na,
            na,
        )

    def rect(self, x, y, w, h, couleur):
        for yy in range(int(y), int(y + h)):
            for xx in range(int(x), int(x + w)):
                self.pose(xx, yy, couleur)

    def rect_arrondi(self, x, y, w, h, rayon, couleur):
        x, y, w, h, rayon = int(x), int(y), int(w), int(h), int(rayon)
        rayon = max(0, min(rayon, w // 2, h // 2))
        for yy in range(y, y + h):
            for xx in range(x, x + w):
                if self._dans_arrondi(xx - x, yy - y, w, h, rayon):
                    self.pose(xx, yy, couleur)

    @staticmethod
    def _dans_arrondi(lx, ly, w, h, rayon):
        if rayon == 0:
            return True
        cx = rayon - 1 if lx < rayon else (w - rayon if lx >= w - rayon else None)
        cy = rayon - 1 if ly < rayon else (h - rayon if ly >= h - rayon else None)
        if cx is None or cy is None:
            return True
        dx, dy = lx - cx, ly - cy
        return dx * dx + dy * dy <= rayon * rayon

    def disque(self, cx, cy, rayon, couleur):
        cx, cy, rayon = int(cx), int(cy), int(rayon)
        for yy in range(cy - rayon, cy + rayon + 1):
            reste = rayon * rayon - (yy - cy) ** 2
            if reste < 0:
                continue
            demi = int(math.isqrt(reste))
            for xx in range(cx - demi, cx + demi + 1):
                self.pose(xx, yy, couleur)

    def degrade_vertical(self, x, y, w, h, haut, bas, rayon=0):
        """Remplit un rectangle (arrondi) d'un degre vertical."""
        x, y, w, h = int(x), int(y), int(w), int(h)
        for ligne in range(h):
            t = ligne / max(1, h - 1)
            couleur = tuple(
                int(haut[i] + (bas[i] - haut[i]) * t) for i in range(4)
            )
            for xx in range(x, x + w):
                if self._dans_arrondi(xx - x, ligne, w, h, rayon):
                    self.pose(xx, y + ligne, couleur)

    def trait(self, x0, y0, x1, y1, epaisseur, couleur):
        """Segment epais, par balayage du rectangle englobant."""
        x0, y0, x1, y1 = float(x0), float(y0), float(x1), float(y1)
        demi = epaisseur / 2.0
        dx, dy = x1 - x0, y1 - y0
        longueur2 = dx * dx + dy * dy
        gauche = int(min(x0, x1) - demi - 1)
        droite = int(max(x0, x1) + demi + 2)
        haut = int(min(y0, y1) - demi - 1)
        bas = int(max(y0, y1) + demi + 2)
        for yy in range(haut, bas):
            for xx in range(gauche, droite):
                if longueur2 == 0:
                    t = 0.0
                else:
                    t = ((xx - x0) * dx + (yy - y0) * dy) / longueur2
                    t = max(0.0, min(1.0, t))
                px, py = x0 + t * dx, y0 + t * dy
                if (xx - px) ** 2 + (yy - py) ** 2 <= demi * demi:
                    self.pose(xx, yy, couleur)

    def reduit(self):
        """Moyenne par blocs de `ECHELLE` x `ECHELLE` : l'antialiassage."""
        sortie = bytearray(TAILLE * TAILLE * 4)
        aire = ECHELLE * ECHELLE
        for y in range(TAILLE):
            for x in range(TAILLE):
                sr = sv = sb = sa = 0
                for dy in range(ECHELLE):
                    ligne = (y * ECHELLE + dy) * BRUT + x * ECHELLE
                    for dx in range(ECHELLE):
                        r, v, b, a = self.px[ligne + dx]
                        # Premultiplier AVANT de moyenner : sans cela, la
                        # couleur d'un pixel transparent -- indefinie -- teinte
                        # le bord de l'icone.
                        sr += r * a
                        sv += v * a
                        sb += b * a
                        sa += a
                index = (y * TAILLE + x) * 4
                if sa == 0:
                    continue
                sortie[index] = min(255, sr // sa)
                sortie[index + 1] = min(255, sv // sa)
                sortie[index + 2] = min(255, sb // sa)
                sortie[index + 3] = min(255, sa // aire)
        return bytes(sortie)


def ecris_png(chemin, rgba):
    """Ecrit un PNG 8 bits RGBA, filtre 0, sans morceau accessoire."""

    def morceau(genre, contenu):
        return (
            struct.pack(">I", len(contenu))
            + genre
            + contenu
            + struct.pack(">I", zlib.crc32(genre + contenu) & 0xFFFFFFFF)
        )

    lignes = bytearray()
    for y in range(TAILLE):
        lignes.append(0)  # filtre « None » : le decodeur du noyau le gere.
        lignes += rgba[y * TAILLE * 4 : (y + 1) * TAILLE * 4]

    entete = struct.pack(">IIBBBBB", TAILLE, TAILLE, 8, 6, 0, 0, 0)
    donnees = zlib.compress(bytes(lignes), 9)
    chemin.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + morceau(b"IHDR", entete)
        + morceau(b"IDAT", donnees)
        + morceau(b"IEND", b"")
    )


# --- Les icones -------------------------------------------------------------

# Une famille visuelle : meme plaque arrondie, meme ombre interne, et une seule
# couleur vive par icone. C'est ce qui fait qu'une rangee d'icones se lit comme
# une rangee, et non comme cinq dessins sans rapport.

def plaque(toile, haut, bas):
    marge = 6 * ECHELLE
    cote = BRUT - marge * 2
    rayon = 26 * ECHELLE
    # Ombre portee, douce, decalee vers le bas.
    toile.rect_arrondi(marge, marge + 3 * ECHELLE, cote, cote, rayon, (0, 0, 0, 60))
    toile.degrade_vertical(marge, marge, cote, cote, haut, bas, rayon)
    # Filet clair sur le haut : ce qui donne l'impression du relief.
    toile.rect_arrondi(marge, marge, cote, 2 * ECHELLE, rayon // 3, (255, 255, 255, 34))
    return marge, cote


def icone_terminal():
    t = Toile()
    marge, cote = plaque(t, (28, 34, 44, 255), (16, 20, 27, 255))
    vert = (63, 185, 80, 255)
    # Chevron « > ».
    x0 = marge + cote * 22 // 100
    y0 = marge + cote * 32 // 100
    y1 = marge + cote * 68 // 100
    pointe = marge + cote * 44 // 100
    e = 5 * ECHELLE
    t.trait(x0, y0, pointe, (y0 + y1) / 2, e, vert)
    t.trait(pointe, (y0 + y1) / 2, x0, y1, e, vert)
    # Curseur.
    t.rect_arrondi(marge + cote * 52 // 100, y1 - 3 * ECHELLE,
                   cote * 26 // 100, 5 * ECHELLE, 2 * ECHELLE, (139, 148, 158, 255))
    return t


def icone_fichiers():
    t = Toile()
    marge, cote = plaque(t, (36, 30, 20, 255), (24, 19, 12, 255))
    ambre_sombre = (191, 133, 26, 255)
    ambre = (232, 168, 46, 255)
    ambre_clair = (247, 199, 92, 255)
    gauche = marge + cote * 16 // 100
    largeur = cote * 68 // 100
    # Onglet du dossier.
    t.rect_arrondi(gauche, marge + cote * 26 // 100, largeur * 44 // 100,
                   cote * 12 // 100, 3 * ECHELLE, ambre_sombre)
    # Dos du dossier.
    t.rect_arrondi(gauche, marge + cote * 33 // 100, largeur, cote * 40 // 100,
                   4 * ECHELLE, ambre_sombre)
    # Face avant, legerement decalee : ce qui fait le volume.
    t.rect_arrondi(gauche, marge + cote * 41 // 100, largeur, cote * 32 // 100,
                   4 * ECHELLE, ambre)
    t.rect_arrondi(gauche, marge + cote * 41 // 100, largeur, 2 * ECHELLE,
                   ECHELLE, ambre_clair)
    return t


def icone_calculatrice():
    t = Toile()
    marge, cote = plaque(t, (40, 46, 58, 255), (24, 28, 36, 255))
    gauche = marge + cote * 20 // 100
    largeur = cote * 60 // 100
    # Ecran.
    t.rect_arrondi(gauche, marge + cote * 20 // 100, largeur, cote * 18 // 100,
                   3 * ECHELLE, (144, 205, 244, 255))
    t.rect_arrondi(gauche + largeur * 55 // 100, marge + cote * 26 // 100,
                   largeur * 34 // 100, 4 * ECHELLE, 2 * ECHELLE, (30, 60, 90, 255))
    # Quatre rangees de trois touches, la derniere colonne en accent.
    touche = largeur * 26 // 100
    ecart = largeur * 11 // 100
    haut = marge + cote * 44 // 100
    for rangee in range(4):
        for colonne in range(3):
            x = gauche + colonne * (touche + ecart)
            y = haut + rangee * (touche + ecart) * 68 // 100
            accent = colonne == 2
            couleur = (79, 124, 255, 255) if accent else (108, 118, 134, 255)
            t.rect_arrondi(x, y, touche, touche * 62 // 100, 2 * ECHELLE, couleur)
    return t


def icone_rustpad():
    t = Toile()
    marge, cote = plaque(t, (34, 30, 28, 255), (22, 19, 18, 255))
    gauche = marge + cote * 22 // 100
    largeur = cote * 56 // 100
    haut = marge + cote * 18 // 100
    hauteur = cote * 64 // 100
    # La feuille.
    t.rect_arrondi(gauche, haut, largeur, hauteur, 4 * ECHELLE, (240, 240, 238, 255))
    # Bandeau de titre, rouille : la couleur de Rust.
    t.rect_arrondi(gauche, haut, largeur, cote * 12 // 100, 4 * ECHELLE,
                   (222, 106, 62, 255))
    t.rect(gauche, haut + cote * 8 // 100, largeur, cote * 4 // 100,
           (222, 106, 62, 255))
    # Lignes de texte.
    for index in range(4):
        y = haut + cote * (20 + index * 10) // 100
        fin = largeur * (86 if index % 2 == 0 else 60) // 100
        t.rect_arrondi(gauche + largeur * 8 // 100, y, fin, 3 * ECHELLE,
                       ECHELLE, (150, 152, 156, 255))
    return t


ICONES = {
    "terminal.png": icone_terminal,
    "fichiers.png": icone_fichiers,
    "calculatrice.png": icone_calculatrice,
    "rustpad.png": icone_rustpad,
}


def octets_png(rgba):
    """Le PNG en memoire, pour pouvoir le comparer sans ecrire de fichier."""
    import io as _io

    tampon = _io.BytesIO()

    class Faux:
        def write_bytes(self, donnees):
            tampon.write(donnees)

    ecris_png(Faux(), rgba)
    return tampon.getvalue()


def verifie():
    """Les fichiers commis sont-ils encore ceux que ce code produit ?

    C'est ce qui rend le generateur utile : sans cette verification, un dessin
    modifie sans regeneration -- ou l'inverse -- laisserait le depot avec du
    code qui n'explique plus ses propres octets.
    """
    ecarts = []
    for nom, fabrique in ICONES.items():
        chemin = SORTIE / nom
        if not chemin.exists():
            ecarts.append(f"  {nom} : absent de {SORTIE.relative_to(RACINE)}")
            continue
        attendu = octets_png(fabrique().reduit())
        if chemin.read_bytes() != attendu:
            ecarts.append(
                f"  {nom} : le fichier commis differe de ce que le generateur "
                f"produit -- relancer `python3 tools/assets/fabrique-icones.py`"
            )
    if ecarts:
        print("icones du bureau : le generateur et les fichiers ont diverge")
        print("\n".join(ecarts))
        return 1
    print(f"ok  src/assets/icons : {len(ICONES)} icone(s) reproduites a "
          f"l'octet pres par le generateur")
    return 0


def main():
    if "--verifie" in sys.argv:
        return verifie()
    SORTIE.mkdir(parents=True, exist_ok=True)
    for nom, fabrique in ICONES.items():
        chemin = SORTIE / nom
        ecris_png(chemin, fabrique().reduit())
        print(f"  ecrit  {chemin.relative_to(RACINE)}  "
              f"{chemin.stat().st_size} octets")
    print(f"{len(ICONES)} icone(s) de {TAILLE}x{TAILLE} en RGBA")
    return 0


if __name__ == "__main__":
    sys.exit(main())
