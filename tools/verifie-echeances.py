#!/usr/bin/env python3
"""Toute echeance posee doit etre declaree a la borne de reveil.

# Ce que ce garde-fou protege

`wake_sleepers()` ne balaie plus la table des taches a chaque `schedule()` : il
consulte d'abord une borne inferieure de la plus proche echeance. C'est ce qui
supprime des milliers de balayages par seconde, chacun sous le gros verrou.

Le raccourci n'est correct que si TOUTE echeance est connue de la borne. Une
seule pose oubliee et la tache dort au-dela de son delai -- un
`pthread_cond_timedwait` qui ne rend jamais la main, un `poll` qui rate son
echeance. Rien dans le journal ne le dirait ; on verrait une machine qui se
fige par moments.

C'est exactement la faute que `futex_wait` portait : l'echeance ne vivait que
dans une variable locale, `task.wake_deadline_ns` restait a zero, et l'attente
n'expirait que si cette tache-la reprenait la main d'elle-meme.

# La regle

Chaque ecriture de `wake_deadline_ns` a une valeur NON NULLE doit etre suivie,
dans les lignes voisines, d'un appel a `arme_echeance`. Remettre le champ a
zero n'a rien a declarer : cela ne peut que reculer le vrai minimum, et la
borne reste inferieure.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
THREAD = RACINE / "src" / "kernel" / "process" / "thread.rs"
THREAD_TREE = RACINE / "src" / "kernel" / "process" / "thread"

def lignes_de(*chemins) -> list[tuple[str, int, str]]:
    """Les lignes d'un sous-systeme, avec leur VRAIE origine.

    Concatener un arbre pour l'analyser est commode, mais rend les numeros de
    ligne faux -- et un garde-fou qui designe la mauvaise ligne fait perdre
    plus de temps qu'il n'en fait gagner. On garde donc, pour chaque ligne, le
    fichier et le numero d'ou elle vient.
    """
    sortie = []
    for chemin in chemins:
        fichiers = sorted(chemin.rglob("*.rs")) if chemin.is_dir() else (
            [chemin] if chemin.exists() else [])
        for fichier in fichiers:
            relatif = fichier.relative_to(RACINE).as_posix()
            for numero, texte in enumerate(
                    fichier.read_text(encoding="utf-8").splitlines(), start=1):
                sortie.append((relatif, numero, texte))
    return sortie


def source_de(*chemins) -> str:
    """Le code d'un sous-systeme, quel que soit son decoupage en fichiers.

    Ce garde-fou lisait un seul fichier. La fragmentation de `bkl.rs` en
    `bkl/**` l'a fait tomber sur une exception -- et comme rien ne l'executait,
    la regle a cesse de proteger quoi que ce soit sans que personne le voie.
    Lire un ARBRE plutot qu'un fichier retire cette facon de casser.
    """
    morceaux = []
    for chemin in chemins:
        if chemin.is_dir():
            for fichier in sorted(chemin.rglob("*.rs")):
                morceaux.append(fichier.read_text(encoding="utf-8"))
        elif chemin.exists():
            morceaux.append(chemin.read_text(encoding="utf-8"))
    return "\n".join(morceaux)


# Combien de lignes apres l'ecriture on accepte de chercher la declaration.
# L'armement suit toujours la fermeture du bloc qui pose l'etat.
PORTEE = 6

ECRITURE = re.compile(r"wake_deadline_ns\s*=\s*([^;]+);")


def main() -> int:
    entrees = lignes_de(THREAD, THREAD_TREE)
    lignes = [texte for _, _, texte in entrees]
    fautes = []

    for numero, ligne in enumerate(lignes):
        trouve = ECRITURE.search(ligne)
        if not trouve:
            continue
        valeur = trouve.group(1).strip()
        # Remettre le champ a zero n'a rien a declarer : cela ne peut que
        # reculer le vrai minimum, et la borne reste inferieure.
        if valeur == "0":
            continue
        voisinage = lignes[numero : numero + 1 + PORTEE]
        if not any("arme_echeance(" in l for l in voisinage):
            fichier, reelle, _ = entrees[numero]
            fautes.append(
                f"  {fichier}:{reelle}  `wake_deadline_ns = {valeur}` sans "
                f"`arme_echeance` dans les {PORTEE} lignes suivantes : cette "
                f"echeance serait invisible du raccourci, donc jamais servie"
            )

    if "fn arme_echeance(" not in "\n".join(lignes):
        fautes.append("  thread.rs  `arme_echeance` a disparu")

    # Et le raccourci lui-meme doit rester en place : sans lui, la regle
    # ci-dessus est inutile, mais la verifier ne coute rien.
    if "commence_balayage(" not in "\n".join(lignes):
        fautes.append(
            "  thread.rs  `wake_sleepers` ne revendique plus la borne : le "
            "balayage complet concurrent est revenu"
        )

    if fautes:
        print("echeances de reveil : regle violee")
        print("\n".join(fautes))
        return 1

    print(
        "ok  src/kernel/process/thread.rs : toute echeance posee est declaree "
        "a la borne de reveil"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
