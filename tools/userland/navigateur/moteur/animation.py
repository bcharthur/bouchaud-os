"""Animations et transitions : ce qui fait qu'une page bouge.

Deux mecanismes, une seule machinerie. Une **animation** joue un gabarit
`@keyframes` du seul fait qu'une regle la nomme. Une **transition** part
d'elle-meme quand une valeur change — au survol, presque toujours. Les deux
finissent au meme endroit : interpoler entre deux valeurs CSS selon une
fraction de temps, et poser le resultat dans le style calcule.

## Ou cela s'accroche

`css.applique` termine par un passage chez l'animateur, s'il y en a un. Le
moteur d'animation n'est donc consulte qu'une fois par element et par mise en
page, apres la cascade — c'est-a-dire au seul moment ou l'on connait a la fois
ce que la feuille demande et ce que l'element affichait avant.

L'animateur appartient au [`Document`] : une nouvelle page en recoit un neuf,
et l'etat de l'ancienne — les depart d'animation, les transitions en cours —
s'en va avec elle.

## Ce qui est interpole

Les longueurs et les nombres (en gardant leur unite), les couleurs (canal par
canal, alpha compris) et les transformations (matrice par matrice). Tout le
reste bascule a mi-parcours, comme le veut la norme pour les valeurs qu'on ne
sait pas melanger.

## Ce qui ne l'est pas

Les etapes intermediaires d'un `@keyframes` servent de bornes, mais une
propriete absente d'une etape n'est pas cherchee plus loin que les deux etapes
qui encadrent l'instant courant. Les animations composees (`animation-composition`)
et les `@keyframes` a rythme propre par etape ne sont pas rendus.
"""

from . import css

# Une animation ou une transition qui ne dure rien ne s'anime pas : elle saute
# a sa valeur finale. C'est aussi ce qui evite une division par zero.
_INFIME = 0.0001


# --- Rythmes ------------------------------------------------------------------

# Les courbes nommees de CSS, en points de controle de Bezier cubique.
_COURBES = {
    "linear": None,
    "ease": (0.25, 0.1, 0.25, 1.0),
    "ease-in": (0.42, 0.0, 1.0, 1.0),
    "ease-out": (0.0, 0.0, 0.58, 1.0),
    "ease-in-out": (0.42, 0.0, 0.58, 1.0),
    "step-start": ("pas", 1, "start"),
    "step-end": ("pas", 1, "end"),
}


def rythme(declaration, fraction):
    """Deforme une fraction de temps par la fonction de rythme demandee."""
    if fraction <= 0.0:
        return 0.0
    if fraction >= 1.0:
        return 1.0
    nom = (declaration or "ease").strip().lower()

    if nom.startswith("cubic-bezier("):
        parties = css.separe(nom[len("cubic-bezier("):-1] if nom.endswith(")")
                             else nom[len("cubic-bezier("):])
        try:
            points = tuple(float(p) for p in parties[:4])
        except ValueError:
            return fraction
        return _bezier(points, fraction) if len(points) == 4 else fraction

    if nom.startswith("steps("):
        parties = css.separe(nom[len("steps("):-1] if nom.endswith(")")
                             else nom[len("steps("):])
        try:
            nombre = max(1, int(float(parties[0])))
        except (ValueError, IndexError):
            return fraction
        position = parties[1].strip() if len(parties) > 1 else "end"
        return _pas(nombre, position, fraction)

    courbe = _COURBES.get(nom, _COURBES["ease"])
    if courbe is None:
        return fraction
    if isinstance(courbe, tuple) and courbe and courbe[0] == "pas":
        return _pas(courbe[1], courbe[2], fraction)
    return _bezier(courbe, fraction)


def _pas(nombre, position, fraction):
    if position in ("start", "jump-start"):
        return min(1.0, (int(fraction * nombre) + 1) / float(nombre))
    return int(fraction * nombre) / float(nombre)


def _bezier(points, x):
    """Ordonnee de la courbe de Bezier pour cette abscisse.

    L'abscisse est le temps, l'ordonnee l'avancement. La courbe n'est pas une
    fonction de `x` sous forme close : on cherche le parametre par dichotomie,
    ce qui converge en une vingtaine de tours pour une precision tres au-dela
    de ce qu'un pixel demande.
    """
    x1, y1, x2, y2 = points

    def sur(axe_a, axe_b, t):
        u = 1.0 - t
        return 3 * u * u * t * axe_a + 3 * u * t * t * axe_b + t * t * t

    bas, haut = 0.0, 1.0
    for _ in range(24):
        milieu = (bas + haut) / 2.0
        if sur(x1, x2, milieu) < x:
            bas = milieu
        else:
            haut = milieu
    return sur(y1, y2, (bas + haut) / 2.0)


# --- Interpolation ------------------------------------------------------------

_UNITES = ("px", "rem", "em", "%", "vw", "vh", "vmin", "vmax", "pt", "cm", "mm",
           "in", "pc", "ex", "ch", "deg", "s", "ms")


def melange(depart, arrivee, fraction, propriete=""):
    """Une valeur CSS entre deux autres. Bascule a mi-parcours si indissociable."""
    if depart is None or arrivee is None:
        return arrivee if fraction >= 0.5 else depart
    if depart == arrivee:
        return depart
    if fraction <= 0.0:
        return depart
    if fraction >= 1.0:
        return arrivee

    if propriete == "transform" or "translate" in str(depart) or "matrix" in str(depart):
        melangee = _transformations(depart, arrivee, fraction)
        if melangee is not None:
            return melangee

    couleur_depart = css.couleur(depart)
    couleur_arrivee = css.couleur(arrivee)
    if couleur_depart is not None and couleur_arrivee is not None:
        return _couleurs(couleur_depart, couleur_arrivee, fraction)
    # `transparent` rend `None` : on melange alors vers la meme teinte, alpha nul.
    if couleur_depart is not None and _est_transparent(arrivee):
        return _couleurs(couleur_depart, couleur_depart & 0x00FFFFFF, fraction)
    if couleur_arrivee is not None and _est_transparent(depart):
        return _couleurs(couleur_arrivee & 0x00FFFFFF, couleur_arrivee, fraction)

    jetons_depart = str(depart).split()
    jetons_arrivee = str(arrivee).split()
    if len(jetons_depart) == len(jetons_arrivee) and jetons_depart:
        melanges = []
        for un, autre in zip(jetons_depart, jetons_arrivee):
            valeur = _nombres(un, autre, fraction)
            if valeur is None:
                return arrivee if fraction >= 0.5 else depart
            melanges.append(valeur)
        return " ".join(melanges)

    return arrivee if fraction >= 0.5 else depart


def _est_transparent(valeur):
    return str(valeur).strip().lower() in ("transparent", "none")


def _nombres(depart, arrivee, fraction):
    """Deux longueurs de meme unite, melangees. `None` si ce n'en sont pas."""
    valeur_depart, unite_depart = _decoupe_nombre(depart)
    valeur_arrivee, unite_arrivee = _decoupe_nombre(arrivee)
    if valeur_depart is None or valeur_arrivee is None:
        return None
    # Melanger des `px` avec des `%` demanderait de connaitre la reference ;
    # une unite nulle s'accorde en revanche a n'importe laquelle (`0` = `0px`).
    if unite_depart != unite_arrivee:
        if valeur_depart == 0.0:
            unite_depart = unite_arrivee
        elif valeur_arrivee == 0.0:
            unite_arrivee = unite_depart
        else:
            return None
    resultat = valeur_depart + (valeur_arrivee - valeur_depart) * fraction
    return _ecrit(resultat, unite_depart)


def _decoupe_nombre(texte):
    texte = str(texte).strip()
    for unite in sorted(_UNITES, key=len, reverse=True):
        if texte.endswith(unite):
            try:
                return float(texte[:-len(unite)]), unite
            except ValueError:
                return None, ""
    try:
        return float(texte), ""
    except ValueError:
        return None, ""


def _ecrit(valeur, unite):
    if abs(valeur - round(valeur)) < 0.001:
        return "%d%s" % (int(round(valeur)), unite)
    return "%.3f%s" % (valeur, unite)


def _couleurs(depart, arrivee, fraction):
    canaux = []
    for decalage in (24, 16, 8, 0):
        un = (depart >> decalage) & 0xFF
        autre = (arrivee >> decalage) & 0xFF
        canaux.append(int(round(un + (autre - un) * fraction)))
    alpha, rouge, vert, bleu = canaux
    return "rgba(%d, %d, %d, %.3f)" % (rouge, vert, bleu, alpha / 255.0)


def _transformations(depart, arrivee, fraction):
    """Deux `transform` melanges terme a terme sur leur matrice.

    Interpoler les matrices n'est pas interpoler les rotations : une rotation
    d'un demi-tour passe par l'aplatissement plutot que par le quart de tour.
    C'est le compromis usuel d'un moteur qui ne decompose pas, et il est exact
    pour les translations et les homotheties — l'immense majorite des cas.
    """
    matrice_depart = css.transformation(depart) or (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
    matrice_arrivee = css.transformation(arrivee) or (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
    if css.transformation(depart) is None and css.transformation(arrivee) is None:
        return None
    melangee = tuple(un + (autre - un) * fraction
                     for un, autre in zip(matrice_depart, matrice_arrivee))
    return "matrix(%s)" % ", ".join("%.4f" % v for v in melangee)


# --- L'animateur --------------------------------------------------------------

class Animateur:
    """Tient le temps, les gabarits, et l'etat de ce qui bouge."""

    def __init__(self):
        self.keyframes = {}
        self.temps = 0.0
        self._debuts = {}         # cle -> (nom, instant de depart)
        self._precedents = {}     # cle -> style calcule au tour precedent
        self._transitions = {}    # cle -> {propriete: (depart, de, vers, duree, retard, rythme)}
        self._en_cours = False

    # -- Horloge ---------------------------------------------------------------

    def pose_temps(self, millisecondes):
        self.temps = float(millisecondes)

    def anime(self):
        """Quelque chose bouge-t-il ? Le navigateur s'en sert pour redessiner."""
        return self._en_cours

    def oublie(self):
        """Repart de zero : nouvelle page, ou feuilles rechargees."""
        self._debuts.clear()
        self._precedents.clear()
        self._transitions.clear()
        self._en_cours = False

    def nouveau_tour(self):
        """A appeler avant une mise en page : remet le drapeau d'activite."""
        self._en_cours = False

    # -- Le point d'entree de la cascade ---------------------------------------

    def applique(self, element, style, pseudo=None):
        """Superpose au style calcule ce que les animations en font."""
        cle = (id(element), pseudo)
        style = self._anime(cle, style)
        style = self._transitionne(cle, style)
        self._precedents[cle] = style
        return style

    # -- Animations (`@keyframes`) ---------------------------------------------

    def _anime(self, cle, style):
        nom = (style.get("animation-name") or "").strip()
        if not nom or nom == "none":
            self._debuts.pop(cle, None)
            return style
        etapes = self.keyframes.get(nom)
        if not etapes:
            return style

        duree = _duree(style.get("animation-duration"), 0.0)
        if duree <= _INFIME:
            return style
        retard = _duree(style.get("animation-delay"), 0.0)

        depart = self._debuts.get(cle)
        if depart is None or depart[0] != nom:
            depart = (nom, self.temps)
            self._debuts[cle] = depart
        ecoule = self.temps - depart[1] - retard
        if ecoule < 0.0:
            # Pendant le retard, `backwards` montre deja la premiere etape.
            self._en_cours = True
            remplissage = (style.get("animation-fill-mode") or "none").strip()
            if remplissage in ("backwards", "both"):
                return _superpose(style, _etat(etapes, 0.0))
            return style

        tours = style.get("animation-iteration-count", "1")
        infini = str(tours).strip().lower() == "infinite"
        try:
            nombre_tours = float(tours)
        except (TypeError, ValueError):
            nombre_tours = 1.0

        tour = ecoule / duree
        if not infini and tour >= nombre_tours:
            # Animation finie : `forwards` retient la derniere image.
            remplissage = (style.get("animation-fill-mode") or "none").strip()
            if remplissage in ("forwards", "both"):
                fin = _sens(style, int(nombre_tours - _INFIME), 1.0)
                return _superpose(style, _etat(etapes, fin))
            return style

        self._en_cours = True
        index_tour = int(tour)
        fraction = _sens(style, index_tour, tour - index_tour)
        fraction = rythme(style.get("animation-timing-function"), fraction)
        return _superpose(style, _etat(etapes, fraction))

    # -- Transitions -----------------------------------------------------------

    def _transitionne(self, cle, style):
        proprietes = _proprietes_transition(style)
        if not proprietes:
            self._transitions.pop(cle, None)
            return style

        precedent = self._precedents.get(cle)
        en_cours = self._transitions.setdefault(cle, {})

        for propriete, (duree, retard, courbe) in proprietes.items():
            vise = style.get(propriete)
            if duree <= _INFIME:
                continue
            if precedent is not None:
                avant = precedent.get(propriete)
                active = en_cours.get(propriete)
                # La cible a change : une nouvelle transition part de la valeur
                # actuellement affichee, pas de celle que la regle demandait —
                # sans quoi un survol interrompu ferait un saut.
                if active is None:
                    if avant != vise and avant is not None:
                        en_cours[propriete] = (self.temps, avant, vise, duree,
                                               retard, courbe)
                elif active[2] != vise:
                    affichee = _valeur_courante(active, self.temps, propriete)
                    en_cours[propriete] = (self.temps, affichee, vise, duree,
                                           retard, courbe)

        if not en_cours:
            return style

        resultat = dict(style)
        finies = []
        for propriete, active in en_cours.items():
            debut, de, vers, duree, retard, courbe = active
            ecoule = self.temps - debut - retard
            if ecoule < 0.0:
                resultat[propriete] = de
                self._en_cours = True
                continue
            if ecoule >= duree:
                finies.append(propriete)
                continue
            self._en_cours = True
            resultat[propriete] = melange(de, vers, rythme(courbe, ecoule / duree),
                                          propriete)
        for propriete in finies:
            del en_cours[propriete]
        return resultat


def _valeur_courante(active, temps, propriete):
    debut, de, vers, duree, retard, courbe = active
    ecoule = temps - debut - retard
    if ecoule <= 0.0:
        return de
    if ecoule >= duree:
        return vers
    return melange(de, vers, rythme(courbe, ecoule / duree), propriete)


def _sens(style, index_tour, fraction):
    """Applique `animation-direction` a la fraction du tour courant."""
    direction = (style.get("animation-direction") or "normal").strip().lower()
    if direction == "reverse":
        return 1.0 - fraction
    if direction == "alternate":
        return 1.0 - fraction if index_tour % 2 else fraction
    if direction == "alternate-reverse":
        return fraction if index_tour % 2 else 1.0 - fraction
    return fraction


def _etat(etapes, fraction):
    """Les declarations d'un gabarit a cette fraction, les bornes melangees."""
    avant = etapes[0]
    apres = etapes[-1]
    for arret, declarations in etapes:
        if arret <= fraction:
            avant = (arret, declarations)
        else:
            apres = (arret, declarations)
            break
    else:
        apres = avant

    portee = apres[0] - avant[0]
    locale = 0.0 if portee <= 0 else (fraction - avant[0]) / portee

    resultat = dict(avant[1])
    for propriete, valeur in apres[1].items():
        resultat[propriete] = melange(avant[1].get(propriete), valeur, locale,
                                      propriete)
    return resultat


def _superpose(style, ajouts):
    resultat = dict(style)
    resultat.update(ajouts)
    return resultat


def _duree(valeur, defaut):
    """Une duree CSS en millisecondes."""
    if valeur is None:
        return defaut
    texte = str(valeur).strip().lower()
    try:
        if texte.endswith("ms"):
            return float(texte[:-2])
        if texte.endswith("s"):
            return float(texte[:-1]) * 1000.0
        return float(texte) * 1000.0
    except ValueError:
        return defaut


def _proprietes_transition(style):
    """`propriete -> (duree, retard, rythme)` d'apres les `transition-*`."""
    noms = [n.strip() for n in str(style.get("transition-property") or "").split(",")
            if n.strip()]
    if not noms:
        return {}
    durees = [d.strip() for d in str(style.get("transition-duration") or "").split(",")]
    retards = [d.strip() for d in str(style.get("transition-delay") or "0s").split(",")]
    courbes = css.separe(str(style.get("transition-timing-function") or "ease"))

    resultat = {}
    for index, nom in enumerate(noms):
        duree = _duree(durees[index % len(durees)] if durees else None, 0.0)
        retard = _duree(retards[index % len(retards)] if retards else None, 0.0)
        courbe = courbes[index % len(courbes)] if courbes else "ease"
        if nom in ("all", "none"):
            if nom == "all":
                # `all` s'applique a ce qui sait se melanger : les inventorier
                # tous couterait plus que de nommer ceux qui bougent vraiment.
                for propriete in _ANIMABLES:
                    resultat[propriete] = (duree, retard, courbe)
            continue
        resultat[nom] = (duree, retard, courbe)
    return resultat


# Les proprietes que `transition: all` couvre. La norme dit « toutes celles qui
# s'animent » ; les enumerer evite de comparer, a chaque element et a chaque
# tour, un dictionnaire de style entier contre le precedent.
_ANIMABLES = (
    "color", "background-color", "opacity", "transform",
    "border-top-color", "border-right-color", "border-bottom-color",
    "border-left-color", "border-radius",
    "width", "height", "min-width", "min-height", "max-width", "max-height",
    "margin-top", "margin-right", "margin-bottom", "margin-left",
    "padding-top", "padding-right", "padding-bottom", "padding-left",
    "top", "right", "bottom", "left", "font-size", "box-shadow",
    "border-top-width", "border-right-width", "border-bottom-width",
    "border-left-width",
)
