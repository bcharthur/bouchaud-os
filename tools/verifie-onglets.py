#!/usr/bin/env python3
"""Verifie que le chrome sert l'onglet ACTIF, et lui seul.

LE DEFAUT QUE CE FICHIER GARDE FERME
------------------------------------
Le chrome connaissait la page 1, en dur. Chaque rappel vers le moteur capturait
`page_id = 1`, chaque hook de `PageClient` supposait que l'evenement venait de
la seule page qui existe, et chaque capture etait composee sans se demander
d'ou elle venait.

Avec plusieurs onglets, chacune de ces suppositions devient un defaut, et aucun
ne fait echouer un test :

  * un rappel qui vise une page figee agit sur un onglet que l'utilisateur ne
    regarde pas. Un clic recharge l'autre onglet ; Ctrl+F cherche dans l'autre
    document ;
  * un hook qui ne dit pas de quelle page il vient fait ecrire le titre du
    second onglet dans la barre d'adresse du premier ;
  * une capture composee sans verifier son onglet fait clignoter la page qu'on
    regarde avec celle d'a cote, des qu'un chargement d'arriere-plan se
    termine.

Le troisieme se voit. Les deux premiers se manifestent comme « le navigateur
fait n'importe quoi », une fois sur cinq, et personne ne sait par quel bout le
prendre.

LES REGLES
----------
1. Les rappels du chrome vers le moteur demandent l'onglet ACTIF. `page_id`
   n'est plus une valeur capturee mais un appel.

2. Les hooks qui remontent au chrome portent leur identifiant de page. Un hook
   qui n'en porte pas ne peut pas savoir si l'evenement concerne l'onglet
   affiche.

3. Le moteur cree la page, le chrome cree l'onglet -- et jamais l'inverse. Une
   fenetre surgissante passe par `page_did_request_new_web_view`.

4. La fermeture retire l'onglet AVANT `remove_page`. Apres, l'identifiant
   designerait un onglet dont la page n'existe plus.

5. Les identifiants ne sont jamais reutilises. Une capture partie avant une
   fermeture -- il y en a toujours une en vol -- serait sinon prise pour celle
   du nouvel onglet.

6. La bande se peint avec la barre d'outils, et le degat du chrome couvre les
   deux. Une bande peinte sans etre annoncee reste invisible ; annoncee sans
   etre peinte, elle montre ce qu'il y avait avant.

7. Les trois raccourcis existent : ouvrir, fermer, circuler.

Code de retour : 0 si les sept regles sont respectees.
"""

import re
import sys
from pathlib import Path

# Les scripts de portage delimitent leurs blocs de code par des triples
# apostrophes. Les ecrire litteralement ici obligerait a echapper chaque
# occurrence dans ce fichier-ci ; les composer une fois est plus lisible.
TRIPLE = "'" * 3

RACINE = Path(__file__).resolve().parent.parent
CHROME = RACINE / "tools" / "ladybird" / "chrome" / "BouchaudChrome.h"
M11 = RACINE / "tools" / "ladybird" / "prepare-m11-chrome.py"
V19 = RACINE / "tools" / "ladybird" / "prepare-v19-navigateur.py"
HOTE = RACINE / "tools" / "ladybird" / "prepare-full-browser-host.py"


def corps(source, signature):
    """Le corps qui suit `signature`, accolades equilibrees.

    Une DECLARATION anticipee -- `inline void f();` -- contient la meme chaine
    qu'une signature partielle de la definition. Ce qui distingue les deux est
    le point-virgule : une declaration en porte un AVANT la prochaine accolade.
    """
    debut = 0
    while True:
        trouve = source.find(signature, debut)
        if trouve < 0:
            return None
        ouvrante = source.find("{", trouve)
        if ouvrante < 0:
            return None
        point_virgule = source.find(";", trouve)
        if point_virgule < 0 or point_virgule > ouvrante:
            break
        debut = point_virgule + 1

    profondeur = 0
    for index in range(ouvrante, len(source)):
        if source[index] == "{":
            profondeur += 1
        elif source[index] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouvrante : index + 1]
    return None


def regle_rappels(m11, fautes):
    """1. Les rappels visent l'onglet actif."""
    if "constexpr u64 page_id = 1;" in m11:
        fautes.append(
            "prepare-m11-chrome.py : `page_id` est redevenu une constante. "
            "Chaque rappel agirait alors sur le premier onglet, quel que soit "
            "celui que l'utilisateur regarde -- un clic rechargerait l'autre."
        )
    if "BouchaudChrome::page_active()" not in m11:
        fautes.append(
            "prepare-m11-chrome.py : les rappels ne demandent plus quel onglet "
            "est actif."
        )
    # Un usage nu de `page_id` -- sans les parentheses -- serait la lambda
    # elle-meme la ou on attend un identifiant.
    #
    # La regle ne porte que sur le CORPS de `bouchaud_m11_start` : ailleurs
    # dans le script, `page_id` designe la variable du chemin M9, ou il n'y a
    # qu'une page et ou elle vaut bien 1.
    corps_m11 = re.search("m11_start = r" + TRIPLE + "(.*?)" + TRIPLE, m11, re.S)
    if not corps_m11:
        fautes.append(
            "prepare-m11-chrome.py : le corps de `bouchaud_m11_start` n'est "
            "plus une chaine brute delimitee par des triples apostrophes."
        )
        return
    for ligne in corps_m11.group(1).splitlines():
        nue = ligne.split("//", 1)[0]
        if "page_id" not in nue or "page_id()" in nue:
            continue
        # La capture d'une lambda et sa declaration nomment le rappel lui-meme.
        if "[this, page_id" in nue or "auto const page_id" in nue:
            continue
        # Le premier onglet et sa premiere URL portent l'identifiant reel.
        if "ajoute_onglet" in nue or "set_committed_url" in nue:
            continue
        fautes.append(
            "prepare-m11-chrome.py : `page_id` employe sans parentheses hors "
            "capture :\n           %s" % ligne.strip()
        )


def regle_hooks(chrome, m11, hote, fautes):
    """2. Les hooks portent leur identifiant de page."""
    for signature, quoi in (
        ("inline bool present(u64 page_id,", "une capture"),
        ("inline void set_committed_url(u64 page_id,", "une URL commitee"),
        ("inline void set_loading(u64 page_id,", "un etat de chargement"),
        ("inline void set_title(u64 page_id,", "un titre"),
    ):
        if signature not in chrome:
            fautes.append(
                "BouchaudChrome.h : %s arrive sans dire de quelle page elle "
                "vient. Le chrome l'appliquerait a l'onglet affiche, quel que "
                "soit celui qui a change." % quoi
            )

    for source, nom in ((m11, "prepare-m11-chrome.py"), (hote, "prepare-full-browser-host.py")):
        for appel in re.findall(
            r"BouchaudChrome::(set_committed_url|set_loading|set_title|present|present_complet)"
            r"\((m_id|page_id\(\)|page_id|initial_page_id|[^,)]*)",
            source,
        ):
            premier = appel[1].strip()
            if premier in ("m_id", "page_id", "initial_page_id", "page_id()"):
                continue
            fautes.append(
                "%s : `BouchaudChrome::%s` est appelee sans identifiant de "
                "page (premier argument : %r)." % (nom, appel[0], premier)
            )


def regle_creation(v19, fautes):
    """3 et 4."""
    # La substitution est reperee par son ETIQUETTE, et non par le nom du
    # hook : celui-ci apparait d'abord dans la docstring du script, ou une
    # fenetre de recherche ne contient evidemment pas le code.
    etiquette = v19.find('"nouvelle vue",')
    bloc = v19[max(0, etiquette - 3000) : etiquette] if etiquette >= 0 else ""
    if etiquette < 0 or "create_page(" not in bloc:
        fautes.append(
            "prepare-v19-navigateur.py : une fenetre surgissante ne cree plus "
            "de page. `target=_blank` ne ferait rien du tout."
        )
    elif "ajoute_onglet(" not in bloc:
        fautes.append(
            "prepare-v19-navigateur.py : la page creee par une fenetre "
            "surgissante n'apparait dans aucun onglet : elle vivrait sans que "
            "rien ne puisse l'atteindre."
        )

    fermeture = v19.rfind("page_did_close_top_level_traversable")
    if fermeture < 0 or "retire_onglet(" not in v19[fermeture : fermeture + 1500]:
        fautes.append(
            "prepare-v19-navigateur.py : la fermeture d'une page ne retire "
            "plus son onglet. La bande garderait une ligne dont la page "
            "n'existe plus."
        )
    else:
        extrait = v19[fermeture : fermeture + 1500]
        retire = extrait.find("retire_onglet(")
        arret = extrait.find("stop_presenting_to_client")
        if arret >= 0 and retire > arret:
            fautes.append(
                "prepare-v19-navigateur.py : l'onglet est retire APRES que la "
                "page a commence a disparaitre."
            )


def regle_identifiants(chrome, fautes):
    """5. Les identifiants ne sont jamais reutilises."""
    bloc = corps(chrome, "inline u64 prochaine_page()")
    if bloc is None:
        fautes.append("BouchaudChrome.h : `prochaine_page` a disparu.")
        return
    if "++" not in bloc:
        fautes.append(
            "BouchaudChrome.h : `prochaine_page` ne progresse plus. Deux "
            "onglets porteraient le meme identifiant."
        )
    for interdit in ("onglets.size()", "-", "="):
        if interdit == "=" and "prochaine_page++" in bloc:
            continue
        if interdit in ("onglets.size()", "-") and interdit in bloc:
            fautes.append(
                "BouchaudChrome.h : `prochaine_page` calcule son numero au "
                "lieu de l'incrementer. Un identifiant reutilise ferait "
                "prendre une capture en vol pour celle du nouvel onglet."
            )


def regle_peinture(chrome, fautes):
    """6. La bande se peint avec la barre, et le degat couvre les deux."""
    bloc = corps(chrome, "inline void draw_chrome(")
    if bloc is None or "draw_onglets(" not in bloc or "draw_toolbar(" not in bloc:
        fautes.append(
            "BouchaudChrome.h : `draw_chrome` ne peint plus les deux. Une "
            "bande peinte sans la barre -- ou l'inverse -- laisse a l'ecran "
            "celle qui n'a pas ete repeinte."
        )
    if "draw_toolbar(canvas);" in chrome.replace(
            corps(chrome, "inline void draw_chrome(") or "", ""):
        fautes.append(
            "BouchaudChrome.h : `draw_toolbar` est appelee ailleurs que depuis "
            "`draw_chrome` ; la bande d'onglets ne serait pas repeinte avec."
        )
    if "min(toolbar_height, s.surface_height)" in chrome:
        fautes.append(
            "BouchaudChrome.h : le degat du chrome s'arrete a la barre "
            "d'outils. La bande serait peinte sans etre annoncee, donc "
            "invisible jusqu'a la prochaine trame complete."
        )


def regle_raccourcis(chrome, fautes):
    """7."""
    bloc = corps(chrome, "inline bool raccourci_navigateur(")
    if bloc is None:
        fautes.append("BouchaudChrome.h : `raccourci_navigateur` introuvable.")
        return
    for appel, quoi in (
        ("nouvel_onglet()", "Ctrl+T"),
        ("ferme_onglet(", "Ctrl+W"),
        ("bascule_onglet(", "Ctrl+Tab"),
    ):
        if appel not in bloc:
            fautes.append("BouchaudChrome.h : %s ne fait plus rien." % quoi)


def main():
    fautes = []
    for chemin in (CHROME, M11, V19, HOTE):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        for faute in fautes:
            print("  - %s" % faute)
        return 1

    chrome = CHROME.read_text(encoding="utf-8")
    m11 = M11.read_text(encoding="utf-8")
    v19 = V19.read_text(encoding="utf-8")
    hote = HOTE.read_text(encoding="utf-8")

    regle_rappels(m11, fautes)
    regle_hooks(chrome, m11, hote, fautes)
    regle_creation(v19, fautes)
    regle_identifiants(chrome, fautes)
    regle_peinture(chrome, fautes)
    regle_raccourcis(chrome, fautes)

    if fautes:
        print("onglets : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print("onglets : chaque rappel vise l'onglet actif, chaque hook dit sa "
          "page, aucun identifiant reutilise")
    return 0


if __name__ == "__main__":
    sys.exit(main())
