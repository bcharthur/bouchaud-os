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

2. Le degat de `paint_next_frame()` est ACCUMULE, pas ecrase. Le pump ne garde
   qu'une capture en vol ; les etapes de rendu qui tombent pendant une capture
   ont bel et bien change des pixels. Ne retenir que la derniere laisserait
   leurs traces a l'ecran, et le defaut serait pire que celui qu'on corrige --
   une trainee au lieu d'une lenteur.

3. Les deux `PaintConfig` -- celle du rendu, celle de la capture -- restent
   identiques mot pour mot. Elles se comparent par egalite : une difference
   d'un seul pixel fait reenregistrer TOUTE la liste d'affichage a chaque
   capture, et empeche le calcul de degat de se declencher, puisqu'il exige que
   la config memorisee soit deja la sienne. Rien ne casse ; tout redevient
   lent.

4. La cible d'une capture M11 est anonyme des l'origine, et reutilisee.
   `Gfx::Bitmap::create()` alloue de la memoire ordinaire, puis
   `to_shareable_bitmap()` en alloue une seconde pour y recopier la premiere :
   six mebioctets de pages neuves par trame. C'est ce chemin que le journal de
   la machine nommait `bottleneck=memory-pagefault`.

5. `BouchaudDegat.h` voyage avec le chrome. L'oublier ne se voit pas ici : cela
   echoue a la compilation de WebContent, vingt minutes plus tard.

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
REPAINT = RACINE / "tools" / "ladybird" / "prepare-repaint.py"
M11 = RACINE / "tools" / "ladybird" / "prepare-m11-chrome.py"
HOTE = RACINE / "tools" / "ci" / "run_host_tests.sh"


def texte(chemin, fautes):
    if not chemin.exists():
        fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
        return None
    return chemin.read_text(encoding="utf-8")


def corps(source, signature):
    """Le corps d'une fonction, des accolades equilibrees."""
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
                return source[ouvrante : index + 1]
    return None


def regle_present(chrome, fautes):
    """1. `present()` compose par degat."""
    bloc = corps(chrome, "inline bool present(Gfx::ShareableBitmap const& screenshot, int degat_x")
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

    plan = corps(chrome, "inline bool compose_page(")
    if plan is None or "suivi_page.planifie(" not in (plan or ""):
        fautes.append(
            "BouchaudChrome.h : `compose_page()` ne consulte plus le suivi de "
            "degat. Le plan de composition serait decide ailleurs, et le banc "
            "d'essai hote ne garderait plus rien."
        )
        return
    # Le rectangle publie doit dependre du plan. Une publication de la surface
    # entiere n'a le droit d'exister que dans la branche « trame complete ».
    for numero, ligne in enumerate(plan.splitlines(), start=1):
        nue = ligne.split("//", 1)[0]
        if "send_frame_ready" in nue and "plan.publie" not in nue and "plan.complet" not in nue:
            if "s.surface_width, s.surface_height" in nue:
                continue  # la branche complete, verifiee ci-dessous
            fautes.append(
                "BouchaudChrome.h : `compose_page()` publie un rectangle qui ne "
                "vient pas du plan.\n           %s" % ligne.strip()
            )
    if "send_frame_ready({ plan.publie.x" not in plan:
        fautes.append(
            "BouchaudChrome.h : `compose_page()` ne publie plus le rectangle "
            "partiel calcule par le plan."
        )


def regle_accumulation(repaint, fautes):
    """2. Le degat est accumule, pas ecrase."""
    if "bouchaud_accumulate_frame_damage" not in repaint:
        fautes.append(
            "prepare-repaint.py : le degat de `paint_next_frame()` n'est plus "
            "accumule. Il redeviendrait ce qu'il etait : calcule puis jete."
        )
        return
    if "present_frame(viewport_rect, damage_rect)" not in repaint:
        fautes.append(
            "prepare-repaint.py : l'ancre de `paint_next_frame()` a disparu ; "
            "l'accumulation ne s'accroche plus a rien."
        )
    if "m_bouchaud_frame_damage.united(" not in repaint:
        fautes.append(
            "prepare-repaint.py : les degats successifs ne sont plus REUNIS. "
            "Le pump ne capture pas toutes les etapes de rendu : ne garder que "
            "la derniere laisse a l'ecran les pixels changes par les autres."
        )
    if "bouchaud_require_full_frame_damage" not in repaint:
        fautes.append(
            "prepare-repaint.py : plus aucun moyen de dire « on ne sait plus ». "
            "Un navigable imbrique peint dans son propre repere : reunir son "
            "rectangle avec celui du sommet designerait des pixels au hasard."
        )


def bloc_substitution(source, etiquette):
    """Le texte d'un appel `substitute(...)` designe par son etiquette.

    Chercher dans tout le fichier laisserait la docstring repondre a la place
    du code : elle CITE les expressions qu'elle explique. Une regle satisfaite
    par un commentaire ne protege rien.
    """
    fin = source.find('"%s"' % etiquette)
    if fin < 0:
        return None
    debut = source.rfind("substitute(", 0, fin)
    if debut < 0:
        return None
    return source[debut:fin]


def regle_config_identique(repaint, fautes):
    """3. Les deux PaintConfig restent identiques."""
    bloc = bloc_substitution(repaint, "taille de la capture")
    if bloc is None:
        fautes.append(
            "prepare-repaint.py : la substitution « taille de la capture » a "
            "disparu ; plus rien ne dimensionne la capture M11."
        )
        return

    compact = " ".join(bloc.split())
    if "? page().css_to_device_rect(this->viewport_rect())" not in compact:
        fautes.append(
            "prepare-repaint.py : la capture M11 ne convertit plus le viewport "
            "comme `paint_next_frame()`. `PaintConfig` se compare par egalite : "
            "une difference d'un pixel reenregistre toute la liste d'affichage "
            "a chaque capture et eteint le calcul de degat, sans rien casser."
        )
    if "task.bouchaud_interactive_frame" not in compact:
        fautes.append(
            "prepare-repaint.py : la conversion alignee n'est plus reservee a "
            "la trame interactive, ou elle ne s'y applique plus."
        )


def regle_cible_reutilisee(repaint, fautes):
    """4. La cible de capture M11 est anonyme et reutilisee."""
    if "bouchaud_interactive_frame_bitmap" not in repaint:
        fautes.append(
            "prepare-repaint.py : la cible d'une capture M11 n'est plus "
            "reutilisee. Chaque trame rallouerait six mebioctets de pages "
            "neuves -- `bottleneck=memory-pagefault` dans le journal."
        )
        return
    if "create_shareable" not in repaint:
        fautes.append(
            "prepare-repaint.py : la cible n'est plus allouee dans un tampon "
            "anonyme. `to_shareable_bitmap()` en allouerait un second et y "
            "recopierait toute l'image a chaque trame."
        )
    if "bouchaud_take_frame_damage" not in repaint:
        fautes.append(
            "prepare-repaint.py : la capture ne prend plus le degat accumule. "
            "Elle partirait sans lui, et PageClient n'aurait rien a transmettre."
        )


def regle_entete_voyage(m11, fautes):
    """5. Les en-tetes du chrome voyagent avec lui, et sont DECOUVERTS.

    Ils etaient enumeres, un `shutil.copyfile` par fichier. Chaque piece
    extraite dans son propre en-tete -- parce qu'elle ne depend de rien et
    devient donc verifiable sur l'hote -- demandait une ligne de plus, et en
    oublier une ne se voit qu'a la compilation de WebContent, vingt minutes
    plus tard, sur un `#include` introuvable.
    """
    if 'glob("Bouchaud*.h")' not in m11:
        fautes.append(
            "prepare-m11-chrome.py : les en-tetes du chrome ne sont plus "
            "decouverts. Un en-tete ajoute serait oublie, et cela ne se "
            "verrait qu'a la compilation de WebContent."
        )
        return
    if "shutil.copyfile(source_header" not in m11:
        fautes.append(
            "prepare-m11-chrome.py : les en-tetes decouverts ne sont plus "
            "copies dans l'arbre Ladybird."
        )


def regle_banc_decouvert(hote, fautes):
    """6. Le banc d'essai hote existe et reste decouvert."""
    if "test_*.cpp" not in hote:
        fautes.append(
            "run_host_tests.sh : les suites C++ hote ne sont plus decouvertes. "
            "L'arithmetique de degat ne s'executerait plus que dans QEMU, ou "
            "une erreur d'un pixel ne fait echouer aucun test."
        )


def regle_viewport(chrome, m11, fautes):
    """7. Le viewport suit la fenetre."""
    if "on_resize" not in chrome:
        fautes.append(
            "BouchaudChrome.h : plus de rappel de redimensionnement. Le bouton "
            "plein ecran agrandirait le cadre sans rien changer a ce qu'il "
            "encadre."
        )
    if "chrome.on_resize = [" not in m11:
        fautes.append(
            "prepare-m11-chrome.py : `on_resize` n'est plus branche ; le moteur "
            "continuerait de mettre en page a la largeur du demarrage."
        )
        return
    if "set_viewport(page_id" not in m11:
        fautes.append(
            "prepare-m11-chrome.py : le redimensionnement ne change plus le "
            "viewport. C'est LibWeb qui decide de la largeur de ligne, pas nous."
        )


def main():
    fautes = []

    chrome = texte(CHROME, fautes)
    degat = texte(DEGAT, fautes)
    banc = texte(BANC, fautes)
    repaint = texte(REPAINT, fautes)
    m11 = texte(M11, fautes)
    hote = texte(HOTE, fautes)

    if None in (chrome, degat, banc, repaint, m11, hote):
        for faute in fautes:
            print("ECHEC  %s" % faute)
        return 1

    regle_present(chrome, fautes)
    regle_accumulation(repaint, fautes)
    regle_config_identique(repaint, fautes)
    regle_cible_reutilisee(repaint, fautes)
    regle_entete_voyage(m11, fautes)
    regle_banc_decouvert(hote, fautes)
    regle_viewport(chrome, m11, fautes)

    if fautes:
        for faute in fautes:
            print("ECHEC  %s" % faute)
        return 1

    print("repeinture partielle : degat accumule, config alignee, cible "
          "reutilisee, viewport suivi, banc hote decouvert")
    return 0


if __name__ == "__main__":
    sys.exit(main())
