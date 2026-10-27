//! La decision pure de l'EOI paravirtualise KVM (P13_EOI_PARAVIRTUALISE_V1),
//! testee sur l'hote (`tools/smp/test_eoi_pv.rs`).

use core::sync::atomic::{AtomicU32, Ordering};

/// CPUID 0x40000001 EAX : `KVM_FEATURE_PV_EOI`.
pub const FONCTION_PV_EOI: u32 = 1 << 6;
/// Bit 0 du mot partage : l'hote dispense de l'ecriture EOI a l'APIC.
pub const DISPENSE: u32 = 1;

/// L'hyperviseur est-il KVM et annonce-t-il l'EOI paravirtualise ?
/// `signature` : EBX, ECX, EDX de la feuille 0x40000000 ; `max_feuille` :
/// son EAX ; `fonctions` : EAX de la feuille 0x40000001.
pub fn kvm_avec_pv_eoi(signature: [u32; 3], max_feuille: u32, fonctions: u32) -> bool {
    let mut nom = [0u8; 12];
    nom[0..4].copy_from_slice(&signature[0].to_le_bytes());
    nom[4..8].copy_from_slice(&signature[1].to_le_bytes());
    nom[8..12].copy_from_slice(&signature[2].to_le_bytes());
    &nom == b"KVMKVMKVM\0\0\0" && max_feuille >= 0x4000_0001 && fonctions & FONCTION_PV_EOI != 0
}

/// Consomme la dispense, s'il y en a une : rend `true` si l'EOI est signale
/// par l'effacement du bit (aucune ecriture APIC), `false` si l'ecriture
/// APIC reste due.
///
/// L'ABI exige que la lecture du bit et son effacement soient UNE SEULE
/// instruction : l'hote peut le poser ou l'effacer a toute sortie de VM, donc
/// entre deux instructions de l'invite. `fetch_and` est une instruction
/// verrouillee (ou une boucle cmpxchg qui relit apres toute modification) :
/// un bit efface par l'hote entre-temps rend `false`, et l'EOI est ecrit.
/// Les autres bits ne sont jamais modifies.
pub fn consomme_dispense(mot: &AtomicU32) -> bool {
    mot.fetch_and(!DISPENSE, Ordering::AcqRel) & DISPENSE != 0
}
