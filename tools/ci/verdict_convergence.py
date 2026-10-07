#!/usr/bin/env python3
"""Verdict de la campagne de convergence Ladybird (BOUCHAUD_CONVERGENCE_VERDICT_V1).

Lance par le job `convergence` du workflow ladybird-native-browser, APRES les
jobs qui jouent Ladybird sur Bouchaud. Chacun de ces jobs construit le noyau
depuis le MEME checkout et verifie que l'artefact Ladybird a ete produit par
le MEME HEAD (`artifact_manifest.py verify` : `artefact d'un autre HEAD`
sinon). Ce script ne rejoue rien : il rassemble les resultats et refuse le
faux vert.

    BOUCHAUD_TEST_HEAD=<sha>
    BOUCHAUD_LADYBIRD_CONVERGENCE_OK            si toutes les epreuves passent
    BOUCHAUD_LADYBIRD_CONVERGENCE_FAIL causes=  sinon, avec la liste
"""
import os
import subprocess
import sys

EPREUVES = [
    ("R_BUILD", "build Ladybird (chaine canonique, 17 preparateurs)"),
    ("R_SMOKE", "smoke BrowserHost (canvas, codecs, JS, workers x10, molette, audio, isolation, lien Compositor)"),
    ("R_CACHE", "cache HTTP et SQLite apres redemarrage du navigateur"),
    ("R_WPT", "WPT smoke (50 fichiers, compares a Linux)"),
    ("R_ENDURANCE", "endurance 10 min (cadres, workers, onglets cross-site) sans crash Compositor"),
    ("R_SITES", "sites reels HTTPS (Example exige, Wikipedia mesure), pixels du Compositor"),
    ("R_CRASH_RENDU", "crash d'un WebContent : l'autre site et le Compositor vivent, reprise par un nouveau WebContent"),
]


def main() -> int:
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    print(f"BOUCHAUD_TEST_HEAD={head}")
    print("provenance : noyau construit dans chaque job depuis ce checkout ; artefact Ladybird du job build "
          "de ce run, verifie par artifact_manifest.py (refus d'un autre HEAD)")
    causes = []
    for cle, nom in EPREUVES:
        resultat = os.environ.get(cle, "absent")
        print(f"  {'ok     ' if resultat == 'success' else 'ECHEC  '} {nom} : {resultat}")
        if resultat != "success":
            causes.append(f"{cle.lower()[2:]}={resultat}")
    if causes:
        print(f"BOUCHAUD_LADYBIRD_CONVERGENCE_FAIL causes={','.join(causes)}")
        return 1
    print("BOUCHAUD_LADYBIRD_CONVERGENCE_OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
