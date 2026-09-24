//! Physical page cache for immutable disk-backed, read-only mappings.
//!
//! P0-NG1 keeps the proven indexed cache and adds two performance properties:
//! clean-page allocation/free uses the bounded per-CPU frame cache, and reclaim
//! becomes pressure-aware. Cache state is never discarded without revalidating
//! `mappings == 0` under the entry lock.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::kernel::sync::{SpinLock, WaitQueue};
use crate::kernel::vmm::PAGE_SIZE;

const MAX_RECLAIMABLE_PAGES: usize = 16_384;
const LOW_PRESSURE_TARGET: usize = 4_096;
const CRITICAL_PRESSURE_TARGET: usize = 1_024;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub node: usize,
    pub offset: u64,
    pub generation: u64,
}

#[derive(Clone, Copy)]
enum State { Loading, Present { frame: u64, mappings: usize }, Failed }
struct Entry { key: Key, state: SpinLock<State>, waiters: WaitQueue }

struct Cache {
    entrees: BTreeMap<Key, Arc<Entry>>,
    candidats: VecDeque<Key>,
}
const LIMITE_CANDIDATS: usize = 4 * MAX_RECLAIMABLE_PAGES;
impl Cache {
    fn propose(&mut self, key: Key) {
        if self.candidats.len() < LIMITE_CANDIDATS { self.candidats.push_back(key); }
    }
}

static CACHE: SpinLock<Cache> = SpinLock::new(Cache {
    entrees: BTreeMap::new(), candidats: VecDeque::new(),
});
static RECUPERABLES: AtomicUsize = AtomicUsize::new(0);
static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);
static WAITS: AtomicU64 = AtomicU64::new(0);
static SHARED_MAPS: AtomicU64 = AtomicU64::new(0);
static RECLAIMED: AtomicU64 = AtomicU64::new(0);

#[inline]
fn devient_recuperable() { RECUPERABLES.fetch_add(1, Ordering::Relaxed); }
#[inline]
fn cesse_d_etre_recuperable() {
    let _ = RECUPERABLES.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| Some(v.saturating_sub(1)));
}
fn recuperable(etat: &State) -> bool {
    matches!(etat, State::Present { mappings: 0, .. } | State::Failed)
}

fn allocate_clean_frame() -> Option<u64> {
    if let Some(frame) = crate::kernel::frame_cache::alloc_frame() { return Some(frame); }
    if crate::kernel::memory_pressure::recover_allocation() {
        if let Some(frame) = crate::kernel::frame_cache::alloc_frame() { return Some(frame); }
    }
    crate::kernel::memory_pressure::note_oom();
    None
}

// BOUCHAUD_C52_ACQUIRE_N_EST_PAS_UN_LOOKUP
//
// `acquire` ne consulte pas un cache : sur un defaut, il LIT LE SUPPORT
// lui-meme (voir la branche `loader` plus bas). Chronometrer l'appel entier et
// l'appeler « cache » melange donc deux choses qui n'ont rien a voir :
//
//     un hit          -> une prise de verrou et un compteur
//     un miss         -> une allocation de trame PLUS une lecture ATA
//     une attente     -> un autre coeur fait le travail
//
// La premiere version de la decomposition des fautes publiait `cache_us=62 ms`
// et `backing_us=0`, ce qui se lisait « le cache est lent et le disque ne fait
// rien ». C'etait faux : le disque travaillait DANS le cache.
static HIT_NS: AtomicU64 = AtomicU64::new(0);
static MISS_NS: AtomicU64 = AtomicU64::new(0);
static MISS_READ_NS: AtomicU64 = AtomicU64::new(0);
static WAIT_NS: AtomicU64 = AtomicU64::new(0);
static PIRE_NS: AtomicU64 = AtomicU64::new(0);

/// (hits, miss, attentes, hit_ns, miss_ns, miss_read_ns, wait_ns, pire_ns)
pub fn acquire_timing() -> (u64, u64, u64, u64, u64, u64, u64, u64) {
    (
        HITS.load(Ordering::Relaxed),
        MISSES.load(Ordering::Relaxed),
        WAITS.load(Ordering::Relaxed),
        HIT_NS.load(Ordering::Relaxed),
        MISS_NS.load(Ordering::Relaxed),
        MISS_READ_NS.load(Ordering::Relaxed),
        WAIT_NS.load(Ordering::Relaxed),
        PIRE_NS.load(Ordering::Relaxed),
    )
}

/// Assemble la mesure. Un seul endroit ou l'horloge de fin est lue.
#[inline]
fn rends(
    frame: Option<u64>,
    issue: IssueAcquire,
    debut_ns: u64,
    backing_ns: u64,
) -> MesureAcquire {
    MesureAcquire {
        frame,
        issue,
        total_ns: crate::kernel::timer::monotonic_ns().saturating_sub(debut_ns),
        backing_ns,
    }
}

#[inline]
fn note_duree(compteur: &AtomicU64, debut_ns: u64) {
    let d = crate::kernel::timer::monotonic_ns().saturating_sub(debut_ns);
    compteur.fetch_add(d, Ordering::Relaxed);
    PIRE_NS.fetch_max(d, Ordering::Relaxed);
}

/// Comment une acquisition s'est terminee.
///
/// Les trois sont des ISSUES, pas des phases : une acquisition en traverse
/// exactement une. Les additionner comme des etapes serait faux.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IssueAcquire {
    /// La page etait la : une prise de verrou et un compteur.
    Hit,
    /// La page manquait : allocation d'une trame PLUS lecture du support.
    Miss,
    /// Un autre coeur la chargeait deja : on a attendu son reveil.
    Attente,
}

/// Ce qu'une acquisition a COUTE, rendu a l'appelant.
///
/// BOUCHAUD_C54_PAR_PID_OU_RIEN
///
/// Les compteurs globaux disent combien le systeme entier a lu. Ils ne peuvent
/// pas dire combien CE WebWorker a lu quand six services tournent ensemble.
/// Le chemin de faute, lui, connait son PID : c'est donc a lui que le temps
/// doit revenir, au moment ou il est paye.
///
/// `backing_ns` est INCLUS dans `total_ns` -- c'est un « dont », pas un terme
/// a additionner.
#[derive(Clone, Copy, Debug)]
pub struct MesureAcquire {
    pub frame: Option<u64>,
    pub issue: IssueAcquire,
    pub total_ns: u64,
    pub backing_ns: u64,
}

/// L'acquisition ordinaire. Conserve pour les appelants qui ne mesurent pas.
pub fn acquire(key: Key) -> Option<u64> {
    acquire_mesure(key).frame
}

pub fn acquire_mesure(key: Key) -> MesureAcquire {
    let debut_acquire_ns = crate::kernel::timer::monotonic_ns();
    let mut backing_ns = 0u64;
    let mut issue = IssueAcquire::Hit;
    if crate::fs::backing::generation(key.node) != Some(key.generation)
        || key.offset % PAGE_SIZE != 0
    {
        return MesureAcquire {
            frame: None, issue: IssueAcquire::Hit, total_ns: 0, backing_ns: 0,
        };
    }

    let (entry, loader, evicted) = {
        let mut cache = CACHE.lock();
        if let Some(entry) = cache.entrees.get(&key) {
            let entry = Arc::clone(entry);
            let mut state = entry.state.lock();
            match *state {
                State::Present { frame, mappings } => {
                    *state = State::Present {
                        frame,
                        mappings: mappings.checked_add(1).expect("clean cache ref overflow"),
                    };
                    HITS.fetch_add(1, Ordering::Relaxed);
                    if mappings == 0 { cesse_d_etre_recuperable(); }
                    else { SHARED_MAPS.fetch_add(1, Ordering::Relaxed); }
                    note_duree(&HIT_NS, debut_acquire_ns);
                    return rends(Some(frame), IssueAcquire::Hit,
                                 debut_acquire_ns, 0);
                }
                State::Failed => {
                    return rends(None, IssueAcquire::Hit, debut_acquire_ns, 0);
                }
                State::Loading => { drop(state); (entry, false, None) }
            }
        } else {
            let evicted = if cache.entrees.len() >= MAX_RECLAIMABLE_PAGES {
                retire_un_candidat(&mut cache)
            } else { None };
            let entry = Arc::new(Entry {
                key, state: SpinLock::new(State::Loading), waiters: WaitQueue::new(),
            });
            cache.entrees.insert(key, Arc::clone(&entry));
            (entry, true, evicted)
        }
    };

    if let Some(old) = evicted {
        let frame = {
            let mut state = old.state.lock();
            match *state {
                State::Present { frame, mappings: 0 } => {
                    *state = State::Failed;
                    Some(frame)
                }
                _ => None,
            }
        };
        if let Some(frame) = frame {
            crate::kernel::frame_cache::free_frame(frame);
            RECLAIMED.fetch_add(1, Ordering::Relaxed);
        }
    }

    if loader {
        MISSES.fetch_add(1, Ordering::Relaxed);
        let result = allocate_clean_frame().and_then(|frame| {
            let dst = crate::kernel::memory::phys_to_virt(frame);
            let bytes = unsafe { core::slice::from_raw_parts_mut(dst, PAGE_SIZE as usize) };
            let avant_lecture = crate::kernel::timer::monotonic_ns();
            let got = crate::fs::backing::read_at(key.node, key.offset as usize, bytes);
            let lu_ns = crate::kernel::timer::monotonic_ns()
                .saturating_sub(avant_lecture);
            MISS_READ_NS.fetch_add(lu_ns, Ordering::Relaxed);
            backing_ns = backing_ns.saturating_add(lu_ns);
            if got == PAGE_SIZE as usize
                && crate::fs::backing::generation(key.node) == Some(key.generation)
            {
                Some(frame)
            } else {
                crate::kernel::frame_cache::free_frame(frame);
                None
            }
        });
        let mut etat = entry.state.lock();
        *etat = match result {
            Some(frame) => State::Present { frame, mappings: 1 },
            None => State::Failed,
        };
        drop(etat);
        if result.is_none() {
            devient_recuperable();
            CACHE.lock().propose(key);
        }
        entry.waiters.wake_all();
        note_duree(&MISS_NS, debut_acquire_ns);
        return rends(result, IssueAcquire::Miss, debut_acquire_ns, backing_ns);
    }
    issue = IssueAcquire::Attente;

    loop {
        let ticket = entry.waiters.ticket();
        let mut state = entry.state.lock();
        match *state {
            State::Present { frame, mappings } => {
                *state = State::Present {
                    frame,
                    mappings: mappings.checked_add(1).expect("clean cache ref overflow"),
                };
                HITS.fetch_add(1, Ordering::Relaxed);
                if mappings == 0 { cesse_d_etre_recuperable(); }
                else { SHARED_MAPS.fetch_add(1, Ordering::Relaxed); }
                note_duree(&WAIT_NS, debut_acquire_ns);
                return rends(Some(frame), issue, debut_acquire_ns, backing_ns);
            }
            State::Failed => {
                return rends(None, issue, debut_acquire_ns, backing_ns);
            }
            State::Loading => {
                drop(state);
                WAITS.fetch_add(1, Ordering::Relaxed);
                entry.waiters.wait(ticket);
            }
        }
    }
}

pub fn retain(key: Key) -> bool {
    let cache = CACHE.lock();
    let Some(entry) = cache.entrees.get(&key) else { return false; };
    let mut state = entry.state.lock();
    if let State::Present { frame, mappings } = *state {
        *state = State::Present {
            frame,
            mappings: mappings.checked_add(1).expect("clean cache ref overflow"),
        };
        if mappings == 0 { cesse_d_etre_recuperable(); }
        true
    } else { false }
}

pub fn release(key: Key) {
    let entry = CACHE.lock().entrees.get(&key).cloned();
    let Some(entry) = entry else { panic!("clean page cache: release of unregistered key"); };
    let mut etat = entry.state.lock();
    let devenue_libre = if let State::Present { frame, mappings } = *etat {
        assert!(mappings != 0, "clean page cache: double release");
        *etat = State::Present { frame, mappings: mappings - 1 };
        mappings == 1
    } else { panic!("clean page cache: release of non-present entry"); };
    drop(etat);
    drop(entry);
    if devenue_libre {
        devient_recuperable();
        CACHE.lock().propose(key);
    }
    reclaim_excess();
}

// BOUCHAUD_C68_LE_BALAYAGE_DE_SECOURS
//
// SONDE, PAS CORRECTION. Le repli ci-dessous parcourt TOUTE la table et prend
// le verrou d'etat de chaque entree, sous le verrou global du cache. Au run
// 35944547625 la table portait 52 537 entrees.
//
// C'est un candidat pour le temps noyau que la decomposition des fautes
// n'explique pas -- 7,1 ms par defaut de cache dont seulement 1,7 ms de
// lecture reelle. Candidat, pas cause : tant que ces compteurs n'ont pas
// montre que ce chemin est seulement EMPRUNTE, l'accuser serait une
// supposition de plus.
//
// Ces compteurs disent trois choses independantes : combien de fois le repli
// est atteint, combien d'entrees il a fallu parcourir en tout, et le temps
// qu'il a coute. Un repli jamais atteint refute l'hypothese d'un seul coup.
pub static BALAYAGE_APPELS: AtomicU64 = AtomicU64::new(0);
pub static BALAYAGE_ENTREES: AtomicU64 = AtomicU64::new(0);
pub static BALAYAGE_NS: AtomicU64 = AtomicU64::new(0);
pub static BALAYAGE_PIRE_NS: AtomicU64 = AtomicU64::new(0);
/// Combien de fois la file de candidats a suffi -- le chemin qui NE balaye pas.
pub static CANDIDATS_SUFFISANTS: AtomicU64 = AtomicU64::new(0);
/// Balayages SUPPRIMES parce que `RECUPERABLES` valait zero. C'est la mesure
/// directe de la correction : sans elle, chacun aurait parcouru la table.
pub static BALAYAGE_EVITES: AtomicU64 = AtomicU64::new(0);

/// `appels, entrees_parcourues, total_ns, pire_ns, candidats_suffisants`
pub fn balayage_stats() -> (u64, u64, u64, u64, u64) {
    (
        BALAYAGE_APPELS.load(Ordering::Relaxed),
        BALAYAGE_ENTREES.load(Ordering::Relaxed),
        BALAYAGE_NS.load(Ordering::Relaxed),
        BALAYAGE_PIRE_NS.load(Ordering::Relaxed),
        CANDIDATS_SUFFISANTS.load(Ordering::Relaxed),
    )
}

/// `balayages_evites, entrees_en_table, pages_recuperees`
///
/// Publies A COTE du temps de balayage : un balayage supprime ne doit pas
/// faire deborder la table ni faire chuter les recuperations. Sans ces deux
/// temoins, « plus rapide » et « ne fait plus son travail » se confondraient.
pub fn balayage_temoins() -> (u64, usize, u64) {
    (
        BALAYAGE_EVITES.load(Ordering::Relaxed),
        CACHE.lock().entrees.len(),
        RECLAIMED.load(Ordering::Relaxed),
    )
}

fn retire_un_candidat(cache: &mut Cache) -> Option<Arc<Entry>> {
    while let Some(key) = cache.candidats.pop_front() {
        let sortable = match cache.entrees.get(&key) {
            Some(entry) => recuperable(&entry.state.lock()),
            None => false,
        };
        if sortable {
            cesse_d_etre_recuperable();
            CANDIDATS_SUFFISANTS.fetch_add(1, Ordering::Relaxed);
            return cache.entrees.remove(&key);
        }
    }
    // BOUCHAUD_C70_NE_PAS_BALAYER_POUR_NE_RIEN_TROUVER
    //
    // CAUSE CONFIRMEE, et corrigee ici.
    //
    // `acquire` appelle cette fonction a CHAQUE defaut de cache des que la
    // table atteint `MAX_RECLAIMABLE_PAGES`. Quand la file de candidats est
    // vide -- ce qui est le cas normal tant que les pages restent mappees --
    // le repli parcourait toute la table, en prenant le verrou d'etat de
    // chaque entree, sous le verrou GLOBAL du cache.
    //
    // Le modele « un balayage par defaut de cache une fois la table pleine »
    // se verifie au chiffre pres sur deux charges independantes :
    //
    //     banc local      20 480 miss - 16 384 = 4 096 attendus,  4 096 observes
    //     Ladybird #357   52 534 miss - 16 384 = 36 150 attendus, 35 495 observes
    //
    // Cout mesure : 277 secondes sur le run 35955074619, soit 57 % des 490 s
    // de temps noyau du run entier.
    //
    // # Pourquoi ce test suffit, et pourquoi il est SUR
    //
    // `RECUPERABLES` compte les entrees recuperables ; il est incremente et
    // decremente par paires aux memes endroits que l'etat qu'il resume. A
    // zero, le balayage est GARANTI de ne rien trouver -- il ne fait que
    // decouvrir lentement ce que le compteur dit deja.
    //
    // On ne change donc ni la taille du cache, ni la politique d'eviction, ni
    // l'ordre des victimes : on supprime un travail dont le resultat est
    // connu d'avance. Si le compteur etait errone en MOINS, une eviction
    // legitime serait sautee et la table depasserait son plafond ; c'est
    // pourquoi le banc verifie aussi `entrees` et `RECLAIMED`.
    if RECUPERABLES.load(Ordering::Relaxed) == 0 {
        BALAYAGE_EVITES.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let debut = crate::kernel::timer::monotonic_ns();
    let mut parcourues = 0u64;
    let victime = cache.entrees.iter()
        .map(|(key, entry)| { parcourues += 1; (key, entry) })
        .find(|(_, entry)| recuperable(&entry.state.lock()))
        .map(|(key, _)| *key);
    let ecoule = crate::kernel::timer::monotonic_ns().saturating_sub(debut);
    BALAYAGE_APPELS.fetch_add(1, Ordering::Relaxed);
    BALAYAGE_ENTREES.fetch_add(parcourues, Ordering::Relaxed);
    BALAYAGE_NS.fetch_add(ecoule, Ordering::Relaxed);
    BALAYAGE_PIRE_NS.fetch_max(ecoule, Ordering::Relaxed);
    let victime = victime?;
    cesse_d_etre_recuperable();
    cache.entrees.remove(&victime)
}

fn pressure_target() -> usize {
    match crate::kernel::memory_pressure::level() {
        crate::kernel::memory_pressure::Level::Normal => MAX_RECLAIMABLE_PAGES,
        crate::kernel::memory_pressure::Level::Low => LOW_PRESSURE_TARGET,
        crate::kernel::memory_pressure::Level::Critical => CRITICAL_PRESSURE_TARGET,
    }
}

fn take_reclaimable() -> Option<Option<u64>> {
    let entry = {
        let mut cache = CACHE.lock();
        retire_un_candidat(&mut cache)?
    };
    let mut state = entry.state.lock();
    match *state {
        State::Present { frame, mappings: 0 } => {
            *state = State::Failed;
            Some(Some(frame))
        }
        State::Failed => Some(None),
        _ => Some(None),
    }
}

fn reclaim_excess() {
    let target = pressure_target();
    while RECUPERABLES.load(Ordering::Relaxed) > target {
        match take_reclaimable() {
            Some(Some(frame)) => {
                crate::kernel::frame_cache::free_frame(frame);
                RECLAIMED.fetch_add(1, Ordering::Relaxed);
            }
            Some(None) => continue,
            None => return,
        }
    }
}

/// Pressure reclaim API: pages go directly to the global VMM, bypassing local
/// caches so this operation really increases globally available RAM.
pub fn reclaim_pages(limit: usize) -> usize {
    let mut freed = 0usize;
    while freed < limit {
        match take_reclaimable() {
            Some(Some(frame)) => {
                crate::kernel::frame_cache::free_frame_global(frame);
                freed += 1;
            }
            Some(None) => continue,
            None => break,
        }
    }
    if freed != 0 { RECLAIMED.fetch_add(freed as u64, Ordering::Relaxed); }
    freed
}

pub fn stats() -> (u64, u64, u64, u64) {
    (HITS.load(Ordering::Relaxed), MISSES.load(Ordering::Relaxed),
     WAITS.load(Ordering::Relaxed), SHARED_MAPS.load(Ordering::Relaxed))
}

pub fn lifetime_stats() -> (usize, usize) {
    let cache = CACHE.lock();
    let reclaimable = cache.entrees.values().filter(|entry| recuperable(&entry.state.lock())).count();
    (cache.entrees.len(), reclaimable)
}

pub fn indice_recuperables() -> usize { RECUPERABLES.load(Ordering::Relaxed) }
pub fn reclaimed_pages() -> u64 { RECLAIMED.load(Ordering::Relaxed) }

pub fn log_ng_stats() {
    // Reporting must stay lock-free: browser_report can run while compatibility
    // code still owns the BKL. Taking CACHE here would turn observability into
    // another lock-order edge. RECUPERABLES and RECLAIMED are sufficient to
    // prove that the pressure/reclaim path is active.
    crate::serial_println!(
        "[MEM-NG-PAGECACHE] reclaimable_index={} reclaimed={}",
        indice_recuperables(), reclaimed_pages()
    );
}
