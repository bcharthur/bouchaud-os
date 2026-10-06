#!/usr/bin/env python3
"""Verifie qu'une capture de page ne repeint plus toute la fenetre.

LE DEFAUT
---------
Sur la page d'accueil de Google -- dont le champ de recherche prend le focus
tout seul, et dont le curseur clignote donc deux fois par seconde -- le
navigateur produisait, sans une seule entree de l'utilisateur :

    M11_RENDER_STATS full=312 toolbar=40 page=312 pixels=486703296
    PERF-BROWSER pid=5 frames_delta=61 inputs_delta=0 bottleneck=memory-pagefault

312 recompositions COMPLETES en trois minutes, a 1 554 048 pixels chacune. La
page paraissait « se rafraichir en boucle » parce qu'elle se rafraichissait
reellement en boucle. Sur une page statique le compteur restait a 3 : le modele
d'invalidation de LibWeb fonctionnait deja. Ce qui manquait, c'est que sa
conclusion -- « voici le rectangle qui a change » -- etait calculee, puis jetee.

LES REGLES
----------
Sept, et aucune ne suffit seule. Chacune correspond a une facon de reperdre le
gain sans qu'aucun test ne devienne rouge : le symptome n'est pas une panne,
c'est une machine qui rame et une page qui clignote.

1. `present()` compose par degat, pas par trame complete. C'est le defaut
   lui-meme : un `compose_full()` remis ici et tout revient.

2. Le degat est celui du COMPOSITOR (BOUCHAUD_UI_V1). Il accumule ce qui a
   change entre deux trames presentees a une vue ; `server_did_paint` le remet
   a la vue, qui le transmet intact a la fenetre puis au chrome. Le remplacer
   par la trame entiere ramenerait le defaut d'origine.

3. Aucune capture. La page etait peinte deux fois -- une fois pour le
   Compositor, une fois pour la capture que le chrome demandait. Elle ne l'est
   plus qu'une fois.

4. Le backing store presente n'est copie qu'une fois, dans la surface, et
   seulement les lignes du plan. Aucune copie de transit
   (`to_shareable_bitmap`, `clone`) : six mebioctets de pages neuves par trame,
   c'est ce que le journal nommait `bottleneck=memory-pagefault`.

5. `BouchaudDegat.h` voyage avec le chrome (`prepare-ui-bouchaud.py`). L'oublier
   ne se voit pas ici : cela echoue a la compilation du navigateur.

6. Le banc d'essai hote existe et reste decouvert. C'est la seule chose qui
   exerce cette arithmetique ailleurs que dans QEMU -- et une erreur d'un pixel
   ne fait echouer aucun test d'integration, elle laisse une trainee.

7. Le viewport suit la fenetre. Le bouton plein ecran agrandissait le cadre
   sans rien changer a ce qu'il encadre : le moteur continuait de mettre en
   page a la largeur du demarrage.

CE QUE CE VERIFICATEUR NE PEUT PAS VOIR
---------------------------------------
Que le rectangle soit JUSTE. C'est le travail de
`tools/ladybird/chrome/test_degat.cpp`, qui compile l'arithmetique sur l'hote
et l'exerce. Les deux sont complementaires : celui-ci garde le chemin, l'autre
garde le calcul.

Code de retour : 0 si les sept regles sont respectees.
"""

import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent

CHROME = RACINE / "tools" / "ladybird" / "chrome" / "BouchaudChrome.h"
DEGAT = RACINE / "tools" / "ladybird" / "chrome" / "BouchaudDegat.h"
BANC = RACINE / "tools" / "ladybird" / "chrome" / "test_degat.cpp"
VUE = RACINE / "tools" / "ladybird" / "ui-bouchaud" / "BouchaudWebView.cpp"
FENETRE = RACINE / "tools" / "ladybird" / "ui-bouchaud" / "BrowserWindow.cpp"
INSTALLATEUR = RACINE / "tools" / "ladybird" / "prepare-ui-bouchaud.py"
HOTE = RACINE / "tools" / "ci" / "run_host_tests.sh"


def texte(chemin, fautes):
    if not chemin.exists():
        fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
        return None
    return chemin.read_text(encoding="utf-8")


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

def etendue(source, signature):
    """Les bornes du bloc accolade qui suit `signature`, ou None.

    `corps()` rend le TEXTE du bloc, ce qui ne suffit pas ici : une ligne
    identique ecrite en dehors du bloc y apparait aussi comme sous-chaine. La
    premiere version de la regle ci-dessous s'y est laissee prendre -- une
    publication de toute la surface deplacee AVANT `if (plan.complet)` passait,
    parce que la meme ligne existait aussi a l'interieur.
    """
    debut = source.find(signature)
    if debut < 0:
        return None
    ouvrante = source.find("{", debut)
    if ouvrante < 0:
        return None
    profondeur = 0
    for index in range(ouvrante, len(source)):
        if source[index] == "{":
            profondeur += 1
        elif source[index] == "}":
            profondeur -= 1
            if profondeur == 0:
                return (ouvrante, index + 1)
    return None


def regle_present(chrome, fautes):
    """1. `present()` compose par degat."""
    bloc = corps(chrome, "inline bool present(u64 page_id, NonnullRefPtr<Gfx::Bitmap> bitmap")
    if bloc is None:
        fautes.append(
            "BouchaudChrome.h : `present()` ne prend plus de degat. Une capture "
            "qui ne dit pas ce qui a change ne peut que tout repeindre."
        )
        return
    if "compose_page(" not in bloc:
        fautes.append(
            "BouchaudChrome.h : `present()` n'appelle plus `compose_page()`. "
            "Chaque capture repeindrait de nouveau toute la fenetre."
        )
    if "compose_full()" in bloc:
        fautes.append(
            "BouchaudChrome.h : `present()` appelle `compose_full()`. C'est le "
            "defaut d'origine : un curseur qui clignote repeint 1 554 048 pixels."
        )
    # BOUCHAUD_C22_ONGLETS : une capture d'onglet INACTIF est rangee, pas
    # composee. Les pages d'arriere-plan continuent de tourner -- un chargement
    # se termine, une animation avance -- et composer leur capture ferait
    # clignoter la page qu'on regarde avec celle d'a cote.
    if "page_active()" not in bloc:
        fautes.append(
            "BouchaudChrome.h : `present()` ne regarde plus de quel onglet "
            "vient la capture. Une page d'arriere-plan qui finit de charger "
            "s'afficherait par-dessus celle qu'on regarde."
        )

    plan = corps(chrome, "inline bool compose_page(")
    if plan is None or "suivi_page.planifie(" not in (plan or ""):
        fautes.append(
            "BouchaudChrome.h : `compose_page()` ne consulte plus le suivi de "
            "degat. Le plan de composition serait decide ailleurs, et le banc "
            "d'essai hote ne garderait plus rien."
        )
        return
    # Le rectangle publie doit DEPENDRE du plan.
    #
    # Il n'est plus `plan.publie` tel quel : depuis les calques
    # (BOUCHAUD_CHROME_V19_CALQUES), il part du plan puis grandit de ce que le
    # chrome ajoute par-dessus -- la barre d'outils repeinte sur une trame
    # partielle. La regle porte donc sur les deux bouts de cette chaine :
    #
    #   * il PART du plan, et d'aucune autre valeur ;
    #   * il ne peut grandir que par `englobe()`, sauf dans la branche « trame
    #     complete », la seule qui a le droit de publier toute la surface ;
    #   * une seule publication par trame, et c'est lui qu'elle annonce.
    #
    # Ce que cela interdit reste ce qu'on veut interdire : republier la fenetre
    # entiere a chaque capture, qui est le defaut d'origine.
    if "auto publie = plan.publie;" not in plan:
        fautes.append(
            "BouchaudChrome.h : `compose_page()` ne fait plus partir le "
            "rectangle publie du plan. C'est la seule chose qui garantit "
            "qu'une trame partielle publie un rectangle partiel."
        )

    complet = etendue(plan, "if (plan.complet)")
    position = 0
    for ligne in plan.splitlines(keepends=True):
        debut = position
        position += len(ligne)
        nue = ligne.split("//", 1)[0].strip()
        if not nue.startswith("publie = ") or ".englobe(" in nue:
            continue
        if complet is not None and complet[0] <= debut < complet[1]:
            continue
        fautes.append(
            "BouchaudChrome.h : `compose_page()` reecrit le rectangle publie "
            "hors de la branche « trame complete » et sans `englobe()`.\n"
            "           %s" % ligne.strip()
        )

    appels = [
        ligne.strip()
        for ligne in plan.splitlines()
        if "send_frame_ready" in ligne.split("//", 1)[0]
    ]
    if len(appels) != 1:
        fautes.append(
            "BouchaudChrome.h : `compose_page()` publie %d fois. Une trame, une "
            "publication : deux messages pour la meme trame font recopier deux "
            "fois au compositeur." % len(appels)
        )
    elif "publie" not in appels[0]:
        fautes.append(
            "BouchaudChrome.h : `compose_page()` publie un rectangle qui ne "
            "vient pas du plan.\n           %s" % appels[0]
        )


def regle_degat_natif(vue, fenetre, fautes):
    """2. Le degat est celui du Compositor, transmis intact.

    BOUCHAUD_UI_V1 : le Compositor accumule le degat entre deux trames
    presentees (`BackingStoreManager`), et `server_did_paint` le remet a la
    vue. Il ne doit etre ni remplace par la trame entiere, ni perdu en route."""
    accepte = corps(vue, "void BouchaudWebView::did_accept_presented_backing_store(")
    if accepte is None or "m_window.present(" not in accepte or "damage_rect)" not in accepte:
        fautes.append(
            "BouchaudWebView.cpp : la trame presentee n'arrive plus a la "
            "fenetre avec le degat du Compositor."
        )
    present = corps(fenetre, "void BrowserWindow::present(")
    if present is None or "BouchaudChrome::present(" not in present:
        fautes.append("BrowserWindow.cpp : `present` ne remet plus la trame au chrome.")
    elif "degat.x(), degat.y(), degat.width(), degat.height()" not in present:
        fautes.append(
            "BrowserWindow.cpp : `present` ne transmet plus le degat recu. "
            "Chaque trame redeviendrait complete."
        )


def regle_aucune_capture(vue, fenetre, fautes):
    """3. Aucune capture : la page arrive par le Compositor, une fois."""
    for nom, source in (("BouchaudWebView.cpp", vue), ("BrowserWindow.cpp", fenetre)):
        for interdit in ("take_screenshot", "queue_screenshot_task", "request_screenshot",
                         "process_screenshot_requests", "on_ready_to_paint"):
            if interdit in source:
                fautes.append(
                    "%s : `%s` -- la page serait peinte une seconde fois pour "
                    "etre capturee, en plus de la trame du Compositor." % (nom, interdit)
                )


def regle_aucune_copie(vue, fenetre, chrome, fautes):
    """4. Le backing store n'est copie qu'une fois : dans la surface.

    La vue remet une REFERENCE sur le tampon de face ; le chrome en recopie les
    seules lignes du plan. Toute copie intermediaire (`to_shareable_bitmap`,
    `clone`, une `Bitmap::create` de transit) referait les six mebioctets par
    trame que le journal nommait `bottleneck=memory-pagefault`."""
    for nom, source in (("BouchaudWebView.cpp", vue), ("BrowserWindow.cpp", fenetre)):
        for interdit in ("to_shareable_bitmap", "->clone(", ".clone(", "Bitmap::create("):
            if interdit in source:
                fautes.append("%s : copie intermediaire de la trame (`%s`)." % (nom, interdit))
    if "bitmap_if_present()" not in vue:
        fautes.append(
            "BouchaudWebView.cpp : la vue ne lit plus le tampon de face du "
            "backing store presente."
        )
    if "ShareableBitmap last_page" in chrome:
        fautes.append(
            "BouchaudChrome.h : `last_page` redevient une capture partagee "
            "(`ShareableBitmap`) au lieu d'une reference sur le backing store."
        )


def regle_entete_voyage(installateur, fautes):
    """5. Les en-tetes du chrome voyagent avec le frontend."""
    if 'chrome.glob("Bouchaud*.h")' not in installateur:
        fautes.append(
            "prepare-ui-bouchaud.py : les en-tetes du chrome (dont "
            "BouchaudDegat.h) ne sont plus copies dans UI/Bouchaud ; la "
            "compilation du navigateur echouerait vingt minutes plus tard."
        )


def regle_banc_decouvert(hote, fautes):
    """6. Le banc d'essai hote existe et reste decouvert."""
    if "test_*.cpp" not in hote:
        fautes.append(
            "run_host_tests.sh : les suites C++ hote ne sont plus decouvertes. "
            "L'arithmetique de degat ne s'executerait plus que dans QEMU, ou "
            "une erreur d'un pixel ne fait echouer aucun test."
        )


def regle_viewport(chrome, fenetre, fautes):
    """7. Le viewport suit la fenetre."""
    if "on_resize" not in chrome:
        fautes.append(
            "BouchaudChrome.h : plus de rappel de redimensionnement. Le bouton "
            "plein ecran agrandirait le cadre sans rien changer a ce qu'il "
            "encadre."
        )
    i = fenetre.find("c.on_resize = [")
    if i < 0:
        fautes.append(
            "BrowserWindow.cpp : `on_resize` n'est plus branche ; le moteur "
            "continuerait de mettre en page a la largeur du demarrage."
        )
        return
    if "reset_viewport_size(" not in fenetre[i : i + 600]:
        fautes.append(
            "BrowserWindow.cpp : le redimensionnement ne change plus le "
            "viewport des vues. C'est LibWeb qui decide de la largeur de ligne."
        )


def main():
    fautes = []

    chrome = texte(CHROME, fautes)
    degat = texte(DEGAT, fautes)
    banc = texte(BANC, fautes)
    vue = texte(VUE, fautes)
    fenetre = texte(FENETRE, fautes)
    installateur = texte(INSTALLATEUR, fautes)
    hote = texte(HOTE, fautes)

    if None in (chrome, degat, banc, vue, fenetre, installateur, hote):
        for faute in fautes:
            print("ECHEC  %s" % faute)
        return 1

    regle_present(chrome, fautes)
    regle_degat_natif(vue, fenetre, fautes)
    regle_aucune_capture(vue, fenetre, fautes)
    regle_aucune_copie(vue, fenetre, chrome, fautes)
    regle_entete_voyage(installateur, fautes)
    regle_banc_decouvert(hote, fautes)
    regle_viewport(chrome, fenetre, fautes)

    if fautes:
        for faute in fautes:
            print("ECHEC  %s" % faute)
        return 1

    print("repeinture partielle : degat du Compositor transmis, aucune "
          "capture, aucune copie de transit, viewport suivi, banc hote decouvert")
    return 0


if __name__ == "__main__":
    sys.exit(main())
