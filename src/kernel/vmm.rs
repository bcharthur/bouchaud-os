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

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::registers::control::{Cr3, Cr3Flags};
use x86_64::structures::paging::PhysFrame;
use x86_64::PhysAddr;

use crate::arch::x86_64::smp;
use crate::kernel::memory;

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

/// Masque de l'adresse physique dans une entree (bits 12..51).
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

// --- Allocateur de frames physiques -----------------------------------------

struct FrameAllocator {
    /// Regions encore vierges : (prochaine frame libre, fin exclusive).
    regions: Vec<(u64, u64)>,
    /// Frames rendues par `free_frame`, reutilisees en priorite.
    freed: Vec<u64>,
    total: u64,
    used: u64,
    allocations: u64,
    frees: u64,
    failures: u64,
    high_watermark: u64,
}

static mut FRAMES: Option<FrameAllocator> = None;

fn frames() -> &'static mut FrameAllocator {
    unsafe {
        if FRAMES.is_none() {
            FRAMES = Some(FrameAllocator {
                regions: Vec::new(),
                freed: Vec::new(),
                total: 0,
                used: 0,
                allocations: 0,
                frees: 0,
                failures: 0,
                high_watermark: 0,
            });
        }
        FRAMES.as_mut().unwrap()
    }
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
    let f = frames();
    f.total += (end - start) / PAGE_SIZE;
    f.regions.push((start, end));
}

/// Alloue une frame physique de 4 KiB, mise a zero. `None` si plus de RAM.
pub fn alloc_frame() -> Option<u64> {
    let f = frames();
    let candidate = if let Some(p) = f.freed.pop() {
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

    f.used = f.used.saturating_add(1);
    f.allocations = f.allocations.wrapping_add(1);
    f.high_watermark = f.high_watermark.max(f.used);
    unsafe {
        core::ptr::write_bytes(memory::phys_to_virt(phys), 0, PAGE_SIZE as usize);
    }
    Some(phys)
}

/// Rend une frame a l'allocateur.
pub fn free_frame(phys: u64) {
    let f = frames();
    if f.used > 0 {
        f.used -= 1;
    }
    f.frees = f.frees.wrapping_add(1);
    f.freed.push(phys & !(PAGE_SIZE - 1));
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
    /// Frames de donnees mappees pour l'utilisateur.
    pages: Vec<u64>,
}

/// Stable CR3 identity shared with `Mm`'s context-switch fast path.
pub(crate) struct AddressSpaceIdentity {
    pml4: u64,
    active_cpus: AtomicU64,
}

impl AddressSpaceIdentity {
    pub(crate) unsafe fn activate(&self) {
        self.mark_active(smp::cpu_index());
        let frame = PhysFrame::containing_address(PhysAddr::new(self.pml4));
        Cr3::write(frame, Cr3Flags::empty());
    }

    pub(crate) fn mark_active(&self, cpu: usize) {
        if cpu < 64 {
            self.active_cpus.fetch_or(1u64 << cpu, Ordering::AcqRel);
        }
    }

    pub(crate) fn mark_inactive(&self, cpu: usize) {
        if cpu < 64 {
            self.active_cpus.fetch_and(!(1u64 << cpu), Ordering::AcqRel);
        }
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
}

impl UnmapRetirement {
    pub fn invalidation(&self) -> TlbInvalidation {
        self.invalidation
    }
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

impl AddressSpace {
    /// Cree un espace d'adressage : entrees noyau reprises telles quelles,
    /// creneau utilisateur vierge et exclusif.
    pub fn new() -> Option<Self> {
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
            }),
            tables: Vec::new(),
            pages: Vec::new(),
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
        self.identity.active_cpus.load(Ordering::Acquire)
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
        self.identity.activate();
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
        match self.entry_mut(virt, true) {
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
                true
            }
            None => false,
        }
    }

    /// Alloue et mappe `len` octets a partir de `virt` (arrondi a la page).
    /// Les pages deja presentes sont conservees (mmap peut recouvrir).
    pub fn map_alloc(&mut self, virt: u64, len: u64, flags: u64) -> bool {
        let start = virt & !(PAGE_SIZE - 1);
        let end = (virt + len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let mut page = start;
        while page < end {
            if self.translate(page).is_none() {
                let frame = match alloc_frame() {
                    Some(f) => f,
                    None => return false,
                };
                self.pages.push(frame);
                if !self.map(page, frame, flags) {
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
                    let new = (*entry & ADDR_MASK) | flags | PTE_PRESENT;
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

    /// Supprime le mapping d'une plage (munmap). Les PTE sont d'abord
    /// retirees partout, puis le shootdown TLB est ACKe, et seulement ensuite
    /// les frames sont rendues. Cet ordre interdit tout use-after-free TLB.
    pub fn prepare_unmap(&mut self, virt: u64, len: u64) -> UnmapRetirement {
        let start = virt & !(PAGE_SIZE - 1);
        let end = (virt + len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let mut page = start;
        let mut retired: Vec<u64> = Vec::new();
        while page < end {
            if let Some(entry) = self.entry_mut(page, false) {
                let phys = *entry & ADDR_MASK;
                if *entry & PTE_PRESENT != 0 {
                    *entry = 0;
                    flush(page);
                    if self.pages.contains(&phys) {
                        retired.push(phys);
                    }
                }
            }
            page += PAGE_SIZE;
        }

        UnmapRetirement {
            invalidation: self.invalidation(start, end.saturating_sub(start)),
            frames: retired,
        }
    }

    /// Rend les frames uniquement apres `UnmapRetirement::invalidation()`.
    pub fn finish_unmap(&mut self, retirement: UnmapRetirement) {
        for phys in retirement.frames {
            if let Some(pos) = self.pages.iter().position(|&f| f == phys) {
                self.pages.swap_remove(pos);
                free_frame(phys);
            }
        }
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

    /// Compte les PTE utilisateur reellement presentes et les classe selon
    /// leur VMA. C'est le RSS reel, y compris MAP_SHARED et device mappings.
    pub fn resident_stats(
        &self,
        regions: &[crate::kernel::vma::Vma],
    ) -> ResidentStats {
        let mut stats = ResidentStats::default();
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
                        stats.total_pages += 1;
                        match crate::kernel::vma::trouve(regions, virt)
                            .map(|region| region.backing)
                        {
                            Some(crate::kernel::vma::Backing::Zero) => {
                                stats.anonymous_pages += 1;
                            }
                            Some(crate::kernel::vma::Backing::File { .. }) => {
                                stats.file_private_pages += 1;
                            }
                            Some(crate::kernel::vma::Backing::SharedFile { .. }) => {
                                stats.shared_pages += 1;
                            }
                            Some(crate::kernel::vma::Backing::Framebuffer { .. }) => {
                                stats.device_pages += 1;
                            }
                            None => stats.untracked_pages += 1,
                        }
                    }
                }
            }
        }

        stats
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

    /// Duplique le contenu utilisateur de `self` dans un espace neuf.
    ///
    /// Copie **immediate** de chaque page, sans copie-a-l'ecriture. C'est plus
    /// couteux qu'un COW au moment du `fork`, mais cela evite d'avoir a
    /// compter les references sur chaque frame et a traiter les fautes
    /// d'ecriture : deux sources de corruption silencieuse difficiles a
    /// diagnostiquer. Le cas d'usage courant (`fork` suivi d'un `execve`)
    /// paie ici une copie que le COW aurait economisee ; c'est un compromis
    /// assume, pas un oubli.
    ///
    /// Les pages empruntees (framebuffer, cache partage) sont re-mappees sur
    /// les memes frames plutot que dupliquees : c'est le comportement attendu
    /// d'un `MAP_SHARED`.
    pub fn duplicate(&self) -> Option<AddressSpace> {
        let mut child = AddressSpace::new()?;
        for (virt, entry) in self.iter_user_pages() {
            let phys = entry & ADDR_MASK;
            let flags = entry & !ADDR_MASK;
            if self.owns_frame(phys) {
                let frame = alloc_frame()?;
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        memory::phys_to_virt(phys),
                        memory::phys_to_virt(frame),
                        PAGE_SIZE as usize,
                    );
                }
                child.pages.push(frame);
                if !child.map(virt, frame, flags) {
                    return None;
                }
            } else if !child.map(virt, phys, flags) {
                return None;
            }
        }
        Some(child)
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
    pub fn write(&mut self, virt: u64, data: &[u8]) -> bool {
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
        for &frame in self.pages.iter() {
            free_frame(frame);
        }
        for &frame in self.tables.iter() {
            free_frame(frame);
        }
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
pub fn activate_kernel() {
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
