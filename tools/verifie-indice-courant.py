#!/usr/bin/env python3
"""L'indice de la tache courante se lit sans migration possible
(BOUCHAUD_INDICE_COURANT_ATOMIQUE_V1, C14).

Regles :

  1. `current_index_raw` lit le coeur ET sa case interruptions masquees
     (`interrupts::without_interrupts`), comme `contexte_execution` ;
  2. la lecture en deux temps n'existe que dans `rearme_dechire_pour_banc`,
     appelee seulement sous `INJECTION_DECHIREE` (banc `compta-injection
     dechiree`, jamais pose au demarrage) ;
  3. le banc la rejoue en CI : `run_compta_injection.sh` lance le bras
     `dechiree`, et exige qu'il MONTRE le defaut (panique `compta:` ou
     chevauchement).

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "diag": RACINE / "src/kernel/process/thread/diagnostic_stall.rs",
    "compta": RACINE / "src/kernel/process/thread/comptabilite.rs",
    "banc": RACINE / "tools/ci/run_compta_injection.sh",
    "flux": RACINE / ".github/workflows/os-primitives.yml",
}
LECTURE = "interrupts::without_interrupts(|| CURRENT[local_cpu()].load(Ordering::Acquire))"
APPEL = "    if INJECTION_DECHIREE.load(Ordering::Relaxed) {\n        rearme_dechire_pour_banc();\n        return;\n    }"


def corps(source: str, signature: str) -> str:
    debut = source.find(signature)
    if debut < 0:
        return ""
    ouverture = source.find("{", debut)
    profondeur = 0
    for i in range(ouverture, len(source)):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouverture:i + 1]
    return ""


def verifie(t: dict) -> list:
    fautes = []
    if corps(t["diag"], "fn current_index_raw() -> usize").strip("{} \n") != LECTURE:
        fautes.append("current_index_raw : le coeur et sa case ne sont plus lus interruptions masquees")
    rearme = corps(t["compta"], "fn rearme_compta_apres_idle() {")
    if APPEL not in rearme:
        fautes.append("rearme_compta_apres_idle : la lecture dechiree n'est plus confinee au banc")
    if t["compta"].count("rearme_dechire_pour_banc();") != 1:
        fautes.append("rearme_dechire_pour_banc : appelee hors du banc")
    if "static INJECTION_DECHIREE: AtomicBool = AtomicBool::new(false);" not in t["compta"] \
            or t["compta"].count("INJECTION_DECHIREE.store(") != 1:
        fautes.append("INJECTION_DECHIREE : posee au demarrage")
    if "bras dechiree" not in t["banc"] or "compta-injection-dechiree.log" not in t["banc"] \
            or "COMPTA_INJECTION_ECHEC dechiree" not in t["banc"]:
        fautes.append("run_compta_injection.sh : le temoin de la lecture dechiree n'est plus joue ou plus juge")
    if "tools/ci/run_compta_injection.sh" not in t["flux"]:
        fautes.append("os-primitives.yml : le banc de comptabilite n'est plus joue")
    return fautes


def main() -> int:
    t = {cle: p.read_text(encoding="utf-8") for cle, p in F.items()}
    fautes = verifie(t)
    mutations = [
        ("lecture en deux temps", "diag", LECTURE, "CURRENT[local_cpu()].load(Ordering::Acquire)"),
        ("lecture dechiree hors banc", "compta", APPEL,
         "    rearme_dechire_pour_banc();\n    if INJECTION_DECHIREE.load(Ordering::Relaxed) {\n        return;\n    }"),
        ("injection posee au demarrage", "compta", "static INJECTION_DECHIREE: AtomicBool = AtomicBool::new(false);",
         "static INJECTION_DECHIREE: AtomicBool = AtomicBool::new(true);"),
        ("temoin non joue", "banc", "  bras dechiree\n", "  true\n"),
        ("temoin non juge", "banc", "COMPTA_INJECTION_ECHEC dechiree", "COMPTA_INJECTION_OK dechiree"),
    ]
    for nom, cle, avant, apres in mutations:
        if avant not in t[cle]:
            fautes.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = m[cle].replace(avant, apres, 1)
        if not verifie(m):
            fautes.append(f"mutation non detectee : {nom}")
    for faute in fautes:
        print(f"ECHEC {faute}")
    if fautes:
        return 1
    print(f"indice courant atomique : ok ({len(mutations)} mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
