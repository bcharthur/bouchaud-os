//! Legacy handle catalogue.
//!
//! New native applications use `kernel::native::handle`, whose handles are
//! process-local, generational and rights-bearing.  This module keeps the old
//! shell/debug API source-compatible while removing its historical `static mut`
//! data race.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::kernel::sync::SpinLock;

#[derive(Clone, Copy, PartialEq)]
pub enum HandleKind {
    File,
    Window,
    Socket,
    Device,
}

#[derive(Clone, Copy)]
pub struct Handle {
    pub id: u32,
    pub kind: HandleKind,
    pub owner_pid: u32,
}

static TABLE: SpinLock<Vec<Handle>> = SpinLock::new(Vec::new());
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

/// Ouvre un handle legacy. Les nouveaux chemins doivent preferer
/// `kernel::native::handle`.
pub fn open(kind: HandleKind, owner_pid: u32) -> u32 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed).max(1);
    TABLE.lock().push(Handle { id, kind, owner_pid });
    id
}

pub fn close(id: u32) {
    TABLE.lock().retain(|handle| handle.id != id);
}

pub fn close_owner(owner_pid: u32) {
    TABLE.lock().retain(|handle| handle.owner_pid != owner_pid);
}

pub fn count() -> usize {
    TABLE.lock().len()
}
