use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::kernel::sync::SpinLock;

use super::abi::types::{Error, Result, Signals};
use super::object::Object;

#[derive(Clone)]
struct Watch {
    key: u64,
    object: Arc<Object>,
}

pub struct WaitSet {
    watches: SpinLock<Vec<Watch>>,
}

impl WaitSet {
    pub fn new() -> Self { Self { watches: SpinLock::new(Vec::new()) } }

    /// BOUCHAUD_NATIVE_WAITSET_SANS_CYCLE_V1
    ///
    /// Un waitset ne surveille JAMAIS un autre waitset, ni lui-meme.
    ///
    /// `poll` et `signals` tiennent `watches` pendant qu'ils interrogent les
    /// objets surveilles. Un waitset qui se surveillait reprenait donc son
    /// propre verrou non reentrant : deux appels depuis ring 3 --
    /// `waitset_add(w, w)` puis `waitset_poll(w)` -- suffisaient a paniquer le
    /// noyau (`SpinLock recursive acquisition`, reproduit en TCG par
    /// userland/tests/native-cycles-probe.c). Un cycle W1 -> W2 -> W1 donnait
    /// la meme chose sur deux verrous, et une chaine longue une recursion
    /// noyau non bornee. Detecter les cycles demanderait de parcourir un
    /// graphe sous verrous ; l'imbrication n'a aucun usage dans l'ABI v1 :
    /// on la refuse au point d'entree, avant toute mutation.
    pub fn add(&self, key: u64, object: Arc<Object>) -> Result<()> {
        if matches!(object.as_ref(), Object::WaitSet(_)) {
            return Err(Error::WrongType);
        }
        let mut watches = self.watches.lock();
        if watches.iter().any(|watch| watch.key == key) {
            return Err(Error::InvalidArgument);
        }
        watches.push(Watch { key, object });
        Ok(())
    }

    pub fn remove(&self, key: u64) -> Result<()> {
        let mut watches = self.watches.lock();
        let before = watches.len();
        watches.retain(|watch| watch.key != key);
        if watches.len() == before { Err(Error::NotFound) } else { Ok(()) }
    }

    pub fn poll(&self, cap: usize) -> Vec<(u64, Signals)> {
        let watches = self.watches.lock();
        let mut ready = Vec::new();
        for watch in watches.iter() {
            let signals = watch.object.signals();
            if !signals.is_empty() {
                ready.push((watch.key, signals));
                if ready.len() == cap { break; }
            }
        }
        ready
    }

    pub fn signals(&self) -> Signals {
        if self.watches.lock().iter().any(|watch| !watch.object.signals().is_empty()) {
            Signals::READABLE
        } else {
            Signals::NONE
        }
    }
}

impl Default for WaitSet {
    fn default() -> Self { Self::new() }
}
