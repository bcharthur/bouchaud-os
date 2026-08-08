#!/usr/bin/env python3
"""Peint une page avec le moteur du navigateur, sans Qt et sans l'OS.

    ./apercu.py https://pypi.org/ -o pypi.png
    ./apercu.py fichier.html --largeur 1280 --hauteur 900

## A quoi cela sert

Voir ce que le moteur affiche demande normalement de construire Qt (des
heures), de fabriquer l'image de disque et de demarrer l'emulateur. Cet outil
saute les trois : il execute **le vrai moteur** — meme analyseur HTML, meme
cascade, meme mise en page, meme liste d'affichage — et remplace le seul hote
Qt par un peintre adosse a Pillow.

## Ce que l'image montre fidelement

La liste d'affichage est la meme qu'a l'ecran : les operations peintes ici sont
exactement celles que `hote.cpp` recevrait. Les polices sont celles que l'OS
embarque (`src/assets/fonts/DejaVu*`), donc les mesures de texte — dont depend
tout le retour a la ligne — sont les vraies.

## Ce qu'elle ne montre pas

Ce n'est **pas** une capture de Bouchaud OS : ni le noyau, ni Qt, ni le
framebuffer ne sont dans la boucle. Le rendu du texte est celui de Pillow et
non de Qt, donc l'anticrenelage differe au pixel. Un glyphe sous une rotation
est pose sans tourner, et le flou d'une ombre est approche par des couches
concentriques comme cote Qt, mais leur addition n'est pas identique.
"""

import argparse
import math
import os
import subprocess
import sys
import types

RACINE = os.path.dirname(os.path.abspath(__file__))
POLICES = os.path.join(RACINE, "..", "..", "..", "src", "assets", "fonts")

try:
    from PIL import Image, ImageDraw, ImageFont
except ImportError:  # pragma: no cover
    sys.exit("apercu : Pillow est requis (pip install pillow)")


# --- Couleurs -----------------------------------------------------------------

def _teinte(valeur):
    """Un entier 0xAARRGGBB en quadruplet Pillow."""
    valeur = int(valeur) & 0xFFFFFFFF
    return ((valeur >> 16) & 0xFF, (valeur >> 8) & 0xFF, valeur & 0xFF,
            (valeur >> 24) & 0xFF)


# --- Polices ------------------------------------------------------------------

class Fontes:
    """Les quatre variantes que l'OS embarque, chargees a la demande."""

    FICHIERS = {
        (False, False): "DejaVuSans.ttf",
        (True, False): "DejaVuSans-Bold.ttf",
        (False, True): "DejaVuSansMono.ttf",
        (True, True): "DejaVuSansMono-Bold.ttf",
    }

    def __init__(self):
        self._cache = {}

    def prend(self, taille, gras=False, fixe=False):
        taille = max(1, int(round(float(taille))))
        cle = (taille, bool(gras), bool(fixe))
        if cle not in self._cache:
            chemin = os.path.join(POLICES, self.FICHIERS[(bool(gras), bool(fixe))])
            try:
                self._cache[cle] = ImageFont.truetype(chemin, taille)
            except OSError:
                self._cache[cle] = ImageFont.load_default()
        return self._cache[cle]


FONTES = Fontes()

# Les images decodees, par numero. L'hote les range au chargement de la page ;
# la liste d'affichage ne porte que leur numero, et le peintre les y retrouve —
# exactement comme le cache de `hote.cpp`.
IMAGES = []


# --- Le peintre ---------------------------------------------------------------

class Peintre:
    """Joue une liste d'affichage sur une image.

    Chaque operation est dessinee sur une tuile de la taille de son englobant,
    puis composee sur la toile a travers le rognage courant et l'opacite. C'est
    ce qui permet de rendre correctement les couleurs translucides — Pillow
    remplace l'alpha au lieu de le melanger quand on dessine en place — sans
    allouer une image entiere par operation.
    """

    def __init__(self, largeur, hauteur, fond=(255, 255, 255, 255)):
        self.largeur = int(largeur)
        self.hauteur = int(hauteur)
        self.toile = Image.new("RGBA", (self.largeur, self.hauteur), fond)
        self.images = IMAGES
        self._rognages = []
        self._etats = []          # (matrice, opacite) empiles par transforme/opacite
        self._matrice = (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
        self._opacite = 1.0

    # -- Etat ------------------------------------------------------------------

    @property
    def rognage(self):
        if not self._rognages:
            return (0, 0, self.largeur, self.hauteur)
        return self._rognages[-1]

    def _applique(self, x, y):
        a, b, c, d, e, f = self._matrice
        return (a * x + c * y + e, b * x + d * y + f)

    def _tourne(self):
        """La matrice courante fait-elle autre chose que translater et grandir ?"""
        _, b, c, _, _, _ = self._matrice
        return abs(b) > 1e-6 or abs(c) > 1e-6

    def _echelle(self):
        a, b, c, d, _, _ = self._matrice
        return math.sqrt(abs(a * d - b * c)) or 1.0

    def _rect(self, x, y, l, h):
        """Un rectangle transforme, en coordonnees de toile."""
        x0, y0 = self._applique(x, y)
        x1, y1 = self._applique(x + l, y + h)
        return (min(x0, x1), min(y0, y1), max(x0, x1), max(y0, y1))

    # -- Composition -----------------------------------------------------------

    def _pose(self, englobant, dessine):
        """Dessine dans une tuile puis la compose a travers rognage et opacite."""
        cx0, cy0, cl, ch = self.rognage
        x0 = max(int(math.floor(englobant[0])), int(cx0), 0)
        y0 = max(int(math.floor(englobant[1])), int(cy0), 0)
        x1 = min(int(math.ceil(englobant[2])), int(cx0 + cl), self.largeur)
        y1 = min(int(math.ceil(englobant[3])), int(cy0 + ch), self.hauteur)
        if x1 <= x0 or y1 <= y0:
            return
        tuile = Image.new("RGBA", (x1 - x0, y1 - y0), (0, 0, 0, 0))
        dessine(ImageDraw.Draw(tuile), -x0, -y0)
        if self._opacite < 1.0:
            alpha = tuile.getchannel("A").point(
                lambda v: int(v * self._opacite))
            tuile.putalpha(alpha)
        self.toile.alpha_composite(tuile, (x0, y0))

    # -- Le repartiteur --------------------------------------------------------

    def joue(self, liste):
        for operation in liste:
            try:
                self.operation(operation)
            except Exception as e:  # noqa: BLE001 — une page ne casse pas l'apercu
                print("apercu: %s ignoree (%s)" % (operation[0], e), file=sys.stderr)

    def operation(self, op):
        genre = op[0]
        methode = getattr(self, "_op_" + genre, None)
        if methode is not None:
            methode(*op[1:])

    # -- Formes ----------------------------------------------------------------

    def _op_rect(self, x, y, l, h, couleur):
        if l <= 0 or h <= 0:
            return
        if self._tourne():
            coins = [self._applique(x, y), self._applique(x + l, y),
                     self._applique(x + l, y + h), self._applique(x, y + h)]
            return self._op_polygone(coins, couleur)
        zone = self._rect(x, y, l, h)
        teinte = _teinte(couleur)
        self._pose(zone, lambda d, dx, dy: d.rectangle(
            [zone[0] + dx, zone[1] + dy, zone[2] + dx, zone[3] + dy], fill=teinte))

    def _op_rond(self, x, y, l, h, rayon, couleur):
        if l <= 0 or h <= 0:
            return
        zone = self._rect(x, y, l, h)
        r = max(0.0, min(float(rayon) * self._echelle(),
                         (zone[2] - zone[0]) / 2.0, (zone[3] - zone[1]) / 2.0))
        teinte = _teinte(couleur)

        def dessine(d, dx, dy):
            boite = [zone[0] + dx, zone[1] + dy, zone[2] + dx, zone[3] + dy]
            if r >= 1.0:
                d.rounded_rectangle(boite, radius=r, fill=teinte)
            else:
                d.rectangle(boite, fill=teinte)
        self._pose(zone, dessine)

    def _op_contour(self, x, y, l, h, rayon, epaisseur, couleur):
        zone = self._rect(x, y, l, h)
        e = max(1, int(round(float(epaisseur) * self._echelle())))
        r = max(0.0, float(rayon) * self._echelle())
        teinte = _teinte(couleur)

        def dessine(d, dx, dy):
            boite = [zone[0] + dx + e / 2.0, zone[1] + dy + e / 2.0,
                     zone[2] + dx - e / 2.0, zone[3] + dy - e / 2.0]
            if boite[2] <= boite[0] or boite[3] <= boite[1]:
                return
            if r >= 1.0:
                d.rounded_rectangle(boite, radius=max(0.0, r - e / 2.0),
                                    outline=teinte, width=e)
            else:
                d.rectangle(boite, outline=teinte, width=e)
        self._pose(zone, dessine)

    def _op_polygone(self, points, couleur):
        transformes = [self._applique(px, py) for px, py in points]
        xs = [p[0] for p in transformes]
        ys = [p[1] for p in transformes]
        zone = (min(xs), min(ys), max(xs), max(ys))
        teinte = _teinte(couleur)
        self._pose(zone, lambda d, dx, dy: d.polygon(
            [(px + dx, py + dy) for px, py in transformes], fill=teinte))

    def _op_ligne(self, x1, y1, x2, y2, epaisseur, couleur):
        a = self._applique(x1, y1)
        b = self._applique(x2, y2)
        e = max(1, int(round(float(epaisseur) * self._echelle())))
        zone = (min(a[0], b[0]) - e, min(a[1], b[1]) - e,
                max(a[0], b[0]) + e, max(a[1], b[1]) + e)
        teinte = _teinte(couleur)
        self._pose(zone, lambda d, dx, dy: d.line(
            [a[0] + dx, a[1] + dy, b[0] + dx, b[1] + dy], fill=teinte, width=e))

    # -- Ombres ----------------------------------------------------------------

    def _op_ombre(self, x, y, l, h, rayon, flou, couleur):
        """Comme l'hote : des couches concentriques dont la somme fait un bord doux."""
        if l <= 0 or h <= 0:
            return
        couches = max(1, min(16, int(math.ceil(float(flou) or 1))))
        r, v, b, alpha = _teinte(couleur)
        part = max(1, int(alpha / float(couches + 1)))
        for i in range(couches, -1, -1):
            extension = float(flou) * i / couches
            self._op_rond(x - extension, y - extension,
                          l + 2 * extension, h + 2 * extension,
                          float(rayon) + extension,
                          (part << 24) | (r << 16) | (v << 8) | b)

    def _op_ombre_interne(self, x, y, l, h, rayon, dx, dy, flou, etendue, couleur):
        if l <= 0 or h <= 0:
            return
        self._op_clip(x, y, l, h)
        couches = max(1, min(16, int(math.ceil(float(flou) + float(etendue)))))
        r, v, b, alpha = _teinte(couleur)
        part = max(1, int(alpha / float(couches + 1)))
        for i in range(couches, -1, -1):
            epaisseur = (float(flou) + float(etendue)) * (i + 1) / (couches + 1)
            self._op_contour(x + dx, y + dy, l, h, rayon, max(1.0, epaisseur),
                             (part << 24) | (r << 16) | (v << 8) | b)
        self._op_declip()

    # -- Degrades --------------------------------------------------------------

    def _op_degrade(self, x, y, l, h, rayon, angle, etapes):
        radians = math.radians(float(angle))
        dx, dy = math.sin(radians), -math.cos(radians)
        portee = abs(l * dx) + abs(h * dy)
        cx, cy = x + l / 2.0, y + h / 2.0
        depart = (cx - dx * portee / 2.0, cy - dy * portee / 2.0)
        arrivee = (cx + dx * portee / 2.0, cy + dy * portee / 2.0)
        self._op_degrade_points(x, y, l, h, depart[0], depart[1],
                                arrivee[0], arrivee[1], etapes, rayon)

    def _op_degrade_points(self, x, y, l, h, x0, y0, x1, y1, etapes, rayon=0.0):
        if l <= 0 or h <= 0:
            return
        zone = self._rect(x, y, l, h)
        a = self._applique(x0, y0)
        b = self._applique(x1, y1)
        largeur = int(math.ceil(zone[2] - zone[0]))
        hauteur = int(math.ceil(zone[3] - zone[1]))
        if largeur <= 0 or hauteur <= 0:
            return

        # La bande est dessinee colonne par colonne le long de la ligne du
        # degrade : une projection par pixel suffit et reste rapide.
        vecteur = (b[0] - a[0], b[1] - a[1])
        norme = vecteur[0] ** 2 + vecteur[1] ** 2

        def dessine(d, ddx, ddy):
            for j in range(hauteur):
                for i in range(0, largeur, 2):
                    px, py = zone[0] + i, zone[1] + j
                    if norme <= 0:
                        t = 0.0
                    else:
                        t = ((px - a[0]) * vecteur[0]
                             + (py - a[1]) * vecteur[1]) / norme
                    d.rectangle([px + ddx, py + ddy, px + ddx + 1, py + ddy],
                                fill=_melange(etapes, t))
        self._pose(zone, dessine)

    def _op_degrade_radial(self, x, y, l, h, rayon, angle, etapes):
        cx, cy = x + l / 2.0, y + h / 2.0
        portee = math.sqrt(l * l + h * h) / 2.0
        self._op_degrade_cercle(x, y, l, h, cx, cy, portee, etapes)

    def _op_degrade_cercle(self, x, y, l, h, cx, cy, rayon, etapes):
        if l <= 0 or h <= 0:
            return
        zone = self._rect(x, y, l, h)
        centre = self._applique(cx, cy)
        r = max(1.0, float(rayon) * self._echelle())
        largeur = int(math.ceil(zone[2] - zone[0]))
        hauteur = int(math.ceil(zone[3] - zone[1]))

        def dessine(d, ddx, ddy):
            for j in range(hauteur):
                for i in range(0, largeur, 2):
                    px, py = zone[0] + i, zone[1] + j
                    distance = math.hypot(px - centre[0], py - centre[1]) / r
                    d.rectangle([px + ddx, py + ddy, px + ddx + 1, py + ddy],
                                fill=_melange(etapes, distance))
        self._pose(zone, dessine)

    # -- Texte -----------------------------------------------------------------

    def _op_texte(self, x, y, texte, couleur, taille, gras, italique, fixe,
                  souligne):
        if not texte:
            return
        echelle = self._echelle()
        fonte = FONTES.prend(float(taille) * echelle, gras, fixe)
        origine = self._applique(x, y)
        largeur = fonte.getlength(texte)
        hauteur = float(taille) * echelle * 1.6
        zone = (origine[0] - 2, origine[1] - 2,
                origine[0] + largeur + 2, origine[1] + hauteur + 2)
        teinte = _teinte(couleur)

        def dessine(d, dx, dy):
            d.text((origine[0] + dx, origine[1] + dy), texte, font=fonte,
                   fill=teinte)
            if souligne:
                base = origine[1] + dy + fonte.size * 1.15
                d.line([origine[0] + dx, base, origine[0] + dx + largeur, base],
                       fill=teinte, width=max(1, int(fonte.size / 14)))
        self._pose(zone, dessine)

    # -- Images ----------------------------------------------------------------

    def _op_image(self, x, y, l, h, identifiant):
        self._op_imagepart(x, y, l, h, identifiant, None, None, None, None)

    def _op_imagepart(self, x, y, l, h, identifiant, sx, sy, sl, sh):
        if identifiant is None or identifiant < 0 or identifiant >= len(self.images):
            return
        source = self.images[int(identifiant)]
        if source is None or l <= 0 or h <= 0:
            return
        if sx is not None:
            source = source.crop((int(sx), int(sy),
                                  int(sx + sl), int(sy + sh)))
        zone = self._rect(x, y, l, h)
        cible = (max(1, int(round(zone[2] - zone[0]))),
                 max(1, int(round(zone[3] - zone[1]))))
        redimensionnee = source.convert("RGBA").resize(cible, Image.LANCZOS)
        self._pose(zone, lambda d, dx, dy: None)   # borne le rognage
        cx0, cy0, cl, ch = self.rognage
        x0 = max(int(zone[0]), int(cx0), 0)
        y0 = max(int(zone[1]), int(cy0), 0)
        x1 = min(int(zone[0]) + cible[0], int(cx0 + cl), self.largeur)
        y1 = min(int(zone[1]) + cible[1], int(cy0 + ch), self.hauteur)
        if x1 <= x0 or y1 <= y0:
            return
        morceau = redimensionnee.crop((x0 - int(zone[0]), y0 - int(zone[1]),
                                       x1 - int(zone[0]), y1 - int(zone[1])))
        if self._opacite < 1.0:
            alpha = morceau.getchannel("A").point(
                lambda v: int(v * self._opacite))
            morceau.putalpha(alpha)
        self.toile.alpha_composite(morceau, (x0, y0))

    # -- Rognage et enveloppes -------------------------------------------------

    def _op_clip(self, x, y, l, h):
        zone = self._rect(x, y, l, h)
        nouvelle = (zone[0], zone[1], zone[2] - zone[0], zone[3] - zone[1])
        if self._rognages:
            ax, ay, al, ah = self._rognages[-1]
            gx = max(ax, nouvelle[0])
            gy = max(ay, nouvelle[1])
            nouvelle = (gx, gy,
                        max(0.0, min(ax + al, nouvelle[0] + nouvelle[2]) - gx),
                        max(0.0, min(ay + ah, nouvelle[1] + nouvelle[3]) - gy))
        self._rognages.append(nouvelle)

    def _op_declip(self):
        if self._rognages:
            self._rognages.pop()

    def _op_transforme(self, a, b, c, d, e, f, ox, oy):
        self._etats.append((self._matrice, self._opacite))
        vers = (1.0, 0.0, 0.0, 1.0, ox, oy)
        retour = (1.0, 0.0, 0.0, 1.0, -ox, -oy)
        locale = _compose(_compose(vers, (a, b, c, d, e, f)), retour)
        self._matrice = _compose(self._matrice, locale)

    def _op_opacite(self, alpha):
        self._etats.append((self._matrice, self._opacite))
        self._opacite *= float(alpha)

    def _op_desenveloppe(self):
        if self._etats:
            self._matrice, self._opacite = self._etats.pop()


def _compose(gauche, droite):
    a1, b1, c1, d1, e1, f1 = gauche
    a2, b2, c2, d2, e2, f2 = droite
    return (a1 * a2 + c1 * b2, b1 * a2 + d1 * b2,
            a1 * c2 + c1 * d2, b1 * c2 + d1 * d2,
            a1 * e2 + c1 * f2 + e1, b1 * e2 + d1 * f2 + f1)


def _melange(etapes, t):
    """La couleur d'un degrade a cette fraction, bornes comprises."""
    t = max(0.0, min(1.0, t))
    if not etapes:
        return (0, 0, 0, 0)
    avant = etapes[0]
    apres = etapes[-1]
    for arret in etapes:
        if arret[0] <= t:
            avant = arret
        else:
            apres = arret
            break
    else:
        apres = avant
    portee = apres[0] - avant[0]
    locale = 0.0 if portee <= 0 else (t - avant[0]) / portee
    un = _teinte(avant[1])
    autre = _teinte(apres[1])
    return tuple(int(round(un[i] + (autre[i] - un[i]) * locale)) for i in range(4))


# --- L'hote de substitution ----------------------------------------------------

def installe_hote(largeur, hauteur):
    """Pose un module `bo` adosse a Pillow, avant tout import du moteur."""
    bo = types.ModuleType("bo")

    def largeur_texte(texte, taille, gras=False, fixe=False):
        return FONTES.prend(taille, gras, fixe).getlength(str(texte))

    def hauteur_ligne(taille, fixe=False):
        fonte = FONTES.prend(taille, False, fixe)
        montee, descente = fonte.getmetrics()
        return float(montee + descente)

    def image(octets):
        import io
        try:
            ouverte = Image.open(io.BytesIO(bytes(octets)))
            ouverte.load()
        except Exception:  # noqa: BLE001
            return None
        IMAGES.append(ouverte)
        return (len(IMAGES) - 1, ouverte.width, ouverte.height)

    def image_brute(octets, l, h, identifiant=-1):
        try:
            ouverte = Image.frombytes("RGBA", (int(l), int(h)), bytes(octets), "raw",
                                      "BGRA")
        except Exception:  # noqa: BLE001
            return None
        IMAGES.append(ouverte)
        return (len(IMAGES) - 1, int(l), int(h))

    def rasterise(operations, l, h):
        hors = Peintre(int(l), int(h), fond=(0, 0, 0, 0))
        hors.joue(operations)
        return hors.toile.tobytes("raw", "BGRA")

    bo.largeur_texte = largeur_texte
    bo.hauteur_ligne = hauteur_ligne
    bo.taille = lambda: (largeur, hauteur)
    bo.titre = lambda _: None
    bo.redessiner = lambda: None
    bo.image = image
    bo.image_brute = image_brute
    bo.rasterise = rasterise
    bo.formats_images = lambda: ["png", "jpeg", "jpg", "gif", "bmp", "webp", "ico"]
    sys.modules["bo"] = bo

    # `bojs` n'est fourni que si QuickJS a ete construit. Sans lui, la page est
    # rendue sans JavaScript — ce qui reste la bonne image d'un site rendu cote
    # serveur, et le dit.
    if "bojs" not in sys.modules:
        try:
            import bojs  # noqa: F401
        except ImportError:
            pass


# --- Le reseau, par le proxy de l'environnement --------------------------------

def installe_reseau(reseau, agent=None):
    """Fait passer les requetes du moteur par `curl`.

    Le client HTTP du moteur ouvre ses propres prises et porte son propre
    resolveur DNS — ce qu'il faut sous Bouchaud OS, et ce qui ne marche pas
    derriere le proxy d'un conteneur. On lui substitue `curl`, qui honore
    `HTTPS_PROXY` et le magasin de certificats de l'environnement. Tout le reste
    du moteur — cache, prechargement, redirections — reste en place.
    """
    entete_agent = agent or ("Mozilla/5.0 (X11; Linux x86_64) "
                             "BouchaudOS/1.0 Nautile/2.0")

    def _charge_http(url, methode="GET", corps=None, entetes=None, brut=False):
        commande = ["curl", "-sS", "-L", "--max-time", "40",
                    "-A", entete_agent,
                    "-w", "\\n__BO_CODE__%{http_code}\\n__BO_TYPE__%{content_type}",
                    "-X", str(methode), url]
        for nom, valeur in (entetes or {}).items():
            commande[1:1] = ["-H", "%s: %s" % (nom, valeur)]
        if corps:
            commande[1:1] = ["--data-binary", corps if isinstance(corps, str)
                             else corps.decode("utf-8", "replace")]
        try:
            fini = subprocess.run(commande, capture_output=True, timeout=60)
        except Exception as e:  # noqa: BLE001
            return reseau.Reponse(url, "", "text/plain", 0, str(e))

        octets = fini.stdout
        code, type_mime = 0, "application/octet-stream"
        marque = octets.rfind(b"\n__BO_CODE__")
        if marque >= 0:
            queue = octets[marque + 1:].decode("utf-8", "replace")
            octets = octets[:marque]
            for ligne in queue.split("\n"):
                if ligne.startswith("__BO_CODE__"):
                    try:
                        code = int(ligne[len("__BO_CODE__"):].strip())
                    except ValueError:
                        code = 0
                elif ligne.startswith("__BO_TYPE__"):
                    type_mime = ligne[len("__BO_TYPE__"):].strip().split(";")[0] \
                        or type_mime
        if code == 0:
            return reseau.Reponse(url, "", "text/plain", 0,
                                  fini.stderr.decode("utf-8", "replace").strip()
                                  or "curl a echoue")
        # Le texte est decode dans tous les cas : `brut` ne veut pas dire « sans
        # texte » mais « sans enrobage HTML ». Une feuille de style est demandee
        # en brut et lue par `.contenu` — la vider ici affichait la page nue.
        texte = octets.decode("utf-8", "replace")
        if not brut and type_mime not in ("text/html", "application/xhtml+xml"):
            if type_mime.startswith("text/"):
                texte = "<pre>%s</pre>" % (texte.replace("&", "&amp;")
                                           .replace("<", "&lt;"))
            else:
                texte = ("<h1>Type non affichable</h1><p>%s — %d octets.</p>"
                         % (type_mime, len(octets)))
            type_mime = "text/html"
        return reseau.Reponse(url, texte, type_mime, code,
                              entetes={"content-type": type_mime}, octets=octets)

    reseau._charge_http = _charge_http


# --- Programme ----------------------------------------------------------------

def principal(argv=None):
    analyseur = argparse.ArgumentParser(
        description="Peint une page avec le moteur du navigateur, sans Qt.")
    analyseur.add_argument("cible", help="URL, ou chemin d'un fichier HTML")
    analyseur.add_argument("-o", "--sortie", default="apercu.png")
    analyseur.add_argument("--largeur", type=int, default=1280)
    analyseur.add_argument("--hauteur", type=int, default=900)
    analyseur.add_argument("--pleine-hauteur", action="store_true",
                           help="peint la page entiere, pas seulement la fenetre")
    analyseur.add_argument("--defilement", type=float, default=0.0)
    arguments = analyseur.parse_args(argv)

    sys.path.insert(0, RACINE)
    installe_hote(arguments.largeur, arguments.hauteur)

    import moteur
    from moteur import reseau
    installe_reseau(reseau)

    cible = arguments.cible
    if not cible.startswith(("http://", "https://", "file://")):
        cible = "file://" + os.path.abspath(cible)

    journal = []
    document = moteur.charge(cible, arguments.largeur,
                             journal=lambda n, t: journal.append((n, t)))
    document.remet_en_page(arguments.largeur, arguments.hauteur)
    # Un battement laisse le JavaScript de la page construire son contenu ; sans
    # lui, une page qui se remplit dans `DOMContentLoaded` sortirait vide.
    for _ in range(3):
        document.rafraichis()

    hauteur = arguments.hauteur
    if arguments.pleine_hauteur:
        hauteur = max(arguments.hauteur, int(document.hauteur) + 20)

    peintre = Peintre(arguments.largeur, hauteur)
    peintre.joue(document.liste_affichage(arguments.defilement,
                                          arguments.largeur, hauteur))
    peintre.toile.convert("RGB").save(arguments.sortie)

    print("%s -> %s (%dx%d, %d octets de page, %d operations)" % (
        cible, arguments.sortie, arguments.largeur, hauteur,
        len(document.racine.texte() or ""),
        len(document.liste_affichage(arguments.defilement, arguments.largeur,
                                     hauteur))))
    for niveau, texte in journal[:8]:
        print("  [%s] %s" % (niveau, texte))
    return 0


if __name__ == "__main__":
    sys.exit(principal())
