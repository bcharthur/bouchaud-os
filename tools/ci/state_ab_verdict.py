#!/usr/bin/env python3
"""BOUCHAUD_STATE_AB_V1 -- verdict du banc STATE (tools/ci/run_state_ab.sh).

    state_ab_verdict.py boot-1.log boot-2.log boot-3.log

Trois demarrages du MEME disque A/B, chacun coupe brutalement (SIGKILL) apres
son compteur : 1 et 2 sur le slot A, 3 sur le slot B (META reecrit entre 2
et 3, comme apres une mise a jour). Fail closed, exige pour le demarrage k :
  - la partition STATE montee (BOUCHAUD_STAGE2_PERSIST_NVME) ;
  - exactement une ligne `BOUCHAUD_PERSIST_DEMARRAGES n=k slot=S sync>=0` ;
  - le slot reellement demarre (BOUCHAUD_AB_BOOT slot=S) egal a celui du compteur ;
  - aucune panique.
Un compteur qui repart a 1 apres le changement de slot = donnees perdues.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
ATTENDU = [(1, "A"), (2, "A"), (3, "B")]
COMPTEUR = re.compile(r"BOUCHAUD_PERSIST_DEMARRAGES n=(\d+) slot=([AB-]) sync=(-?\d+)")
INTERDIT = ("*** KERNEL PANIC ***", "DOUBLE FAULT", "TRIPLE FAULT", "BOUCHAUD_AB_RECOVERY_REQUIRED")


def verifie(journaux: list[str]) -> list[str]:
    if len(journaux) != len(ATTENDU):
        return [f"{len(ATTENDU)} journaux attendus, {len(journaux)} recus"]
    erreurs = []
    for k, (texte, (n, slot)) in enumerate(zip(journaux, ATTENDU), 1):
        t = ANSI.sub("", texte).replace("\r", "")
        if "BOUCHAUD_STAGE2_PERSIST_NVME" not in t:
            erreurs.append(f"demarrage {k} : STATE non montee")
        lignes = COMPTEUR.findall(t)
        if len(lignes) != 1:
            erreurs.append(f"demarrage {k} : {len(lignes)} compteur(s) au lieu d'un")
            continue
        vu_n, vu_slot, sync = int(lignes[0][0]), lignes[0][1], int(lignes[0][2])
        if vu_n != n:
            erreurs.append(f"demarrage {k} : compteur {vu_n}, attendu {n}"
                           + (" (donnees STATE perdues)" if vu_n < n else ""))
        if vu_slot != slot:
            erreurs.append(f"demarrage {k} : compteur ecrit depuis le slot {vu_slot}, attendu {slot}")
        if sync < 0:
            erreurs.append(f"demarrage {k} : synchronisation en echec ({sync})")
        if f"BOUCHAUD_AB_BOOT slot={slot}" not in t:
            erreurs.append(f"demarrage {k} : le chargeur n'a pas demarre le slot {slot}")
        for x in INTERDIT:
            if x in t:
                erreurs.append(f"demarrage {k} : {x}")
    return erreurs


def main(argv: list[str]) -> int:
    try:
        journaux = [Path(p).read_text(encoding="utf-8", errors="replace") for p in argv[1:]]
    except OSError as e:
        print(f"STATE_AB_FAIL {e}", file=sys.stderr)
        return 2
    erreurs = verifie(journaux)
    for e in erreurs:
        print(f"STATE_AB_FAIL {e}", file=sys.stderr)
    if erreurs:
        return 1
    print("STATE_AB_OK demarrages=3 slots=A,A,B compteur=1,2,3 PHYSICAL=NOT_RUN")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
