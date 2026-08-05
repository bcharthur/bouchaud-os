"""Peinture : de l'arbre de boites a une liste d'affichage.

La liste d'affichage est une suite de tuples plats que l'hote Qt sait peindre
(voir `hote.cpp`). Aucun objet Qt ne remonte jusqu'ici : le moteur decrit ce
qu'il veut voir, l'hote s'occupe de le dessiner. C'est ce qui permet de tester
le moteur sans ecran, et de changer d'hote sans toucher au moteur.
"""

from . import css
from .mise_en_page import _est_fixe, _est_gras, _taille_police

# Couleur du soulignement et du texte des liens quand la feuille n'en donne pas.
COULEUR_LIEN = 0xFF1A56DB


def peint(boite, defilement, largeur_vue, hauteur_vue, zones_liens):
    """Rend la liste d'affichage d'un arbre de boites.

    `zones_liens` est rempli au passage avec des `(x, y, largeur, hauteur, url)`
    en coordonnees de page : c'est ce qui permet ensuite de savoir sur quel lien
    un clic est tombe, sans reparcourir l'arbre.
    """
    liste = []
    _peint_boite(boite, liste, defilement, largeur_vue, hauteur_vue, zones_liens)
    return liste


def _peint_boite(boite, liste, defilement, largeur_vue, hauteur_vue, zones_liens):
    haut = boite.y - defilement
    bas = haut + boite.hauteur

    # Elagage : une boite entierement hors de la vue n'est pas peinte, mais ses
    # enfants peuvent l'etre — une boite peut deborder de sa hauteur calculee.
    visible = bas >= -200 and haut <= hauteur_vue + 200

    style = boite.style
    if visible:
        fond = css.couleur(style.get("background-color", ""))
        if fond is not None and boite.hauteur > 0:
            liste.append(("rect", boite.x, haut, boite.largeur, boite.hauteur, fond))

        epaisseur = css.longueur(style.get("border-width", "0"), boite.largeur,
                                 _taille_police(style)) or 0.0
        if epaisseur > 0:
            couleur_bordure = css.couleur(style.get("border-color", "#d0d7de"))
            if couleur_bordure is None:
                couleur_bordure = 0xFFD0D7DE
            l, h = boite.largeur, boite.hauteur
            liste.append(("rect", boite.x, haut, l, epaisseur, couleur_bordure))
            liste.append(("rect", boite.x, haut + h - epaisseur, l, epaisseur, couleur_bordure))
            liste.append(("rect", boite.x, haut, epaisseur, h, couleur_bordure))
            liste.append(("rect", boite.x + l - epaisseur, haut, epaisseur, h, couleur_bordure))

        for fragment in boite.lignes:
            _peint_fragment(fragment, liste, defilement, hauteur_vue, zones_liens)

        for x, y_page, l, h, identifiant, lien in boite.images:
            y = y_page - defilement
            if y + h < 0 or y > hauteur_vue:
                continue
            # L'image est deja decodee : la liste d'affichage ne porte que son
            # numero, l'hote la retrouve dans son cache.
            liste.append(("image", x, y, l, h, identifiant))
            if lien:
                zones_liens.append((x, y_page, l, h, lien))

    for enfant in boite.enfants:
        _peint_boite(enfant, liste, defilement, largeur_vue, hauteur_vue, zones_liens)


def _peint_fragment(fragment, liste, defilement, hauteur_vue, zones_liens):
    y = fragment.y - defilement
    if y + fragment.hauteur < 0 or y > hauteur_vue:
        return
    style = fragment.style
    taille = _taille_police(style)
    gras = _est_gras(style)
    fixe = _est_fixe(style)
    italique = style.get("font-style", "normal") == "italic"

    teinte = css.couleur(style.get("color", "#202124"))
    if teinte is None:
        teinte = 0xFF202124
    souligne = "underline" in style.get("text-decoration", "")
    if fragment.lien:
        souligne = souligne or True
        if style.get("color") is None:
            teinte = COULEUR_LIEN

    liste.append(("texte", fragment.x, y, fragment.texte, teinte, taille,
                  gras, italique, fixe, souligne))

    if fragment.lien:
        import bo
        largeur = bo.largeur_texte(fragment.texte, taille, gras, fixe)
        zones_liens.append((fragment.x, fragment.y, largeur, fragment.hauteur,
                            fragment.lien))
