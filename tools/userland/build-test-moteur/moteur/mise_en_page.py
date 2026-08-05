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

from . import css, images
from .html import Element, Texte


class Boite:
    """Une boite positionnee, prete a peindre."""

    __slots__ = ("element", "style", "x", "y", "largeur", "hauteur",
                 "enfants", "lignes", "lien", "puce", "images")

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

    def __init__(self, regles, largeur_page, url="", image_video=None):
        self.regles = regles
        self.largeur_page = largeur_page
        # Adresse de la page : les `src` relatifs des images s'y resolvent.
        self.url = url
        # Rappel qui rend l'image courante d'un `<video>`, ou `None`.
        self.image_video = image_video


def construit(racine, regles, largeur_page, url="", image_video=None):
    """Construit l'arbre de boites et rend (racine, hauteur totale)."""
    contexte = Contexte(regles, largeur_page, url, image_video)
    style_initial = {
        "color": "#202124", "font-size": "16px", "line-height": "1.5",
        "display": "block",
    }
    corps = racine.trouve("body") or racine
    boite = _boite_pour(corps, style_initial, [corps], contexte)
    if boite is None:
        boite = Boite(corps, style_initial)
    _dispose_bloc(boite, 0.0, 0.0, largeur_page, contexte)
    return boite, boite.hauteur


def _boite_pour(element, style_parent, chemin, contexte):
    style = css.applique(contexte.regles, element, chemin, style_parent)
    if style.get("display", "inline") == "none":
        return None
    boite = Boite(element, style)
    if element.balise == "a" and element.attributs.get("href"):
        boite.lien = element.attributs["href"]
    return boite


def _dispose_bloc(boite, x, y, largeur_disponible, contexte):
    """Positionne une boite de type bloc et tout ce qu'elle contient."""
    style = boite.style
    taille = _taille_police(style)

    marge_g = _longueur(style, "margin-left", largeur_disponible, taille)
    marge_d = _longueur(style, "margin-right", largeur_disponible, taille)
    marge_h = _longueur(style, "margin-top", largeur_disponible, taille)
    marge_b = _longueur(style, "margin-bottom", largeur_disponible, taille)
    pad_g = _longueur(style, "padding-left", largeur_disponible, taille)
    pad_d = _longueur(style, "padding-right", largeur_disponible, taille)
    pad_h = _longueur(style, "padding-top", largeur_disponible, taille)
    pad_b = _longueur(style, "padding-bottom", largeur_disponible, taille)
    bordure = _longueur(style, "border-width", largeur_disponible, taille)

    boite.x = x + marge_g
    boite.y = y + marge_h
    largeur = largeur_disponible - marge_g - marge_d
    imposee = css.longueur(style.get("width", "auto"), largeur_disponible, taille)
    if imposee is not None and imposee > 0:
        largeur = min(largeur, imposee)
    boite.largeur = max(0.0, largeur)

    interieur_x = boite.x + pad_g + bordure
    interieur_l = max(0.0, boite.largeur - pad_g - pad_d - 2 * bordure)
    curseur = boite.y + pad_h + bordure

    # Une liste numerotee ou a puces compte ses elements pour les marquer.
    compteur = 0
    ordonnee = boite.element.balise == "ol" if isinstance(boite.element, Element) else False

    enfants = boite.element.enfants if isinstance(boite.element, Element) else []
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
        style_enfant = css.applique(contexte.regles, enfant, chemin, boite.style)
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

    hauteur = curseur + pad_b + bordure - boite.y
    imposee = css.longueur(style.get("height", "auto"), 0.0, taille)
    if imposee is not None and imposee > hauteur:
        hauteur = imposee
    if boite.element.balise == "hr":
        hauteur = max(hauteur, bordure if bordure else 1.0)
    boite.hauteur = max(0.0, hauteur)
    boite.hauteur += 0.0 if marge_b else 0.0


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
        style_enfant = css.applique(contexte.regles, nœud, chemin, style)
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
