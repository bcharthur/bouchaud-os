#!/usr/bin/env python3
"""Verifie l'ordre de prise des verrous du cache de pages propres.

Un seul ordre est permis :

    CACHE  ->  Entry::state

Une inversion -- un chemin qui tient `state` et demande `CACHE` pendant qu'un
autre tient `CACHE` et demande `state` -- bloque les deux CPU pour toujours.
Rien ne le signale : aucune assertion, aucun compteur, le noyau cesse
simplement d'exister. Et cela ne se trouve pas par la mesure, puisqu'il faut
que l'entrelacement se produise.

Ce verificateur lit la source. Il suit les gardes d'etat NOMMES, par profondeur
d'accolades, et echoue si `CACHE.lock()` apparait alors qu'un garde est encore
vivant.

CE QU'IL NE PEUT PAS VOIR
-------------------------
La duree de vie d'un garde TEMPORAIRE. Dans

    *entry.state.lock() = State::Failed;

le garde meurt a la fin de l'instruction -- donc avant un `CACHE.lock()` a la
ligne suivante. C'est correct, mais la surete tient alors a une regle de duree
de vie plutot qu'a quelque chose de visible, et un refactor ordinaire qui nomme
le garde suffit a produire un interblocage.

Ce verificateur refuse donc cette forme : un garde d'etat qui MUTE doit etre
nomme. L'analyse devient decidable, et l'ordre se lit au lieu de se deduire.

Code de retour : 0 si l'ordre est respecte.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
CIBLE = RACINE / "src" / "kernel" / "memory" / "page_cache.rs"

# `let mut etat = entry.state.lock();`  /  `let etat = old.state.lock();`
LIAISON = re.compile(r"\blet\s+(?:mut\s+)?(\w+)\s*=\s*[\w.]*\bstate\.lock\(\)")
# `drop(etat);`
LIBERATION = re.compile(r"\bdrop\s*\(\s*(\w+)\s*\)")
# `*entry.state.lock() = ...` : garde temporaire qui mute, interdit.
TEMPORAIRE_MUTANT = re.compile(r"\*\s*[\w.]*\bstate\.lock\(\)\s*=")
PRISE_CACHE = re.compile(r"\bCACHE\s*\.\s*lock\s*\(\)")


def sans_commentaires(ligne: str) -> str:
    """Retire les commentaires de ligne. Suffisant : ce fichier n'a pas de
    commentaire de bloc a l'interieur d'un corps de fonction."""
    position = ligne.find("//")
    return ligne if position < 0 else ligne[:position]


def verifie(chemin: Path) -> list[str]:
    fautes: list[str] = []
    profondeur = 0
    # nom du garde -> profondeur d'accolades a sa liaison
    vivants: dict[str, int] = {}

    for numero, brute in enumerate(chemin.read_text(encoding="utf-8").splitlines(), 1):
        ligne = sans_commentaires(brute)

        if TEMPORAIRE_MUTANT.search(ligne):
            fautes.append(
                f"{chemin.name}:{numero} garde d'etat TEMPORAIRE qui mute.\n"
                f"           {brute.strip()}\n"
                "           Nomme-le et relache-le explicitement : sa duree de "
                "vie doit se lire,\n"
                "           pas se deduire d'une regle sur les temporaires."
            )

        liaison = LIAISON.search(ligne)
        if liaison:
            vivants[liaison.group(1)] = profondeur

        if vivants and PRISE_CACHE.search(ligne):
            tenus = ", ".join(sorted(vivants))
            fautes.append(
                f"{chemin.name}:{numero} CACHE.lock() alors que le garde d'etat "
                f"`{tenus}` est encore vivant.\n"
                f"           {brute.strip()}\n"
                "           C'est l'ordre state -> CACHE, donc un interblocage "
                "SMP en puissance.\n"
                "           Relache le garde avant de prendre CACHE."
            )

        for nom in LIBERATION.findall(ligne):
            vivants.pop(nom, None)

        profondeur += ligne.count("{") - ligne.count("}")
        # Un garde meurt avec le bloc qui le contient.
        for nom in [n for n, d in vivants.items() if d > profondeur]:
            del vivants[nom]

    return fautes


def main() -> int:
    if not CIBLE.exists():
        print(f"ECHEC  fichier introuvable : {CIBLE}")
        return 1
    fautes = verifie(CIBLE)
    if fautes:
        print("ECHEC  ordre des verrous du cache de pages propres\n")
        for faute in fautes:
            print(f"  {faute}\n")
        print(
            "L'ordre unique est CACHE -> Entry::state. Voir l'en-tete de\n"
            "src/kernel/memory/page_cache.rs et tools/smp/test_ordre_verrous.rs."
        )
        return 1
    print(f"ok  {CIBLE.relative_to(RACINE)} : ordre CACHE -> Entry::state respecte")
    return 0


if __name__ == "__main__":
    sys.exit(main())
