//! Physical page cache for immutable disk-backed, read-only mappings.
//!
//! # ORDRE DES VERROUS
//!
//! Ce module a deux niveaux de verrou, et un seul ordre est permis :
//!
//! ```text
//!     CACHE  ->  Entry::state
//! ```
//!
//! Jamais l'inverse. Un chemin qui tient `state` et demande `CACHE` pendant
//! qu'un autre tient `CACHE` et demande `state` bloque les deux CPU pour
//! toujours.
//!
//! Deux chemins publient un etat PUIS proposent la cle a la recuperation —
//! `acquire` quand le chargement echoue, et `release` quand le dernier mapping
//! tombe. Ils ne prenaient pas les deux verrous en meme temps : le garde
//! d'etat etait un TEMPORAIRE, detruit a la fin de son instruction, donc
//! relache avant `CACHE.lock()`. Il n'y avait pas d'interblocage.
//!
//! Mais la surete tenait alors a une regle de duree de vie des temporaires, pas
//! a quelque chose de visible. Il suffisait de nommer le garde — un refactor
//! ordinaire — pour transformer un code correct en interblocage SMP, sans que
//! rien ne le signale.
//!
//! Les gardes sont donc desormais NOMMES et relaches explicitement avant toute
//! prise de `CACHE`. L'ordre se lit au lieu de se deduire, et
//! `tools/verifie-ordre-verrous.py` echoue si un chemin `state -> CACHE`
//! reapparait.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::kernel::sync::{SpinLock, WaitQueue};
use crate::kernel::vmm::{self, PAGE_SIZE};

/// Maximum number of reclaimable (zero-mapping) pages. Live mapped entries are
/// not evictable and may exceed this number by design.
/// V14: 16,384 pages = 64 MiB of clean reusable ELF/library data. The current
/// Ladybird profile has several GiB free, so evicting at 8 MiB only creates
/// avoidable ATA rereads and repeated loader stalls.
const MAX_RECLAIMABLE_PAGES: usize = 16_384;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub node: usize,
    pub offset: u64,
    pub generation: u64,
}

#[derive(Clone, Copy)]
enum State { Loading, Present { frame: u64, mappings: usize }, Failed }

struct Entry { key: Key, state: SpinLock<State>, waiters: WaitQueue }

// BOUCHAUD_P2_PAGE_CACHE_INDEX_V1
//
// CE QUE CETTE STRUCTURE REMPLACE, ET POURQUOI
// --------------------------------------------
// Le cache etait un `Vec<Arc<Entry>>` :
//
//   * `acquire`, `retain` et `release` cherchaient la cle par
//     `iter().find(..)` — `O(C)` par appel, sur le chemin de faute de page ;
//   * `release` appelait ensuite `reclaim_excess`, qui COMPTAIT les entrees
//     recuperables par `iter().filter(..).count()` en prenant le verrou de
//     CHAQUE entree — un balayage complet, avec `C` prises de verrou,
//     A CHAQUE LIBERATION.
//
// Un `madvise(DONTNEED)` qui rend K pages propres coutait donc `O(K x C)`
// prises de verrou, le gros verrou tenu.
//
// L'index rend la recherche `O(log C)`. Le compteur rend la decision
// « faut-il recuperer ? » constante : une lecture atomique.
//
// `RECUPERABLES` est un INDICE, pas un invariant. Il est mis a jour aux
// transitions, parfois sans le verrou du cache, donc il peut etre legerement
// faux. C'est admis parce qu'il ne sert qu'a decider s'il faut REGARDER : la
// recuperation elle-meme revalide chaque candidat sous les verrous. Un indice
// un peu faux fait chercher un peu trop tot ou un peu trop tard, jamais faux.
struct Cache {
    entrees: BTreeMap<Key, Arc<Entry>>,
    /// Cles devenues recuperables, dans l'ordre. Validees a la sortie : une
    /// cle re-referencee entre-temps est simplement ignoree.
    candidats: VecDeque<Key>,
}

/// Au-dela, on cesse d'empiler des candidats : le compteur declenchera quand
/// meme la recuperation, qui se rabattra sur un balayage. Ce repli est rare et
/// borne ; sans lui, la file serait la seule structure non bornee du cache.
const LIMITE_CANDIDATS: usize = 4 * MAX_RECLAIMABLE_PAGES;

impl Cache {
    fn propose(&mut self, key: Key) {
        if self.candidats.len() < LIMITE_CANDIDATS {
            self.candidats.push_back(key);
        }
    }
}

static CACHE: SpinLock<Cache> = SpinLock::new(Cache {
    entrees: BTreeMap::new(),
    candidats: VecDeque::new(),
});
/// Indice du nombre d'entrees a zero mapping. Voir le commentaire ci-dessus.
static RECUPERABLES: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn devient_recuperable() {
    RECUPERABLES.fetch_add(1, Ordering::Relaxed);
}

#[inline]
fn cesse_d_etre_recuperable() {
    // Saturant : l'indice ne doit jamais reboucler sous zero, sinon la
    // recuperation ne se declencherait plus jamais.
    let _ = RECUPERABLES.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |valeur| {
        Some(valeur.saturating_sub(1))
    });
}

/// L'entree est-elle recuperable dans son etat actuel ?
fn recuperable(etat: &State) -> bool {
    matches!(etat, State::Present { mappings: 0, .. } | State::Failed)
}
static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);
static WAITS: AtomicU64 = AtomicU64::new(0);
static SHARED_MAPS: AtomicU64 = AtomicU64::new(0);

/// Acquire one mapping reference. Only immutable disk extents are eligible, so
/// their backing generation is the registered extent itself.
pub fn acquire(key: Key) -> Option<u64> {
    if crate::fs::backing::generation(key.node) != Some(key.generation)
        || key.offset % PAGE_SIZE != 0 {
        return None;
    }
    let (entry, loader, evicted) = {
        let mut cache = CACHE.lock();
        if let Some(entry) = cache.entrees.get(&key) {
            let entry = Arc::clone(entry);
            let mut state = entry.state.lock();
            match *state {
                State::Present { frame, mappings } => {
                    *state = State::Present { frame, mappings: mappings.checked_add(1).expect("clean cache ref overflow") };
                    HITS.fetch_add(1, Ordering::Relaxed);
                    if mappings == 0 {
                        cesse_d_etre_recuperable();
                    } else {
                        SHARED_MAPS.fetch_add(1, Ordering::Relaxed);
                    }
                    return Some(frame);
                }
                State::Failed => return None,
                State::Loading => { drop(state); (entry, false, None) }
            }
        } else {
            let evicted = if cache.entrees.len() >= MAX_RECLAIMABLE_PAGES {
                retire_un_candidat(&mut cache)
            } else { None };
            let entry = Arc::new(Entry {
                key,
                state: SpinLock::new(State::Loading),
                waiters: WaitQueue::new(),
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
            vmm::free_frame(frame);
        }
    }

    if loader {
        MISSES.fetch_add(1, Ordering::Relaxed);
        let result = vmm::alloc_frame().and_then(|frame| {
            let dst = crate::kernel::memory::phys_to_virt(frame);
            let bytes = unsafe { core::slice::from_raw_parts_mut(dst, PAGE_SIZE as usize) };
            let got = crate::fs::backing::read_at(key.node, key.offset as usize, bytes);
            if got == PAGE_SIZE as usize
                && crate::fs::backing::generation(key.node) == Some(key.generation) {
                Some(frame)
            } else {
                vmm::free_frame(frame);
                None
            }
        });
        // ORDRE DES VERROUS : le garde d'etat est NOMME et relache avant toute
        // prise de `CACHE`. Voir l'en-tete du module.
        let mut etat = entry.state.lock();
        *etat = match result {
            Some(frame) => State::Present { frame, mappings: 1 },
            None => State::Failed,
        };
        drop(etat);

        if result.is_none() {
            // Une entree en echec ne sera jamais referencee : elle rejoint
            // les candidats a la recuperation des maintenant.
            devient_recuperable();
            CACHE.lock().propose(key);
        }
        // Reveiller APRES avoir tout publie : un dormeur reveille lit l'etat,
        // et il ne doit pas pouvoir le lire avant qu'il soit ecrit.
        entry.waiters.wake_all();
        return result;
    }

    loop {
        let ticket = entry.waiters.ticket();
        let mut state = entry.state.lock();
        match *state {
            State::Present { frame, mappings } => {
                *state = State::Present { frame, mappings: mappings.checked_add(1).expect("clean cache ref overflow") };
                HITS.fetch_add(1, Ordering::Relaxed);
                if mappings == 0 {
                    cesse_d_etre_recuperable();
                } else {
                    SHARED_MAPS.fetch_add(1, Ordering::Relaxed);
                }
                return Some(frame);
            }
            State::Failed => return None,
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
        *state = State::Present { frame, mappings: mappings.checked_add(1).expect("clean cache ref overflow") };
        if mappings == 0 {
            cesse_d_etre_recuperable();
        }
        true
    } else { false }
}

pub fn release(key: Key) {
    let entry = CACHE.lock().entrees.get(&key).cloned();
    let Some(entry) = entry else {
        panic!("clean page cache: release of unregistered key");
    };
    // ORDRE DES VERROUS : `state` est relache avant `CACHE`. Voir l'en-tete.
    let mut etat = entry.state.lock();
    let devenue_libre = if let State::Present { frame, mappings } = *etat {
        assert!(mappings != 0, "clean page cache: double release");
        *etat = State::Present { frame, mappings: mappings - 1 };
        mappings == 1
    } else {
        panic!("clean page cache: release of non-present entry");
    };
    drop(etat);
    drop(entry);
    if devenue_libre {
        devient_recuperable();
        CACHE.lock().propose(key);
    }
    reclaim_excess();
}

/// Sort UNE entree recuperable du cache, sans le balayer.
///
/// Les candidats sont valides a la sortie : une cle re-referencee depuis
/// qu'elle a ete proposee est simplement jetee. Le repli par balayage ne sert
/// que si la file est vide alors que l'indice annonce du travail — ce qui ne
/// peut arriver qu'apres un debordement de `LIMITE_CANDIDATS`.
fn retire_un_candidat(cache: &mut Cache) -> Option<Arc<Entry>> {
    while let Some(key) = cache.candidats.pop_front() {
        let sortable = match cache.entrees.get(&key) {
            Some(entry) => recuperable(&entry.state.lock()),
            None => false,
        };
        if sortable {
            cesse_d_etre_recuperable();
            return cache.entrees.remove(&key);
        }
    }
    // Repli : la file a deborde, on cherche une victime par balayage.
    let victime = cache
        .entrees
        .iter()
        .find(|(_, entry)| recuperable(&entry.state.lock()))
        .map(|(key, _)| *key)?;
    cesse_d_etre_recuperable();
    cache.entrees.remove(&victime)
}

fn reclaim_excess() {
    loop {
        // Le chemin normal d'une liberation ne coute QUE cette lecture.
        if RECUPERABLES.load(Ordering::Relaxed) <= MAX_RECLAIMABLE_PAGES {
            return;
        }
        let frame = {
            let mut cache = CACHE.lock();
            let Some(entry) = retire_un_candidat(&mut cache) else { return; };
            let mut state = entry.state.lock();
            match *state {
                State::Present { frame, mappings: 0 } => {
                    *state = State::Failed;
                    Some(frame)
                }
                State::Failed => None,
                _ => unreachable!(),
            }
        };
        if let Some(frame) = frame {
            vmm::free_frame(frame);
        }
    }
}

pub fn stats() -> (u64, u64, u64, u64) {
    (HITS.load(Ordering::Relaxed), MISSES.load(Ordering::Relaxed),
     WAITS.load(Ordering::Relaxed), SHARED_MAPS.load(Ordering::Relaxed))
}

/// (all entries, entries eligible for reclaim).
pub fn lifetime_stats() -> (usize, usize) {
    // Le releve, lui, a le droit de compter vraiment : il passe une fois par
    // seconde, pas une fois par page liberee. C'est aussi ce qui permet de
    // verifier l'indice au lieu de le croire.
    let cache = CACHE.lock();
    let reclaimable = cache
        .entrees
        .values()
        .filter(|entry| recuperable(&entry.state.lock()))
        .count();
    (cache.entrees.len(), reclaimable)
}

/// Indice courant, et sa valeur exacte. Un ecart durable est un defaut.
pub fn indice_recuperables() -> usize {
    RECUPERABLES.load(Ordering::Relaxed)
}
