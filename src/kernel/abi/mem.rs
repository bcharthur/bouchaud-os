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
//!   un mensonge : chacun travaillerait sur sa copie.
//!
//! `/dev/fb0` suit une troisieme voie : ses pages sont celles de la memoire
//! video, empruntees telles quelles.

use crate::kernel::abi::errno;
use crate::kernel::fd::FdKind;
use crate::kernel::task;
use crate::kernel::vmm::{self, PAGE_SIZE};

pub const PROT_NONE: u32 = 0;
pub const PROT_READ: u32 = 1;
pub const PROT_WRITE: u32 = 2;
pub const PROT_EXEC: u32 = 4;

pub const MAP_SHARED: u32 = 0x01;
pub const MAP_PRIVATE: u32 = 0x02;
pub const MAP_FIXED: u32 = 0x10;
pub const MAP_ANONYMOUS: u32 = 0x20;
pub const MAP_NORESERVE: u32 = 0x4000;

/// Traduit `PROT_*` en drapeaux de table de pages.
pub fn prot_to_flags(prot: u32) -> u64 {
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
    let mut process = process.borrow_mut();

    if addr == 0 || addr < process.brk_start {
        return process.brk as i64;
    }
    let current = process.brk;
    if addr > current {
        let start = (current + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let end = (addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        if end > start {
            let flags = vmm::PTE_PRESENT | vmm::PTE_USER | vmm::PTE_WRITE | vmm::PTE_NO_EXEC;
            if !process.space.map_alloc(start, end - start, flags) {
                return current as i64;
            }
        }
    } else if addr < current {
        let start = (addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let end = (current + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        if end > start {
            process.space.unmap(start, end - start);
        }
    }
    process.brk = addr;
    addr as i64
}

/// `mmap`.
pub fn sys_mmap(addr: u64, length: u64, prot: u32, flags: u32, fd: i32, offset: u64) -> i64 {
    if length == 0 {
        return -errno::EINVAL;
    }
    let length = (length + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let process = task::current_process();
    let mut process = process.borrow_mut();

    // Un fd de framebuffer se mappe sur la vraie memoire video : c'est ce qui
    // permet a un serveur graphique de dessiner sans copie intermediaire.
    if flags & MAP_ANONYMOUS == 0 && fd >= 0 {
        if let Some(desc) = process.files.get(fd) {
            if let FdKind::Framebuffer = desc.kind {
                let base = if addr != 0 && flags & MAP_FIXED != 0 {
                    addr & !(PAGE_SIZE - 1)
                } else {
                    let base = process.mmap_next;
                    process.mmap_next += length + PAGE_SIZE;
                    base
                };
                return match map_framebuffer(&mut process, base, length, offset) {
                    Some(address) => address as i64,
                    None => -errno::ENODEV,
                };
            }
        }
    }

    let base = if flags & MAP_FIXED != 0 && addr != 0 {
        addr & !(PAGE_SIZE - 1)
    } else if addr != 0 && vmm::is_user_addr(addr) && process.space.translate(addr).is_none() {
        addr & !(PAGE_SIZE - 1)
    } else {
        let base = process.mmap_next;
        // Une page de garde entre deux allocations : un debordement fait une
        // faute de page au lieu de corrompre silencieusement le voisin.
        process.mmap_next += length + PAGE_SIZE;
        base
    };

    if !vmm::is_user_addr(base) || !vmm::is_user_addr(base + length) {
        return -errno::ENOMEM;
    }

    // On mappe en ecriture le temps d'initialiser, puis on applique `prot`.
    let temporary = vmm::PTE_PRESENT | vmm::PTE_USER | vmm::PTE_WRITE;
    if !process.space.map_alloc(base, length, temporary) {
        return -errno::ENOMEM;
    }

    if flags & MAP_ANONYMOUS == 0 && fd >= 0 {
        let node = match process.files.get(fd) {
            Some(desc) => match desc.kind {
                FdKind::File(node) => Some(node),
                _ => None,
            },
            None => return -errno::EBADF,
        };
        if let Some(node) = node {
            if flags & MAP_SHARED != 0 {
                // Partage reel : les pages viennent du cache global, les memes
                // frames pour tous les processus qui mappent ce fichier.
                process.space.unmap(base, length);
                if !map_shared_file(&mut process, node, base, length, offset) {
                    return -errno::ENOMEM;
                }
            } else {
                // MAP_PRIVATE : copie du contenu dans des pages a soi.
                let fs = crate::fs::ramfs::fs();
                let content = &fs.nodes[node].content;
                let start = offset as usize;
                if start < content.len() {
                    let end = core::cmp::min(content.len(), start + length as usize);
                    let slice = content[start..end].to_vec();
                    process.space.write(base, &slice);
                }
            }
        }
    }

    if prot != PROT_NONE {
        process.space.protect(base, length, prot_to_flags(prot));
    }
    base as i64
}

/// Cache de pages : (nœud RAMFS, index de page) -> frame physique partagee.
///
/// Sans lui, `MAP_SHARED` sur fichier serait un mensonge : chaque processus
/// travaillerait sur sa copie, et les ecritures ne seraient vues de personne.
/// Avec lui, deux processus qui mappent le meme fichier pointent sur les memes
/// frames — c'est le partage que POSIX promet.
static mut PAGE_CACHE: Option<alloc::vec::Vec<(usize, u64, u64)>> = None;

fn page_cache() -> &'static mut alloc::vec::Vec<(usize, u64, u64)> {
    unsafe {
        if PAGE_CACHE.is_none() {
            PAGE_CACHE = Some(alloc::vec::Vec::new());
        }
        PAGE_CACHE.as_mut().unwrap()
    }
}

/// Frame partagee d'une page de fichier, creee et remplie au premier acces.
fn cached_page(node: usize, page_index: u64) -> Option<u64> {
    if let Some((_, _, frame)) = page_cache().iter().find(|(n, p, _)| *n == node && *p == page_index) {
        return Some(*frame);
    }
    let frame = vmm::alloc_frame()?;
    // Remplissage depuis le contenu du fichier.
    let fs = crate::fs::ramfs::fs();
    let content = &fs.nodes[node].content;
    let start = (page_index * PAGE_SIZE) as usize;
    if start < content.len() {
        let end = core::cmp::min(content.len(), start + PAGE_SIZE as usize);
        unsafe {
            core::ptr::copy_nonoverlapping(
                content[start..end].as_ptr(),
                crate::kernel::memory::phys_to_virt(frame),
                end - start,
            );
        }
    }
    page_cache().push((node, page_index, frame));
    Some(frame)
}

/// Mappe un fichier en partage sur les frames du cache.
fn map_shared_file(
    process: &mut task::Process,
    node: usize,
    base: u64,
    length: u64,
    offset: u64,
) -> bool {
    let flags = vmm::PTE_PRESENT | vmm::PTE_USER | vmm::PTE_WRITE;
    let mut done = 0u64;
    while done < length {
        let page_index = (offset + done) / PAGE_SIZE;
        let frame = match cached_page(node, page_index) {
            Some(frame) => frame,
            None => return false,
        };
        // `map_foreign` : la frame appartient au cache, pas au processus. Elle
        // ne sera donc ni liberee avec lui, ni dupliquee par `fork`.
        if !process.space.map_foreign(base + done, frame, flags) {
            return false;
        }
        done += PAGE_SIZE;
    }
    true
}

/// Recopie les pages partagees d'un fichier vers son contenu RAMFS.
///
/// Appele par `msync` et a la fermeture : c'est ce qui rend les ecritures d'un
/// `MAP_SHARED` visibles par un `read` ordinaire.
pub fn writeback(node: usize) {
    let pages: alloc::vec::Vec<(u64, u64)> = page_cache()
        .iter()
        .filter(|(n, _, _)| *n == node)
        .map(|(_, page, frame)| (*page, *frame))
        .collect();
    if pages.is_empty() {
        return;
    }
    let fs = crate::fs::ramfs::fs();
    for (page_index, frame) in pages {
        let start = (page_index * PAGE_SIZE) as usize;
        if start >= fs.nodes[node].content.len() {
            continue;
        }
        let end = core::cmp::min(fs.nodes[node].content.len(), start + PAGE_SIZE as usize);
        unsafe {
            core::ptr::copy_nonoverlapping(
                crate::kernel::memory::phys_to_virt(frame),
                fs.nodes[node].content[start..end].as_mut_ptr(),
                end - start,
            );
        }
    }
}

/// Mappe le framebuffer materiel dans l'espace utilisateur.
fn map_framebuffer(process: &mut task::Process, base: u64, length: u64, offset: u64) -> Option<u64> {
    let phys = crate::drivers::gfx::lfb_phys()?;
    let flags = vmm::PTE_PRESENT | vmm::PTE_USER | vmm::PTE_WRITE | vmm::PTE_NO_EXEC | vmm::PTE_WRITE_THROUGH;
    let mut done = 0u64;
    while done < length {
        if !process.space.map(base + done, phys + offset + done, flags) {
            return None;
        }
        done += PAGE_SIZE;
    }
    Some(base)
}

/// `munmap`.
pub fn sys_munmap(addr: u64, length: u64) -> i64 {
    if length == 0 || addr & (PAGE_SIZE - 1) != 0 {
        return -errno::EINVAL;
    }
    let process = task::current_process();
    let mut process = process.borrow_mut();
    process.space.unmap(addr, length);
    0
}

/// `mprotect`.
pub fn sys_mprotect(addr: u64, length: u64, prot: u32) -> i64 {
    if length == 0 {
        return 0;
    }
    let process = task::current_process();
    let mut process = process.borrow_mut();
    if !vmm::is_user_addr(addr) {
        return -errno::ENOMEM;
    }
    process.space.protect(addr, length, prot_to_flags(prot));
    0
}

/// `mremap` : agrandissement par nouvelle allocation + recopie.
pub fn sys_mremap(old_addr: u64, old_size: u64, new_size: u64, _flags: u32) -> i64 {
    if new_size <= old_size {
        // Retrecissement : on libere la queue, l'adresse ne bouge pas.
        if old_size > new_size {
            let process = task::current_process();
            let mut process = process.borrow_mut();
            let start = (old_addr + new_size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
            let end = (old_addr + old_size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
            if end > start {
                process.space.unmap(start, end - start);
            }
        }
        return old_addr as i64;
    }

    let new_addr = sys_mmap(0, new_size, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if new_addr < 0 {
        return new_addr;
    }
    let process = task::current_process();
    let mut process = process.borrow_mut();
    let mut buffer = alloc::vec![0u8; old_size as usize];
    if process.space.read(old_addr, &mut buffer) {
        process.space.write(new_addr as u64, &buffer);
    }
    process.space.unmap(old_addr, old_size);
    new_addr
}

/// `msync` : repercute les ecritures d'un `MAP_SHARED` vers le fichier.
pub fn sys_msync(addr: u64, _length: u64) -> i64 {
    // On ne sait pas a quel fichier appartient l'adresse ; recopier tout le
    // cache est correct et sans risque : les frames sont la source de verite,
    // le contenu RAMFS n'en est que le reflet.
    let nodes: alloc::vec::Vec<usize> = {
        let mut list: alloc::vec::Vec<usize> = page_cache().iter().map(|(node, _, _)| *node).collect();
        list.sort_unstable();
        list.dedup();
        list
    };
    let _ = addr;
    for node in nodes {
        writeback(node);
    }
    0
}
