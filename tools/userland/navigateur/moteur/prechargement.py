"""Prechargement parallele des sous-ressources.

Une page cite des images, des feuilles de style et des scripts. Chacun est un
aller-retour reseau, et jusqu'ici ils partaient l'un apres l'autre : la
deuxieme image attendait la premiere, la troisieme les deux precedentes. Sur
une page de vingt images et un lien qui met cinquante millisecondes a repondre,
cela fait une seconde d'attente pure, sans qu'un seul octet supplementaire ne
transite.

Le prechargement lit l'arbre des la fin de l'analyse, releve les adresses
citees et les demande **ensemble** sur quelques fils. La reserve de connexions
de `reseau` fait le reste : les fils se partagent les connexions deja ouvertes
vers le meme hote.

## Pourquoi peu de fils

Quatre. Un navigateur de bureau en ouvre six par hote, mais il ne tourne pas
sur une machine qui emule son processeur. Ici chaque fil supplementaire coute un
changement de contexte de plus dans l'ordonnanceur du noyau, et le gain
s'aplatit vite : l'essentiel vient du passage de un a deux.

## Ce que le prechargement ne fait pas

Il ne decode rien et n'analyse rien : il rapporte des octets et les depose. Le
decodage d'une image reste au fil principal, parce qu'il passe par Qt et que Qt
n'accepte d'etre touche que de la. C'est aussi ce qui le rend sans danger — il
n'a aucun effet observable en dehors de la reserve de reponses.
"""

import urllib.parse

from . import reseau
from .html import Element

# Fils de telechargement. Voir l'en-tete pour le choix du nombre.
FILS = 4

# Au-dela, on ne precharge plus : une page qui cite trois cents images n'a pas
# besoin qu'on les demande toutes avant d'afficher son premier paragraphe.
RESSOURCES_MAX = 48


def adresses(racine, base):
    """Les adresses des sous-ressources citees par la page, sans doublon.

    L'ordre est celui du document : les premieres citees sont le plus
    probablement les premieres visibles, ce sont donc elles qui partent en
    premier.
    """
    vues = set()
    liste = []
    for nœud in racine.parcours():
        if not isinstance(nœud, Element):
            continue
        source = _adresse_de(nœud)
        if not source:
            continue
        source = source.strip()
        # `data:` porte deja ses octets ; `about:` et `javascript:` ne sont pas
        # des ressources.
        if not source or source.startswith(("data:", "about:", "javascript:", "#")):
            continue
        url = urllib.parse.urljoin(base or "", source)
        if not url.startswith(("http://", "https://")):
            continue
        if url in vues:
            continue
        vues.add(url)
        liste.append(url)
        if len(liste) >= RESSOURCES_MAX:
            break
    return liste


def _adresse_de(element):
    balise = element.balise
    if balise in ("img", "script"):
        return element.attributs.get("src")
    if balise == "link":
        rel = element.attributs.get("rel", "").lower().split()
        if "stylesheet" in rel:
            return element.attributs.get("href")
    return None


def precharge(racine, base, journal=None):
    """Demande en parallele les sous-ressources de la page.

    Rend le nombre d'adresses rapportees. Ne leve jamais : une ressource qui
    manque n'est pas une panne, et le chargement ordinaire la redemandera au
    moment ou elle sera reellement necessaire.
    """
    liste = adresses(racine, base)
    if not liste:
        return 0

    try:
        from concurrent.futures import ThreadPoolExecutor
    except ImportError:  # interprete sans fils : le chargement reste paresseux
        return 0

    obtenues = []

    def demande(url):
        try:
            reponse = reseau.charge(url, brut=True, document=base,
                                    destination="subresource")
        except Exception:  # noqa: BLE001
            return
        if reponse.code and reponse.code < 400:
            reseau.depose(url, reponse)
            obtenues.append(url)

    try:
        with ThreadPoolExecutor(max_workers=min(FILS, len(liste))) as fils:
            list(fils.map(demande, liste))
    except Exception as e:  # noqa: BLE001
        if journal:
            journal("warn", "prechargement interrompu : %s" % e)
        return len(obtenues)

    if journal:
        journal("info", "%d sous-ressources prechargees sur %d citees"
                % (len(obtenues), len(liste)))
    return len(obtenues)
