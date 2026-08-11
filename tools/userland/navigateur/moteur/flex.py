"""Disposition flexible : `display: flex`.

C'est le mode qui porte le web depuis une dizaine d'annees. Sans lui, une page
d'aujourd'hui ne s'affiche pas « approximativement » : elle s'effondre
entierement, tout empile verticalement dans l'ordre du source, parce que ses
barres, ses cartes et ses grilles reposent toutes dessus.

## L'algorithme, en quatre temps

1. **Taille de base.** Chaque enfant recoit sa taille souhaitee sur l'axe
   principal — `flex-basis`, a defaut sa largeur ou sa hauteur, a defaut ce que
   son contenu demande.
2. **Repartition.** On compare la somme des bases a la place disponible. S'il
   reste de la place, `flex-grow` la distribue ; s'il en manque, `flex-shrink`
   la reprend. Les deux au prorata de leur coefficient.
3. **Lignes.** Avec `flex-wrap`, ce qui deborde passe a la ligne suivante, et
   chaque ligne rejoue la repartition pour elle seule.
4. **Alignement.** `justify-content` place les enfants le long de l'axe
   principal, `align-items` en travers.

## Deux passes, et pourquoi

Un enfant est dispose deux fois : une premiere a l'origine pour savoir ce qu'il
occupe, une seconde a sa place definitive. Le deplacer apres coup ne suffirait
pas — sa propre descendance a deja ete posee en coordonnees absolues, et seule
une nouvelle disposition la suit. C'est le prix d'un arbre de boites qui porte
des positions plutot que des decalages.

## Ce qui est simplifie

Pas de `align-content` entre lignes multiples autre que le
simple empilement, pas de tailles intrinseques calculees par un vrai
`min-content` — on prend la largeur du contenu telle que la mise en page en
ligne la produit. Ces ecarts se voient sur des dispositions inhabituelles ; ils
ne se voient pas sur une barre de navigation ou une grille de cartes, qui sont
ce que le web emploie.
"""

from . import css


class Article:
    """Un enfant du conteneur, avec ce que la repartition doit savoir de lui."""

    __slots__ = ("boite", "base", "grandir", "retrecir", "taille", "travers",
                 "marge_avant", "marge_apres", "marge_travers_avant",
                 "marge_travers_apres", "alignement", "rang")

    def __init__(self, boite):
        self.boite = boite
        self.base = 0.0
        self.grandir = 0.0
        self.retrecir = 1.0
        self.taille = 0.0
        self.travers = 0.0
        self.marge_avant = 0.0
        self.marge_apres = 0.0
        self.marge_travers_avant = 0.0
        self.marge_travers_apres = 0.0
        self.alignement = "stretch"


def est_flexible(style):
    return style.get("display", "") in ("flex", "inline-flex")


def direction(style):
    """`(horizontal, inverse)` de l'axe principal."""
    valeur = style.get("flex-direction", "row")
    return (valeur.startswith("row"), valeur.endswith("reverse"))


def dispose(boite, x, y, largeur_disponible, contexte, pose_enfant):
    """Dispose les enfants d'un conteneur flexible. Rend la hauteur consommee.

    `pose_enfant(sous_boite, largeur, x, y, forcee)` dispose un enfant et rend
    `(largeur, hauteur)` qu'il occupe. C'est le point ou la disposition flexible
    rend la main a la disposition ordinaire : un enfant de conteneur flexible
    reste une boite de bloc a l'interieur.
    """
    style = boite.style
    horizontal, inverse = direction(style)
    taille_police = css.longueur(style.get("font-size", "16px"), 16.0, 16.0) or 16.0

    espace_principal = _longueur(style, "column-gap" if horizontal else "row-gap",
                                 largeur_disponible, taille_police)
    espace_travers = _longueur(style, "row-gap" if horizontal else "column-gap",
                               largeur_disponible, taille_police)

    # En colonne, l'axe principal est la hauteur — que le conteneur ne connait
    # que si elle est declaree. Sans elle, il n'y a pas de place a distribuer :
    # `flex-grow` n'a rien a partager, et c'est le comportement correct.
    if horizontal:
        principal = largeur_disponible
    else:
        principal = css.longueur(style.get("height", "auto"), 0.0, taille_police)

    articles = _prepare(boite, largeur_disponible, taille_police,
                        horizontal, pose_enfant,
                        getattr(contexte, "mesureur", None))
    if not articles:
        return 0.0

    lignes = _decoupe_en_lignes(articles, style, principal,
                                espace_principal, horizontal)

    curseur_travers = 0.0
    etendue_principale = 0.0
    for ligne in lignes:
        _repartit(ligne, principal, espace_principal)
        taille_travers, etendue = _pose(
            ligne, x, y, curseur_travers, principal, largeur_disponible,
            espace_principal, horizontal, inverse, style, pose_enfant)
        curseur_travers += taille_travers + espace_travers
        etendue_principale = max(etendue_principale, etendue)

    if lignes:
        curseur_travers -= espace_travers

    # La hauteur consommee est le travers pour une rangee, le principal pour
    # une colonne : confondre les deux donnait des conteneurs en colonne hauts
    # comme leur enfant le plus large.
    return max(0.0, curseur_travers if horizontal else etendue_principale)


# --- Preparation --------------------------------------------------------------

def _prepare(boite, largeur_disponible, taille_police, horizontal, pose_enfant,
             mesureur=None):
    articles = []
    for sous_boite in boite.enfants:
        style = sous_boite.style
        article = Article(sous_boite)
        article.grandir = _nombre(style.get("flex-grow"), 0.0)
        article.retrecir = _nombre(style.get("flex-shrink"), 1.0)
        article.alignement = style.get("align-self") or \
            boite.style.get("align-items", "stretch")

        taille_enfant = css.longueur(style.get("font-size", "16px"),
                                     taille_police, taille_police) or taille_police
        article.marge_avant = _longueur(
            style, "margin-left" if horizontal else "margin-top",
            largeur_disponible, taille_enfant)
        article.marge_apres = _longueur(
            style, "margin-right" if horizontal else "margin-bottom",
            largeur_disponible, taille_enfant)
        article.marge_travers_avant = _longueur(
            style, "margin-top" if horizontal else "margin-left",
            largeur_disponible, taille_enfant)
        article.marge_travers_apres = _longueur(
            style, "margin-bottom" if horizontal else "margin-right",
            largeur_disponible, taille_enfant)

        article.base = _base(article, largeur_disponible, taille_enfant,
                             horizontal, pose_enfant, mesureur)
        article.rang = _entier(style.get("order"), 0)
        articles.append(article)

    # `order` reordonne l'affichage sans toucher au document. Le tri est stable :
    # a rang egal, l'ordre du source est conserve, comme l'exige la norme.
    if any(a.rang for a in articles):
        articles.sort(key=lambda a: a.rang)
    return articles


def _entier(valeur, defaut):
    try:
        return int(str(valeur).strip())
    except (TypeError, ValueError):
        return defaut


def _base(article, largeur_disponible, taille_police, horizontal, pose_enfant,
          mesureur=None):
    """Taille souhaitee sur l'axe principal, avant repartition.

    ## Ce que cette fonction faisait, et ce qu'elle fait

    Elle **posait l'enfant a l'origine** pour lire ce qu'il occupait, puis le
    laissait etre repose ailleurs a sa place definitive. Deux dispositions
    completes de tout un sous-arbre pour obtenir un nombre.

    Elle demande maintenant ce nombre au calcul intrinseque, qui ne pose rien :
    il descend l'arbre des elements en composant des largeurs de texte. Le
    chemin par la pose reste en secours pour ce que le calcul ne couvre pas
    encore — sa reponse est correcte, seulement plus chere.
    """
    style = article.boite.style
    declaree = style.get("flex-basis", "auto")
    if declaree not in ("auto", "content", ""):
        mesure = css.longueur(declaree, largeur_disponible, taille_police)
        if mesure is not None:
            return max(0.0, mesure)

    propre = style.get("width" if horizontal else "height", "auto")
    mesure = css.longueur(propre, largeur_disponible, taille_police)
    if mesure is not None:
        return max(0.0, mesure)

    # Ni base ni taille declaree : on demande au contenu ce qu'il occupe.
    #
    # `max-content` est exactement la taille de base que la norme demande pour
    # `flex-basis: content` : ce qu'il faudrait pour que rien ne revienne a la
    # ligne. La repartition la reduira ensuite si la place manque, ce qui est
    # son role.
    if horizontal and mesureur is not None:
        element = article.boite.element
        if element is not None:
            tailles = mesureur.tailles(element, style)
            return max(0.0, min(largeur_disponible, tailles.max_contenu))

    # Secours : sur l'axe vertical, et pour ce que la mesure ne sait pas encore
    # faire, on retombe sur la pose. Sur l'axe horizontal, ce que rend la pose
    # est la largeur de la **boite**, et une boite de bloc prend tout ce qu'on
    # lui donne : trois articles reclamaient donc chacun la largeur entiere, et
    # se la partageaient en trois parts egales. C'est l'etendue du contenu qu'il
    # faut, pas celle du contenant.
    largeur, hauteur = pose_enfant(article.boite, largeur_disponible, 0.0, 0.0, None)
    if not horizontal:
        return hauteur
    from .mise_en_page import etendue_contenu
    return max(0.0, min(largeur, etendue_contenu(article.boite)))


# --- Lignes et repartition ----------------------------------------------------

def _decoupe_en_lignes(articles, style, disponible, espace, horizontal):
    if style.get("flex-wrap", "nowrap") == "nowrap" or disponible is None:
        return [articles]

    lignes = []
    courante = []
    occupe = 0.0
    for article in articles:
        besoin = article.base + article.marge_avant + article.marge_apres
        supplement = espace if courante else 0.0
        if courante and occupe + supplement + besoin > disponible:
            lignes.append(courante)
            courante = [article]
            occupe = besoin
        else:
            courante.append(article)
            occupe += supplement + besoin
    if courante:
        lignes.append(courante)
    return lignes


def _repartit(ligne, disponible, espace):
    """Distribue la place restante (ou reprend celle qui manque)."""
    for article in ligne:
        article.taille = article.base

    if disponible is None:
        return

    occupe = sum(a.base + a.marge_avant + a.marge_apres for a in ligne)
    occupe += espace * max(0, len(ligne) - 1)
    reste = disponible - occupe

    if abs(reste) < 0.01:
        return

    if reste > 0:
        total = sum(a.grandir for a in ligne)
        if total <= 0:
            return
        for article in ligne:
            article.taille = article.base + reste * article.grandir / total
        return

    # Manque de place : on retrecit au prorata du coefficient **pondere par la
    # base**, comme le veut la norme — un article deux fois plus large cede deux
    # fois plus, a coefficient egal.
    total = sum(a.retrecir * a.base for a in ligne)
    if total <= 0:
        return
    for article in ligne:
        part = (article.retrecir * article.base) / total
        article.taille = max(0.0, article.base + reste * part)


# --- Pose ---------------------------------------------------------------------

def _pose(ligne, x, y, decalage_travers, principal, largeur_disponible,
          espace, horizontal, inverse, style, pose_enfant):
    """Place les articles d'une ligne.

    Rend `(taille de la ligne en travers, etendue occupee sur l'axe principal)`.
    """
    base_x = x + (0.0 if horizontal else decalage_travers)
    base_y = y + (decalage_travers if horizontal else 0.0)

    # Premiere passe : chaque article est dispose a sa taille principale finale
    # pour qu'il annonce ce qu'il occupe en travers.
    for article in ligne:
        if horizontal:
            _, hauteur = pose_enfant(article.boite, article.taille,
                                     0.0, 0.0, article.taille)
            article.travers = hauteur
        else:
            largeur, _ = pose_enfant(article.boite, largeur_disponible,
                                     0.0, 0.0, None)
            article.travers = largeur

    taille_travers = max(
        (a.travers + a.marge_travers_avant + a.marge_travers_apres for a in ligne),
        default=0.0)

    occupe = sum(a.taille + a.marge_avant + a.marge_apres for a in ligne)
    occupe += espace * max(0, len(ligne) - 1)
    libre = max(0.0, (principal - occupe) if principal is not None else 0.0)

    justification = style.get("justify-content", "flex-start")
    debut, entre = _distribution(justification, libre, len(ligne))

    ordonnee = list(reversed(ligne)) if inverse else ligne
    curseur = debut
    for article in ordonnee:
        curseur += article.marge_avant
        travers = _place_en_travers(article, taille_travers)

        # Seconde passe : la boite et toute sa descendance sont posees a leur
        # place definitive. Les marges sont retranchees parce que la mise en
        # page ordinaire les rajoute a l'origine qu'on lui donne.
        if horizontal:
            pose_enfant(article.boite, article.taille,
                        base_x + curseur - article.marge_avant,
                        base_y + travers - article.marge_travers_avant,
                        article.taille)
            article.boite.hauteur = max(article.boite.hauteur, article.travers)
        else:
            pose_enfant(article.boite, article.travers,
                        base_x + travers - article.marge_travers_avant,
                        base_y + curseur - article.marge_avant,
                        article.travers)
            article.boite.hauteur = max(article.boite.hauteur, article.taille)

        curseur += article.taille + article.marge_apres + espace + entre

    etendue = curseur - espace - entre if ligne else 0.0
    return (taille_travers, max(0.0, etendue))


def _distribution(justification, libre, nombre):
    """`(decalage initial, espace ajoute entre articles)`."""
    if nombre <= 0:
        return (0.0, 0.0)
    if justification in ("flex-end", "end", "right"):
        return (libre, 0.0)
    if justification == "center":
        return (libre / 2.0, 0.0)
    if justification == "space-between":
        return (0.0, libre / max(1, nombre - 1)) if nombre > 1 else (0.0, 0.0)
    if justification == "space-around":
        part = libre / nombre if nombre else 0.0
        return (part / 2.0, part)
    if justification == "space-evenly":
        part = libre / (nombre + 1)
        return (part, part)
    return (0.0, 0.0)


def _place_en_travers(article, taille_ligne):
    """Decalage de l'article sur l'axe secondaire."""
    place = taille_ligne - article.marge_travers_avant - article.marge_travers_apres
    alignement = article.alignement
    if alignement in ("flex-end", "end"):
        return article.marge_travers_avant + max(0.0, place - article.travers)
    if alignement == "center":
        return article.marge_travers_avant + max(0.0, (place - article.travers) / 2.0)
    if alignement == "stretch":
        # L'etirement n'a de sens que si l'article n'a pas de taille propre sur
        # cet axe ; sinon on le laisse ou il est.
        article.travers = max(article.travers, place)
    return article.marge_travers_avant


# --- Petits outils ------------------------------------------------------------

def _longueur(style, propriete, reference, taille_police):
    mesure = css.longueur(style.get(propriete, "0"), reference, taille_police)
    return mesure if mesure is not None else 0.0


def _nombre(valeur, defaut):
    try:
        return float(str(valeur).strip())
    except (TypeError, ValueError):
        return defaut
