//! Metadonnees des dalles du tas noyau.
//!
//! C9.1 n'essaie PAS encore de rendre une dalle au compagnon. Il construit la
//! condition necessaire au reclaim : savoir, sans allocation auxiliaire et
//! sans verrou global sur le chemin chaud, quelle dalle porte un bloc et
//! combien d'objets de cette dalle sont encore vivants.
//!
//! Le registre est volontairement borne et fail-open. S'il est sature, le tas
//! continue de fonctionner exactement comme avant C9 ; seule l'observabilite
//! de cette dalle manque et le compteur `saturations` le dit. Le reclaim C9.2
//! refusera naturellement toute dalle non suivie.
//!
//! Chaque entree est publiee par sa `base` avec Release. Les compteurs d'objets
//! sont atomiques : allouer/liberer un petit objet n'introduit donc aucun gros
//! verrou partage entre les CPU.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::kernel::pages_tas;

const ENTREES: usize = 16 * 1024;
const RESERVEE: usize = 1;
const PROBES_CHAUDS: usize = 32;

struct Dalle {
    base: AtomicUsize,
    taille: AtomicUsize,
    octets: AtomicUsize,
    total: AtomicUsize,
    vivants: AtomicUsize,
}

impl Dalle {
    const fn neuve() -> Self {
        Self {
            base: AtomicUsize::new(0),
            taille: AtomicUsize::new(0),
            octets: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
            vivants: AtomicUsize::new(0),
        }
    }
}

static DALLES: [Dalle; ENTREES] = [const { Dalle::neuve() }; ENTREES];

static ENREGISTREES: AtomicU64 = AtomicU64::new(0);
static CANDIDATS_VIDES: AtomicU64 = AtomicU64::new(0);
static MANQUES: AtomicU64 = AtomicU64::new(0);
static SATURATIONS: AtomicU64 = AtomicU64::new(0);
static SOUS_FLUX: AtomicU64 = AtomicU64::new(0);
static SURALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static MAX_PROBE: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn hache(base: usize) -> usize {
    let page = base >> 12;
    page.wrapping_mul(0x9E37_79B1_85EB_CA87usize) & (ENTREES - 1)
}

pub const fn taille_dalle(taille: usize, lot: usize) -> usize {
    let brut = taille.saturating_mul(lot);
    let pages = (brut + pages_tas::PAGE - 1) / pages_tas::PAGE;
    if pages == 0 {
        pages_tas::PAGE
    } else {
        pages * pages_tas::PAGE
    }
}

#[inline]
fn base_pour(adresse: usize, taille: usize, octets: usize) -> Option<usize> {
    let origine = pages_tas::base();
    if origine == 0 || adresse < origine || octets == 0 || taille == 0 {
        return None;
    }
    let decalage = adresse - origine;
    Some(origine + (decalage / octets) * octets)
}

fn cherche(base: usize) -> Option<(&'static Dalle, usize)> {
    if base <= RESERVEE {
        return None;
    }
    let depart = hache(base);
    for probe in 0..PROBES_CHAUDS {
        let entree = &DALLES[(depart + probe) & (ENTREES - 1)];
        let vue = entree.base.load(Ordering::Acquire);
        if vue == base {
            MAX_PROBE.fetch_max(probe + 1, Ordering::Relaxed);
            return Some((entree, probe + 1));
        }
        if vue == 0 {
            MAX_PROBE.fetch_max(probe + 1, Ordering::Relaxed);
            return None;
        }
    }
    MAX_PROBE.fetch_max(PROBES_CHAUDS, Ordering::Relaxed);
    None
}

pub fn enregistre(base: usize, taille: usize, octets: usize) -> bool {
    if base <= RESERVEE || taille == 0 || octets == 0 || octets % taille != 0 {
        MANQUES.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    if !pages_tas::nous_appartient(base) {
        MANQUES.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    let Some(calculee) = base_pour(base, taille, octets) else {
        MANQUES.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    if calculee != base {
        MANQUES.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    let depart = hache(base);
    for probe in 0..ENTREES {
        let entree = &DALLES[(depart + probe) & (ENTREES - 1)];
        let vue = entree.base.load(Ordering::Acquire);
        if vue == base {
            MANQUES.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        if vue != 0 {
            continue;
        }
        if entree
            .base
            .compare_exchange(0, RESERVEE, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            continue;
        }

        entree.taille.store(taille, Ordering::Relaxed);
        entree.octets.store(octets, Ordering::Relaxed);
        entree.total.store(octets / taille, Ordering::Relaxed);
        entree.vivants.store(0, Ordering::Relaxed);
        entree.base.store(base, Ordering::Release);

        ENREGISTREES.fetch_add(1, Ordering::Relaxed);
        MAX_PROBE.fetch_max(probe + 1, Ordering::Relaxed);
        return true;
    }

    SATURATIONS.fetch_add(1, Ordering::Relaxed);
    false
}

pub fn note_allocation(adresse: usize, taille: usize, lot: usize) {
    if !pages_tas::nous_appartient(adresse) {
        return;
    }
    let octets = taille_dalle(taille, lot);
    let Some(base) = base_pour(adresse, taille, octets) else {
        MANQUES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some((entree, _)) = cherche(base) else {
        MANQUES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if entree.taille.load(Ordering::Acquire) != taille {
        MANQUES.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let avant = entree.vivants.fetch_add(1, Ordering::AcqRel);
    let total = entree.total.load(Ordering::Acquire);
    if avant >= total {
        SURALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn note_liberation(adresse: usize, taille: usize, lot: usize) -> bool {
    if !pages_tas::nous_appartient(adresse) {
        return false;
    }
    let octets = taille_dalle(taille, lot);
    let Some(base) = base_pour(adresse, taille, octets) else {
        MANQUES.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    let Some((entree, _)) = cherche(base) else {
        MANQUES.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    if entree.taille.load(Ordering::Acquire) != taille {
        MANQUES.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    let mut courant = entree.vivants.load(Ordering::Acquire);
    loop {
        if courant == 0 {
            SOUS_FLUX.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        match entree.vivants.compare_exchange_weak(
            courant,
            courant - 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                if courant == 1 {
                    CANDIDATS_VIDES.fetch_add(1, Ordering::Relaxed);
                    return true;
                }
                return false;
            }
            Err(vu) => courant = vu,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub enregistrees: u64,
    pub dalles_vides: usize,
    pub objets_vivants: usize,
    pub candidats_vides: u64,
    pub manques: u64,
    pub saturations: u64,
    pub sous_flux: u64,
    pub surallocations: u64,
    pub max_probe: usize,
}

pub fn stats() -> Stats {
    let mut vides = 0usize;
    let mut vivants = 0usize;
    for entree in DALLES.iter() {
        let base = entree.base.load(Ordering::Acquire);
        if base <= RESERVEE {
            continue;
        }
        let n = entree.vivants.load(Ordering::Acquire);
        vivants = vivants.saturating_add(n);
        if n == 0 {
            vides += 1;
        }
    }
    Stats {
        enregistrees: ENREGISTREES.load(Ordering::Relaxed),
        dalles_vides: vides,
        objets_vivants: vivants,
        candidats_vides: CANDIDATS_VIDES.load(Ordering::Relaxed),
        manques: MANQUES.load(Ordering::Relaxed),
        saturations: SATURATIONS.load(Ordering::Relaxed),
        sous_flux: SOUS_FLUX.load(Ordering::Relaxed),
        surallocations: SURALLOCATIONS.load(Ordering::Relaxed),
        max_probe: MAX_PROBE.load(Ordering::Relaxed),
    }
}

pub fn log_stats() {
    let s = stats();
    crate::serial_println!(
        "[MEM-NG-DALLES] enregistrees={} vides={} objets_vivants={} candidats_vides={} manques={} saturations={} sous_flux={} surallocations={} max_probe={}",
        s.enregistrees,
        s.dalles_vides,
        s.objets_vivants,
        s.candidats_vides,
        s.manques,
        s.saturations,
        s.sous_flux,
        s.surallocations,
        s.max_probe,
    );
}
