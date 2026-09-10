#!/usr/bin/env python3
"""Garde-fou : aucune chaine d'appels ne deborde la pile noyau.

# Le defaut que ce garde-fou a ete ecrit pour empecher de revenir

`net::transport::tcp::fetch` posait sa file de retransmission sur la PILE :
`SEGMENTS_MAX` segments de `CHARGE_MAX` octets, soit quatre-vingt-treize
kibioctets, dans une pile noyau qui en fait soixante-quatre. Le prologue de la
fonction sortait `RSP` de la pile AVANT la premiere instruction de son corps,
puis la remise a zero de la file ecrivait trente kibioctets dans ce qui precede
la pile en memoire.

Le symptome etait un DOUBLE FAULT, et le double fault ne nomme ni la fonction
fautive ni la profondeur atteinte : il nomme la derniere instruction, qui est
innocente. Le defaut a vecu dans l'arbre sans qu'aucun test le voie, parce
qu'aucun test ne peut le voir -- il ne se manifeste que sur un chemin
d'execution reel, et il detruit alors le contexte qui aurait permis de le
comprendre.

Il est en revanche parfaitement visible a la COMPILATION : chaque fonction
declare sa trame dans son prologue.

# Les deux regles

  1. **Aucune trame unique ne depasse la moitie de la pile utilisable.** Une
     seule fonction qui reclame la moitie de la pile est une faute de
     conception, quelle que soit la profondeur de la chaine qui l'appelle : il
     ne reste alors rien pour ce qu'elle appelle a son tour.
  2. **Aucune chaine partant d'un point d'entree ne depasse la pile
     utilisable.** C'est la propriete qui compte vraiment, et c'est celle
     qu'une trame prise isolement ne dit pas.

# Ce garde-fou a besoin d'un noyau CONSTRUIT, ET DU BON

Il lit un binaire. En integration continue, le noyau est compile avant les
garde-fous et son absence est donc une FAUTE -- le rendre permissif rendrait
vert un garde-fou qui ne verifie rien, ce qui est pire que rouge. En local, sur
un arbre jamais construit, l'absence n'est pas une faute, et le message dit
quoi lancer.

Le PROFIL compte autant que la presence. Les deux artefacts livres -- le
bootimage BIOS et l'image UEFI du TRIGKEY -- sortent du profil `dev`. Une
analyse faite sur un binaire `release` ne dirait rien de ce qui est flashe :
l'inlining y fusionne des trames et en supprime d'autres. Le binaire analyse
est donc NOMME dans la sortie, profil compris, pour qu'aucun lecteur n'ait a
le deviner.
"""

import os
import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
MESURE = RACINE / "tools/mesure-pile-noyau.py"
MODELES = RACINE / "src/kernel/process/thread/modeles.rs"

# Dans l'ordre de preference, et TOUS en profil `dev` : c'est celui des deux
# artefacts livres. `BO_NOYAU_ANALYSE` permet d'en designer un autre -- pour
# analyser exactement le noyau d'une image deja flashee, par exemple.
BINAIRES = [
    "target/x86_64-bouchaud_os/debug/bouchaud-os",
    "target/x86_64-bouchaud_os_uefi/debug/bouchaud-os",
]

# Les points d'entree d'une pile noyau. Une tache commence a son trampoline ;
# la boucle du gestionnaire de fenetres est ajoutee parce qu'elle est la plus
# longue chaine du systeme et celle que le bureau execute en permanence.
RACINES = [
    "task_trampoline",
    "kernel_task_trampoline",
    "window_manager6boucle",
]


def budget_pile() -> tuple[int, int]:
    """(taille d'allocation, octets utilisables), lus dans le code."""
    texte = MODELES.read_text(encoding="utf-8")
    m = re.search(r"const KSTACK_SIZE: usize = ([^;]+);", texte)
    garde = re.search(r"pub const GARDE_PILE: usize = ([^;]+);", texte)
    if not m:
        raise SystemExit("modeles.rs : KSTACK_SIZE introuvable")
    taille = eval(m.group(1).replace("_", ""), {"__builtins__": {}})
    protege = eval(garde.group(1).replace("_", ""), {"__builtins__": {}}) if garde else 0
    return taille, taille - protege


def main() -> int:
    taille, utilisable = budget_pile()
    impose = os.environ.get("BO_NOYAU_ANALYSE")
    if impose:
        if not Path(impose).is_file():
            print("pile noyau : BO_NOYAU_ANALYSE designe un fichier absent : %s" % impose)
            return 1
        binaire = impose
    else:
        binaire = next((b for b in BINAIRES if (RACINE / b).is_file()), None)
    if binaire is None:
        if os.environ.get("CI"):
            print("pile noyau : aucun binaire construit, et l'integration continue")
            print("             en compile un avant les garde-fous. C'est une faute.")
            for b in BINAIRES:
                print("             attendu : %s" % b)
            return 1
        print("pile noyau : non mesuree, aucun noyau construit.")
        print("  construire d'abord : cargo build")
        return 0

    argv = [
        sys.executable,
        str(MESURE),
        binaire,
        "--budget", str(utilisable),
        "--trames", str(utilisable // 2),
    ]
    for racine in RACINES:
        argv += ["--racine", racine]

    resultat = subprocess.run(argv, capture_output=True, text=True, cwd=RACINE)
    if resultat.returncode != 0:
        print(resultat.stdout.rstrip())
        print(resultat.stderr.rstrip())
        return 1

    # UN BINAIRE PERIME EST UN GARDE-FOU QUI NE VERIFIE RIEN.
    #
    # L'analyse porte sur ce qui a ete COMPILE. Si des sources ont change
    # depuis, le chiffre rendu decrit un noyau qui n'existe plus -- et il le
    # decrit en vert, ce qui est la pire des reponses.
    binaire_ts = (RACINE / binaire).stat().st_mtime if not impose else Path(binaire).stat().st_mtime
    plus_recent = None
    for dossier in ("src", "targets"):
        base = RACINE / dossier
        if not base.is_dir():
            continue
        for chemin in base.rglob("*"):
            if chemin.is_file() and chemin.suffix in (".rs", ".json"):
                ts = chemin.stat().st_mtime
                if plus_recent is None or ts > plus_recent[0]:
                    plus_recent = (ts, chemin)
    if plus_recent and plus_recent[0] > binaire_ts:
        print("pile noyau : le binaire analyse est PERIME.")
        print("  binaire  : %s" % binaire)
        print("  plus recent : %s" % plus_recent[1].relative_to(RACINE).as_posix())
        print("  reconstruire d'abord : cargo build")
        return 1

    profil = "release" if "/release/" in binaire else "dev"
    print(
        "pile noyau : %s (profil %s) ; pile de %d octets dont %d utilisables ; "
        "aucune trame au-dela de %d, aucune chaine au-dela de %d"
        % (binaire, profil, taille, utilisable, utilisable // 2, utilisable)
    )
    if profil == "release":
        print(
            "  ATTENTION : les artefacts livres sortent du profil dev. Une mesure "
            "release ne dit rien de ce qui est flashe."
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
