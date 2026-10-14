//! Le temps vole par l'hote, prouve par l'hyperviseur (BOUCHAUD_PREUVE_HOTE_V1).
//!
//! # Pourquoi
//!
//! L'attribution des reveils (BOUCHAUD_ATTRIBUTION_REVEIL_V2) classait
//! « externe » toute absence de battement que le noyau n'expliquait pas par
//! une section masquee. C'etait conclure de l'ABSENCE d'explication : un
//! tic perdu par le noyau lui-meme (vecteur mal arme, gestionnaire qui
//! ne rend pas la main, livraison retenue ailleurs) etait mis au compte de
//! QEMU, et le reveil qu'il retardait sortait de la comparaison.
//!
//! Sous KVM, l'hyperviseur publie pour chaque processeur virtuel le temps
//! pendant lequel il etait PRET mais que l'hote ne l'executait pas
//! (`MSR_KVM_STEAL_TIME`, la `run_delay` de son fil sur l'hote). C'est une
//! preuve positive : une absence n'est desormais « externe » que si ce
//! compteur a avance d'au moins la moitie de sa duree.
//!
//! # Ce que ce compteur ne voit pas (limites)
//!
//!   * le temps que le fil du processeur virtuel passe HORS de l'invite mais
//!     en marche : sortie vers QEMU (port serie, MMIO), interruption de
//!     l'hote. Il n'est pas vole au sens de l'hote ; une absence qu'il cause
//!     reste « inexpliquee » et compte contre le noyau ;
//!   * le delai d'injection d'un temporisateur dans un processeur virtuel
//!     arrete sur `hlt` (horloge de l'hote, reveil du fil) : idem ;
//!   * sous TCG, rien : aucune preuve n'existe, `disponible()` est faux.
//!
//! Ces trous font pencher du cote du noyau : une absence non prouvee n'est
//! jamais excusee.
//!
//! Protocole (ABI KVM, `Documentation/virt/kvm/x86/msr.rst`) : la feuille
//! CPUID 0x40000000 porte la signature « KVMKVMKVM », 0x40000001 EAX bit 5
//! annonce le vol ; chaque coeur ecrit dans le MSR 0x4b564d03 l'adresse
//! PHYSIQUE (alignee sur 64 octets) d'une structure de 64 octets, bit 0 a 1.
//! L'hote y ecrit `steal` (ns, cumule) encadre par une `version` impaire
//! pendant l'ecriture.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const FEUILLE_SIGNATURE: u32 = 0x4000_0000;
const FEUILLE_FONCTIONS: u32 = 0x4000_0001;
const FONCTION_VOL: u32 = 1 << 5;
const MSR_KVM_STEAL_TIME: u32 = 0x4b56_4d03;
const KVM_MSR_ENABLED: u64 = 1;
/// Taille d'une structure `kvm_steal_time`, et son alignement exige.
const TAILLE: u64 = 64;
const COEURS: usize = crate::arch::x86_64::smp::MAX_CPUS;

/// La page physique qui porte les structures de tous les coeurs (16 x 64
/// octets), allouee une fois.
static PAGE: AtomicU64 = AtomicU64::new(0);
static PRISE: AtomicBool = AtomicBool::new(false);
/// Les coeurs dont l'hote ecrit la structure.
static ACTIFS: AtomicU64 = AtomicU64::new(0);

// La lecture versionnee, pure : `tools/smp/test_vol_hote.rs`.
#[path = "vol_hote_lecture.rs"]
mod lecture;
pub use lecture::lecture_versionnee;

fn hyperviseur_kvm_avec_vol() -> bool {
    use core::arch::x86_64::__cpuid;
    // CPUID.1:ECX[31] : un hyperviseur est la. Sans lui, les feuilles
    // 0x4000_00xx ne veulent rien dire (materiel nu).
    let base = __cpuid(1);
    if base.ecx & (1 << 31) == 0 {
        return false;
    }
    let sig = __cpuid(FEUILLE_SIGNATURE);
    let mut nom = [0u8; 12];
    nom[0..4].copy_from_slice(&sig.ebx.to_le_bytes());
    nom[4..8].copy_from_slice(&sig.ecx.to_le_bytes());
    nom[8..12].copy_from_slice(&sig.edx.to_le_bytes());
    if &nom != b"KVMKVMKVM\0\0\0" || sig.eax < FEUILLE_FONCTIONS {
        return false;
    }
    __cpuid(FEUILLE_FONCTIONS).eax & FONCTION_VOL != 0
}

fn page() -> Option<u64> {
    let p = PAGE.load(Ordering::Acquire);
    if p != 0 {
        return Some(p);
    }
    if PRISE.swap(true, Ordering::AcqRel) {
        // Un autre coeur alloue : il publiera la page.
        for _ in 0..1_000_000 {
            let p = PAGE.load(Ordering::Acquire);
            if p != 0 {
                return Some(p);
            }
            core::hint::spin_loop();
        }
        return None;
    }
    let p = crate::kernel::vmm::alloc_frame()?;
    PAGE.store(p, Ordering::Release);
    Some(p)
}

/// Demande a l'hote de publier le vol de CE coeur. Appele une fois par
/// coeur, apres l'initialisation de la memoire. Sans effet hors KVM.
pub fn active_ce_coeur(cpu: usize) {
    if cpu >= COEURS || !hyperviseur_kvm_avec_vol() {
        return;
    }
    let Some(base) = page() else { return };
    let phys = base + cpu as u64 * TAILLE;
    unsafe {
        crate::arch::x86_64::usermode::write_msr(MSR_KVM_STEAL_TIME, phys | KVM_MSR_ENABLED);
    }
    ACTIFS.fetch_or(1 << cpu, Ordering::AcqRel);
}

/// L'hote publie-t-il le vol d'au moins un coeur ?
pub fn disponible() -> bool {
    ACTIFS.load(Ordering::Acquire) != 0
}

/// Le temps vole cumule de `cpu` (ns), `None` sans preuve pour ce coeur.
pub fn vole_ns(cpu: usize) -> Option<u64> {
    if cpu >= COEURS || ACTIFS.load(Ordering::Acquire) & (1 << cpu) == 0 {
        return None;
    }
    let base = PAGE.load(Ordering::Acquire);
    let ptr = crate::kernel::memory::phys_to_virt(base + cpu as u64 * TAILLE);
    // `steal` a l'offset 0 (u64), `version` a l'offset 8 (u32). Ecrits par
    // l'hote : lectures volatiles.
    let steal = ptr as *const u64;
    let version = unsafe { ptr.add(8) } as *const u32;
    lecture_versionnee(
        || unsafe {
            let v = core::ptr::read_volatile(version);
            core::sync::atomic::fence(Ordering::Acquire);
            let s = core::ptr::read_volatile(steal);
            core::sync::atomic::fence(Ordering::Acquire);
            (v, s)
        },
        || unsafe { core::ptr::read_volatile(version) },
        8,
    )
}

/// Le vol cumule de tous les coeurs publies (ns), et leur nombre.
pub fn vole_total_ns() -> (u64, u32) {
    let mut total = 0u64;
    let mut n = 0u32;
    for cpu in 0..COEURS {
        if let Some(v) = vole_ns(cpu) {
            total = total.saturating_add(v);
            n += 1;
        }
    }
    (total, n)
}
