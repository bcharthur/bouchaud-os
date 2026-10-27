#!/usr/bin/env python3
"""EOI paravirtualise KVM, borne a ce que l'ABI garantit
(P13_EOI_PARAVIRTUALISE_V1).

Regles :

  1. `eoi_local` ne saute l'ecriture APIC que si `eoi_dispense` a consomme
     la dispense de CE coeur ; sinon l'ecriture historique est faite ;
  2. la consommation lit ET efface le bit en une seule operation atomique
     (`fetch_and`) -- jamais une lecture puis une ecriture ;
  3. rien n'est arme sans la signature KVM ET la fonction annoncee (CPUID
     0x40000001 EAX bit 6), ni sans hyperviseur (CPUID.1:ECX[31]) ;
  4. le mot partage vient d'une frame mise a zero (`vmm::alloc_frame`) ;
  5. chaque coeur s'active a son demarrage (BSP dans main.rs, AP dans
     `bouchaud_ap_entry`), a cote du temps vole.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "smp": RACINE / "src/arch/x86_64/smp.rs",
    "pv": RACINE / "src/arch/x86_64/eoi_pv.rs",
    "pur": RACINE / "src/arch/x86_64/eoi_pv_drapeau.rs",
    "main": RACINE / "src/main.rs",
    "test": RACINE / "tools/smp/test_eoi_pv.rs",
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
    eoi = corps(t["smp"], "pub fn eoi_local()")
    dispense = eoi.find("if crate::arch::x86_64::eoi_pv::eoi_dispense(cpu_index()) {\n        return;\n    }")
    ecriture = eoi.find("lapic_write(lapic, LAPIC_EOI, 0);")
    if not 0 <= dispense < ecriture or "usermode::write_msr(X2APIC_EOI, 0);" not in eoi:
        fautes.append("eoi_local : l'EOI APIC n'est plus ecrit sans dispense consommee")
    if "mot.fetch_and(!DISPENSE, Ordering::AcqRel) & DISPENSE != 0" not in corps(t["pur"], "pub fn consomme_dispense("):
        fautes.append("la dispense n'est plus lue et effacee en une seule operation atomique")
    detection = corps(t["pur"], "pub fn kvm_avec_pv_eoi(")
    if '&nom == b"KVMKVMKVM\\0\\0\\0" && max_feuille >= 0x4000_0001 && fonctions & FONCTION_PV_EOI != 0' not in detection:
        fautes.append("la detection n'exige plus KVM et la fonction annoncee")
    if "pub const FONCTION_PV_EOI: u32 = 1 << 6;" not in t["pur"]:
        fautes.append("bit de fonction PV EOI incorrect")
    hv = corps(t["pv"], "fn hyperviseur_avec_pv_eoi()")
    if "__cpuid(1).ecx & (1 << 31) == 0" not in hv:
        fautes.append("PV EOI arme sans hyperviseur annonce")
    activation = corps(t["pv"], "pub fn active_ce_coeur(")
    if not 0 <= activation.find("!hyperviseur_avec_pv_eoi()") < activation.find("write_msr(MSR_KVM_PV_EOI_EN"):
        fautes.append("MSR PV EOI ecrit avant la detection")
    if "crate::kernel::vmm::alloc_frame()?" not in corps(t["pv"], "fn page()"):
        fautes.append("le mot partage ne vient plus d'une frame mise a zero")
    if "arch::x86_64::eoi_pv::active_ce_coeur(0);" not in t["main"] \
            or "crate::arch::x86_64::eoi_pv::active_ce_coeur(cpu);" not in t["smp"]:
        fautes.append("un coeur n'active pas l'EOI paravirtualise")
    for nom in ("la_dispense_est_consommee_une_seule_fois", "seul_kvm_qui_annonce_la_fonction_est_retenu",
                "sans_dispense_l_eoi_apic_reste_du_et_rien_n_est_modifie"):
        if f"fn {nom}()" not in t["test"]:
            fautes.append(f"test hote absent : {nom}")
    return fautes


def negatifs(t: dict) -> list:
    mutations = [
        ("eoi toujours saute", "smp", "if crate::arch::x86_64::eoi_pv::eoi_dispense(cpu_index()) {", "if true {"),
        ("lecture puis ecriture", "pur", "mot.fetch_and(!DISPENSE, Ordering::AcqRel) & DISPENSE != 0",
         "{ let v = mot.load(Ordering::Relaxed); mot.store(v & !DISPENSE, Ordering::Relaxed); v & DISPENSE != 0 }"),
        ("sans signature", "pur", 'b"KVMKVMKVM\\0\\0\\0"', 'b"KVMKVMKVM\\0\\0\\0" || true'),
        ("sans fonction", "pur", "fonctions & FONCTION_PV_EOI != 0", "true"),
        ("materiel nu", "pv", "__cpuid(1).ecx & (1 << 31) == 0", "false"),
        ("AP oublie", "smp", "    crate::arch::x86_64::eoi_pv::active_ce_coeur(cpu);\n", ""),
        ("frame non nulle", "pv", "crate::kernel::vmm::alloc_frame()?", "crate::kernel::vmm::alloc_frame_a_recouvrir()?"),
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
    print("EOI paravirtualise : ok (7 mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
