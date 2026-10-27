#!/usr/bin/env python3
"""Aucune lecture de IA32_APIC_BASE sur le chemin IRQ/IPI apres la premiere
(P13_APIC_BASE_SANS_RDMSR_V1).

`local_apic()` relisait IA32_APIC_BASE par RDMSR a chaque `eoi_local()` et a
chaque `reschedule_cpu()` : une sortie de VM par IRQ et par IPI sous KVM, sur
le chemin echeance -> pret -> elu que mesure le P2.

Regles :

  1. `local_apic` ne lit le MSR que si la memo de CE coeur est absente ou
     sans bit d'activation, et memorise alors ce qu'il a lu (et active) ;
  2. la memo est indexee par coeur (`APIC_BASE_MEMO.get(cpu_index())`) : le
     mode xAPIC/x2APIC est par coeur ;
  3. la premisse : le noyau n'ecrit jamais IA32_APIC_BASE ailleurs que dans
     `local_apic` (activation), et ne bascule jamais le mode x2APIC ;
  4. `eoi_local` et `reschedule_cpu` passent par `local_apic`, sans RDMSR
     direct de IA32_APIC_BASE.

Fail-closed ; tests negatifs par mutation.
"""
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
SMP = RACINE / "src/arch/x86_64/smp.rs"
LECTURE = "usermode::read_msr(IA32_APIC_BASE)"
ECRITURE = "usermode::write_msr(IA32_APIC_BASE, apic_base);"


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


def verifie(smp: str, autres: str) -> list:
    fautes = []
    apic = corps(smp, "unsafe fn local_apic()")
    test = apic.find("if connue & APIC_ENABLE != 0 {")
    sinon = apic.find("} else {", test)
    lecture = apic.find(LECTURE)
    memo = apic.find("memo.store(apic_base, Ordering::Relaxed);")
    if not 0 <= test < sinon < lecture < memo:
        fautes.append("local_apic : IA32_APIC_BASE relu sans consulter la memo, ou jamais memorise")
    if "APIC_BASE_MEMO.get(cpu_index())" not in apic:
        fautes.append("local_apic : la memo n'est pas indexee par coeur")
    if smp.count(LECTURE) != 1 or smp.count("write_msr(IA32_APIC_BASE") != 1 or ECRITURE not in apic:
        fautes.append("smp.rs : IA32_APIC_BASE lu ou ecrit hors de local_apic")
    if re.search(r"write_msr\(\s*(IA32_APIC_BASE|0x1[bB])\b", autres) or "X2APIC_ENABLE" in autres:
        fautes.append("le mode APIC est modifie hors de smp.rs : la memo serait perimee")
    usages = smp.replace("const X2APIC_ENABLE: u64 = 1 << 10;", "").replace("apic_base & X2APIC_ENABLE != 0", "")
    if "X2APIC_ENABLE" in usages:
        fautes.append("smp.rs : le noyau bascule le mode x2APIC : la memo serait perimee")
    for fonction in ("pub fn eoi_local()", "pub fn reschedule_cpu("):
        f = corps(smp, fonction)
        if "local_apic()" not in f or "IA32_APIC_BASE" in f:
            fautes.append(f"{fonction} : RDMSR direct de IA32_APIC_BASE sur le chemin IRQ/IPI")
    return fautes


def negatifs(smp: str, autres: str) -> list:
    mutations = [
        ("memo ignoree", "if connue & APIC_ENABLE != 0 {", "if false {"),
        ("memo jamais posee", "            memo.store(apic_base, Ordering::Relaxed);\n", ""),
        ("memo globale", "APIC_BASE_MEMO.get(cpu_index())", "APIC_BASE_MEMO.get(0)"),
        ("bascule x2APIC", "            apic_base |= APIC_ENABLE;", "            apic_base |= APIC_ENABLE | X2APIC_ENABLE;"),
        ("EOI relit le MSR", "pub fn eoi_local() {\n    unsafe {\n",
         "pub fn eoi_local() {\n    unsafe {\n        let _ = usermode::read_msr(IA32_APIC_BASE);\n"),
    ]
    echecs = []
    for nom, avant, apres in mutations:
        if avant not in smp:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        if not verifie(smp.replace(avant, apres, 1), autres):
            echecs.append(f"mutation non detectee : {nom}")
    if not verifie(smp, autres + "\nfn x() { unsafe { write_msr(IA32_APIC_BASE, 0) } }\n"):
        echecs.append("mutation non detectee : ecriture hors de smp.rs")
    return echecs


def main() -> int:
    smp = SMP.read_text(encoding="utf-8")
    autres = "\n".join(p.read_text(encoding="utf-8") for p in sorted((RACINE / "src").rglob("*.rs")) if p != SMP)
    fautes = verifie(smp, autres) + negatifs(smp, autres)
    for faute in fautes:
        print(f"ECHEC {faute}")
    if fautes:
        return 1
    print("APIC base memorisee par coeur : ok (6 mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
