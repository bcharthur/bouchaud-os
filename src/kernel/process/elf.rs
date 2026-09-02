//! Chargeur ELF64 (System V, x86-64) et construction de la pile initiale.
//!
//! Charge un executable Linux x86-64 dans un [`AddressSpace`] : segments
//! `PT_LOAD` mappes avec leurs droits reels, `.bss` mis a zero, et — si le
//! binaire est dynamique — l'editeur de liens `PT_INTERP` charge a son tour a
//! une base separee. Le noyau ne resout aucun symbole : c'est `ld.so` qui le
//! fait, en ring 3, a partir des informations du vecteur auxiliaire.
//!
//! ## Le vecteur auxiliaire n'est pas optionnel
//!
//! Une libc statique musl commence par relire ses propres `Phdr` via
//! `AT_PHDR`/`AT_PHNUM` pour initialiser le TLS ; `ld.so` a besoin en plus de
//! `AT_BASE` (sa propre base de chargement) et `AT_ENTRY` (le point d'entree du
//! programme). Un `AT_RANDOM` absent fait planter le canari de pile de toute
//! libc moderne. Ces valeurs sont donc calculees ici et empilees par
//! [`build_stack`].
//!
//! ## Ou sont charges les binaires
//!
//! Le noyau se reserve les creneaux PML4 bas ; l'espace utilisateur vit dans un
//! creneau dedie (cf. `kernel::vmm`). Un binaire PIE (`ET_DYN`), le cas normal
//! avec `musl-gcc -static-pie`, est charge a `USER_LOAD_BASE`. Un `ET_EXEC` est
//! accepte seulement s'il a ete lie a l'interieur de ce creneau
//! (`-Wl,-Ttext-segment=0x400000000000`), sinon ses adresses fixes tomberaient
//! sur des tables de pages appartenant au noyau.

use alloc::string::String;
use alloc::vec::Vec;

use crate::kernel::vmm::{self, AddressSpace, PAGE_SIZE};

// --- Constantes du format ----------------------------------------------------

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EM_X86_64: u16 = 62;

const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_PHDR: u32 = 6;
const PT_TLS: u32 = 7;
const PT_GNU_STACK: u32 = 0x6474_E551;

const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

// --- Vecteur auxiliaire ------------------------------------------------------

pub const AT_NULL: u64 = 0;
pub const AT_IGNORE: u64 = 1;
pub const AT_PHDR: u64 = 3;
pub const AT_PHENT: u64 = 4;
pub const AT_PHNUM: u64 = 5;
pub const AT_PAGESZ: u64 = 6;
pub const AT_BASE: u64 = 7;
pub const AT_FLAGS: u64 = 8;
pub const AT_ENTRY: u64 = 9;
pub const AT_UID: u64 = 11;
pub const AT_EUID: u64 = 12;
pub const AT_GID: u64 = 13;
pub const AT_EGID: u64 = 14;
pub const AT_PLATFORM: u64 = 15;
pub const AT_HWCAP: u64 = 16;
pub const AT_CLKTCK: u64 = 17;
pub const AT_SECURE: u64 = 23;
pub const AT_RANDOM: u64 = 25;
pub const AT_HWCAP2: u64 = 26;
pub const AT_EXECFN: u64 = 31;
pub const AT_SYSINFO_EHDR: u64 = 33;

/// En-tete de programme lu depuis le fichier.
#[derive(Clone, Copy, Debug)]
pub struct ProgramHeader {
    pub kind: u32,
    pub flags: u32,
    pub offset: u64,
    pub vaddr: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub align: u64,
}

/// En-tete ELF utile au chargement.
#[derive(Clone, Debug)]
pub struct ElfHeader {
    pub kind: u16,
    pub entry: u64,
    pub phoff: u64,
    pub phentsize: u16,
    pub phnum: u16,
    pub headers: Vec<ProgramHeader>,
}

/// Resultat du chargement d'une image ELF.
#[derive(Clone, Debug)]
pub struct LoadedImage {
    /// Decalage applique aux adresses du fichier (0 pour un `ET_EXEC`).
    pub base: u64,
    /// Point d'entree effectif (`e_entry + base`).
    pub entry: u64,
    /// Adresse des `Phdr` telle que vue par le programme (`AT_PHDR`).
    pub phdr_addr: u64,
    pub phentsize: u64,
    pub phnum: u64,
    /// Premiere adresse libre apres l'image (depart du `brk`).
    pub end: u64,
    /// Chemin de l'interpreteur (`PT_INTERP`), si le binaire est dynamique.
    pub interp: Option<String>,
    /// Segment TLS (`PT_TLS`) : (adresse, taille fichier, taille memoire, alignement).
    pub tls: Option<(u64, u64, u64, u64)>,
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&data[offset..offset + 8]);
    u64::from_le_bytes(bytes)
}

/// Analyse un en-tete ELF64 x86-64 et ses en-tetes de programme.
pub fn parse(data: &[u8]) -> Result<ElfHeader, &'static str> {
    if data.len() < 64 {
        return Err("fichier trop court pour un ELF64");
    }
    if data[0..4] != ELF_MAGIC {
        return Err("signature ELF absente");
    }
    if data[4] != ELFCLASS64 {
        return Err("ELF 32 bits non supporte (ELFCLASS64 attendu)");
    }
    if data[5] != ELFDATA2LSB {
        return Err("ELF gros-boutiste non supporte");
    }
    let kind = read_u16(data, 16);
    let machine = read_u16(data, 18);
    if machine != EM_X86_64 {
        return Err("architecture non supportee (x86-64 attendu)");
    }
    if kind != ET_EXEC && kind != ET_DYN {
        return Err("type ELF non executable (ET_EXEC ou ET_DYN attendu)");
    }

    let entry = read_u64(data, 24);
    let phoff = read_u64(data, 32);
    let phentsize = read_u16(data, 54);
    let phnum = read_u16(data, 56);

    if phentsize < 56 {
        return Err("taille d'en-tete de programme invalide");
    }
    let mut headers = Vec::new();
    for index in 0..phnum as usize {
        let base = phoff as usize + index * phentsize as usize;
        if base + 56 > data.len() {
            return Err("en-tetes de programme hors du fichier");
        }
        headers.push(ProgramHeader {
            kind: read_u32(data, base),
            flags: read_u32(data, base + 4),
            offset: read_u64(data, base + 8),
            vaddr: read_u64(data, base + 16),
            filesz: read_u64(data, base + 32),
            memsz: read_u64(data, base + 40),
            align: read_u64(data, base + 48),
        });
    }

    Ok(ElfHeader {
        kind,
        entry,
        phoff,
        phentsize,
        phnum,
        headers,
    })
}

/// Analyse un ELF sans charger son fichier complet.
pub fn parse_node(node: usize) -> Result<ElfHeader, &'static str> {
    let mut ehdr = [0u8; 64];
    if crate::fs::backing::read_at(node, 0, &mut ehdr) != ehdr.len() {
        return Err("fichier trop court pour un ELF64");
    }
    if ehdr[0..4] != ELF_MAGIC {
        return Err("signature ELF absente");
    }

    let phoff = read_u64(&ehdr, 32) as usize;
    let phentsize = read_u16(&ehdr, 54) as usize;
    let phnum = read_u16(&ehdr, 56) as usize;
    if phentsize < 56 {
        return Err("taille d'en-tete de programme invalide");
    }

    let need = phoff
        .checked_add(phentsize.checked_mul(phnum).ok_or("Phdr overflow")?)
        .ok_or("Phdr overflow")?;
    if need > crate::fs::backing::logical_len(node) {
        return Err("en-tetes de programme hors du fichier");
    }

    let mut metadata = alloc::vec![0u8; need.max(64)];
    if crate::fs::backing::read_at(node, 0, &mut metadata) != metadata.len() {
        return Err("lecture metadata ELF incomplete");
    }
    parse(&metadata)
}

/// Transforme les PT_LOAD en promesses file-backed. Aucune frame n'est allouee.
pub fn load_node_lazy(
    node: usize,
    base_hint: u64,
    promises: &mut Vec<crate::kernel::task::Promesse>,
) -> Result<LoadedImage, &'static str> {
    let header = parse_node(node)?;
    let base = if header.kind == ET_DYN { base_hint } else { 0 };
    let total = crate::fs::backing::logical_len(node) as u64;

    for ph in header.headers.iter().filter(|p| p.kind == PT_LOAD) {
        let start = base + ph.vaddr;
        let end = start.checked_add(ph.memsz).ok_or("segment overflow")?;
        if !vmm::is_user_addr(start) || !vmm::is_user_addr(end) {
            return Err("segment hors du creneau utilisateur (relier en PIE ou avec -Ttext-segment=0x400000000000)");
        }
        if ph.filesz > ph.memsz || ph.offset.saturating_add(ph.filesz) > total {
            return Err("segment PT_LOAD hors du fichier");
        }
    }

    let mut interp = None;
    let mut tls = None;
    let mut image_end = 0u64;
    let mut phdr_addr = 0u64;

    for ph in header.headers.iter() {
        match ph.kind {
            PT_INTERP => {
                let mut raw = alloc::vec![0u8; ph.filesz as usize];
                if crate::fs::backing::read_at(node, ph.offset as usize, &mut raw) != raw.len() {
                    return Err("PT_INTERP hors du fichier");
                }
                let text = raw.split(|&b| b == 0).next().unwrap_or(&raw);
                interp = Some(String::from_utf8_lossy(text).into_owned());
            }
            PT_TLS => {
                tls = Some((base + ph.vaddr, ph.filesz, ph.memsz, ph.align.max(1)));
            }
            PT_PHDR => {
                phdr_addr = base + ph.vaddr;
            }
            PT_LOAD => {
                if ph.memsz == 0 {
                    continue;
                }
                let vaddr = base + ph.vaddr;
                let page_start = vaddr & !(PAGE_SIZE - 1);
                let page_end = (vaddr + ph.memsz + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
                crate::kernel::vma::overlay(promises, crate::kernel::task::Promesse {
                    id: crate::kernel::vma::nouvelle_identite(),
                    debut: page_start,
                    fin: page_end,
                    drapeaux: page_flags(ph.flags),
                    backing: crate::kernel::task::PromesseBacking::File {
                        node,
                        mapping_start: vaddr,
                        file_offset: ph.offset,
                        file_size: ph.filesz,
                    },
                });
                image_end = image_end.max(page_end);
            }
            PT_DYNAMIC | PT_GNU_STACK => {}
            _ => {}
        }
    }

    if image_end == 0 {
        return Err("aucun segment PT_LOAD chargeable");
    }
    if phdr_addr == 0 {
        phdr_addr = base + header.phoff;
    }

    Ok(LoadedImage {
        base,
        entry: base + header.entry,
        phdr_addr,
        phentsize: header.phentsize as u64,
        phnum: header.phnum as u64,
        end: image_end,
        interp,
        tls,
    })
}

/// Traduit les droits `PF_*` d'un segment en drapeaux de table de pages.
fn page_flags(flags: u32) -> u64 {
    let mut value = vmm::PTE_PRESENT | vmm::PTE_USER;
    if flags & PF_W != 0 {
        value |= vmm::PTE_WRITE;
    }
    if flags & PF_X == 0 {
        value |= vmm::PTE_NO_EXEC;
    }
    value
}

/// Charge une image ELF dans `space`.
///
/// `base_hint` sert de base de chargement pour un binaire PIE ; il est ignore
/// pour un `ET_EXEC`, qui impose ses adresses.
pub fn load(
    space: &mut AddressSpace,
    data: &[u8],
    base_hint: u64,
) -> Result<LoadedImage, &'static str> {
    let header = parse(data)?;
    let base = if header.kind == ET_DYN { base_hint } else { 0 };

    // Verifie que l'image tient dans le creneau utilisateur : un ET_EXEC lie
    // pour Linux (0x400000) tomberait sinon dans les tables du noyau.
    for ph in header.headers.iter().filter(|p| p.kind == PT_LOAD) {
        let start = base + ph.vaddr;
        let end = start + ph.memsz;
        if !vmm::is_user_addr(start) || !vmm::is_user_addr(end) {
            return Err("segment hors du creneau utilisateur (relier en PIE ou avec -Ttext-segment=0x400000000000)");
        }
    }

    let mut interp = None;
    let mut tls = None;
    let mut image_end = 0u64;
    let mut phdr_addr = 0u64;
    let mut first_vaddr = u64::MAX;

    for ph in header.headers.iter() {
        match ph.kind {
            PT_INTERP => {
                let start = ph.offset as usize;
                let end = start + ph.filesz as usize;
                if end > data.len() {
                    return Err("PT_INTERP hors du fichier");
                }
                let raw = &data[start..end];
                let text = raw.split(|&b| b == 0).next().unwrap_or(raw);
                interp = Some(String::from_utf8_lossy(text).into_owned());
            }
            PT_TLS => {
                tls = Some((base + ph.vaddr, ph.filesz, ph.memsz, ph.align.max(1)));
            }
            PT_PHDR => {
                phdr_addr = base + ph.vaddr;
            }
            PT_LOAD => {
                if ph.memsz == 0 {
                    continue;
                }
                first_vaddr = first_vaddr.min(ph.vaddr);
                let vaddr = base + ph.vaddr;
                let page_start = vaddr & !(PAGE_SIZE - 1);
                let page_end = (vaddr + ph.memsz + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

                // On mappe d'abord en ecriture pour pouvoir recopier le contenu,
                // les droits definitifs sont appliques ensuite.
                let writable = vmm::PTE_PRESENT | vmm::PTE_USER | vmm::PTE_WRITE;
                if !space.map_alloc_accounted(
                    page_start,
                    page_end - page_start,
                    writable,
                    vmm::ResidentKind::FilePrivate,
                ) {
                    return Err("memoire physique insuffisante pour le segment");
                }

                let file_start = ph.offset as usize;
                let file_end = file_start + ph.filesz as usize;
                if file_end > data.len() {
                    return Err("segment PT_LOAD hors du fichier");
                }
                if !space.write(vaddr, &data[file_start..file_end]) {
                    return Err("ecriture du segment impossible");
                }
                // `.bss` : la difference memsz - filesz doit etre a zero. Les
                // frames fraiches le sont deja, mais une page partiellement
                // recouverte par un autre segment ne l'est pas forcement.
                let bss_start = vaddr + ph.filesz;
                let bss_len = ph.memsz - ph.filesz;
                if bss_len > 0 {
                    let zeros = alloc::vec![0u8; core::cmp::min(bss_len as usize, 1 << 20)];
                    let mut written = 0u64;
                    while written < bss_len {
                        let chunk = core::cmp::min(zeros.len() as u64, bss_len - written);
                        space.write(bss_start + written, &zeros[..chunk as usize]);
                        written += chunk;
                    }
                }

                if let Some(invalidation) = space.prepare_protect(
                    page_start,
                    page_end - page_start,
                    page_flags(ph.flags),
                ) {
                    // L'espace ELF est encore inactif: execute() ne cible
                    // aucun CPU et n'attend donc jamais sous le borrow appelant.
                    invalidation.execute();
                }
                image_end = image_end.max(page_end);
            }
            PT_DYNAMIC | PT_GNU_STACK => {}
            _ => {}
        }
    }

    if image_end == 0 {
        return Err("aucun segment PT_LOAD chargeable");
    }

    // Sans PT_PHDR, on retrouve les Phdr dans le premier segment charge : ils
    // sont a `phoff` dans le fichier, donc a base + phoff si le segment couvre
    // le debut du fichier (cas normal).
    if phdr_addr == 0 {
        phdr_addr = base + header.phoff;
    }

    Ok(LoadedImage {
        base,
        entry: base + header.entry,
        phdr_addr,
        phentsize: header.phentsize as u64,
        phnum: header.phnum as u64,
        end: image_end,
        interp,
        tls,
    })
}

/// Description de la pile initiale a construire.
pub struct StackLayout<'a> {
    pub argv: &'a [String],
    pub envp: &'a [String],
    pub image: &'a LoadedImage,
    /// Base de chargement de `ld.so` (`AT_BASE`), 0 si binaire statique.
    pub interp_base: u64,
    pub uid: u32,
    pub gid: u32,
}

/// Construit la pile initiale du processus et renvoie le RSP de depart.
///
/// Disposition exigee par l'ABI System V a l'entree de `_start` :
///
/// ```text
///   [rsp]        argc
///   [rsp+8]      argv[0..argc]
///   ...          NULL
///   ...          envp[0..], NULL
///   ...          auxv (paires cle/valeur), AT_NULL
///   ...          chaines pointees ci-dessus
/// ```
pub fn build_stack(space: &mut AddressSpace, layout: &StackLayout) -> Result<u64, &'static str> {
    let top = vmm::user_stack_top();
    let size = vmm::USER_STACK_SIZE;
    let flags = vmm::PTE_PRESENT | vmm::PTE_USER | vmm::PTE_WRITE | vmm::PTE_NO_EXEC;
    if !space.map_alloc_accounted(
        top - size,
        size,
        flags,
        vmm::ResidentKind::Anonymous,
    ) {
        return Err("memoire physique insuffisante pour la pile utilisateur");
    }

    // 1. Les chaines, en haut de la pile.
    let mut cursor = top - 16;
    let push_bytes = |space: &mut AddressSpace, data: &[u8], cursor: &mut u64| -> u64 {
        *cursor -= data.len() as u64;
        *cursor &= !0x7;
        space.write(*cursor, data);
        *cursor
    };

    let mut argv_ptrs = Vec::new();
    for arg in layout.argv.iter() {
        let mut bytes = arg.as_bytes().to_vec();
        bytes.push(0);
        argv_ptrs.push(push_bytes(space, &bytes, &mut cursor));
    }
    let mut envp_ptrs = Vec::new();
    for env in layout.envp.iter() {
        let mut bytes = env.as_bytes().to_vec();
        bytes.push(0);
        envp_ptrs.push(push_bytes(space, &bytes, &mut cursor));
    }

    // 16 octets aleatoires pour AT_RANDOM : la libc y puise son canari de pile
    // et la graine de son allocateur.
    let mut random = [0u8; 16];
    crate::net::security::tls::rng::fill(&mut random);
    let random_addr = push_bytes(space, &random, &mut cursor);
    let platform_addr = push_bytes(space, b"x86_64\0", &mut cursor);
    let execfn_addr = if let Some(first) = layout.argv.first() {
        let mut bytes = first.as_bytes().to_vec();
        bytes.push(0);
        push_bytes(space, &bytes, &mut cursor)
    } else {
        platform_addr
    };

    // 2. Le vecteur auxiliaire.
    let mut auxv: Vec<(u64, u64)> = Vec::new();
    auxv.push((AT_PHDR, layout.image.phdr_addr));
    auxv.push((AT_PHENT, layout.image.phentsize));
    auxv.push((AT_PHNUM, layout.image.phnum));
    auxv.push((AT_PAGESZ, PAGE_SIZE));
    auxv.push((AT_BASE, layout.interp_base));
    auxv.push((AT_FLAGS, 0));
    auxv.push((AT_ENTRY, layout.image.entry));
    auxv.push((AT_UID, layout.uid as u64));
    auxv.push((AT_EUID, layout.uid as u64));
    auxv.push((AT_GID, layout.gid as u64));
    auxv.push((AT_EGID, layout.gid as u64));
    auxv.push((AT_SECURE, 0));
    auxv.push((AT_CLKTCK, crate::kernel::timer::TICKS_PER_SECOND));
    // CPUID leaf 1 EDX : les bits FPU/TSC/CMOV/MMX/FXSR/SSE/SSE2 que toute
    // libc x86-64 suppose presents.
    auxv.push((AT_HWCAP, 0x0783_FBFF));
    auxv.push((AT_HWCAP2, 0));
    auxv.push((AT_RANDOM, random_addr));
    auxv.push((AT_PLATFORM, platform_addr));
    auxv.push((AT_EXECFN, execfn_addr));
    // Pas de vDSO : la libc doit passer par l'instruction `syscall`.
    auxv.push((AT_SYSINFO_EHDR, 0));
    auxv.push((AT_NULL, 0));

    // 3. Le bloc argc/argv/envp/auxv, aligne sur 16 octets a l'entree de _start.
    let words = 1                       // argc
        + argv_ptrs.len() + 1           // argv + NULL
        + envp_ptrs.len() + 1           // envp + NULL
        + auxv.len() * 2; // auxv
    let mut rsp = cursor - (words as u64) * 8;
    rsp &= !0xF;

    let mut offset = rsp;
    let push_word = |space: &mut AddressSpace, value: u64, offset: &mut u64| {
        space.write(*offset, &value.to_le_bytes());
        *offset += 8;
    };

    push_word(space, argv_ptrs.len() as u64, &mut offset);
    for &ptr in argv_ptrs.iter() {
        push_word(space, ptr, &mut offset);
    }
    push_word(space, 0, &mut offset);
    for &ptr in envp_ptrs.iter() {
        push_word(space, ptr, &mut offset);
    }
    push_word(space, 0, &mut offset);
    for &(key, value) in auxv.iter() {
        push_word(space, key, &mut offset);
        push_word(space, value, &mut offset);
    }

    Ok(rsp)
}

/// Resume lisible d'un binaire ELF (commande `elfinfo`).
pub fn describe(data: &[u8]) {
    match parse(data) {
        Err(message) => crate::println!("elfinfo: {}", message),
        Ok(header) => {
            crate::println!(
                "ELF64 x86-64, type {}",
                match header.kind {
                    ET_EXEC => "ET_EXEC (adresses fixes)",
                    ET_DYN => "ET_DYN (PIE / bibliotheque)",
                    _ => "?",
                }
            );
            crate::println!("  point d'entree : {:#x}", header.entry);
            crate::println!(
                "  {} en-tetes de programme ({} octets chacun)",
                header.phnum,
                header.phentsize
            );
            for ph in header.headers.iter() {
                let kind = match ph.kind {
                    PT_LOAD => "LOAD",
                    PT_DYNAMIC => "DYNAMIC",
                    PT_INTERP => "INTERP",
                    PT_PHDR => "PHDR",
                    PT_TLS => "TLS",
                    PT_GNU_STACK => "GNU_STACK",
                    _ => "autre",
                };
                let r = if ph.flags & PF_R != 0 { 'r' } else { '-' };
                let w = if ph.flags & PF_W != 0 { 'w' } else { '-' };
                let x = if ph.flags & PF_X != 0 { 'x' } else { '-' };
                crate::println!(
                    "  {:<9} {}{}{} vaddr={:#x} filesz={} memsz={}",
                    kind,
                    r,
                    w,
                    x,
                    ph.vaddr,
                    ph.filesz,
                    ph.memsz
                );
            }
            let interp = header.headers.iter().find(|p| p.kind == PT_INTERP);
            match interp {
                Some(ph) => {
                    let start = ph.offset as usize;
                    let end = (start + ph.filesz as usize).min(data.len());
                    let raw = &data[start..end];
                    let text = raw.split(|&b| b == 0).next().unwrap_or(raw);
                    crate::println!("  interpreteur : {}", String::from_utf8_lossy(text));
                    crate::println!("  -> binaire dynamique : necessite ld.so dans le RAMFS");
                }
                None => crate::println!("  binaire statique (aucun interpreteur requis)"),
            }
        }
    }
}
