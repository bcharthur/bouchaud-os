#!/usr/bin/env python3
"""Garde-fou : trois listes de fichiers du navigateur qui doivent s'accorder.

# Le defaut que ceci previent

Les binaires du navigateur -- BouchaudBrowserHost, WebContent, RequestServer,
ImageDecoder, Compositor, WebWorker, WebDriver -- ne sont pas dans le depot.
Ce sont des artefacts de construction.

Trois endroits parlent d'eux :

  * `.github/workflows/ladybird-native-browser.yml` PUBLIE l'artefact ;
  * `tools/reference/recupere-navigateur-ci.ps1` le RECUPERE et annonce
    « native-browser-m9 pret » ;
  * `tools/reference/prepare-reference-ladybird.ps1` l'EXIGE, fichier par
    fichier, et refuse s'il en manque un.

Si la liste du recuperateur derive de celle du preparateur, le premier
annonce « pret » sur un artefact que le second rejette -- et l'utilisateur
lit deux verdicts contradictoires sur le meme repertoire. Si le nom de
l'artefact derive de celui que la CI publie, le telechargement echoue sans
que personne sache pourquoi.

Ces listes ne coutent rien a tenir accordees, et beaucoup a debusquer quand
elles ne le sont plus : reconstruire Ladybird pour verifier prend des heures.

# Ce qui est verifie

1. Le recuperateur exige AU MOINS tout ce que le preparateur exige.
2. Il emploie le nom d'artefact que la CI publie.
3. Il dit que l'artefact a une duree de vie, et laquelle.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
RECUPERE = RACINE / "tools/reference/recupere-navigateur-ci.ps1"
PREPARE = RACINE / "tools/reference/prepare-reference-ladybird.ps1"
WORKFLOW = RACINE / ".github/workflows/ladybird-native-browser.yml"


def liste_apres(texte, ancre):
    """Les chaines citees du premier bloc @( ... ) qui suit `ancre`."""
    debut = texte.find(ancre)
    if debut < 0:
        return None
    ouvre = texte.find("@(", debut)
    if ouvre < 0:
        return None
    ferme = texte.find(")", ouvre)
    return set(re.findall(r'"([^"]+)"', texte[ouvre:ferme]))


def main():
    fautes = []
    for chemin in (RECUPERE, PREPARE, WORKFLOW):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1

    recupere = RECUPERE.read_text(encoding="utf-8")
    prepare = PREPARE.read_text(encoding="utf-8")
    workflow = WORKFLOW.read_text(encoding="utf-8")

    # --- 1. Le recuperateur couvre ce que le preparateur exige -----------
    exiges = liste_apres(prepare, "$Required = ")
    verifies = liste_apres(recupere, "foreach ($Nom in ")
    if exiges is None:
        fautes.append(
            "prepare-reference-ladybird.ps1 : la liste `$Required` est "
            "illisible ; l'accord des deux listes ne peut plus etre verifie."
        )
    elif verifies is None:
        fautes.append(
            "recupere-navigateur-ci.ps1 : la liste de verification est "
            "illisible ; il annoncerait « pret » sans avoir rien regarde."
        )
    else:
        manquants = sorted(exiges - verifies)
        if manquants:
            fautes.append(
                "recupere-navigateur-ci.ps1 ne verifie pas %s, que "
                "prepare-reference-ladybird.ps1 EXIGE.\n\n    Le recuperateur "
                "annoncerait « native-browser-m9 pret » sur un artefact que le "
                "preparateur rejette aussitot, et l'utilisateur lirait deux "
                "verdicts contradictoires sur le meme repertoire."
                % ", ".join("`%s`" % m for m in manquants)
            )

    # --- 2. Le nom d'artefact est celui que la CI publie -----------------
    publies = set(re.findall(r"name:\s*([a-z0-9-]*ladybird[a-z0-9-]*)", workflow))
    if not publies:
        fautes.append(
            "ladybird-native-browser.yml : aucun artefact Ladybird publie n'a "
            "ete trouve ; le recuperateur n'a plus de source."
        )
    else:
        demande = re.search(r'\$Artefact\s*=\s*"([^"]+)"', recupere)
        if not demande:
            fautes.append(
                "recupere-navigateur-ci.ps1 : le nom de l'artefact n'est plus "
                "une constante lisible."
            )
        elif demande.group(1) not in publies:
            fautes.append(
                "recupere-navigateur-ci.ps1 demande l'artefact `%s`, que la CI "
                "ne publie pas. Elle publie : %s. Le telechargement echouerait "
                "sans que le message dise pourquoi."
                % (demande.group(1), ", ".join(sorted(publies)))
            )

    # --- 3. La duree de vie de l'artefact est dite ----------------------
    if "QUATORZE JOURS" not in recupere and "quatorze jours" not in recupere:
        fautes.append(
            "recupere-navigateur-ci.ps1 ne dit plus que l'artefact expire. "
            "Une execution de plus de quatorze jours n'a plus d'artefact, et "
            "le telechargement echoue d'une facon qui ressemble a une panne "
            "d'outil plutot qu'a une retention expiree."
        )

    if fautes:
        print("recuperation navigateur : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "recuperation navigateur : les listes du recuperateur et du "
        "preparateur s'accordent, le nom d'artefact est celui que la CI "
        "publie, la retention est annoncee"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
