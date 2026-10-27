#!/usr/bin/env python3
"""Verdict de la campagne de convergence Ladybird (BOUCHAUD_CONVERGENCE_VERDICT_V1).

Lance par le job `convergence` du workflow ladybird-native-browser, APRES les
jobs qui jouent Ladybird sur Bouchaud. Chacun de ces jobs construit le noyau
depuis le MEME checkout et verifie l'artefact Ladybird avant de le jouer
(tools/ci/verifie_artefact_ladybird.sh : liaison au HEAD, ou aux entrees de
build quand l'artefact est reutilise). Ce script ne rejoue rien : il rassemble
les resultats et refuse le faux vert.

    BOUCHAUD_TEST_HEAD=<sha>
    BOUCHAUD_LADYBIRD_CONVERGENCE_OK            qualification, tout est vert
    BOUCHAUD_LADYBIRD_INTEGRATION_OK            developpement, tout est vert
    BOUCHAUD_LADYBIRD_CONVERGENCE_FAIL causes=  sinon, avec la liste

BOUCHAUD_CI_PALIERS_V2 : les exigences de chaque palier sont dans
tools/ci/contrat_paliers.py, la source unique que la garde
tools/verifie-ci-paliers.py confronte au workflow. Un job requis en
`failure`, `cancelled`, `skipped` ou absent est bloquant ; un job hors-run doit
etre `skipped`. CONVERGENCE_OK n'est imprime qu'en qualification.
"""
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import contrat_paliers as contrat  # noqa: E402

juge = contrat.juge


def main() -> int:
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    print(f"BOUCHAUD_TEST_HEAD={head}")
    print("provenance : noyau construit dans chaque job depuis ce checkout ; artefact Ladybird du job build "
          "de ce run (liaison au HEAD) ou d'un run aux entrees identiques (liaison par empreinte), "
          "verifie par tools/ci/verifie_artefact_ladybird.sh dans chaque job")
    lignes, causes = juge(os.environ)
    for ligne in lignes:
        print(ligne)
    if causes:
        print(f"BOUCHAUD_LADYBIRD_CONVERGENCE_FAIL causes={','.join(causes)}")
        return 1
    if os.environ.get("BOUCHAUD_PALIER") == contrat.QUALIFICATION:
        print("BOUCHAUD_LADYBIRD_CONVERGENCE_OK")
    else:
        print(f"BOUCHAUD_LADYBIRD_INTEGRATION_OK palier=developpement artefact={os.environ.get('PROVENANCE_MODE')}")
        print("BOUCHAUD_LADYBIRD_CONVERGENCE_NON_QUALIFIANTE raison=palier developpement "
              "(primitives P13 hors run ; qualification = workflow_dispatch palier=qualification)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
