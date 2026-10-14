#!/usr/bin/env python3
"""L'endurance ne ment ni par un echo ni par un NON_QUALIFIANT.

Les tests de `tools/ci/reliability/` (dont `test_traces_endurance.py`, qui
exerce ces regles sur le vrai `soak.py`) sont executes par
`tools/ci/run_host_tests.sh` (CI Fast, Reliability) ; ce garde-fou verifie
qu'ils le restent, et verrouille les regles dans le code.

Regles (fail-closed, negatifs par mutation) :

  1. BOUCHAUD_MARQUEUR_HORS_ECHO_V1 : `logscan.scan_text` cherche les
     marqueurs exiges dans les lignes EMISES (`lignes_emises`), jamais dans le
     texte brut, ou l'echo « + ... && echo X_OK » du shell les contient tous ;
     `run_mm_ng6.sh` fait de meme ;
  2. BOUCHAUD_ENDURANCE_NON_QUALIFIANT_V1 : `soak.py` n'arrete la campagne
     que sur un ECHEC, rend le code de `code_de_sortie(verdict)` (jamais 0
     pour un NON_QUALIFIANT), et l'endurance designe la sonde par
     `--marqueur-sonde ENDURANCE_ORDONNANCEUR_OK` ;
  3. `run_host_tests.sh` decouvre toujours la suite unittest de
     `tools/ci/reliability`.
"""
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
FIABILITE = RACINE / "tools/ci/reliability"
FICHIERS = {
    "logscan": FIABILITE / "logscan.py",
    "soak": FIABILITE / "soak.py",
    "etat": FIABILITE / "etat_cycle.py",
    "endurance": RACINE / ".github/workflows/endurance.yml",
    "mm": RACINE / "tools/ci/run_mm_ng6.sh",
    "hote": RACINE / "tools/ci/run_host_tests.sh",
}
DECOUVERTE = "python3 -m unittest discover -s tools/ci/reliability -p 'test_*.py'"


def verifie(t: dict) -> list:
    fautes = []
    if "missing = tuple(marker for marker in required_markers if marker not in emises)" not in t["logscan"] \
            or "emises = lignes_emises(lines)" not in t["logscan"]:
        fautes.append("logscan.py : les marqueurs exiges sont cherches dans le texte brut (echo des commandes compris).")
    if 'grep -aF "$marker" mm-ng6.emis.log' not in t["mm"]:
        fautes.append("run_mm_ng6.sh : les marqueurs sont cherches dans le journal brut (echo des commandes compris).")
    if not re.search(r"            if etat == ECHEC:\n                break\n", t["soak"]):
        fautes.append("soak.py : la campagne s'arrete sur autre chose qu'un ECHEC (ou ne s'arrete plus).")
    if 'return code_de_sortie(payload["verdict"])' not in t["soak"]:
        fautes.append("soak.py : le code de sortie ne suit plus le verdict (OK 0, NON_QUALIFIANT 2, ECHEC 1).")
    if "return {OK: 0, NON_QUALIFIANT: 2}.get(verdict, 1)" not in t["etat"]:
        fautes.append("etat_cycle.py : un NON_QUALIFIANT peut sortir en 0.")
    if "--marqueur-sonde ENDURANCE_ORDONNANCEUR_OK" not in t["endurance"]:
        fautes.append("endurance.yml : la sonde n'est plus designee (--marqueur-sonde).")
    if DECOUVERTE not in t["hote"]:
        fautes.append("run_host_tests.sh : les tests de tools/ci/reliability ne sont plus decouverts.")
    return fautes


def main() -> int:
    t = {}
    for nom, chemin in FICHIERS.items():
        if not chemin.is_file():
            print(f"tests de fiabilite : {chemin} absent")
            return 1
        t[nom] = chemin.read_text(encoding="utf-8")
    fautes = verifie(t)
    if fautes:
        print("tests de fiabilite : regle violee")
        for f in fautes:
            print("  " + f)
        return 1
    negatifs = [
        ("logscan", "if marker not in emises)", "if marker not in text)"),
        ("mm", 'grep -aF "$marker" mm-ng6.emis.log', 'grep -aF "$marker" mm-ng6.log'),
        ("soak", "            if etat == ECHEC:\n                break\n", "            break\n"),
        ("soak", 'return code_de_sortie(payload["verdict"])', 'return 0 if payload["verdict"] != "ECHEC" else 1'),
        ("etat", "return {OK: 0, NON_QUALIFIANT: 2}.get(verdict, 1)", "return {OK: 0, NON_QUALIFIANT: 0}.get(verdict, 1)"),
        ("endurance", "--marqueur-sonde ENDURANCE_ORDONNANCEUR_OK", "--marqueur-sonde AUTRE"),
        ("hote", DECOUVERTE, "python3 -m unittest discover -s tools/remote -p 'test_*.py'"),
    ]
    for n, (cle, a, b) in enumerate(negatifs, 1):
        if a not in t[cle]:
            print(f"tests de fiabilite : le test negatif {n} ne s'applique plus ({cle}) -- garde inoperant")
            return 1
        mute = dict(t)
        mute[cle] = t[cle].replace(a, b, 1)
        if not verifie(mute):
            print(f"tests de fiabilite : le test negatif {n} ne rougit pas ({cle}) -- garde inoperant")
            return 1
    print(f"[OK] tests de fiabilite : marqueurs emis seuls, NON_QUALIFIANT distinct et jamais vert, "
          f"suite unittest decouverte par run_host_tests.sh (negatifs={len(negatifs)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
