#!/usr/bin/env python3
"""Garde-fou : les piles noyau gardent leur canari, et quelqu'un le regarde.

Un debordement de pile noyau n'a AUCUN symptome propre. Les piles sont des
allocations de 64 Kio du tas, sans page de garde : quand une pile deborde, elle
ecrit dans le tas voisin, silencieusement. La faute apparait ailleurs, dans un
sous-systeme sans rapport -- ou nulle part, jusqu'a ce qu'un `RSP` finisse hors
de toute region valide et que le processeur double-faute.

C'est exactement ce qui a ete observe sur la machine de reference.

Trois maillons rendent ce defaut reperable, et en casser un seul suffit a
retrouver le silence :

  1. le canari est POSE a la creation de chaque pile ;
  2. il est RELU a chaque election de la tache ;
  3. sa rupture est FATALE -- continuer propagerait une corruption deja ecrite.

Aucun test ne peut retrouver ces trois-la apres coup : ils ne se voient que
lorsqu'un debordement se produit, c'est-a-dire jamais pendant une campagne de
test. D'ou ce garde-fou.
"""

import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
TACHE = RACINE / "src/kernel/process/thread/tache.rs"
CREATION = RACINE / "src/kernel/process/thread/creation.rs"
COMMUTATION = RACINE / "src/kernel/process/thread/commutation.rs"

# Un canari nul, ou fait d'un octet repete, ressemble par accident a de la
# memoire fraiche ou remise a zero. Ces valeurs sont refusees nommement.
CANARIS_REFUSES = {"0", "0x0", "0xFFFFFFFFFFFFFFFF", "0xffffffffffffffff"}


def main() -> int:
    fautes = []

    for chemin in (TACHE, CREATION, COMMUTATION):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
            return 1

    tache = TACHE.read_text(encoding="utf-8")
    creation = CREATION.read_text(encoding="utf-8")
    commutation = COMMUTATION.read_text(encoding="utf-8")

    if "CANARI_PILE" not in tache:
        fautes.append(
            "tache.rs : `CANARI_PILE` a disparu ; les piles noyau n'ont plus de temoin."
        )
    else:
        ligne = next(
            (l for l in tache.splitlines() if "pub const CANARI_PILE" in l), ""
        )
        valeur = ligne.split("=")[-1].strip().rstrip(";").replace("_", "")
        if valeur.lower() in CANARIS_REFUSES:
            fautes.append(
                "tache.rs : `CANARI_PILE` vaut %s ; une valeur triviale ressemble a de "
                "la memoire fraiche et ne temoigne de rien." % valeur
            )

    if "pose_le_canari" not in creation:
        fautes.append(
            "creation.rs : le canari n'est plus pose a la creation d'une pile ; "
            "il n'y a plus rien a relire."
        )
    if "pub fn pile_intacte" not in creation:
        fautes.append("creation.rs : `pile_intacte` a disparu ; plus rien ne relit le canari.")

    if "verifie_le_canari" not in commutation:
        fautes.append(
            "commutation.rs : le canari n'est plus relu a l'election d'une tache ; "
            "un debordement redevient silencieux."
        )
    else:
        debut = commutation.find("fn verifie_le_canari")
        fin = commutation.find("\nfn install", debut)
        corps = commutation[debut:fin if fin > 0 else len(commutation)]
        if "panic!" not in corps:
            fautes.append(
                "commutation.rs : une pile debordee ne panique plus. Elle a DEJA ecrit "
                "dans le tas voisin ; continuer propage une corruption dont le symptome "
                "apparaitra dans un sous-systeme sans rapport."
            )
        if "install(task" not in commutation and "fn install" not in commutation:
            fautes.append("commutation.rs : `install` a disparu ; le point de controle n'a plus d'entonnoir.")
        else:
            apres = commutation[commutation.find("fn install(task"):]
            if "verifie_le_canari(task)" not in apres[:400]:
                fautes.append(
                    "commutation.rs : `install` ne verifie plus le canari en entree."
                )

    if fautes:
        print("pile noyau : %d probleme(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1

    print("pile noyau : canari pose a la creation, relu a chaque election, rupture fatale")
    return 0


if __name__ == "__main__":
    sys.exit(main())
