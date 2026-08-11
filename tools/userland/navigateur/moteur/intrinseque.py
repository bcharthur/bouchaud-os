"""Ce qu'une boite demande, avant qu'on lui donne quoi que ce soit.

## Le probleme

Le moteur ne savait mesurer qu'en posant. Pour savoir ce qu'un article
flexible occupe, il le disposait une premiere fois a l'origine, lisait
l'etendue de son contenu, puis le disposait une seconde fois a sa place
definitive. Meme chose pour un flottant (trois poses), une boite en ligne
(deux), une cellule de tableau (deux).

Le profil l'avait chiffre : 2,16 poses par boite, dont seulement 15 % avec des
arguments identiques. Un cache sur la pose n'aurait donc rien rapporte — ce ne
sont pas des doublons, ce sont deux phases distinctes. La bonne reponse n'est
pas de memoriser la pose, c'est de **ne plus avoir besoin de poser pour
mesurer**.

## Les deux tailles

CSS Sizing definit deux tailles intrinseques, et elles suffisent a tout :

* `min-content` — la plus petite largeur ou le contenu tient encore sans
  deborder. Pour du texte, c'est la largeur du mot le plus long : on peut
  toujours revenir a la ligne entre deux mots, jamais au milieu d'un mot.
* `max-content` — la largeur qu'il faudrait pour que rien ne revienne a la
  ligne. Pour du texte, la largeur de tout le texte d'une traite.

Tout le reste s'en deduit par composition. Un bloc prend le maximum de ce que
ses enfants demandent ; une ligne de `flex` prend leur somme ; une grille prend
la somme de ses colonnes.

## Ce qui est couvert, et ce qui ne l'est pas

Couvert : texte, blocs, elements en ligne, images et autres elements remplaces,
articles `flex` simples, pistes de grille simples, `width: min-content`,
`max-content` et `fit-content()`.

Pas couvert : `subgrid`, les contributions intrinseques d'un element qui
s'etend sur plusieurs pistes, `table-layout: auto` dans toute sa generalite,
les flottants a l'interieur du calcul. Ces cas retombent sur l'ancien chemin —
poser pour mesurer — qui reste correct, seulement plus cher.

**Le meme moteur sert les deux usages.** `width: min-content` ecrit dans une
feuille et la taille de base d'un article `flex` passent par la meme fonction.
Ce n'etait pas negociable : deux implementations d'une meme notion divergent
toujours, et la divergence se manifeste par un affichage faux que rien ne
signale.

## Le cache, et pourquoi il ne peut pas mentir

Les tailles sont memorisees par `(element, generation de mise en page)`. La
generation change a chaque passe : le cache nait et meurt avec elle, et ne peut
donc pas devenir perime. C'est la meme discipline que les autres caches du
moteur, et c'est deliberement plus prudent qu'une invalidation fine — une
taille intrinseque perimee produit une mise en page fausse, et une mise en page
fausse ne se voit pas dans un profil.
"""

from . import css, telemetrie
from .html import Element, Texte


class Tailles:
    """Les deux largeurs intrinseques d'une boite."""

    __slots__ = ("min_contenu", "max_contenu")

    def __init__(self, min_contenu=0.0, max_contenu=0.0):
        self.min_contenu = min_contenu
        # `max-content` ne peut pas etre plus petit que `min-content` : le texte
        # qui tient sur une ligne tient a plus forte raison sur plusieurs.
        self.max_contenu = max(max_contenu, min_contenu)

    def __repr__(self):
        return "Tailles(min=%.1f, max=%.1f)" % (self.min_contenu, self.max_contenu)


# Elements dont la taille ne vient pas de leur contenu mais d'eux-memes.
REMPLACES = frozenset(("img", "video", "canvas", "iframe", "input", "svg",
                       "select", "textarea", "object", "embed"))

# Largeur par defaut d'un element remplace sans dimension connue. La valeur
# vient de CSS : un remplace sans taille intrinseque fait 300x150.
LARGEUR_REMPLACE = 300.0

# Les modes d'affichage qui posent leurs enfants cote a cote.
_EN_RANGEE = frozenset(("flex", "inline-flex"))
_GRILLES = frozenset(("grid", "inline-grid"))


def _mots(texte):
    """Le texte decoupe comme la mise en page le decoupera."""
    return texte.split()


class Mesureur:
    """Ce qu'il faut pour mesurer : l'hote, les regles, et le cache de la passe.

    Un objet plutot que des variables de module : la mise en page peut ainsi
    mesurer deux documents sans qu'ils se marchent dessus, et le cache disparait
    avec l'objet.
    """

    __slots__ = ("bo", "contexte", "style_de", "_cache", "_generation")

    def __init__(self, bo, contexte, style_de):
        self.bo = bo
        self.contexte = contexte
        # `style_de(element, style_parent)` : la cascade, passee par le cache de
        # la passe. Un rappel plutot qu'un import direct, pour ne pas nouer
        # `intrinseque` et `mise_en_page` l'un a l'autre.
        self.style_de = style_de
        self._cache = {}
        self._generation = css.generation()

    def _verifie_generation(self):
        """Vide le cache si la passe a change. Voir la note d'en-tete."""
        actuelle = css.generation()
        if actuelle != self._generation:
            self._generation = actuelle
            self._cache.clear()

    def tailles(self, element, style):
        """Les tailles intrinseques de cet element, memorisees pour la passe."""
        self._verifie_generation()
        cle = id(element)
        connues = self._cache.get(cle)
        if connues is not None:
            telemetrie.compte("intrinseque.evitees")
            return connues
        telemetrie.compte("intrinseque.calculees")
        mesurees = self._calcule(element, style)
        self._cache[cle] = mesurees
        return mesurees

    # -- Le calcul ------------------------------------------------------------

    def _calcule(self, element, style):
        if isinstance(element, Texte):
            return self._texte(element.contenu, style)
        if not isinstance(element, Element):
            return Tailles()

        affichage = style.get("display", "inline")
        if affichage == "none":
            return Tailles()

        # Une largeur declaree en unites absolues clot la question : le contenu
        # ne decide plus rien.
        taille_police = _taille_police(style)
        declaree = css.longueur(style.get("width", "auto"), None, taille_police)
        if declaree is not None and declaree > 0:
            return self._avec_boite(Tailles(declaree, declaree), style, taille_police)

        if element.balise in REMPLACES:
            return self._avec_boite(self._remplace(element, style), style,
                                    taille_police)

        if affichage in _EN_RANGEE:
            interieur = self._rangee(element, style)
        elif affichage in _GRILLES:
            interieur = self._grille(element, style)
        else:
            interieur = self._flux(element, style, affichage)
        return self._avec_boite(interieur, style, taille_police)

    def _texte(self, contenu, style):
        """Le mot le plus long, et le texte entier."""
        taille = _taille_police(style)
        gras = _est_gras(style)
        fixe = _est_fixe(style)
        entier = contenu.strip()
        if not entier:
            return Tailles()
        # `white-space: nowrap` interdit le retour a la ligne : la plus petite
        # largeur possible devient la plus grande.
        insecable = style.get("white-space", "normal") in ("nowrap", "pre")
        maximum = self.bo.largeur_texte(entier, taille, gras, fixe)
        if insecable:
            return Tailles(maximum, maximum)
        minimum = 0.0
        for mot in _mots(entier):
            largeur = self.bo.largeur_texte(mot, taille, gras, fixe)
            if largeur > minimum:
                minimum = largeur
        return Tailles(minimum, maximum)

    def _remplace(self, element, style):
        """Un element remplace apporte sa propre largeur."""
        taille = _taille_police(style)
        for source in ("width",):
            valeur = css.longueur(style.get(source, "auto"), None, taille)
            if valeur is not None and valeur > 0:
                return Tailles(valeur, valeur)
        brut = element.attributs.get("width")
        if brut:
            try:
                valeur = float(brut.rstrip("px").strip())
                if valeur > 0:
                    return Tailles(valeur, valeur)
            except ValueError:
                pass
        # Un champ de saisie sans largeur : la taille de son texte de garde ou
        # de sa valeur, faute de mieux.
        if element.balise in ("input", "select", "textarea"):
            texte = (element.attributs.get("value")
                     or element.attributs.get("placeholder") or "")
            if texte:
                mesuree = self._texte(texte, style)
                return Tailles(mesuree.max_contenu, mesuree.max_contenu)
        return Tailles(LARGEUR_REMPLACE, LARGEUR_REMPLACE)

    def _enfants_mesurables(self, element, style):
        """Les enfants et leur style, dans l'ordre du document."""
        for enfant in element.enfants:
            if isinstance(enfant, Texte):
                if enfant.contenu.strip():
                    yield enfant, style
                continue
            if not isinstance(enfant, Element):
                continue
            sous = self._style_enfant(enfant, style)
            if sous is not None and sous.get("display", "inline") != "none":
                yield enfant, sous

    def _style_enfant(self, enfant, style_parent):
        """Le style calcule de l'enfant.

        Il vient du cache de la passe s'il y est deja, sinon la cascade le
        calcule — et l'y depose. C'est ce qui rend la mesure prealable moins
        chere que la pose qu'elle remplace : le travail se deplace en amont, la
        pose n'aura plus a le refaire.
        """
        fixe = getattr(enfant, "style_engendre", None)
        if fixe is not None:
            return fixe
        connu = self.contexte._styles.get(id(enfant))
        if connu is not None:
            return connu
        try:
            return self.style_de(enfant, style_parent)
        except Exception:
            # Mesurer ne doit jamais empecher d'afficher : un style qu'on ne
            # sait pas calculer rend simplement cet enfant non mesurable, et la
            # pose reprendra son cours.
            return None

    def _flux(self, element, style, affichage):
        """Bloc ou element en ligne.

        Un **bloc** empile : chaque enfant est seul sur sa largeur, donc le
        parent demande le maximum de ce que ses enfants demandent.

        Un **element en ligne** coule : ses enfants se suivent sur la meme
        ligne, donc `max-content` est leur somme. Son `min-content` reste le
        maximum des leurs — on peut revenir a la ligne entre deux enfants.
        """
        en_ligne = affichage in ("inline", "inline-block", "inline-flex")
        minimum = 0.0
        maximum = 0.0
        somme = 0.0
        espace = self.bo.largeur_texte(" ", _taille_police(style),
                                       _est_gras(style), _est_fixe(style))
        premier = True
        for enfant, sous in self._enfants_mesurables(element, style):
            tailles = self.tailles(enfant, sous)
            if tailles.min_contenu > minimum:
                minimum = tailles.min_contenu
            if tailles.max_contenu > maximum:
                maximum = tailles.max_contenu
            sous_affichage = (sous.get("display", "inline")
                              if isinstance(enfant, Element) else "inline")
            if sous_affichage in ("inline", "inline-block", "inline-flex"):
                somme += tailles.max_contenu + (0.0 if premier else espace)
            else:
                # Un bloc au milieu d'un flux en ligne casse la ligne : la
                # somme repart de lui.
                somme = max(somme, tailles.max_contenu)
            premier = False
        if en_ligne:
            return Tailles(minimum, max(maximum, somme))
        # Un bloc dont tous les enfants sont en ligne se comporte comme eux
        # pour `max-content` : le texte se met bout a bout avant de revenir a
        # la ligne. C'est ce qui fait qu'un paragraphe demande la largeur de son
        # texte entier, et non celle de sa plus longue portion.
        return Tailles(minimum, max(maximum, somme))

    def _rangee(self, element, style):
        """Un conteneur `flex`.

        En rangee sans retour a la ligne, `max-content` est la somme des
        `max-content` des articles plus les espaces ; `min-content` est la somme
        des `min-content`, parce que rien ne peut passer a la ligne. Avec
        `flex-wrap`, `min-content` redevient le maximum : la ligne peut se
        couper entre deux articles.
        """
        direction = style.get("flex-direction", "row")
        colonne = direction.startswith("column")
        enroule = style.get("flex-wrap", "nowrap") != "nowrap"
        espace = _espace(style, _taille_police(style))

        minimums = []
        maximums = []
        for enfant, sous in self._enfants_mesurables(element, style):
            tailles = self.tailles(enfant, sous)
            minimums.append(tailles.min_contenu)
            maximums.append(tailles.max_contenu)
        if not maximums:
            return Tailles()

        if colonne:
            # En colonne, les articles sont empiles : la largeur est celle du
            # plus large.
            return Tailles(max(minimums), max(maximums))

        ecarts = espace * (len(maximums) - 1)
        maximum = sum(maximums) + ecarts
        minimum = max(minimums) if enroule else sum(minimums) + ecarts
        return Tailles(minimum, maximum)

    def _grille(self, element, style):
        """Une grille : la somme de ses colonnes.

        On ne resout ici que le cas courant — des articles poses un par cellule,
        sans etendue. Les pistes fixes apportent leur largeur, les pistes `auto`
        et `fr` celle de leur contenu.
        """
        colonnes = style.get("grid-template-columns", "").strip()
        espace = _espace(style, _taille_police(style))
        articles = [self.tailles(enfant, sous)
                    for enfant, sous in self._enfants_mesurables(element, style)]
        if not articles:
            return Tailles()

        nombre = _nombre_de_colonnes(colonnes) or len(articles)
        nombre = max(1, min(nombre, len(articles)))
        # Chaque colonne prend le maximum de ce que demandent les articles qui
        # y tombent.
        min_colonnes = [0.0] * nombre
        max_colonnes = [0.0] * nombre
        for rang, tailles in enumerate(articles):
            colonne = rang % nombre
            min_colonnes[colonne] = max(min_colonnes[colonne], tailles.min_contenu)
            max_colonnes[colonne] = max(max_colonnes[colonne], tailles.max_contenu)
        ecarts = espace * (nombre - 1)
        return Tailles(sum(min_colonnes) + ecarts, sum(max_colonnes) + ecarts)

    def _avec_boite(self, interieur, style, taille_police):
        """Ajoute ce que la boite elle-meme occupe autour de son contenu."""
        autour = 0.0
        for propriete in ("padding-left", "padding-right",
                          "border-left-width", "border-right-width"):
            valeur = css.longueur(style.get(propriete, "0"), None, taille_police)
            if valeur:
                autour += valeur
        if autour == 0.0:
            return interieur
        return Tailles(interieur.min_contenu + autour, interieur.max_contenu + autour)


# --- Aides -------------------------------------------------------------------


def _taille_police(style):
    valeur = css.longueur(style.get("font-size", "16px"), 16.0, 16.0)
    return valeur if valeur and valeur > 0 else 16.0


def _est_gras(style):
    poids = style.get("font-weight", "normal")
    if poids in ("bold", "bolder"):
        return True
    try:
        return int(poids) >= 600
    except ValueError:
        return False


def _est_fixe(style):
    return "monospace" in style.get("font-family", "").lower()


def _espace(style, taille_police):
    """L'ecart entre deux articles (`gap` / `column-gap`)."""
    for propriete in ("column-gap", "gap"):
        brut = style.get(propriete)
        if not brut:
            continue
        premier = brut.split()[0]
        valeur = css.longueur(premier, None, taille_police)
        if valeur is not None:
            return valeur
    return 0.0


def _nombre_de_colonnes(gabarit):
    """Combien de pistes `grid-template-columns` declare-t-il ?"""
    if not gabarit:
        return 0
    if gabarit.startswith("repeat(") and gabarit.endswith(")"):
        interieur = gabarit[7:-1]
        virgule = interieur.find(",")
        if virgule > 0:
            compte = interieur[:virgule].strip()
            if compte.isdigit():
                # `repeat(3, 1fr)` : trois pistes.
                return int(compte)
        return 0
    return len(css.separe_liste(gabarit)) if gabarit else 0


# --- Le mot-cle CSS ----------------------------------------------------------
#
# `width: min-content` et ses cousins passent par le meme calcul que la taille
# de base d'un article flexible. C'est l'exigence centrale du module : une seule
# implementation de la notion, deux facons de la demander.

MOTS_CLES = frozenset(("min-content", "max-content", "fit-content", "auto"))


def largeur_mot_cle(valeur, tailles, disponible):
    """Resout `width: min-content | max-content | fit-content(...)`.

    Rend `None` si la valeur n'est pas un de ces mots-cles — l'appelant reprend
    alors son chemin ordinaire.
    """
    if not valeur or tailles is None:
        return None
    valeur = valeur.strip()
    if valeur == "min-content":
        return tailles.min_contenu
    if valeur == "max-content":
        return tailles.max_contenu
    if valeur == "fit-content":
        # `fit-content` sans argument : la formule de la norme, bornee des deux
        # cotes par ce que le contenu peut accepter.
        if disponible is None:
            return tailles.max_contenu
        return max(tailles.min_contenu, min(tailles.max_contenu, disponible))
    if valeur.startswith("fit-content(") and valeur.endswith(")"):
        argument = valeur[12:-1].strip()
        limite = css.longueur(argument, disponible, 16.0)
        if limite is None:
            limite = disponible
        if limite is None:
            return tailles.max_contenu
        return max(tailles.min_contenu, min(tailles.max_contenu, limite))
    return None
