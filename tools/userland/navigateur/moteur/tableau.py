"""Tableaux : des lignes et des colonnes qui s'alignent.

Les cellules etaient en ligne, comme n'importe quel `<span>` : elles se
suivaient sur une ligne de texte, sans colonne, sans remplissage, sans bordure.
`<th>Propriete</th><th>Etat</th>` s'affichait donc « ProprieteEtat », et rien
ne s'alignait d'une ligne a l'autre.

## L'algorithme

Le meme que celui des navigateurs, dans sa forme simple — celle qui suffit a un
tableau de documentation ou de donnees :

1. chaque cellule est posee une fois a vide pour connaitre l'etendue de son
   contenu ;
2. la largeur d'une colonne est la plus grande de ses cellules ;
3. si la somme depasse la place, les colonnes retrecissent au prorata, jamais
   en dessous de ce qu'il leur faut au minimum ;
4. les cellules sont reposees a leur colonne, et toutes les cellules d'une
   ligne recoivent la hauteur de la plus haute.

## Ce qui n'est pas rendu

`rowspan` — une cellule qui descend sur plusieurs lignes occupe la sienne.
`table-layout: fixed`, les groupes de colonnes (`<colgroup>`), et la separation
des bordures (`border-spacing`) : les bordures sont toujours jointes, ce que
`border-collapse: collapse` demande et que la plupart des feuilles posent.
"""

from . import css


def est_tableau(style, balise):
    return style.get("display", "") == "table" or balise == "table"


def dispose(boite, lignes, x, y, largeur_disponible, contexte, pose_enfant):
    """Pose les cellules en colonnes. Rend la hauteur consommee.

    `lignes` est une liste de listes de boites de cellules, deja construites :
    ce module ne parcourt pas le document, il ne fait que placer.
    """
    if not lignes:
        return 0.0

    nombre = max(sum(_etendue(cellule) for cellule in ligne) for ligne in lignes)
    if nombre <= 0:
        return 0.0

    demandes = _demandes(lignes, nombre, largeur_disponible, pose_enfant)
    largeurs = _repartit(demandes, largeur_disponible)

    hauteur_totale = 0.0
    for ligne in lignes:
        colonne = 0
        hauteur_ligne = 0.0
        for cellule in ligne:
            etendue = _etendue(cellule)
            largeur = sum(largeurs[colonne:colonne + etendue])
            decalage = sum(largeurs[:colonne])
            _, hauteur = pose_enfant(cellule, largeur, x + decalage,
                                     y + hauteur_totale, largeur)
            hauteur_ligne = max(hauteur_ligne, hauteur)
            colonne += etendue
        # Toutes les cellules d'une ligne ont la hauteur de la plus haute,
        # sinon leurs fonds et leurs bordures s'arreteraient a des hauteurs
        # differentes et la ligne paraitrait dechiree.
        for cellule in ligne:
            cellule.hauteur = hauteur_ligne
        hauteur_totale += hauteur_ligne
    return hauteur_totale


def _etendue(cellule):
    """Le `colspan` d'une cellule, au moins 1."""
    element = cellule.element
    brut = getattr(element, "attributs", {}).get("colspan", "1")
    try:
        return max(1, int(str(brut).strip()))
    except (TypeError, ValueError):
        return 1


def _demandes(lignes, nombre, largeur_disponible, pose_enfant):
    """Ce que chaque colonne reclame : la plus large de ses cellules."""
    demandes = [0.0] * nombre
    from .mise_en_page import etendue_contenu

    for ligne in lignes:
        colonne = 0
        for cellule in ligne:
            etendue = _etendue(cellule)
            pose_enfant(cellule, largeur_disponible, 0.0, 0.0, None)
            voulue = max(0.0, etendue_contenu(cellule))
            declaree = css.longueur(cellule.style.get("width", "auto"),
                                    largeur_disponible, 16.0)
            if declaree is not None and declaree > 0:
                voulue = max(voulue, declaree)
            if etendue == 1 and colonne < nombre:
                demandes[colonne] = max(demandes[colonne], voulue)
            else:
                # Une cellule qui s'etend sur plusieurs colonnes ne dicte la
                # largeur d'aucune : elle partage sa demande entre elles.
                part = voulue / float(etendue)
                for index in range(colonne, min(colonne + etendue, nombre)):
                    demandes[index] = max(demandes[index], part)
            colonne += etendue
    return demandes


def _repartit(demandes, largeur_disponible):
    """Ajuste les colonnes a la place reelle, au prorata de leur demande."""
    total = sum(demandes)
    if total <= 0:
        part = largeur_disponible / float(len(demandes) or 1)
        return [part] * len(demandes)
    if total <= largeur_disponible:
        # Il reste de la place : on la donne aux colonnes, au prorata, pour que
        # le tableau occupe sa largeur comme le fait un navigateur.
        surplus = largeur_disponible - total
        return [d + surplus * (d / total) for d in demandes]
    facteur = largeur_disponible / total
    return [d * facteur for d in demandes]
