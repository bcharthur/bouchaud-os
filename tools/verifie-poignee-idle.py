#!/usr/bin/env python3
"""Verifie la poignee de main entre un coeur qui s'endort et celui qui le reveille.

LE MOTIF
--------
Deux coeurs, deux objets, deux ecritures suivies de deux lectures CROISEES :

    dormeur      publie IDLE=true,  puis relit SA FILE
    reveilleur   met en file,       puis relit IDLE

C'est le meme motif que `kernel::sync::rendezvous`, et il a la meme exigence :
si les deux cotes peuvent laisser passer leur lecture avant leur ecriture, ils
se manquent tous les deux. Le dormeur voit une file vide, le reveilleur voit un
coeur eveille, personne n'envoie d'IPI, et la tache ne repart jamais.

Sur x86 ce n'est pas theorique : le SEUL reordonnancement que la machine
s'autorise est justement store -> load. `Release`/`Acquire` ne l'interdisent
pas -- ils n'ordonnent une ecriture qu'avec la lecture qui la SUIT chez
l'autre, ce qui n'est pas ce motif.

CE QUI EST VERIFIE
------------------
  1. `idle_enter` publie `IDLE` en `SeqCst`. Sur x86 cette ecriture vide le
     tampon, donc la relecture de la file ne peut plus la depasser.
  2. `publish_ready` pose une BARRIERE `SeqCst` entre la mise en file et
     `is_idle`. Une lecture SeqCst ne suffirait pas : elle reste un `mov` et ne
     vide pas le tampon d'ecriture.
  3. `commit_scheduler_idle` relit la file AVANT son `hlt`. Sans cette
     relecture, l'ordre memoire ne sert a rien : il n'y a rien a lire.

Les trois sont necessaires ensemble. En retirer une seule rouvre la fenetre,
et le symptome est un noyau qui se fige sans rien imprimer -- c'est la
regression mm-ng6 SMP4, restee cinq minutes muette.

Code de retour : 0 si la poignee de main est complete.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
ETAT = RACINE / "src" / "arch" / "x86_64" / "cpu" / "idle" / "etat.rs"
ORDONNANCEUR = RACINE / "src" / "arch" / "x86_64" / "cpu" / "idle" / "scheduler.rs"
CREATION = RACINE / "src" / "kernel" / "process" / "thread" / "creation.rs"


def sans_commentaires(texte: str) -> str:
    """Retire les commentaires de ligne.

    Necessaire, et pas cosmetique : les commentaires de ce code PARLENT de
    `hlt` et de l'ordre memoire. Les laisser ferait couper l'analyse a la
    premiere explication au lieu de la premiere instruction -- ce garde-fou
    s'est accuse lui-meme ainsi a sa premiere execution.
    """
    return "\n".join(ligne.split("//")[0] for ligne in texte.splitlines())


def corps(chemin: Path, signature: str) -> str:
    """Le corps d'une fonction, par comptage d'accolades."""
    texte = sans_commentaires(chemin.read_text(encoding="utf-8"))
    debut = texte.find(signature)
    if debut < 0:
        raise SystemExit(
            f"ECHEC  `{signature}` introuvable dans {chemin.relative_to(RACINE)}.\n"
            "       La poignee de main a ete deplacee : mets ce chemin a jour,\n"
            "       sinon ce garde-fou ne verifie plus rien."
        )
    ouvrante = texte.index("{", debut)
    profondeur, fin = 0, ouvrante
    for i in range(ouvrante, len(texte)):
        if texte[i] == "{":
            profondeur += 1
        elif texte[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                fin = i
                break
    return texte[ouvrante : fin + 1]


def main() -> int:
    fautes = []

    publication = corps(ETAT, "fn idle_enter(")
    if not re.search(r"IDLE\[cpu\]\s*\.\s*store\(\s*true\s*,\s*Ordering::SeqCst\s*\)", publication):
        fautes.append(
            "idle_enter ne publie pas `IDLE` en SeqCst.\n"
            "    Le dormeur relit sa file juste apres : sans ordre total, la\n"
            "    relecture passe avant la publication et les deux cotes se\n"
            "    manquent."
        )

    reveil = corps(CREATION, "fn publish_ready(")
    avant_is_idle = reveil.split("is_idle(")[0]
    if "fence(Ordering::SeqCst)" not in avant_is_idle:
        fautes.append(
            "publish_ready n'a pas de barriere SeqCst entre la mise en file et\n"
            "    `is_idle`. Une lecture SeqCst ne suffit pas : sur x86 elle reste\n"
            "    un `mov` et ne vide pas le tampon d'ecriture."
        )

    sommeil = corps(ORDONNANCEUR, "pub fn commit_scheduler_idle(")
    avant_hlt = sommeil.split("hlt")[0]
    if "file_non_vide_essai" not in avant_hlt:
        fautes.append(
            "commit_scheduler_idle atteint `hlt` sans relire la file.\n"
            "    L'ordre memoire ne sert alors a rien : il n'y a rien a lire, et\n"
            "    le coeur s'endort avec une tache en file."
        )

    if fautes:
        print("ECHEC  poignee de main de l'endormissement incomplete\n")
        for faute in fautes:
            print(f"  {faute}\n")
        print(
            "Les trois moities sont necessaires ENSEMBLE. Voir l'en-tete de ce\n"
            "fichier et `src/kernel/sync/rendezvous.rs`, qui porte le meme argument."
        )
        return 1

    print("ok  poignee de main complete : publication SeqCst, barriere du "
          "reveilleur, relecture avant hlt")
    return 0


if __name__ == "__main__":
    sys.exit(main())
