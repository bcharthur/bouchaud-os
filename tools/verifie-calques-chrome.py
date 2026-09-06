#!/usr/bin/env python3
"""Verifie que les surfaces flottantes du chrome ne laissent pas de trainees.

LE DEFAUT QU'IL GARDE FERME
---------------------------
Depuis `BouchaudDegat.h`, une capture de page ne recopie que le rectangle que
le moteur a signale. C'est ce qui a fait tomber le nombre de pixels ecrits d'un
facteur cent. Mais cela pose une contrainte a tout ce que le chrome dessine
PAR-DESSUS la page -- bulle de survol, barre de recherche, menu contextuel :
ces pixels-la n'appartiennent a personne. Le moteur ne les connait pas, donc il
ne les signalera jamais comme changes, et la surface les porte encore quand le
calque disparait.

Deux fautes symetriques, et toutes deux laissent une trainee a l'ecran :

  * le calque disparait, personne ne demande de reecrire la ou il etait : ses
    pixels restent jusqu'a la prochaine trame complete, qui peut ne jamais
    venir ;
  * la page se repeint sous un calque immobile, le calque n'est pas redessine :
    un trou rectangulaire s'y ouvre.

Aucune des deux ne fait echouer un test d'integration. La capture du smoke
passe ; c'est l'oeil de l'utilisateur qui trouve le defaut, une semaine plus
tard, sur une capture d'ecran.

LES REGLES
----------
1. Tout calque declare est POSE. Ajouter un membre a l'enumeration et oublier
   `place()` donne un calque qui n'apparait jamais -- et le compilateur est
   content, puisque l'enumeration compile.

2. Tout calque declare est DESSINE. La faute symetrique : un calque dont la
   boite est suivie, donc dont le degat est reclame, mais que rien ne peint.
   La page est effacee dessous et rien ne la remplace.

3. Le degat des calques rejoint celui de la page AVANT le plan. Apres, le plan
   a deja decide de ne rien recopier, et la restauration n'aura pas lieu.

4. Les calques se dessinent APRES la copie de page. Avant, la copie les
   recouvre.

5. `acte()` n'est appele qu'une fois, et depuis la composition. Acter sur une
   trame abandonnee ferait croire au suivi que la surface porte un calque qui
   n'y a jamais ete ecrit : il ne le redemanderait plus jamais.

6. `tick()` regarde les calques avant le compteur de barre d'outils.
   `compose_toolbar_only()` ne touche aucun pixel de page : il ne peut pas
   restaurer ce qu'un calque deplace laisse derriere lui.

7. Le texte des calques passe par `draw_ui_text`. C'est le point unique que
   `modernise-v15.py` remplace par le rendu Skia ; un `draw_text` direct
   donnerait une bulle en police bitmap a cote d'une barre d'adresse en DejaVu.

8. Les ancres de `modernise-v15.py` existent encore, mot pour mot. C'est la
   regle qui coute vingt minutes quand elle lache : la substitution echoue au
   milieu de la construction, ou -- pire -- ne s'applique plus et le chrome
   part avec la police de secours sans que rien ne le dise.

CE QUE CE VERIFICATEUR NE PEUT PAS VOIR
---------------------------------------
Que les rectangles soient JUSTES. C'est le travail de
`tools/ladybird/chrome/test_calques.cpp`, qui compile l'arithmetique sur l'hote
et l'exerce. Les deux sont complementaires : celui-ci garde le chemin, l'autre
garde le calcul.

Code de retour : 0 si les huit regles sont respectees.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent

CHROME = RACINE / "tools" / "ladybird" / "chrome" / "BouchaudChrome.h"
CALQUES = RACINE / "tools" / "ladybird" / "chrome" / "BouchaudCalques.h"
BANC = RACINE / "tools" / "ladybird" / "chrome" / "test_calques.cpp"
V15 = RACINE / "tools" / "ladybird" / "chrome" / "modernise-v15.py"


def texte(chemin, fautes):
    if not chemin.exists():
        fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
        return None
    return chemin.read_text(encoding="utf-8")


def sans_commentaires(source):
    """Le meme texte, commentaires retires, chaines preservees.

    Toutes les regles d'ORDRE de ce fichier comparent des positions dans le
    source. Un commentaire qui cite le code -- et ce fichier-ci en contient,
    puisqu'il explique la discipline qu'il applique -- deplacerait ces
    positions et compterait comme un appel. Le premier essai de ce
    verificateur a compte deux `acte()` la ou il n'y en a qu'un, parce que le
    second etait dans un schema en commentaire.

    Un `//` a l'interieur d'une chaine -- `"https://"sv` -- n'ouvre pas de
    commentaire : c'est pour cela que ce petit analyseur suit les chaines
    plutot que de decouper ligne a ligne.
    """
    sortie = []
    index = 0
    taille = len(source)
    while index < taille:
        c = source[index]
        if c == '"' or c == "'":
            delimiteur = c
            sortie.append(c)
            index += 1
            while index < taille:
                sortie.append(source[index])
                if source[index] == "\\" and index + 1 < taille:
                    sortie.append(source[index + 1])
                    index += 2
                    continue
                if source[index] == delimiteur:
                    index += 1
                    break
                index += 1
            continue
        if source.startswith("//", index):
            fin = source.find("\n", index)
            index = taille if fin < 0 else fin
            continue
        if source.startswith("/*", index):
            fin = source.find("*/", index + 2)
            index = taille if fin < 0 else fin + 2
            continue
        sortie.append(c)
        index += 1
    return "".join(sortie)


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

def calques_declares(code, fautes):
    """Les noms de l'enumeration du chrome, `Nombre` exclu.

    Ils sont LUS et non recopies ici : une liste ecrite dans ce fichier serait
    une seconde source de verite, et c'est exactement le genre de liste qu'on
    oublie de completer.
    """
    bloc = re.search(r"enum Calque : int \{(.*?)\};", code, re.S)
    if not bloc:
        fautes.append("BouchaudChrome.h : enumeration Calque introuvable.")
        return []
    noms = re.findall(r"([A-Za-z_][A-Za-z0-9_]*)\s*(?:=\s*\d+\s*)?,", bloc.group(1))
    noms = [nom for nom in noms if nom != "Nombre"]
    if not noms:
        fautes.append("BouchaudChrome.h : aucun calque declare.")
    return noms


def regle_places(code, noms, fautes):
    """1. Tout calque declare est pose."""
    bloc = corps(code, "inline void mesure_calques()")
    if bloc is None:
        fautes.append(
            "BouchaudChrome.h : `mesure_calques()` a disparu. C'est le seul "
            "endroit qui decide ou va chaque calque ; sans lui, aucune boite "
            "ne suit le redimensionnement de la fenetre."
        )
        return
    for nom in noms:
        if not re.search(r"\b%s\b" % nom, bloc):
            fautes.append(
                "BouchaudChrome.h : le calque %s est declare mais "
                "`mesure_calques()` ne le pose pas. Il n'apparaitra jamais." % nom
            )


def regle_dessines(code, noms, fautes):
    """2. Tout calque declare est dessine."""
    bloc = corps(code, "inline void dessine_calques(Canvas const& canvas")
    if bloc is None:
        fautes.append(
            "BouchaudChrome.h : `dessine_calques()` a disparu. La page serait "
            "effacee sous les calques sans que rien ne les repeigne."
        )
        return
    for nom in noms:
        if not re.search(r"\b%s\b" % nom, bloc):
            fautes.append(
                "BouchaudChrome.h : le calque %s est suivi mais "
                "`dessine_calques()` ne le peint pas : la page est effacee "
                "dessous et rien ne la remplace." % nom
            )
        if "doit_redessiner(" not in bloc:
            fautes.append(
                "BouchaudChrome.h : `dessine_calques()` ne consulte plus "
                "`doit_redessiner`. Un calque immobile dont la page se repeint "
                "dessous n'est alors plus redessine."
            )
            return


def regle_ordre_composition(code, fautes):
    """3, 4 et 5 : l'ordre des trois appels dans la composition."""
    bloc = corps(code, "inline bool compose_page(BouchaudDegat::Rect degat)")
    if bloc is None:
        fautes.append("BouchaudChrome.h : `compose_page()` introuvable.")
        return

    mesure = bloc.find("mesure_calques()")
    plan = bloc.find("planifie(")
    copie = bloc.find("bitmap->scanline(")
    dessin = bloc.find("dessine_calques(")
    acte = bloc.find("calques.acte()")

    if mesure < 0 or plan < 0:
        fautes.append(
            "BouchaudChrome.h : `compose_page()` ne mesure plus les calques "
            "avant de planifier. Leur degat ne rejoint donc plus celui de la "
            "page, et rien ne restaure les pixels sous un calque efface."
        )
    elif mesure > plan:
        fautes.append(
            "BouchaudChrome.h : les calques sont mesures APRES `planifie()`. "
            "Le plan a deja decide de ne rien recopier : la restauration "
            "n'aura pas lieu et le calque efface laissera sa trainee."
        )

    if "degat_calques" not in bloc or ".englobe(" not in bloc:
        fautes.append(
            "BouchaudChrome.h : le degat des calques ne rejoint plus celui du "
            "moteur dans `compose_page()`."
        )

    if dessin < 0:
        fautes.append(
            "BouchaudChrome.h : `compose_page()` ne dessine plus les calques."
        )
    elif copie >= 0 and dessin < copie:
        fautes.append(
            "BouchaudChrome.h : les calques sont dessines AVANT la copie de "
            "page, qui les recouvre aussitot."
        )

    if acte < 0:
        fautes.append(
            "BouchaudChrome.h : `compose_page()` n'acte plus la trame. Le "
            "suivi redemanderait indefiniment les memes pixels."
        )
    elif dessin >= 0 and acte < dessin:
        fautes.append(
            "BouchaudChrome.h : la trame est actee AVANT que les calques "
            "soient dessines."
        )

    total = code.count("calques.acte()")
    if total != 1:
        fautes.append(
            "BouchaudChrome.h : %d appels a `calques.acte()` au lieu d'un "
            "seul. Acter depuis une trame abandonnee ferait croire au suivi "
            "que la surface porte un calque qui n'y a jamais ete ecrit : il "
            "ne le redemanderait plus jamais." % total
        )


def regle_tick(code, fautes):
    """6. `tick()` regarde les calques avant le compteur de barre."""
    bloc = corps(code, "inline void tick()")
    if bloc is None:
        fautes.append("BouchaudChrome.h : `tick()` introuvable.")
        return
    calque = bloc.find("calques.degat()")
    barre = bloc.find("chrome_frames_pending")
    if calque < 0:
        fautes.append(
            "BouchaudChrome.h : `tick()` ne consulte plus le degat des "
            "calques. Une bulle qui disparait resterait a l'ecran jusqu'a la "
            "prochaine capture de page, qui peut ne jamais venir."
        )
        return
    if "compose_page(" not in bloc:
        fautes.append(
            "BouchaudChrome.h : `tick()` ne compose plus la page quand un "
            "calque bouge. `compose_toolbar_only()` ne touche aucun pixel de "
            "page : il ne peut pas restaurer ce qu'un calque laisse derriere."
        )
    if barre >= 0 and barre < calque:
        fautes.append(
            "BouchaudChrome.h : `tick()` traite la barre d'outils avant les "
            "calques. La barre consomme le tic et le calque attend le suivant."
        )


def regle_texte_unique(code, fautes):
    """7. Le texte des calques passe par `draw_ui_text`."""
    if corps(code, "inline void draw_ui_text(Canvas const& canvas") is None:
        fautes.append(
            "BouchaudChrome.h : `draw_ui_text` a disparu. C'est le point "
            "unique que modernise-v15.py remplace par le rendu Skia."
        )
        return
    for nom in re.findall(r"^inline void (dessine_[a-z_]+)\(Canvas", code, re.M):
        bloc = corps(code, "inline void %s(Canvas" % nom)
        if bloc is None:
            continue
        if "draw_text(" in bloc:
            fautes.append(
                "BouchaudChrome.h : `%s` appelle `draw_text` directement. "
                "Le texte du chrome passe par `draw_ui_text`, sinon ce calque "
                "restera en police bitmap quand V15 modernisera le reste." % nom
            )


def regle_ancres_v15(chrome, v15, fautes):
    """8. Les ancres de modernise-v15.py existent encore, mot pour mot."""
    ancres = re.findall(r"^anchor = '(.*)'$", v15, re.M)
    if not ancres:
        fautes.append("modernise-v15.py : plus aucune ancre `anchor = ...`.")
    for brute in ancres:
        ancre = brute.encode("utf-8").decode("unicode_escape").replace("\\'", "'")
        if ancre not in chrome:
            fautes.append(
                "modernise-v15.py vise une ancre absente de BouchaudChrome.h :\n"
                "    %r\n"
                "La substitution echouera au milieu de la construction." % ancre
            )

    bloc = re.search(r"^old = '''(.*?)'''$", v15, re.M | re.S)
    if not bloc:
        fautes.append(
            "modernise-v15.py : le corps de `draw_ui_text` a remplacer n'est "
            "plus une chaine `old = '''...'''`."
        )
    elif bloc.group(1) not in chrome:
        fautes.append(
            "modernise-v15.py : le corps de `draw_ui_text` a remplacer ne "
            "figure plus tel quel dans BouchaudChrome.h. Le chrome partirait "
            "avec la police de secours sans que rien ne le signale."
        )


def main():
    fautes = []
    chrome = texte(CHROME, fautes)
    calques = texte(CALQUES, fautes)
    v15 = texte(V15, fautes)
    texte(BANC, fautes)

    if chrome is not None and calques is not None:
        code = sans_commentaires(chrome)
        noms = calques_declares(code, fautes)
        regle_places(code, noms, fautes)
        regle_dessines(code, noms, fautes)
        regle_ordre_composition(code, fautes)
        regle_tick(code, fautes)
        regle_texte_unique(code, fautes)
        # Les ancres de V15 sont des commentaires de documentation : elles se
        # cherchent dans le texte brut, pas dans le code depouille.
        if v15 is not None:
            regle_ancres_v15(chrome, v15, fautes)

    if fautes:
        print("calques du chrome : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print("calques du chrome : les huit regles sont respectees")
    return 0


if __name__ == "__main__":
    sys.exit(main())
