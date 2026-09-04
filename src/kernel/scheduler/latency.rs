//! Observatoire de la latence reveil -> sur un coeur.
//!
//! # Ce qu'une moyenne et un maximum ne disent pas
//!
//! `avg_ns` et `max_ns` bornaient deja les figements de plusieurs secondes.
//! Ils ne disent rien de ce que l'utilisateur RESSENT : une moyenne noie les
//! quelques pour cent de reveils lents qui font qu'une interface « accroche »,
//! et un maximum unique peut venir d'un seul evenement de boot.
//!
//! Ce qui se voit a l'ecran est le CENTILE. Un p99 a 30 ms, c'est un clic sur
//! cent qui repond en retard -- visible, et invisible dans la moyenne.
//!
//! # Pourquoi un histogramme, et pas des echantillons
//!
//! `record` s'execute a chaque installation de tache sur un coeur, sur le
//! chemin de commutation, interruptions potentiellement masquees. Il n'y a ni
//! allocation ni verrou possibles. Un histogramme a classes fixes -- une classe
//! par puissance de deux de nanosecondes -- se met a jour avec un
//! `leading_zeros` et un `fetch_add`, et se lit a froid.
//!
//! La resolution d'une classe est un facteur deux ; un centile lu ici est donc
//! une BORNE SUPERIEURE de la vraie valeur, jamais une sous-estimation. C'est
//! le bon sens de l'erreur pour un budget.
//!
//! # Separees par classe
//!
//! Interactive et Normale ont des histogrammes distincts. Melangees, la classe
//! interactive -- minoritaire en nombre d'evenements -- disparaissait dans la
//! masse du travail de fond, et c'est precisement elle que le chantier 2
//! cherche a ameliorer.

use core::sync::atomic::{AtomicU64, Ordering};

/// Une classe par puissance de deux de nanosecondes. `CLASSES - 1` couvre
/// jusqu'a 2^47 ns, soit environ 39 heures : rien ne deborde.
pub const CLASSES: usize = 48;

/// Les deux classes d'ordonnancement observees separement.
pub const INTERACTIVE: usize = 0;
pub const NORMALE: usize = 1;
const CLASSES_ORDONNANCEMENT: usize = 2;

static COUNT: AtomicU64 = AtomicU64::new(0);
static SUM_NS: AtomicU64 = AtomicU64::new(0);
static MAX_NS: AtomicU64 = AtomicU64::new(0);
static INTERACTIVE_COUNT: AtomicU64 = AtomicU64::new(0);
static INTERACTIVE_MAX_NS: AtomicU64 = AtomicU64::new(0);
static B_LT_100US: AtomicU64 = AtomicU64::new(0);
static B_LT_500US: AtomicU64 = AtomicU64::new(0);
static B_LT_2MS: AtomicU64 = AtomicU64::new(0);
static B_LT_8MS: AtomicU64 = AtomicU64::new(0);
static B_LT_16MS: AtomicU64 = AtomicU64::new(0);
static B_GE_16MS: AtomicU64 = AtomicU64::new(0);

/// L'histogramme par classe d'ordonnancement.
static HISTOGRAMME: [[AtomicU64; CLASSES]; CLASSES_ORDONNANCEMENT] =
    [const { [const { AtomicU64::new(0) }; CLASSES] }; CLASSES_ORDONNANCEMENT];
static MAX_PAR_CLASSE: [AtomicU64; CLASSES_ORDONNANCEMENT] =
    [const { AtomicU64::new(0) }; CLASSES_ORDONNANCEMENT];

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub count: u64,
    pub average_ns: u64,
    pub max_ns: u64,
    pub interactive_count: u64,
    pub interactive_max_ns: u64,
    pub buckets: [u64; 6],
}

/// Les centiles d'une classe d'ordonnancement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Centiles {
    pub count: u64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
}

/// La classe d'histogramme d'une duree : sa puissance de deux.
///
/// `0 ns` tombe dans la classe 0, dont la borne superieure est 1 ns. Une
/// latence nulle est donc rapportee comme « au plus 1 ns », ce qui est vrai.
#[inline]
pub const fn classe(ns: u64) -> usize {
    let bits = 64 - ns.leading_zeros() as usize;
    if bits >= CLASSES { CLASSES - 1 } else { bits }
}

/// La borne SUPERIEURE d'une classe, en nanosecondes.
#[inline]
pub const fn borne_superieure(classe: usize) -> u64 {
    if classe == 0 {
        1
    } else if classe >= 63 {
        u64::MAX
    } else {
        1u64 << classe
    }
}

pub fn record(ns: u64, interactive: bool) {
    COUNT.fetch_add(1, Ordering::Relaxed);
    SUM_NS.fetch_add(ns, Ordering::Relaxed);
    MAX_NS.fetch_max(ns, Ordering::Relaxed);
    if interactive {
        INTERACTIVE_COUNT.fetch_add(1, Ordering::Relaxed);
        INTERACTIVE_MAX_NS.fetch_max(ns, Ordering::Relaxed);
    }
    let classe_ordo = if interactive { INTERACTIVE } else { NORMALE };
    HISTOGRAMME[classe_ordo][classe(ns)].fetch_add(1, Ordering::Relaxed);
    MAX_PAR_CLASSE[classe_ordo].fetch_max(ns, Ordering::Relaxed);
    match ns {
        0..=99_999 => { B_LT_100US.fetch_add(1, Ordering::Relaxed); }
        100_000..=499_999 => { B_LT_500US.fetch_add(1, Ordering::Relaxed); }
        500_000..=1_999_999 => { B_LT_2MS.fetch_add(1, Ordering::Relaxed); }
        2_000_000..=7_999_999 => { B_LT_8MS.fetch_add(1, Ordering::Relaxed); }
        8_000_000..=15_999_999 => { B_LT_16MS.fetch_add(1, Ordering::Relaxed); }
        _ => { B_GE_16MS.fetch_add(1, Ordering::Relaxed); }
    }
}

/// Les centiles d'une classe d'ordonnancement, lus a froid.
///
/// L'histogramme peut bouger pendant la lecture : le total est donc calcule
/// sur les memes valeurs que le parcours, en copiant d'abord.
pub fn centiles(classe_ordo: usize) -> Centiles {
    if classe_ordo >= CLASSES_ORDONNANCEMENT {
        return Centiles::default();
    }
    let mut copie = [0u64; CLASSES];
    let mut total = 0u64;
    for (indice, case) in HISTOGRAMME[classe_ordo].iter().enumerate() {
        copie[indice] = case.load(Ordering::Relaxed);
        total = total.saturating_add(copie[indice]);
    }
    if total == 0 {
        return Centiles::default();
    }
    Centiles {
        count: total,
        p50_ns: centile_depuis(&copie, total, 50),
        p95_ns: centile_depuis(&copie, total, 95),
        p99_ns: centile_depuis(&copie, total, 99),
        max_ns: MAX_PAR_CLASSE[classe_ordo].load(Ordering::Relaxed),
    }
}

/// La borne superieure de la classe qui contient le `centile`-ieme echantillon.
pub(crate) fn centile_depuis(histogramme: &[u64; CLASSES], total: u64, centile: u64) -> u64 {
    // Rang au PLAFOND (methode du rang le plus proche). Avec un plancher, le
    // p95 de dix echantillons serait le neuvieme : le seul retardataire du lot
    // serait invisible, et un budget p95 resterait vert en le contenant. Le
    // plafond fait tomber le centile sur le dixieme, donc sur le retardataire.
    let rang = (total.saturating_mul(centile) + 99) / 100;
    let rang = rang.max(1);
    let mut cumul = 0u64;
    for (indice, compte) in histogramme.iter().enumerate() {
        cumul = cumul.saturating_add(*compte);
        if cumul >= rang {
            return borne_superieure(indice);
        }
    }
    borne_superieure(CLASSES - 1)
}

pub fn stats() -> Stats {
    let count = COUNT.load(Ordering::Relaxed);
    Stats {
        count,
        average_ns: if count == 0 { 0 } else { SUM_NS.load(Ordering::Relaxed) / count },
        max_ns: MAX_NS.load(Ordering::Relaxed),
        interactive_count: INTERACTIVE_COUNT.load(Ordering::Relaxed),
        interactive_max_ns: INTERACTIVE_MAX_NS.load(Ordering::Relaxed),
        buckets: [
            B_LT_100US.load(Ordering::Relaxed),
            B_LT_500US.load(Ordering::Relaxed),
            B_LT_2MS.load(Ordering::Relaxed),
            B_LT_8MS.load(Ordering::Relaxed),
            B_LT_16MS.load(Ordering::Relaxed),
            B_GE_16MS.load(Ordering::Relaxed),
        ],
    }
}

pub fn log_stats() {
    let s = stats();
    crate::serial_println!(
        "[SCHED-NG-LAT] count={} avg_ns={} max_ns={} interactive_count={} interactive_max_ns={} buckets_lt100us={},lt500us={},lt2ms={},lt8ms={},lt16ms={},ge16ms={}",
        s.count, s.average_ns, s.max_ns, s.interactive_count,
        s.interactive_max_ns, s.buckets[0], s.buckets[1], s.buckets[2],
        s.buckets[3], s.buckets[4], s.buckets[5]
    );
    for (classe_ordo, nom) in [(INTERACTIVE, "interactive"), (NORMALE, "normale")] {
        let c = centiles(classe_ordo);
        crate::serial_println!(
            "[SCHED-NG-CENTILES] classe={} count={} p50_ns={} p95_ns={} p99_ns={} max_ns={}",
            nom, c.count, c.p50_ns, c.p95_ns, c.p99_ns, c.max_ns
        );
    }
}
