"""Peinture : de l'arbre de boites a une liste d'affichage.

La liste d'affichage est une suite de tuples plats que l'hote Qt sait peindre
(voir `hote.cpp`). Aucun objet Qt ne remonte jusqu'ici : le moteur decrit ce
qu'il veut voir, l'hote s'occupe de le dessiner. C'est ce qui permet de tester
le moteur sans ecran, et de changer d'hote sans toucher au moteur.

Une boite se peint dans l'ordre de la norme : l'ombre portee d'abord, puis le
fond — couleur ou degrade —, puis les quatre bords, puis le contenu. Un
`transform` et une `opacity` enveloppent le tout, elle et sa descendance.
"""

from . import css
from .mise_en_page import _est_fixe, _est_gras, _taille_police, famille

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


def _applique(matrice, x, y):
    a, b, c, d, e, f = matrice
    return (a * x + c * y + e, b * x + d * y + f)


def _zone_transformee(matrice, x, y, largeur, hauteur):
    """La boite englobante d'un rectangle une fois transforme.

    Sans cela, un clic sur un lien pose dans un bloc translate tomberait a
    l'endroit ou le lien serait sans la transformation.
    """
    if matrice is None:
        return (x, y, largeur, hauteur)
    coins = [_applique(matrice, x, y), _applique(matrice, x + largeur, y),
             _applique(matrice, x, y + hauteur),
             _applique(matrice, x + largeur, y + hauteur)]
    xs = [p[0] for p in coins]
    ys = [p[1] for p in coins]
    return (min(xs), min(ys), max(xs) - min(xs), max(ys) - min(ys))


def _colle(boite, style, haut, defilement, hauteur_vue, contenant):
    """Le decalage vertical d'une boite `sticky` ou `fixed`, en pixels.

    Les deux se resolvent a la peinture parce que les deux dependent du
    defilement, que la mise en page ne connait pas. Une barre `sticky` reste a
    sa place tant qu'elle est visible, se colle a son ancrage ensuite, et
    repart avec son contenant quand celui-ci sort par le haut.
    """
    position = style.get("position", "static")
    if position == "fixed":
        # `fixed` se rapporte a la fenetre : annuler le defilement, que la
        # peinture applique a tout le reste, revient a l'y fixer.
        return defilement
    if position != "sticky":
        return 0.0

    taille = _taille_police(style)
    ancrage = css.longueur(style.get("top", ""), hauteur_vue, taille)
    if ancrage is None:
        return 0.0
    colle = max(haut, ancrage)
    if contenant is not None and contenant.hauteur > 0:
        # Elle ne depasse jamais le bas de son contenant.
        limite = (contenant.y + contenant.hauteur - defilement) - boite.hauteur
        colle = min(colle, max(limite, haut))
    return colle - haut


def _peint_curseur(boite, liste, defilement, hauteur_vue):
    """La selection derriere le texte, puis le trait du curseur devant.

    L'ordre compte : peindre la selection apres le texte le recouvrirait. Elle
    est donc dessinee avant — mais les fragments l'ont deja ete, alors on la
    pose en dessous en la dessinant en premier dans cette passe, ce que la
    liste d'affichage permet parce que la selection est translucide.
    """
    x, y, hauteur, selection = boite.curseur
    haut = y - defilement
    if haut + hauteur < 0 or haut > hauteur_vue:
        return
    if selection is not None:
        debut_x, largeur = selection
        if largeur > 0:
            liste.append(("rect", debut_x, haut, largeur, hauteur, 0x553B82F6))
    liste.append(("rect", x, haut, 1.0, hauteur, 0xFF1F2937))


def _peint_boite(boite, liste, defilement, largeur_vue, hauteur_vue, zones_liens,
                 matrice=None, contenant=None):
    style = boite.style
    haut = boite.y - defilement

    # `transform` et `opacity` enveloppent la boite et toute sa descendance.
    # L'origine par defaut est le centre de la boite, comme le veut CSS.
    propre = css.transformation(style.get("transform", ""), boite.largeur,
                                boite.hauteur, _taille_police(style))
    decalage = _colle(boite, style, haut, defilement, hauteur_vue, contenant)
    if decalage:
        # Un collage est une translation : il passe par le meme mecanisme que
        # `transform`, et beneficie donc du meme report sur les zones de liens.
        glissement = (1.0, 0.0, 0.0, 1.0, 0.0, decalage)
        propre = glissement if propre is None else css._compose(glissement, propre)
    alpha = css.opacite(style.get("opacity"))
    enveloppes = 0
    if propre is not None:
        origine_x = boite.x + boite.largeur / 2.0
        origine_y = haut + boite.hauteur / 2.0
        liste.append(("transforme",) + tuple(propre) + (origine_x, origine_y))
        enveloppes += 1
        # La matrice cumulee sert aux zones de liens, exprimees en page : elle
        # est donc composee autour de l'origine, en coordonnees de page.
        vers_origine = (1.0, 0.0, 0.0, 1.0, boite.x + boite.largeur / 2.0,
                        boite.y + boite.hauteur / 2.0)
        retour = (1.0, 0.0, 0.0, 1.0, -(boite.x + boite.largeur / 2.0),
                  -(boite.y + boite.hauteur / 2.0))
        locale = css._compose(css._compose(vers_origine, propre), retour)
        matrice = locale if matrice is None else css._compose(matrice, locale)
    if alpha < 1.0:
        liste.append(("opacite", alpha))
        enveloppes += 1

    _peint_contenu(boite, liste, defilement, largeur_vue, hauteur_vue,
                   zones_liens, matrice)

    for _ in range(enveloppes):
        liste.append(("desenveloppe",))


def _peint_contenu(boite, liste, defilement, largeur_vue, hauteur_vue, zones_liens,
                   matrice):
    haut = boite.y - defilement
    bas = haut + boite.hauteur

    # Elagage : une boite entierement hors de la vue n'est pas peinte, mais ses
    # enfants peuvent l'etre — une boite peut deborder de sa hauteur calculee.
    # Sous une transformation, la boite n'est plus la ou ses coordonnees la
    # disent : l'elagage serait alors un decoupage au hasard.
    visible = matrice is not None or (bas >= -200 and haut <= hauteur_vue + 200)

    style = boite.style
    if visible:
        taille = _taille_police(style)
        l, h = boite.largeur, boite.hauteur
        coins = css.rayons(style.get("border-radius", ""), l, taille)
        rayon = max(coins) if coins else 0.0

        # Les ombres portees se peignent sous la boite, donc avant elle ; les
        # ombres interieures apres son fond, donc plus bas.
        interieures = []
        for dx, dy, flou, etendue, teinte, interieure in css.ombres(
                style.get("box-shadow", ""), taille):
            if h <= 0:
                continue
            if interieure:
                interieures.append((dx, dy, flou, etendue, teinte))
                continue
            liste.append(("ombre", boite.x + dx - etendue, haut + dy - etendue,
                          l + 2 * etendue, h + 2 * etendue, rayon, flou, teinte))

        # Le fond : un degrade s'il y en a un, la couleur sinon.
        pente = css.degrade(style.get("background-image", ""))
        fond = css.couleur(style.get("background-color", ""))
        if h > 0 and pente is not None:
            genre, angle, etapes = pente
            operation = "degrade_radial" if genre == "radial" else "degrade"
            liste.append((operation, boite.x, haut, l, h, rayon, angle, etapes))
        elif fond is not None and h > 0:
            if rayon > 0:
                liste.append(("rond", boite.x, haut, l, h, rayon, fond))
            else:
                liste.append(("rect", boite.x, haut, l, h, fond))

        # Une ombre interieure se peint par-dessus le fond, rognee a la boite :
        # c'est un halo pose le long du bord interne, pas une forme posee
        # dessous. Elle etait laissee de cote faute de savoir la rogner.
        for dx, dy, flou, etendue, teinte in interieures:
            liste.append(("ombre_interne", boite.x, haut, l, h, rayon,
                          dx, dy, flou, etendue, teinte))

        # Les quatre bords, chacun avec son epaisseur et sa couleur. Un
        # `border-bottom` seul — le separateur le plus repandu du web — ne
        # peignait rien tant que le moteur ne lisait qu'une bordure unique.
        (e_h, c_h), (e_d, c_d), (e_b, c_b), (e_g, c_g) = css.bordures(
            style, l, taille)
        if rayon > 0 and e_h > 0 and e_h == e_d == e_b == e_g:
            # Un cadre arrondi uniforme se trace en un contour, sinon les quatre
            # rectangles deborderaient des coins.
            liste.append(("contour", boite.x, haut, l, h, rayon, e_h, c_h))
        else:
            if e_h > 0:
                liste.append(("rect", boite.x, haut, l, e_h, c_h))
            if e_b > 0:
                liste.append(("rect", boite.x, haut + h - e_b, l, e_b, c_b))
            if e_g > 0:
                liste.append(("rect", boite.x, haut, e_g, h, c_g))
            if e_d > 0:
                liste.append(("rect", boite.x + l - e_d, haut, e_d, h, c_d))

        for fragment in boite.lignes:
            _peint_fragment(fragment, liste, defilement, hauteur_vue, zones_liens,
                            matrice)

        # Le curseur et la selection d'un champ au foyer. Ils sont poses par la
        # mise en page, pas inseres dans le texte : un caret ecrit dans la
        # valeur partirait avec le formulaire, et changerait la largeur mesuree
        # a chaque battement de clignotement.
        if boite.curseur is not None:
            _peint_curseur(boite, liste, defilement, hauteur_vue)

        # Le dessin d'une toile est une liste d'affichage a part entiere,
        # exprimee dans les coordonnees de la toile : la poser revient a la
        # decaler et a la rogner a ses bords.
        # Un `<iframe>` : la liste d'affichage du document enfant, decalee a la
        # place de la boite et rognee a ses bornes.
        #
        # Le rognage n'est pas cosmetique : sans lui, un document enfant plus
        # haut que son cadre deborderait sur la page parente, et une page
        # hostile pourrait recouvrir l'interface de son hote. C'est la premiere
        # chose qu'un `<iframe>` doit garantir.
        if boite.cadre is not None:
            _peint_cadre(boite, liste, haut)

        if boite.toile:
            liste.append(("clip", boite.x, haut, boite.largeur, boite.hauteur))
            for operation in boite.toile:
                liste.append(_decale(operation, boite.x, haut))
            liste.append(("declip",))

        for x, y_page, l_img, h_img, identifiant, lien, source in boite.images:
            y = y_page - defilement
            if y + h_img < 0 or y > hauteur_vue:
                continue
            # L'image est deja decodee : la liste d'affichage ne porte que son
            # numero, l'hote la retrouve dans son cache.
            if source is None:
                liste.append(("image", x, y, l_img, h_img, identifiant))
            else:
                # `object-fit: cover` prend une portion de l'image : l'hote a
                # besoin du rectangle source, sans quoi il l'etirerait.
                liste.append(("imagepart", x, y, l_img, h_img, identifiant)
                             + tuple(source))
            if lien:
                zones_liens.append(
                    _zone_transformee(matrice, x, y_page, l_img, h_img) + (lien,))

    # `overflow: hidden` rogne ce qui depasse. Les zones s'emboitent — l'hote
    # les empile et les intersecte — ce qui rend la paire toujours sure : la
    # sortie d'un rognage rend celui qui l'englobait, jamais l'absence de
    # rognage.
    rogne_ici = boite.rogne and boite.hauteur > 0 and boite.largeur > 0
    if rogne_ici:
        liste.append(("clip", boite.x, haut, boite.largeur, boite.hauteur))

    for enfant in boite.enfants:
        _peint_boite(enfant, liste, defilement, largeur_vue, hauteur_vue,
                     zones_liens, matrice, boite)

    if rogne_ici:
        liste.append(("declip",))


def _peint_cadre(boite, liste, haut):
    """Insere la liste d'affichage d'un document enfant dans celle du parent.

    Le document enfant est peint dans **son** repere, a partir de (0, 0) et
    avec son propre defilement. On le translate ensuite a la place de la boite.
    Cette separation compte : l'enfant ne sait rien de l'endroit ou il est
    affiche, ce qui est exactement ce qu'il faudra quand il sera peint dans un
    autre processus et transmis par un tampon partage.

    Les zones de liens de l'enfant ne remontent **pas** au parent : un clic dans
    un iframe se resout dans le contexte de l'iframe, pas dans celui de la page
    hote. Les melanger ferait qu'un lien d'une page tierce serait suivi comme
    s'il venait de la notre.
    """
    contexte = boite.cadre
    document = getattr(contexte, "document", None)
    if document is None or getattr(document, "boite", None) is None:
        return
    if boite.largeur <= 0 or boite.hauteur <= 0:
        return

    try:
        interieure = peint(document.boite, contexte.defilement_y,
                           boite.largeur, boite.hauteur, [])
    except Exception:  # noqa: BLE001 — un iframe casse ne casse pas la page
        return

    liste.append(("clip", boite.x, haut, boite.largeur, boite.hauteur))
    dx = boite.x - contexte.defilement_x
    for operation in interieure:
        liste.append(_decale(operation, dx, haut))
    liste.append(("declip",))


def _peint_fragment(fragment, liste, defilement, hauteur_vue, zones_liens,
                    matrice=None):
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
                  gras, italique, fixe, souligne, famille(style)))

    if fragment.lien:
        import bo
        largeur = bo.largeur_texte(fragment.texte, taille, gras, fixe,
                                   famille(style))
        zones_liens.append(
            _zone_transformee(matrice, fragment.x, fragment.y, largeur,
                              fragment.hauteur) + (fragment.lien,))


def _decale(operation, dx, dy):
    """Deplace une operation de toile a l'emplacement de sa boite."""
    genre = operation[0]
    if genre == "rect":
        return (genre, operation[1] + dx, operation[2] + dy) + tuple(operation[3:])
    if genre == "image":
        return (genre, operation[1] + dx, operation[2] + dy) + tuple(operation[3:])
    if genre == "texte":
        return (genre, operation[1] + dx, operation[2] + dy) + tuple(operation[3:])
    if genre == "ligne":
        return (genre, operation[1] + dx, operation[2] + dy,
                operation[3] + dx, operation[4] + dy) + tuple(operation[5:])
    if genre in ("ombre", "clip"):
        return (genre, operation[1] + dx, operation[2] + dy) + tuple(operation[3:])
    if genre == "polygone":
        return (genre, tuple((x + dx, y + dy) for x, y in operation[1]),
                operation[2])
    if genre == "degrade_points":
        # La boite **et** les deux extremites de la ligne se deplacent.
        return (genre, operation[1] + dx, operation[2] + dy, operation[3],
                operation[4], operation[5] + dx, operation[6] + dy,
                operation[7] + dx, operation[8] + dy, operation[9])
    if genre == "degrade_cercle":
        return (genre, operation[1] + dx, operation[2] + dy, operation[3],
                operation[4], operation[5] + dx, operation[6] + dy,
                operation[7], operation[8])
    return operation
