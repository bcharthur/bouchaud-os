use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::kernel::sync::SpinLock;

use super::super::abi::types::{Error, HandleId, ObjectKind, Result, Rights};
use super::super::object::Object;

pub const MAX_HANDLES_PER_PROCESS: usize = 4096;

#[derive(Clone)]
pub struct Entry {
    pub object: Arc<Object>,
    pub rights: Rights,
}

struct Slot {
    generation: u32,
    entry: Option<Entry>,
}

struct Inner {
    slots: Vec<Slot>,
    live: usize,
}

pub struct HandleTable {
    inner: SpinLock<Inner>,
}

impl HandleTable {
    pub fn new() -> Self {
        Self { inner: SpinLock::new(Inner { slots: Vec::new(), live: 0 }) }
    }

    fn next_generation(previous: u32) -> u32 {
        let next = previous.wrapping_add(1) & 0x7fff_ffff;
        if next == 0 { 1 } else { next }
    }

    fn insert_locked(inner: &mut Inner, entry: Entry) -> Result<HandleId> {
        if inner.live >= MAX_HANDLES_PER_PROCESS { return Err(Error::NoSpace); }

        for (slot, cell) in inner.slots.iter_mut().enumerate() {
            if cell.entry.is_none() {
                cell.generation = Self::next_generation(cell.generation);
                cell.entry = Some(entry);
                inner.live += 1;
                return Ok(HandleId::new(slot as u32 + 1, cell.generation));
            }
        }

        if inner.slots.len() >= MAX_HANDLES_PER_PROCESS { return Err(Error::NoSpace); }
        inner.slots.push(Slot { generation: 1, entry: Some(entry) });
        inner.live += 1;
        Ok(HandleId::new(inner.slots.len() as u32, 1))
    }

    pub fn insert(&self, object: Arc<Object>, rights: Rights) -> Result<HandleId> {
        Self::insert_locked(&mut self.inner.lock(), Entry { object, rights })
    }

    pub fn insert_many(&self, entries: &[Entry]) -> Result<Vec<HandleId>> {
        let mut inner = self.inner.lock();
        if inner.live.saturating_add(entries.len()) > MAX_HANDLES_PER_PROCESS {
            return Err(Error::NoSpace);
        }

        let mut installed = Vec::with_capacity(entries.len());
        for entry in entries.iter().cloned() {
            match Self::insert_locked(&mut inner, entry) {
                Ok(id) => installed.push(id),
                Err(error) => {
                    // Rollback is safe under the same table lock.
                    for id in installed.drain(..) {
                        let _ = Self::close_locked(&mut inner, id);
                    }
                    return Err(error);
                }
            }
        }
        Ok(installed)
    }

    fn index(id: HandleId) -> Result<usize> {
        if !id.valid() || id.slot() == 0 { return Err(Error::BadHandle); }
        Ok(id.slot() as usize - 1)
    }

    pub fn lookup(&self, id: HandleId, required: Rights) -> Result<Entry> {
        let inner = self.inner.lock();
        let index = Self::index(id)?;
        let slot = inner.slots.get(index).ok_or(Error::BadHandle)?;
        if slot.generation != id.generation() { return Err(Error::BadHandle); }
        let entry = slot.entry.as_ref().ok_or(Error::BadHandle)?;
        if !entry.rights.contains(required) { return Err(Error::AccessDenied); }
        Ok(entry.clone())
    }

    pub fn lookup_kind(&self, id: HandleId, required: Rights, kind: ObjectKind) -> Result<Entry> {
        let entry = self.lookup(id, required)?;
        if entry.object.kind() != kind { return Err(Error::WrongType); }
        Ok(entry)
    }

    fn close_locked(inner: &mut Inner, id: HandleId) -> Result<()> {
        let index = Self::index(id)?;
        let slot = inner.slots.get_mut(index).ok_or(Error::BadHandle)?;
        if slot.generation != id.generation() || slot.entry.is_none() {
            return Err(Error::BadHandle);
        }
        slot.entry = None;
        inner.live = inner.live.saturating_sub(1);
        Ok(())
    }

    pub fn close(&self, id: HandleId) -> Result<()> {
        Self::close_locked(&mut self.inner.lock(), id)
    }

    pub fn duplicate(&self, id: HandleId, requested: Rights) -> Result<HandleId> {
        let source = self.lookup(id, Rights::DUP)?;
        if !requested.subset_of(source.rights) { return Err(Error::AccessDenied); }
        self.insert(Arc::clone(&source.object), requested)
    }

    pub fn export(&self, id: HandleId) -> Result<Entry> {
        let mut entry = self.lookup(id, Rights::TRANSFER)?;
        // The receiver may pass the object onward only if the sender was itself
        // allowed to transfer it; that bit is therefore preserved. No rights
        // are ever gained by IPC.
        entry.rights = entry.rights.intersection(entry.rights);
        Ok(entry)
    }

    pub fn count(&self) -> usize { self.inner.lock().live }

    pub fn available(&self) -> usize {
        MAX_HANDLES_PER_PROCESS.saturating_sub(self.inner.lock().live)
    }
}

impl Default for HandleTable {
    fn default() -> Self { Self::new() }
}
