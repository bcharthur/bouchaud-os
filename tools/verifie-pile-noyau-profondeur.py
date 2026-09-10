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

# Ce garde-fou a besoin d'un noyau CONSTRUIT

Il lit un binaire. En integration continue, le noyau est compile avant les
garde-fous et son absence est donc une FAUTE. En local, sur un arbre jamais
construit, elle ne l'est pas -- et le message dit quoi lancer.
"""

import os
import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
MESURE = RACINE / "tools/mesure-pile-noyau.py"
MODELES = RACINE / "src/kernel/process/thread/modeles.rs"

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

    print(
        "pile noyau : pile de %d octets dont %d utilisables ; aucune trame au-dela "
        "de %d, aucune chaine au-dela de %d" % (taille, utilisable, utilisable // 2, utilisable)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
