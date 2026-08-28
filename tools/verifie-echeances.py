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

# Combien de lignes apres l'ecriture on accepte de chercher la declaration.
# L'armement suit toujours la fermeture du bloc qui pose l'etat.
PORTEE = 6

ECRITURE = re.compile(r"wake_deadline_ns\s*=\s*([^;]+);")


def main() -> int:
    lignes = THREAD.read_text(encoding="utf-8").splitlines()
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
            fautes.append(
                f"  thread.rs:{numero + 1}  `wake_deadline_ns = {valeur}` sans "
                f"`arme_echeance` dans les {PORTEE} lignes suivantes : cette "
                f"echeance serait invisible du raccourci, donc jamais servie"
            )

    if "fn arme_echeance(" not in "\n".join(lignes):
        fautes.append("  thread.rs  `arme_echeance` a disparu")

    # Et le raccourci lui-meme doit rester en place : sans lui, la regle
    # ci-dessus est inutile, mais la verifier ne coute rien.
    if "doit_balayer(" not in "\n".join(lignes):
        fautes.append(
            "  thread.rs  `wake_sleepers` ne consulte plus la borne : le "
            "balayage complet est revenu"
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
