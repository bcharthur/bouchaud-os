#!/usr/bin/env python3
"""Garde-fou : le compositeur ring 3 est EXECUTE, pas seulement compile.

`userland/services/composited/composited-slice.c` porte le tranchant vertical du
compositeur en espace utilisateur : demande de surface, transfert de capacite
attenuee, propriete des tampons, reveil par waitset, retour du tampon libere.

Il etait COMPILE par CI Fast et jamais EXECUTE -- un commentaire du workflow
affirmait meme le contraire. Compiler un contrat ne prouve que sa syntaxe : les
appels natifs, l'attenuation des droits et la propriete des tampons ne se
verifient qu'en ring 3, sur un vrai noyau.

Ce garde-fou verifie les quatre maillons de l'execution reelle :

  1. le harnais QEMU CONSTRUIT le service ;
  2. il le PLACE dans l'image, par une option que le fabricant d'image connait ;
  3. il EXIGE le verdict `COMPOSITED_SLICE_OK` ;
  4. les marqueurs exiges existent MOT POUR MOT dans le service.

Le quatrieme est le moins evident et le plus utile : un marqueur qui ne
correspond a rien fait echouer la CI, mais un marqueur RENOMME du cote du
service et pas du cote du harnais transformerait la verification en formalite
qu'aucune regression ne peut plus casser.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
SERVICE = RACINE / "userland/services/composited/composited-slice.c"
HARNAIS = RACINE / "tools/ci/run_native_ipc_probe.sh"
IMAGE = RACINE / "tools/native/make_native_ipc_probe_image.py"
CONSTRUCTION = RACINE / "userland/services/composited/build.sh"


def main() -> int:
    fautes = []
    for chemin in (SERVICE, HARNAIS, IMAGE, CONSTRUCTION):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
            return 1

    service = SERVICE.read_text(encoding="utf-8")
    harnais = HARNAIS.read_text(encoding="utf-8")
    image = IMAGE.read_text(encoding="utf-8")
    construction_brute = CONSTRUCTION.read_text(encoding="utf-8")
    # Les commentaires de ce script EXPLIQUENT les deux defauts, en les nommant.
    # Une regle qui lirait le fichier entier se declencherait sur sa propre
    # explication -- et la facon la plus rapide de la faire taire serait
    # d'effacer l'explication. On ne regarde donc que les commandes.
    construction = "\n".join(
        ligne for ligne in construction_brute.splitlines()
        if not ligne.lstrip().startswith("#")
    )

    # DEUX DEFAUTS QUI ONT EMPECHE CE SERVICE DE S'EXECUTER, ET LEURS GARDES.
    #
    # Les quatre maillons ci-dessous etaient TOUS verts pendant que le service
    # mourait au chargement, puis a sa quatrieme etape. Un garde-fou qui verifie
    # qu'un binaire est construit et lance ne dit rien de ce qui se passe une
    # fois qu'il est lance.
    #
    # 1. L'edition de liens produisait UN segment RWE (`-n`, plus
    #    `--no-warn-rwx-segments` pour faire taire l'avertissement). Le noyau le
    #    refusait au titre de W^X, et il avait raison.
    if " -static -n" in construction or " -n " in construction:
        fautes.append(
            "composited/build.sh : `-n` (nmagic) est revenu ; il fusionne code et "
            "donnees dans un unique segment RWE, que le noyau refuse au titre de W^X."
        )
    if "--no-warn-rwx-segments" in construction:
        fautes.append(
            "composited/build.sh : `--no-warn-rwx-segments` fait taire l'avertissement "
            "qui disait exactement ce qui n'allait pas. Corriger le segment, pas "
            "l'avertissement."
        )
    if "separate-code" not in construction:
        fautes.append(
            "composited/build.sh : `-z separate-code` a disparu ; rien ne garantit "
            "plus que le code et les donnees vivent dans des segments distincts."
        )
    if "RWE" not in construction:
        fautes.append(
            "composited/build.sh : la construction ne verifie plus ce qu'elle a "
            "produit. Un binaire refuse au chargement doit echouer a la "
            "construction, pas trois minutes plus tard sous la forme d'un marqueur "
            "manquant qui n'explique rien."
        )

    # 2. `_start` etait une fonction C ordinaire. Le noyau SAUTE a l'entree avec
    #    une pile alignee sur seize octets ; une fonction C suppose qu'un `call`
    #    vient d'empiler une adresse de retour. Le decalage de huit octets tuait
    #    le processus au premier `movaps` emis par le compilateur.
    if "andq $-16, %rsp" not in service:
        fautes.append(
            "composited-slice.c : `_start` ne realigne plus la pile. L'entree d'un "
            "processus n'est pas un appel de fonction : sans realignement, la "
            "premiere instruction qui exige seize octets -- un `movaps` de "
            "compilateur, par exemple -- leve une faute de protection generale."
        )

    if "userland/services/composited" not in harnais:
        fautes.append(
            "run_native_ipc_probe.sh : le service n'est plus construit par le harnais "
            "QEMU ; il redevient un binaire que personne n'execute."
        )
    if "--composited" not in harnais:
        fautes.append(
            "run_native_ipc_probe.sh : le service n'est plus place dans l'image ; "
            "l'autorun ne peut plus le lancer."
        )
    if "--composited" not in image:
        fautes.append(
            "make_native_ipc_probe_image.py : l'option `--composited` a disparu ; "
            "le harnais la passe dans le vide."
        )
    if "/bin/composited-slice" not in image:
        fautes.append(
            "make_native_ipc_probe_image.py : l'autorun ne lance plus "
            "`/bin/composited-slice`."
        )
    if "COMPOSITED_SLICE_OK" not in harnais:
        fautes.append(
            "run_native_ipc_probe.sh : le verdict du service n'est plus exige ; "
            "un tranchant qui echoue passerait inapercu."
        )
    if "COMPOSITED_SLICE_FAIL" not in harnais:
        fautes.append(
            "run_native_ipc_probe.sh : l'echec du service n'est plus detecte tot ; "
            "la CI attendrait l'echeance au lieu de dire pourquoi."
        )

    # Les marqueurs exiges doivent exister MOT POUR MOT dans le service.
    noms = set(re.findall(r'exige\([^;]*?"((?:[^"\\]|\\.)*)"', service, re.S))
    exiges = [
        ligne.strip().strip("'").split("[COMPOSITED] ok   ", 1)[1]
        for ligne in harnais.splitlines()
        if "[COMPOSITED] ok   " in ligne
    ]
    if not exiges:
        fautes.append(
            "run_native_ipc_probe.sh : plus aucun marqueur de succes du compositeur "
            "n'est exige ; `COMPOSITED_SLICE_OK` seul ne dit pas ce qui a ete prouve."
        )
    for attendu in exiges:
        if attendu not in noms:
            fautes.append(
                "run_native_ipc_probe.sh exige un marqueur que le service n'imprime "
                "pas : %r. Un marqueur renomme d'un seul cote ferait echouer la CI ; "
                "renomme des DEUX cotes." % attendu
            )

    if fautes:
        print("execution composited : %d probleme(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1

    print(
        "composited : construit, place dans l'image, execute en ring 3, "
        "%d marqueur(s) exige(s) et presents" % len(exiges)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
