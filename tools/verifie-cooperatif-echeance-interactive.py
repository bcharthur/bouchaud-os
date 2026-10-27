#!/usr/bin/env python3
"""Au point cooperatif d'un fil noyau, une echeance interactive echue leve le
plancher de segment (P13_COOPERATIF_ECHEANCE_INTERACTIVE_V1).

Regles :

  1. `point_cooperatif` decide par `reveil::interactive_attend(en_file,
     borne, maintenant)` ;
  2. la borne n'est recalculee (`borne_interactive_a_jour`, registre) que si
     la lecture atomique la dit echue ET que la cession est sure, et
     interruptions coupees ;
  3. la decision pure leve le plancher pour une interactive en file OU une
     borne echue, jamais pour une borne future (test hote
     `test_reveil_cible.rs`).

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "sommeil": RACINE / "src/kernel/process/thread/sommeil.rs",
    "pur": RACINE / "src/kernel/scheduler/reveil.rs",
    "test": RACINE / "tools/smp/test_reveil_cible.rs",
}


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
    point = corps(t["sommeil"], "pub fn point_cooperatif(")
    garde = point.find("prochaine_echeance_interactive_ns(cpu) <= maintenant")
    sure = point.find("crate::kernel::scheduler::preempt::cession_volontaire_sure()")
    recalcul = point.find("borne_interactive_a_jour(cpu, maintenant)")
    decision = point.find("crate::kernel::scheduler::reveil::interactive_attend(")
    plancher = point.find("if (segment < SEGMENT_MIN_NS && !interactive_attend)")
    if not 0 <= garde < sure < recalcul < decision < plancher:
        fautes.append("point_cooperatif : echeance interactive echue ignoree, ou recalcul hors garde")
    if "without_interrupts(|| {\n            borne_interactive_a_jour(cpu, maintenant)" not in point:
        fautes.append("point_cooperatif : recalcul de la borne interruptions ouvertes")
    if "en_file > 0 || borne_ns <= maintenant_ns" not in corps(t["pur"], "pub fn interactive_attend("):
        fautes.append("reveil.rs : la decision n'est plus celle testee")
    if "fn une_echeance_interactive_echue_leve_le_plancher_comme_une_interactive_en_file()" not in t["test"]:
        fautes.append("test hote absent")
    return fautes


def negatifs(t: dict) -> list:
    mutations = [
        ("borne ignoree", "pur", "en_file > 0 || borne_ns <= maintenant_ns", "en_file > 0"),
        ("borne future acceptee", "pur", "en_file > 0 || borne_ns <= maintenant_ns", "en_file > 0 || borne_ns != u64::MAX"),
        ("recalcul sans garde de cession", "sommeil",
         "        && crate::kernel::scheduler::preempt::cession_volontaire_sure()\n    {", "    {"),
        ("recalcul IF ouvert", "sommeil", "without_interrupts(|| {\n            borne_interactive_a_jour(cpu, maintenant)",
         "identity(|| {\n            borne_interactive_a_jour(cpu, maintenant)"),
        ("decision court-circuitee", "sommeil", "crate::kernel::scheduler::reveil::interactive_attend(\n        en_file, borne_interactive, maintenant)",
         "(en_file > 0)"),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if avant not in t[cle]:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = m[cle].replace(avant, apres, 1)
        if not verifie(m):
            echecs.append(f"mutation non detectee : {nom}")
    return echecs


def main() -> int:
    t = {cle: p.read_text(encoding="utf-8") for cle, p in F.items()}
    fautes = verifie(t) + negatifs(t)
    for faute in fautes:
        print(f"ECHEC {faute}")
    if fautes:
        return 1
    print("point cooperatif et echeance interactive echue : ok (5 mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
