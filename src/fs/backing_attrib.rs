//! Attribution des lectures disque : qui lit quoi, combien de fois, et ce que
//! coute l'attente du controleur.
//!
//! # Pourquoi cette sonde
//!
//! Sous Ladybird, `BACKING_DISK_GLOBAL` donne 4 474 lectures, 293 Mio et
//! 120,9 s (run 36239992990). Trois questions restaient sans reponse :
//!
//!   1. QUI : les instantanes par service (`FAULT_FILE_SNAPSHOT`) n'expliquent
//!      que ~22 s des lectures du cache de pages ; le reste n'a pas de nom ;
//!   2. SOMME OU CHEMIN : `total_us` additionne des durees mesurees AVANT la
//!      prise du verrou du controleur ATA. Deux lectures concurrentes comptent
//!      chacune l'attente de l'autre : la somme n'est pas le temps pendant
//!      lequel le disque a travaille ;
//!   3. RELU : le cache de pages propres est plafonne (64 Mio) pour des
//!      binaires de 60 a 196 Mio. Une page evincee puis refautee est relue sur
//!      le disque ; sans temoin, « lire les binaires » et « relire les memes
//!      pages » se confondent.
//!
//! # Ce qui est mesure
//!
//!   * par (pid, noeud) : lectures, octets, lectures sequentielles (debut ==
//!     fin de la precedente du meme couple), temps de SERVICE (verrou du
//!     controleur tenu : le disque travaille pour cette lecture), temps
//!     d'ATTENTE du verrou (une autre lecture occupe le controleur), pire
//!     service, pages deja lues auparavant ;
//!   * global : service cumule -- le controleur est une ressource SERIE, sa
//!     somme de service est donc le temps d'occupation reel du disque, borne
//!     par l'ecoule --, attente cumulee, pages neuves / relues, histogramme
//!     des tailles.
//!
//! # Sans verrou, par construction
//!
//! Tout est atomique : ces compteurs sont publies depuis `exit_current`, qui
//! tient `process.lifecycle` (voir `verifie-sondes-sans-verrou.py`). La table
//! est a adressage ouvert, cle posee par `compare_exchange` ; une table pleine
//! compte ses debordements au lieu de les taire.

use core::sync::atomic::{AtomicU64, Ordering};

const COUPLES: usize = 64;
const NOEUDS_SUIVIS: usize = 16;
/// Pages couvertes par le bitmap d'un noeud : 256 Mio. Au-dela, la page est
/// comptee `hors_bitmap`, jamais devinee.
const PAGES_PAR_NOEUD: usize = 65_536;
const MOTS_PAR_NOEUD: usize = PAGES_PAR_NOEUD / 64;
const PAGE: usize = 4096;
const TAILLES: usize = 8;

struct Couple {
    cle: AtomicU64,
    lectures: AtomicU64,
    octets: AtomicU64,
    sequentielles: AtomicU64,
    service_ns: AtomicU64,
    attente_ns: AtomicU64,
    pire_service_ns: AtomicU64,
    pages_relues: AtomicU64,
    fin_precedente: AtomicU64,
}

impl Couple {
    const fn neuf() -> Self {
        Self {
            cle: AtomicU64::new(0),
            lectures: AtomicU64::new(0),
            octets: AtomicU64::new(0),
            sequentielles: AtomicU64::new(0),
            service_ns: AtomicU64::new(0),
            attente_ns: AtomicU64::new(0),
            pire_service_ns: AtomicU64::new(0),
            pages_relues: AtomicU64::new(0),
            fin_precedente: AtomicU64::new(u64::MAX),
        }
    }
}

static TABLE: [Couple; COUPLES] = [const { Couple::neuf() }; COUPLES];
static NOEUDS: [AtomicU64; NOEUDS_SUIVIS] = [const { AtomicU64::new(0) }; NOEUDS_SUIVIS];
static BITS: [[AtomicU64; MOTS_PAR_NOEUD]; NOEUDS_SUIVIS] =
    [const { [const { AtomicU64::new(0) }; MOTS_PAR_NOEUD] }; NOEUDS_SUIVIS];

static LECTURES: AtomicU64 = AtomicU64::new(0);
static OCTETS: AtomicU64 = AtomicU64::new(0);
static SERVICE_NS: AtomicU64 = AtomicU64::new(0);
static ATTENTE_NS: AtomicU64 = AtomicU64::new(0);
static PAGES_NEUVES: AtomicU64 = AtomicU64::new(0);
static PAGES_RELUES: AtomicU64 = AtomicU64::new(0);
static HORS_BITMAP: AtomicU64 = AtomicU64::new(0);
static DEBORDEMENTS: AtomicU64 = AtomicU64::new(0);
static RELEVES: AtomicU64 = AtomicU64::new(0);
static TAILLE: [AtomicU64; TAILLES] = [const { AtomicU64::new(0) }; TAILLES];

fn couple(pid: u64, noeud: usize) -> Option<&'static Couple> {
    let cle = ((pid + 1) << 24) | (noeud as u64 & 0xFF_FFFF);
    let depart = (cle.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 58) as usize % COUPLES;
    for pas in 0..COUPLES {
        let c = &TABLE[(depart + pas) % COUPLES];
        let vue = c.cle.load(Ordering::Acquire);
        if vue == cle {
            return Some(c);
        }
        if vue == 0 {
            match c.cle.compare_exchange(0, cle, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(c),
                Err(autre) if autre == cle => return Some(c),
                Err(_) => continue,
            }
        }
    }
    DEBORDEMENTS.fetch_add(1, Ordering::Relaxed);
    None
}

fn bitmap(noeud: usize) -> Option<&'static [AtomicU64; MOTS_PAR_NOEUD]> {
    let cle = noeud as u64 + 1;
    for (i, slot) in NOEUDS.iter().enumerate() {
        let vue = slot.load(Ordering::Acquire);
        if vue == cle {
            return Some(&BITS[i]);
        }
        if vue == 0 {
            match slot.compare_exchange(0, cle, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(&BITS[i]),
                Err(autre) if autre == cle => return Some(&BITS[i]),
                Err(_) => continue,
            }
        }
    }
    None
}

/// Une lecture disque terminee. `service_ns` : duree verrou du controleur
/// tenu ; `attente_ns` : duree passee a l'attendre.
pub fn note(pid: u64, noeud: usize, offset: usize, octets: usize, service_ns: u64, attente_ns: u64) {
    if octets == 0 {
        return;
    }
    LECTURES.fetch_add(1, Ordering::Relaxed);
    OCTETS.fetch_add(octets as u64, Ordering::Relaxed);
    SERVICE_NS.fetch_add(service_ns, Ordering::Relaxed);
    ATTENTE_NS.fetch_add(attente_ns, Ordering::Relaxed);
    let classe = (octets.saturating_sub(1) / PAGE).checked_ilog2().map_or(0, |b| b as usize + 1);
    TAILLE[classe.min(TAILLES - 1)].fetch_add(1, Ordering::Relaxed);

    let mut relues = 0u64;
    let premiere = offset / PAGE;
    let derniere = (offset + octets - 1) / PAGE;
    match bitmap(noeud) {
        Some(bits) => {
            for page in premiere..=derniere {
                if page >= PAGES_PAR_NOEUD {
                    HORS_BITMAP.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let masque = 1u64 << (page % 64);
                if bits[page / 64].fetch_or(masque, Ordering::Relaxed) & masque != 0 {
                    relues += 1;
                } else {
                    PAGES_NEUVES.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        None => {
            HORS_BITMAP.fetch_add((derniere - premiere + 1) as u64, Ordering::Relaxed);
        }
    }
    PAGES_RELUES.fetch_add(relues, Ordering::Relaxed);

    if let Some(c) = couple(pid, noeud) {
        c.lectures.fetch_add(1, Ordering::Relaxed);
        c.octets.fetch_add(octets as u64, Ordering::Relaxed);
        c.service_ns.fetch_add(service_ns, Ordering::Relaxed);
        c.attente_ns.fetch_add(attente_ns, Ordering::Relaxed);
        c.pire_service_ns.fetch_max(service_ns, Ordering::Relaxed);
        c.pages_relues.fetch_add(relues, Ordering::Relaxed);
        let fin = (offset + octets) as u64;
        if c.fin_precedente.swap(fin, Ordering::Relaxed) == offset as u64 {
            c.sequentielles.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Publie le releve. Sans verrou : voir l'en-tete.
///
/// Les lignes `BACKING_DISK_ATTRIB` d'un meme releve portent le meme
/// `releve=` : le lecteur prend le dernier releve entier, pas les dix
/// dernieres lignes.
pub fn publie(t_ms: u64) {
    let releve = RELEVES.fetch_add(1, Ordering::Relaxed) + 1;
    let t = |i: usize| TAILLE[i].load(Ordering::Relaxed);
    crate::kernel::dmesg::log_fmt(format_args!(
        "BACKING_DISK_DECOMP scope=global releve={} t={} lectures={} octets={} service_us={} \
attente_us={} pages_neuves={} pages_relues={} hors_bitmap={} debordements={} \
tailles=4k:{},8k:{},16k:{},32k:{},64k:{},128k:{},256k:{},plus:{}",
        releve,
        t_ms,
        LECTURES.load(Ordering::Relaxed),
        OCTETS.load(Ordering::Relaxed),
        SERVICE_NS.load(Ordering::Relaxed) / 1_000,
        ATTENTE_NS.load(Ordering::Relaxed) / 1_000,
        PAGES_NEUVES.load(Ordering::Relaxed),
        PAGES_RELUES.load(Ordering::Relaxed),
        HORS_BITMAP.load(Ordering::Relaxed),
        DEBORDEMENTS.load(Ordering::Relaxed),
        t(0), t(1), t(2), t(3), t(4), t(5), t(6), t(7),
    ));
    for c in TABLE.iter() {
        let cle = c.cle.load(Ordering::Acquire);
        let lectures = c.lectures.load(Ordering::Relaxed);
        if cle == 0 || lectures == 0 {
            continue;
        }
        crate::kernel::dmesg::log_fmt(format_args!(
            "BACKING_DISK_ATTRIB releve={} pid={} node={} lectures={} octets={} sequentielles={} \
service_us={} attente_us={} pire_service_us={} pages_relues={}",
            releve,
            (cle >> 24) - 1,
            cle & 0xFF_FFFF,
            lectures,
            c.octets.load(Ordering::Relaxed),
            c.sequentielles.load(Ordering::Relaxed),
            c.service_ns.load(Ordering::Relaxed) / 1_000,
            c.attente_ns.load(Ordering::Relaxed) / 1_000,
            c.pire_service_ns.load(Ordering::Relaxed) / 1_000,
            c.pages_relues.load(Ordering::Relaxed),
        ));
    }
}
