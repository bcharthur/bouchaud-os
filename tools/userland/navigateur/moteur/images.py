"""Images : du `src` d'une balise a des pixels decodes.

Le decodage lui-meme revient a l'hote (`bo.image`), donc a Qt, qui sait lire
PNG, JPEG, GIF et BMP. Ce module s'occupe du reste : resoudre l'adresse, la
telecharger une seule fois, retenir le resultat, et refuser poliment ce qui est
trop gros.

## Pourquoi un cache

Une page cite souvent la meme image plusieurs fois — puces, icones, avatars — et
la mise en page est refaite a chaque changement de taille de fenetre comme a
chaque modification du DOM par le JavaScript. Sans cache, chacune de ces passes
retelechargerait tout.

Le cache est volontairement porte par le module et non par le document : deux
pages du meme site partagent leurs images, ce qui est exactement le
comportement attendu d'un navigateur.
"""

import base64
import urllib.parse

import bo

from . import reseau

# Au-dela, on renonce : decoder une image de plusieurs dizaines de mega-octets
# sous emulation coute plus que ce qu'elle apporte.
TAILLE_MAX = 8 * 1024 * 1024

# Nombre d'images distinctes retenues. Passe ce seuil, le cache se vide plutot
# que de grandir sans fin sur une navigation longue.
ENTREES_MAX = 256

_cache = {}


def vide():
    """Oublie tout : appele quand la memoire compte plus que la vitesse."""
    _cache.clear()


def charge(base, source):
    """Rend `(identifiant, largeur, hauteur)` pour cette image, ou `None`.

    `None` couvre tous les echecs — adresse absente, reseau muet, format que
    l'hote ne sait pas lire — et l'appelant retombe alors sur le texte de
    remplacement, comme n'importe quel navigateur.
    """
    if not source:
        return None
    source = source.strip()
    if not source:
        return None

    url = source if source.startswith("data:") \
        else urllib.parse.urljoin(base or "", source)

    if url in _cache:
        return _cache[url]

    if len(_cache) >= ENTREES_MAX:
        _cache.clear()

    resultat = _decode(_octets(url))
    _cache[url] = resultat
    return resultat


def _octets(url):
    if url.startswith("data:"):
        return _octets_data(url)
    try:
        reponse = reseau.charge(url, brut=True)
    except Exception:  # noqa: BLE001 — une image cassee n'est pas une panne
        return None
    if reponse.code and reponse.code >= 400:
        return None
    donnees = reponse.octets
    if not donnees or len(donnees) > TAILLE_MAX:
        return None
    return donnees


def _octets_data(url):
    """`data:image/png;base64,…` — les petites icones voyagent souvent ainsi."""
    try:
        entete, charge_utile = url.split(",", 1)
    except ValueError:
        return None
    if ";base64" in entete:
        try:
            return base64.b64decode(charge_utile)
        except Exception:  # noqa: BLE001
            return None
    return urllib.parse.unquote_to_bytes(charge_utile)


def _decode(donnees):
    if not donnees:
        return None
    try:
        return bo.image(donnees)
    except Exception:  # noqa: BLE001
        return None


def dimensions(naturelle, attributs, style, largeur_disponible):
    """Taille d'affichage : ce que la page demande, a defaut la taille reelle.

    L'ordre est celui des navigateurs : `width`/`height` en CSS l'emportent sur
    les attributs de la balise, qui l'emportent sur les pixels de l'image. Quand
    une seule des deux dimensions est donnee, l'autre suit pour conserver les
    proportions.
    """
    from . import css

    _, naturelle_l, naturelle_h = naturelle
    if naturelle_l <= 0 or naturelle_h <= 0:
        return 0.0, 0.0
    rapport = naturelle_h / float(naturelle_l)

    largeur = _dimension(style.get("width"), attributs.get("width"),
                         largeur_disponible, css)
    hauteur = _dimension(style.get("height"), attributs.get("height"),
                         largeur_disponible, css)

    # `aspect-ratio` prend la place du rapport naturel quand la page le declare :
    # c'est ainsi que les sites recents reservent la place d'une image avant de
    # l'avoir recue, et ignorer la propriete faisait sauter la mise en page au
    # moment ou l'image arrivait.
    impose = proportion(style.get("aspect-ratio"))
    if impose is not None:
        rapport = impose

    if largeur is None and hauteur is None:
        if impose is not None:
            largeur = min(float(naturelle_l), largeur_disponible or naturelle_l)
            hauteur = largeur * rapport
        else:
            largeur, hauteur = float(naturelle_l), float(naturelle_h)
    elif hauteur is None:
        hauteur = largeur * rapport
    elif largeur is None:
        largeur = hauteur / rapport if rapport else float(naturelle_l)

    # Une image plus large que la colonne la fait deborder : on la ramene, en
    # gardant ses proportions.
    if largeur_disponible > 0 and largeur > largeur_disponible:
        hauteur *= largeur_disponible / largeur
        largeur = largeur_disponible

    return max(0.0, largeur), max(0.0, hauteur)


def _dimension(valeur_css, attribut, reference, css):
    if valeur_css and valeur_css != "auto":
        mesure = css.longueur(valeur_css, reference, 16.0)
        if mesure and mesure > 0:
            return float(mesure)
    if attribut:
        texte = str(attribut).strip().rstrip("px").strip()
        try:
            mesure = float(texte)
        except ValueError:
            return None
        if mesure > 0:
            return mesure
    return None


def proportion(valeur):
    """`aspect-ratio` en rapport hauteur/largeur. `None` si absent ou libre."""
    if not valeur:
        return None
    texte = str(valeur).strip().lower()
    if not texte or texte in ("auto", "none", "initial"):
        return None
    # `16 / 9`, `1.7778`, et la forme `auto 16/9` dont seul le rapport compte.
    texte = texte.replace("auto", "").strip()
    try:
        if "/" in texte:
            largeur, hauteur = texte.split("/", 1)
            largeur, hauteur = float(largeur), float(hauteur)
            return hauteur / largeur if largeur else None
        rapport = float(texte)
        return 1.0 / rapport if rapport else None
    except ValueError:
        return None


def ajuste(fit, boite_l, boite_h, naturelle_l, naturelle_h):
    """Place l'image dans sa boite selon `object-fit`.

    Rend `(dx, dy, largeur, hauteur, source)` : le decalage dans la boite, la
    taille peinte, et la portion de l'image a prendre — `None` pour tout.

    Sans cette propriete, toute image etait etiree a la taille de sa boite. Une
    vignette carree posee sur une carte large s'en trouvait deformee, ce qui se
    voit immediatement sur un visage.
    """
    fit = (fit or "fill").strip().lower()
    if boite_l <= 0 or boite_h <= 0 or naturelle_l <= 0 or naturelle_h <= 0:
        return (0.0, 0.0, boite_l, boite_h, None)
    if fit == "fill":
        return (0.0, 0.0, boite_l, boite_h, None)

    echelle_l = boite_l / float(naturelle_l)
    echelle_h = boite_h / float(naturelle_h)

    if fit == "none":
        echelle = 1.0
    elif fit == "scale-down":
        echelle = min(1.0, min(echelle_l, echelle_h))
    elif fit == "cover":
        echelle = max(echelle_l, echelle_h)
    else:  # contain
        echelle = min(echelle_l, echelle_h)

    peinte_l = naturelle_l * echelle
    peinte_h = naturelle_h * echelle

    if peinte_l <= boite_l + 0.01 and peinte_h <= boite_h + 0.01:
        # Elle tient : on la centre, et rien n'est rogne.
        return ((boite_l - peinte_l) / 2.0, (boite_h - peinte_h) / 2.0,
                peinte_l, peinte_h, None)

    # Elle depasse : on prend la portion centrale qui remplit exactement la
    # boite, plutot que de la deformer ou de la laisser sortir.
    source_l = min(float(naturelle_l), boite_l / echelle)
    source_h = min(float(naturelle_h), boite_h / echelle)
    return (0.0, 0.0, min(boite_l, peinte_l), min(boite_h, peinte_h),
            ((naturelle_l - source_l) / 2.0, (naturelle_h - source_h) / 2.0,
             source_l, source_h))
