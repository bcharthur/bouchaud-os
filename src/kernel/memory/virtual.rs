//! Memoire virtuelle : frames physiques + tables de pages par processus.
//!
//! Jusqu'ici le noyau se contentait de la cartographie faite par le bootloader
//! (`map_physical_memory`) : toute la RAM accessible via un offset fixe, un tas
//! unique, aucune notion d'espace d'adressage. C'est suffisant tant que tout le
//! code tourne en ring 0, mais pas pour executer un binaire ELF en ring 3 : il
//! faut des pages marquees `USER` que le noyau puisse creer, proteger et
//! detruire par processus.
//!
//! Ce module fournit donc :
//!   - un **allocateur de frames** (4 KiB) alimente par les regions de RAM que
//!     `kernel::memory` n'a donnees ni au tas ni au DMA ;
//!   - un **espace d'adressage** ([`AddressSpace`]) : une PML4 par processus,
//!     qui reprend les entrees noyau (le noyau reste mappe, sinon le premier
//!     `mov cr3` planterait sur l'instruction suivante) et possede en propre un
//!     creneau PML4 de 512 GiB reserve a l'utilisateur ;
//!   - les acces controles noyau -> memoire utilisateur ([`copy_from_user`],
//!     [`copy_to_user`]), qui traduisent l'adresse virtuelle via les tables du
//!     processus au lieu de dereferencer un pointeur venu du ring 3.
//!
//! ## Pourquoi un creneau PML4 dedie
//!
//! Copier les entrees noyau dans la PML4 du processus partage les tables de
//! niveau inferieur : ecrire une page utilisateur dans un creneau deja utilise
//! par le noyau modifierait les tables de *tous* les espaces d'adressage. On
//! reserve donc a l'utilisateur un creneau PML4 entierement libre (par defaut
//! celui de [`USER_SPACE_BASE`]) : ses PDPT/PD/PT appartiennent au processus,
//! sont detruites avec lui, et ne peuvent pas empieter sur le noyau.

use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use x86_64::registers::control::{Cr3, Cr3Flags};
use x86_64::structures::paging::PhysFrame;
use x86_64::PhysAddr;

use crate::arch::x86_64::smp;
use crate::kernel::memory;
use crate::kernel::sync::{SpinLockIrq, SpinLockIrqGuard};

/// Taille d'une page (et d'une frame) : 4 KiB.
pub const PAGE_SIZE: u64 = 4096;

/// Base du creneau PML4 reserve a l'espace utilisateur (creneau 128).
///
/// 0x4000_0000_0000 = 128 * 512 GiB : adresse canonique de la moitie basse,
/// tres loin de l'image noyau (chargee vers 0x20_0000) et de la cartographie
/// physique du bootloader (moitie haute). Les binaires PIE sont charges ici ;
/// un ELF `ET_EXEC` doit avoir ete lie dans ce creneau
/// (`-Wl,-Ttext-segment=0x400000000000`).
///
/// C'est la valeur *souhaitee* : si le bootloader occupait deja ce creneau,
/// [`init`] en choisit un autre et [`user_slot_base`] fait foi.
pub const USER_SPACE_BASE: u64 = 0x0000_4000_0000_0000;

/// Nombre d'entrees PML4 consecutives reservees a l'espace utilisateur.
///
/// Une seule entree — 512 GiB — a longtemps suffi. Ce n'est plus vrai depuis
/// que Bouchaud execute LibJS : `GC::PrimitiveStorage` **reserve** une « cage »
/// de **4 TiB** d'espace d'adressage des la construction de la `JS::VM`, pour
/// que tous ses pointeurs compresses partagent les memes bits de poids fort.
/// Cette reservation est en `PROT_NONE` et ne consomme aucune page physique,
/// mais elle exige que l'adressage la contienne — et 4 TiB, c'est huit fois
/// l'ancien creneau.
///
/// Ce n'est pas propre a Ladybird : la compression de pointeurs par cage est la
/// technique courante des moteurs JavaScript modernes. Un noyau 64 bits qui
/// borne son userland a 512 GiB rencontrera la meme limite a chaque fois.
///
/// Et ce n'est pas une reservation mais **deux** : `GC::BlockAllocator` reserve
/// a son tour une `HeapRegion` de 4 TiB, pour que tout bloc du tas gere se
/// reconnaisse a ses bits de poids fort. Un processus LibJS reserve donc 8 TiB
/// d'espace d'adressage avant d'avoir alloue le moindre octet utile.
///
/// Soixante-quatre entrees donnent 32 TiB : les deux reservations, la pile en
/// haut, et de la marge pour celles que LibWeb ajoutera. Le cout est nul tant
/// qu'une entree n'est pas utilisee — une entree PML4 absente ne coute pas de
/// table intermediaire, et une plage reservee n'en cree aucune.
pub const USER_SLOTS: usize = 64;

/// Taille du creneau utilisateur (64 entrees PML4 = 32 TiB).
pub const USER_SPACE_SIZE: u64 = USER_SLOTS as u64 * 512 * 1024 * 1024 * 1024;

/// Decalage, dans le creneau, de l'adresse de chargement d'un binaire PIE.
pub const LOAD_OFFSET: u64 = 0x40_0000;
/// Decalage de la base de l'editeur de liens dynamique (`PT_INTERP`).
pub const INTERP_OFFSET: u64 = 0x4000_0000;
/// Decalage de la base des allocations `mmap`.
pub const MMAP_OFFSET: u64 = 0x40_0000_0000;
/// Decalage du sommet de la pile utilisateur principale.
///
/// En haut du creneau, et non plus a 508 GiB : entre la base des `mmap`
/// (256 GiB) et la pile s'etend desormais la zone ou doit tenir la cage de
/// 4 TiB de LibJS. Laisser la pile au milieu la couperait en deux.
///
/// 16 GiB de marge sous le sommet, pour que la pile et ses gardes ne touchent
/// jamais la derniere page du creneau.
pub const STACK_TOP_OFFSET: u64 = USER_SPACE_SIZE - 16 * 1024 * 1024 * 1024;

/// Taille par defaut de la pile utilisateur principale (8 MiB, comme Linux).
pub const USER_STACK_SIZE: u64 = 8 * 1024 * 1024;

/// Adresse de chargement d'un binaire PIE (`ET_DYN`).
pub fn user_load_base() -> u64 {
    user_slot_base() + LOAD_OFFSET
}

/// Adresse de chargement de l'editeur de liens dynamique.
pub fn user_interp_base() -> u64 {
    user_slot_base() + INTERP_OFFSET
}

/// Base des allocations `mmap`.
pub fn user_mmap_base() -> u64 {
    user_slot_base() + MMAP_OFFSET
}

/// Sommet de la pile utilisateur principale.
pub fn user_stack_top() -> u64 {
    user_slot_base() + STACK_TOP_OFFSET
}

// --- Drapeaux d'entree de table de pages ------------------------------------

pub const PTE_PRESENT: u64 = 1 << 0;
pub const PTE_WRITE: u64 = 1 << 1;
pub const PTE_USER: u64 = 1 << 2;
pub const PTE_WRITE_THROUGH: u64 = 1 << 3;
pub const PTE_NO_CACHE: u64 = 1 << 4;
pub const PTE_ACCESSED: u64 = 1 << 5;
pub const PTE_DIRTY: u64 = 1 << 6;
pub const PTE_HUGE: u64 = 1 << 7;
pub const PTE_NO_EXEC: u64 = 1 << 63;
/// Bit `PAT` d'une entree de QUATRE KIBIOCTETS.
///
/// Il partage sa position avec `PTE_HUGE`, et ce n'est pas une collision : au
/// niveau feuille, la taille n'a plus a etre decrite, et le bit sept y designe
/// la table PAT. Les deux constantes existent separement parce que confondre
/// « grande page » et « type memoire » ne se remarquerait qu'a l'execution.
pub const PTE_PAT_PETITE_PAGE: u64 = 1 << 7;
/// Bit `PAT` d'une entree de DEUX MEBIOCTETS.
pub const PTE_PAT_GRANDE_PAGE: u64 = 1 << 12;

// Les bits 9..11 d'une PTE x86-64 sont reserves au logiciel. Bouchaud y
// conserve la classe RSS de chaque mapping. Ce petit tag rend le compte
// resident incremental : le moniteur n'a plus a reparcourir des centaines de
// milliers de PTE, sous le verrou Mm, a chaque releve du navigateur.
const PTE_RSS_KIND_SHIFT: u64 = 9;
const PTE_RSS_KIND_MASK: u64 = 0b111 << PTE_RSS_KIND_SHIFT;

/// Masque de l'adresse physique dans une entree (bits 12..51).
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

// BOUCHAUD_COW_V1 -- copie a l'ecriture.
//
// Bit 52, ignore par le MMU (bits 52..58 libres au logiciel tant que les cles
// de protection ne sont pas activees). Pose sur une PTE dont la frame est
// POSSEDEE et a ete partagee par `fork` : la PTE est alors en lecture seule,
// et toute ecriture -- du ring 3 par faute, du noyau par `write` ou par la
// verification de `fault_in_user_range` -- doit passer par
// `AddressSpace::prepare_casse_cow` : copie si un autre espace possede encore
// la frame, promotion sur place sinon. Le bit survit a `mprotect` : rendre une
// telle plage inscriptible n'autorise PAS l'ecriture directe.
pub const PTE_COW: u64 = 1 << 52;
/// Avec `PTE_COW` : la page est INSCRIPTIBLE pour son espace (elle l'etait au
/// `fork`, ou `mprotect` l'a rendue telle). Sans lui, une page COW est en
/// lecture seule pour de bon : une ecriture du ring 3 y reste une faute.
pub const PTE_COW_ECRITURE: u64 = 1 << 53;
const PTE_COW_BITS: u64 = PTE_COW | PTE_COW_ECRITURE;

/// La table des frames privees possedees par plusieurs espaces. Ordre des
/// verrous : `Mm` -> `COW` -> allocateur de frames. Jamais l'inverse.
static COW: crate::kernel::sync::SpinLock<crate::kernel::cow::TableCow> =
    crate::kernel::sync::SpinLock::new(crate::kernel::cow::TableCow::neuve());
static COW_COPIES: AtomicU64 = AtomicU64::new(0);
static COW_PROMOTIONS: AtomicU64 = AtomicU64::new(0);
static COW_ECHECS_ALLOCATION: AtomicU64 = AtomicU64::new(0);
static PAGES_ZERO_PROJETEES: AtomicU64 = AtomicU64::new(0);
static BARRIERES_CR3: AtomicU64 = AtomicU64::new(0);

/// Barrieres CR3 executees avant de rendre un espace (BOUCHAUD_BARRIERE_CR3_V1).
pub fn barrieres_cr3() -> u64 { BARRIERES_CR3.load(Ordering::Relaxed) }
static PAGES_ZERO_MATERIALISEES: AtomicU64 = AtomicU64::new(0);

/// (projections de la page zero, pages zero devenues privees par ecriture)
pub fn page_zero_stats() -> (u64, u64) {
    (PAGES_ZERO_PROJETEES.load(Ordering::Relaxed), PAGES_ZERO_MATERIALISEES.load(Ordering::Relaxed))
}

// BOUCHAUD_PAGE_ZERO_V1 -- UNE frame de zeros, partagee par toute la machine.
//
// Allouee a la premiere lecture d'une page anonyme, jamais rendue, jamais
// possedee par un espace (absente de tous les `pages`) : `munmap`, la mort et
// `fork` la traitent comme une frame empruntee. Toujours projetee en lecture
// seule avec `PTE_COW` : la premiere ecriture passe par `prepare_casse_cow`,
// qui lui substitue une frame privee neuve (et ne la « rend » jamais).
static PAGE_ZERO: AtomicU64 = AtomicU64::new(0);

/// La frame zero, allouee au premier besoin. `None` si l'allocation echoue.
pub fn page_zero() -> Option<u64> {
    let actuelle = PAGE_ZERO.load(Ordering::Acquire);
    if actuelle != 0 {
        return Some(actuelle);
    }
    let neuve = alloc_frame()?;
    match PAGE_ZERO.compare_exchange(0, neuve, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Some(neuve),
        Err(gagnante) => {
            free_frame(neuve);
            Some(gagnante)
        }
    }
}

/// `phys` (alignee ou non) est-elle dans la page zero ?
#[inline]
pub fn est_page_zero(phys: u64) -> bool {
    let zero = PAGE_ZERO.load(Ordering::Acquire);
    zero != 0 && phys & ADDR_MASK == zero
}

/// Un espace cesse de posseder ces frames : rend celles que plus personne ne
/// possede. Une seule prise de la table pour tout le lot.
fn frames_a_liberer(frames: impl IntoIterator<Item = u64>) -> Vec<u64> {
    let mut table = COW.lock();
    if table.frames_partagees() == 0 {
        drop(table);
        return frames.into_iter().collect();
    }
    frames
        .into_iter()
        .filter(|&f| table.rend(f) == crate::kernel::cow::Rendu::Liberer)
        .collect()
}

/// Un espace cesse de posseder `frame` (apres une copie a l'ecriture, APRES
/// le shootdown qui a retire l'ancienne traduction de tous ses coeurs).
pub fn rend_frame_possedee(frame: u64) {
    // La page zero n'est possedee par personne : la remplacer ne rend rien.
    if est_page_zero(frame) {
        return;
    }
    let liberer = COW.lock().rend(frame) == crate::kernel::cow::Rendu::Liberer;
    if liberer {
        free_frame(frame);
    }
}

/// (frames partagees maintenant, partages cumules, redevenues exclusives,
/// copies a l'ecriture, promotions sans copie, echecs d'allocation)
pub fn cow_stats() -> (usize, u64, u64, u64, u64, u64) {
    let table = COW.lock();
    let (partages, exclusives) = table.cumuls();
    (
        table.frames_partagees(),
        partages,
        exclusives,
        COW_COPIES.load(Ordering::Relaxed),
        COW_PROMOTIONS.load(Ordering::Relaxed),
        COW_ECHECS_ALLOCATION.load(Ordering::Relaxed),
    )
}

/// Issue de la casse d'une copie-a-l'ecriture sur une page.
#[must_use]
pub enum CasseCow {
    /// La PTE n'est pas COW et n'est pas inscriptible (absente, en lecture
    /// seule pour de bon) : l'ecriture reste une faute.
    PasCow,
    /// La PTE est deja inscriptible : un frere a casse la copie entre la faute
    /// et le verrou. Decide sous le MEME verrou que le reste -- relire
    /// l'inscriptibilite a part laissait un `fork` concurrent remettre la page
    /// en COW entre les deux lectures, et tuer un fil qui avait le droit
    /// d'ecrire (tempete-exec, mode fork : `COW_REFUS ... proprietaires=2`).
    DejaInscriptible,
    /// Seul proprietaire : la PTE est devenue inscriptible, sans copie.
    Promue,
    /// Copiee dans une frame neuve. L'appelant DOIT executer `invalidation`
    /// (hors du verrou `Mm` si possible) PUIS `rend_frame_possedee(ancienne)`
    /// -- dans cet ordre : tant qu'un coeur de cet espace peut encore lire
    /// l'ancienne frame, l'autre proprietaire ne doit pas pouvoir la
    /// promouvoir et y ecrire.
    Copiee { invalidation: TlbInvalidation, ancienne: u64 },
    /// Plus de frame : rien n'a change.
    SansMemoire,
}
const FREE_LIST_END: u64 = u64::MAX;

// --- Allocateur de frames physiques -----------------------------------------

struct FrameAllocator {
    /// Regions encore vierges : (prochaine frame libre, fin exclusive).
    regions: Vec<(u64, u64)>,
    /// Tete d'une liste intrusive de frames rendues. Le premier mot de chaque
    /// frame libre contient l'adresse de la suivante (`u64::MAX` la termine),
    /// ce qui interdit toute allocation de tas dans `free_frame`.
    freed_head: Option<u64>,
    total: u64,
    used: u64,
    allocations: u64,
    frees: u64,
    failures: u64,
    high_watermark: u64,
    // BOUCHAUD_P2_FREE_FRAME_O1_V1
    //
    // Quelles frames sont DANS la liste libre, un bit chacune. La liste
    // chainee reste la structure d'allocation ; ce bitmap n'est que le test
    // d'appartenance, qui etait auparavant un parcours integral. Voir
    // `kernel::frames_libres`.
    libres: crate::kernel::frames_libres::FramesLibres,
}

static FRAMES: SpinLockIrq<FrameAllocator> = SpinLockIrq::new(FrameAllocator {
    regions: Vec::new(),
    libres: crate::kernel::frames_libres::FramesLibres::neuf(),
    freed_head: None,
    total: 0,
    used: 0,
    allocations: 0,
    frees: 0,
    failures: 0,
    high_watermark: 0,
});
// Eventually-consistent mirrors for interrupt/panic-safe telemetry. Allocator
// ownership and detailed accounting remain protected exclusively by FRAMES.
static FRAME_TOTAL_RELAXED: AtomicU64 = AtomicU64::new(0);
static FRAME_USED_RELAXED: AtomicU64 = AtomicU64::new(0);

fn frames() -> SpinLockIrqGuard<'static, FrameAllocator> {
    FRAMES.lock()
}

/// Ajoute une region physique [start, end) a l'allocateur de frames.
///
/// Appele par `kernel::memory::init` avec ce qui reste apres le tas noyau et
/// l'arene DMA. Les bornes sont alignees sur la page vers l'interieur.
pub fn add_region(start: u64, end: u64) {
    let start = (start + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let end = end & !(PAGE_SIZE - 1);
    if end <= start {
        return;
    }
    let mut f = frames();
    f.total = f
        .total
        .checked_add((end - start) / PAGE_SIZE)
        .expect("vmm: total frame accounting overflow");
    f.regions.push((start, end));
    f.libres.couvre(start, end);
    FRAME_TOTAL_RELAXED.store(f.total, Ordering::Relaxed);
}

/// Alloue une frame physique de 4 KiB, mise a zero. `None` si plus de RAM.
/// Une frame NON MISE A ZERO, pour un appelant qui va la recouvrir en entier.
///
/// BOUCHAUD_C35_NE_PAS_ECRIRE_DEUX_FOIS
///
/// # Ce que cette fonction corrige
///
/// `alloc_frame` met la page a zero avant de la rendre. C'est la bonne regle :
/// une frame qui sort de l'allocateur porte encore ce qu'un autre processus y
/// avait ecrit, et la rendre telle quelle serait une fuite.
///
/// Mais `AddressSpace::duplicate` fait, pour chaque page du pere :
///
///     let frame = alloc_frame()?;            // 4 Kio de zeros
///     copy_nonoverlapping(..., PAGE_SIZE);   // 4 Kio recouverts
///
/// **Huit kibioctets ecrits pour en transmettre quatre.** La mise a zero n'est
/// pas seulement inutile ici : elle est integralement effacee a la ligne
/// suivante. Sur un `fork` de 256 Mio c'est un demi-gibioctet de memoire
/// ecrite pour rien.
///
/// # Le contrat, et pourquoi il n'est pas negociable
///
/// L'appelant DOIT ecrire les `PAGE_SIZE` octets avant que la page ne devienne
/// visible en anneau 3. Un appelant qui n'ecrirait qu'une partie exposerait le
/// reste -- c'est-a-dire de la memoire d'un autre processus -- et ce serait une
/// fuite silencieuse, du genre qu'aucun test d'integration ne remarque.
///
/// C'est pour cela qu'elle porte un nom different plutot qu'un parametre
/// booleen : un `alloc_frame(false)` se glisse dans un appel existant sans
/// qu'on y pense, un nom ne se choisit pas par accident.
///
/// `tools/ci/run_cout_fork.sh` verifie le contrat octet par octet, et le
/// verifie en le mettant en echec.
pub fn alloc_frame_a_recouvrir() -> Option<u64> {
    let mut f = frames();
    let candidate = if let Some(p) = f.freed_head {
        let next = unsafe { *(memory::phys_to_virt(p) as *const u64) };
        f.freed_head = if next == FREE_LIST_END { None } else { Some(next) };
        // La frame quitte la liste libre : le bit tombe AVANT qu'elle ne
        // soit rendue a un appelant, sinon un `free` immediat passerait
        // pour un double `free`.
        assert!(
            f.libres.marque_occupee(p),
            "vmm: frame {p:#x} en tete de liste libre mais absente du bitmap",
        );
        Some(p)
    } else {
        let mut found = None;
        for region in f.regions.iter_mut() {
            if region.0 + PAGE_SIZE <= region.1 {
                found = Some(region.0);
                region.0 += PAGE_SIZE;
                break;
            }
        }
        found
    };

    let phys = match candidate {
        Some(phys) => phys,
        None => {
            f.failures = f.failures.wrapping_add(1);
            return None;
        }
    };
    f.used = f
        .used
        .checked_add(1)
        .expect("vmm: used frame accounting overflow");
    f.allocations = f.allocations.wrapping_add(1);
    f.high_watermark = f.high_watermark.max(f.used);
    FRAME_USED_RELAXED.store(f.used, Ordering::Relaxed);
    Some(phys)
}

/// Une frame mise a zero. C'est l'allocation par defaut, et elle doit le rester.
///
/// La mise a zero est la SEULE difference avec `alloc_frame_a_recouvrir` :
/// les deux partagent le meme corps pour qu'aucune correction de l'allocateur
/// ne puisse n'en toucher qu'une.
pub fn alloc_frame() -> Option<u64> {
    let phys = alloc_frame_a_recouvrir()?;
    unsafe {
        core::ptr::write_bytes(memory::phys_to_virt(phys), 0, PAGE_SIZE as usize);
    }
    Some(phys)
}

/// BANC DE COURSE C5 (options `banc-c5-*`, jamais dans `default`).
#[cfg(any(feature = "banc-c5-ordre-historique", feature = "banc-c5-fenetre-corrigee"))]
fn attente_banc_c5() {
    let fin = crate::kernel::timer::monotonic_ns().saturating_add(2_000_000);
    while crate::kernel::timer::monotonic_ns() < fin {
        core::hint::spin_loop();
    }
}

/// Rend une frame a l'allocateur.
// BOUCHAUD_P2_FREE_FRAME_O1_V1
//
// LE DEFAUT QUE CETTE FONCTION AVAIT
// ----------------------------------
// Le double `free` etait detecte en parcourant TOUTE la liste libre. Cette
// liste est chainee dans les frames liberees elles-memes : chaque pas
// dereference une page physique froide. Le cout d'un `free` etait donc
// `O(frames libres)` defauts de cache, et la liste libre grandit avec l'age de
// la session.
//
// Liberer R frames coutait `O(R x L)`. Un `madvise(DONTNEED)` de 16 Mio
// (R = 4096) avec 100 000 frames libres, c'est 4 x 10^8 lectures memoire
// froides — des dizaines de secondes sous TCG, LE GROS VERROU TENU. C'est la
// tenue de 15 s attribuee a `madvise`, et l'explication de la degradation
// progressive : plus la session dure, plus la liste libre est longue.
//
// L'assertion n'est pas supprimee — elle est rendue exacte et constante. Un
// `free` d'une frame etrangere aux regions est desormais detecte lui aussi,
// ce que le parcours ne voyait pas.
pub fn free_frame(phys: u64) {
    let mut f = frames();
    let phys = phys & !(PAGE_SIZE - 1);
    assert!(
        f.libres.couverte(phys),
        "vmm: free d'une frame {phys:#x} hors des regions de l'allocateur",
    );
    assert!(f.used != 0, "vmm: frame accounting underflow for {phys:#x}");
    assert!(
        f.libres.marque_libre(phys),
        "vmm: double free frame {phys:#x}",
    );
    f.used -= 1;
    f.frees = f.frees.wrapping_add(1);
    unsafe {
        *(memory::phys_to_virt(phys) as *mut u64) = f.freed_head.unwrap_or(FREE_LIST_END);
    }
    f.freed_head = Some(phys);
    FRAME_USED_RELAXED.store(f.used, Ordering::Relaxed);
}

/// Rend un LOT de frames en prenant le verrou par tranches.
///
/// BOUCHAUD_C37_RENDRE_PAR_LOTS
///
/// # La mesure qui justifie cette fonction
///
///     PERF_EXECVE image=/sortie duree_us=99383 liberation_us=99024
///                 pages_rendues=65540
///
/// Quatre-vingt-dix-neuf virgule six pour cent d'un `execve` passent a rendre
/// les frames que le `fork` venait de copier -- environ une microseconde et
/// demie par frame. `free_frame` prend `frames()`, un verrou tournant qui
/// masque les interruptions, UNE FOIS PAR FRAME : soixante-cinq mille prises
/// et relachements pour un seul `execve`.
///
/// # Pourquoi des tranches, et pas une seule prise
///
/// Prendre le verrou une seule fois pour soixante-cinq mille frames le
/// tiendrait, interruptions masquees, pendant des dizaines de millisecondes.
/// C'est exactement le genre de tenue que les budgets d'execution de ce depot
/// refusent, et a juste titre : pendant ce temps aucun autre coeur ne peut
/// allouer et aucune interruption ne passe.
///
/// La tranche ramene le nombre de prises d'un facteur `TRANCHE` tout en
/// bornant la tenue. Ce n'est pas un compromis tiede : le cout d'une prise
/// est fixe et celui d'une tranche est lineaire, donc l'essentiel du gain est
/// acquis des les premieres dizaines.
/// Taille de tranche. 256 est CONSERVE -- la mesure ne justifie pas d'en changer.
///
/// BOUCHAUD_C45_LA_TENUE_SE_MESURE
///
/// 256 avait ete choisi en estimant la tenue a « environ cent microsecondes ».
/// Le balayage (`tools/ci/balaye_tranche_liberation.sh`, banc du cout du fork,
/// 65540 pages rendues) a mesure les cinq tailles :
///
///     taille  liberation_us  tranches  tenue_pire_us  attente_us  contentions
///         32          71069     16447            931       15526         2928
///         64          58238      8238            560       10578         1852
///        128          60527      4135           1888        5042         1272
///        256          59362      2080           2549        3193         1027
///        512          82493      1056           1553        2179          922
///
/// ## Ce que la mesure etablit
///
/// La tenue MOYENNE par tranche est exactement lineaire, comme attendu :
/// 17, 32, 63, 130 us pour 32/64/128/256. A 256, elle vaut 130 us -- proche
/// de l'estimation d'origine, qui etait donc raisonnable.
///
/// La contention decroit de facon monotone quand la tranche grandit :
/// 15,5 ms d'attente cumulee et 2928 contentions a 32, contre 3,2 ms et 1027
/// a 256. C'est l'effet recherche, et il est reel.
///
/// Les deux extremes degradent le debit : 32 par contention, 512 pour une
/// raison non elucidee -- probablement son tampon de pile de 4 Kio. Entre 64
/// et 256, le debit est indiscernable.
///
/// ## Ce que la mesure N'ETABLIT PAS
///
/// Sur un seul echantillon, 64 semblait gagner sur les deux axes (560 us de
/// tenue pire-cas contre 2549). Trois executions a 64 donnent :
///
///     tenue_pire_us   868   1896   2921
///
/// La distribution CHEVAUCHE celle de 256 et la depasse. Le pire-cas est
/// domine par le bruit d'emulation pendant la section critique, pas par la
/// taille de tranche. Le « gain » etait un artefact de n=1.
///
/// 256 est donc conserve : changer sur la foi d'un echantillon unique aurait
/// ete exactement l'erreur que ce balayage existe pour empecher.
const TRANCHE_LIBERATION: usize = 256;
const TRANCHE_LIBERATION_MAX: usize = 512;

static TRANCHE_EFFECTIVE: AtomicU64 = AtomicU64::new(TRANCHE_LIBERATION as u64);

/// Change la taille de tranche. Reservee au banc de mesure.
///
/// Bornee des deux cotes : zero bouclerait sans fin, au-dela du tampon
/// deborderait la pile. Rend la valeur reellement appliquee.
pub fn regle_tranche_liberation(taille: usize) -> usize {
    let taille = taille.clamp(1, TRANCHE_LIBERATION_MAX);
    TRANCHE_EFFECTIVE.store(taille as u64, Ordering::Relaxed);
    taille
}

pub fn tranche_liberation() -> usize {
    (TRANCHE_EFFECTIVE.load(Ordering::Relaxed) as usize).clamp(1, TRANCHE_LIBERATION_MAX)
}

// BOUCHAUD_C45_LA_TENUE_SE_MESURE
//
// Rendre les frames par lots divise le nombre de prises du verrou par la
// taille de tranche -- et multiplie d'autant la duree de CHAQUE prise. Le
// verrou masque les interruptions : une tranche trop grande echangerait un
// debit contre une latence, ce qui est precisement le troc qu'il ne faut pas
// faire a l'aveugle.
//
// Ces compteurs sont relaches et ne coutent rien sur le chemin : deux lectures
// d'horloge et quatre additions atomiques par TRANCHE, pas par frame.
static LOT_TRANCHES: AtomicU64 = AtomicU64::new(0);
static LOT_FRAMES: AtomicU64 = AtomicU64::new(0);
static LOT_TENUE_NS: AtomicU64 = AtomicU64::new(0);
static LOT_TENUE_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static LOT_ATTENTE_NS: AtomicU64 = AtomicU64::new(0);
static LOT_CONTENTIONS: AtomicU64 = AtomicU64::new(0);

/// (tranches, frames, tenue_totale_ns, tenue_pire_ns, attente_ns, contentions)
pub fn stats_liberation_lot() -> (u64, u64, u64, u64, u64, u64) {
    (
        LOT_TRANCHES.load(Ordering::Relaxed),
        LOT_FRAMES.load(Ordering::Relaxed),
        LOT_TENUE_NS.load(Ordering::Relaxed),
        LOT_TENUE_PIRE_NS.load(Ordering::Relaxed),
        LOT_ATTENTE_NS.load(Ordering::Relaxed),
        LOT_CONTENTIONS.load(Ordering::Relaxed),
    )
}

pub fn remet_stats_liberation_lot() {
    LOT_TRANCHES.store(0, Ordering::Relaxed);
    LOT_FRAMES.store(0, Ordering::Relaxed);
    LOT_TENUE_NS.store(0, Ordering::Relaxed);
    LOT_TENUE_PIRE_NS.store(0, Ordering::Relaxed);
    LOT_ATTENTE_NS.store(0, Ordering::Relaxed);
    LOT_CONTENTIONS.store(0, Ordering::Relaxed);
}

pub fn free_frames_lot(lot: impl IntoIterator<Item = u64>) {
    let taille = tranche_liberation();
    let mut tampon = [0u64; TRANCHE_LIBERATION_MAX];
    let mut rempli = 0usize;
    for phys in lot {
        tampon[rempli] = phys;
        rempli += 1;
        if rempli == taille {
            free_tranche(&tampon[..rempli]);
            rempli = 0;
        }
    }
    if rempli != 0 {
        free_tranche(&tampon[..rempli]);
    }
}

/// Le corps de `free_frame`, applique a une tranche sous UNE seule prise.
///
/// Les assertions sont mot pour mot celles de `free_frame` : rendre par lots
/// ne doit rien relacher de ce qui detecte un double `free` ou une frame
/// etrangere aux regions. Une version « rapide » qui les sauterait echangerait
/// une microseconde contre une corruption silencieuse.
fn free_tranche(tranche: &[u64]) {
    // L'ATTENTE est mesuree separement de la TENUE.
    //
    // Les confondre ferait passer la contention d'un voisin pour une section
    // critique trop longue -- et conduirait a reduire la tranche alors que le
    // probleme serait ailleurs.
    let avant_verrou = crate::kernel::timer::monotonic_ns();
    let mut f = frames();
    let apres_verrou = crate::kernel::timer::monotonic_ns();
    let attente = apres_verrou.saturating_sub(avant_verrou);
    for &brute in tranche {
        let phys = brute & !(PAGE_SIZE - 1);
        assert!(
            f.libres.couverte(phys),
            "vmm: free d'une frame {phys:#x} hors des regions de l'allocateur",
        );
        assert!(f.used != 0, "vmm: frame accounting underflow for {phys:#x}");
        assert!(
            f.libres.marque_libre(phys),
            "vmm: double free frame {phys:#x}",
        );
        f.used -= 1;
        f.frees = f.frees.wrapping_add(1);
        unsafe {
            *(memory::phys_to_virt(phys) as *mut u64) = f.freed_head.unwrap_or(FREE_LIST_END);
        }
        f.freed_head = Some(phys);
    }
    FRAME_USED_RELAXED.store(f.used, Ordering::Relaxed);
    drop(f);

    let tenue = crate::kernel::timer::monotonic_ns().saturating_sub(apres_verrou);
    LOT_TRANCHES.fetch_add(1, Ordering::Relaxed);
    LOT_FRAMES.fetch_add(tranche.len() as u64, Ordering::Relaxed);
    LOT_TENUE_NS.fetch_add(tenue, Ordering::Relaxed);
    LOT_ATTENTE_NS.fetch_add(attente, Ordering::Relaxed);
    // Une microseconde d'attente n'est pas du bruit d'horloge : c'est un autre
    // coeur qui tenait le verrou.
    if attente > 1_000 {
        LOT_CONTENTIONS.fetch_add(1, Ordering::Relaxed);
    }
    LOT_TENUE_PIRE_NS.fetch_max(tenue, Ordering::Relaxed);
}

/// Approximate frame usage for interrupt/panic-safe diagnostics.
///
/// This deliberately never touches `FRAMES`: the two atomics may describe
/// adjacent allocator mutations, which is sufficient for a visual percentage.
pub fn frame_stats_relaxed() -> (u64, u64) {
    (
        FRAME_USED_RELAXED.load(Ordering::Relaxed),
        FRAME_TOTAL_RELAXED.load(Ordering::Relaxed),
    )
}

/// (frames utilisees, frames libres, frames totales).
pub fn frame_stats() -> (u64, u64, u64) {
    let f = frames();
    (f.used, f.total.saturating_sub(f.used), f.total)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStatsDetailed {
    pub used: u64,
    pub free: u64,
    pub total: u64,
    pub allocations: u64,
    pub frees: u64,
    pub failures: u64,
    pub high_watermark: u64,
}

pub fn frame_stats_detailed() -> FrameStatsDetailed {
    let f = frames();
    FrameStatsDetailed {
        used: f.used,
        free: f.total.saturating_sub(f.used),
        total: f.total,
        allocations: f.allocations,
        frees: f.frees,
        failures: f.failures,
        high_watermark: f.high_watermark,
    }
}

// --- Acces aux tables de pages ----------------------------------------------

/// Tableau des 512 entrees d'une table, vu depuis la cartographie physique.
fn table_at(phys: u64) -> &'static mut [u64; 512] {
    unsafe { &mut *(memory::phys_to_virt(phys) as *mut [u64; 512]) }
}

/// Adresse physique de la PML4 active (CR3).
pub fn current_pml4() -> u64 {
    Cr3::read().0.start_address().as_u64()
}

static mut KERNEL_PML4: u64 = 0;
static mut NEXT_USER_SLOT: usize = 0;

/// Index PML4 (bits 39..47) d'une adresse virtuelle.
fn pml4_index(virt: u64) -> usize {
    ((virt >> 39) & 0x1FF) as usize
}

/// Initialise le sous-systeme : memorise la PML4 noyau, active NXE (bit
/// execute-disable) et choisit le creneau PML4 reserve a l'utilisateur.
pub fn init() {
    unsafe {
        KERNEL_PML4 = current_pml4();

        // EFER.NXE : sans ce bit, le bit 63 d'une entree est reserve et toute
        // page marquee non-executable provoque une faute de page.
        let mut efer = x86_64::registers::model_specific::Msr::new(0xC000_0080);
        let value = efer.read();
        if value & (1 << 11) == 0 {
            efer.write(value | (1 << 11));
        }

        let kernel = table_at(KERNEL_PML4);
        // Il faut desormais `USER_SLOTS` entrees **consecutives** et libres :
        // l'espace utilisateur est un intervalle continu, pas une collection de
        // creneaux. Une entree isolee ne suffirait plus (voir `USER_SLOTS`).
        let libres = |depart: usize| -> bool {
            depart + USER_SLOTS <= 256
                && (depart..depart + USER_SLOTS).all(|i| kernel[i] & PTE_PRESENT == 0)
        };
        let wanted = pml4_index(USER_SPACE_BASE);
        NEXT_USER_SLOT = if libres(wanted) {
            wanted
        } else {
            // Creneau occupe (cartographie inattendue) : on cherche la premiere
            // suite libre assez longue dans la moitie basse.
            let mut slot = 0;
            for i in 1..256 {
                if libres(i) {
                    slot = i;
                    break;
                }
            }
            slot
        };
    }

    let (_, _, total) = frame_stats();
    crate::kernel::dmesg::log_fmt(format_args!(
        "vmm: {} frames libres ({} MiB), creneau user PML4 #{}..{} ({} TiB)",
        total,
        total * PAGE_SIZE / (1024 * 1024),
        unsafe { NEXT_USER_SLOT },
        unsafe { NEXT_USER_SLOT } + USER_SLOTS - 1,
        USER_SPACE_SIZE / (1024 * 1024 * 1024 * 1024)
    ));
}

/// Creneau PML4 attribue a l'espace utilisateur.
pub fn user_slot() -> usize {
    unsafe { NEXT_USER_SLOT }
}

/// Base virtuelle effective du creneau utilisateur.
pub fn user_slot_base() -> u64 {
    (user_slot() as u64) << 39
}


/// Mappe une page physique basse a la meme adresse virtuelle dans la PML4
/// noyau. Sert uniquement au trampoline AP (0x8000/0x9000) pendant le passage
/// real mode -> long mode. Les entrees creees restent supervisor-only.
pub fn identity_map_kernel_page(virt_phys: u64) -> bool {
    let page = virt_phys & !(PAGE_SIZE - 1);
    let root = unsafe { KERNEL_PML4 };
    if root == 0 {
        return false;
    }

    let mut table = table_at(root);
    for level in (1..4).rev() {
        let shift = 12 + 9 * level;
        let index = ((page >> shift) & 0x1FF) as usize;
        let entry = table[index];
        if entry & PTE_PRESENT != 0 {
            if entry & PTE_HUGE != 0 {
                // 1 GiB (niveau 3) ou 2 MiB (niveau 2) deja identitaire : rien
                // a modifier. Une huge page non identitaire ne doit surtout pas
                // etre decoupee silencieusement pendant le boot.
                let span = 1u64 << shift;
                let phys_base = (entry & ADDR_MASK) & !(span - 1);
                return phys_base + (page & (span - 1)) == page;
            }
            table = table_at(entry & ADDR_MASK);
            continue;
        }

        let frame = match alloc_frame() {
            Some(frame) => frame,
            None => return false,
        };
        table[index] = frame | PTE_PRESENT | PTE_WRITE;
        table = table_at(frame);
    }

    let leaf = ((page >> 12) & 0x1FF) as usize;
    table[leaf] = page | PTE_PRESENT | PTE_WRITE;
    flush(page);
    true
}

// ---------------------------------------------------------------------------
// BOUCHAUD_MMIO_HORS_FENETRE_V1 -- les registres d'un peripherique au-dela de
// la fenetre physique du chargeur.
// ---------------------------------------------------------------------------
//
// Le chargeur ne mappe la fenetre physique que de 0 a la fin de sa carte
// memoire (au moins 4 Gio). Les pilotes lisaient pourtant leurs registres par
// `phys_to_virt(BAR)`, comme si toute adresse physique y etait.
//
// Sous TCG, OVMF borne l'espace physique a 40 bits et place sa fenetre PCI
// 64 bits sous 1 Tio, que sa carte memoire couvre : les BAR tombaient dans la
// fenetre par chance. Sous WHPX (Ryzen 7800X3D, 18/11), l'hote expose 46 bits
// et OVMF place la fenetre a 56 Tio : BAR0 du xHCI = 0x380000000000, lu a
// `phys_offset + BAR` = 0x3a8000000000 -- non mappe. Faute de page dans
// `xhci_probe::probe`, panique avant le bureau. Reproduit sous TCG par
// `-cpu max,phys-bits=46`. Une vraie machine qui place ses BAR au-dessus de
// 4 Gio (decodage au-dela de 4G, BAR redimensionnables) tombait de meme.
//
// `mappe_mmio` garde la convention de tous les pilotes -- registre virtuel =
// `phys_offset + physique` -- et cree les pages manquantes, non cachables.
// Elle REFUSE plutot que de risquer :
//   * une plage hors de la moitie basse canonique, ou dans le creneau
//     utilisateur ;
//   * une page deja mappee vers AUTRE CHOSE (collision avec une autre region) ;
//   * une entree PML4 neuve apres la creation du premier espace de processus :
//     ces espaces copient la PML4 noyau a leur creation et ne la verraient pas,
//     le noyau fauterait alors pendant un appel systeme.
// Un refus rend le peripherique indisponible ; il ne fait pas paniquer.

static ESPACES_CREES: AtomicBool = AtomicBool::new(false);
static MMIO_VERROU: crate::kernel::sync::SpinLock<()> = crate::kernel::sync::SpinLock::new(());
static MMIO_PAGES_CREEES: AtomicU64 = AtomicU64::new(0);

/// Fin (exclue) de la moitie basse canonique.
const MOITIE_BASSE_FIN: u64 = 0x0000_8000_0000_0000;

/// Rend `phys_offset + phys` utilisable pour `[phys, phys + octets)`.
///
/// Rend l'adresse virtuelle de `phys`, ou la raison du refus.
pub fn mappe_mmio(phys: u64, octets: u64) -> Result<u64, &'static str> {
    if octets == 0 {
        return Err("plage vide");
    }
    let fin = phys.checked_add(octets).ok_or("plage debordante")?;
    let root = unsafe { KERNEL_PML4 };
    if root == 0 {
        return Err("table noyau absente");
    }
    let offset = memory::phys_offset();
    let premiere = phys & !(PAGE_SIZE - 1);
    let derniere = fin.checked_add(PAGE_SIZE - 1).ok_or("plage debordante")? & !(PAGE_SIZE - 1);
    let v_debut = offset.checked_add(premiere).ok_or("hors espace virtuel")?;
    let v_fin = offset.checked_add(derniere).ok_or("hors espace virtuel")?;
    if v_fin > MOITIE_BASSE_FIN {
        return Err("hors moitie basse canonique");
    }
    let creneau = user_slot_base();
    let creneau_fin = creneau + ((USER_SLOTS as u64) << 39);
    if v_debut < creneau_fin && v_fin > creneau {
        return Err("collision avec le creneau utilisateur");
    }

    let _garde = MMIO_VERROU.lock();
    let mut page = premiere;
    while page < derniere {
        mappe_page_mmio(root, offset + page, page)?;
        page += PAGE_SIZE;
    }
    Ok(offset + phys)
}

/// Pages que `mappe_mmio` a du creer (zero : tout etait deja dans la fenetre).
pub fn mmio_pages_creees() -> u64 {
    MMIO_PAGES_CREEES.load(Ordering::Relaxed)
}

fn mappe_page_mmio(root: u64, virt: u64, phys: u64) -> Result<(), &'static str> {
    let mut table = table_at(root);
    for niveau in (1..4).rev() {
        let decalage = 12 + 9 * niveau;
        let index = ((virt >> decalage) & 0x1FF) as usize;
        let entree = table[index];
        if entree & PTE_PRESENT != 0 {
            if entree & PTE_HUGE != 0 {
                // Grande page deja posee : elle doit couvrir CETTE adresse
                // physique, sinon c'est une autre region.
                let portee = 1u64 << decalage;
                let base = (entree & ADDR_MASK) & !(portee - 1);
                return if base + (virt & (portee - 1)) == phys {
                    Ok(())
                } else {
                    Err("collision avec une grande page existante")
                };
            }
            table = table_at(entree & ADDR_MASK);
            continue;
        }
        if niveau == 3 && ESPACES_CREES.load(Ordering::Acquire) {
            return Err("entree PML4 neuve apres creation des espaces de processus");
        }
        let cadre = alloc_frame().ok_or("plus de cadre pour les tables")?;
        table[index] = cadre | PTE_PRESENT | PTE_WRITE;
        table = table_at(cadre);
    }
    let feuille = ((virt >> 12) & 0x1FF) as usize;
    let entree = table[feuille];
    if entree & PTE_PRESENT != 0 {
        return if entree & ADDR_MASK == phys { Ok(()) } else { Err("collision avec une page existante") };
    }
    // Non cachable (PCD|PWT : entree PAT 3, UC) et non executable : ce sont
    // des registres, l'ordre et l'effet de chaque acces sont le contrat.
    table[feuille] = phys | PTE_PRESENT | PTE_WRITE | PTE_NO_CACHE | PTE_WRITE_THROUGH | PTE_NO_EXEC;
    flush(virt);
    MMIO_PAGES_CREEES.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

/// Passe une plage PHYSIQUE en ecriture combinee, dans la fenetre physique.
///
/// # Pourquoi cette fonction existe
///
/// Le framebuffer est une fenetre PCIe : le micrologiciel la decrit comme non
/// cachable dans ses MTRR, et il a raison pour des registres. Un framebuffer
/// n'en est pas un -- l'ordre des pixels n'importe pas, et personne ne relit
/// un pixel -- mais rien ne le disait au processeur.
///
/// Le cout, mesure sur la machine de reference : 8,3 Mo en 75 ms, soit
/// 110 Mo/s, quand la memoire normale en fait dix a vingt mille. Chaque trame
/// plein ecran coutait soixante-quinze millisecondes.
///
/// # Ce qu'elle change, et ce qu'elle refuse de changer
///
/// Elle pose le bit `PAT` sur les entrees qui couvrent la plage, ce qui les
/// fait designer l'entree quatre de la table -- celle que
/// `arch::x86_64::pat` a reprogrammee en ecriture combinee. Le type effectif
/// devient alors ECRITURE COMBINEE (Intel Vol. 3, combinaison MTRR non
/// cachable + PAT ecriture combinee).
///
/// Elle REFUSE une page d'un gibioctet. Une telle page couvre bien plus que le
/// framebuffer : la passer en ecriture combinee emporterait avec elle des
/// registres de peripherique voisins, dont l'ordre des ecritures est le seul
/// contrat. Mieux vaut un ecran lent qu'un pilote qui ecrit dans le desordre.
///
/// Prend l'adresse VIRTUELLE de la plage, celle par laquelle le noyau y ecrit.
///
/// Rend le nombre de pages effectivement changees, ou une raison.
pub fn passe_en_ecriture_combinee(virt_base: u64, octets: u64) -> Result<usize, &'static str> {
    if octets == 0 {
        return Err("plage vide");
    }
    let root = unsafe { KERNEL_PML4 };
    if root == 0 {
        return Err("table noyau absente");
    }

    // L'ADRESSE VIRTUELLE, ET PAS LA PHYSIQUE.
    //
    // Le framebuffer du micrologiciel n'est PAS dans la fenetre physique. Sur
    // la machine de reference, le GOP est a 0x18000000000 quand la fenetre
    // physique commence a 0x28000000000 : le chargeur le mappe a part. Calculer
    // `offset + physique` aurait donc pointe ailleurs -- et change le type
    // memoire de pages qui n'avaient rien demande.
    let debut = virt_base & !(PAGE_SIZE - 1);
    let fin = virt_base.saturating_add(octets);
    let mut changees = 0usize;
    let mut page = debut;
    while page < fin {
        let virt = page;
        let mut table = table_at(root);
        let mut pas = PAGE_SIZE;
        let mut atteint = false;
        for niveau in (1..4).rev() {
            let decalage = 12 + 9 * niveau;
            let index = ((virt >> decalage) & 0x1FF) as usize;
            let entree = table[index];
            if entree & PTE_PRESENT == 0 {
                return Err("page absente de la table noyau");
            }
            if entree & PTE_HUGE != 0 {
                let portee = 1u64 << decalage;
                if portee > 2 * 1024 * 1024 {
                    // Un gibioctet : voir la note ci-dessus.
                    return Err("page trop large, refus de l'etendre a des registres voisins");
                }
                // Sur une grande page, le bit PAT est le douzieme, et non le
                // septieme : le septieme y designe deja la taille.
                table[index] = entree | PTE_PAT_GRANDE_PAGE;
                flush(virt);
                pas = portee;
                atteint = true;
                changees += 1;
                break;
            }
            table = table_at(entree & ADDR_MASK);
        }
        if !atteint {
            let feuille = ((virt >> 12) & 0x1FF) as usize;
            let entree = table[feuille];
            if entree & PTE_PRESENT == 0 {
                return Err("page absente de la table noyau");
            }
            table[feuille] = entree | PTE_PAT_PETITE_PAGE;
            flush(virt);
            changees += 1;
        }
        page = page.saturating_add(pas);
    }
    Ok(changees)
}

/// Une adresse est-elle dans le creneau utilisateur ?
pub fn is_user_addr(virt: u64) -> bool {
    let base = user_slot_base();
    virt >= base && virt < base + USER_SPACE_SIZE
}

// --- Espace d'adressage ------------------------------------------------------

/// Espace d'adressage d'un processus : une PML4 + les frames qu'elle possede.
pub struct AddressSpace {
    pml4: u64,
    // BOUCHAUD_SMP_NG2_ADDRESS_SPACE_ACTIVE_CPUS_V1
    /// CPU logiques dont le CR3 pointe actuellement sur cet espace.
    identity: Arc<AddressSpaceIdentity>,
    /// Frames de tables intermediaires, liberees avec l'espace.
    tables: Vec<u64>,
    // BOUCHAUD_P2_PAGES_POSSEDEES_SET_V1
    //
    // ENSEMBLE, ET NON LISTE.
    //
    // `pages` etait un `Vec<u64>` interroge par balayage lineaire :
    //
    //   * `prepare_unmap`  : `self.pages.contains(&phys)` PAR PAGE de la plage ;
    //   * `finish_unmap`   : `self.pages.iter().position(..)` PAR frame rendue ;
    //   * `owns_frame`     : `contains`, appele PAR PAGE par `clone_for_fork`.
    //
    // Soit `O(R x P)` ou R est la plage traitee et P le nombre de frames
    // residentes du processus. Pour un WebContent a 200 Mio resident
    // (P = 51 200) et un `madvise(DONTNEED)` de 16 Mio (R = 4096), c'est
    // 2 x 10^8 comparaisons pour `prepare_unmap` et autant pour `finish_unmap`
    // — le gros verrou tenu. `fork` payait le meme prix, en `O(P^2)`.
    //
    // Un ensemble ordonne rend les trois operations `O(log P)`. Il est aussi
    // plus juste : une frame ne peut appartenir qu'une fois a un espace, ce que
    // le `Vec` n'imposait pas — et `finish_unmap` n'en retirait alors qu'une
    // occurrence, laissant fuir les autres.
    /// Frames de donnees mappees pour l'utilisateur.
    pages: BTreeSet<u64>,
    /// Compteurs exacts mis a jour lors de la publication/retrait des PTE.
    /// Une lecture de RSS est ainsi O(1), quel que soit le nombre de pages.
    resident: ResidentStats,
    #[cfg(feature = "banc-gpu-venus")]
    baux_gpu: Vec<BailGpuVma>,
}

#[cfg(feature = "banc-gpu-venus")]
#[derive(Clone)]
struct BailGpuVma {
    id: u64,
    debut: u64,
    fin: u64,
    bail: Arc<crate::drivers::virtio_gpu::BailGem>,
}

/// Stable CR3 identity shared with `Mm`'s context-switch fast path.
pub(crate) struct AddressSpaceIdentity {
    pml4: u64,
    active_cpus: AtomicU64,
    retiring: AtomicBool,
    quiescence_seq: AtomicU64,
    quiescence_waiter: AtomicU64,
}

impl AddressSpaceIdentity {
    pub(crate) unsafe fn try_activate(&self) -> bool {
        if self.retiring.load(Ordering::Acquire) {
            return false;
        }
        // BOUCHAUD_CR3_MEME_ESPACE_V1 : ce coeur est deja dans CET espace, et
        // il n'en est jamais sorti (son bit est reste pose : chaque shootdown
        // de l'espace l'a vise). Recharger CR3 ne ferait que vider un TLB
        // juste -- sous TCG, tout le TLB logiciel de QEMU ; sans PCID, le TLB
        // materiel. Le bit n'est garde que par `switch_to`/la preemption
        // entre deux fils du MEME processus (`Mm::reste_charge_ici`).
        let cpu = smp::cpu_index();
        if cpu < 64
            && self.active_cpus.load(Ordering::Acquire) & (1u64 << cpu) != 0
            && Cr3::read().0.start_address().as_u64() == self.pml4
        {
            CR3_EVITES.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        CR3_ECRITS.fetch_add(1, Ordering::Relaxed);
        self.mark_active(cpu);
        if self.retiring.load(Ordering::Acquire) {
            self.mark_inactive(smp::cpu_index());
            return false;
        }
        let frame = PhysFrame::containing_address(PhysAddr::new(self.pml4));
        Cr3::write(frame, Cr3Flags::empty());
        true
    }

    /// Ce coeur execute-t-il CET espace, marque actif, sans retrait en cours ?
    pub(crate) fn charge_ici(&self, cpu: usize) -> bool {
        cpu < 64
            && !self.retiring.load(Ordering::Acquire)
            && self.active_cpus.load(Ordering::Acquire) & (1u64 << cpu) != 0
            && Cr3::read().0.start_address().as_u64() == self.pml4
    }

    pub(crate) fn mark_active(&self, cpu: usize) {
        if cpu < 64 {
            self.active_cpus.fetch_or(1u64 << cpu, Ordering::AcqRel);
        }
    }

    pub(crate) fn mark_inactive(&self, cpu: usize) {
        if cpu < 64 {
            self.active_cpus.fetch_and(!(1u64 << cpu), Ordering::AcqRel);
            if self.retiring.load(Ordering::Acquire) {
                self.quiescence_seq.fetch_add(1, Ordering::Release);
                let waiter = self.quiescence_waiter.load(Ordering::Acquire);
                if waiter < 64 && waiter as usize != cpu {
                    smp::reschedule_cpu(waiter as usize);
                }
            }
        }
    }

    pub(crate) fn begin_retire(&self) {
        self.retiring.store(true, Ordering::Release);
    }

    /// BOUCHAUD_BARRIERE_CR3_V1 -- a appeler APRES `begin_retire` et la
    /// quiescence des bits, AVANT de rendre la PML4 et ses tables.
    ///
    /// Un coeur efface son bit d'activite dans `switch_to` AVANT d'ecrire le
    /// CR3 suivant : pendant cette fenetre (IF=0), il execute encore sous
    /// l'ancienne PML4, et un defaut de TLB y parcourt les tables. Rendre la
    /// PML4 sur la seule foi des bits la laissait reutiliser et reecrire sous
    /// ses pieds -- faute noyau, double faute, triple faute (tempete-exec,
    /// mode complet, sous forte charge de l'hote).
    ///
    /// Un IPI vers TOUS les coeurs en ligne, et l'attente de leurs
    /// acquittements : un coeur dans la fenetre a IF=0 et n'acquitte qu'apres
    /// avoir ecrit son nouveau CR3 ; l'identite etant en retrait, aucun ne
    /// peut plus y entrer apres.
    pub(crate) fn barriere_cr3(&self) {
        smp::barriere_cr3(self.pml4);
        BARRIERES_CR3.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn active_cpus(&self) -> u64 {
        self.active_cpus.load(Ordering::Acquire)
    }

    /// Wait without burning a CPU while remote CR3 users leave. Interrupts are
    /// disabled across the final sequence/mask recheck and `sti; hlt`, closing
    /// the classic notification-before-HLT lost-wakeup window.
    pub(crate) fn wait_remote_quiescent(&self, self_cpu: usize) {
        self.quiescence_waiter.store(self_cpu as u64, Ordering::Release);
        let self_bit = 1u64 << self_cpu;
        while self.active_cpus() & !self_bit != 0 {
            let ticket = self.quiescence_seq.load(Ordering::Acquire);
            x86_64::instructions::interrupts::disable();
            if self.quiescence_seq.load(Ordering::Acquire) == ticket
                && self.active_cpus() & !self_bit != 0
            {
                x86_64::instructions::interrupts::enable_and_hlt();
            } else {
                x86_64::instructions::interrupts::enable();
            }
        }
        self.quiescence_waiter.store(u64::MAX, Ordering::Release);
    }
}

/// Invalidation distante preparee pendant une mutation de tables, puis executee
/// seulement apres avoir relache le BKL et tout emprunt de `Process`.
#[derive(Clone, Copy)]
pub struct TlbInvalidation {
    pml4: u64,
    active_cpus: u64,
    start: u64,
    len: u64,
}

impl TlbInvalidation {
    pub fn execute(self) {
        smp::shootdown_tlb(self.pml4, self.active_cpus, self.start, self.len);
    }
}

/// Frames detachees des PTE mais conservees jusqu'a la fin du shootdown.
pub struct UnmapRetirement {
    invalidation: TlbInvalidation,
    frames: Vec<u64>,
    #[cfg(feature = "banc-gpu-venus")]
    _baux_gpu: Vec<Arc<crate::drivers::virtio_gpu::BailGem>>,
}

impl UnmapRetirement {
    pub fn invalidation(&self) -> TlbInvalidation {
        self.invalidation
    }

    /// Combien de frames `finish_unmap` aura a rendre. Sert au marqueur de
    /// phase : une finition longue doit pouvoir dire COMBIEN de pages.
    pub fn frames_a_rendre(&self) -> usize {
        self.frames.len()
    }
}

/// Ce qu'une duplication d'espace d'adressage a REELLEMENT fait.
///
/// `fork` ne coute pas la meme chose selon la nature des pages du pere : une
/// page possedee est RECOPIEE (allocation de frame plus quatre kibioctets de
/// memcpy), une page empruntee est seulement re-projetee. Un chiffre unique
/// melangerait les deux et ne dirait pas lequel il faut attaquer.
#[derive(Clone, Copy, Default)]
pub struct DuplicationCompte {
    pub pages_vues: u64,
    pub pages_copiees: u64,
    pub pages_empruntees: u64,
    /// Frames possedees partagees en copie-a-l'ecriture (BOUCHAUD_COW_V1).
    pub pages_partagees: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ResidentStats {
    pub total_pages: u64,
    pub anonymous_pages: u64,
    pub file_private_pages: u64,
    pub shared_pages: u64,
    pub device_pages: u64,
    pub untracked_pages: u64,
}

/// Origine comptable d'une page residente.
///
/// La valeur voyage dans les bits logiciels de la PTE. Elle survit donc a
/// `mprotect`, a `fork` et n'exige aucune table auxiliaire par page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResidentKind {
    Untracked,
    Anonymous,
    FilePrivate,
    Shared,
    Device,
}

impl ResidentKind {
    #[inline]
    const fn bits(self) -> u64 {
        let code = match self {
            Self::Untracked => 0,
            Self::Anonymous => 1,
            Self::FilePrivate => 2,
            Self::Shared => 3,
            Self::Device => 4,
        };
        code << PTE_RSS_KIND_SHIFT
    }

    #[inline]
    const fn from_entry(entry: u64) -> Self {
        match (entry & PTE_RSS_KIND_MASK) >> PTE_RSS_KIND_SHIFT {
            1 => Self::Anonymous,
            2 => Self::FilePrivate,
            3 => Self::Shared,
            4 => Self::Device,
            _ => Self::Untracked,
        }
    }
}

impl ResidentStats {
    #[inline]
    fn mapped(&mut self, entry: u64) {
        self.total_pages = self.total_pages.checked_add(1)
            .expect("vmm: resident page counter overflow");
        let counter = match ResidentKind::from_entry(entry) {
            ResidentKind::Anonymous => &mut self.anonymous_pages,
            ResidentKind::FilePrivate => &mut self.file_private_pages,
            ResidentKind::Shared => &mut self.shared_pages,
            ResidentKind::Device => &mut self.device_pages,
            ResidentKind::Untracked => &mut self.untracked_pages,
        };
        *counter = counter.checked_add(1)
            .expect("vmm: resident class counter overflow");
    }

    #[inline]
    fn unmapped(&mut self, entry: u64) {
        self.total_pages = self.total_pages.checked_sub(1)
            .expect("vmm: resident page counter underflow");
        let counter = match ResidentKind::from_entry(entry) {
            ResidentKind::Anonymous => &mut self.anonymous_pages,
            ResidentKind::FilePrivate => &mut self.file_private_pages,
            ResidentKind::Shared => &mut self.shared_pages,
            ResidentKind::Device => &mut self.device_pages,
            ResidentKind::Untracked => &mut self.untracked_pages,
        };
        *counter = counter.checked_sub(1)
            .expect("vmm: resident class counter underflow");
    }
}

impl AddressSpace {
    /// Cree un espace d'adressage : entrees noyau reprises telles quelles,
    /// creneau utilisateur vierge et exclusif.
    pub fn new() -> Option<Self> {
        // BOUCHAUD_MMIO_HORS_FENETRE_V1 : passe ce point, une entree PML4
        // noyau creee plus tard ne serait plus vue par cet espace.
        ESPACES_CREES.store(true, Ordering::Release);
        let pml4 = alloc_frame()?;
        let dst = table_at(pml4);
        let src = table_at(unsafe { KERNEL_PML4 });
        // Le noyau doit rester mappe apres le `mov cr3` : on recopie toutes
        // ses entrees (image noyau, tas, cartographie physique).
        dst.copy_from_slice(src);
        // Le creneau utilisateur nous appartient : on repart de zero. Toutes
        // ses entrees, et pas seulement la premiere — l'espace utilisateur
        // s'etend sur `USER_SLOTS` entrees PML4 consecutives.
        for i in 0..USER_SLOTS {
            dst[user_slot() + i] = 0;
        }
        Some(AddressSpace {
            pml4,
            identity: Arc::new(AddressSpaceIdentity {
                pml4,
                active_cpus: AtomicU64::new(0),
                retiring: AtomicBool::new(false),
                quiescence_seq: AtomicU64::new(0),
                quiescence_waiter: AtomicU64::new(u64::MAX),
            }),
            tables: Vec::new(),
            pages: BTreeSet::new(),
            resident: ResidentStats::default(),
            #[cfg(feature = "banc-gpu-venus")]
            baux_gpu: Vec::new(),
        })
    }

    /// Adresse physique de la PML4 (valeur a charger dans CR3).
    pub fn pml4(&self) -> u64 {
        self.pml4
    }

    pub(crate) fn identity(&self) -> Arc<AddressSpaceIdentity> {
        Arc::clone(&self.identity)
    }

    #[inline]
    pub fn mark_active(&self, cpu: usize) {
        self.identity.mark_active(cpu);
    }

    #[inline]
    pub fn mark_inactive(&self, cpu: usize) {
        self.identity.mark_inactive(cpu);
    }

    pub fn active_cpus(&self) -> u64 {
        self.identity.active_cpus()
    }

    #[inline]
    fn invalidation(&self, start: u64, len: u64) -> TlbInvalidation {
        TlbInvalidation {
            pml4: self.pml4,
            active_cpus: self.active_cpus(),
            start,
            len,
        }
    }

    /// Active cet espace d'adressage sur le CPU courant.
    ///
    /// # Securite
    /// Le noyau doit rester mappe (garanti par `new`) et l'espace doit rester
    /// vivant tant qu'il est actif dans CR3.
    pub unsafe fn activate(&self) {
        // Publier l'activite AVANT CR3 ferme la race avec un unmap concurrent:
        // soit son snapshot nous cible, soit notre chargement CR3 est posterieur
        // a la mutation PTE et ne peut importer aucune traduction retiree.
        assert!(self.identity.try_activate(), "vmm: activation of retiring address space");
    }

    /// Descend (en creant si besoin) jusqu'a l'entree de table de pages qui
    /// decrit `virt`. Les tables intermediaires sont marquees permissives
    /// (`USER|WRITE`) : ce sont les entrees feuilles qui portent la protection
    /// reelle, comme sous Linux.
    fn entry_mut(&mut self, virt: u64, create: bool) -> Option<&'static mut u64> {
        let mut table = table_at(self.pml4);
        for level in (1..4).rev() {
            let index = ((virt >> (12 + 9 * level)) & 0x1FF) as usize;
            let entry = table[index];
            let next = if entry & PTE_PRESENT != 0 {
                entry & ADDR_MASK
            } else {
                if !create {
                    return None;
                }
                let frame = alloc_frame()?;
                self.tables.push(frame);
                table[index] = frame | PTE_PRESENT | PTE_WRITE | PTE_USER;
                frame
            };
            table = table_at(next);
        }
        let index = ((virt >> 12) & 0x1FF) as usize;
        Some(&mut table[index])
    }

    /// Mappe `virt` (aligne page) sur la frame physique `phys`.
    pub fn map(&mut self, virt: u64, phys: u64, flags: u64) -> bool {
        let inserted = match self.entry_mut(virt, true) {
            Some(entry) => {
                let old = *entry;
                let new = (phys & ADDR_MASK) | flags | PTE_PRESENT;
                // Un remplacement doit retirer l'ancienne PTE et attendre ses
                // ACK avant de publier la nouvelle. Refuser ici est plus sûr
                // qu'un remplacement silencieux sans invalidation en release.
                if old & PTE_PRESENT != 0 && old != new {
                    return false;
                }
                *entry = new;
                flush(virt);
                old & PTE_PRESENT == 0
            }
            None => return false,
        };
        if inserted {
            self.resident.mapped((phys & ADDR_MASK) | flags | PTE_PRESENT);
        }
        true
    }

    /// Mappe une page en portant sa classe RSS dans la PTE.
    pub fn map_accounted(
        &mut self,
        virt: u64,
        phys: u64,
        flags: u64,
        kind: ResidentKind,
    ) -> bool {
        self.map(virt, phys, (flags & !PTE_RSS_KIND_MASK) | kind.bits())
    }

    /// BANC DE COURSE C5 : elargit la fenetre de publication (2 ms).
    #[cfg(any(feature = "banc-c5-ordre-historique", feature = "banc-c5-fenetre-corrigee"))]
    pub fn attente_banc_c5_publique() {
        attente_banc_c5();
    }

    /// P13_PAGE_REMPLIE_AVANT_PUBLICATION_V1 : mappe sur `virt` (aligne page)
    /// une frame NEUVE dont le contenu est `contenu`, recopie AVANT que la PTE
    /// ne devienne presente.
    ///
    /// `map_alloc_accounted` puis `write` publiait une frame vierge, puis la
    /// remplissait. Le verrou Mm n'exclut que les autres fautes : un fil frere
    /// qui EXECUTE la page sur un autre coeur ne fait pas de faute -- la PTE est
    /// presente -- et lit des zeros (`00 00` = `add [rax], al`) ou un contenu a
    /// moitie copie (C5 : RIP au milieu d'une instruction, `hlt` de musl,
    /// `CR2=0`, deux freres tues ensemble). Ici, la copie precede la PTE ; la
    /// barriere interdit au compilateur de deplacer les ecritures du contenu
    /// apres celle de la PTE (x86 garde l'ordre des ecritures pour les autres
    /// coeurs et pour leurs parcours de table).
    pub fn map_new_frame_accounted(
        &mut self,
        virt: u64,
        contenu: &[u8],
        flags: u64,
        kind: ResidentKind,
    ) -> bool {
        if virt & (PAGE_SIZE - 1) != 0 || contenu.len() != PAGE_SIZE as usize {
            return false;
        }
        let frame = match alloc_frame_a_recouvrir() {
            Some(f) => f,
            None => return false,
        };
        unsafe {
            core::ptr::copy_nonoverlapping(
                contenu.as_ptr(),
                memory::phys_to_virt(frame),
                PAGE_SIZE as usize,
            );
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        #[cfg(feature = "banc-c5-fenetre-corrigee")]
        attente_banc_c5();
        self.pages.insert(frame);
        if self.map_accounted(virt, frame, flags, kind) {
            return true;
        }
        // Publication refusee : la frame n'a jamais ete visible, elle est
        // rendue tout de suite plutot qu'a la destruction de l'espace.
        if self.pages.remove(&frame) {
            free_frame(frame);
        }
        false
    }

    /// Alloue et mappe `len` octets a partir de `virt` (arrondi a la page).
    /// Les pages deja presentes sont conservees (mmap peut recouvrir).
    pub fn map_alloc(&mut self, virt: u64, len: u64, flags: u64) -> bool {
        self.map_alloc_accounted(virt, len, flags, ResidentKind::Untracked)
    }

    /// Alloue une plage dont chaque PTE porte la meme classe RSS.
    pub fn map_alloc_accounted(
        &mut self,
        virt: u64,
        len: u64,
        flags: u64,
        kind: ResidentKind,
    ) -> bool {
        let start = virt & !(PAGE_SIZE - 1);
        let end = (virt + len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let mut page = start;
        while page < end {
            if self.translate(page).is_none() {
                let frame = match alloc_frame() {
                    Some(f) => f,
                    None => return false,
                };
                self.pages.insert(frame);
                if !self.map_accounted(page, frame, flags, kind) {
                    return false;
                }
            }
            page += PAGE_SIZE;
        }
        true
    }

    /// Change les droits d'une plage deja mappee (mprotect).
    pub fn prepare_protect(&mut self, virt: u64, len: u64, flags: u64) -> Option<TlbInvalidation> {
        let start = virt & !(PAGE_SIZE - 1);
        let end = (virt + len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let mut page = start;
        let mut changed = false;
        while page < end {
            if let Some(entry) = self.entry_mut(page, false) {
                if *entry & PTE_PRESENT != 0 {
                    // Les droits changent, pas l'origine comptable de la page.
                    let masque = ADDR_MASK | PTE_RSS_KIND_MASK | PTE_COW;
                    #[cfg(feature = "banc-gpu-venus")]
                    let masque = masque | PTE_NO_CACHE | PTE_WRITE_THROUGH;
                    let mut new = (*entry & masque) | (flags & !PTE_COW_BITS) | PTE_PRESENT;
                    // BOUCHAUD_COW_V1 : une frame partagee reste en lecture
                    // seule ; l'ecriture que `mprotect` autorise est notee
                    // dans `PTE_COW_ECRITURE` et passera par la copie.
                    if new & PTE_COW != 0 && new & PTE_WRITE != 0 {
                        new = (new & !PTE_WRITE) | PTE_COW_ECRITURE;
                    }
                    if *entry != new {
                        *entry = new;
                        flush(page);
                        changed = true;
                    }
                }
            }
            page += PAGE_SIZE;
        }
        changed.then(|| self.invalidation(start, end.saturating_sub(start)))
    }

    #[cfg(feature = "banc-gpu-venus")]
    pub fn retient_bail_gpu(
        &mut self, id: u64, debut: u64, fin: u64, bail: Arc<crate::drivers::virtio_gpu::BailGem>,
    ) {
        self.baux_gpu.push(BailGpuVma { id, debut, fin, bail });
    }

    #[cfg(feature = "banc-gpu-venus")]
    pub fn retire_baux_gpu_sans_vma(&mut self, promesses: &[crate::kernel::vma::Vma]) {
        // UnmapRetirement conserve les baux intersectés, même pour un
        // retrait partiel, jusqu'au dernier ACK TLB de ce retrait.
        self.baux_gpu.retain(|bail| promesses.iter().any(|vma| {
            matches!(vma.backing, crate::kernel::vma::Backing::GemGpu { bail_id, .. } if bail_id == bail.id)
        }));
    }

    /// Supprime le mapping d'une plage (munmap). Les PTE sont d'abord
    /// retirees partout, puis le shootdown TLB est ACKe, et seulement ensuite
    /// les frames sont rendues. Cet ordre interdit tout use-after-free TLB.
    pub fn prepare_unmap(&mut self, virt: u64, len: u64) -> UnmapRetirement {
        let start = virt & !(PAGE_SIZE - 1);
        let end = (virt + len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let mut page = start;
        let mut retired: Vec<u64> = Vec::new();
        while page < end {
            let removed = if let Some(entry) = self.entry_mut(page, false) {
                let old = *entry;
                if old & PTE_PRESENT != 0 {
                    *entry = 0;
                    flush(page);
                    Some(old)
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(old) = removed {
                self.resident.unmapped(old);
                let phys = old & ADDR_MASK;
                if self.pages.contains(&phys) {
                    retired.push(phys);
                }
            }
            page += PAGE_SIZE;
        }

        UnmapRetirement {
            invalidation: self.invalidation(start, end.saturating_sub(start)),
            frames: retired,
            #[cfg(feature = "banc-gpu-venus")]
            _baux_gpu: self.baux_gpu.iter().filter(|bail| bail.debut < end && bail.fin > start)
                .map(|bail| Arc::clone(&bail.bail)).collect(),
        }
    }

    /// Rend les frames uniquement apres `UnmapRetirement::invalidation()`.
    pub fn finish_unmap(&mut self, retirement: UnmapRetirement) {
        // `remove` rend `false` si la frame a deja ete retiree : c'est le
        // cas quand la meme frame apparait deux fois dans `retirement`,
        // et c'est ce qui interdit le double `free`.
        let rendues: Vec<u64> = retirement
            .frames
            .into_iter()
            .filter(|phys| self.pages.remove(phys))
            .collect();
        // BOUCHAUD_COW_V1 : une frame encore possedee par un autre espace
        // (partagee par `fork`) n'est pas rendue a l'allocateur.
        for phys in frames_a_liberer(rendues) {
            free_frame(phys);
        }
    }

    /// BOUCHAUD_COW_V1 -- rend `page` inscriptible pour CET espace si elle
    /// est en copie-a-l'ecriture. Voir `CasseCow` pour ce que l'appelant doit
    /// faire de l'issue. Ne verifie PAS le droit d'ecrire (la VMA) : c'est a
    /// l'appelant de le faire.
    ///
    /// `pour_ecriture_utilisateur` : la casse est demandee pour une ecriture
    /// soumise aux droits (faute du ring 3, `user_write`) ; une page COW sans
    /// `PTE_COW_ECRITURE` rend alors `PasCow` -- l'ecriture reste interdite.
    /// Les ecritures directes du noyau (`write`) cassent sans exiger le droit
    /// et gardent la page en lecture seule.
    pub fn prepare_casse_cow(&mut self, page: u64, pour_ecriture_utilisateur: bool) -> CasseCow {
        let page = page & !(PAGE_SIZE - 1);
        let entree = match self.entry_mut(page, false) {
            Some(entree) => entree,
            None => return CasseCow::PasCow,
        };
        let ancienne_pte = *entree;
        if ancienne_pte & PTE_PRESENT == 0 {
            return CasseCow::PasCow;
        }
        if ancienne_pte & PTE_COW == 0 {
            return if ancienne_pte & (PTE_WRITE | PTE_USER) == PTE_WRITE | PTE_USER {
                CasseCow::DejaInscriptible
            } else {
                CasseCow::PasCow
            };
        }
        let ancienne = ancienne_pte & ADDR_MASK;
        let depuis_zero = est_page_zero(ancienne);
        if !depuis_zero && !self.pages.contains(&ancienne) {
            // Une PTE COW sur une frame que l'espace ne possede pas viole
            // l'invariant : refuser plutot que d'ecrire dans la frame d'un
            // autre.
            return CasseCow::PasCow;
        }
        let ecriture_permise = ancienne_pte & PTE_COW_ECRITURE != 0;
        if pour_ecriture_utilisateur && !ecriture_permise {
            return CasseCow::PasCow;
        }
        let drapeaux = (ancienne_pte & !ADDR_MASK & !PTE_COW_BITS)
            | if ecriture_permise { PTE_WRITE } else { 0 };
        if !depuis_zero && !COW.lock().est_partagee(ancienne) {
            // Seul proprietaire : promotion sur place. Elargir un droit ne
            // laisse aucune traduction dangereuse ailleurs (au pire une faute
            // de plus, qui verra la page inscriptible).
            *entree = ancienne | drapeaux;
            flush(page);
            COW_PROMOTIONS.fetch_add(1, Ordering::Relaxed);
            return CasseCow::Promue;
        }
        // Depuis la page zero : une frame mise a zero, rien a copier.
        let neuve = match if depuis_zero { alloc_frame() } else { alloc_frame_a_recouvrir() } {
            Some(f) => f,
            None => {
                COW_ECHECS_ALLOCATION.fetch_add(1, Ordering::Relaxed);
                return CasseCow::SansMemoire;
            }
        };
        if !depuis_zero {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    memory::phys_to_virt(ancienne),
                    memory::phys_to_virt(neuve),
                    PAGE_SIZE as usize,
                );
            }
        }
        // Le contenu precede la PTE (P13_PAGE_REMPLIE_AVANT_PUBLICATION_V1).
        core::sync::atomic::fence(Ordering::Release);
        *entree = neuve | drapeaux;
        flush(page);
        if depuis_zero {
            // La page quitte la classe « partagee » pour devenir anonyme.
            self.resident.unmapped(ancienne_pte);
            *entree = (neuve | drapeaux) & !PTE_RSS_KIND_MASK | ResidentKind::Anonymous.bits();
            self.resident.mapped(*entree);
            PAGES_ZERO_MATERIALISEES.fetch_add(1, Ordering::Relaxed);
        } else {
            self.pages.remove(&ancienne);
            COW_COPIES.fetch_add(1, Ordering::Relaxed);
        }
        self.pages.insert(neuve);
        CasseCow::Copiee { invalidation: self.invalidation(page, PAGE_SIZE), ancienne }
    }

    /// BOUCHAUD_PAGE_ZERO_V1 : projette `page` (absente) sur la page zero,
    /// en lecture seule et copie-a-l'ecriture ; `drapeaux` sont ceux de la
    /// VMA (le droit d'ecrire devient `PTE_COW_ECRITURE`). Faux si la page
    /// zero n'existe pas ou si une PTE est deja la.
    pub fn mappe_page_zero(&mut self, page: u64, drapeaux: u64) -> bool {
        let Some(zero) = page_zero() else { return false };
        let mut pte = (drapeaux & !PTE_WRITE & !PTE_COW_BITS) | PTE_COW;
        if drapeaux & PTE_WRITE != 0 {
            pte |= PTE_COW_ECRITURE;
        }
        if self.map_accounted(page & !(PAGE_SIZE - 1), zero, pte, ResidentKind::Shared) {
            PAGES_ZERO_PROJETEES.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Diagnostic d'une faute d'ecriture non resolue : (PTE brute, frame
    /// possedee, nombre de proprietaires de la frame).
    pub fn etat_cow(&mut self, page: u64) -> Option<(u64, bool, u32)> {
        let pte = *self.entry_mut(page & !(PAGE_SIZE - 1), false)?;
        let frame = pte & ADDR_MASK;
        Some((pte, self.pages.contains(&frame), COW.lock().proprietaires(frame)))
    }

    /// La PTE de `page` est-elle en copie-a-l'ecriture ?
    pub fn est_cow(&mut self, page: u64) -> bool {
        matches!(self.entry_mut(page & !(PAGE_SIZE - 1), false),
            Some(e) if *e & (PTE_PRESENT | PTE_COW) == PTE_PRESENT | PTE_COW)
    }

    /// Traduit une adresse virtuelle utilisateur en adresse physique.
    pub fn translate(&mut self, virt: u64) -> Option<u64> {
        let entry = *self.entry_mut(virt, false)?;
        if entry & PTE_PRESENT == 0 {
            return None;
        }
        Some((entry & ADDR_MASK) | (virt & (PAGE_SIZE - 1)))
    }

    /// L'adresse est-elle accessible en ecriture depuis le ring 3 ?
    pub fn writable(&mut self, virt: u64) -> bool {
        match self.entry_mut(virt, false) {
            Some(entry) => *entry & (PTE_PRESENT | PTE_WRITE | PTE_USER) == (PTE_PRESENT | PTE_WRITE | PTE_USER),
            None => false,
        }
    }

    /// Nombre de pages de donnees actuellement mappees.
    pub fn mapped_pages(&self) -> usize {
        self.pages.len()
    }

    /// Rend le RSS exact en O(1). Le cout ne depend ni du RSS, ni du nombre de
    /// threads qui partagent cet espace.
    pub fn resident_stats(&self) -> ResidentStats {
        self.resident
    }

    /// BOUCHAUD_C22_PTE_OWNER_V22: existing PTE classification, no faults or mappings.
    /// Classification is shared with the kernel's resident RSS accounting.
    pub fn c22_rss_class_entry(entry: u64) -> usize {
        match ResidentKind::from_entry(entry) {
            ResidentKind::Anonymous => 0,
            ResidentKind::FilePrivate => 1,
            ResidentKind::Shared => 2,
            ResidentKind::Device => 3,
            ResidentKind::Untracked => 4,
        }
    }

    /// Enumere les pages utilisateur mappees : (adresse virtuelle, entree).
    ///
    /// Parcourt les quatre niveaux du creneau utilisateur. Sert a `fork`, qui
    /// doit recopier tout ce que le parent a mappe, et au diagnostic.
    ///
    /// Les `USER_SLOTS` entrees PML4 sont parcourues, pas seulement la
    /// premiere. C'est le genre de detail qu'un elargissement du creneau rend
    /// soudain determinant : avec la pile deplacee en haut de l'espace, elle
    /// tombe dans la derniere entree, et n'en parcourir qu'une faisait rendre
    /// a `fork` un enfant sans pile — qui fautait a son premier appel.
    pub fn iter_user_pages(&self) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        let pml4 = table_at(self.pml4);
        for creneau in 0..USER_SLOTS {
            let slot = user_slot() + creneau;
            let entry4 = pml4[slot];
            if entry4 & PTE_PRESENT == 0 {
                continue;
            }
            let pdpt = table_at(entry4 & ADDR_MASK);
            for (i3, &entry3) in pdpt.iter().enumerate() {
                if entry3 & PTE_PRESENT == 0 {
                    continue;
                }
                let pd = table_at(entry3 & ADDR_MASK);
                for (i2, &entry2) in pd.iter().enumerate() {
                    if entry2 & PTE_PRESENT == 0 || entry2 & PTE_HUGE != 0 {
                        continue;
                    }
                    let pt = table_at(entry2 & ADDR_MASK);
                    for (i1, &entry1) in pt.iter().enumerate() {
                        if entry1 & PTE_PRESENT == 0 {
                            continue;
                        }
                        let virt = ((slot as u64) << 39)
                            | ((i3 as u64) << 30)
                            | ((i2 as u64) << 21)
                            | ((i1 as u64) << 12);
                        out.push((virt, entry1));
                    }
                }
            }
        }
        out
    }

    /// Cette page appartient-elle a l'espace (frame allouee par lui) ?
    ///
    /// Les pages qui n'appartiennent pas a l'espace sont celles empruntees a un
    /// autre proprietaire : framebuffer materiel, cache de pages partage. Elles
    /// ne doivent etre ni dupliquees ni liberees avec le processus.
    pub fn owns_frame(&self, phys: u64) -> bool {
        self.pages.contains(&(phys & ADDR_MASK))
    }

    /// Mappe une frame qui appartient a quelqu'un d'autre (framebuffer, cache
    /// de pages partage) : elle ne sera ni liberee ni dupliquee avec l'espace.
    pub fn map_foreign(&mut self, virt: u64, phys: u64, flags: u64) -> bool {
        self.map(virt, phys, flags)
    }

    /// Variante comptabilisee pour framebuffer et caches partages.
    pub fn map_foreign_accounted(
        &mut self,
        virt: u64,
        phys: u64,
        flags: u64,
        kind: ResidentKind,
    ) -> bool {
        self.map_accounted(virt, phys, flags, kind)
    }

    /// Duplique l'espace utilisateur de `self` pour `fork`.
    ///
    /// BOUCHAUD_COW_V1 : les frames POSSEDEES ne sont plus recopiees. Elles
    /// sont partagees en copie-a-l'ecriture : la PTE du pere et celle du fils
    /// pointent la meme frame, en lecture seule avec `PTE_COW`, et la table
    /// compte les proprietaires. La premiere ecriture de l'un ou de l'autre
    /// copie (ou promeut, s'il est devenu seul).
    ///
    /// Les pages empruntees (framebuffer, cache partage, `MAP_SHARED`) sont
    /// re-mappees sur les memes frames, inscriptibles comme avant : c'est la
    /// semantique de `MAP_SHARED`.
    ///
    /// L'appelant DOIT executer l'invalidation rendue -- hors du verrou `Mm`
    /// -- AVANT de laisser courir le fils : un autre fil du pere peut encore
    /// tenir une traduction inscriptible d'une frame desormais partagee.
    ///
    /// Echec (tables du fils introuvables) : le fils est detruit, sa
    /// destruction rend ses parts ; les PTE du pere restent en COW, ce qui
    /// est sans danger (sa prochaine ecriture promeut, la frame etant
    /// redevenue exclusive).
    pub fn duplicate(&mut self) -> Option<(AddressSpace, DuplicationCompte, Option<TlbInvalidation>)> {
        let mut child = AddressSpace::new()?;
        #[cfg(feature = "banc-gpu-venus")]
        child.baux_gpu.clone_from(&self.baux_gpu);
        let mut compte = DuplicationCompte::default();
        let pages = self.iter_user_pages();
        compte.pages_vues = pages.len() as u64;
        let mut pere_modifie = false;
        let mut partagees = BTreeSet::new();
        // Une seule prise de la table pour tout le fork. Ordre respecte :
        // `Mm` (tenu par l'appelant) -> `COW` -> allocateur (tables du fils).
        let mut table = COW.lock();
        for (virt, entry) in pages {
            let phys = entry & ADDR_MASK;
            if self.owns_frame(phys) {
                let partagee = if entry & PTE_COW != 0 {
                    entry // deja partagee (fils d'un fils) : droits inchanges
                } else if entry & PTE_WRITE != 0 {
                    (entry & !PTE_WRITE) | PTE_COW | PTE_COW_ECRITURE
                } else {
                    entry | PTE_COW
                };
                if partagee != entry {
                    if let Some(e) = self.entry_mut(virt, false) {
                        *e = partagee;
                        flush(virt);
                        pere_modifie = true;
                    }
                }
                // Une part par frame et par fork, meme si la frame est
                // projetee a deux adresses de l'espace.
                if partagees.insert(phys) {
                    table.partage(phys);
                    child.pages.insert(phys);
                }
                if !child.map(virt, phys, partagee & !ADDR_MASK) {
                    // La destruction du fils rend ses parts : la table doit
                    // etre libre AVANT.
                    drop(table);
                    return None;
                }
                compte.pages_partagees += 1;
            } else {
                if !child.map(virt, phys, entry & !ADDR_MASK) {
                    drop(table);
                    return None;
                }
                compte.pages_empruntees += 1;
            }
        }
        drop(table);
        let invalidation = pere_modifie.then(|| self.invalidation(0, 0));
        Some((child, compte, invalidation))
    }

    /// Copie `len` octets depuis la memoire utilisateur vers un tampon noyau.
    /// Echoue si une page de la plage n'est pas mappee.
    pub fn read(&mut self, virt: u64, out: &mut [u8]) -> bool {
        let mut done = 0usize;
        while done < out.len() {
            let addr = virt + done as u64;
            let phys = match self.translate(addr) {
                Some(p) => p,
                None => return false,
            };
            let offset = (addr & (PAGE_SIZE - 1)) as usize;
            let chunk = core::cmp::min(PAGE_SIZE as usize - offset, out.len() - done);
            unsafe {
                core::ptr::copy_nonoverlapping(memory::phys_to_virt(phys), out.as_mut_ptr().add(done), chunk);
            }
            done += chunk;
        }
        true
    }

    /// Copie un tampon noyau vers la memoire utilisateur.
    ///
    /// BOUCHAUD_COW_V1 : le noyau ecrit par la vue physique, sans passer par
    /// le droit d'ecriture de la PTE. Une page COW est donc d'abord cassee
    /// pour CET espace -- sinon l'ecriture modifierait aussi la memoire de
    /// l'autre proprietaire. Les chemins courants (`user_write`, usercopy
    /// native) la cassent AVANT de prendre le verrou ; ici n'arrivent que les
    /// ecritures directes, et le shootdown eventuel s'execute alors sous le
    /// verrou `Mm` (les coeurs vises l'acquittent en IRQ).
    pub fn write(&mut self, virt: u64, data: &[u8]) -> bool {
        if data.is_empty() {
            return true;
        }
        let fin = match virt.checked_add(data.len() as u64) {
            Some(fin) => fin,
            None => return false,
        };
        let mut page = virt & !(PAGE_SIZE - 1);
        while page < fin {
            match self.prepare_casse_cow(page, false) {
                CasseCow::Copiee { invalidation, ancienne } => {
                    invalidation.execute();
                    rend_frame_possedee(ancienne);
                }
                CasseCow::SansMemoire => return false,
                CasseCow::PasCow | CasseCow::Promue | CasseCow::DejaInscriptible => {}
            }
            page += PAGE_SIZE;
        }
        let mut done = 0usize;
        while done < data.len() {
            let addr = virt + done as u64;
            let phys = match self.translate(addr) {
                Some(p) => p,
                None => return false,
            };
            let offset = (addr & (PAGE_SIZE - 1)) as usize;
            let chunk = core::cmp::min(PAGE_SIZE as usize - offset, data.len() - done);
            unsafe {
                core::ptr::copy_nonoverlapping(data.as_ptr().add(done), memory::phys_to_virt(phys), chunk);
            }
            done += chunk;
        }
        true
    }

    /// Lit une chaine C (terminee par 0) depuis l'espace utilisateur.
    pub fn read_cstr(&mut self, virt: u64, max: usize) -> Option<alloc::string::String> {
        let mut bytes = Vec::new();
        for i in 0..max {
            let mut byte = [0u8; 1];
            if !self.read(virt + i as u64, &mut byte) {
                return None;
            }
            if byte[0] == 0 {
                return Some(alloc::string::String::from_utf8_lossy(&bytes).into_owned());
            }
            bytes.push(byte[0]);
        }
        None
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        // Ne jamais liberer un espace encore actif dans CR3.
        if current_pml4() == self.pml4 {
            self.mark_inactive(smp::cpu_index());
            unsafe {
                let frame = PhysFrame::containing_address(PhysAddr::new(KERNEL_PML4));
                Cr3::write(frame, Cr3Flags::empty());
            }
        }
        debug_assert_eq!(
            self.active_cpus(),
            0,
            "vmm: destruction d'un AddressSpace encore actif sur un CPU"
        );
        // Voir `free_frames_lot` : un espace d'adressage de 256 Mio rend
        // soixante-cinq mille frames, et les rendre une par une prenait le
        // verrou de l'allocateur autant de fois.
        // BOUCHAUD_COW_V1 : les frames partagees avec un autre espace lui
        // restent.
        free_frames_lot(frames_a_liberer(self.pages.iter().copied()));
        free_frames_lot(self.tables.iter().copied());
        free_frame(self.pml4);
    }
}

/// Invalide l'entree TLB d'une page.
pub fn flush(virt: u64) {
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) virt, options(nostack, preserves_flags));
    }
}

/// Invalidation locale utilisee par le handler IPI TLB.
/// Une grande plage recharge CR3 en une seule operation; une petite plage
/// utilise INVLPG pour limiter le cout.
pub fn flush_local_range(start: u64, len: u64) {
    let pages = if len == 0 {
        u64::MAX
    } else {
        (len + PAGE_SIZE - 1) / PAGE_SIZE
    };

    if pages > 64 {
        let (frame, flags) = Cr3::read();
        unsafe { Cr3::write(frame, flags); }
        return;
    }

    let begin = start & !(PAGE_SIZE - 1);
    let mut page = begin;
    let end = begin.saturating_add(pages.saturating_mul(PAGE_SIZE));
    while page < end {
        flush(page);
        page = page.saturating_add(PAGE_SIZE);
    }
}

/// Restaure la PML4 du noyau (retour au contexte noyau pur).
/// BOUCHAUD_CR3_MEME_ESPACE_V1 : chargements de CR3 faits et evites.
pub static CR3_ECRITS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static CR3_EVITES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static CR3_NOYAU: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static CR3_NOYAU_EVITES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Commutations entre deux fils du meme processus ou le coeur garde l'espace.
pub static ESPACES_GARDES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

pub fn activate_kernel() {
    // BOUCHAUD_CR3_MEME_ESPACE_V1 : d'un fil noyau a un autre, la PML4 noyau
    // est deja chargee ; elle n'est jamais retiree, ses traductions restent
    // justes. Recharger CR3 ne ferait que vider le TLB.
    if Cr3::read().0.start_address().as_u64() == unsafe { KERNEL_PML4 } {
        CR3_NOYAU_EVITES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        return;
    }
    CR3_NOYAU.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    unsafe {
        let frame = PhysFrame::containing_address(PhysAddr::new(KERNEL_PML4));
        Cr3::write(frame, Cr3Flags::empty());
    }
}

/// Affiche l'etat de la memoire virtuelle (commande `vmstat`).
pub fn print_info() {
    let (used, free, total) = frame_stats();
    crate::println!("Memoire virtuelle (pagination x86_64, 4 niveaux):");
    crate::println!("  PML4 noyau      : {:#x}", unsafe { KERNEL_PML4 });
    crate::println!("  PML4 active     : {:#x}", current_pml4());
    crate::println!("  creneau user    : #{} -> {:#x}", user_slot(), user_slot_base());
    crate::println!("  frames totales  : {} ({} MiB)", total, total * PAGE_SIZE / (1024 * 1024));
    crate::println!("  frames utilisees: {}", used);
    crate::println!("  frames libres   : {} ({} MiB)", free, free * PAGE_SIZE / (1024 * 1024));
    crate::println!("  base chargement : {:#x}", user_load_base());
    crate::println!("  base ld.so      : {:#x}", user_interp_base());
    crate::println!("  base mmap       : {:#x}", user_mmap_base());
    crate::println!("  sommet de pile  : {:#x} ({} MiB)", user_stack_top(), USER_STACK_SIZE / (1024 * 1024));
}
