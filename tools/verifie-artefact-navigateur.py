#!/usr/bin/env python3
"""Verifie qu'un job qui CONSOMME l'artefact Ladybird ne peut pas le cacher.

LE DEFAUT
---------
`ladybird-native-browser.yml` a deux jobs, et un seul fabrique quelque chose :

    ladybird / build once          -> upload-artifact    (producteur)
    ladybird / browser-host smoke  -> download-artifact   (consommateur)

`run.ps1` cherchait l'artefact avec `gh run list --status success`, c'est-a-dire
« le dernier run dont TOUS les jobs sont verts ». Le smoke fait tourner le
navigateur complet dans QEMU sans acceleration ; quand il depasse son delai, la
conclusion du RUN passe au rouge -- alors que l'artefact est publie, intact et
telechargeable.

`--status success` le sautait. L'image Ladybird de la machine de developpement
cessait donc de se mettre a jour a cause d'un job qui ne la fabrique pas, et le
symptome ne ressemblait pas du tout a sa cause : on voyait « les icones du
navigateur sont restees en pixels », pas « le selecteur d'artefact est trop
strict ».

LA REGLE
--------
Trois choses doivent rester vraies ensemble. Aucune ne suffit seule.

1. La selection ne gate PAS sur la conclusion du run. C'est le defaut lui-meme.

2. Le nom d'artefact que `run.ps1` telecharge est celui que le workflow
   televerse. Une derive de nom ne casse rien a la construction : elle se voit
   des mois plus tard, sur un poste, sous la forme d'un telechargement vide.

3. Le producteur televerse avec `if-no-files-found: error`. C'est CE reglage
   qui autorise la regle 1 : sans lui, un artefact pourrait exister en etant
   vide, et « l'artefact est la » cesserait de valoir « la construction a
   abouti ». Relacher ce reglage rendrait la selection permissive sans que rien
   d'autre ne bouge.

CE QUE CE VERIFICATEUR NE PEUT PAS VOIR
---------------------------------------
Qu'un troisieme job soit ajoute au workflow et devienne, lui aussi, producteur.
Il verifie la forme qui a produit le defaut : un consommateur ne doit pas
decider a la place d'un producteur.

Code de retour : 0 si les trois regles sont respectees.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
SCRIPT = RACINE / "run.ps1"
WORKFLOW = RACINE / ".github" / "workflows" / "ladybird-native-browser.yml"

ARTEFACT = "bouchaud-ladybird-native-browser"
WORKFLOW_NOM = "ladybird-native-browser.yml"

# La selection s'etend sur plusieurs lignes continuees par un accent grave.
# On la reconstitue avant de la lire, sinon `--status success` sur sa propre
# ligne echapperait a une recherche ligne par ligne.
CONTINUATION = re.compile(r"`\s*\n\s*")


def texte(chemin):
    if not chemin.exists():
        print("ECHEC  fichier absent : %s" % chemin.relative_to(RACINE))
        return None
    return chemin.read_text(encoding="utf-8")


def appels_gh_run_list(source):
    """Les appels `gh run list` de `run.ps1`, lignes de continuation repliees."""
    aplati = CONTINUATION.sub(" ", source)
    return [
        ligne.strip()
        for ligne in aplati.splitlines()
        if "gh run list" in ligne
    ]


def regle_selection(source, fautes):
    """1. Aucun `gh run list` sur le workflow navigateur ne filtre sur --status."""
    appels = appels_gh_run_list(source)
    vises = [appel for appel in appels if WORKFLOW_NOM in appel]

    if not vises:
        fautes.append(
            "run.ps1 : aucun `gh run list --workflow %s`. La selection de "
            "l'artefact a-t-elle ete deplacee ?" % WORKFLOW_NOM
        )
        return

    for appel in vises:
        if "--status" in appel:
            fautes.append(
                "run.ps1 : la selection de l'artefact filtre sur --status, "
                "donc sur la conclusion du RUN. Un job consommateur rouge "
                "cacherait un producteur sain.\n           %s" % appel
            )


def regle_nom(source, workflow, fautes):
    """2. Le nom telecharge est le nom televerse."""
    if ARTEFACT not in source:
        fautes.append(
            "run.ps1 : le nom d'artefact %r n'apparait pas ; il ne peut plus "
            "correspondre a ce que le workflow televerse." % ARTEFACT
        )
    if ARTEFACT not in workflow:
        fautes.append(
            "%s : le nom d'artefact %r n'apparait pas."
            % (WORKFLOW_NOM, ARTEFACT)
        )


def regle_televersement(workflow, fautes):
    """3. Le producteur refuse de televerser du vide."""
    # On cherche le bloc `upload-artifact` qui porte NOTRE nom, pas les autres
    # (le smoke televerse aussi des journaux, et lui a le droit d'etre laxiste).
    blocs = workflow.split("uses: actions/upload-artifact")
    producteur = [bloc for bloc in blocs[1:] if ARTEFACT in bloc.split("- name:")[0]]

    if not producteur:
        fautes.append(
            "%s : aucun bloc upload-artifact ne publie %r."
            % (WORKFLOW_NOM, ARTEFACT)
        )
        return

    if "if-no-files-found: error" not in producteur[0]:
        fautes.append(
            "%s : le televersement de %r n'a plus `if-no-files-found: error`. "
            "Un artefact vide deviendrait indiscernable d'une construction "
            "reussie, et la selection par artefact cesserait d'etre sure."
            % (WORKFLOW_NOM, ARTEFACT)
        )


def main():
    source = texte(SCRIPT)
    workflow = texte(WORKFLOW)
    if source is None or workflow is None:
        return 1

    fautes = []
    regle_selection(source, fautes)
    regle_nom(source, workflow, fautes)
    regle_televersement(workflow, fautes)

    if fautes:
        for faute in fautes:
            print("ECHEC  %s" % faute)
        return 1

    print("artefact navigateur : selection sur le producteur, nom accorde, "
          "televersement strict")
    return 0


if __name__ == "__main__":
    sys.exit(main())
