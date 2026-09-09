#!/usr/bin/env python3
"""Garde-fou : les piles noyau ont une page de garde, et quelqu'un la regarde.

Un debordement de pile noyau n'a AUCUN symptome propre. Les piles sont des
allocations de 64 Kio du tas : quand une pile deborde, elle ecrit dans le tas
voisin, silencieusement. La faute apparait ailleurs, dans un sous-systeme sans
rapport -- ou nulle part, jusqu'a ce qu'un `RSP` finisse hors de toute region
valide et que le processeur double-faute.

C'est exactement ce qui a ete observe sur la machine de reference.

Quatre maillons rendent ce defaut reperable, et en casser un seul suffit a
retrouver le silence :

  1. une PAGE de garde est reservee au pied de chaque pile, et remplie ;
  2. son HAUT -- ce qu'une pile qui descend touche en premier -- est RELU a
     chaque election de la tache ;
  3. sa rupture est FATALE -- continuer propagerait une corruption deja ecrite ;
  4. la PROFONDEUR atteinte est publiee, parce qu'un depassement de quelques
     centaines d'octets et un `RSP` perdu ne se cherchent pas au meme endroit.

Aucun test ne peut retrouver ces quatre-la apres coup : ils ne se voient que
lorsqu'un debordement se produit, c'est-a-dire jamais pendant une campagne de
test. D'ou ce garde-fou.
"""

import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
TACHE = RACINE / "src/kernel/process/thread/tache.rs"
CREATION = RACINE / "src/kernel/process/thread/creation.rs"
COMMUTATION = RACINE / "src/kernel/process/thread/commutation.rs"
MODELES = RACINE / "src/kernel/process/thread/modeles.rs"

# Un canari nul, ou fait d'un octet repete, ressemble par accident a de la
# memoire fraiche ou remise a zero. Ces valeurs sont refusees nommement.
CANARIS_REFUSES = {"0", "0x0", "0xFFFFFFFFFFFFFFFF", "0xffffffffffffffff"}


def main() -> int:
    fautes = []

    for chemin in (TACHE, CREATION, COMMUTATION, MODELES):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
            return 1

    tache = TACHE.read_text(encoding="utf-8")
    creation = CREATION.read_text(encoding="utf-8")
    commutation = COMMUTATION.read_text(encoding="utf-8")
    modeles = MODELES.read_text(encoding="utf-8")

    ligne = next((l for l in modeles.splitlines() if "pub const GARDE_PILE" in l), "")
    if not ligne:
        fautes.append(
            "modeles.rs : `GARDE_PILE` a disparu ; les piles noyau n'ont plus de page "
            "de garde, et un debordement de moins de 60 Kio redevient invisible."
        )
    else:
        try:
            octets = int(ligne.split("=")[-1].strip().rstrip(";").replace("_", ""), 0)
        except ValueError:
            octets = 0
        if octets < 4096:
            fautes.append(
                "modeles.rs : `GARDE_PILE` vaut %s ; moins d'une page ne couvre plus "
                "un debordement ordinaire." % octets
            )

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
    if "GARDE_MOTS - CANARI_MOTS" not in creation:
        fautes.append(
            "creation.rs : le controle chaud ne lit plus le HAUT de la garde. Une pile "
            "descend : relire le bas ne temoigne que des depassements de plus de 60 Kio, "
            "c'est-a-dire de presque aucun."
        )
    if "pub fn profondeur_dans_la_garde" not in creation:
        fautes.append(
            "creation.rs : `profondeur_dans_la_garde` a disparu ; la rupture ne dit plus "
            "de combien la pile a deborde."
        )

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
        if "profondeur_dans_la_garde()" not in corps:
            fautes.append(
                "commutation.rs : la rupture ne mesure plus la profondeur atteinte ; "
                "le releve ne distingue plus un debordement d'un RSP perdu."
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

    print(
        "pile noyau : page de garde posee a la creation, haut relu a chaque election, "
        "rupture fatale et mesuree"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
