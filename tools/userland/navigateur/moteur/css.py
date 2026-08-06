"""Feuilles de style : analyse, specificite, cascade et heritage.

Le sous-ensemble couvert est celui qui change vraiment l'apparence d'une page :
les selecteurs simples, descendants et enfants directs, le modele de boite, les
couleurs, la typographie, les proprietes de disposition flexible et en grille,
les `@media` — evaluees contre la taille de fenetre reelle — et les
pseudo-elements `::before`/`::after`.

Les proprietes personnalisees (`--fond`) et `var()` en font partie : c'est sur
elles que repose la mise en forme de tout site recent, et les ignorer n'affichait
pas une page approximative mais une page sans couleurs ni espacements.

Ce qui manque (animations, transformations) est ignore proprement plutot que mal
interprete : une declaration inconnue est laissee de cote, elle ne fait pas
echouer la regle qui la contient.
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

class Simple:
    """Un maillon de selecteur : balise, classes, identifiant, pseudo-element."""

    __slots__ = ("balise", "classes", "identifiant", "pseudo")

    def __init__(self, texte):
        self.balise = None
        self.classes = []
        self.identifiant = None
        # `::before` et `::after` designent une boite qui n'existe pas dans le
        # document : il faut donc les retenir, la mise en page les fabriquera.
        # C'est ainsi que la moitie des sites posent leurs icones et leurs
        # separateurs — les effacer, comme on le faisait, revenait a jeter cette
        # moitie-la.
        self.pseudo = None
        trouve = re.search(r"::?(before|after)\b", texte)
        if trouve:
            self.pseudo = trouve.group(1)
        # `:root` designe l'element racine, donc `<html>` en HTML. Le laisser
        # tomber avec les autres pseudo-classes en faisait un selecteur
        # universel : les variables qu'une page y pose etaient alors reposees
        # sur **chaque** element, ou elles ecrasaient celles qu'un theme local
        # avait redefinies plus haut dans l'arbre.
        texte = re.sub(r":root\b", "html", texte)
        # Les pseudo-classes, elles, restent ignorees : `a:hover` se comporte
        # comme `a`. Les prendre au pied de la lettre demanderait un etat
        # d'interaction que le moteur ne tient pas ; les rejeter perdrait la
        # regle entiere.
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

    def cle(self):
        """Le trait le plus discriminant du maillon, pour l'indexation.

        L'identifiant d'abord, une classe ensuite, la balise a defaut. `None`
        pour un maillon universel, qu'aucun index ne peut restreindre.
        """
        if self.identifiant:
            return ("#", self.identifiant)
        if self.classes:
            return (".", self.classes[0])
        if self.balise:
            return ("b", self.balise)
        return None


class Selecteur:
    """Une suite de maillons, relies par la descendance ou par `>`."""

    __slots__ = ("maillons", "combinateurs", "specificite", "pseudo")

    def __init__(self, texte):
        # `+` et `~` designent des freres ; la mise en page ne connait de chaque
        # element que sa lignee, pas sa fratrie, donc ils sont ramenes a la
        # descendance — moins precis, mais bien plus proche du resultat attendu
        # que de jeter la regle entiere.
        texte = re.sub(r"\s*[+~]\s*", " ", texte)
        # `>` en revanche est tenu : `.menu > li` ne doit pas atteindre les `li`
        # d'un sous-menu, et le confondre avec la descendance donnait a des
        # elements imbriques la mise en forme de leur parent.
        texte = re.sub(r"\s*>\s*", " > ", texte)

        self.maillons = []
        # `combinateurs[i]` relie `maillons[i-1]` a `maillons[i]`. La premiere
        # case ne sert pas ; elle existe pour que les indices coincident.
        self.combinateurs = []
        enfant_direct = False
        for morceau in texte.split():
            if morceau == ">":
                enfant_direct = True
                continue
            self.maillons.append(Simple(morceau))
            self.combinateurs.append(">" if enfant_direct else " ")
            enfant_direct = False

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
        """`chemin` est la liste des ancetres, du plus lointain a l'element."""
        if not self.maillons:
            return False
        index = len(self.maillons) - 1
        if not self.maillons[index].correspond(chemin[-1]):
            return False
        index -= 1
        position = len(chemin) - 2
        while index >= 0:
            if position < 0:
                return False
            if self.combinateurs[index + 1] == ">":
                # Enfant direct : ce maillon-ci doit correspondre au parent
                # immediat, sans quoi la regle ne s'applique pas du tout.
                if not self.maillons[index].correspond(chemin[position]):
                    return False
                index -= 1
            elif self.maillons[index].correspond(chemin[position]):
                index -= 1
            position -= 1
        return True


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
    "content", "border-radius", "opacity",
)


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
        elif nom == "border-radius":
            resultat["border-radius"] = valeur.split()[0] if valeur.split() else "0"
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
hr { margin: 8px 0; border-width: 1px; border-color: #d0d7de; }
table { margin: 8px 0; }
th { font-weight: bold; }
td, th { padding: 4px 8px; }
small { font-size: 13px; }
button, input, select, textarea { background-color: #f3f4f6; padding: 4px 8px;
                                  border-width: 1px; border-color: #c9ced6; }
mark { background-color: #fff3a3; }
"""
