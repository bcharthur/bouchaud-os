"""Disposition en grille : `display: grid`.

Moins repandue que la disposition flexible, mais elle porte ce que celle-ci ne
sait pas faire : les mises en page a deux dimensions — un gabarit de page, une
galerie, un tableau de bord. Une page qui l'emploie et ne l'obtient pas ne perd
pas son alignement, elle perd sa structure entiere.

## Ce qui est mis en œuvre

Les pistes explicites (`grid-template-columns`, `grid-template-rows`) avec les
unites qu'on rencontre reellement : longueurs, pourcentages, `fr`, `auto`,
`repeat()` et `minmax()`. Le placement automatique remplit les cases dans
l'ordre du source, ligne par ligne. Le placement explicite (`grid-column`,
`grid-row`) est honore, y compris `span`.

## Ce qui ne l'est pas

Les lignes nommees, le placement
dense, et les pistes implicites au-dela d'un remplissage simple. Une page qui en
depend s'affichera avec ses elements dans l'ordre, ce qui reste lisible — c'est
la degradation qu'on veut, pas un ecroulement.
"""

import re

from . import css


class Piste:
    """Une colonne ou une ligne de la grille."""

    __slots__ = ("type", "valeur", "taille", "minimum", "plafond",
                 "min_contenu", "max_contenu")

    def __init__(self, type_, valeur, minimum=0.0, plafond=None):
        # `fixe` (pixels), `fraction` (fr), `auto`, `min-content`,
        # `max-content`, `pourcent`
        self.type = type_
        self.valeur = valeur
        self.taille = 0.0
        self.minimum = minimum
        # Borne haute d'un `minmax(a, b)` : `None` quand il n'y en a pas.
        self.plafond = plafond
        # Ce que le contenu de cette piste demande. Rempli avant la mesure, a
        # partir des articles qui y tombent — c'est precisement ce que la
        # mesure n'avait pas et qui l'obligeait a partager la place au hasard.
        self.min_contenu = 0.0
        self.max_contenu = 0.0


def est_grille(style):
    return style.get("display", "") in ("grid", "inline-grid")


def dispose(boite, x, y, largeur_disponible, contexte, pose_enfant):
    """Dispose les enfants en grille. Rend la hauteur consommee.

    `pose_enfant(sous_boite, largeur, x, y, forcee)` dispose un enfant et rend
    `(largeur, hauteur)`. Comme pour la disposition flexible, chaque case est
    mesuree a l'origine puis reposee a sa place : deplacer la boite apres coup
    laisserait sa descendance derriere elle.
    """
    style = boite.style
    taille_police = css.longueur(style.get("font-size", "16px"), 16.0, 16.0) or 16.0

    espace_colonne = _longueur(style, "column-gap", largeur_disponible, taille_police)
    espace_ligne = _longueur(style, "row-gap", largeur_disponible, taille_police)

    zones_nommees, colonnes_nommees = zones(style.get("grid-template-areas", ""))

    colonnes = _pistes(style.get("grid-template-columns", "none"),
                       largeur_disponible, taille_police)
    if not colonnes:
        if colonnes_nommees:
            # Le gabarit dit combien de colonnes il faut, meme sans
            # `grid-template-columns` : chaque piste est un objet distinct,
            # puisque la mesure les modifie une a une.
            colonnes = [Piste("fraction", 1.0) for _ in range(colonnes_nommees)]
        else:
            # Sans colonnes declarees, la grille se comporte comme une colonne
            # unique : c'est ce que fait un navigateur, et cela reste lisible.
            colonnes = [Piste("fraction", 1.0)]

    enfants = list(boite.enfants)
    if not enfants:
        return 0.0

    # Le placement vient **avant** la mesure : on ne peut pas savoir ce qu'une
    # colonne demande tant qu'on ignore ce qui y tombe. C'est l'inversion qui
    # rend le dimensionnement par le contenu possible.
    placements = _place(enfants, len(colonnes), zones_nommees)
    nombre_lignes = max((p[1] + p[3] for p in placements), default=1)

    contributions(colonnes, enfants, placements,
                  getattr(contexte, "mesureur", None),
                  getattr(contexte, "_styles", None))
    _mesure_colonnes(colonnes, largeur_disponible, espace_colonne)

    lignes = _pistes(style.get("grid-template-rows", "none"),
                     largeur_disponible, taille_police)
    while len(lignes) < nombre_lignes:
        lignes.append(Piste("auto", 0.0))

    # Hauteur de chaque ligne : le plus grand contenu qu'elle porte. Une case
    # qui s'etend sur plusieurs lignes ne dicte la hauteur d'aucune d'elles —
    # la norme lui fait partager sa demande, ce qui n'a d'effet visible que sur
    # des gabarits que nous ne pretendons pas rendre au pixel.
    hauteurs = [0.0] * len(lignes)
    largeurs = []
    for enfant, (colonne, ligne, etendue_colonne, etendue_ligne) in zip(enfants, placements):
        largeur_case = max(0.0, _somme(colonnes, colonne, etendue_colonne, espace_colonne))
        _, hauteur = pose_enfant(enfant, largeur_case, 0.0, 0.0, largeur_case)
        largeurs.append(largeur_case)
        if etendue_ligne == 1 and ligne < len(hauteurs):
            hauteurs[ligne] = max(hauteurs[ligne], hauteur)

    for index, piste in enumerate(lignes):
        if piste.type == "fixe" and index < len(hauteurs):
            hauteurs[index] = max(hauteurs[index], piste.valeur)

    # Pose definitive, a la place que les pistes mesurees designent.
    for (enfant, (colonne, ligne, _etendue_c, etendue_ligne), largeur_case) \
            in zip(enfants, placements, largeurs):
        decalage_x = _debut(colonnes, colonne, espace_colonne)
        decalage_y = _debut_lignes(hauteurs, ligne, espace_ligne)
        pose_enfant(enfant, largeur_case, x + decalage_x, y + decalage_y,
                    largeur_case)
        haute = _somme_hauteurs(hauteurs, ligne, etendue_ligne, espace_ligne)
        enfant.hauteur = max(enfant.hauteur, haute)

    total = sum(hauteurs) + espace_ligne * max(0, len(hauteurs) - 1)
    return max(0.0, total)


# --- Analyse des pistes -------------------------------------------------------

_REPEAT = re.compile(r"repeat\(\s*([0-9]+|auto-fill|auto-fit)\s*,\s*(.+?)\s*\)")
_MINMAX = re.compile(r"minmax\(\s*([^,]+)\s*,\s*([^)]+)\s*\)")


def _pistes(declaration, disponible, taille_police):
    """Traduit `grid-template-*` en liste de pistes."""
    declaration = (declaration or "").strip()
    if not declaration or declaration in ("none", "auto"):
        return []

    # `repeat(3, 1fr)` devient `1fr 1fr 1fr`. `auto-fill` sans largeur de piste
    # calculable est ramene a quatre, faute de mieux : mieux vaut une grille de
    # quatre colonnes qu'une colonne unique.
    def developpe(m):
        nombre = m.group(1)
        motif = m.group(2)
        if nombre in ("auto-fill", "auto-fit"):
            mesure = _mesure_unique(motif, disponible, taille_police)
            nombre = int(disponible // mesure) if mesure and mesure > 0 else 4
            nombre = max(1, min(nombre, 24))
        else:
            nombre = int(nombre)
        return " ".join([motif] * nombre)

    declaration = _REPEAT.sub(developpe, declaration)
    # `minmax(a, b)` : on retient le maximum, qui est ce que la piste occupe
    # quand la place ne manque pas.
    declaration = _MINMAX.sub(lambda m: m.group(2).strip(), declaration)

    pistes = []
    for morceau in declaration.split():
        pistes.append(_piste(morceau, disponible, taille_police))
    return pistes


def _piste(morceau, disponible, taille_police):
    morceau = morceau.strip().lower()

    # `minmax(a, b)` : un plancher et un plafond. C'est la forme la plus
    # courante d'une grille reelle — `minmax(200px, 1fr)` dit « au moins deux
    # cents pixels, sinon partage-toi le reste » — et la traiter comme une
    # piste `auto` faisait perdre les deux bornes a la fois.
    correspondance = _MINMAX.fullmatch(morceau)
    if correspondance is not None:
        bas = _piste(correspondance.group(1), disponible, taille_police)
        haut = _piste(correspondance.group(2), disponible, taille_police)
        piste = Piste(haut.type, haut.valeur,
                      minimum=bas.valeur if bas.type == "fixe" else 0.0)
        if bas.type in ("min-contenu", "max-contenu", "auto"):
            piste.type = haut.type
            piste.minimum = 0.0
            # Le plancher vient du contenu : on le retient pour que la mesure
            # sache lequel appliquer.
            piste.plafond = None
            piste.valeur = haut.valeur
        if haut.type == "fixe":
            piste.plafond = haut.valeur
        return piste

    if morceau.endswith("fr"):
        try:
            return Piste("fraction", float(morceau[:-2] or 1))
        except ValueError:
            return Piste("fraction", 1.0)
    # `min-content` et `max-content` ne sont pas `auto` : l'une prend la plus
    # petite largeur ou le contenu tient, l'autre celle qu'il faudrait pour que
    # rien ne revienne a la ligne. Les confondre — ce qui etait le cas —
    # revenait a ignorer les deux.
    if morceau == "min-content":
        return Piste("min-contenu", 0.0)
    if morceau == "max-content":
        return Piste("max-contenu", 0.0)
    if morceau == "auto":
        return Piste("auto", 0.0)
    mesure = css.longueur(morceau, disponible, taille_police)
    if mesure is None:
        return Piste("auto", 0.0)
    return Piste("fixe", mesure)


def _mesure_unique(motif, disponible, taille_police):
    piste = _piste(_MINMAX.sub(lambda m: m.group(2).strip(), motif).split()[0],
                   disponible, taille_police)
    return piste.valeur if piste.type == "fixe" else None


def _mesure_colonnes(colonnes, disponible, espace):
    """Attribue sa largeur a chaque colonne.

    ## Ce qui a change

    Les pistes `auto` se partageaient la place restante **a parts egales**,
    faute de savoir ce qu'elles portaient. Le commentaire d'alors le disait
    franchement : « la norme leur donne la taille de leur contenu, que la mise
    en page n'a pas encore ». Elle l'a maintenant — `intrinseque` la calcule
    sans rien poser — et les pistes la recoivent dans `min_contenu` et
    `max_contenu` avant d'arriver ici.

    L'ordre est celui de la norme, simplifie :

      1. les pistes fixes prennent leur valeur ;
      2. `min-content` et `max-content` prennent exactement ce que leur nom
         dit ;
      3. les pistes `auto` prennent leur `max-content`, bornees a la place
         disponible — c'est le comportement d'une grille reelle : une colonne
         `auto` s'ajuste a sa colonne la plus large, elle ne prend pas le tiers
         de la page parce qu'elles sont trois ;
      4. ce qui reste va aux `fr`, au prorata ;
      5. si les demandes depassent la place, tout ce qui n'est pas fixe est
         reduit proportionnellement, sans jamais descendre sous `min-content` —
         une colonne plus etroite que son mot le plus long deborde.
    """
    total_espace = espace * max(0, len(colonnes) - 1)
    reste = max(0.0, disponible - total_espace)

    # 1 et 2 : ce que chaque piste demande, avant partage.
    for piste in colonnes:
        if piste.type == "fixe":
            piste.taille = piste.valeur
        elif piste.type == "min-contenu":
            piste.taille = piste.min_contenu
        elif piste.type == "max-contenu":
            piste.taille = piste.max_contenu
        elif piste.type == "auto":
            piste.taille = max(piste.min_contenu, piste.max_contenu)
        else:
            piste.taille = 0.0
        if piste.minimum:
            piste.taille = max(piste.taille, piste.minimum)
        if piste.plafond is not None:
            piste.taille = min(piste.taille, piste.plafond)

    rigides = sum(p.taille for p in colonnes if p.type != "fraction")
    fractions = sum(p.valeur for p in colonnes if p.type == "fraction")

    # 4 : les `fr` se partagent ce qui reste. Chacune a droit au moins a son
    # contenu minimal — une piste `1fr` dans une grille pleine ne disparait pas.
    if fractions > 0:
        libre = max(0.0, reste - rigides)
        for piste in colonnes:
            if piste.type != "fraction":
                continue
            part = libre * piste.valeur / fractions
            piste.taille = max(part, piste.min_contenu, piste.minimum or 0.0)
            if piste.plafond is not None:
                piste.taille = min(piste.taille, piste.plafond)

    # 5 : le depassement. On reduit ce qui peut l'etre, jamais sous le
    # `min-content` : une colonne plus etroite que son mot le plus long
    # deborde, et deborder est pire que serrer.
    demande = sum(p.taille for p in colonnes)
    if demande > reste and demande > 0:
        reductible = sum(max(0.0, p.taille - p.min_contenu)
                         for p in colonnes if p.type != "fixe")
        exces = demande - reste
        if reductible > 0:
            facteur = min(1.0, exces / reductible)
            for piste in colonnes:
                if piste.type == "fixe":
                    continue
                marge = max(0.0, piste.taille - piste.min_contenu)
                piste.taille -= marge * facteur


def contributions(colonnes, enfants, placements, mesureur, styles):
    """Reporte sur chaque colonne ce que ses articles demandent.

    Un article qui s'etend sur plusieurs colonnes ne dicte la taille d'aucune :
    la norme repartit sa demande, et le faire naivement — l'imputer entiere a
    la premiere — donnerait une colonne enorme a cote de colonnes vides. On
    l'ignore donc pour le dimensionnement, ce qui est la simplification la moins
    fausse tant que le cas reste rare.
    """
    if mesureur is None:
        return
    for enfant, (colonne, _ligne, etendue, _el) in zip(enfants, placements):
        if etendue != 1 or colonne >= len(colonnes):
            continue
        element = getattr(enfant, "element", None)
        if element is None:
            continue
        style = getattr(enfant, "style", None) or (styles or {}).get(id(element))
        if style is None:
            continue
        try:
            tailles = mesureur.tailles(element, style)
        except Exception:  # noqa: BLE001 — mesurer ne doit pas empecher de poser
            continue
        piste = colonnes[colonne]
        piste.min_contenu = max(piste.min_contenu, tailles.min_contenu)
        piste.max_contenu = max(piste.max_contenu, tailles.max_contenu)


# --- Placement ----------------------------------------------------------------

def zones(declaration):
    """`grid-template-areas` en `(nom -> (colonne, ligne, etendue_c, etendue_l), colonnes)`.

    Une zone nommee sur plusieurs cases devient un rectangle : son coin haut
    gauche et son etendue. Le point `.` marque une case vide.
    """
    rangees = []
    for guillemets, apostrophes in re.findall(r'"([^"]*)"|\'([^\']*)\'',
                                              declaration or ""):
        noms = (guillemets or apostrophes).split()
        if noms:
            rangees.append(noms)
    if not rangees:
        return {}, 0

    colonnes = max(len(r) for r in rangees)
    boites = {}
    for index_ligne, rangee in enumerate(rangees):
        for index_colonne, nom in enumerate(rangee):
            if nom == ".":
                continue
            if nom in boites:
                c0, l0, ec, el = boites[nom]
                droite = max(c0 + ec, index_colonne + 1)
                bas = max(l0 + el, index_ligne + 1)
                c0, l0 = min(c0, index_colonne), min(l0, index_ligne)
                boites[nom] = (c0, l0, droite - c0, bas - l0)
            else:
                boites[nom] = (index_colonne, index_ligne, 1, 1)
    return boites, colonnes


def _place(enfants, nombre_colonnes, zones_nommees=None):
    """Rend, pour chaque enfant, `(colonne, ligne, etendue_colonne, etendue_ligne)`."""
    placements = []
    colonne_courante = 0
    ligne_courante = 0

    for enfant in enfants:
        style = enfant.style
        # Une zone nommee decide de tout : place et etendue, sur les deux axes.
        nom = (style.get("grid-area") or "").strip()
        if zones_nommees and nom in zones_nommees:
            placements.append(zones_nommees[nom])
            continue
        colonne, etendue_colonne = _position(style, "grid-column", nombre_colonnes)
        ligne, etendue_ligne = _position(style, "grid-row", 0)

        if colonne is None:
            # Placement automatique : a la suite, en passant a la ligne quand la
            # rangee est pleine.
            if colonne_courante + etendue_colonne > nombre_colonnes:
                colonne_courante = 0
                ligne_courante += 1
            colonne = colonne_courante
            colonne_courante += etendue_colonne
        else:
            colonne_courante = colonne + etendue_colonne

        if ligne is None:
            ligne = ligne_courante

        placements.append((colonne, ligne, etendue_colonne, etendue_ligne))

    return placements


def _position(style, prefixe, nombre_colonnes):
    """`(index de depart ou None, etendue)` pour `grid-column`/`grid-row`."""
    debut = (style.get("%s-start" % prefixe) or "").strip()
    fin = (style.get("%s-end" % prefixe) or "").strip()

    etendue = 1
    index = None

    if debut.startswith("span"):
        etendue = _entier(debut[4:], 1)
    elif debut and debut != "auto":
        valeur = _entier(debut, 0)
        if valeur > 0:
            index = valeur - 1
        elif valeur < 0 and nombre_colonnes:
            index = max(0, nombre_colonnes + valeur)

    if fin.startswith("span"):
        etendue = _entier(fin[4:], 1)
    elif fin and fin != "auto" and index is not None:
        valeur = _entier(fin, 0)
        if valeur > 0:
            etendue = max(1, valeur - 1 - index)

    return index, max(1, etendue)


def _entier(texte, defaut):
    try:
        return int(str(texte).strip())
    except (TypeError, ValueError):
        return defaut


# --- Geometrie ----------------------------------------------------------------

def _debut(pistes, index, espace):
    return sum(p.taille for p in pistes[:index]) + espace * index


def _debut_lignes(hauteurs, index, espace):
    return sum(hauteurs[:index]) + espace * index


def _somme_hauteurs(hauteurs, index, etendue, espace):
    fin = min(len(hauteurs), index + etendue)
    if index >= len(hauteurs):
        return 0.0
    return sum(hauteurs[index:fin]) + espace * max(0, fin - index - 1)


def _somme(pistes, index, etendue, espace):
    fin = min(len(pistes), index + etendue)
    if index >= len(pistes):
        return 0.0
    largeur = sum(p.taille for p in pistes[index:fin])
    return largeur + espace * max(0, fin - index - 1)


def _longueur(style, propriete, reference, taille_police):
    mesure = css.longueur(style.get(propriete, "0"), reference, taille_police)
    return mesure if mesure is not None else 0.0
