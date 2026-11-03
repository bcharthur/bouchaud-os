use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::sync::SpinLock;

use super::table::HandleTable;

struct ProcessTable {
    pid: u32,
    table: Arc<HandleTable>,
}

static REGISTRY: SpinLock<Vec<ProcessTable>> = SpinLock::new(Vec::new());
static NATIVE_CALLS: AtomicU64 = AtomicU64::new(0);

pub fn table_for(pid: u32) -> Arc<HandleTable> {
    let mut registry = REGISTRY.lock();
    if let Some(row) = registry.iter().find(|row| row.pid == pid) {
        return Arc::clone(&row.table);
    }

    let table = Arc::new(HandleTable::new());
    registry.push(ProcessTable { pid, table: Arc::clone(&table) });
    table
}

/// Drop a dead process' handle table.
///
/// The table is detached under the registry lock and dropped AFTER it: the
/// objects it frees (channel endpoints, process objects that may reap a
/// zombie) must never run their destructors under the registry lock.
pub fn release_process(pid: u32) {
    let detached: Vec<Arc<HandleTable>> = {
        let mut registry = REGISTRY.lock();
        let mut out = Vec::new();
        registry.retain(|row| {
            if row.pid == pid {
                out.push(Arc::clone(&row.table));
                false
            } else {
                true
            }
        });
        out
    };
    for table in detached {
        table.clear();
    }
}

/// Opportunistic lifecycle reclamation.
///
/// Native handle tables are not stored in `Process` yet, so the ABI owns their
/// lifecycle explicitly. Every 256 native calls it compares the registry to the
/// scheduler process registry and drops tables for processes that no longer
/// exist. This avoids a permanent per-PID leak without adding a new field to
/// every Process constructor in the compatibility layer.
pub fn maintenance() {
    let call = NATIVE_CALLS.fetch_add(1, Ordering::Relaxed);
    if call & 0xff != 0 { return; }

    let alive = crate::kernel::task::processes();
    let mut pids = Vec::with_capacity(alive.len());
    for process in alive { pids.push(process.pid); }

    let detached: Vec<Arc<HandleTable>> = {
        let mut registry = REGISTRY.lock();
        let mut out = Vec::new();
        registry.retain(|row| {
            let alive = pids.iter().any(|pid| *pid == row.pid);
            if !alive { out.push(Arc::clone(&row.table)); }
            alive
        });
        out
    };
    for table in detached {
        table.clear();
    }
}

pub fn registered_processes() -> usize { REGISTRY.lock().len() }
