//! BOUCHAUD_PROJECTION_NEUTRE_V1 -- la mecanique des projections d'un espace
//! utilisateur, hors de toute ABI.
//!
//! Elle vivait dans `compat/linux/mem.rs`, si bien que l'ABI NATIVE
//! (`bo_shm_map` / `bo_shm_unmap`) appelait la compatibilite Linux pour
//! projeter une `SharedRegion` : le coeur natif reposait sur la couche qu'il
//! doit pouvoir remplacer (`tools/verifie-abi-native.py`). Les deux ABI
//! traduisent maintenant LEURS arguments (`PROT_*`/`MAP_*`, droits d'un
//! handle) puis appellent ce module :
//!
//! * [`trouve_base`] : un trou dans l'espace, au-dela de `mmap_next` ;
//! * [`installe_vma`] : la promesse (VMA), en retirant ce qu'elle remplace ;
//! * [`finit_remplacement`] : invalidation TLB puis rendu des frames ;
//! * [`demappe`] : le retrait d'une plage, dans l'ordre invalidation ->
//!   frames -> regions natives -> cache partage ;
//! * [`projette_region`] / [`retire_projection`] : la projection d'une region
//!   native, jamais executable (W^X par construction).
//!
//! Aucun changement de comportement : le code est celui de `mem.rs`.

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::kernel::clean_page_cache;
use crate::kernel::partage;
use crate::kernel::task;
use crate::kernel::vma::{self, Backing, Vma};
use crate::kernel::vmm::{self, PAGE_SIZE};

/// Ce qu'un remplacement de projection doit rendre APRES l'invalidation.
pub(crate) type Remplaces = (Option<vmm::UnmapRetirement>, Vec<usize>, Vec<clean_page_cache::Key>);

/// Un trou de `length` octets apres `mmap_next`, et `mmap_next` avance (une
/// page de garde entre deux projections).
pub(crate) fn trouve_base(mm: &mut task::MmState, length: u64) -> Option<u64> {
    let limite = vmm::user_slot_base().checked_add(vmm::USER_SPACE_SIZE)?;
    let base = vma::trouve_trou(&mm.promesses, mm.mmap_next, length, limite)?;
    mm.mmap_next = base.checked_add(length)?.checked_add(PAGE_SIZE)?;
    Some(base)
}

/// Installe la promesse `[base, base+length)`. `fixe` : ce qui occupait la
/// plage est retire (et rendu par [`finit_remplacement`], apres l'invalidation).
pub(crate) fn installe_vma(
    mm: &mut task::MmState,
    base: u64,
    length: u64,
    drapeaux: u64,
    backing: Backing,
    fixe: bool,
) -> Remplaces {
    let fin = base + length;
    let (retirement, shared_nodes, clean_pages) = if fixe {
        let shared_nodes = mm.retire_partages(base, length);
        let clean_pages = mm.retire_clean_pages(base, length);
        let retirement = mm.space.prepare_unmap(base, length);
        vma::retire(&mut mm.promesses, base, fin);
        #[cfg(feature = "banc-gpu-venus")]
        mm.space.retire_baux_gpu_sans_vma(&mm.promesses);
        (Some(retirement), shared_nodes, clean_pages)
    } else {
        (None, Vec::new(), Vec::new())
    };
    vma::remplace(
        &mut mm.promesses,
        Vma { id: vma::nouvelle_identite(), debut: base, fin, drapeaux, backing },
    );
    (retirement, shared_nodes, clean_pages)
}

/// Apres le verrou `Mm` : invalidation, PUIS rendu de ce qui a ete remplace.
/// Une frame `MAP_SHARED` ne peut etre rendue qu'apres le meme ACK TLB.
pub(crate) fn finit_remplacement(
    process: &Arc<task::Process>,
    retirement: Option<vmm::UnmapRetirement>,
    shared_nodes: &[usize],
    clean_pages: &[clean_page_cache::Key],
) {
    if let Some(retirement) = retirement {
        retirement.invalidation().execute();
        process.mm.lock().space.finish_unmap(retirement);
    }
    for &node in shared_nodes {
        partage::demappe(node);
    }
    for &key in clean_pages {
        clean_page_cache::release(key);
    }
}

/// Retire `[addr, addr+length)` (page-aligne, deja valide) de `process`.
pub(crate) fn demappe(process: &Arc<task::Process>, addr: u64, length: u64) {
    let fin = addr + length;
    let regions_rendues;
    let (retirement, liberes, clean_liberes, pml4) = {
        let mut mm = process.mm.lock();
        let liberes = mm.retire_partages(addr, length);
        let clean_liberes = mm.retire_clean_pages(addr, length);
        regions_rendues = mm.retire_regions_natives(addr, length);
        vma::retire(&mut mm.promesses, addr, fin);
        let retirement = mm.space.prepare_unmap(addr, length);
        #[cfg(feature = "banc-gpu-venus")]
        {
            let mm = &mut *mm;
            mm.space.retire_baux_gpu_sans_vma(&mm.promesses);
        }
        let pml4 = mm.space.pml4();
        (retirement, liberes, clean_liberes, pml4)
    };

    task::retire_fault_range(pml4, addr, length);

    // Aucun garde `Mm` ne traverse cette fenetre : les fautes distantes et le
    // gestionnaire d'IPI progressent.
    retirement.invalidation().execute();

    process.mm.lock().space.finish_unmap(retirement);
    // Apres l'invalidation : plus aucun coeur ne peut lire ces frames.
    drop(regions_rendues);

    for node in liberes {
        partage::demappe(node);
    }
    for key in clean_liberes {
        clean_page_cache::release(key);
    }
}

/// Pourquoi une projection native a ete refusee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErreurProjection {
    /// Region ou adresse mal alignee, longueur nulle, adresse inconnue.
    Argument,
    /// Plus de place : plafond d'espace du processus, ou pas de trou.
    Espace,
}

/// BOUCHAUD_SHM_PROJECTION_V1 -- projette une region native (physiquement
/// contigue, `phys`, `longueur` arrondie a la page) dans le processus
/// courant, a une adresse choisie par le noyau. Les droits ont ete verifies
/// par l'appelant (handle). Jamais executable : W^X tient par construction.
pub fn projette_region(
    phys: u64,
    longueur: u64,
    ecriture: bool,
    region: Arc<crate::kernel::native::object::Object>,
) -> Result<u64, ErreurProjection> {
    if longueur == 0 || longueur & (PAGE_SIZE - 1) != 0 || phys & (PAGE_SIZE - 1) != 0 {
        return Err(ErreurProjection::Argument);
    }
    let drapeaux = vmm::PTE_PRESENT | vmm::PTE_USER | vmm::PTE_NO_EXEC
        | if ecriture { vmm::PTE_WRITE } else { 0 };
    let process_rc = task::current_process();
    let mut mm = process_rc.mm.lock();
    if !mm.tient_sous_limite(longueur) {
        return Err(ErreurProjection::Espace);
    }
    let base = trouve_base(&mut mm, longueur).ok_or(ErreurProjection::Espace)?;
    let (retirement, liberes, clean_liberes) = installe_vma(
        &mut mm,
        base,
        longueur,
        drapeaux,
        Backing::RegionNative { phys_base: phys, mapping_start: base },
        false,
    );
    mm.regions_natives.push(task::RegionProjetee { base, length: longueur, region });
    drop(mm);
    finit_remplacement(&process_rc, retirement, &liberes, &clean_liberes);
    Ok(base)
}

/// Retire la projection native qui commence EXACTEMENT a `base`.
pub fn retire_projection(base: u64) -> Result<(), ErreurProjection> {
    let process = task::current_process();
    let longueur = {
        let mm = process.mm.lock();
        mm.regions_natives.iter().find(|r| r.base == base).map(|r| r.length)
    }
    .ok_or(ErreurProjection::Argument)?;
    demappe(&process, base, longueur);
    Ok(())
}
