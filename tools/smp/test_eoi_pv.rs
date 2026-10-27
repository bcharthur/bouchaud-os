//! EOI paravirtualise KVM : detection et consommation de la dispense
//! (P13_EOI_PARAVIRTUALISE_V1). Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/arch/x86_64/eoi_pv_drapeau.rs"]
mod drapeau;

use core::sync::atomic::{AtomicU32, Ordering};
use drapeau::{consomme_dispense, kvm_avec_pv_eoi, DISPENSE, FONCTION_PV_EOI};

fn signature(nom: &[u8; 12]) -> [u32; 3] {
    [0, 4, 8].map(|i| u32::from_le_bytes([nom[i], nom[i + 1], nom[i + 2], nom[i + 3]]))
}

#[test]
fn seul_kvm_qui_annonce_la_fonction_est_retenu() {
    let kvm = signature(b"KVMKVMKVM\0\0\0");
    assert!(kvm_avec_pv_eoi(kvm, 0x4000_0001, FONCTION_PV_EOI));
    assert!(kvm_avec_pv_eoi(kvm, 0x4000_0010, FONCTION_PV_EOI | 1 << 5));
    assert!(!kvm_avec_pv_eoi(kvm, 0x4000_0001, 1 << 5), "fonction non annoncee");
    assert!(!kvm_avec_pv_eoi(kvm, 0x4000_0000, FONCTION_PV_EOI), "feuille des fonctions absente");
    assert!(!kvm_avec_pv_eoi(signature(b"Microsoft Hv"), 0x4000_0006, FONCTION_PV_EOI));
    assert!(!kvm_avec_pv_eoi(signature(b"TCGTCGTCGTCG"), 0x4000_0001, FONCTION_PV_EOI));
}

#[test]
fn la_dispense_est_consommee_une_seule_fois() {
    let mot = AtomicU32::new(DISPENSE);
    assert!(consomme_dispense(&mot), "bit pose par l'hote : pas d'ecriture APIC");
    assert_eq!(mot.load(Ordering::Relaxed), 0);
    assert!(!consomme_dispense(&mot), "deja consommee : l'ecriture APIC est due");
}

#[test]
fn sans_dispense_l_eoi_apic_reste_du_et_rien_n_est_modifie() {
    let mot = AtomicU32::new(0);
    assert!(!consomme_dispense(&mot));
    assert_eq!(mot.load(Ordering::Relaxed), 0);
    // Les autres bits n'appartiennent pas a l'invite : jamais modifies.
    let mot = AtomicU32::new(0xFFFF_FFFE);
    assert!(!consomme_dispense(&mot));
    assert_eq!(mot.load(Ordering::Relaxed), 0xFFFF_FFFE);
    let mot = AtomicU32::new(0xFFFF_FFFF);
    assert!(consomme_dispense(&mot));
    assert_eq!(mot.load(Ordering::Relaxed), 0xFFFF_FFFE);
}

#[test]
fn un_effacement_par_l_hote_entre_deux_lectures_rend_l_eoi_du() {
    // L'hote efface le bit (a une sortie de VM) avant l'operation de
    // l'invite : la consommation voit 0 et l'EOI APIC est ecrit.
    let mot = AtomicU32::new(DISPENSE);
    mot.store(0, Ordering::Relaxed);
    assert!(!consomme_dispense(&mot));
}
