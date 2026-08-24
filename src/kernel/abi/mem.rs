//! Memoire du processus : `brk`, `mmap`, `munmap`, `mprotect`, `mremap`.
//!
//! L'allocateur de musl (`mallocng`) demande sa memoire par `mmap` anonyme et
//! par `brk` ; un `ld.so` mappe les segments d'une bibliotheque partagee par
//! `mmap` de fichier puis ajuste les droits par `mprotect`. Les deux chemins
//! sont donc necessaires avant meme d'esperer lancer un binaire dynamique.
//!
//! Deux traitements pour un `mmap` de fichier, selon le mode demande :
//!
//! - `MAP_PRIVATE` : copie du contenu dans des pages appartenant au processus.
//!   C'est le cas de `ld.so` chargeant une bibliotheque ;
//! - `MAP_SHARED` : les pages viennent d'un **cache global** indexe par
//!   (fichier, numero de page). Deux processus qui mappent le meme fichier
//!   pointent alors sur les memes frames physiques, et `msync` repercute les
//!   ecritures dans le contenu du fichier. Sans ce cache, `MAP_SHARED` serait
//!   un mensonge : chacun travaillerait sur sa copie. Le cache et son cycle de
//!   vie vivent dans [`crate::kernel::partage`] ; ce module ne fait qu'y
//!   puiser et lui declarer les mappages qu'il cree.
//!
//! `/dev/fb0` suit une troisieme voie : ses pages sont celles de la memoire
//! video, empruntees telles quelles.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::abi::errno;
use crate::kernel::fd::FdKind;
use crate::kernel::partage;
use crate::kernel::task;
use crate::kernel::vma::{self, Backing, Vma};
use crate::kernel::vmm::{self, PAGE_SIZE};

static CLEAN_WRITE_REJECTS: AtomicU64 = AtomicU64::new(0);

fn execute_process_invalidation(
    _process: &Arc<task::Process>,
    invalidation: vmm::TlbInvalidation,
) {
    let depth = crate::kernel::smp_lock::suspend_for_schedule();
    invalidation.execute();
    crate::kernel::smp_lock::resume_after_schedule(depth);
}

fn finish_mapping_replacement(
    process: &Arc<task::Process>,
    retirement: Option<vmm::UnmapRetirement>,
    shared_nodes: &[usize],
    clean_pages: &[crate::kernel::clean_page_cache::Key],
) {
    if let Some(retirement) = retirement {
        execute_process_invalidation(process, retirement.invalidation());
        process.mm.lock().space.finish_unmap(retirement);
    }
    // Une frame MAP_SHARED ne peut être rendue qu'après le même ACK TLB.
    for &node in shared_nodes {
        partage::demappe(node);
    }
    for &key in clean_pages {
        crate::kernel::clean_page_cache::release(key);
    }
}

pub const PROT_NONE: u32 = 0;
pub const PROT_READ: u32 = 1;
pub const PROT_WRITE: u32 = 2;
pub const PROT_EXEC: u32 = 4;

pub const MAP_SHARED: u32 = 0x01;
pub const MAP_PRIVATE: u32 = 0x02;
pub const MAP_FIXED: u32 = 0x10;
pub const MAP_ANONYMOUS: u32 = 0x20;
pub const MAP_NORESERVE: u32 = 0x4000;
pub const MAP_POPULATE: u32 = 0x8000;
pub const MAP_FIXED_NOREPLACE: u32 = 0x100000;

/// Traduit `PROT_*` en drapeaux de table de pages.
pub fn prot_to_flags(prot: u32) -> u64 {
    if prot == PROT_NONE {
        return vmm::PTE_PRESENT | vmm::PTE_NO_EXEC;
    }

    let mut flags = vmm::PTE_PRESENT | vmm::PTE_USER;
    if prot & PROT_WRITE != 0 {
        flags |= vmm::PTE_WRITE;
    }
    if prot & PROT_EXEC == 0 {
        flags |= vmm::PTE_NO_EXEC;
    }
    flags
}

/// `brk` : deplace la fin du tas du processus.
///
/// `brk(0)` renvoie la valeur courante ; toute autre valeur tente de l'ajuster
/// et renvoie la valeur effective (Linux ne renvoie jamais d'erreur ici, la
/// libc compare simplement le retour a sa demande).
pub fn sys_brk(addr: u64) -> i64 {
    let process = task::current_process();
    let mut retirement = None;
    let result = {
        let mut borrowed = process.mm.lock();
        if addr == 0 || addr < borrowed.brk_start {
            return borrowed.brk as i64;
        }

        let current = borrowed.brk;
        if addr > current {
            let start = (current + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
            let end = (addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
            if end > start {
                let growth = end - start;
                if !borrowed.tient_sous_limite(growth) {
                    return current as i64;
                }
                vma::remplace(
                    &mut borrowed.promesses,
                    Vma {
                        id: vma::nouvelle_identite(),
                        debut: start,
                        fin: end,
                        drapeaux: prot_to_flags(PROT_READ | PROT_WRITE),
                        backing: Backing::Zero,
                    },
                );
            }
        } else if addr < current {
            let start = (addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
            let end = (current + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
            if end > start {
                vma::retire(&mut borrowed.promesses, start, end);
                retirement = Some((
                    borrowed.space.prepare_unmap(start, end - start),
                    borrowed.space.pml4(), start, end - start,
                ));
            }
        }
        borrowed.brk = addr;
        addr as i64
    };

    if let Some((retirement, pml4, start, len)) = retirement {
        task::retire_fault_range(pml4, start, len);
        execute_process_invalidation(&process, retirement.invalidation());
        process.mm.lock().space.finish_unmap(retirement);
    }
    result
}


fn plage_user_valide(base: u64, length: u64) -> bool {
    if length == 0 {
        return false;
    }
    let fin = match base.checked_add(length) {
        Some(fin) => fin,
        None => return false,
    };
    vmm::is_user_addr(base) && vmm::is_user_addr(fin - 1)
}

fn choisit_base_mmap(
    process: &mut task::MmState,
    addr: u64,
    length: u64,
    flags: u32,
) -> Result<u64, i64> {
    let fixed = flags & (MAP_FIXED | MAP_FIXED_NOREPLACE) != 0;

    if fixed {
        if addr == 0 || addr & (PAGE_SIZE - 1) != 0 {
            return Err(-errno::EINVAL);
        }
        let fin = addr.checked_add(length).ok_or(-errno::ENOMEM)?;
        if !plage_user_valide(addr, length) {
            return Err(-errno::ENOMEM);
        }
        if flags & MAP_FIXED_NOREPLACE != 0
            && vma::chevauche(&process.promesses, addr, fin)
        {
            return Err(-errno::EEXIST);
        }
        return Ok(addr);
    }

    if addr != 0 {
        let hint = addr & !(PAGE_SIZE - 1);
        if plage_user_valide(hint, length) {
            let fin = hint + length;
            if !vma::chevauche(&process.promesses, hint, fin) {
                return Ok(hint);
            }
        }
    }

    let limite = vmm::user_slot_base()
        .checked_add(vmm::USER_SPACE_SIZE)
        .ok_or(-errno::ENOMEM)?;
    let base = vma::trouve_trou(
        &process.promesses,
        process.mmap_next,
        length,
        limite,
    )
    .ok_or(-errno::ENOMEM)?;

    process.mmap_next = base
        .checked_add(length)
        .and_then(|value| value.checked_add(PAGE_SIZE))
        .ok_or(-errno::ENOMEM)?;
    Ok(base)
}

fn installe_vma(
    process: &mut task::MmState,
    base: u64,
    length: u64,
    drapeaux: u64,
    backing: Backing,
    fixed: bool,
) -> (Option<vmm::UnmapRetirement>, alloc::vec::Vec<usize>, alloc::vec::Vec<crate::kernel::clean_page_cache::Key>) {
    let fin = base + length;
    let (retirement, shared_nodes, clean_pages) = if fixed {
        let shared_nodes = process.retire_partages(base, length);
        let clean_pages = process.retire_clean_pages(base, length);
        let retirement = process.space.prepare_unmap(base, length);
        vma::retire(&mut process.promesses, base, fin);
        (Some(retirement), shared_nodes, clean_pages)
    } else {
        (None, alloc::vec::Vec::new(), alloc::vec::Vec::new())
    };
    vma::remplace(
        &mut process.promesses,
        Vma {
            id: vma::nouvelle_identite(),
            debut: base,
            fin,
            drapeaux,
            backing,
        },
    );
    (retirement, shared_nodes, clean_pages)
}

/// `mmap`.
pub fn sys_mmap(
    addr: u64,
    length: u64,
    prot: u32,
    flags: u32,
    fd: i32,
    offset: u64,
) -> i64 {
    if length == 0
        || prot & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0
    {
        return -errno::EINVAL;
    }

    let map_type = flags & (MAP_SHARED | MAP_PRIVATE);
    if map_type != MAP_SHARED && map_type != MAP_PRIVATE {
        return -errno::EINVAL;
    }

    let length = match length.checked_add(PAGE_SIZE - 1) {
        Some(value) => value & !(PAGE_SIZE - 1),
        None => return -errno::ENOMEM,
    };

    if flags & MAP_ANONYMOUS == 0 && offset & (PAGE_SIZE - 1) != 0 {
        return -errno::EINVAL;
    }

    let process_rc = task::current_process();
    let fd_kind = process_rc.files.lock().get(fd).map(|desc| desc.kind.clone());
    let ecran = process_rc.metadata.lock().ecran;
    let mut process = process_rc.mm.lock();
    let base = match choisit_base_mmap(&mut process, addr, length, flags) {
        Ok(base) => base,
        Err(error) => return error,
    };
    let fixed = flags & MAP_FIXED != 0;

    let drapeaux = prot_to_flags(prot);

    if flags & MAP_ANONYMOUS == 0 && fd >= 0 {
        if let Some(FdKind::Framebuffer) = &fd_kind {
                if let Some(ecran) = ecran {
                    partage::mappe(ecran.node);
                    let (retirement, liberes, clean_liberes) = installe_vma(
                        &mut process,
                        base,
                        length,
                        drapeaux,
                        Backing::SharedFile {
                            node: ecran.node,
                            mapping_start: base,
                            file_offset: offset,
                            file_size: length,
                        },
                        fixed,
                    );
                    process.partages.push(task::Partage {
                        base,
                        length,
                        node: ecran.node,
                    });
                    let populate =
                        flags & MAP_POPULATE != 0 && prot != PROT_NONE;
                    let pml4 = process.space.pml4();
                    drop(process);
                    if fixed { task::retire_fault_range(pml4, base, length); }
                    finish_mapping_replacement(&process_rc, retirement, &liberes, &clean_liberes);
                    if populate {
                        let mut page = base;
                        while page < base + length {
                            if task::peuple_a_la_demande(page, false) != task::FaultOutcome::Resolved {
                                return -errno::ENOMEM;
                            }
                            page += PAGE_SIZE;
                        }
                    }
                    return base as i64;
                }

                let phys = match crate::drivers::gfx::lfb_phys() {
                    Some(phys) => phys,
                    None => return -errno::ENODEV,
                };
                let (retirement, liberes, clean_liberes) = installe_vma(
                    &mut process,
                    base,
                    length,
                    drapeaux | vmm::PTE_WRITE_THROUGH,
                    Backing::Framebuffer {
                        phys_base: phys,
                        mapping_start: base,
                        phys_offset: offset,
                    },
                    fixed,
                );
                let pml4 = process.space.pml4();
                drop(process);
                if fixed { task::retire_fault_range(pml4, base, length); }
                finish_mapping_replacement(&process_rc, retirement, &liberes, &clean_liberes);
                return base as i64;
        }
    }

    let mut new_shared_node = None;
    let backing = if flags & MAP_ANONYMOUS != 0 {
        Backing::Zero
    } else {
        let node = match fd_kind {
            Some(FdKind::File(node)) => node,
            Some(_) => return -errno::ENODEV,
            None => return -errno::EBADF,
        };

        let total = crate::fs::backing::logical_len(node) as u64;
        let file_size = total.saturating_sub(offset).min(length);
        if flags & MAP_SHARED != 0 {
            if prot & PROT_WRITE != 0 && crate::fs::backing::is_disk_backed(node) {
                return -errno::EACCES;
            }
            partage::mappe(node);
            new_shared_node = Some(node);
            Backing::SharedFile {
                node,
                mapping_start: base,
                file_offset: offset,
                file_size,
            }
        } else {
            Backing::File {
                node,
                mapping_start: base,
                file_offset: offset,
                file_size,
            }
        }
    };

    if !fixed && !process.tient_sous_limite(length) {
        if let Backing::SharedFile { node, .. } = backing {
            partage::demappe(node);
        }
        return -errno::ENOMEM;
    }

    let (retirement, liberes, clean_liberes) = installe_vma(
        &mut process,
        base,
        length,
        drapeaux,
        backing,
        fixed,
    );
    if let Some(node) = new_shared_node {
        process.partages.push(task::Partage { base, length, node });
    }

    let populate = flags & MAP_POPULATE != 0 && prot != PROT_NONE;
    let pml4 = process.space.pml4();
    drop(process);
    if fixed { task::retire_fault_range(pml4, base, length); }
    finish_mapping_replacement(&process_rc, retirement, &liberes, &clean_liberes);

    if populate {
        let mut page = base;
        while page < base + length {
            if task::peuple_a_la_demande(page, false) != task::FaultOutcome::Resolved {
                return -errno::ENOMEM;
            }
            page += PAGE_SIZE;
        }
    }

    base as i64
}

/// `munmap`.
///
/// Deux choses a defaire, pas une : les entrees de table de pages, et la
/// reference que la plage detenait sur le cache partage. Oublier la seconde
/// etait exactement la fuite corrigee ici.
pub fn sys_munmap(addr: u64, length: u64) -> i64 {
    if length == 0 || addr & (PAGE_SIZE - 1) != 0 {
        return -errno::EINVAL;
    }

    let length = match length.checked_add(PAGE_SIZE - 1) {
        Some(value) => value & !(PAGE_SIZE - 1),
        None => return -errno::EINVAL,
    };
    let fin = match addr.checked_add(length) {
        Some(fin) => fin,
        None => return -errno::EINVAL,
    };

    let process = task::current_process();
    let (retirement, liberes, clean_liberes, pml4) = {
        let mut process = process.mm.lock();
        let liberes = process.retire_partages(addr, length);
        let clean_liberes = process.retire_clean_pages(addr, length);
        vma::retire(&mut process.promesses, addr, fin);
        let retirement = process.space.prepare_unmap(addr, length);
        let pml4 = process.space.pml4();
        (retirement, liberes, clean_liberes, pml4)
    };

    task::retire_fault_range(pml4, addr, length);

    // No Mm guard crosses this window; remote faults and the IPI handler can progress.
    execute_process_invalidation(&process, retirement.invalidation());

    process.mm.lock().space.finish_unmap(retirement);

    for node in liberes {
        partage::demappe(node);
    }
    for key in clean_liberes {
        crate::kernel::clean_page_cache::release(key);
    }
    0
}

/// `mprotect`.
pub fn sys_mprotect(addr: u64, length: u64, prot: u32) -> i64 {
    if length == 0 {
        return 0;
    }
    if addr & (PAGE_SIZE - 1) != 0
        || prot & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0
    {
        return -errno::EINVAL;
    }

    let length = match length.checked_add(PAGE_SIZE - 1) {
        Some(value) => value & !(PAGE_SIZE - 1),
        None => return -errno::ENOMEM,
    };
    let fin = match addr.checked_add(length) {
        Some(fin) => fin,
        None => return -errno::ENOMEM,
    };

    if !plage_user_valide(addr, length) {
        return -errno::ENOMEM;
    }

    let process_rc = task::current_process();
    let mut process = process_rc.mm.lock();

    // Clean cache frames are deliberately shared without COW. Never make one
    // writable in place: that would grant one process writes into every other
    // address space mapping the same ELF page.
    if prot & PROT_WRITE != 0 && process.has_clean_pages(addr, length) {
        let count = CLEAN_WRITE_REJECTS.fetch_add(1, Ordering::Relaxed) + 1;
        crate::kernel::dmesg::log_fmt(format_args!(
            "[MM-CLEAN-WRITE-REJECT] pid={} addr={:#x} len={} count={}",
            process_rc.pid, addr, length, count,
        ));
        return -errno::EACCES;
    }
    if prot & PROT_WRITE != 0 && process.promesses.iter().any(|vma| {
        vma.debut < fin && vma.fin > addr && matches!(vma.backing,
            Backing::SharedFile { node, .. } if crate::fs::backing::is_disk_backed(node))
    }) {
        return -errno::EACCES;
    }

    if !vma::protege(
        &mut process.promesses,
        addr,
        fin,
        prot_to_flags(prot),
    ) {
        // Compatibilite transitoire : certaines piles creees eager n'ont pas
        // encore de VMA explicite, mais leurs pages sont bien residentes.
        let mut page = addr;
        let mut resident = true;
        while page < fin {
            if process.space.translate(page).is_none() {
                resident = false;
                break;
            }
            page += PAGE_SIZE;
        }
        if !resident {
            return -errno::ENOMEM;
        }
        vma::remplace(
            &mut process.promesses,
            Vma {
                id: vma::nouvelle_identite(),
                debut: addr,
                fin,
                drapeaux: prot_to_flags(prot),
                backing: Backing::Zero,
            },
        );
    }

    let invalidation = process
        .space
        .prepare_protect(addr, length, prot_to_flags(prot));
    let pml4 = process.space.pml4();
    drop(process);
    task::retire_fault_range(pml4, addr, length);
    if let Some(invalidation) = invalidation {
        execute_process_invalidation(&process_rc, invalidation);
    }
    0
}

/// `mremap` : agrandissement par nouvelle allocation + recopie.
pub fn sys_mremap(
    old_addr: u64,
    old_size: u64,
    new_size: u64,
    _flags: u32,
) -> i64 {
    if old_size == 0 || new_size == 0 {
        return -errno::EINVAL;
    }

    if new_size <= old_size {
        if old_size > new_size {
            let start =
                (old_addr + new_size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
            let end =
                (old_addr + old_size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
            if end > start {
                let result = sys_munmap(start, end - start);
                if result < 0 {
                    return result;
                }
            }
        }
        return old_addr as i64;
    }

    let new_addr = sys_mmap(
        0,
        new_size,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE,
        -1,
        0,
    );
    if new_addr < 0 {
        return new_addr;
    }

    const CHUNK: usize = 64 * 1024;
    let mut copied = 0u64;
    while copied < old_size {
        let wanted =
            core::cmp::min(CHUNK as u64, old_size - copied) as usize;
        let data = match super::user_read(old_addr + copied, wanted) {
            Some(data) => data,
            None => {
                let _ = sys_munmap(new_addr as u64, new_size);
                return -errno::EFAULT;
            }
        };
        if !super::user_write(new_addr as u64 + copied, &data) {
            let _ = sys_munmap(new_addr as u64, new_size);
            return -errno::EFAULT;
        }
        copied += wanted as u64;
    }

    let result = sys_munmap(old_addr, old_size);
    if result < 0 {
        let _ = sys_munmap(new_addr as u64, new_size);
        return result;
    }
    new_addr
}


/// `madvise`.
///
/// DONTNEED/FREE abandonnent les frames residentes et conservent la VMA.
pub fn sys_madvise(addr: u64, length: u64, advice: i32) -> i64 {
    const MADV_NORMAL: i32 = 0;
    const MADV_RANDOM: i32 = 1;
    const MADV_SEQUENTIAL: i32 = 2;
    const MADV_WILLNEED: i32 = 3;
    const MADV_DONTNEED: i32 = 4;
    const MADV_FREE: i32 = 8;
    const MADV_DONTFORK: i32 = 10;
    const MADV_DOFORK: i32 = 11;
    const MADV_HUGEPAGE: i32 = 14;
    const MADV_NOHUGEPAGE: i32 = 15;

    if length == 0 {
        return 0;
    }
    if addr & (PAGE_SIZE - 1) != 0 {
        return -errno::EINVAL;
    }

    let length = match length.checked_add(PAGE_SIZE - 1) {
        Some(value) => value & !(PAGE_SIZE - 1),
        None => return -errno::EINVAL,
    };
    let fin = match addr.checked_add(length) {
        Some(fin) => fin,
        None => return -errno::EINVAL,
    };

    let process = task::current_process();
    let retirement = {
        let mut borrowed = process.mm.lock();
        if !vma::couvre(&borrowed.promesses, addr, fin) {
            return -errno::ENOMEM;
        }
        match advice {
            // La VMA reste : le prochain accès rematérialise la page.
            MADV_DONTNEED | MADV_FREE => {
                Some(borrowed.space.prepare_unmap(addr, length))
            }
            MADV_NORMAL
            | MADV_RANDOM
            | MADV_SEQUENTIAL
            | MADV_WILLNEED
            | MADV_DONTFORK
            | MADV_DOFORK
            | MADV_HUGEPAGE
            | MADV_NOHUGEPAGE => None,
            _ => return -errno::EINVAL,
        }
    };
    if let Some(retirement) = retirement {
        execute_process_invalidation(&process, retirement.invalidation());
        process.mm.lock().space.finish_unmap(retirement);
    }
    0
}

/// `msync` : repercute les ecritures d'un `MAP_SHARED` vers le fichier.
///
/// Si l'adresse designe une plage partagee connue du processus, on ne recopie
/// que le nœud concerne ; sinon on recopie tout, ce qui reste correct — les
/// frames sont la source de verite, le contenu RAMFS n'en est que le reflet.
pub fn sys_msync(addr: u64, _length: u64) -> i64 {
    let node = {
        let process = task::current_process();
        let process = process.mm.lock();
        process.partage_a(addr)
    };
    match node {
        Some(node) => partage::writeback(node),
        None => partage::writeback_tout(),
    }
    0
}
