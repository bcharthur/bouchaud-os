#!/usr/bin/env python3
"""Refuse qu'un verrou tournant reste tenu pendant une attente du gros verrou.

Un `SpinLock` ne dort pas : le cœur qui l'attend brule le processeur jusqu'a ce
que le detenteur le rende. Si le detenteur, lui, attend le BKL — ou se met a
dormir, ce qui revient au meme puisqu'il ne rendra rien avant d'etre reveilli —
alors tout cœur qui tient le BKL et demande ce verrou tournant s'y bloque pour
toujours. Les deux s'attendent, et la machine entiere s'arrete derriere eux.

C'est exactement ce qui est arrive le 25 aout dans `fs::backing::read_at_uncached` :

    let extent = match EXTENTS.lock().iter().find(...).copied() {
        Some(extent) => extent,
        None => { let _kernel = smp_lock::enter(); ... }
    };

Le garde temporaire du SUJET d'un `match` vit jusqu'a la fin de la construction,
donc jusque dans la branche qui prend le BKL. Rien ne le signale : ni le
compilateur, ni le boot, ni les sondes. Le noyau a tourne quinze minutes puis
s'est fige, un cœur tenant `EXTENTS` en attendant le BKL, un autre tenant le BKL
en demandant `EXTENTS`.

Ce script relit chaque fichier du noyau et refuse deux formes :

  1. `match` / `if let` / `while let` dont le SUJET prend un verrou, et dont un
     bras attend le BKL ou l'ordonnanceur — le cas ci-dessus ;
  2. `let garde = X.lock();` dont la portee traverse la meme attente sans que le
     garde ait ete rendu par `drop`.

Ce qu'il ne peut PAS faire : suivre un appel de fonction. Un garde tenu pendant
un appel qui, trois niveaux plus bas, prend le BKL, lui echappe. Il ferme la
forme syntaxique qui a effectivement mordu, et il la ferme completement.

Une exception legitime se declare dans `AUDITS_NOMMES` avec sa raison, pour
qu'elle se relise.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent / "src"

# Ce qui ne doit jamais se produire pendant qu'un verrou tournant est tenu.
ATTENTE = re.compile(
    r"smp_lock::enter\(\)"
    r"|smp_lock::try_enter"
    r"|suspend_for_schedule"
    r"|resume_after_schedule"
    r"|\bschedule\(\)"
    r"|sleep_ticks"
    r"|attends_un_tick"
    r"|attends_io_adaptatif"
    r"|attente_cedante"
    r"|park_current_on"
    r"|\.wait\("
    r"|shootdown_tlb"
    r"|block::read_blocks"
)

SUJET = re.compile(r"^\s*(?:\}\s*)?(?:let\s+.*?=\s*)?(match|if\s+let|while\s+let)\b")
LIAISON = re.compile(r"^\s*let\s+(?:mut\s+)?(\w+)\s*=\s*.+\.lock\(\)\s*;\s*$")

# Exceptions relues et nommees : « fichier:ligne » -> raison.
AUDITS_NOMMES: dict[str, str] = {}


def code(ligne: str) -> str:
    """La ligne sans son commentaire de fin, pour ne pas lire une prose."""
    return ligne.split("//")[0]


def fin_de_bloc(lignes: list[str], depart: int) -> int | None:
    """Indice de la ligne qui ferme le bloc ouvert a partir de `depart`."""
    profondeur = 0
    ouvert = False
    for i in range(depart, len(lignes)):
        c = code(lignes[i])
        profondeur += c.count("{") - c.count("}")
        if not ouvert and c.count("{"):
            ouvert = True
        if ouvert and profondeur <= 0:
            return i
    return None


def examine(chemin: Path) -> list[str]:
    lignes = chemin.read_text(encoding="utf-8", errors="replace").split("\n")
    relatif = chemin.relative_to(RACINE.parent).as_posix()
    fautes: list[str] = []

    for i, ligne in enumerate(lignes):
        c = code(ligne)

        # Forme 1 : le sujet prend un verrou, un bras attend.
        if SUJET.match(c) and ".lock()" in c:
            fin = fin_de_bloc(lignes, i)
            if fin is not None:
                for j in range(i + 1, fin + 1):
                    if ATTENTE.search(code(lignes[j])):
                        cle = f"{relatif}:{i + 1}"
                        if cle in AUDITS_NOMMES:
                            break
                        fautes.append(
                            f"{cle}: le sujet du `{SUJET.match(c).group(1)}` prend un verrou "
                            f"tournant, et la ligne {j + 1} attend dessous "
                            f"(« {code(lignes[j]).strip()[:70]} »).\n"
                            f"    Le garde du sujet vit jusqu'a la fin de la construction. "
                            f"Lie-le d'abord dans un `let`, il tombera au point-virgule."
                        )
                        break
            continue

        # Forme 2 : un garde nomme traverse une attente sans avoir ete rendu.
        liaison = LIAISON.match(c)
        if not liaison:
            continue
        nom = liaison.group(1)
        rendu = re.compile(r"\bdrop\(\s*" + re.escape(nom) + r"\s*\)")
        profondeur = 0
        for j in range(i + 1, len(lignes)):
            cj = code(lignes[j])
            if rendu.search(cj):
                break
            if ATTENTE.search(cj):
                cle = f"{relatif}:{i + 1}"
                if cle in AUDITS_NOMMES:
                    break
                fautes.append(
                    f"{cle}: le garde `{nom}` est encore tenu ligne {j + 1} "
                    f"(« {cj.strip()[:70]} »).\n"
                    f"    Rends-le par `drop({nom})` avant, ou restreins sa portee."
                )
                break
            profondeur += cj.count("{") - cj.count("}")
            if profondeur < 0:
                break

    return fautes


def main() -> int:
    fautes: list[str] = []
    for chemin in sorted(RACINE.rglob("*.rs")):
        fautes.extend(examine(chemin))

    if fautes:
        print("VERROUS TOURNANTS : attente du gros verrou sous un verrou tournant\n")
        for faute in fautes:
            print(f"  {faute}\n")
        print(
            f"{len(fautes)} cas. Chacun peut figer la machine entiere : un cœur tient\n"
            "le verrou tournant et attend le BKL, un autre tient le BKL et demande le\n"
            "verrou tournant. Aucun des deux ne rendra jamais rien."
        )
        return 1

    print("verrous tournants : aucun garde ne traverse une attente du gros verrou.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
