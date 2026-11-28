#!/usr/bin/env python3
"""Témoin apparié du budget de latence au démarrage SMP1."""

import argparse
import re
from pathlib import Path

from budgets_endurance import CHECK_BUDGETS, evalue_budgets
from etat_cycle import OK, etat_du_cycle
from qemu_matrix import qemu_binary, run_one

BATTEMENT = re.compile(r"BOUCHAUD_SMP_BATTEMENT cpu=0 quantums=[1-9]\d* fenetre_ms=200 bat=1")
POMPE_INITIALE = re.compile(r"\[SMP-TASK\].*tid=100 pid=4 .*name=net-rx-poll")
PIRE_POMPE = re.compile(r"\[SCHED-NG-PIRE\] classe=normale .*tid=100 pid=4 cpu=0 .*coherent=1")
DEPASSEMENT = re.compile(r"^\s*ready_latency_max_ms \(.*\) : .* > .* \(budget\)", re.M)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("temoin", type=Path)
    parser.add_argument("corrige", type=Path)
    parser.add_argument("scenario", type=Path, help="dossier produit par fabrique-scenario-endurance.sh")
    parser.add_argument("sortie", type=Path, help="nouveau dossier des deux cycles")
    args = parser.parse_args()
    args.sortie.mkdir(parents=True, exist_ok=False)

    marqueurs = (args.scenario / "marqueurs-endurance.txt").read_text().splitlines()
    qemu = qemu_binary()
    for nom, image in (("temoin", args.temoin), ("corrige", args.corrige)):
        dossier = args.sortie / nom
        resultat = run_one(qemu, image, 1, 300, 4096, dossier, marqueurs, args.scenario / "endurance.img")
        trace = Path(resultat["log"])
        journal = trace.read_text(errors="replace")
        etat, raison = etat_du_cycle(resultat, journal, "ENDURANCE_ORDONNANCEUR_OK")
        budgets = evalue_budgets(trace, CHECK_BUDGETS)
        (dossier / "budgets.log").write_text(budgets["sortie"])
        print(f"{nom}: noyau={etat} budgets={budgets['rc']} {raison}", flush=True)
        if etat != OK or not BATTEMENT.search(journal):
            return 1
        if nom == "temoin":
            if (budgets["rc"] != 1 or budgets["requis_absents"]
                or not DEPASSEMENT.search(budgets["sortie"])
                or not POMPE_INITIALE.search(journal) or not PIRE_POMPE.search(journal)):
                print("Le témoin ne reproduit pas le dépassement de la pompe RX.", flush=True)
                return 1
        elif budgets["rc"] != 0 or budgets["requis_absents"]:
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
