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
Cinq choses doivent rester vraies ensemble. Aucune ne suffit seule.

1. La selection ne gate PAS sur la conclusion du run. C'est le defaut lui-meme.

2. Le nom d'artefact que `run.ps1` telecharge est celui que le workflow
   televerse. Une derive de nom ne casse rien a la construction : elle se voit
   des mois plus tard, sur un poste, sous la forme d'un telechargement vide.

3. Le marqueur de capacite que le producteur ECRIT est LU par `run.ps1`.
   Le workflow stampe `V16_UI_CAPABLE` dans l'artefact pour dire qu'il porte
   le chrome moderne. Personne ne le lisait : la completude se jugeait sur
   `M9_CAPABLE`, present dans un artefact d'avant ce chantier. Un artefact
   telecharge il y a des mois passait donc le controle indefiniment, et
   aucune correction de l'interface n'atteignait la machine. Un marqueur de
   capacite qui n'est jamais lu ne protege de rien.

4. `run.ps1` filtre le JSON en PowerShell, JAMAIS par `--jq`.
   Windows PowerShell 5.1 reconstruit une ligne de commande pour lancer un
   programme natif et mange les guillemets doubles d'un argument. Un
   programme jq contenant `select(.name == "...")` arrivait a `gh` sans ses
   guillemets, et jq lisait une suite de soustractions suivie d'un appel a
   une fonction inexistante : "function not defined: browser/0". L'erreur
   n'accusait ni gh ni PowerShell -- elle ressemblait a une panne de
   l'outil. `ConvertFrom-Json` supprime la classe entiere.

5. Le producteur televerse avec `if-no-files-found: error`. C'est CE reglage
   qui autorise la regle 1 : sans lui, un artefact pourrait exister en etant
   vide, et « l'artefact est la » cesserait de valoir « la construction a
   abouti ». Relacher ce reglage rendrait la selection permissive sans que rien
   d'autre ne bouge.

CE QUE CE VERIFICATEUR NE PEUT PAS VOIR
---------------------------------------
Qu'un troisieme job soit ajoute au workflow et devienne, lui aussi, producteur.
Il verifie la forme qui a produit le defaut : un consommateur ne doit pas
decider a la place d'un producteur.

Code de retour : 0 si les cinq regles sont respectees.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
# La selection a demenage dans son propre fichier pour devenir testable. Le
# verificateur lit les DEUX : une regle qui ne regarde plus le code qu'elle
# protege ne protege plus rien, et rien ne l'aurait signale.
SCRIPTS = [
    RACINE / "run.ps1",
    RACINE / "tools" / "ladybird" / "selection-artefact.ps1",
]
WORKFLOW = RACINE / ".github" / "workflows" / "ladybird-native-browser.yml"

ARTEFACT = "bouchaud-ladybird-native-browser"
WORKFLOW_NOM = "ladybird-native-browser.yml"
# Les marqueurs de capacite sont DECOUVERTS dans le workflow, pas enumeres ici.
#
# Il y en avait un. Il y en a deux, et il y en aura d'autres : chaque
# generation d'interface en ajoute un, parce qu'un marqueur ne peut dire que ce
# qu'il savait au moment ou il a ete ecrit. Une liste tenue ici serait la
# premiere chose qu'on oublierait de completer -- et le defaut qu'elle laisse
# passer est exactement celui que ce fichier existe pour attraper.
MARQUEUR = re.compile(r"> third_party/native-browser-bouchaud/(\w+_UI_CAPABLE)")

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


def regle_selection(nom, source, fautes):
    """1. Aucun `gh run list` sur le workflow navigateur ne filtre sur --status."""
    appels = appels_gh_run_list(source)
    vises = [appel for appel in appels if WORKFLOW_NOM in appel]

    if not vises:
        return

    for appel in vises:
        if "--status" in appel:
            fautes.append(
                "%s : la selection de l'artefact filtre sur --status, donc "
                "sur la conclusion du RUN. Un job consommateur rouge "
                "cacherait un producteur sain.\n           %s" % (nom, appel)
            )


def regle_nom(source, workflow, fautes):
    """2. Le nom telecharge est le nom televerse."""
    if ARTEFACT not in source:
        fautes.append(
            "le nom d'artefact %r n'apparait dans aucun script ; il ne peut "
            "plus correspondre a ce que le workflow televerse." % ARTEFACT
        )
    if ARTEFACT not in workflow:
        fautes.append(
            "%s : le nom d'artefact %r n'apparait pas."
            % (WORKFLOW_NOM, ARTEFACT)
        )


def listes_requises(source):
    """Chaque `$RequiredLadybirdFiles = @( ... )` du script, une par branche."""
    return re.findall(r"\$RequiredLadybirdFiles\s*=\s*@\((.*?)\)", source, re.S)


def regle_capacite(source, workflow, fautes):
    """3. Chaque marqueur ecrit par le producteur est exige par le consommateur.

    Et reciproquement. Les deux sens comptent, et ils echouent differemment :

      * un marqueur ECRIT que personne n'exige laisse un artefact perime passer
        pour complet -- l'interface d'avant reste en place, et le symptome ne
        ressemble pas a sa cause ;
      * un marqueur EXIGE que personne n'ecrit rend tout artefact incomplet,
        donc retelecharge a chaque lancement, indefiniment.

    La verification porte sur CHAQUE branche de la liste des fichiers requis,
    et non sur leur reunion : `run.ps1` en a plusieurs, et un marqueur present
    dans l'une seulement laisse les autres sans protection. Seule la branche M8
    est exempte -- elle affiche une page locale fixe, sans barre d'adresse ni
    boutons --, et elle se reconnait a ce qu'elle n'exige pas `M9_CAPABLE`.
    """
    marqueurs = sorted(set(MARQUEUR.findall(workflow)))
    if not marqueurs:
        fautes.append(
            "%s : le producteur n'ecrit plus aucun marqueur de capacite. Un "
            "artefact d'une generation quelconque passerait le controle de "
            "completude." % WORKFLOW_NOM
        )
        return

    listes = listes_requises(source)
    if not listes:
        fautes.append("run.ps1 : la liste des fichiers requis a disparu.")
        return

    # Un marqueur peut etre exige par son nom ou par la variable qui le porte.
    variables = dict(
        re.findall(r'\$(\w+)\s*=\s*"(\w+_UI_CAPABLE)"', source)
    )

    def exiges_par(liste):
        noms = set(re.findall(r"\w+_UI_CAPABLE", liste))
        for variable in re.findall(r"\$(\w+)", liste):
            if variable in variables:
                noms.add(variables[variable])
        return noms

    tous_exiges = set()
    for liste in listes:
        exiges = exiges_par(liste)
        tous_exiges |= exiges
        if "M9_CAPABLE" not in liste:
            continue  # la branche M8, exemptee et documentee
        for marqueur in marqueurs:
            if marqueur in exiges:
                continue
            fautes.append(
                "run.ps1 : le producteur ecrit %r, et une des listes de "
                "fichiers requis ne l'exige pas. Un artefact anterieur a cette "
                "generation passerait le controle de completude et resterait "
                "en place -- avec l'interface d'avant, et sans que rien ne le "
                "dise." % marqueur
            )

    for marqueur in sorted(tous_exiges - set(marqueurs)):
        fautes.append(
            "run.ps1 : %r est exige, et le producteur ne l'ecrit pas. Aucun "
            "artefact ne passerait jamais le controle de completude : "
            "`run.ps1` retelechargerait a chaque lancement." % marqueur
        )


def regle_sans_jq(nom, source, fautes):
    """4. Aucun `--jq` : le filtrage se fait en PowerShell."""
    for numero, ligne in enumerate(source.splitlines(), start=1):
        # Le commentaire qui explique la regle a le droit de nommer `--jq`.
        nue = ligne.split("#", 1)[0]
        if "--jq" in nue:
            fautes.append(
                "%s:%d : `--jq` passe un programme jq a un programme natif. "
                "Windows PowerShell 5.1 mange les guillemets doubles d'un tel "
                "argument. Filtrer avec ConvertFrom-Json.\n"
                "           %s" % (nom, numero, ligne.strip())
            )


def regle_televersement(workflow, fautes):
    """5. Le producteur refuse de televerser du vide."""
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
    workflow = texte(WORKFLOW)
    if workflow is None:
        return 1

    sources = {}
    for chemin in SCRIPTS:
        contenu = texte(chemin)
        if contenu is None:
            return 1
        sources[chemin.relative_to(RACINE).as_posix()] = contenu

    ensemble = "\n".join(sources.values())

    fautes = []
    for nom, source in sources.items():
        regle_selection(nom, source, fautes)
        regle_sans_jq(nom, source, fautes)

    # `gh run list` doit exister QUELQUE PART : c'est la selection elle-meme.
    if not any(appels_gh_run_list(source) for source in sources.values()):
        fautes.append(
            "aucun `gh run list --workflow %s` dans %s. La selection de "
            "l'artefact a-t-elle ete deplacee ?"
            % (WORKFLOW_NOM, " ni ".join(sources))
        )

    regle_nom(ensemble, workflow, fautes)
    regle_capacite(ensemble, workflow, fautes)
    regle_televersement(workflow, fautes)

    if fautes:
        for faute in fautes:
            print("ECHEC  %s" % faute)
        return 1

    print("artefact navigateur : selection sur le producteur, nom accorde, "
          "capacite UI lue, sans --jq, televersement strict")
    return 0


if __name__ == "__main__":
    sys.exit(main())
