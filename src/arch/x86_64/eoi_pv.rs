//! EOI paravirtualise KVM (P13_EOI_PARAVIRTUALISE_V1).
//!
//! # Pourquoi
//!
//! Chaque entree d'un vecteur LAPIC se termine par un EOI. Sans
//! virtualisation materielle de l'APIC (APICv/AVIC -- absente en
//! virtualisation imbriquee : runners CI, WSL2), cette ecriture est une
//! SORTIE DE VM : MMIO en mode xAPIC (decodage et emulation de
//! l'instruction par l'hote), WRMSR en x2APIC. Sur le chemin d'un reveil
//! precis -- tir du minuteur local, balayage, election, commutation --
//! c'est une sortie de plus que le chemin du controle `off` n'a pas : celui
//! qui reveille sur un point sur d'appel systeme n'a aucune IRQ.
//!
//! # Le protocole (ABI KVM, `Documentation/virt/kvm/x86/msr.rst`)
//!
//! CPUID 0x40000001 EAX bit 6 annonce `MSR_KVM_PV_EOI_EN` (0x4b564d04).
//! Chaque coeur y ecrit l'adresse PHYSIQUE d'un mot de 4 octets aligne,
//! initialement nul, bit 0 a 1. L'hote pose le bit 0 de ce mot quand il
//! injecte une interruption qui peut se passer d'EOI APIC (front, aucune
//! autre en attente, pas d'IOAPIC) ; l'invite l'efface alors au lieu
//! d'ecrire l'EOI, et l'hote constate l'effacement a la sortie suivante.
//! Ignorer la dispense est TOUJOURS sur : l'ecriture APIC reste valide.
//!
//! Sans KVM, ou sans la fonction annoncee, rien n'est arme et `eoi_local`
//! ecrit l'EOI comme avant.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

#[path = "eoi_pv_drapeau.rs"]
mod drapeau;

const FEUILLE_SIGNATURE: u32 = 0x4000_0000;
const FEUILLE_FONCTIONS: u32 = 0x4000_0001;
const MSR_KVM_PV_EOI_EN: u32 = 0x4b56_4d04;
const KVM_MSR_ENABLED: u64 = 1;
const COEURS: usize = crate::arch::x86_64::smp::MAX_CPUS;
/// Un mot par coeur, espaces de 64 octets : pas de ligne de cache partagee
/// entre coeurs, et l'alignement de 4 octets exige par l'ABI.
const PAS: u64 = 64;

/// La frame physique (mise a zero) qui porte les mots de tous les coeurs.
static PAGE: AtomicU64 = AtomicU64::new(0);
/// Les coeurs pour lesquels l'hote a accepte l'adresse.
static ACTIFS: AtomicU64 = AtomicU64::new(0);

fn hyperviseur_avec_pv_eoi() -> bool {
    use core::arch::x86_64::__cpuid;
    // CPUID.1:ECX[31] : sans hyperviseur, les feuilles 0x4000_00xx ne
    // veulent rien dire (materiel nu).
    if __cpuid(1).ecx & (1 << 31) == 0 {
        return false;
    }
    let sig = __cpuid(FEUILLE_SIGNATURE);
    if sig.eax < FEUILLE_FONCTIONS {
        return false;
    }
    drapeau::kvm_avec_pv_eoi([sig.ebx, sig.ecx, sig.edx], sig.eax, __cpuid(FEUILLE_FONCTIONS).eax)
}

/// La frame commune, allouee une fois. Appele au demarrage de chaque coeur,
/// l'un apres l'autre (BSP, puis chaque AP dans son entree).
fn page() -> Option<u64> {
    let p = PAGE.load(Ordering::Acquire);
    if p != 0 {
        return Some(p);
    }
    let p = crate::kernel::vmm::alloc_frame()?;
    match PAGE.compare_exchange(0, p, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Some(p),
        // Une autre frame a gagne : la notre reste allouee, sans usage.
        Err(gagnante) => Some(gagnante),
    }
}

fn mot(cpu: usize) -> Option<&'static AtomicU32> {
    let base = PAGE.load(Ordering::Acquire);
    if cpu >= COEURS || base == 0 {
        return None;
    }
    let ptr = crate::kernel::memory::phys_to_virt(base + cpu as u64 * PAS) as *const AtomicU32;
    // SAFETY : frame du noyau, jamais rendue, alignee sur 64 octets ; seul
    // ce coeur et l'hote (pendant les sorties de ce coeur) y ecrivent.
    Some(unsafe { &*ptr })
}

/// Demande l'EOI paravirtualise pour CE coeur. Sans effet hors KVM ou sans
/// la fonction. Appele une fois par coeur, apres l'initialisation memoire.
pub fn active_ce_coeur(cpu: usize) {
    if cpu >= COEURS || !hyperviseur_avec_pv_eoi() {
        return;
    }
    let Some(base) = page() else { return };
    let phys = base + cpu as u64 * PAS;
    if let Some(m) = mot(cpu) {
        m.store(0, Ordering::Release);
    }
    unsafe {
        crate::arch::x86_64::usermode::write_msr(MSR_KVM_PV_EOI_EN, phys | KVM_MSR_ENABLED);
    }
    ACTIFS.fetch_or(1 << cpu, Ordering::AcqRel);
}

/// Dans un gestionnaire, interruptions masquees : l'hote a-t-il dispense ce
/// coeur de l'ecriture EOI ? Si oui, la dispense est consommee (l'EOI est
/// signale) et l'appelant n'ecrit rien a l'APIC.
#[inline]
pub fn eoi_dispense(cpu: usize) -> bool {
    if cpu >= COEURS || ACTIFS.load(Ordering::Relaxed) & (1 << cpu) == 0 {
        return false;
    }
    mot(cpu).map_or(false, drapeau::consomme_dispense)
}

/// `(coeurs actifs)`, pour le diagnostic.
pub fn coeurs_actifs() -> u32 {
    ACTIFS.load(Ordering::Relaxed).count_ones()
}
