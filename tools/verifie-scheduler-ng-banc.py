#!/usr/bin/env python3
"""Le banc Scheduler NG mesure ce qu'il dit, et ne se fige jamais.

BOUCHAUD_SCHEDULER_NG_BANC_V1

`tools/userland/scheduler-ng-banc.c` fournit la ligne de base de chaque phase
du chantier Scheduler NG (docs/SCHEDULER_NG_BASELINE.md). Un banc qui se
bloquerait sur le defaut qu'il cherche, ou qui compterait mal, rendrait
toute comparaison fausse. Ce garde exige :

  1. une recolte BORNEE (`recolte` + `WNOHANG` + echeance), jamais un
     `waitpid` bloquant ;
  2. le detecteur de resurrection : compteurs relus apres la recolte, ET la
     verification que la memoire partagee traverse `fork` (sinon le
     detecteur serait aveugle sans le dire) ;
  3. le compte des coeurs par /sys, et la publication de ce que rend
     `sysconf` (les deux divergent aujourd'hui : c'est une mesure) ;
  4. les quatre sections et la ligne de fin `sec=FIN` que l'agregateur
     exige ;
  5. le banc dans la chaine de construction musl ;
  6. l'agregateur passe son propre test (`--test`).

Fail-closed ; quatre tests negatifs.
"""
import re
import subprocess
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
BANC = "tools/userland/scheduler-ng-banc.c"
BUILD = "tools/userland/build.sh"
AGREGE = "tools/ci/scheduler_ng_baseline.py"
FICHIERS = (BANC, BUILD, AGREGE)


def verifie(racine: Path, avec_agregateur: bool = True) -> list[str]:
    try:
        src = {f: (racine / f).read_text(encoding="utf-8") for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []
    banc = src[BANC]
    if re.search(r"waitpid\([^,]+,[^,]+,\s*0\s*\)", banc):
        fautes.append(f"{BANC} : waitpid bloquant -- le banc peut se figer")
    if "WNOHANG" not in banc or "ECHEANCE_RECOLTE_US" not in banc:
        fautes.append(f"{BANC} : la recolte n'est plus bornee")
    if "ressuscites" not in banc or "= verifie_partage(p)" not in banc:
        fautes.append(f"{BANC} : le detecteur de resurrection a disparu ou est aveugle")
    if "/sys/devices/system/cpu/online" not in banc or "coeurs_sysconf" not in banc:
        fautes.append(f"{BANC} : le compte des coeurs n'est plus mesure")
    for sec in ("section_a", "section_b", "section_c", "section_d", "sec=FIN"):
        if sec not in banc:
            fautes.append(f"{BANC} : {sec} absent")
    if "scheduler-ng-banc.c" not in src[BUILD]:
        fautes.append(f"{BUILD} : le banc n'est plus construit")
    if avec_agregateur:
        r = subprocess.run([sys.executable, str(racine / AGREGE), "--test"],
                           capture_output=True, text=True)
        if r.returncode != 0 or "SCHEDULER_NG_BASELINE_TEST_OK" not in r.stdout:
            fautes.append(f"{AGREGE} --test : {r.stdout.strip()} {r.stderr.strip()}")
    return fautes


def mutation(fichier: str, avant: str, apres: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for f in FICHIERS:
            dest = copie / f
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text((RACINE / f).read_text(encoding="utf-8"), encoding="utf-8")
        cible = copie / fichier
        texte = cible.read_text(encoding="utf-8")
        if avant not in texte:
            return False
        cible.write_text(texte.replace(avant, apres, 1), encoding="utf-8")
        return bool(verifie(copie, avec_agregateur=False))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("scheduler-ng-banc : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (BANC, "        pid_t r = waitpid(pid, statut, WNOHANG);", "        pid_t r = waitpid(pid, statut, 0);"),
        (BANC, "    int partage = verifie_partage(p);", "    int partage = 1;"),
        (BANC, '    FILE *f = fopen("/sys/devices/system/cpu/online", "r");', "    FILE *f = NULL;"),
        (BUILD, "scheduler-ng-banc.c ", ""),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"scheduler-ng-banc : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"SCHEDULER_NG_BANC_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
