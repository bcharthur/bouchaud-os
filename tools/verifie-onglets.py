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
1. Les rappels du chrome vers le moteur agissent sur la vue de l'onglet
   ACTIF (`BrowserWindow::vue_active`, qui le demande au chrome a chaque
   appel). BOUCHAUD_UI_V1 : le chrome vit dans le processus navigateur.

2. Les hooks qui remontent au chrome portent leur identifiant d'onglet. Un
   hook qui n'en porte pas ne peut pas savoir si l'evenement concerne l'onglet
   affiche.

3. Le moteur cree la page, le navigateur cree la vue puis l'onglet -- et
   jamais l'inverse. Une fenetre surgissante passe par `on_new_web_view`.

4. La fermeture passe par le moteur (`request_close`, `beforeunload`) et
   retire la vue ET l'onglet, apres coup, hors du rappel de la vue.

5. Les identifiants d'onglet ne sont jamais reutilises. Une trame partie
   avant une fermeture -- il y en a toujours une en vol -- serait sinon prise
   pour celle du nouvel onglet.

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
FENETRE = RACINE / "tools" / "ladybird" / "ui-bouchaud" / "BrowserWindow.cpp"


def corps(source, signature):
    """Le corps de la FONCTION dont la signature est donnee.

    Une signature peut apparaitre d'abord comme DECLARATION anticipee. On les
    distingue par le point-virgule qui la termine -- mais SEULEMENT celui qui
    est a profondeur nulle : `-> [u8; 16]` en contient un, et une regle qui le
    prenait pour une fin de declaration cherchait la fonction suivante sans le
    dire, puis rapportait « introuvable » sur une fonction bien presente.
    """
    debut = 0
    while True:
        trouve = source.find(signature, debut)
        if trouve < 0:
            return None
        ouvrante = -1
        declaration = -1
        profondeur = 0
        # On repart du DEBUT de la signature : certaines « signatures »
        # recherchees incluent deja leur accolade (`struct Champ {`), et
        # repartir apres elle ferait manquer la seule accolade qui compte.
        i = trouve
        while i < len(source):
            c = source[i]
            if c in "([<":
                profondeur += 1
            elif c in ")]>":
                if profondeur > 0:
                    profondeur -= 1
            elif profondeur == 0:
                if c == "{":
                    ouvrante = i
                    break
                if c == ";":
                    declaration = i
                    break
            i += 1
        if ouvrante >= 0:
            break
        if declaration < 0:
            return None
        debut = declaration + 1
    profondeur = 0
    i = ouvrante
    while i < len(source):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouvrante : i + 1]
        i += 1
    return None


RAPPELS_MOTEUR = (
    "on_mouse_event", "on_key_event", "on_navigate", "on_history_delta", "on_reload",
    "on_stop", "on_zoom", "on_find", "on_find_next", "on_find_previous",
    "on_select_all", "on_copy", "on_cut", "on_paste",
)


def rappel(fenetre, nom):
    """Le corps de la lambda posee sur `c.<nom>` dans `branche_chrome`."""
    m = re.search(r"\bc\." + re.escape(nom) + r" = \[", fenetre)
    if not m:
        return None
    return corps(fenetre, fenetre[m.start():m.start() + len(nom) + 6])


def regle_rappels(fenetre, fautes):
    """1. Les rappels visent l'onglet actif."""
    actif = corps(fenetre, "BouchaudWebView* BrowserWindow::vue_active() const")
    if actif is None or "BouchaudChrome::page_active()" not in actif:
        fautes.append(
            "BrowserWindow.cpp : `vue_active` ne demande plus au chrome quel "
            "onglet est actif. Un clic rechargerait l'autre onglet."
        )
    for nom in RAPPELS_MOTEUR:
        bloc = rappel(fenetre, nom)
        if bloc is None:
            fautes.append("BrowserWindow.cpp : `%s` n'est plus branche." % nom)
            continue
        if "vue_active()" not in bloc:
            fautes.append(
                "BrowserWindow.cpp : `%s` n'agit plus sur la vue de l'onglet "
                "ACTIF." % nom
            )


def regle_hooks(chrome, fenetre, fautes):
    """2. Les hooks portent leur identifiant d'onglet."""
    for signature, quoi in (
        ("inline bool present(u64 page_id,", "une trame"),
        ("inline void set_committed_url(u64 page_id,", "une URL commitee"),
        ("inline void set_loading(u64 page_id,", "un etat de chargement"),
        ("inline void set_title(u64 page_id,", "un titre"),
    ):
        if signature not in chrome:
            fautes.append(
                "BouchaudChrome.h : %s arrive sans dire de quel onglet elle "
                "vient. Le chrome l'appliquerait a l'onglet affiche, quel que "
                "soit celui qui a change." % quoi
            )
    for appel in re.findall(
        r"BouchaudChrome::(set_committed_url|set_loading|set_title|present)\(([^,)]*)",
        fenetre,
    ):
        premier = appel[1].strip()
        if premier == "onglet" or premier.startswith(("vue.onglet(", "vue->onglet(")):
            continue
        fautes.append(
            "BrowserWindow.cpp : `BouchaudChrome::%s` est appelee sans "
            "identifiant d'onglet (premier argument : %r)." % (appel[0], premier)
        )


def regle_creation(fenetre, fautes):
    """3 et 4. Le navigateur cree la vue, puis l'onglet ; la fermeture passe
    par le moteur (`request_close`, `beforeunload`) et retire les deux."""
    popup = corps(fenetre, "String BrowserWindow::ouvre_vue_demandee_par_la_page(")
    if popup is None or "create_child(" not in popup:
        fautes.append(
            "BrowserWindow.cpp : une fenetre surgissante ne cree plus de vue "
            "sur la page que le moteur a deja creee. `target=_blank` ne ferait "
            "rien du tout."
        )
    elif "ajoute_onglet(" not in popup:
        fautes.append(
            "BrowserWindow.cpp : la page ouverte par le document n'apparait "
            "dans aucun onglet : elle vivrait sans que rien ne puisse l'atteindre."
        )
    fermer = rappel(fenetre, "on_fermer_onglet")
    if fermer is None or "request_close()" not in fermer:
        fautes.append(
            "BrowserWindow.cpp : fermer un onglet ne passe plus par le moteur "
            "(`request_close`) : `beforeunload` serait saute."
        )
    if "Core::deferred_invoke([this, onglet] { retire_vue(onglet); })" not in fenetre:
        fautes.append(
            "BrowserWindow.cpp : `on_close` ne differe plus la destruction de "
            "la vue -- elle se detruirait dans son propre rappel."
        )
    retire = corps(fenetre, "void BrowserWindow::retire_vue(")
    if retire is None or "m_vues.remove(" not in retire or "BouchaudChrome::retire_onglet(onglet)" not in retire:
        fautes.append(
            "BrowserWindow.cpp : `retire_vue` ne retire plus a la fois la vue "
            "et l'onglet. La bande garderait une ligne sans page, ou l'inverse."
        )


def regle_identifiants(fenetre, fautes):
    """5. Les identifiants d'onglet ne sont jamais reutilises.

    La fenetre les attribue (`m_prochain_onglet`) : ni le chrome, ni le
    `page_id` du moteur -- qui change quand la vue change de WebContent."""
    for sig in ("u64 BrowserWindow::ouvre_onglet(", "String BrowserWindow::ouvre_vue_demandee_par_la_page("):
        bloc = corps(fenetre, sig)
        if bloc is None or "m_prochain_onglet++" not in bloc:
            fautes.append(
                "BrowserWindow.cpp : %s n'attribue plus un identifiant neuf. "
                "Deux onglets porteraient le meme, et une trame en vol serait "
                "prise pour celle du nouvel onglet." % sig.split("::")[1]
            )
    if re.search(r"m_prochain_onglet\s*(=|-=|--)", fenetre):
        fautes.append(
            "BrowserWindow.cpp : `m_prochain_onglet` est recule ou reaffecte : "
            "un identifiant serait reutilise."
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
    for chemin in (CHROME, FENETRE):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        for faute in fautes:
            print("  - %s" % faute)
        return 1

    chrome = CHROME.read_text(encoding="utf-8")
    fenetre = FENETRE.read_text(encoding="utf-8")

    regle_rappels(fenetre, fautes)
    regle_hooks(chrome, fenetre, fautes)
    regle_creation(fenetre, fautes)
    regle_identifiants(fenetre, fautes)
    regle_peinture(chrome, fautes)
    regle_raccourcis(chrome, fautes)

    if fautes:
        print("onglets : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print("onglets : chaque rappel vise l'onglet actif, chaque hook dit son "
          "onglet, aucun identifiant reutilise")
    return 0


if __name__ == "__main__":
    sys.exit(main())
