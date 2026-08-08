"""Mise en page : de l'arbre style aux boites positionnees.

Deux modes de disposition, ceux qui portent 90 % du web textuel :

- **bloc** : les enfants s'empilent verticalement, chacun sur toute la largeur
  disponible, avec marges, bordures et remplissage ;
- **en ligne** : les enfants se suivent horizontalement et passent a la ligne
  quand la largeur est atteinte.

La largeur d'un mot n'est pas calculable en Python : elle depend de la fonte
reelle. C'est la seule chose que le moteur demande a l'hote Qt, par
`bo.largeur_texte`. Tout le reste — cascade, boites, retours a la ligne — se
fait ici.
"""

import bo

from . import css, flex, grille, images
from .html import Element, Texte


class Boite:
    """Une boite positionnee, prete a peindre."""

    __slots__ = ("element", "style", "x", "y", "largeur", "hauteur",
                 "enfants", "lignes", "lien", "puce", "images",
                 "hors_flux", "rogne", "toile")

    def __init__(self, element, style):
        self.element = element
        self.style = style
        self.x = 0.0
        self.y = 0.0
        self.largeur = 0.0
        self.hauteur = 0.0
        self.enfants = []
        # Fragments de texte : (x, y, texte, hauteur_ligne, style)
        self.lignes = []
        self.lien = None
        self.puce = None
        # Images posees dans le flux en ligne : (x, y, largeur, hauteur, id, lien)
        self.images = []
        # Boites sorties du flux (`position: absolute`/`fixed`), placees apres
        # coup par rapport a leur bloc contenant.
        self.hors_flux = []
        # `overflow: hidden`/`auto`/`scroll` : la peinture y rognera.
        self.rogne = False
        # Operations dessinees par un contexte 2D, si c'est un `<canvas>`.
        self.toile = None


class Fragment:
    """Un morceau de texte pose a un endroit precis."""

    __slots__ = ("x", "y", "texte", "hauteur", "style", "lien")

    def __init__(self, x, y, texte, hauteur, style, lien):
        self.x = x
        self.y = y
        self.texte = texte
        self.hauteur = hauteur
        self.style = style
        self.lien = lien


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


def _longueur(style, propriete, reference, taille):
    valeur = css.longueur(style.get(propriete, "0"), reference, taille)
    return valeur if valeur is not None else 0.0


class Contexte:
    """Ce qui ne change pas d'une boite a l'autre pendant une mise en page."""

    def __init__(self, regles, largeur_page, url="", image_video=None,
                 toile=None):
        self.regles = regles
        self.largeur_page = largeur_page
        # Adresse de la page : les `src` relatifs des images s'y resolvent.
        self.url = url
        # Rappel qui rend l'image courante d'un `<video>`, ou `None`.
        self.image_video = image_video
        # Rappel qui rend ce qu'un `<canvas>` a dessine, ou `None`.
        self.toile = toile


def construit(racine, regles, largeur_page, url="", image_video=None, toile=None):
    """Construit l'arbre de boites et rend (racine, hauteur totale)."""
    contexte = Contexte(regles, largeur_page, url, image_video, toile)
    style_initial = {
        "color": "#202124", "font-size": "16px", "line-height": "1.5",
        "display": "block",
    }
    # Le style de l'element racine se calcule avant celui du corps, et il
    # compte : c'est sur `<html>` — donc sur `:root` — qu'une page moderne pose
    # ses variables de theme, et c'est la aussi que `font-size` fixe la
    # reference de tous les `rem`. Partir directement du corps, comme on le
    # faisait, jetait les deux.
    corps = racine.trouve("body") or racine
    # L'element racine est le nœud racine lui-meme quand l'analyse a produit un
    # `<html>` implicite — ce qui est le cas ordinaire ; `trouve` ne le rendrait
    # pas, puisqu'il ne cherche que dans la descendance.
    element_racine = racine if getattr(racine, "balise", "") == "html" \
        else racine.trouve("html")
    css.pose_taille_racine(16.0)
    if element_racine is not None and element_racine is not corps:
        style_initial = css.applique(contexte.regles, element_racine,
                                     [element_racine], style_initial)
        # `rem` se rapporte a cette taille-la, pas a 16 px par principe. Le
        # pourcentage s'y compte depuis la valeur par defaut du navigateur.
        css.pose_taille_racine(
            css.longueur(style_initial.get("font-size", "16px"), 16.0, 16.0))
    # Le chemin complet, et non le seul corps : `html body` doit correspondre.
    boite = _boite_pour(corps, style_initial, _chemin(corps), contexte)
    if boite is None:
        boite = Boite(corps, style_initial)
    _dispose_bloc(boite, 0.0, 0.0, largeur_page, contexte)
    return boite, boite.hauteur


def _style_de(element, style_parent, chemin, contexte):
    """Style calcule d'un element, ou celui deja fixe d'une boite engendree.

    Un `::before` n'est pas dans l'arbre : la cascade ne peut pas le retrouver a
    partir de son chemin, on lui attache donc son style au moment ou on le
    fabrique et on le reprend tel quel ici.
    """
    fixe = getattr(element, "style_engendre", None)
    if fixe is not None:
        return fixe
    style = css.applique(contexte.regles, element, chemin, style_parent)

    # Un element en ligne qui porte un bloc devient un bloc. La norme, elle,
    # decoupe l'element en ligne autour de son bloc et engendre des boites
    # anonymes ; le resultat visible est le meme dans presque tous les cas, et
    # ce qu'on faisait avant ne l'etait pas du tout : le bloc et tout son
    # contenu disparaissaient. `<span><p>…</p></span>` n'affichait rien, et une
    # balise inconnue — donc tout composant web — non plus.
    if style.get("display", "inline") in ("inline", "inline-block") \
            and _porte_un_bloc(element):
        style = dict(style)
        style["display"] = "block"
    return style


# Les balises qui font une boite de bloc dans la feuille de l'agent utilisateur.
# Une table suffit ici : il ne s'agit pas de decider de la mise en page, mais de
# reperer qu'un enfant direct en reclame une.
_BLOCS = frozenset((
    "address", "article", "aside", "blockquote", "canvas", "dd", "div", "dl",
    "dt", "fieldset", "figcaption", "figure", "footer", "form", "h1", "h2",
    "h3", "h4", "h5", "h6", "header", "hr", "li", "main", "nav", "ol", "p",
    "pre", "section", "table", "tbody", "td", "tfoot", "th", "thead", "tr",
    "ul", "video", css.SUPPORT_OMBRE,
))


def _porte_un_bloc(element):
    # Une fente n'a pas d'enfants a elle : ce qu'elle affiche lui est confie.
    # Sans ce detour, un `<slot>` restait en ligne et le contenu clair qu'il
    # devait poser ne recevait jamais de boite.
    enfants = _distribues(element) if element.balise == "slot" else element.enfants
    for enfant in enfants:
        if not isinstance(enfant, Element):
            continue
        if enfant.balise in _BLOCS:
            return True
        # Un style en ligne peut faire un bloc de n'importe quoi ; les
        # composants se declarent souvent ainsi.
        declare = enfant.attributs.get("style", "")
        if "display" in declare and "block" in declare:
            return True
    return False


def _boite_pour(element, style_parent, chemin, contexte):
    style = _style_de(element, style_parent, chemin, contexte)
    if style.get("display", "inline") == "none":
        return None
    boite = Boite(element, style)
    if element.balise == "a" and element.attributs.get("href"):
        boite.lien = element.attributs["href"]
    return boite


def _engendre(boite, contexte, pseudo):
    """Fabrique le nœud d'un `::before`/`::after`, ou `None`.

    La boite engendree est un element de synthese portant son texte et son
    style : le reste de la mise en page ne fait alors aucune difference entre
    elle et un element du document, ce qui evite un second chemin de code.
    """
    element = boite.element
    if not isinstance(element, Element):
        return None
    style = css.contenu_engendre(contexte.regles, element, _chemin(element),
                                 boite.style, pseudo)
    if style is None:
        return None

    valeur = style.get("content", "")
    texte = css.texte_du_contenu(valeur)
    if not texte and valeur.strip().startswith("attr("):
        nom = valeur.strip()[5:].rstrip(")").strip().strip("\"'")
        texte = element.attributs.get(nom, "")

    engendree = Element("::" + pseudo)
    engendree.parent = element
    if texte:
        engendree.ajoute(Texte(texte))
    engendree.style_engendre = style
    return engendree


def _enfants(boite, contexte):
    """Les enfants a disposer : ceux du document, encadres par les engendres.

    Deux detours, tous deux dus au DOM d'ombre. Un element qui porte une racine
    d'ombre n'affiche **que** cette racine : ses enfants clairs ne s'affichent
    que la ou un `<slot>` les reclame. Et un `<slot>`, justement, affiche les
    enfants clairs de son hote plutot que les siens — les siens ne sont que le
    repli montre quand personne ne lui en confie.
    """
    element = boite.element
    if not isinstance(element, Element):
        return []

    if element.balise == "slot":
        liste = _distribues(element)
    else:
        liste = list(element.enfants)
        support = _racine_ombre(element)
        if support is not None:
            liste = [support]

    avant = _engendre(boite, contexte, "before")
    if avant is not None:
        liste.insert(0, avant)
    apres = _engendre(boite, contexte, "after")
    if apres is not None:
        liste.append(apres)
    return liste


def _racine_ombre(element):
    """Le support d'ombre porte par cet element, s'il en a un."""
    for enfant in element.enfants:
        if isinstance(enfant, Element) and enfant.balise == css.SUPPORT_OMBRE:
            return enfant
    return None


def _distribues(fente):
    """Les nœuds clairs que ce `<slot>` affiche, ou son repli s'il n'y en a pas.

    Le nom decide : `<slot name="titre">` prend les enfants portant
    `slot="titre"`, la fente sans nom prend tous les autres.
    """
    support = css.ombre_de(fente)
    hote = support.parent if support is not None else None
    if hote is None:
        return list(fente.enfants)

    nom = fente.attributs.get("name", "")
    retenus = []
    for enfant in hote.enfants:
        if isinstance(enfant, Element):
            if enfant.balise == css.SUPPORT_OMBRE:
                continue
            if enfant.attributs.get("slot", "") != nom:
                continue
        elif nom:
            # Un nœud de texte n'a pas d'attribut : il ne va qu'a la fente
            # sans nom.
            continue
        retenus.append(enfant)
    return retenus or list(fente.enfants)


def _boites_enfants(boite, contexte):
    """Construit les boites des enfants d'un conteneur flexible ou en grille.

    La disposition en bloc les fabrique au fil de l'eau, en meme temps qu'elle
    les pose. Flex et grille, elles, doivent toutes les connaitre avant de
    decider ou chacune va : d'ou cette passe separee.
    """
    for enfant in _enfants(boite, contexte):
        if isinstance(enfant, Texte):
            if not enfant.contenu.strip():
                continue
            # Du texte nu dans un conteneur flexible forme un article anonyme :
            # la norme l'exige, et sans cela une barre faite de mots seuls
            # disparaitrait.
            enveloppe = Element("span")
            enveloppe.parent = boite.element
            enveloppe.ajoute(Texte(enfant.contenu))
            style_anonyme = dict(boite.style)
            style_anonyme["display"] = "block"
            enveloppe.style_engendre = style_anonyme
            enfant = enveloppe

        chemin = _chemin(enfant)
        sous_boite = _boite_pour(enfant, boite.style, chemin, contexte)
        if sous_boite is None:
            continue
        if sous_boite.style.get("position", "static") in ("absolute", "fixed"):
            boite.hors_flux.append((sous_boite, chemin))
            continue
        # Les enfants d'un conteneur flexible ou en grille sont « blocifies » :
        # un `<span>` y devient une boite a part entiere, pas un bout de ligne.
        if sous_boite.style.get("display", "inline") in ("inline", "inline-block"):
            sous_boite.style["display"] = "block"
        boite.enfants.append(sous_boite)


def _dispose_bloc(boite, x, y, largeur_disponible, contexte, largeur_forcee=None):
    """Positionne une boite de type bloc et tout ce qu'elle contient.

    `largeur_forcee` court-circuite le calcul de largeur : flex et grille
    attribuent aux articles une taille que la boite ne peut pas deduire seule.
    """
    style = boite.style
    taille = _taille_police(style)

    # Une boite peut etre disposee deux fois — flex et grille la mesurent avant
    # de la poser. Sans cette remise a zero, la seconde passe empilerait ses
    # fragments sur ceux de la premiere et tout le texte serait double.
    del boite.enfants[:]
    del boite.lignes[:]
    del boite.images[:]
    del boite.hors_flux[:]
    boite.toile = None

    marge_g = _longueur(style, "margin-left", largeur_disponible, taille)
    marge_d = _longueur(style, "margin-right", largeur_disponible, taille)
    marge_h = _longueur(style, "margin-top", largeur_disponible, taille)
    marge_b = _longueur(style, "margin-bottom", largeur_disponible, taille)
    pad_g = _longueur(style, "padding-left", largeur_disponible, taille)
    pad_d = _longueur(style, "padding-right", largeur_disponible, taille)
    pad_h = _longueur(style, "padding-top", largeur_disponible, taille)
    pad_b = _longueur(style, "padding-bottom", largeur_disponible, taille)
    # Chaque bord a son epaisseur : un `border-bottom` seul ne doit pas decaler
    # le contenu des trois autres cotes.
    (b_h, _), (b_d, _), (b_b, _), (b_g, _) = css.bordures(style, largeur_disponible,
                                                          taille)

    boite.x = x + marge_g
    boite.y = y + marge_h
    largeur = largeur_disponible - marge_g - marge_d
    imposee = css.longueur(style.get("width", "auto"), largeur_disponible, taille)
    if largeur_forcee is not None:
        largeur = largeur_forcee
    elif imposee is not None and imposee > 0:
        # `box-sizing: border-box` : la largeur declaree comprend deja bordures
        # et remplissage — c'est ce que pose la quasi-totalite des feuilles de
        # style modernes, souvent sur `*`. Le defaut de CSS, lui, veut que
        # `width` designe le seul contenu et que le reste s'y ajoute.
        if style.get("box-sizing", "content-box") != "border-box":
            imposee += pad_g + pad_d + b_g + b_d
        largeur = min(largeur, imposee)
    boite.largeur = max(0.0, largeur)

    interieur_x = boite.x + pad_g + b_g
    interieur_l = max(0.0, boite.largeur - pad_g - pad_d - b_g - b_d)
    curseur = boite.y + pad_h + b_h

    # Une liste numerotee ou a puces compte ses elements pour les marquer.
    compteur = 0
    ordonnee = boite.element.balise == "ol" if isinstance(boite.element, Element) else False

    boite.rogne = style.get("overflow", "visible") in ("hidden", "auto", "scroll") \
        or style.get("overflow-y", "visible") in ("hidden", "auto", "scroll")

    # Une toile est une boite de taille declaree qui porte un dessin, pas du
    # contenu : ses enfants sont le repli qu'on montre a qui ne sait pas la
    # dessiner, et nous savons.
    if isinstance(boite.element, Element) and boite.element.balise == "canvas":
        _dispose_toile(boite, style, taille, curseur, pad_b, b_b,
                       interieur_l, contexte)
        return

    mode = style.get("display", "block")
    if mode in ("flex", "inline-flex", "grid", "inline-grid"):
        _boites_enfants(boite, contexte)

        def pose(sous, largeur, px=0.0, py=0.0, forcee=None):
            _dispose_bloc(sous, px, py, max(0.0, largeur), contexte, forcee)
            return (sous.largeur, sous.hauteur)

        module = flex if mode in ("flex", "inline-flex") else grille
        curseur += module.dispose(boite, interieur_x, curseur, interieur_l,
                                  contexte, pose)
        _termine_bloc(boite, style, curseur, pad_b, b_b, taille, interieur_l)
        _pose_hors_flux(boite, contexte)
        return

    enfants = _enfants(boite, contexte)
    en_attente = []  # suite de nœuds en ligne a disposer ensemble

    def vide_en_attente():
        nonlocal curseur
        if not en_attente:
            return
        hauteur = _dispose_ligne(boite, en_attente, interieur_x, curseur,
                                 interieur_l, boite.style, contexte)
        curseur += hauteur
        en_attente.clear()

    for enfant in enfants:
        if isinstance(enfant, Texte):
            if enfant.contenu.strip() or _preserve_espaces(boite.style):
                en_attente.append(enfant)
            continue

        chemin = _chemin(enfant)
        style_enfant = _style_de(enfant, boite.style, chemin, contexte)
        affichage = style_enfant.get("display", "inline")
        if affichage == "none":
            continue

        if affichage in ("inline", "inline-block"):
            en_attente.append(enfant)
            continue

        vide_en_attente()
        sous_boite = _boite_pour(enfant, boite.style, chemin, contexte)
        if sous_boite is None:
            continue
        # Positionnement absolu : la boite quitte le flux. On la garde de cote,
        # elle sera placee quand le bloc contenant connaitra sa taille.
        if style_enfant.get("position", "static") in ("absolute", "fixed"):
            boite.hors_flux.append((sous_boite, chemin))
            continue
        if affichage == "list-item":
            compteur += 1
            sous_boite.puce = "%d." % compteur if ordonnee else "•"
        boite.enfants.append(sous_boite)
        _dispose_bloc(sous_boite, interieur_x, curseur, interieur_l, contexte)
        style_s = sous_boite.style
        taille_s = _taille_police(style_s)
        curseur = (sous_boite.y + sous_boite.hauteur
                   + _longueur(style_s, "margin-bottom", interieur_l, taille_s))

    vide_en_attente()
    _termine_bloc(boite, style, curseur, pad_b, b_b, taille, interieur_l)
    _pose_hors_flux(boite, contexte)


def _dispose_toile(boite, style, taille, curseur, pad_b, bordure, reference,
                   contexte):
    """Donne a un `<canvas>` sa taille et le dessin qu'il porte.

    La taille vient des attributs `width`/`height`, comme le veut la norme —
    ce sont les dimensions du dessin, pas de l'affichage. Une regle CSS peut
    les remplacer, et c'est alors elle qui gagne.
    """
    attributs = boite.element.attributs
    largeur = _entier(attributs.get("width"), 300.0)
    hauteur = _entier(attributs.get("height"), 150.0)

    imposee = css.longueur(style.get("width", "auto"), reference, taille)
    if imposee is not None and imposee > 0:
        largeur = imposee
    boite.largeur = max(0.0, min(largeur, boite.largeur) if imposee is None else largeur)

    hauteur_css = css.longueur(style.get("height", "auto"), 0.0, taille)
    if hauteur_css is not None and hauteur_css > 0:
        hauteur = hauteur_css
    boite.hauteur = max(0.0, (curseur - boite.y) + hauteur + pad_b + bordure)

    rappel = getattr(contexte, "toile", None)
    boite.toile = rappel(boite.element) if rappel else None


def _entier(valeur, defaut):
    try:
        return float(str(valeur).strip())
    except (TypeError, ValueError, AttributeError):
        return defaut


def _termine_bloc(boite, style, curseur, pad_b, bordure, taille, reference):
    """Arrete la hauteur d'un bloc, bornes comprises."""
    hauteur = curseur + pad_b + bordure - boite.y
    imposee = css.longueur(style.get("height", "auto"), 0.0, taille)
    if imposee is not None and imposee > hauteur:
        hauteur = imposee

    minimum = css.longueur(style.get("min-height", "auto"), 0.0, taille)
    if minimum is not None:
        hauteur = max(hauteur, minimum)
    maximum = css.longueur(style.get("max-height", "none"), 0.0, taille)
    if maximum is not None:
        hauteur = min(hauteur, maximum)

    if isinstance(boite.element, Element) and boite.element.balise == "hr":
        trait = max(e for e, _ in css.bordures(style, reference, taille))
        hauteur = max(hauteur, trait if trait else 1.0)

    # Les bornes de largeur s'appliquent ici aussi : `max-width: 900px` centre
    # sur une colonne large est la forme la plus repandue de mise en page.
    minimum = css.longueur(style.get("min-width", "auto"), reference, taille)
    if minimum is not None:
        boite.largeur = max(boite.largeur, minimum)
    maximum = css.longueur(style.get("max-width", "none"), reference, taille)
    if maximum is not None and boite.largeur > maximum:
        # `margin: auto` centre ce qui est plus etroit que son contenant.
        if style.get("margin-left") == "auto" and style.get("margin-right") == "auto":
            boite.x += (boite.largeur - maximum) / 2.0
        boite.largeur = maximum

    boite.hauteur = max(0.0, hauteur)


def _pose_hors_flux(boite, contexte):
    """Place les enfants `position: absolute`/`fixed` de cette boite.

    Le bloc contenant est celui-ci des lors qu'il est lui-meme positionne — ce
    que `position: relative` sert precisement a declarer. `fixed` se rapporte a
    la fenetre, ce qui revient ici au meme puisque la peinture applique le
    defilement a tout le reste.
    """
    if not boite.hors_flux:
        return
    for sous_boite, chemin in boite.hors_flux:
        style = sous_boite.style
        taille = _taille_police(style)
        largeur_ref = boite.largeur or 0.0
        hauteur_ref = boite.hauteur or 0.0

        _dispose_bloc(sous_boite, boite.x, boite.y, largeur_ref, contexte)

        gauche = css.longueur(style.get("left", "auto"), largeur_ref, taille)
        droite = css.longueur(style.get("right", "auto"), largeur_ref, taille)
        haut = css.longueur(style.get("top", "auto"), hauteur_ref, taille)
        bas = css.longueur(style.get("bottom", "auto"), hauteur_ref, taille)

        if gauche is not None:
            sous_boite.x = boite.x + gauche
        elif droite is not None:
            sous_boite.x = boite.x + largeur_ref - droite - sous_boite.largeur
        if haut is not None:
            sous_boite.y = boite.y + haut
        elif bas is not None:
            sous_boite.y = boite.y + hauteur_ref - bas - sous_boite.hauteur

        boite.enfants.append(sous_boite)


def _preserve_espaces(style):
    return style.get("white-space", "normal").startswith("pre")


def _chemin(element):
    """Liste des ancetres de l'element, du plus lointain a lui-meme."""
    chemin = []
    courant = element
    while courant is not None:
        chemin.append(courant)
        courant = courant.parent
    chemin.reverse()
    return chemin


def _dispose_ligne(boite, nœuds, x, y, largeur, style_parent, contexte):
    """Dispose une suite de nœuds en ligne. Rend la hauteur consommee."""
    curseur_x = x
    curseur_y = y
    hauteur_ligne_courante = 0.0

    # La puce d'un element de liste occupe le debut de la premiere ligne.
    if boite.puce:
        taille = _taille_police(style_parent)
        largeur_puce = bo.largeur_texte(boite.puce, taille, False, False)
        boite.lignes.append(Fragment(x - largeur_puce - 6, y, boite.puce,
                                     bo.hauteur_ligne(taille, False), style_parent, None))

    pile = [(n, style_parent, None) for n in nœuds]
    while pile:
        nœud, style, lien = pile.pop(0)

        if isinstance(nœud, Texte):
            curseur_x, curseur_y, hauteur_ligne_courante = _pose_texte(
                boite, nœud.contenu, style, lien, x, largeur,
                curseur_x, curseur_y, hauteur_ligne_courante)
            continue

        chemin = _chemin(nœud)
        style_enfant = _style_de(nœud, style, chemin, contexte)
        if style_enfant.get("display", "inline") == "none":
            continue
        lien_enfant = lien
        if nœud.balise == "a" and nœud.attributs.get("href"):
            lien_enfant = nœud.attributs["href"]
        if nœud.balise == "br":
            curseur_x = x
            curseur_y += hauteur_ligne_courante or bo.hauteur_ligne(
                _taille_police(style), _est_fixe(style))
            hauteur_ligne_courante = 0.0
            continue
        if nœud.balise == "video":
            # L'image courante du lecteur, s'il y en a une. Le lecteur vit dans
            # le contexte JavaScript : la mise en page ne fait que demander ce
            # qu'il y a a montrer.
            trame = getattr(contexte, "image_video", None)
            trame = trame(nœud) if trame else None
            if trame is not None:
                identifiant, largeur_v, hauteur_v = trame
                largeur_a, hauteur_a = images.dimensions(
                    (identifiant, largeur_v, hauteur_v), nœud.attributs,
                    style_enfant, largeur)
                if largeur_a > 0 and hauteur_a > 0:
                    if curseur_x > x and curseur_x + largeur_a > x + largeur:
                        curseur_x = x
                        curseur_y += hauteur_ligne_courante
                        hauteur_ligne_courante = 0.0
                    boite.images.append((curseur_x, curseur_y, largeur_a,
                                         hauteur_a, identifiant, lien_enfant))
                    curseur_x += largeur_a
                    hauteur_ligne_courante = max(hauteur_ligne_courante, hauteur_a)
                    continue
            # Pas encore d'image : les enfants du `<video>` (le texte de repli)
            # prennent sa place, comme le veut la norme.
            pile = [(e, style_enfant, lien_enfant) for e in nœud.enfants] + pile
            continue
        if nœud.balise == "img":
            curseur_x, curseur_y, hauteur_ligne_courante = _pose_image(
                boite, nœud, style_enfant, lien_enfant, x, largeur,
                curseur_x, curseur_y, hauteur_ligne_courante, contexte)
            continue
        # Les enfants prennent la place du nœud, en tete de file.
        pile = [(e, style_enfant, lien_enfant) for e in nœud.enfants] + pile

    return (curseur_y - y) + hauteur_ligne_courante


def _pose_image(boite, nœud, style, lien, gauche, largeur,
                curseur_x, curseur_y, hauteur_ligne, contexte):
    """Pose une image dans le flux en ligne, ou son texte de remplacement."""
    naturelle = images.charge(getattr(contexte, "url", ""), nœud.attributs.get("src"))
    if naturelle is None:
        # Rien a montrer : on retombe sur `alt`, ce que fait tout navigateur
        # devant une image absente.
        remplacement = nœud.attributs.get("alt", "")
        if not remplacement:
            return curseur_x, curseur_y, hauteur_ligne
        return _pose_texte(boite, remplacement, style, lien, gauche, largeur,
                           curseur_x, curseur_y, hauteur_ligne)

    largeur_image, hauteur_image = images.dimensions(
        naturelle, nœud.attributs, style, largeur)
    if largeur_image <= 0 or hauteur_image <= 0:
        return curseur_x, curseur_y, hauteur_ligne

    # Elle ne tient pas sur la fin de ligne : on passe a la suivante.
    if curseur_x > gauche and curseur_x + largeur_image > gauche + largeur:
        curseur_x = gauche
        curseur_y += hauteur_ligne
        hauteur_ligne = 0.0

    boite.images.append((curseur_x, curseur_y, largeur_image, hauteur_image,
                         naturelle[0], lien))
    return (curseur_x + largeur_image, curseur_y,
            max(hauteur_ligne, hauteur_image))


def _pose_texte(boite, texte, style, lien, gauche, largeur,
                curseur_x, curseur_y, hauteur_ligne):
    """Coupe le texte en mots et les pose en revenant a la ligne au besoin."""
    taille = _taille_police(style)
    gras = _est_gras(style)
    fixe = _est_fixe(style)
    hauteur = bo.hauteur_ligne(taille, fixe)
    facteur = _facteur_interligne(style)
    hauteur_ligne = max(hauteur_ligne, hauteur * facteur)

    if _preserve_espaces(style):
        for index, ligne in enumerate(texte.split("\n")):
            if index:
                curseur_x = gauche
                curseur_y += hauteur_ligne
            if ligne:
                boite.lignes.append(Fragment(curseur_x, curseur_y, ligne,
                                             hauteur, style, lien))
                curseur_x += bo.largeur_texte(ligne, taille, gras, fixe)
        return curseur_x, curseur_y, hauteur_ligne

    mots = texte.split()
    if not mots:
        return curseur_x, curseur_y, hauteur_ligne

    espace = bo.largeur_texte(" ", taille, gras, fixe)
    tampon = []
    tampon_largeur = 0.0

    def ecrit():
        nonlocal tampon, tampon_largeur, curseur_x
        if not tampon:
            return
        contenu = " ".join(tampon)
        boite.lignes.append(Fragment(curseur_x, curseur_y, contenu, hauteur, style, lien))
        curseur_x += tampon_largeur
        tampon = []
        tampon_largeur = 0.0

    # Un texte qui suit un autre sur la meme ligne a besoin de son espace.
    if curseur_x > gauche and texte[:1].isspace():
        curseur_x += espace

    for mot in mots:
        largeur_mot = bo.largeur_texte(mot, taille, gras, fixe)
        supplement = largeur_mot + (espace if tampon else 0.0)
        if curseur_x + tampon_largeur + supplement > gauche + largeur and (tampon or curseur_x > gauche):
            ecrit()
            curseur_x = gauche
            curseur_y += hauteur_ligne
            hauteur_ligne = hauteur * facteur
            supplement = largeur_mot
        tampon.append(mot)
        tampon_largeur += supplement
    ecrit()

    if texte[-1:].isspace():
        curseur_x += espace
    return curseur_x, curseur_y, hauteur_ligne


def _facteur_interligne(style):
    brut = style.get("line-height", "1.5")
    try:
        valeur = float(brut)
        return valeur if valeur > 0 else 1.5
    except ValueError:
        pass
    mesure = css.longueur(brut, 0.0, _taille_police(style))
    if mesure and mesure > 0:
        return mesure / max(1.0, _taille_police(style))
    return 1.5
