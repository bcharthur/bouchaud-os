#!/usr/bin/env python3
"""Refuse qu'un verrou tournant reste tenu pendant une attente du gros verrou.

Le controle est volontairement syntaxique : il attrape les formes de verrouillage
qui ont deja provoque un deadlock Bouchaud OS, sans pretendre faire une analyse
interprocedurale complete.

Deux formes sont refusees :
  1. match/if let/while let dont le sujet prend un verrou et dont un bras attend ;
  2. let garde = X.lock(); dont la portee traverse une attente sans drop(garde).

Les exceptions legitimes sont nommees et documentees dans AUDITS_NOMMES.
Une exception peut etre liee a "fichier:ligne" ou, pour un garde nomme, a
"fichier:nom_du_garde". Cette seconde forme evite de casser l'audit lorsqu'un
commentaire deplace simplement les numeros de lignes.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent / "src"

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

# Exception relue et nommee : TRANSACTION est declare dans
# src/fs/persistance/transaction.rs comme SleepMutex<()>. sync.rs est include!
# dans le meme module ; le controle syntaxique ne voit donc pas le type du
# receveur et le classait a tort comme verrou tournant.
AUDITS_NOMMES: dict[str, str] = {
    "src/fs/persistance/sync.rs:_transaction": (
        "TRANSACTION est un SleepMutex<()>, pas un SpinLock ; "
        "il peut bloquer la tache et n'est pas un verrou tournant."
    ),
}


def code(ligne: str) -> str:
    return ligne.split("//")[0]


def est_audite(relatif: str, numero: int, garde: str | None = None) -> bool:
    cles = [f"{relatif}:{numero}"]
    if garde:
        cles.append(f"{relatif}:{garde}")
    return any(cle in AUDITS_NOMMES for cle in cles)


def fin_de_bloc(lignes: list[str], depart: int) -> int | None:
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
        sujet = SUJET.match(c)
        if sujet and ".lock()" in c:
            fin = fin_de_bloc(lignes, i)
            if fin is not None:
                for j in range(i + 1, fin + 1):
                    if ATTENTE.search(code(lignes[j])):
                        if est_audite(relatif, i + 1):
                            break
                        fautes.append(
                            f"{relatif}:{i + 1}: le sujet du `{sujet.group(1)}` prend un verrou "
                            f"tournant, et la ligne {j + 1} attend dessous "
                            f"(« {code(lignes[j]).strip()[:70]} »).\n"
                            "    Le garde du sujet vit jusqu'a la fin de la construction. "
                            "Lie-le d'abord dans un `let`, il tombera au point-virgule."
                        )
                        break
            continue

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
                if est_audite(relatif, i + 1, nom):
                    break
                fautes.append(
                    f"{relatif}:{i + 1}: le garde `{nom}` est encore tenu ligne {j + 1} "
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
            f"{len(fautes)} cas. Chacun peut figer la machine entiere : un coeur tient\n"
            "le verrou tournant et attend le BKL, un autre tient le BKL et demande le\n"
            "verrou tournant. Aucun des deux ne rendra jamais rien."
        )
        return 1

    print("verrous tournants : aucun garde non audite ne traverse une attente du gros verrou.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
