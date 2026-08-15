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

use crate::kernel::abi::errno;
use crate::kernel::fd::FdKind;
use crate::kernel::partage;
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
            // `brk` ne rend pas d'erreur : la libc compare le retour a sa
            // demande. Rendre le sommet inchange est donc la facon correcte de
            // dire non, et c'est ce que fait un depassement de `RLIMIT_AS`.
            if !process.tient_sous_limite(end - start) {
                return current as i64;
            }
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
                // Ecran virtuel : `/dev/fb0` designe une surface partagee, et
                // la projeter est exactement ce que fait un `MAP_SHARED` sur un
                // `memfd`. Le client obtient les frames que le compositeur lit,
                // sans avoir a savoir qu'il ne touche pas le materiel.
                if let Some(ecran) = process.ecran {
                    return if map_shared_file(&mut process, ecran.node, base, length, offset) {
                        process.space.protect(base, length, prot_to_flags(prot));
                        base as i64
                    } else {
                        -errno::ENOMEM
                    };
                }
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

    // --- Reservation d'espace d'adressage -----------------------------------
    //
    // Un `mmap` en `PROT_NONE` ne demande pas de la memoire : il demande une
    // **plage d'adresses** que personne d'autre n'utilisera. Aucune page n'y
    // sera jamais lue ni ecrite tant qu'elle n'aura pas ete confirmee par un
    // second `mmap` en `MAP_FIXED` avec de vrais droits. Allouer des frames ici
    // reviendrait a prendre au serieux une intention que l'appelant n'a pas.
    //
    // Ce n'est pas un cas theorique. `GC::PrimitiveStorage` reserve **4 TiB**
    // ainsi des la construction de la `JS::VM`, pour que tous les pointeurs
    // compresses de LibJS partagent les memes bits de poids fort. Avec une
    // allocation avide, cet appel demandait 4 TiB de memoire physique et
    // rendait `ENOMEM` — et LibJS s'arretait la, sur son tout premier appel
    // systeme d'importance.
    //
    // Le noyau n'a donc rien a faire que reserver l'intervalle : ne pas mapper
    // ces pages *est* le comportement correct. Un acces avant confirmation fait
    // une faute de page, comme sous Linux.
    //
    // `MAP_NORESERVE` exprime la meme intention pour une plage accessible : ne
    // pas engager la memoire d'avance. Bouchaud n'a pas de sur-engagement, donc
    // on ne peut l'honorer que lorsqu'il accompagne `PROT_NONE` — sinon on
    // alloue, ce qui est plus strict que demande, jamais moins.
    if prot == PROT_NONE {
        // Avec `MAP_FIXED`, la plage remplace ce qui s'y trouvait : c'est ainsi
        // que Linux definit l'appel, et c'est ainsi que `decommit_memory()` de
        // LibCore **rend** la memoire — un `mmap(PROT_NONE, MAP_FIXED)` sur une
        // plage vivante. Se contenter de rendre l'adresse ferait de la
        // liberation une operation sans effet : la memoire ne redescendrait
        // jamais, et le premier programme a exercer son ramasse-miettes
        // epuiserait la machine.
        //
        // C'est ce qui est arrive : le temoin LibJS tenait jusqu'a sa dixieme
        // verification, celle qui abandonne 200 000 objets, puis la machine
        // manquait de memoire.
        if flags & MAP_FIXED != 0 {
            process.space.unmap(base, length);
        }
        process.promesses.retain(|p| p.fin <= base || p.debut >= base + length);
        return base as i64;
    }

    // --- `MAP_NORESERVE` : promettre sans engager --------------------------
    //
    // La plage sera accessible, mais l'appelant annonce qu'il n'en touchera
    // qu'une part. Sous Linux la memoire n'est engagee qu'a l'acces ; c'est ce
    // que fait mimalloc — l'allocateur d'AK, donc de tout Ladybird — qui prend
    // ses arenes **par gibioctet**, en lecture-ecriture, et n'en emploie qu'une
    // fraction.
    //
    // Les peupler ici epuisait les 2 Gio de la machine en deux arenes, et le
    // temoin LibJS mourait d'une dereference nulle a sa dixieme verification —
    // l'allocateur ayant rendu ce qu'il rend quand il n'a plus rien. Les
    // refuser aurait ete tout aussi faux.
    //
    // On note donc la promesse, et le gestionnaire de faute de page peuple
    // chaque page a son premier acces, avec les droits enregistres ici.
    if flags & MAP_NORESERVE != 0 && flags & MAP_ANONYMOUS != 0 {
        let drapeaux = prot_to_flags(prot);
        process.promesses.push(task::Promesse {
            debut: base,
            fin: base + length,
            drapeaux,
        });
        return base as i64;
    }

    // Le plafond se verifie avant d'allouer quoi que ce soit : echouer a
    // mi-chemin laisserait des pages mappees pour une `mmap` qui rend une
    // erreur, et l'appelant n'aurait aucun moyen de les rendre.
    if !process.tient_sous_limite(length) {
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

/// Mappe un fichier en partage sur les frames du cache.
///
/// Le mappage est **declare** au cache (`partage::mappe`) et enregistre dans le
/// processus : c'est ce qui permettra a `munmap`, a `execve` et a la mort du
/// processus de rendre la reference. Sans cette declaration, les frames
/// resteraient allouees pour toujours — c'est precisement le defaut que le
/// module `partage` corrige.
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
        let frame = match partage::page(node, page_index) {
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
    partage::mappe(node);
    process.partages.push(task::Partage { base, length, node });
    true
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
///
/// Deux choses a defaire, pas une : les entrees de table de pages, et la
/// reference que la plage detenait sur le cache partage. Oublier la seconde
/// etait exactement la fuite corrigee ici.
pub fn sys_munmap(addr: u64, length: u64) -> i64 {
    if length == 0 || addr & (PAGE_SIZE - 1) != 0 {
        return -errno::EINVAL;
    }
    let process = task::current_process();
    let mut process = process.borrow_mut();
    let liberes = process.retire_partages(addr, length);
    process.space.unmap(addr, length);
    // Les references sont rendues **apres** avoir demappe : tant qu'une entree
    // de table pointe encore sur la frame, la liberer serait un usage apres
    // liberation en attente d'un ordonnancement malheureux.
    for node in liberes {
        partage::demappe(node);
    }
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

    // Donner des droits a une plage **reservee**, c'est la confirmer.
    //
    // C'est l'autre moitie de la reservation d'espace d'adressage (voir
    // `sys_mmap`), et l'oublier ne se voit pas tout de suite : les droits sont
    // bien poses, l'appel rend 0, et le programme faute a la premiere ecriture
    // sur une adresse qu'il croit legitimement posseder.
    //
    // C'est exactement ce qui arrivait avec mimalloc, l'allocateur d'AK : il
    // reserve ses arenes en `PROT_NONE`, puis les confirme par `mprotect` — pas
    // par un second `mmap`. Sans allocation ici, LibJS mourait d'une faute de
    // page a la dixieme verification du temoin, loin de sa cause.
    //
    // On n'alloue que pour les pages absentes : `map_alloc` laisse intactes
    // celles qui existent deja, donc un `mprotect` ordinaire sur une plage
    // vivante ne coute rien de plus.
    if prot != PROT_NONE {
        let debut = addr & !(PAGE_SIZE - 1);
        let fin = (addr + length + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let drapeaux = prot_to_flags(prot);
        // La promesse plutot que l'allocation : `mprotect` peut porter sur une
        // plage enorme (mimalloc confirme ses arenes ainsi), et tout peupler
        // ici reviendrait a engager la memoire que `MAP_NORESERVE` disait
        // justement ne pas vouloir engager. Les pages deja presentes gardent
        // leur frame et ne font que changer de droits, juste apres.
        process.promesses.push(task::Promesse { debut, fin, drapeaux });
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
///
/// Si l'adresse designe une plage partagee connue du processus, on ne recopie
/// que le nœud concerne ; sinon on recopie tout, ce qui reste correct — les
/// frames sont la source de verite, le contenu RAMFS n'en est que le reflet.
pub fn sys_msync(addr: u64, _length: u64) -> i64 {
    let node = {
        let process = task::current_process();
        let process = process.borrow();
        process.partage_a(addr)
    };
    match node {
        Some(node) => partage::writeback(node),
        None => partage::writeback_tout(),
    }
    0
}
