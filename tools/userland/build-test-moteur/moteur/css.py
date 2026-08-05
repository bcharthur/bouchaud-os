"""Feuilles de style : analyse, specificite, cascade et heritage.

Le sous-ensemble couvert est celui qui change vraiment l'apparence d'une page :
les selecteurs simples et descendants, le modele de boite, les couleurs, la
typographie. Ce qui manque (media queries, flex, grid, animations) est ignore
proprement plutot que mal interprete — une declaration inconnue est laissee de
cote, elle ne fait pas echouer la regle qui la contient.
"""

import re

# --- Couleurs ----------------------------------------------------------------

NOMS_COULEURS = {
    "black": 0x000000, "white": 0xFFFFFF, "red": 0xFF0000, "green": 0x008000,
    "blue": 0x0000FF, "yellow": 0xFFFF00, "cyan": 0x00FFFF, "aqua": 0x00FFFF,
    "magenta": 0xFF00FF, "fuchsia": 0xFF00FF, "gray": 0x808080, "grey": 0x808080,
    "silver": 0xC0C0C0, "maroon": 0x800000, "olive": 0x808000, "lime": 0x00FF00,
    "navy": 0x000080, "teal": 0x008080, "purple": 0x800080, "orange": 0xFFA500,
    "pink": 0xFFC0CB, "brown": 0xA52A2A, "gold": 0xFFD700, "indigo": 0x4B0082,
    "violet": 0xEE82EE, "beige": 0xF5F5DC, "ivory": 0xFFFFF0, "khaki": 0xF0E68C,
    "lavender": 0xE6E6FA, "salmon": 0xFA8072, "tan": 0xD2B48C, "turquoise": 0x40E0D0,
    "crimson": 0xDC143C, "darkblue": 0x00008B, "darkgreen": 0x006400,
    "darkred": 0x8B0000, "darkgray": 0xA9A9A9, "darkgrey": 0xA9A9A9,
    "lightgray": 0xD3D3D3, "lightgrey": 0xD3D3D3, "lightblue": 0xADD8E6,
    "steelblue": 0x4682B4, "royalblue": 0x4169E1, "dodgerblue": 0x1E90FF,
    "whitesmoke": 0xF5F5F5, "gainsboro": 0xDCDCDC, "transparent": None,
}


def couleur(valeur):
    """Traduit une couleur CSS en entier 0xAARRGGBB. `None` si transparente."""
    v = valeur.strip().lower()
    if not v or v in ("transparent", "none", "inherit", "initial"):
        return None
    if v.startswith("#"):
        chiffres = v[1:]
        if len(chiffres) == 3:
            chiffres = "".join(c * 2 for c in chiffres)
        if len(chiffres) == 4:
            chiffres = "".join(c * 2 for c in chiffres)
        try:
            if len(chiffres) == 6:
                return 0xFF000000 | int(chiffres, 16)
            if len(chiffres) == 8:
                # CSS met l'alpha en dernier, notre format le met en premier.
                brut = int(chiffres, 16)
                return ((brut & 0xFF) << 24) | (brut >> 8)
        except ValueError:
            return None
        return None
    m = re.match(r"rgba?\(([^)]*)\)", v)
    if m:
        parties = [p.strip() for p in m.group(1).replace("/", ",").split(",") if p.strip()]
        try:
            composantes = [_canal(p) for p in parties[:3]]
            alpha = 255
            if len(parties) > 3:
                a = parties[3]
                alpha = int(float(a[:-1]) * 255 / 100) if a.endswith("%") else int(float(a) * 255)
            alpha = max(0, min(255, alpha))
            if alpha == 0:
                return None
            return (alpha << 24) | (composantes[0] << 16) | (composantes[1] << 8) | composantes[2]
        except (ValueError, IndexError):
            return None
    if v in NOMS_COULEURS:
        base = NOMS_COULEURS[v]
        return None if base is None else 0xFF000000 | base
    return None


def _canal(texte):
    if texte.endswith("%"):
        return max(0, min(255, int(float(texte[:-1]) * 255 / 100)))
    return max(0, min(255, int(float(texte))))


# --- Longueurs ---------------------------------------------------------------

def longueur(valeur, reference=0.0, taille_police=16.0):
    """Traduit une longueur CSS en pixels. `None` si non exprimable."""
    v = valeur.strip().lower()
    if not v or v in ("auto", "inherit", "initial", "none"):
        return None
    try:
        if v.endswith("px"):
            return float(v[:-2])
        if v.endswith("rem"):
            return float(v[:-3]) * 16.0
        if v.endswith("em"):
            return float(v[:-2]) * taille_police
        if v.endswith("%"):
            return float(v[:-1]) * reference / 100.0
        if v.endswith("pt"):
            return float(v[:-2]) * 96.0 / 72.0
        if v.endswith("vw"):
            return float(v[:-2]) * reference / 100.0
        if v.endswith(("vh", "ex", "ch", "cm", "mm", "in", "pc")):
            return float(re.sub(r"[a-z]+$", "", v))
        return float(v)
    except ValueError:
        return None


# --- Selecteurs --------------------------------------------------------------

class Simple:
    """Un maillon de selecteur : balise, classes, identifiant."""

    __slots__ = ("balise", "classes", "identifiant")

    def __init__(self, texte):
        self.balise = None
        self.classes = []
        self.identifiant = None
        # On ignore les pseudo-classes : `a:hover` se comporte comme `a`. Les
        # prendre au pied de la lettre demanderait un etat d'interaction que le
        # moteur n'a pas ; les rejeter perdrait la regle entiere.
        texte = re.sub(r"::?[a-zA-Z-]+(\([^)]*\))?", "", texte)
        texte = re.sub(r"\[[^\]]*\]", "", texte)
        for morceau in re.findall(r"[.#]?[^.#]+", texte):
            if morceau.startswith("."):
                self.classes.append(morceau[1:])
            elif morceau.startswith("#"):
                self.identifiant = morceau[1:]
            elif morceau != "*":
                self.balise = morceau.lower()

    def correspond(self, element):
        if self.balise and element.balise != self.balise:
            return False
        if self.identifiant and element.identifiant != self.identifiant:
            return False
        if self.classes:
            presentes = set(element.classes)
            if not all(c in presentes for c in self.classes):
                return False
        return True

    def poids(self):
        return (1 if self.identifiant else 0, len(self.classes), 1 if self.balise else 0)


class Selecteur:
    """Une suite de maillons separes par des espaces (descendance)."""

    __slots__ = ("maillons", "specificite")

    def __init__(self, texte):
        # `>`, `+` et `~` sont ramenes a la descendance : moins precis, mais
        # infiniment plus proche du resultat attendu que de tout jeter.
        texte = re.sub(r"\s*[>+~]\s*", " ", texte)
        self.maillons = [Simple(m) for m in texte.split() if m]
        a = b = c = 0
        for maillon in self.maillons:
            pa, pb, pc = maillon.poids()
            a, b, c = a + pa, b + pb, c + pc
        self.specificite = (a, b, c)

    def correspond(self, chemin):
        """`chemin` est la liste des ancetres, du plus lointain a l'element."""
        if not self.maillons:
            return False
        index = len(self.maillons) - 1
        if not self.maillons[index].correspond(chemin[-1]):
            return False
        index -= 1
        position = len(chemin) - 2
        while index >= 0 and position >= 0:
            if self.maillons[index].correspond(chemin[position]):
                index -= 1
            position -= 1
        return index < 0


class Regle:
    __slots__ = ("selecteur", "declarations", "ordre")

    def __init__(self, selecteur, declarations, ordre):
        self.selecteur = selecteur
        self.declarations = declarations
        self.ordre = ordre


_COMMENTAIRE = re.compile(r"/\*.*?\*/", re.S)


def analyse(source, ordre_depart=0):
    """Analyse une feuille de style et rend la liste de ses regles."""
    source = _COMMENTAIRE.sub(" ", source)
    regles = []
    ordre = ordre_depart
    position = 0
    longueur_source = len(source)

    while position < longueur_source:
        accolade = source.find("{", position)
        if accolade < 0:
            break
        prelude = source[position:accolade].strip()

        # Regles @ : on saute leur bloc. Sauf @media, dont on garde le contenu —
        # une page dont tout le style est sous @media serait sinon nue.
        if prelude.startswith("@"):
            if prelude.lower().startswith("@media"):
                position = accolade + 1
                continue
            position = _saute_bloc(source, accolade)
            continue

        fin = source.find("}", accolade)
        if fin < 0:
            break
        declarations = _declarations(source[accolade + 1:fin])
        if declarations:
            for brut in prelude.split(","):
                brut = brut.strip()
                if brut:
                    regles.append(Regle(Selecteur(brut), declarations, ordre))
                    ordre += 1
        position = fin + 1

    return regles


def _saute_bloc(source, accolade):
    profondeur = 0
    for index in range(accolade, len(source)):
        if source[index] == "{":
            profondeur += 1
        elif source[index] == "}":
            profondeur -= 1
            if profondeur == 0:
                return index + 1
    return len(source)


def _declarations(bloc):
    resultat = {}
    for morceau in bloc.split(";"):
        if ":" not in morceau:
            continue
        nom, _, valeur = morceau.partition(":")
        nom = nom.strip().lower()
        valeur = valeur.strip()
        if nom and valeur:
            resultat[nom] = valeur
    return resultat


# --- Cascade -----------------------------------------------------------------

# Proprietes transmises aux descendants faute de valeur propre.
HERITEES = {
    "color", "font-size", "font-family", "font-weight", "font-style",
    "line-height", "text-align", "text-decoration", "white-space",
    "list-style-type", "visibility",
}


def applique(regles, element, chemin, style_parent):
    """Calcule le style d'un element : heritage, regles, style en ligne."""
    style = {p: v for p, v in style_parent.items() if p in HERITEES}

    correspondantes = []
    for regle in regles:
        if regle.selecteur.correspond(chemin):
            correspondantes.append(regle)
    correspondantes.sort(key=lambda r: (r.selecteur.specificite, r.ordre))
    for regle in correspondantes:
        style.update(_developpe(regle.declarations))

    en_ligne = element.attributs.get("style")
    if en_ligne:
        style.update(_developpe(_declarations(en_ligne)))
    return style


_RACCOURCIS_BOITE = ("margin", "padding")


def _developpe(declarations):
    """Developpe les raccourcis (`margin`, `padding`, `border`, `font`)."""
    resultat = {}
    for nom, valeur in declarations.items():
        if nom in _RACCOURCIS_BOITE:
            parties = valeur.split()
            haut, droite, bas, gauche = _quatre(parties)
            resultat["%s-top" % nom] = haut
            resultat["%s-right" % nom] = droite
            resultat["%s-bottom" % nom] = bas
            resultat["%s-left" % nom] = gauche
        elif nom == "border":
            for morceau in valeur.split():
                if couleur(morceau) is not None:
                    resultat["border-color"] = morceau
                elif morceau.endswith(("px", "em", "rem")) or morceau.isdigit():
                    resultat["border-width"] = morceau
                elif morceau in ("none", "hidden"):
                    resultat["border-width"] = "0"
        elif nom == "background":
            for morceau in valeur.split():
                if couleur(morceau) is not None:
                    resultat["background-color"] = morceau
                    break
        elif nom == "font":
            for morceau in valeur.split():
                if morceau in ("bold", "bolder"):
                    resultat["font-weight"] = "bold"
                elif morceau == "italic":
                    resultat["font-style"] = "italic"
                elif morceau[:1].isdigit():
                    resultat["font-size"] = morceau.split("/")[0]
        else:
            resultat[nom] = valeur
    return resultat


def _quatre(parties):
    if not parties:
        return ("0", "0", "0", "0")
    if len(parties) == 1:
        return (parties[0],) * 4
    if len(parties) == 2:
        return (parties[0], parties[1], parties[0], parties[1])
    if len(parties) == 3:
        return (parties[0], parties[1], parties[2], parties[1])
    return (parties[0], parties[1], parties[2], parties[3])


# --- Feuille par defaut ------------------------------------------------------

# L'equivalent de la feuille de l'agent utilisateur. Sans elle, une page sans
# CSS serait un bloc de texte uniforme : ce sont ces regles qui donnent aux
# titres leur taille et aux paragraphes leur respiration.
FEUILLE_PAR_DEFAUT = """
html, body { display: block; color: #202124; font-size: 16px; line-height: 1.5; }
body { margin: 8px; background-color: #ffffff; }
div, p, section, article, header, footer, nav, main, aside, figure,
form, fieldset, blockquote, pre, ul, ol, dl, dd, dt, li, table, tr,
h1, h2, h3, h4, h5, h6, hr, address, details, summary { display: block; }
head, script, style, title, meta, link, noscript, template { display: none; }
span, a, b, i, em, strong, small, code, label, abbr, cite, q, sub, sup,
time, mark, u, s, br, img, input, button, select, textarea { display: inline; }
h1 { font-size: 32px; font-weight: bold; margin: 21px 0; }
h2 { font-size: 24px; font-weight: bold; margin: 20px 0; }
h3 { font-size: 19px; font-weight: bold; margin: 18px 0; }
h4 { font-size: 16px; font-weight: bold; margin: 21px 0; }
h5 { font-size: 13px; font-weight: bold; margin: 22px 0; }
h6 { font-size: 11px; font-weight: bold; margin: 24px 0; }
p { margin: 16px 0; }
blockquote { margin: 16px 40px; }
ul, ol { margin: 16px 0; padding-left: 40px; }
li { display: list-item; }
dl { margin: 16px 0; }
dd { margin-left: 40px; }
pre { font-family: monospace; margin: 13px 0; white-space: pre;
      background-color: #f6f8fa; padding: 12px; }
code, kbd, samp { font-family: monospace; }
b, strong { font-weight: bold; }
i, em, cite, address { font-style: italic; }
a { color: #1a56db; text-decoration: underline; }
hr { margin: 8px 0; border-width: 1px; border-color: #d0d7de; }
table { margin: 8px 0; }
th { font-weight: bold; }
td, th { padding: 4px 8px; }
small { font-size: 13px; }
button, input, select, textarea { background-color: #f3f4f6; padding: 4px 8px;
                                  border-width: 1px; border-color: #c9ced6; }
mark { background-color: #fff3a3; }
"""
