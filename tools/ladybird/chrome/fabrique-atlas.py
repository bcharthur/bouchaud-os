#!/usr/bin/env python3
"""Fabrique l'atlas de glyphes du chrome, depuis DejaVu Sans.

# Pourquoi

`BouchaudChrome.h` dessine la barre d'adresse, les boutons et le titre avec une
police BITMAP 8x8 embarquee -- « font8x8 basic », un bit par pixel, agrandie
par un facteur entier. C'est ce qu'on voit sur les captures : un escalier a
chaque diagonale, dans une fenetre dont tout le reste est antialiase, et
strictement aucune lettre accentuee.

Le chrome vit dans WebContent et ne veut dependre d'aucune API de dessin : il
ecrit ses pixels lui-meme dans une `Canvas`. Cette contrainte est deliberee et
elle est bonne -- elle rend le chrome independant de l'etat de LibGfx, y compris
quand la page a plante.

Un ATLAS la respecte. Les glyphes sont rasterises ICI, a la construction, et
embarques comme des octets de couverture : le chrome n'a plus qu'a les melanger.
Il gagne de vraies lettres, l'antialiassage et les accents, sans gagner une
seule dependance.

# Comment

Pas de freetype, pas de Pillow -- rien de tout cela n'est garanti sur la machine
de personne. Un TrueType est un format lisible : un repertoire de tables, des
contours quadratiques dans `glyf`, une table de correspondance `cmap`, des
avances dans `hmtx`. Tout tient dans ce fichier.

Les glyphes COMPOSITES comptent autant que les simples : « e » est un contour,
« e accent aigu » est une composition de deux glyphes. Les ignorer donnerait
exactement le defaut qu'on veut corriger.

Le remplissage se fait par balayage avec sur-echantillonnage vertical et
couverture horizontale exacte, ce qui donne l'antialiassage sans code
d'antialiassage.
"""

import struct
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent.parent.parent
POLICE = RACINE / "src" / "assets" / "fonts" / "DejaVuSans.ttf"
SORTIE = Path(__file__).resolve().parent / "BouchaudAtlas.h"

# Corps rasterise. Le chrome dessine a `scale` 1 ou 2 ; un atlas a 15 pixels
# couvre le premier cas et se laisse doubler proprement pour le second.
CORPS = 15

# Les caracteres du chrome : ASCII imprimable, plus le Latin-1 accentue qu'une
# barre d'adresse ou un titre de page francais rencontre tous les jours.
CARACTERES = [chr(c) for c in range(0x20, 0x7F)] + [
    "à", "â", "ä", "ç", "è", "é", "ê",
    "ë", "î", "ï", "ô", "ö", "ù", "û",
    "ü", "ÿ", "œ", "æ",
    "À", "Â", "Ä", "Ç", "È", "É", "Ê",
    "Ë", "Î", "Ï", "Ô", "Ö", "Ù", "Û",
    "Ü", "Œ", "Æ",
    "«", "»", "’", "‘", "“", "”", "…", "€", "°", "—", "–", "·", "×",
]

SUR_ECHANTILLON = 4


# --- Lecture du fichier ------------------------------------------------------


class Police:
    def __init__(self, octets):
        self.o = octets
        nombre = struct.unpack(">H", octets[4:6])[0]
        self.tables = {}
        for index in range(nombre):
            base = 12 + index * 16
            nom = octets[base : base + 4]
            debut, taille = struct.unpack(">II", octets[base + 8 : base + 16])
            self.tables[nom] = (debut, taille)

        head = self.tables[b"head"][0]
        self.upem = struct.unpack(">H", octets[head + 18 : head + 20])[0]
        self.format_loca = struct.unpack(">h", octets[head + 50 : head + 52])[0]

        maxp = self.tables[b"maxp"][0]
        self.nb_glyphes = struct.unpack(">H", octets[maxp + 4 : maxp + 6])[0]

        hhea = self.tables[b"hhea"][0]
        self.ascent = struct.unpack(">h", octets[hhea + 4 : hhea + 6])[0]
        self.nb_avances = struct.unpack(">H", octets[hhea + 34 : hhea + 36])[0]

        self.loca = self._lit_loca()
        self.cmap = self._lit_cmap()

    def _lit_loca(self):
        debut, _ = self.tables[b"loca"]
        n = self.nb_glyphes + 1
        if self.format_loca == 0:
            brut = struct.unpack(f">{n}H", self.o[debut : debut + n * 2])
            return [valeur * 2 for valeur in brut]
        return list(struct.unpack(f">{n}I", self.o[debut : debut + n * 4]))

    def _lit_cmap(self):
        """Format 4 seulement : c'est celui de toute police Windows/Unicode."""
        debut, _ = self.tables[b"cmap"]
        nombre = struct.unpack(">H", self.o[debut + 2 : debut + 4])[0]
        table = None
        for index in range(nombre):
            base = debut + 4 + index * 8
            plateforme, encodage, decalage = struct.unpack(
                ">HHI", self.o[base : base + 8])
            if (plateforme, encodage) in ((3, 1), (3, 10), (0, 3), (0, 4)):
                table = debut + decalage
                break
        if table is None:
            raise SystemExit("cmap : aucun sous-format Unicode")
        if struct.unpack(">H", self.o[table : table + 2])[0] != 4:
            raise SystemExit("cmap : seul le format 4 est lu")

        segx2 = struct.unpack(">H", self.o[table + 6 : table + 8])[0]
        seg = segx2 // 2
        fin = table + 14
        depart = fin + segx2 + 2
        delta = depart + segx2
        plage = delta + segx2

        correspondance = {}
        for index in range(seg):
            f = struct.unpack(">H", self.o[fin + index * 2 : fin + index * 2 + 2])[0]
            d = struct.unpack(">H", self.o[depart + index * 2 : depart + index * 2 + 2])[0]
            dl = struct.unpack(">h", self.o[delta + index * 2 : delta + index * 2 + 2])[0]
            pl = struct.unpack(">H", self.o[plage + index * 2 : plage + index * 2 + 2])[0]
            if d > f:
                continue
            for code in range(d, min(f, 0xFFFF) + 1):
                if pl == 0:
                    glyphe = (code + dl) & 0xFFFF
                else:
                    adresse = plage + index * 2 + pl + (code - d) * 2
                    if adresse + 2 > len(self.o):
                        continue
                    glyphe = struct.unpack(">H", self.o[adresse : adresse + 2])[0]
                    if glyphe:
                        glyphe = (glyphe + dl) & 0xFFFF
                if glyphe:
                    correspondance[code] = glyphe
        return correspondance

    def avance(self, glyphe):
        debut, _ = self.tables[b"hmtx"]
        index = min(glyphe, self.nb_avances - 1)
        return struct.unpack(">H", self.o[debut + index * 4 : debut + index * 4 + 2])[0]

    def contours(self, glyphe, profondeur=0):
        """Contours du glyphe, en unites de police, composites resolus."""
        if profondeur > 4 or glyphe + 1 >= len(self.loca):
            return []
        debut, _ = self.tables[b"glyf"]
        a, b = self.loca[glyphe], self.loca[glyphe + 1]
        if b <= a:
            return []
        donnees = self.o[debut + a : debut + b]
        nb = struct.unpack(">h", donnees[0:2])[0]
        if nb >= 0:
            return self._contours_simples(donnees, nb)
        return self._contours_composites(donnees, profondeur)

    def _contours_simples(self, donnees, nb_contours):
        position = 10
        fins = list(struct.unpack(f">{nb_contours}H",
                                  donnees[position : position + nb_contours * 2]))
        position += nb_contours * 2
        nb_points = (fins[-1] + 1) if fins else 0
        instructions = struct.unpack(">H", donnees[position : position + 2])[0]
        position += 2 + instructions

        drapeaux = []
        while len(drapeaux) < nb_points:
            d = donnees[position]
            position += 1
            drapeaux.append(d)
            if d & 8:
                repetition = donnees[position]
                position += 1
                drapeaux.extend([d] * repetition)
        drapeaux = drapeaux[:nb_points]

        def lit(bit_court, bit_meme):
            valeurs = []
            courant = 0
            nonlocal position
            for d in drapeaux:
                if d & bit_court:
                    delta = donnees[position]
                    position += 1
                    courant += delta if d & bit_meme else -delta
                elif not d & bit_meme:
                    courant += struct.unpack(">h", donnees[position : position + 2])[0]
                    position += 2
                valeurs.append(courant)
            return valeurs

        xs = lit(2, 16)
        ys = lit(4, 32)

        contours = []
        depart = 0
        for fin in fins:
            points = [(xs[i], ys[i], bool(drapeaux[i] & 1))
                      for i in range(depart, fin + 1)]
            if points:
                contours.append(points)
            depart = fin + 1
        return contours

    def _contours_composites(self, donnees, profondeur):
        position = 10
        contours = []
        while True:
            drapeaux, index = struct.unpack(">HH", donnees[position : position + 4])
            position += 4
            if drapeaux & 1:  # ARG_1_AND_2_ARE_WORDS
                dx, dy = struct.unpack(">hh", donnees[position : position + 4])
                position += 4
            else:
                dx, dy = struct.unpack(">bb", donnees[position : position + 2])
                position += 2
            echelle_x = echelle_y = 1.0
            if drapeaux & 8:  # WE_HAVE_A_SCALE
                echelle_x = echelle_y = _f2dot14(donnees, position)
                position += 2
            elif drapeaux & 0x40:  # X_AND_Y_SCALE
                echelle_x = _f2dot14(donnees, position)
                echelle_y = _f2dot14(donnees, position + 2)
                position += 4
            elif drapeaux & 0x80:  # TWO_BY_TWO
                echelle_x = _f2dot14(donnees, position)
                echelle_y = _f2dot14(donnees, position + 6)
                position += 8

            for contour in self.contours(index, profondeur + 1):
                contours.append([
                    (x * echelle_x + dx, y * echelle_y + dy, sur)
                    for x, y, sur in contour
                ])
            if not drapeaux & 0x20:  # MORE_COMPONENTS
                break
        return contours


def _f2dot14(donnees, position):
    return struct.unpack(">h", donnees[position : position + 2])[0] / 16384.0


# --- Rasterisation -----------------------------------------------------------


def segments(contours, echelle, dx, dy):
    """Les contours quadratiques, aplatis en segments de droite."""
    droites = []
    for points in contours:
        if len(points) < 2:
            continue
        # Un contour peut commencer par un point de controle : on fabrique
        # alors le point sur courbe implicite, comme le veut le format.
        if not points[0][2]:
            if points[-1][2]:
                points = [points[-1]] + points[:-1]
            else:
                milieu = ((points[0][0] + points[-1][0]) / 2,
                          (points[0][1] + points[-1][1]) / 2, True)
                points = [milieu] + points

        chemin = []
        index = 0
        n = len(points)
        depart = (points[0][0], points[0][1])
        courant = depart
        index = 1
        while index <= n:
            point = points[index % n]
            if point[2]:
                chemin.append((courant, (point[0], point[1])))
                courant = (point[0], point[1])
                index += 1
            else:
                suivant = points[(index + 1) % n]
                if suivant[2]:
                    fin = (suivant[0], suivant[1])
                    index += 2
                else:
                    fin = ((point[0] + suivant[0]) / 2,
                           (point[1] + suivant[1]) / 2)
                    index += 1
                controle = (point[0], point[1])
                pas = 8
                precedent = courant
                for k in range(1, pas + 1):
                    t = k / pas
                    u = 1 - t
                    px = u * u * courant[0] + 2 * u * t * controle[0] + t * t * fin[0]
                    py = u * u * courant[1] + 2 * u * t * controle[1] + t * t * fin[1]
                    chemin.append((precedent, (px, py)))
                    precedent = (px, py)
                courant = fin
        if courant != depart:
            chemin.append((courant, depart))
        for (ax, ay), (bx, by) in chemin:
            droites.append((ax * echelle + dx, ay * echelle + dy,
                            bx * echelle + dx, by * echelle + dy))
    return droites


def rasterise(droites, largeur, hauteur):
    """Couverture 0-255, par balayage avec sur-echantillonnage vertical."""
    couverture = [0] * (largeur * hauteur)
    if not droites:
        return couverture
    accumule = [0.0] * (largeur * hauteur)
    for ligne in range(hauteur * SUR_ECHANTILLON):
        y = (ligne + 0.5) / SUR_ECHANTILLON
        croisements = []
        for x0, y0, x1, y1 in droites:
            if (y0 <= y < y1) or (y1 <= y < y0):
                t = (y - y0) / (y1 - y0)
                croisements.append((x0 + t * (x1 - x0), 1 if y1 > y0 else -1))
        if not croisements:
            continue
        croisements.sort()
        # Non-zero winding : c'est la regle de TrueType. L'even-odd creuserait
        # les contre-formes des glyphes composites.
        enroulement = 0
        debut = 0.0
        cible = (ligne // SUR_ECHANTILLON) * largeur
        for x, sens in croisements:
            if enroulement != 0:
                _ajoute_span(accumule, cible, largeur, debut, x)
            if enroulement == 0:
                debut = x
            enroulement += sens
    facteur = 255.0 / SUR_ECHANTILLON
    for index, valeur in enumerate(accumule):
        couverture[index] = min(255, int(valeur * facteur + 0.5))
    return couverture


def _ajoute_span(accumule, base, largeur, x0, x1):
    """Ajoute la couverture horizontale EXACTE du segment [x0, x1)."""
    if x1 <= x0:
        return
    gauche = max(0, int(x0))
    droite = min(largeur - 1, int(x1))
    for colonne in range(gauche, droite + 1):
        part = min(x1, colonne + 1.0) - max(x0, float(colonne))
        if part > 0:
            accumule[base + colonne] += part


# --- Sortie ------------------------------------------------------------------


def main():
    if not POLICE.exists():
        print(f"police introuvable : {POLICE}", file=sys.stderr)
        return 1
    police = Police(POLICE.read_bytes())
    echelle = CORPS / police.upem
    hauteur = int(CORPS * 1.35 + 0.5)
    base = int(police.ascent * echelle + 0.5)

    glyphes = []
    for caractere in CARACTERES:
        identifiant = police.cmap.get(ord(caractere), 0)
        avance = int(police.avance(identifiant) * echelle + 0.5)
        contours = police.contours(identifiant)
        if not contours:
            glyphes.append((caractere, avance, 0, 0, 0, 0, []))
            continue
        xs = [p[0] for c in contours for p in c]
        ys = [p[1] for c in contours for p in c]
        x0 = int(min(xs) * echelle) - 1
        y1 = int(max(ys) * echelle) + 1
        largeur = int(max(xs) * echelle) - x0 + 2
        h = y1 - int(min(ys) * echelle) + 2
        droites = segments(contours, echelle, -x0, 0)
        # L'axe des y d'une police monte ; celui d'une image descend.
        droites = [(ax, y1 - ay, bx, y1 - by) for ax, ay, bx, by in droites]
        couverture = rasterise(droites, largeur, h)
        glyphes.append((caractere, avance, x0, base - y1, largeur, h, couverture))

    lignes = []
    donnees = []
    decalage = 0
    for caractere, avance, gauche, haut, largeur, h, couverture in glyphes:
        lignes.append(
            f"    {{ 0x{ord(caractere):04x}, {avance}, {gauche}, {haut}, "
            f"{largeur}, {h}, {decalage} }},")
        donnees.extend(couverture)
        decalage += len(couverture)

    # Envelopper par ELEMENT, jamais au caractere : couper « 255 » en « 2 » et
    # « 55 » produit un fichier qui compile et un atlas faux.
    enveloppe = []
    for index in range(0, len(donnees), 24):
        enveloppe.append(
            "    " + ", ".join(str(v) for v in donnees[index : index + 24]) + ",")

    contenu = (
        "// Genere par tools/ladybird/chrome/fabrique-atlas.py -- NE PAS EDITER.\n"
        "//\n"
        "// Atlas de glyphes DejaVu Sans, rasterise a la construction. Le chrome\n"
        "// dessinait son texte avec une bitmap 8x8 d'un bit par pixel : un\n"
        "// escalier a chaque diagonale, et aucune lettre accentuee. Il melange\n"
        "// desormais ces couvertures, sans dependre d'aucune API de dessin --\n"
        "// la contrainte qui a fait choisir la bitmap reste tenue.\n"
        "#pragma once\n\n"
        "namespace BouchaudAtlas {\n\n"
        f"inline constexpr int corps = {CORPS};\n"
        f"inline constexpr int hauteur_ligne = {hauteur};\n"
        f"inline constexpr int ligne_de_base = {base};\n\n"
        "struct Glyphe {\n"
        "    unsigned int point_de_code;\n"
        "    int avance;\n"
        "    int gauche;\n"
        "    int haut;\n"
        "    int largeur;\n"
        "    int hauteur;\n"
        "    unsigned int decalage;\n"
        "};\n\n"
        f"inline constexpr int nombre = {len(glyphes)};\n\n"
        "inline constexpr Glyphe glyphes[] = {\n"
        + "\n".join(lignes)
        + "\n};\n\n"
        f"inline constexpr unsigned char couverture[] = {{\n"
        + "\n".join(enveloppe)
        + "\n};\n\n"
        "}\n"
    )

    if "--verifie" in sys.argv:
        # Le fichier commis est-il encore celui que ce code produit ?
        # Sans cette verification, une police remplacee ou un caractere ajoute
        # laisserait le depot avec un atlas que son propre code n'explique plus.
        if not SORTIE.exists():
            print(f"atlas absent : {SORTIE.relative_to(RACINE)}")
            return 1
        if SORTIE.read_text(encoding="utf-8") != contenu:
            print("atlas du chrome : le fichier commis differe de ce que le "
                  "generateur produit -- relancer "
                  "`python3 tools/ladybird/chrome/fabrique-atlas.py`")
            return 1
        print(f"ok  BouchaudAtlas.h : {len(glyphes)} glyphes reproduits a "
              f"l'octet pres depuis DejaVuSans.ttf")
        return 0

    SORTIE.write_text(contenu, encoding="utf-8")
    print(f"  ecrit  {SORTIE.relative_to(RACINE)}  "
          f"{SORTIE.stat().st_size} octets, {len(glyphes)} glyphes, "
          f"{len(donnees)} octets de couverture")
    return 0


if __name__ == "__main__":
    sys.exit(main())
