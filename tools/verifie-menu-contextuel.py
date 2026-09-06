#!/usr/bin/env python3
"""Verifie le menu contextuel du navigateur.

LA DECISION QU'IL GARDE
-----------------------
Le menu ne s'ouvre PAS sur le clic droit. Il s'ouvre quand LibWeb le demande,
apres avoir distribue l'evenement `contextmenu` au document.

Ce detour n'est pas une elegance : c'est ce qui fait qu'une page qui appelle
`preventDefault()` -- un editeur de texte, une carte, un terminal web -- garde
son propre menu. Ouvrir depuis le chrome, sur le bouton, est plus court, marche
partout ailleurs, et casse silencieusement ces pages-la. Aucun test
d'integration ne le dirait : la capture montre un menu, et il a l'air juste.

LES REGLES
----------
1. Le chrome n'ouvre jamais le menu depuis le pointeur. Seuls les hooks du
   moteur l'ouvrent.

2. Les trois demandes du moteur sont branchees -- page, lien, image. En oublier
   une donne un clic droit qui ne fait rien a un endroit precis, et c'est
   exactement le genre de trou qu'on ne trouve qu'en cliquant partout.

3. Toute entree listee par `entrees_menu()` est traitee par
   `active_entree_menu()`. Une entree qui ne fait rien est pire qu'une entree
   absente : elle apprend a se mefier du menu.

4. Les entrees n'ont pas de rappel a elles. Elles appellent ce que les
   raccourcis clavier appellent deja -- un menu qui ferait les choses par un
   second chemin finirait par les faire differemment.

5. Le menu prend le pointeur AVANT la barre d'outils, et le clavier avant les
   barres de saisie. Un menu qu'on peut traverser n'en est pas un.

6. La molette le ferme. Il est ancre a un point de la PAGE, et la page defile
   sous lui : « ouvrir le lien » ouvrirait alors un lien qui n'est plus la.

Code de retour : 0 si les six regles sont respectees.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
CHROME = RACINE / "tools" / "ladybird" / "chrome" / "BouchaudChrome.h"
V19 = RACINE / "tools" / "ladybird" / "prepare-v19-navigateur.py"


def corps(source, signature):
    """Le corps qui suit `signature`, accolades equilibrees.

    Une DECLARATION anticipee -- `inline void f();` -- contient la meme chaine
    qu'une signature partielle de la definition. Prendre la premiere occurrence
    rendrait alors le corps de la fonction SUIVANTE, et la regle porterait sur
    du code sans rapport : ce qui distingue les deux est le point-virgule, une
    declaration en portant un AVANT la prochaine accolade.
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

def sans_commentaires(source):
    """Commentaires de ligne retires, chaines a guillemets doubles preservees.

    Ce fichier cherche des APPELS. Ce depot commente beaucoup, et ses
    commentaires citent le code qu'ils expliquent : sans depouillement, une
    regle se laisse satisfaire par la phrase qui decrit ce qu'il faudrait
    faire.
    """
    sortie = []
    index = 0
    taille = len(source)
    while index < taille:
        caractere = source[index]
        if caractere == '"':
            sortie.append(caractere)
            index += 1
            while index < taille:
                sortie.append(source[index])
                if source[index] == "\\" and index + 1 < taille:
                    sortie.append(source[index + 1])
                    index += 2
                    continue
                if source[index] == '"':
                    index += 1
                    break
                index += 1
            continue
        if source.startswith("//", index):
            fin = source.find("\n", index)
            index = taille if fin < 0 else fin
            continue
        sortie.append(caractere)
        index += 1
    return "".join(sortie)


def regle_origine(code, v19, fautes):
    """1 et 2. Le moteur ouvre le menu, et le chrome jamais."""
    pointeur = corps(code, "inline void handle_pointer(")
    if pointeur is not None and "ouvre_menu_contextuel(" in pointeur:
        fautes.append(
            "BouchaudChrome.h : `handle_pointer` ouvre le menu lui-meme. Une "
            "page qui appelle `preventDefault()` sur `contextmenu` -- un "
            "editeur, une carte, un terminal web -- perdrait alors son propre "
            "menu, et aucune capture ne le montrerait."
        )

    for hook, quoi in (
        ("page_did_request_context_menu", "le clic droit dans la page"),
        ("page_did_request_link_context_menu", "le clic droit sur un lien"),
        ("page_did_request_image_context_menu", "le clic droit sur une image"),
    ):
        bloc = v19.find(hook)
        if bloc < 0 or "ouvre_menu_contextuel" not in v19[bloc : bloc + 1600]:
            fautes.append(
                "prepare-v19-navigateur.py : %s n'ouvre plus le menu (%s)."
                % (quoi, hook)
            )


def regle_entrees(code, fautes):
    """3 et 4. Toute entree listee est traitee, et par les fonctions existantes."""
    liste = corps(code, "inline int entrees_menu(")
    action = corps(code, "inline void active_entree_menu(")
    if liste is None or action is None:
        fautes.append(
            "BouchaudChrome.h : `entrees_menu` ou `active_entree_menu` a "
            "disparu ; le menu ne saurait plus quoi montrer ou quoi faire."
        )
        return

    listees = set(re.findall(r"ajoute\((Menu\w+)\)", liste))
    if not listees:
        fautes.append("BouchaudChrome.h : `entrees_menu` ne liste plus rien.")
    traitees = set(re.findall(r"case (Menu\w+):", action))
    for entree in sorted(listees - traitees):
        fautes.append(
            "BouchaudChrome.h : l'entree %s est affichee mais "
            "`active_entree_menu` ne la traite pas. Une entree qui ne fait "
            "rien apprend a se mefier du menu entier." % entree
        )
    for entree in sorted(traitees - listees):
        fautes.append(
            "BouchaudChrome.h : l'entree %s est traitee mais jamais affichee ; "
            "c'est du code que rien n'atteint." % entree
        )

    # Les entrees appellent ce que les raccourcis appellent deja. Un `on_`
    # nouveau ici voudrait dire un second chemin vers la meme action.
    rappels = set(re.findall(r"s\.(on_\w+)\(", action))
    connus = {"on_navigate", "on_history_delta", "on_reload"}
    for rappel in sorted(rappels - connus):
        fautes.append(
            "BouchaudChrome.h : `active_entree_menu` appelle `%s`, un rappel "
            "que les raccourcis clavier n'utilisent pas. Deux chemins vers la "
            "meme action finissent par diverger." % rappel
        )


def regle_priorite(code, fautes):
    """5 et 6. Le menu passe avant tout, et la molette le ferme."""
    pointeur = corps(code, "inline void handle_pointer(")
    if pointeur is not None:
        menu = pointeur.find("s.menu_ouvert")
        barre = pointeur.find("in_toolbar")
        if menu < 0:
            fautes.append(
                "BouchaudChrome.h : `handle_pointer` ne consulte plus le menu ; "
                "un clic le traverserait et agirait sur ce qu'il recouvre."
            )
        elif barre >= 0 and barre < menu:
            fautes.append(
                "BouchaudChrome.h : `handle_pointer` traite la barre d'outils "
                "avant le menu ; un menu ouvert par-dessus la barre serait "
                "traverse."
            )

    clavier = corps(code, "inline void handle_key(")
    if clavier is not None:
        menu = clavier.find("s.menu_ouvert")
        recherche = clavier.find("s.recherche_focus")
        if menu < 0:
            fautes.append(
                "BouchaudChrome.h : `handle_key` ne consulte plus le menu ; "
                "Echap et les fleches iraient ailleurs qu'a ce qui est ouvert."
            )
        elif recherche >= 0 and recherche < menu:
            fautes.append(
                "BouchaudChrome.h : `handle_key` traite la barre de recherche "
                "avant le menu, qui est pourtant ouvert par-dessus."
            )

    molette = corps(code, "inline void handle_wheel(")
    if molette is None or "ferme_menu()" not in molette:
        fautes.append(
            "BouchaudChrome.h : la molette ne ferme plus le menu. Il est ancre "
            "a un point de la PAGE, et la page defile sous lui : « ouvrir le "
            "lien » ouvrirait un lien qui n'est plus la."
        )


def main():
    fautes = []
    for chemin in (CHROME, V19):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        for faute in fautes:
            print("  - %s" % faute)
        return 1

    code = sans_commentaires(CHROME.read_text(encoding="utf-8"))
    v19 = sans_commentaires(V19.read_text(encoding="utf-8"))

    regle_origine(code, v19, fautes)
    regle_entrees(code, fautes)
    regle_priorite(code, fautes)

    if fautes:
        print("menu contextuel : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print("menu contextuel : ouvert par le moteur, toute entree agit, "
          "prioritaire sur le reste")
    return 0


if __name__ == "__main__":
    sys.exit(main())
