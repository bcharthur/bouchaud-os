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

    pub fn add(&self, key: u64, object: Arc<Object>) -> Result<()> {
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
