#!/usr/bin/env python3
"""Les releves globaux d'une sortie de fil sont espaces ; les faits propres non.

BOUCHAUD_RELEVES_GLOBAUX_ESPACES_V1

Chaque fin de fil emettait sept a dix lignes de compteurs CUMULES de toute la
machine (`scope=global`), dont une sous le verrou `lifecycle`, depuis une tache
deja zombie donc non preemptable. La veille d'attente vive l'a mesure :
scheduler-ng-banc SMP4, 59 attentes de 50 a 631 ms, toutes derriere une tache
dans `exit_group` qui ecrivait sur la liaison serie.

Le garde exige, dans `exit_current` :
  1. chaque releve `scope=global` (CPU_CUMUL, CACHE_BALAYAGE,
     CACHE_BALAYAGE_TEMOINS, FAULT_REPRISE, BACKING_DISK_GLOBAL,
     BACKING_MEMORY_GLOBAL, CLEAN_PAGE_CACHE_GLOBAL, et les publications
     d'attribution disque / controleur ATA) derriere `releve_global` ;
  2. `PROCESS_EXIT` et `FAULT_FILE_BREAKDOWN` (faits propres au processus)
     emis a chaque sortie ;
  3. une periode d'au moins une seconde, un seul gagnant par periode (CAS).

Fail-closed ; trois tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
VIE = "src/kernel/process/thread/lifecycle.rs"


def sans_commentaires(t: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in t.splitlines())


def corps(texte: str, nom: str) -> str:
    m = re.search(r"\bfn\s+" + re.escape(nom) + r"\s*\(", texte)
    if not m:
        return ""
    o = texte.find("{", m.end())
    p = 0
    for i in range(o, len(texte)):
        p += texte[i] == "{"
        p -= texte[i] == "}"
        if p == 0:
            return texte[o:i + 1]
    return ""


def bloc_garde(texte: str, garde: str, marque: str) -> bool:
    """`marque` se trouve dans le bloc ouvert par `garde` (accolades equilibrees)."""
    i = texte.find(garde)
    if i < 0:
        return False
    o = texte.find("{", i)
    p = 0
    for j in range(o, len(texte)):
        p += texte[j] == "{"
        p -= texte[j] == "}"
        if p == 0:
            return marque in texte[o:j]
    return False


def verifie(racine: Path) -> list[str]:
    try:
        src = sans_commentaires((racine / VIE).read_text(encoding="utf-8"))
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []
    ec = corps(src, "exit_current")
    if not bloc_garde(ec, "if releve_global { crate::kernel::dmesg::log_fmt", '"CPU_CUMUL scope=global'):
        fautes.append(f"{VIE} : CPU_CUMUL n'est plus derriere releve_global")
    for marque in ('"CACHE_BALAYAGE scope=global', '"CACHE_BALAYAGE_TEMOINS scope=global', '"FAULT_REPRISE scope=global'):
        if not bloc_garde(ec, "if dernier_thread && releve_global {", marque):
            fautes.append(f"{VIE} : {marque} n'est plus derriere releve_global")
    for marque in ('"BACKING_DISK_GLOBAL scope=global', '"BACKING_MEMORY_GLOBAL scope=global',
                   '"CLEAN_PAGE_CACHE_GLOBAL scope=global', "backing_attrib::publie(", "ata::publie_controleur("):
        if not bloc_garde(ec, "    if releve_global {\n", marque):
            fautes.append(f"{VIE} : {marque} n'est plus derriere releve_global")
    if ec.count("scope=global") != 7:
        fautes.append(f"{VIE} : {ec.count('scope=global')} releves scope=global dans exit_current (7 attendus)")
    if '"PROCESS_EXIT t={}' not in ec or "FAULT_FILE_BREAKDOWN" not in ec:
        fautes.append(f"{VIE} : un fait propre au processus a disparu de la sortie")
    if bloc_garde(ec, "if releve_global {", '"PROCESS_EXIT t={}'):
        fautes.append(f"{VIE} : PROCESS_EXIT est espace -- c'est un fait propre, il sort a chaque fois")
    r = corps(src, "releve_global_du")
    m = re.search(r"const RELEVE_GLOBAL_PERIODE_MS: u64 = ([0-9_]+);", src)
    if not m or int(m.group(1).replace("_", "")) < 1000 or "compare_exchange(" not in r:
        fautes.append(f"{VIE} : periode des releves globaux < 1 s, ou plusieurs gagnants")
    return fautes


def mutation(a: str, b: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        d = Path(tmp) / VIE
        d.parent.mkdir(parents=True, exist_ok=True)
        t = (RACINE / VIE).read_text(encoding="utf-8")
        if a not in t:
            return False
        d.write_text(t.replace(a, b, 1), encoding="utf-8")
        return bool(verifie(Path(tmp)))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("releves globaux : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        ("    if releve_global {\n        let (dr, db, dn, dw)", "    {\n        let (dr, db, dn, dw)"),
        ("const RELEVE_GLOBAL_PERIODE_MS: u64 = 1_000;", "const RELEVE_GLOBAL_PERIODE_MS: u64 = 10;"),
        ("    if dernier_thread && releve_global {", "    if dernier_thread {"),
    ]
    for n, (a, b) in enumerate(negatifs, 1):
        if not mutation(a, b):
            print(f"releves globaux : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"RELEVES_GLOBAUX_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
