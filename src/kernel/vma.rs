//! Virtual Memory Areas (VMA) de Bouchaud OS.
//!
//! Cette couche remplace le vieux modele "promesses" append-only par une vraie
//! carte d'intervalles. Une VMA existe meme si aucune page physique n'existe.
//! `mmap(PROT_NONE)`, `MAP_NORESERVE`, ELF file-backed, `mprotect`, `munmap` et
//! `madvise(DONTNEED)` manipulent donc le meme objet.
//!
//! Invariant principal : une operation sur un sous-intervalle ne doit jamais
//! jeter la metadata des portions gauche/droite encore vivantes.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::vmm::PAGE_SIZE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backing {
    /// Pages anonymes, zero au premier acces.
    Zero,
    /// Mapping prive recharge depuis un fichier.
    File {
        node: usize,
        mapping_start: u64,
        file_offset: u64,
        file_size: u64,
    },
    /// Mapping partage recharge depuis le cache global de pages.
    SharedFile {
        node: usize,
        mapping_start: u64,
        file_offset: u64,
        file_size: u64,
    },
    /// Memoire d'un peripherique mappee directement (framebuffer physique).
    Framebuffer {
        phys_base: u64,
        mapping_start: u64,
        phys_offset: u64,
    },
}

impl Backing {
    pub fn label(self) -> &'static str {
        match self {
            Backing::Zero => "zero",
            Backing::File { .. } => "file-private",
            Backing::SharedFile { .. } => "file-shared",
            Backing::Framebuffer { .. } => "framebuffer",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vma {
    /// Identite monotone du mapping logique. Elle ferme l'ABA adresse/backing
    /// lors d'un fault concurrent avec munmap/MAP_FIXED.
    pub id: u64,
    pub debut: u64,
    pub fin: u64,
    /// Drapeaux feuille x86 a appliquer quand une page devient residente.
    ///
    /// Une VMA `PROT_NONE` existe quand meme ; elle porte simplement des
    /// drapeaux sans `PTE_USER`, donc le fault handler refuse de la peupler.
    pub drapeaux: u64,
    pub backing: Backing,
}

static NEXT_VMA_ID: AtomicU64 = AtomicU64::new(1);

pub fn nouvelle_identite() -> u64 {
    NEXT_VMA_ID.fetch_add(1, Ordering::Relaxed)
}

impl Vma {
    pub fn contient(self, addr: u64) -> bool {
        addr >= self.debut && addr < self.fin
    }

    pub fn chevauche(self, debut: u64, fin: u64) -> bool {
        self.debut < fin && debut < self.fin
    }
}

fn align_up(value: u64) -> Option<u64> {
    value
        .checked_add(PAGE_SIZE - 1)
        .map(|value| value & !(PAGE_SIZE - 1))
}

/// Derniere VMA couvrant l'adresse.
///
/// Les PT_LOAD ELF peuvent partager une page de bordure. Leur ordre d'insertion
/// est significatif pour les droits : le segment le plus recent fait foi.
pub fn trouve(regions: &[Vma], addr: u64) -> Option<Vma> {
    regions.iter().rev().find(|region| region.contient(addr)).copied()
}

pub fn chevauche(regions: &[Vma], debut: u64, fin: u64) -> bool {
    regions.iter().any(|region| region.chevauche(debut, fin))
}

/// Retire `[debut, fin)` en preservant les fragments non couverts.
///
/// C'est la correction structurelle du crash Ladybird : retirer quelques pages
/// d'une arene de 1 Gio ne doit pas supprimer la promesse des centaines de Mio
/// restantes.
pub fn retire(regions: &mut Vec<Vma>, debut: u64, fin: u64) {
    if debut >= fin {
        return;
    }

    let mut next = Vec::with_capacity(regions.len() + 2);
    for region in regions.iter().copied() {
        if !region.chevauche(debut, fin) {
            next.push(region);
            continue;
        }

        if region.debut < debut {
            let mut gauche = region;
            gauche.fin = debut;
            if gauche.debut < gauche.fin {
                next.push(gauche);
            }
        }

        if region.fin > fin {
            let mut droite = region;
            droite.debut = fin;
            if droite.debut < droite.fin {
                next.push(droite);
            }
        }
    }
    *regions = next;
}

/// Remplace un intervalle comme `MAP_FIXED` puis ajoute la nouvelle VMA.
pub fn remplace(regions: &mut Vec<Vma>, region: Vma) {
    retire(regions, region.debut, region.fin);
    regions.push(region);
}

/// Ajoute un overlay sans supprimer les autres VMA.
///
/// Reserve au chargeur ELF : deux `PT_LOAD` peuvent partager une page et leurs
/// octets doivent etre composes au premier fault.
pub fn overlay(regions: &mut Vec<Vma>, region: Vma) {
    regions.push(region);
}

/// L'intervalle est-il entierement couvert par au moins une union de VMA ?
pub fn couvre(regions: &[Vma], debut: u64, fin: u64) -> bool {
    if debut >= fin {
        return true;
    }

    let mut spans: Vec<(u64, u64)> = regions
        .iter()
        .filter(|region| region.chevauche(debut, fin))
        .map(|region| (region.debut.max(debut), region.fin.min(fin)))
        .collect();

    if spans.is_empty() {
        return false;
    }

    spans.sort_by_key(|span| span.0);
    let mut cursor = debut;
    for (start, end) in spans {
        if start > cursor {
            return false;
        }
        cursor = cursor.max(end);
        if cursor >= fin {
            return true;
        }
    }
    false
}

/// Change les droits d'un sous-intervalle sans perdre le backing.
///
/// Renvoie `false` si une partie de la plage n'etait pas mappee/reservee.
pub fn protege(regions: &mut Vec<Vma>, debut: u64, fin: u64, drapeaux: u64) -> bool {
    if !couvre(regions, debut, fin) {
        return false;
    }

    let mut next = Vec::with_capacity(regions.len() + 4);
    for region in regions.iter().copied() {
        if !region.chevauche(debut, fin) {
            next.push(region);
            continue;
        }

        if region.debut < debut {
            let mut gauche = region;
            gauche.fin = debut;
            if gauche.debut < gauche.fin {
                next.push(gauche);
            }
        }

        let mut milieu = region;
        milieu.debut = region.debut.max(debut);
        milieu.fin = region.fin.min(fin);
        let protection_change = region.drapeaux != drapeaux;
        milieu.drapeaux = drapeaux;
        if protection_change {
            milieu.id = nouvelle_identite();
        }
        if milieu.debut < milieu.fin {
            next.push(milieu);
        }

        if region.fin > fin {
            let mut droite = region;
            droite.debut = fin;
            if droite.debut < droite.fin {
                next.push(droite);
            }
        }
    }

    *regions = next;
    true
}

/// Somme de l'union des VMA, sans double-compter les overlays ELF.
pub fn octets_virtuels(regions: &[Vma]) -> u64 {
    if regions.is_empty() {
        return 0;
    }

    let mut spans: Vec<(u64, u64)> = regions.iter().map(|r| (r.debut, r.fin)).collect();
    spans.sort_by_key(|span| span.0);

    let mut total = 0u64;
    let mut start = spans[0].0;
    let mut end = spans[0].1;

    for (next_start, next_end) in spans.into_iter().skip(1) {
        if next_start <= end {
            end = end.max(next_end);
        } else {
            total = total.saturating_add(end.saturating_sub(start));
            start = next_start;
            end = next_end;
        }
    }

    total.saturating_add(end.saturating_sub(start))
}

/// Trouve un trou d'au moins `longueur` a partir de `depart`.
pub fn trouve_trou(regions: &[Vma], depart: u64, longueur: u64, limite: u64) -> Option<u64> {
    let mut candidat = align_up(depart)?;

    loop {
        let fin = candidat.checked_add(longueur)?;
        if fin > limite {
            return None;
        }

        let mut collision_fin = None;
        for region in regions {
            if region.chevauche(candidat, fin) {
                collision_fin = Some(
                    collision_fin
                        .map(|value: u64| value.max(region.fin))
                        .unwrap_or(region.fin),
                );
            }
        }

        match collision_fin {
            None => return Some(candidat),
            Some(end) => {
                candidat = align_up(end.checked_add(PAGE_SIZE)?)?;
            }
        }
    }
}

/// VMA precedente et suivante pour les diagnostics de fault.
pub fn voisines(regions: &[Vma], addr: u64) -> (Option<Vma>, Option<Vma>) {
    let mut avant: Option<Vma> = None;
    let mut apres: Option<Vma> = None;

    for region in regions.iter().copied() {
        if region.fin <= addr {
            match avant {
                Some(current) if current.fin >= region.fin => {}
                _ => avant = Some(region),
            }
        } else if region.debut > addr {
            match apres {
                Some(current) if current.debut <= region.debut => {}
                _ => apres = Some(region),
            }
        }
    }

    (avant, apres)
}

/// Petit autotest sans materiel, execute par `meminfo` et la CI health.
pub fn self_test() -> bool {
    use crate::kernel::vmm::{PTE_NO_EXEC, PTE_PRESENT, PTE_USER, PTE_WRITE};

    let rw = PTE_PRESENT | PTE_USER | PTE_WRITE | PTE_NO_EXEC;
    let ro = PTE_PRESENT | PTE_USER | PTE_NO_EXEC;

    let mut regions = Vec::new();
    remplace(
        &mut regions,
        Vma {
            id: nouvelle_identite(),
            debut: 0x1000,
            fin: 0x9000,
            drapeaux: rw,
            backing: Backing::Zero,
        },
    );
    let original_id = trouve(&regions, 0x2000).unwrap().id;

    retire(&mut regions, 0x3000, 0x5000);
    if !couvre(&regions, 0x1000, 0x3000)
        || couvre(&regions, 0x3000, 0x5000)
        || !couvre(&regions, 0x5000, 0x9000)
    {
        return false;
    }
    if trouve(&regions, 0x2000).unwrap().id != original_id
        || trouve(&regions, 0x8000).unwrap().id != original_id
    {
        return false;
    }

    if !protege(&mut regions, 0x5000, 0x7000, ro) {
        return false;
    }
    if trouve(&regions, 0x6000).map(|r| r.drapeaux) != Some(ro) {
        return false;
    }
    if trouve(&regions, 0x6000).unwrap().id == original_id {
        return false;
    }
    if trouve(&regions, 0x8000).map(|r| r.drapeaux) != Some(rw) {
        return false;
    }

    remplace(
        &mut regions,
        Vma {
            id: nouvelle_identite(),
            debut: 0x3000,
            fin: 0x5000,
            drapeaux: rw,
            backing: Backing::Zero,
        },
    );
    if trouve(&regions, 0x4000).unwrap().id == original_id {
        return false;
    }
    if !couvre(&regions, 0x1000, 0x9000) {
        return false;
    }

    trouve_trou(&regions, 0x1000, 0x1000, 0x20_0000)
        .map(|addr| addr >= 0xA000)
        .unwrap_or(false)
}
