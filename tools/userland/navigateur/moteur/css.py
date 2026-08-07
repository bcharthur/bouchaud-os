"""Feuilles de style : analyse, specificite, cascade et heritage.

Le sous-ensemble couvert est celui qui change vraiment l'apparence d'une page :
les selecteurs simples, descendants et enfants directs, le modele de boite, les
couleurs, la typographie, les proprietes de disposition flexible et en grille,
les `@media` — evaluees contre la taille de fenetre reelle — et les
pseudo-elements `::before`/`::after`.

Les proprietes personnalisees (`--fond`) et `var()` en font partie : c'est sur
elles que repose la mise en forme de tout site recent, et les ignorer n'affichait
pas une page approximative mais une page sans couleurs ni espacements.

Les coins arrondis, les ombres portees, les degrades lineaires, les
transformations et l'opacite en font partie aussi : ce sont eux qui separent
l'allure d'une page d'aujourd'hui de celle d'une page d'il y a vingt ans.

Ce qui manque (animations, degrades radiaux, transformations en trois
dimensions) est ignore proprement plutot que mal interprete : une declaration
inconnue est laissee de cote, elle ne fait pas echouer la regle qui la
contient.
"""

import math
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

# Taille de la fenetre, posee par le document avant chaque mise en page.
#
# `vw` et `vh` s'y rapportent, et les media queries aussi. C'est une variable de
# module parce que la longueur se calcule en dizaines d'endroits : la faire
# circuler en argument jusqu'a chacun d'eux couterait plus qu'il ne rapporte,
# et elle ne change qu'une fois par mise en page.
_FENETRE = [1280.0, 720.0]


def pose_fenetre(largeur, hauteur):
    """Declare la taille de la fenetre. A appeler avant toute mise en page."""
    _FENETRE[0] = float(largeur) or 1280.0
    _FENETRE[1] = float(hauteur) or 720.0


def fenetre():
    return (_FENETRE[0], _FENETRE[1])


# Taille de police de l'element racine, a laquelle `rem` se rapporte.
#
# La supposer a 16 px etait faux des qu'une page ecrit `html { font-size: 62.5% }`
# — la facon la plus repandue de faire valoir 1 rem pour 10 px. Tout le document
# sortait alors une fois et demie trop grand.
_RACINE = [16.0]


def pose_taille_racine(pixels):
    """Declare la taille de police de `<html>`. A appeler avant la mise en page."""
    _RACINE[0] = float(pixels) if pixels and pixels > 0 else 16.0


def taille_racine():
    return _RACINE[0]


def longueur(valeur, reference=0.0, taille_police=16.0):
    """Traduit une longueur CSS en pixels. `None` si non exprimable."""
    if valeur is None:
        return None
    v = valeur.strip().lower()
    if not v or v in ("auto", "inherit", "initial", "none"):
        return None
    if v.startswith("calc(") and v.endswith(")"):
        return _calcule(v[5:-1], reference, taille_police)
    try:
        if v.endswith("px"):
            return float(v[:-2])
        if v.endswith("rem"):
            return float(v[:-3]) * _RACINE[0]
        if v.endswith("em"):
            return float(v[:-2]) * taille_police
        if v.endswith("%"):
            return float(v[:-1]) * reference / 100.0
        if v.endswith("pt"):
            return float(v[:-2]) * 96.0 / 72.0
        # Les unites de fenetre se rapportent a la fenetre, pas au bloc
        # contenant : les confondre donnait des bandeaux de la hauteur de leur
        # parent au lieu de la hauteur de l'ecran.
        if v.endswith("vw"):
            return float(v[:-2]) * _FENETRE[0] / 100.0
        if v.endswith("vh"):
            return float(v[:-2]) * _FENETRE[1] / 100.0
        if v.endswith("vmin"):
            return float(v[:-4]) * min(_FENETRE) / 100.0
        if v.endswith("vmax"):
            return float(v[:-4]) * max(_FENETRE) / 100.0
        if v.endswith("ex"):
            return float(v[:-2]) * taille_police * 0.5
        if v.endswith("ch"):
            return float(v[:-2]) * taille_police * 0.5
        if v.endswith("cm"):
            return float(v[:-2]) * 96.0 / 2.54
        if v.endswith("mm"):
            return float(v[:-2]) * 96.0 / 25.4
        if v.endswith("in"):
            return float(v[:-2]) * 96.0
        if v.endswith("pc"):
            return float(v[:-2]) * 16.0
        return float(v)
    except ValueError:
        return None


_JETON_CALC = re.compile(r"([0-9.]+[a-z%]*|[-+*/()])")


def _calcule(expression, reference, taille_police):
    """Evalue un `calc()`. `None` si l'expression n'est pas evaluable.

    Les longueurs y sont d'abord converties en pixels, ce qui ramene le calcul a
    de l'arithmetique ordinaire — `calc(100% - 2rem)` devient `1280 - 32`. Sans
    cela il faudrait porter les unites jusqu'au bout, pour un resultat identique.
    """
    morceaux = []
    for jeton in _JETON_CALC.findall(expression):
        if jeton in "+-*/()":
            morceaux.append(jeton)
            continue
        if jeton.replace(".", "", 1).isdigit():
            morceaux.append(jeton)
            continue
        mesure = longueur(jeton, reference, taille_police)
        if mesure is None:
            return None
        morceaux.append(repr(mesure))
    rendu = " ".join(morceaux)
    if not rendu or not re.fullmatch(r"[0-9.eE+\-*/() ]+", rendu):
        return None
    try:
        # L'expression ne contient plus que des chiffres et des operateurs :
        # elle a ete reconstruite jeton par jeton, aucun texte de la page n'y
        # subsiste.
        return float(eval(rendu, {"__builtins__": {}}, {}))  # noqa: S307
    except Exception:  # noqa: BLE001
        return None


# --- Selecteurs --------------------------------------------------------------
#
# Un seul moteur sert la cascade et `querySelector`. C'etaient deux
# implementations paralleles, avec deux jeux de limites differents : une regle
# pouvait styler un element que `document.querySelectorAll` ne trouvait pas.
# Elles repondent maintenant la meme chose, et tout ce qui est ajoute ici
# profite aux deux.
#
# Ce qui est reconnu : les combinateurs ` `, `>`, `+`, `~` ; les selecteurs
# d'attribut avec leurs six operateurs et le drapeau d'insensibilite ; les
# pseudo-classes structurelles (`nth-child` et sa famille, `first-child`,
# `empty`, `root`…) ; les pseudo-classes fonctionnelles `:is()`, `:where()`,
# `:not()` et `:has()`.

# Un nom de classe ou d'identifiant. Les echappements comptent : les cadres CSS
# les plus repandus produisent des classes comme `md\:flex` ou `w-1\/2`, et un
# nom coupe au `\` ne designe plus rien.
_NOM = r"(?:\\.|[^\\.#:\[\]()>+~,\s])+"

# Les morceaux d'un selecteur compose (`a.b#c[d]:e`). Les pseudo-classes
# passent avant les classes : sans cela `.a:not(.b)` se lirait comme deux
# classes, dont l'une nommee `not(`.
_JETON = re.compile(r"""
      ::?[a-zA-Z-]+ (?: \( (?: [^()] | \( [^()]* \) )* \) )?
    | \[ [^\]]* \]
    | \.""" + _NOM + r"""
    | \#""" + _NOM + r"""
    | \*
    | [A-Za-z][A-Za-z0-9_-]*
""", re.X)

_ATTRIBUT = re.compile(r"""\[\s*([A-Za-z_:][-A-Za-z0-9_:.]*)\s*
                           (?:([~^$*|]?=)\s*("[^"]*"|'[^']*'|[^\]\s]*))?\s*
                           ([iIsS])?\s*\]""", re.X)

# Pseudo-classes d'etat. Au repos, elles ne designent rien — et c'est la
# reponse juste : les ignorer, ce que faisait le moteur, appliquait en
# permanence le style de survol, si bien qu'un bouton portait la couleur qu'il
# n'aurait du prendre que sous le pointeur.
_ETATS = {
    "hover", "active", "focus", "focus-visible", "focus-within",
    "target", "target-within", "visited", "user-invalid", "user-valid",
    "autofill", "placeholder-shown", "default", "indeterminate",
}

# Pseudo-elements engendres par la mise en page.
_ENGENDRES = {"before", "after"}

_STRUCTURELLES = {
    "root", "empty", "first-child", "last-child", "only-child",
    "first-of-type", "last-of-type", "only-of-type", "checked", "disabled",
    "enabled", "required", "optional", "read-only", "read-write",
    "link", "any-link", "scope",
}

_FONCTIONS_NTH = {
    "nth-child", "nth-last-child", "nth-of-type", "nth-last-of-type",
}


def _desechappe(nom):
    return re.sub(r"\\(.)", r"\1", nom)


def _est_element(nœud):
    """Un nœud d'element se reconnait a sa balise ; un nœud de texte n'en a pas.

    Duck-typing plutot qu'un `isinstance` : `css` ne doit pas dependre de
    `html`, sans quoi les deux modules ne peuvent plus s'importer l'un l'autre.
    """
    return getattr(nœud, "balise", None) is not None


def _parent(element):
    parent = getattr(element, "parent", None)
    return parent if parent is not None and _est_element(parent) else None


def _fratrie(element):
    parent = _parent(element)
    if parent is None:
        return [element]
    return [n for n in parent.enfants if _est_element(n)]


def _descendants(element):
    pile = [n for n in getattr(element, "enfants", ()) if _est_element(n)]
    while pile:
        nœud = pile.pop()
        yield nœud
        pile.extend(n for n in nœud.enfants if _est_element(n))


def _analyse_nth(argument):
    """`2n+1`, `odd`, `-n+3`, `4`… en couple `(a, b)` tel que `an + b`."""
    texte = argument.strip().lower().replace(" ", "")
    if texte == "odd":
        return 2, 1
    if texte == "even":
        return 2, 0
    m = re.fullmatch(r"([+-]?\d*)n([+-]\d+)?", texte)
    if m:
        tete = m.group(1)
        a = 1 if tete in ("", "+") else -1 if tete == "-" else int(tete)
        return a, int(m.group(2) or 0)
    try:
        return 0, int(texte)
    except ValueError:
        # Un argument incomprehensible ne doit designer personne, plutot que
        # tout le monde.
        return 0, 0


def _rang_verifie(a, b, rang):
    """`rang` est compte a partir de 1, comme dans la norme."""
    if a == 0:
        return rang == b
    reste = rang - b
    return reste % a == 0 and reste // a >= 0


class Simple:
    """Un selecteur compose : balise, classes, identifiant, attributs, pseudos."""

    __slots__ = ("balise", "classes", "identifiant", "pseudo", "attributs",
                 "structurelles", "nth", "groupes", "jamais", "_poids")

    def __init__(self, texte):
        self.balise = None
        self.classes = []
        self.identifiant = None
        # `::before` et `::after` designent une boite qui n'existe pas dans le
        # document : il faut donc les retenir, la mise en page les fabriquera.
        # C'est ainsi que la moitie des sites posent leurs icones et leurs
        # separateurs.
        self.pseudo = None
        self.attributs = []
        self.structurelles = []
        self.nth = []
        self.groupes = []
        # Vrai quand le maillon ne peut designer personne au repos : une
        # pseudo-classe d'etat, ou un pseudo-element que le moteur ne peint pas.
        self.jamais = False

        a = b = c = 0
        for jeton in _JETON.findall(texte):
            if jeton.startswith("["):
                m = _ATTRIBUT.match(jeton)
                if m:
                    valeur = m.group(3)
                    if valeur and valeur[:1] in "\"'":
                        valeur = valeur[1:-1]
                    insensible = (m.group(4) or "").lower() == "i"
                    self.attributs.append(
                        (m.group(1).lower(), m.group(2), valeur, insensible))
                    b += 1
                continue
            if jeton.startswith(":"):
                pa, pb, pc = self._pseudo(jeton)
                a, b, c = a + pa, b + pb, c + pc
                continue
            if jeton.startswith("."):
                self.classes.append(_desechappe(jeton[1:]))
                b += 1
            elif jeton.startswith("#"):
                self.identifiant = _desechappe(jeton[1:])
                a += 1
            elif jeton != "*":
                self.balise = jeton.lower()
                c += 1
        self._poids = (a, b, c)

    def _pseudo(self, jeton):
        """Range une pseudo-classe et rend ce qu'elle pese."""
        double = jeton.startswith("::")
        corps = jeton.lstrip(":")
        argument = None
        ouvrante = corps.find("(")
        if ouvrante >= 0:
            argument = corps[ouvrante + 1:-1]
            corps = corps[:ouvrante]
        corps = corps.lower()

        if corps in _ENGENDRES:
            self.pseudo = corps
            return 0, 0, 1
        if double:
            # `::placeholder`, `::selection`, `::-webkit-…` visent une boite que
            # le moteur ne peint pas. Les ignorer reportait leur mise en forme
            # sur l'element lui-meme — la couleur du texte d'invite devenait
            # celle du champ.
            self.jamais = True
            return 0, 0, 1
        if corps == "root":
            self.structurelles.append("root")
            return 0, 1, 0
        if corps in _ETATS:
            self.jamais = True
            return 0, 1, 0
        if corps in ("is", "where", "not", "has") and argument is not None:
            sous = groupes(argument)
            self.groupes.append((corps, sous))
            if corps == "where" or not sous:
                return 0, 0, 0
            pire = max(s.specificite for s in sous)
            return pire
        if corps in _FONCTIONS_NTH and argument is not None:
            # `nth-child(2n of .x)` n'est pas reconnu : seul le rang l'est.
            self.nth.append((corps, _analyse_nth(argument.split(" of ")[0])))
            return 0, 1, 0
        if corps in _STRUCTURELLES:
            self.structurelles.append(corps)
            return 0, 1, 0
        # Une pseudo-classe inconnue est ignoree plutot que rejetee : elle ne
        # doit pas emporter la regle qui la contient.
        return 0, 1, 0

    # -- Correspondance --------------------------------------------------------

    def correspond(self, element):
        if self.jamais:
            return False
        if self.balise and element.balise != self.balise:
            return False
        if self.identifiant and element.identifiant != self.identifiant:
            return False
        if self.classes:
            presentes = set(element.classes)
            if not all(c in presentes for c in self.classes):
                return False
        for nom, operateur, valeur, insensible in self.attributs:
            if not self._attribut(element, nom, operateur, valeur, insensible):
                return False
        for nom in self.structurelles:
            if not self._structurelle(element, nom):
                return False
        for fonction, (a, b) in self.nth:
            if not self._nth(element, fonction, a, b):
                return False
        for genre, sous in self.groupes:
            if not self._groupe(element, genre, sous):
                return False
        return True

    def _attribut(self, element, nom, operateur, valeur, insensible):
        if nom not in element.attributs:
            return False
        if operateur is None:
            return True
        presente = element.attributs[nom]
        if insensible:
            presente, valeur = presente.lower(), (valeur or "").lower()
        if operateur == "=":
            return presente == valeur
        if operateur == "^=":
            return bool(valeur) and presente.startswith(valeur)
        if operateur == "$=":
            return bool(valeur) and presente.endswith(valeur)
        if operateur == "*=":
            return bool(valeur) and valeur in presente
        if operateur == "~=":
            return valeur in presente.split()
        if operateur == "|=":
            return presente == valeur or presente.startswith(valeur + "-")
        return True

    def _structurelle(self, element, nom):
        if nom == "root":
            return _parent(element) is None
        if nom == "scope":
            return True
        if nom == "empty":
            return not any(
                _est_element(n) or getattr(n, "contenu", "").strip()
                for n in getattr(element, "enfants", ()))
        if nom in ("link", "any-link"):
            return element.balise in ("a", "area") and "href" in element.attributs
        if nom == "checked":
            return "checked" in element.attributs or "selected" in element.attributs
        if nom == "disabled":
            return "disabled" in element.attributs
        if nom == "enabled":
            return "disabled" not in element.attributs
        if nom == "required":
            return "required" in element.attributs
        if nom == "optional":
            return "required" not in element.attributs
        if nom == "read-only":
            return "readonly" in element.attributs or element.balise not in (
                "input", "textarea", "select")
        if nom == "read-write":
            return "readonly" not in element.attributs and element.balise in (
                "input", "textarea", "select")

        fratrie = _fratrie(element)
        if nom == "first-child":
            return bool(fratrie) and fratrie[0] is element
        if nom == "last-child":
            return bool(fratrie) and fratrie[-1] is element
        if nom == "only-child":
            return len(fratrie) == 1
        memes = [n for n in fratrie if n.balise == element.balise]
        if nom == "first-of-type":
            return bool(memes) and memes[0] is element
        if nom == "last-of-type":
            return bool(memes) and memes[-1] is element
        if nom == "only-of-type":
            return len(memes) == 1
        return True

    def _nth(self, element, fonction, a, b):
        fratrie = _fratrie(element)
        if fonction.endswith("of-type"):
            fratrie = [n for n in fratrie if n.balise == element.balise]
        try:
            rang = fratrie.index(element) + 1
        except ValueError:
            return False
        if "last" in fonction:
            rang = len(fratrie) - rang + 1
        return _rang_verifie(a, b, rang)

    def _groupe(self, element, genre, sous):
        if genre == "not":
            return not any(s.correspond_element(element) for s in sous)
        if genre in ("is", "where"):
            return any(s.correspond_element(element) for s in sous)
        if genre == "has":
            return self._has(element, sous)
        return True

    def _has(self, element, sous):
        for selecteur in sous:
            portee = selecteur.portee
            if portee == ">":
                candidats = [n for n in element.enfants if _est_element(n)]
            elif portee in ("+", "~"):
                candidats = _suivants(element, un_seul=portee == "+")
            else:
                candidats = _descendants(element)
            for candidat in candidats:
                if selecteur.correspond_element(candidat, ancre=element):
                    return True
        return False

    def poids(self):
        return self._poids

    def cle(self):
        """Le trait le plus discriminant du maillon, pour l'indexation.

        L'identifiant d'abord, une classe ensuite, la balise a defaut. `None`
        pour un maillon qu'aucun index ne peut restreindre.
        """
        if self.identifiant:
            return ("#", self.identifiant)
        if self.classes:
            return (".", self.classes[0])
        if self.balise:
            return ("b", self.balise)
        return None


def _precedents(element, un_seul):
    """Les freres qui precedent, du plus proche au plus lointain."""
    fratrie = _fratrie(element)
    try:
        rang = fratrie.index(element)
    except ValueError:
        return []
    avant = fratrie[:rang][::-1]
    return avant[:1] if un_seul else avant


def _suivants(element, un_seul):
    fratrie = _fratrie(element)
    try:
        rang = fratrie.index(element)
    except ValueError:
        return []
    apres = fratrie[rang + 1:]
    return apres[:1] if un_seul else apres


# Un combinateur en tete (`> .x` dans un `:has()`) donne sa portee au selecteur.
_TETE = re.compile(r"^\s*([>+~])\s*")


class Selecteur:
    """Une suite de maillons relies par ` `, `>`, `+` ou `~`."""

    __slots__ = ("maillons", "combinateurs", "specificite", "pseudo", "portee")

    def __init__(self, texte):
        texte = texte.strip()
        tete = _TETE.match(texte)
        self.portee = tete.group(1) if tete else ""
        if tete:
            texte = texte[tete.end():]

        self.maillons = []
        # `combinateurs[i]` relie `maillons[i-1]` a `maillons[i]`. La premiere
        # case ne sert pas ; elle existe pour que les indices coincident.
        self.combinateurs = []
        for morceau, lien in _decoupe(texte):
            self.maillons.append(Simple(morceau))
            self.combinateurs.append(lien)

        a = b = c = 0
        for maillon in self.maillons:
            pa, pb, pc = maillon.poids()
            a, b, c = a + pa, b + pb, c + pc
        self.specificite = (a, b, c)
        # Le pseudo-element est porte par le dernier maillon : dans
        # `.carte > p::after`, c'est le `p` qui recoit la boite.
        self.pseudo = self.maillons[-1].pseudo if self.maillons else None

    def cle(self):
        """La cle d'indexation du selecteur : celle de son dernier maillon."""
        return self.maillons[-1].cle() if self.maillons else None

    def correspond(self, chemin):
        """`chemin` est la lignee de l'element, du plus lointain a lui-meme."""
        if not chemin:
            return False
        return self.correspond_element(chemin[-1])

    def correspond_element(self, element, ancre=None):
        """L'element est-il designe ?

        `ancre` borne la remontee : dans `:has(.a .b)`, `.a` ne doit pas etre
        cherche au-dessus de l'element qui porte le `:has()`.
        """
        if not self.maillons:
            return False
        return self._verifie(len(self.maillons) - 1, element, ancre)

    def _verifie(self, index, element, ancre):
        if element is None or not self.maillons[index].correspond(element):
            return False
        if index == 0:
            return True
        lien = self.combinateurs[index]
        precedent = index - 1
        if lien == ">":
            parent = _parent(element)
            if parent is None or parent is ancre:
                return False
            return self._verifie(precedent, parent, ancre)
        if lien == "+":
            freres = _precedents(element, un_seul=True)
            return bool(freres) and self._verifie(precedent, freres[0], ancre)
        if lien == "~":
            return any(self._verifie(precedent, frere, ancre)
                       for frere in _precedents(element, un_seul=False))
        # Descendance : on remonte la lignee, avec retour en arriere. Le
        # parcours glouton d'avant se trompait des qu'un ancetre correspondait
        # sans que la suite du selecteur suive.
        courant = element
        while True:
            courant = _parent(courant)
            if courant is None or courant is ancre:
                return False
            if self._verifie(precedent, courant, ancre):
                return True


def _decoupe(texte):
    """Decoupe un selecteur en couples `(compose, lien au precedent)`.

    Le decoupage respecte les parentheses et les crochets : l'espace de
    `:not(.a .b)` ou de `[title="a b"]` ne separe pas deux maillons.
    """
    morceaux = []
    courant = ""
    lien = " "
    profondeur = 0
    for caractere in texte:
        if profondeur == 0 and caractere in " \t\n\r\f>+~":
            if courant:
                morceaux.append((courant, lien))
                courant = ""
                lien = " "
            if caractere in ">+~":
                lien = caractere
            continue
        if caractere in "([":
            profondeur += 1
        elif caractere in ")]":
            profondeur -= 1
        courant += caractere
    if courant:
        morceaux.append((courant, lien))
    if morceaux:
        # Le premier maillon n'est relie a rien.
        morceaux[0] = (morceaux[0][0], " ")
    return morceaux


def groupes(texte):
    """Les selecteurs d'une liste separee par des virgules."""
    resultat = []
    courant = ""
    profondeur = 0
    for caractere in texte:
        if caractere == "," and profondeur == 0:
            if courant.strip():
                resultat.append(Selecteur(courant))
            courant = ""
            continue
        if caractere in "([":
            profondeur += 1
        elif caractere in ")]":
            profondeur -= 1
        courant += caractere
    if courant.strip():
        resultat.append(Selecteur(courant))
    return [s for s in resultat if s.maillons]


def correspond_un(liste, element):
    """L'element est-il designe par l'un de ces selecteurs ?"""
    return any(s.correspond_element(element) for s in liste)


class Index:
    """Les regles rangees par ce que leur dernier maillon exige.

    Sans index, styler un element coute un essai par regle de la feuille : sur
    une page de trois mille elements et une feuille de deux mille regles, cela
    fait six millions d'essais par mise en page — et la mise en page est refaite
    a chaque battement du JavaScript. L'index ne laisse passer que les regles
    dont le dernier maillon peut correspondre, ce qui en ecarte la quasi-
    totalite d'un seul coup de dictionnaire.

    Ce que l'index ne change pas : le resultat. Une regle ecartee ici est une
    regle dont le dernier maillon exige un identifiant, une classe ou une balise
    que l'element n'a pas — elle n'aurait pas correspondu.
    """

    __slots__ = ("par_id", "par_classe", "par_balise", "universelles")

    def __init__(self, regles):
        self.par_id = {}
        self.par_classe = {}
        self.par_balise = {}
        self.universelles = []
        for regle in regles:
            cle = regle.selecteur.cle()
            if cle is None:
                self.universelles.append(regle)
                continue
            genre, valeur = cle
            table = (self.par_id if genre == "#"
                     else self.par_classe if genre == "." else self.par_balise)
            table.setdefault(valeur, []).append(regle)

    def candidates(self, element):
        """Les regles susceptibles de s'appliquer a cet element."""
        liste = list(self.universelles)
        identifiant = element.identifiant
        if identifiant:
            liste.extend(self.par_id.get(identifiant, ()))
        for classe in element.classes:
            liste.extend(self.par_classe.get(classe, ()))
        liste.extend(self.par_balise.get(element.balise, ()))
        return liste


def indexe(regles):
    """Range une liste de regles pour la consultation. Voir [`Index`]."""
    return Index(regles)


class Regle:
    __slots__ = ("selecteur", "declarations", "ordre")

    def __init__(self, selecteur, declarations, ordre):
        self.selecteur = selecteur
        self.declarations = declarations
        self.ordre = ordre


# --- Decoration ---------------------------------------------------------------
#
# Coins arrondis, ombres portees, degrades, transformations et opacite. Ce sont
# les cinq proprietes qui separent une page de 1998 d'une page d'aujourd'hui :
# une carte moderne est un rectangle arrondi, pose sur une ombre douce, parfois
# rempli d'un degrade. Le moteur les analysait deja pour `border-radius` — sans
# jamais s'en servir — et laissait tomber les quatre autres.


def separe(texte, separateur=","):
    """Decoupe en respectant les parentheses : `rgb(1, 2, 3)` reste entier."""
    morceaux = []
    courant = ""
    profondeur = 0
    for caractere in texte:
        if caractere == "(":
            profondeur += 1
        elif caractere == ")":
            profondeur -= 1
        if caractere == separateur and profondeur == 0:
            morceaux.append(courant)
            courant = ""
            continue
        courant += caractere
    if courant.strip():
        morceaux.append(courant)
    return [m.strip() for m in morceaux if m.strip()]


def rayons(valeur, reference=0.0, taille_police=16.0):
    """`border-radius` en quatre rayons, dans l'ordre des coins CSS.

    Haut-gauche, haut-droit, bas-droit, bas-gauche. La forme elliptique
    (`10px / 20px`) est ramenee a son premier terme : le moteur ne peint que
    des coins circulaires.
    """
    if not valeur:
        return (0.0, 0.0, 0.0, 0.0)
    valeur = valeur.split("/")[0]
    parties = [longueur(p, reference, taille_police) or 0.0 for p in valeur.split()]
    if not parties:
        return (0.0, 0.0, 0.0, 0.0)
    if len(parties) == 1:
        return (parties[0],) * 4
    if len(parties) == 2:
        return (parties[0], parties[1], parties[0], parties[1])
    if len(parties) == 3:
        return (parties[0], parties[1], parties[2], parties[1])
    return tuple(parties[:4])


def ombres(valeur, taille_police=16.0):
    """`box-shadow` en liste de `(dx, dy, flou, etendue, couleur, interieure)`."""
    if not valeur or valeur.strip() in ("none", "initial"):
        return []
    resultat = []
    for morceau in separe(valeur):
        interieure = False
        longueurs = []
        teinte = None
        for jeton in separe(morceau, " "):
            if jeton == "inset":
                interieure = True
                continue
            mesure = longueur(jeton, 0.0, taille_police)
            # Une couleur peut s'ecrire `#0003` ou `rgb(…)` ; une longueur
            # commence toujours par un chiffre ou un signe.
            if mesure is not None and jeton[:1] in "+-.0123456789":
                longueurs.append(mesure)
            elif teinte is None:
                teinte = couleur(jeton)
        if len(longueurs) < 2:
            continue
        dx, dy = longueurs[0], longueurs[1]
        flou = longueurs[2] if len(longueurs) > 2 else 0.0
        etendue = longueurs[3] if len(longueurs) > 3 else 0.0
        resultat.append((dx, dy, flou, etendue,
                         teinte if teinte is not None else 0x40000000, interieure))
    return resultat


# `linear-gradient(…)`, avec ou sans prefixe de fabricant.
_DEGRADE = re.compile(r"(?:repeating-)?linear-gradient\(", re.I)


def degrade(valeur):
    """`linear-gradient(…)` en `(angle, [(position, couleur), …])`.

    L'angle suit la norme CSS : 0 degre pointe vers le haut, et le sens est
    celui des aiguilles d'une montre. `None` si la valeur n'est pas un degrade
    lineaire — les degrades radiaux et coniques ne sont pas peints.
    """
    if not valeur:
        return None
    trouve = _DEGRADE.search(valeur)
    if not trouve:
        return None
    interieur = _entre_parentheses(valeur, trouve.end() - 1)
    if interieur is None:
        return None
    morceaux = separe(interieur)
    if not morceaux:
        return None

    angle = 180.0          # `to bottom`, le defaut de CSS
    premier = morceaux[0].lower()
    if premier.startswith("to "):
        angle = _angle_vers(premier[3:])
        morceaux = morceaux[1:]
    elif premier.endswith(("deg", "turn", "rad", "grad")):
        angle = _angle(premier)
        morceaux = morceaux[1:]

    etapes = []
    for index, morceau in enumerate(morceaux):
        parties = morceau.split()
        teinte = couleur(parties[0]) if parties else None
        if teinte is None and parties:
            # `rgb(1 2 3)` a pu etre coupe : on retente sur le morceau entier.
            teinte = couleur(morceau)
        if teinte is None:
            continue
        position = None
        for part in parties[1:]:
            if part.endswith("%"):
                try:
                    position = float(part[:-1]) / 100.0
                except ValueError:
                    position = None
        if position is None:
            position = index / max(1.0, len(morceaux) - 1.0)
        etapes.append((max(0.0, min(1.0, position)), teinte))
    if len(etapes) < 2:
        return None
    return (angle, etapes)


def _entre_parentheses(texte, ouvrante):
    profondeur = 0
    for index in range(ouvrante, len(texte)):
        if texte[index] == "(":
            profondeur += 1
        elif texte[index] == ")":
            profondeur -= 1
            if profondeur == 0:
                return texte[ouvrante + 1:index]
    return None


_VERS = {
    "top": 0.0, "right": 90.0, "bottom": 180.0, "left": 270.0,
    "top right": 45.0, "right top": 45.0,
    "bottom right": 135.0, "right bottom": 135.0,
    "bottom left": 225.0, "left bottom": 225.0,
    "top left": 315.0, "left top": 315.0,
}


def _angle_vers(mots):
    return _VERS.get(" ".join(mots.split()), 180.0)


def _angle(texte):
    """Un angle CSS en degres."""
    texte = texte.strip().lower()
    try:
        if texte.endswith("deg"):
            return float(texte[:-3])
        if texte.endswith("turn"):
            return float(texte[:-4]) * 360.0
        if texte.endswith("rad"):
            return float(texte[:-3]) * 180.0 / 3.141592653589793
        if texte.endswith("grad"):
            return float(texte[:-4]) * 0.9
        return float(texte)
    except ValueError:
        return 0.0


_FONCTION = re.compile(r"([a-zA-Z0-9]+)\(([^()]*)\)")


def transformation(valeur, largeur=0.0, hauteur=0.0, taille_police=16.0):
    """`transform` en matrice `(a, b, c, d, e, f)`, ou `None`.

    La convention est celle de CSS et de Qt :
    `x' = a·x + c·y + e` et `y' = b·x + d·y + f`.

    Les pourcentages d'une translation se rapportent a la taille de l'element
    lui-meme, d'ou `largeur` et `hauteur`.
    """
    if not valeur or valeur.strip() in ("none", "initial"):
        return None
    matrice = (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
    vue = False
    for nom, arguments in _FONCTION.findall(valeur):
        parties = [p.strip() for p in arguments.split(",") if p.strip()]
        nom = nom.lower()
        suivante = _fonction_transform(nom, parties, largeur, hauteur, taille_police)
        if suivante is None:
            continue
        matrice = _compose(matrice, suivante)
        vue = True
    return matrice if vue else None


def _fonction_transform(nom, parties, largeur, hauteur, taille_police):
    def mesure(texte, reference):
        if texte.endswith("%"):
            try:
                return float(texte[:-1]) / 100.0 * reference
            except ValueError:
                return 0.0
        return longueur(texte, reference, taille_police) or 0.0

    def nombre(texte, defaut=0.0):
        try:
            return float(texte)
        except (ValueError, TypeError):
            return defaut

    if nom in ("translate", "translate3d"):
        dx = mesure(parties[0], largeur) if parties else 0.0
        dy = mesure(parties[1], hauteur) if len(parties) > 1 else 0.0
        return (1.0, 0.0, 0.0, 1.0, dx, dy)
    if nom == "translatex":
        return (1.0, 0.0, 0.0, 1.0, mesure(parties[0], largeur) if parties else 0.0, 0.0)
    if nom == "translatey":
        return (1.0, 0.0, 0.0, 1.0, 0.0, mesure(parties[0], hauteur) if parties else 0.0)
    if nom in ("scale", "scale3d"):
        sx = nombre(parties[0], 1.0) if parties else 1.0
        sy = nombre(parties[1], sx) if len(parties) > 1 else sx
        return (sx, 0.0, 0.0, sy, 0.0, 0.0)
    if nom == "scalex":
        return (nombre(parties[0], 1.0) if parties else 1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
    if nom == "scaley":
        return (1.0, 0.0, 0.0, nombre(parties[0], 1.0) if parties else 1.0, 0.0, 0.0)
    if nom in ("rotate", "rotatez"):
        radians = math.radians(_angle(parties[0])) if parties else 0.0
        cos, sin = math.cos(radians), math.sin(radians)
        return (cos, sin, -sin, cos, 0.0, 0.0)
    if nom in ("skew", "skewx", "skewy"):
        premier = math.tan(math.radians(_angle(parties[0]))) if parties else 0.0
        second = math.tan(math.radians(_angle(parties[1]))) if len(parties) > 1 else 0.0
        if nom == "skewx":
            return (1.0, 0.0, premier, 1.0, 0.0, 0.0)
        if nom == "skewy":
            return (1.0, premier, 0.0, 1.0, 0.0, 0.0)
        return (1.0, second, premier, 1.0, 0.0, 0.0)
    if nom == "matrix" and len(parties) >= 6:
        return tuple(nombre(p) for p in parties[:6])
    # `rotateX`, `perspective`, `matrix3d`… demandent une troisieme dimension
    # que la peinture n'a pas. Les ignorer vaut mieux que de les aplatir a tort.
    return None


def _compose(gauche, droite):
    """Applique `droite` puis `gauche`, comme l'exige l'ordre de CSS."""
    a1, b1, c1, d1, e1, f1 = gauche
    a2, b2, c2, d2, e2, f2 = droite
    return (
        a1 * a2 + c1 * b2,
        b1 * a2 + d1 * b2,
        a1 * c2 + c1 * d2,
        b1 * c2 + d1 * d2,
        a1 * e2 + c1 * f2 + e1,
        b1 * e2 + d1 * f2 + f1,
    )


def opacite(valeur):
    """`opacity` en flottant borne, `1.0` par defaut."""
    if valeur is None or valeur == "":
        return 1.0
    texte = valeur.strip()
    try:
        if texte.endswith("%"):
            return max(0.0, min(1.0, float(texte[:-1]) / 100.0))
        return max(0.0, min(1.0, float(texte)))
    except ValueError:
        return 1.0


COTES = ("top", "right", "bottom", "left")


def bordures(style, reference=0.0, taille_police=16.0):
    """Les quatre bords, chacun en `(epaisseur, couleur)`.

    Le moteur ne lisait qu'une epaisseur et une couleur pour toute la boite :
    un `border-bottom: 1px solid #eee` — le separateur le plus repandu du web —
    ne peignait donc rien du tout, et un `border-left` d'accent non plus.
    """
    resultat = []
    for cote in COTES:
        style_cote = style.get("border-%s-style" % cote, "none")
        epaisseur = longueur(style.get("border-%s-width" % cote, "0"),
                             reference, taille_police) or 0.0
        if style_cote in ("none", "hidden"):
            epaisseur = 0.0
        teinte = couleur(style.get("border-%s-color" % cote, ""))
        if teinte is None:
            # La norme veut que la couleur par defaut soit celle du texte.
            teinte = couleur(style.get("color", "")) or 0xFF202124
        resultat.append((epaisseur, teinte))
    return resultat


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

        # Regles @. `@media` est **evaluee** : garder son contenu sans le
        # verifier, comme on le faisait, appliquait la mise en page telephone
        # par-dessus celle du bureau — les regles de la derniere requete
        # l'emportaient, quelle que soit la largeur reelle.
        if prelude.startswith("@"):
            tete = prelude.lower()
            if tete.startswith("@media"):
                if requete_verifiee(prelude[len("@media"):]):
                    position = accolade + 1
                else:
                    position = _saute_bloc(source, accolade)
                continue
            if tete.startswith("@supports"):
                # On sait faire l'essentiel de ce qui s'y teste : on entre.
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


_CONDITION = re.compile(r"\(\s*([a-z-]+)\s*(?::\s*([^)]+))?\)")


def requete_verifiee(requete):
    """La requete media est-elle vraie pour la fenetre courante ?

    Ce qui est reconnu : les largeurs et hauteurs minimales et maximales, le
    type de media, l'orientation, et les conjonctions `and`. Les disjonctions
    (`,`) sont vraies des qu'un de leurs termes l'est. Ce qui ne l'est pas —
    resolution, preference de theme, pointeur — est considere comme vrai : mieux
    vaut appliquer une regle de trop qu'ecarter la mise en page entiere.
    """
    largeur, hauteur = fenetre()
    for terme in requete.split(","):
        terme = terme.strip().lower()
        if not terme:
            continue
        if _terme_verifie(terme, largeur, hauteur):
            return True
    return not requete.strip()


def _terme_verifie(terme, largeur, hauteur):
    negation = terme.startswith("not ")
    if negation:
        terme = terme[4:].strip()

    resultat = True
    # Type de media : `print` ne nous concerne pas, `screen` et `all` oui.
    tete = terme.split("(")[0].strip().split(" and ")[0].strip()
    if tete in ("print", "speech"):
        resultat = False

    for nom, valeur in _CONDITION.findall(terme):
        if not _condition_verifiee(nom, valeur, largeur, hauteur):
            resultat = False
            break
    return not resultat if negation else resultat


def _condition_verifiee(nom, valeur, largeur, hauteur):
    mesure = longueur(valeur, largeur, 16.0) if valeur else None
    if nom == "min-width":
        return mesure is None or largeur >= mesure
    if nom == "max-width":
        return mesure is None or largeur <= mesure
    if nom == "min-height":
        return mesure is None or hauteur >= mesure
    if nom == "max-height":
        return mesure is None or hauteur <= mesure
    if nom == "width":
        return mesure is None or abs(largeur - mesure) < 1.0
    if nom == "orientation":
        voulue = (valeur or "").strip()
        return voulue == ("landscape" if largeur >= hauteur else "portrait")
    # Tout le reste — resolution, prefers-color-scheme, hover, pointer — est
    # tenu pour vrai : la regle s'applique, ce qui est le moindre mal.
    return True


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


def applique(regles, element, chemin, style_parent, pseudo=None):
    """Calcule le style d'un element : heritage, regles, style en ligne.

    `pseudo` vaut `"before"` ou `"after"` pour calculer le style de la boite
    engendree plutot que celui de l'element lui-meme. Le style en ligne ne s'y
    applique pas : l'attribut `style` vise l'element, pas ses boites engendrees.
    """
    style = {p: v for p, v in style_parent.items() if p in HERITEES}
    # Les proprietes personnalisees s'heritent toutes, sans liste : c'est ce qui
    # permet a `:root { --fond: … }` de servir a toute la page.
    variables = {p: v for p, v in style_parent.items() if p.startswith("--")}

    # `regles` est soit un [`Index`], soit une liste brute. La liste reste
    # acceptee pour que le moteur soit utilisable sans preparation, mais toute
    # mise en page reelle passe par l'index.
    candidates = regles.candidates(element) if isinstance(regles, Index) else regles

    correspondantes = []
    for regle in candidates:
        if regle.selecteur.pseudo != pseudo:
            continue
        if regle.selecteur.correspond(chemin):
            correspondantes.append(regle)
    correspondantes.sort(key=lambda r: (r.selecteur.specificite, r.ordre))

    en_ligne = {}
    if pseudo is None:
        brut = element.attributs.get("style")
        if brut:
            en_ligne = _declarations(brut)

    # Les variables se resolvent en deux temps, comme le veut la norme : toute
    # la cascade des `--*` d'abord, les valeurs qui s'y referent ensuite. Sans
    # cela, une regle placee avant celle qui definit la variable verrait une
    # valeur vide alors que la cascade la lui donne.
    for source in (r.declarations for r in correspondantes):
        for nom, valeur in source.items():
            if nom.startswith("--"):
                variables[nom] = valeur.strip()
    for nom, valeur in en_ligne.items():
        if nom.startswith("--"):
            variables[nom] = valeur.strip()
    style.update(variables)

    for regle in correspondantes:
        style.update(_developpe(_resout_variables(regle.declarations, variables)))
    if en_ligne:
        style.update(_developpe(_resout_variables(en_ligne, variables)))
    return style


# `var(--nom)` ou `var(--nom, valeur de secours)`. La valeur de secours peut
# elle-meme contenir des parentheses — un `var()` imbrique, un `calc()` — d'ou
# le motif qui accepte un niveau de parentheses en son sein.
_VAR = re.compile(
    r"var\(\s*(--[A-Za-z0-9_-]+)\s*(?:,\s*((?:[^()]|\([^()]*\))*))?\)")

# Au-dela, on cesse de substituer : une variable qui se refere a elle-meme
# boucherait sinon indefiniment, et la norme declare ce cycle invalide.
_PROFONDEUR_VAR = 8


def _resout_variables(declarations, variables):
    """Remplace les `var()` d'un bloc de declarations par leur valeur.

    Une variable sans valeur ni secours rend une chaine vide : la declaration
    devient alors inexploitable, ce qui est le comportement voulu — la norme la
    dit invalide, et une longueur ou une couleur vide est deja ignoree partout
    ailleurs dans ce module.
    """
    if not variables:
        # Aucun `--*` en vue : la quasi-totalite des pages n'en ont pas sur la
        # plupart de leurs elements, et parcourir chaque valeur pour rien
        # couterait a chaque mise en page.
        resultat = {}
        for nom, valeur in declarations.items():
            if not nom.startswith("--"):
                resultat[nom] = valeur
        return resultat

    resultat = {}
    for nom, valeur in declarations.items():
        if nom.startswith("--"):
            continue
        resultat[nom] = _substitue(valeur, variables) if "var(" in valeur else valeur
    return resultat


def _substitue(valeur, variables):
    def remplace(trouve):
        nom = trouve.group(1)
        secours = (trouve.group(2) or "").strip()
        if nom in variables:
            return variables[nom]
        return secours

    for _ in range(_PROFONDEUR_VAR):
        suivante = _VAR.sub(remplace, valeur)
        if suivante == valeur:
            break
        valeur = suivante
    return valeur.strip()


def contenu_engendre(regles, element, chemin, style_parent, pseudo):
    """Style d'un `::before`/`::after`, ou `None` s'il n'y en a pas.

    Une boite engendree n'existe que si une regle lui donne un `content` : c'est
    la norme, et c'est ce qui evite d'en fabriquer une pour chaque element de la
    page.
    """
    style = applique(regles, element, chemin, style_parent, pseudo)
    if "content" not in style:
        return None
    if style.get("display", "inline") == "none":
        return None
    return style


def texte_du_contenu(valeur):
    """Traduit la valeur de `content` en texte affichable.

    `attr()` est reconnu par l'appelant, qui seul connait l'element ; les
    guillemets sont retires, et les fonctions qu'on ne sait pas evaluer —
    `counter()`, `url()` — rendent une chaine vide plutot qu'un texte parasite.
    """
    valeur = (valeur or "").strip()
    if not valeur or valeur in ("none", "normal"):
        return ""
    if valeur[:1] in "\"'" and valeur[-1:] == valeur[:1]:
        return _echappements(valeur[1:-1])
    if valeur.startswith(("url(", "counter(", "counters(")):
        return ""
    return ""


def _echappements(texte):
    """`\\2014` devient un tiret cadratin : c'est ainsi qu'on ecrit un
    caractere dans un `content`."""
    def remplace(m):
        try:
            return chr(int(m.group(1), 16))
        except ValueError:
            return ""
    return re.sub(r"\\([0-9a-fA-F]{1,6})\s?", remplace, texte)


_RACCOURCIS_BOITE = ("margin", "padding")

_BORDS = ("border-top", "border-right", "border-bottom", "border-left")

# Une position de grille : un numero de ligne, un `span`, ou `auto`.
_EST_LIGNE = re.compile(r"(auto|span\b|[-+]?\d)", re.I)

_STYLES_TRAIT = ("none", "hidden", "solid", "dashed", "dotted", "double",
                 "groove", "ridge", "inset", "outset")


def _trait(valeur):
    """`1px solid #eee` en `(epaisseur, style, couleur)`, chacun facultatif."""
    largeur = style = teinte = None
    for morceau in separe(valeur, " "):
        bas = morceau.lower()
        if bas in _STYLES_TRAIT:
            style = bas
            if bas in ("none", "hidden") and largeur is None:
                largeur = "0"
        elif longueur(morceau) is not None and bas[:1] in "+-.0123456789":
            largeur = morceau
        elif couleur(morceau) is not None:
            teinte = morceau
    # `border: solid red` sans epaisseur vaut 3 px dans la norme ; c'est aussi
    # ce que fait tout navigateur.
    if style not in (None, "none", "hidden") and largeur is None:
        largeur = "3px"
    return largeur, style, teinte

# Proprietes que la disposition consulte et qui ne doivent surtout pas etre
# heritees : un `display: flex` herite ferait de chaque descendant un conteneur
# flexible. La liste sert de garde-fou lisible plus que de mecanisme — c'est
# `HERITEES` qui decide — mais elle documente l'intention.
NON_HERITEES = (
    "display", "position", "top", "right", "bottom", "left", "z-index",
    "flex-direction", "flex-wrap", "flex-grow", "flex-shrink", "flex-basis",
    "justify-content", "align-items", "align-self", "align-content",
    "grid-template-columns", "grid-template-rows", "grid-column-start",
    "grid-column-end", "grid-row-start", "grid-row-end",
    "row-gap", "column-gap", "overflow", "overflow-x", "overflow-y",
    "box-sizing", "min-width", "max-width", "min-height", "max-height",
    "content", "border-radius", "opacity", "transform", "box-shadow",
    "background-image", "order",
) + tuple("border-%s-%s" % (c, t) for c in COTES
          for t in ("width", "style", "color"))


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
        elif nom == "border" or nom in _BORDS:
            # Un bord se decrit d'un trait : `1px solid #eee`. Le developper en
            # trois proprietes par cote est ce qui permet a la cascade de faire
            # son travail — `border: 1px solid red; border-bottom: none` doit
            # laisser trois bords rouges et en effacer un seul.
            largeur_b, style_b, couleur_b = _trait(valeur)
            cotes = COTES if nom == "border" else (nom.split("-", 1)[1],)
            for cote in cotes:
                if largeur_b is not None:
                    resultat["border-%s-width" % cote] = largeur_b
                if style_b is not None:
                    resultat["border-%s-style" % cote] = style_b
                if couleur_b is not None:
                    resultat["border-%s-color" % cote] = couleur_b
        elif nom in ("border-width", "border-style", "border-color"):
            trait = nom.split("-", 1)[1]
            haut, droite, bas, gauche = _quatre(valeur.split())
            for cote, part in zip(COTES, (haut, droite, bas, gauche)):
                resultat["border-%s-%s" % (cote, trait)] = part
        elif nom == "background":
            degrade_trouve = _DEGRADE.search(valeur)
            if degrade_trouve:
                resultat["background-image"] = valeur
            else:
                for morceau in separe(valeur, " "):
                    if couleur(morceau) is not None:
                        resultat["background-color"] = morceau
                        break
        elif nom == "flex":
            # `flex: 1` vaut `1 1 0%`, `flex: auto` vaut `1 1 auto`. C'est la
            # forme sous laquelle la propriete s'ecrit presque toujours.
            parties = valeur.split()
            if valeur.strip() == "auto":
                parties = ["1", "1", "auto"]
            elif valeur.strip() == "none":
                parties = ["0", "0", "auto"]
            resultat["flex-grow"] = parties[0] if parties else "0"
            resultat["flex-shrink"] = parties[1] if len(parties) > 1 else "1"
            resultat["flex-basis"] = parties[2] if len(parties) > 2 else (
                "auto" if len(parties) > 1 and not parties[1][:1].isdigit() else "0%")
        elif nom == "flex-flow":
            parties = valeur.split()
            for morceau in parties:
                if morceau in ("wrap", "nowrap", "wrap-reverse"):
                    resultat["flex-wrap"] = morceau
                else:
                    resultat["flex-direction"] = morceau
        elif nom == "gap":
            parties = valeur.split()
            resultat["row-gap"] = parties[0]
            resultat["column-gap"] = parties[1] if len(parties) > 1 else parties[0]
        elif nom == "place-items":
            parties = valeur.split()
            resultat["align-items"] = parties[0]
            resultat["justify-items"] = parties[1] if len(parties) > 1 else parties[0]
        elif nom == "place-content":
            parties = valeur.split()
            resultat["align-content"] = parties[0]
            resultat["justify-content"] = parties[1] if len(parties) > 1 else parties[0]
        elif nom == "inset":
            haut, droite, bas, gauche = _quatre(valeur.split())
            resultat["top"], resultat["right"] = haut, droite
            resultat["bottom"], resultat["left"] = bas, gauche
        elif nom == "grid-area":
            parties = [p.strip() for p in valeur.split("/")]
            # `grid-area: entete` nomme une zone du gabarit ; `grid-area: 1 / 2`
            # donne des lignes. Confondre les deux rangeait le nom dans
            # `grid-row-start`, ou plus rien ne le reconnaissait.
            if len(parties) == 1 and not _EST_LIGNE.match(parties[0]):
                resultat["grid-area"] = parties[0]
            else:
                for cle, part in zip(("grid-row-start", "grid-column-start",
                                      "grid-row-end", "grid-column-end"), parties):
                    resultat[cle] = part
        elif nom in ("grid-row", "grid-column"):
            parties = [p.strip() for p in valeur.split("/")]
            resultat["%s-start" % nom] = parties[0]
            if len(parties) > 1:
                resultat["%s-end" % nom] = parties[1]
        elif nom == "grid-template":
            parties = [p.strip() for p in valeur.split("/")]
            resultat["grid-template-rows"] = parties[0]
            if len(parties) > 1:
                resultat["grid-template-columns"] = parties[1]
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
/* Les valeurs de depart vont sur la racine seule, et non sur `html, body` :
   les poser aussi sur le corps les y rendait explicites, et une page qui ecrit
   `html { color: … }` voyait sa declaration battue par la notre au lieu de
   descendre par heritage. */
html { display: block; color: #202124; font-size: 16px; line-height: 1.5; }
body { display: block; margin: 8px; background-color: #ffffff; }
div, p, section, article, header, footer, nav, main, aside, figure,
form, fieldset, blockquote, pre, ul, ol, dl, dd, dt, li, table, tr,
h1, h2, h3, h4, h5, h6, hr, address, details, summary { display: block; }
/* `bo-ombre` porte le contenu d'une racine d'ombre, `bo-fragment` celui d'un
   fragment de document. Ni l'un ni l'autre n'existe dans le HTML : ce sont des
   supports que le prelude fabrique, et ils doivent se comporter comme la boite
   transparente qu'ils representent. Sans cette regle, ils seraient en ligne et
   le contenu d'un composant s'afficherait a la suite du texte voisin. */
bo-ombre, bo-fragment, canvas { display: block; }
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
hr { margin: 8px 0; border-top: 1px solid #d0d7de; }
table { margin: 8px 0; }
th { font-weight: bold; }
td, th { padding: 4px 8px; }
small { font-size: 13px; }
button, input, select, textarea { background-color: #f3f4f6; padding: 4px 8px;
                                  border-width: 1px; border-color: #c9ced6; }
mark { background-color: #fff3a3; }
"""
