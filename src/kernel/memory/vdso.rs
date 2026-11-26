//! BOUCHAUD_VDSO_V1 -- l'heure lue en anneau 3, sans appel systeme.
//!
//! # Ce que cela remplace
//!
//! `AT_SYSINFO_EHDR` valait 0 : chaque `clock_gettime`, `gettimeofday`,
//! `time` et `sched_getcpu` de chaque programme entrait dans le noyau. Sonde
//! `appels-probe` (TCG, SMP2) : une transition nue coute ~5 us, une lecture
//! d'horloge ~10 us, et 100 % des lectures entraient dans le noyau. Une boucle
//! d'evenements en lit plusieurs par tour.
//!
//! # Ce que ce module installe
//!
//! * deux pages physiquement contigues, allouees une fois et jamais rendues :
//!   la page `vvar` (donnees, ecrites par le noyau seul) puis le vDSO
//!   (`src/arch/x86_64/vdso/vdso.so`, construit par `construit.sh`) ;
//! * leur projection dans chaque processus a l'exec, LECTURE SEULE (`vvar` :
//!   jamais executable ; vDSO : jamais inscriptible), par deux VMA
//!   `Backing::Vdso` -- partagees, empruntees, jamais possedees : ni le fork,
//!   ni munmap, ni la mort ne les rendent ;
//! * la publication par sequence (seqlock) des parametres de l'horloge.
//!
//! # Quand le chemin rapide est ouvert
//!
//! Seulement si le TSC est prouve synchrone entre les coeurs
//! (`smp::verifie_tsc_synchrone`) et sa frequence connue. Sinon `mode = 0` :
//! le vDSO fait l'appel systeme, comme avant. Le calcul est celui du noyau
//! (`timer::tsc_vers_ns`), au nanoseconde pres.
//!
//! # Ce qui protege les autres processus
//!
//! Les pages sont communes a TOUS les processus. Aucune ecriture ne doit les
//! atteindre : PTE sans `PTE_WRITE`, `mprotect(PROT_WRITE)` refuse sur elles,
//! et les ecritures du noyau pour le compte d'un processus passent par
//! `AddressSpace::write_utilisateur`, qui exige une PTE inscriptible.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::sync::SpinLock;
use crate::kernel::vmm::{self, PAGE_SIZE};

static IMAGE: &[u8] = include_bytes!("../../arch/x86_64/vdso/vdso.so");

/// Pages de l'image du vDSO.
const PAGES_IMAGE: u64 = (IMAGE.len() as u64 + PAGE_SIZE - 1) / PAGE_SIZE;

/// Decalage de la page vvar au-dessus du sommet de la pile principale. Dans
/// les 16 Gio de marge du creneau, hors de toute zone `mmap` ordinaire ; la
/// VMA l'y reserve de toute facon.
const DECALAGE_SUR_PILE: u64 = 4 * 1024 * 1024 * 1024;

/// La page de donnees, au format exact de `struct bo_vvar` (vdso.c).
#[repr(C)]
struct Vvar {
    seq: u32,
    mode: u32,
    boot_tsc: u64,
    hz: u64,
    rdtscp: u32,
    getcpu: u32,
    realtime_offset_ns: i64,
    realtime_ok: u32,
    aux_etiquette: u32,
}

const _: () = assert!(core::mem::size_of::<Vvar>() == 48);

/// Adresse physique de la page vvar (0 : vDSO absent).
static VVAR_PHYS: AtomicU64 = AtomicU64::new(0);
/// Ecrivains de la vvar : un a la fois (le seqlock ne protege que les lecteurs).
static PUBLICATION: SpinLock<()> = SpinLock::new(());
static PUBLICATIONS: AtomicU64 = AtomicU64::new(0);

/// Alloue et remplit les pages, puis publie. Une fois, apres le test de
/// synchronisation du TSC et la calibration.
pub fn init() {
    if VVAR_PHYS.load(Ordering::Acquire) != 0 {
        return;
    }
    let taille = ((1 + PAGES_IMAGE) * PAGE_SIZE) as usize;
    let Ok(layout) = core::alloc::Layout::from_size_align(taille, PAGE_SIZE as usize) else { return };
    let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
    if ptr.is_null() {
        crate::serial_println!("BOUCHAUD_VDSO absent raison=allocation");
        return;
    }
    // L'arene du tas est physiquement contigue (carte directe) : les pages se
    // projettent en anneau 3 par leur adresse physique, comme une region
    // partagee native. Hors de l'arene : on renonce (et on rend).
    let Some(phys) = crate::kernel::heap::phys_dans_arene(ptr, taille) else {
        unsafe { alloc::alloc::dealloc(ptr, layout) };
        crate::serial_println!("BOUCHAUD_VDSO absent raison=hors-arene");
        return;
    };
    unsafe {
        core::ptr::copy_nonoverlapping(IMAGE.as_ptr(), ptr.add(PAGE_SIZE as usize), IMAGE.len());
    }
    VVAR_PHYS.store(phys, Ordering::Release);
    publie();
}

fn vvar() -> Option<*mut Vvar> {
    let phys = VVAR_PHYS.load(Ordering::Acquire);
    (phys != 0).then(|| crate::kernel::memory::phys_to_virt(phys) as *mut Vvar)
}

/// (Re)publie les parametres de l'horloge : au demarrage, puis quand l'ancre
/// murale est posee. Ordre du seqlock : sequence impaire, champs, sequence
/// paire ; un lecteur qui voit une sequence impaire ou changee recommence.
pub fn publie() {
    let Some(v) = vvar() else { return };
    let _garde = PUBLICATION.lock();
    let (boot_tsc, hz, rdtscp, _) = crate::kernel::timer::parametres_tsc();
    let rapide = hz != 0 && crate::kernel::timer::tsc_synchrone();
    let mural = crate::kernel::abi::decalage_mural_ns();
    unsafe {
        let seq = core::ptr::addr_of_mut!((*v).seq);
        let s = core::ptr::read_volatile(seq);
        core::ptr::write_volatile(seq, s.wrapping_add(1) | 1);
        core::sync::atomic::fence(Ordering::SeqCst);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*v).boot_tsc), boot_tsc);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*v).hz), hz);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*v).rdtscp), rdtscp as u32);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*v).getcpu),
            crate::arch::x86_64::usermode::mode_tsc_aux() as u32);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*v).aux_etiquette),
            crate::arch::x86_64::usermode::TSC_AUX_ETIQUETTE);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*v).realtime_offset_ns), mural.unwrap_or(0));
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*v).realtime_ok), mural.is_some() as u32);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*v).mode), rapide as u32);
        core::sync::atomic::fence(Ordering::SeqCst);
        core::ptr::write_volatile(seq, (s | 1).wrapping_add(1));
    }
    let n = PUBLICATIONS.fetch_add(1, Ordering::Relaxed) + 1;
    crate::serial_println!(
        "BOUCHAUD_VDSO publication={} mode={} hz={} rdtscp={} getcpu={} mural={} image_octets={}",
        n,
        if rapide { "tsc" } else { "appel-systeme" },
        hz,
        rdtscp as u8,
        crate::arch::x86_64::usermode::mode_tsc_aux(),
        mural.is_some() as u8,
        IMAGE.len(),
    );
}

pub(crate) fn adresse_vvar() -> u64 {
    vmm::user_stack_top().saturating_add(DECALAGE_SUR_PILE)
}

/// Projette vvar + vDSO dans un espace neuf (exec) et y reserve leurs VMA.
/// Rend l'adresse de l'en-tete ELF du vDSO (`AT_SYSINFO_EHDR`), ou 0.
pub fn projette(space: &mut vmm::AddressSpace, promesses: &mut alloc::vec::Vec<crate::kernel::vma::Vma>) -> u64 {
    let phys = VVAR_PHYS.load(Ordering::Acquire);
    if phys == 0 {
        return 0;
    }
    let base = adresse_vvar();
    let lecture = vmm::PTE_PRESENT | vmm::PTE_USER | vmm::PTE_NO_EXEC;
    let execution = vmm::PTE_PRESENT | vmm::PTE_USER;
    let zones = [
        (base, PAGE_SIZE, lecture, phys),
        (base + PAGE_SIZE, PAGES_IMAGE * PAGE_SIZE, execution, phys + PAGE_SIZE),
    ];
    for &(debut, longueur, drapeaux, phys_base) in &zones {
        for i in 0..longueur / PAGE_SIZE {
            if !space.map_foreign_accounted(debut + i * PAGE_SIZE, phys_base + i * PAGE_SIZE,
                drapeaux, vmm::ResidentKind::Shared)
            {
                return 0;
            }
        }
        crate::kernel::vma::remplace(promesses, crate::kernel::vma::Vma {
            id: crate::kernel::vma::nouvelle_identite(),
            debut,
            fin: debut + longueur,
            drapeaux,
            backing: crate::kernel::vma::Backing::Vdso { phys_base, mapping_start: debut },
        });
    }
    base + PAGE_SIZE
}

/// Nombre de publications de la vvar (diagnostic).
pub fn publications() -> u64 {
    PUBLICATIONS.load(Ordering::Relaxed)
}
