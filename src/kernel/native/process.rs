//! Native process object: a capability on a child launched by
//! `bo_process_spawn`.
//!
//! BOUCHAUD_NATIVE_PROCESS_OBJECT_V1
//!
//! A supervisor must learn that its service died without polling a PID table
//! and without depending on SIGCHLD. The process handle carries that fact as a
//! signal, `TERMINATED`, observable through a wait set like any other object.
//!
//! Lost-exit rule. The death can happen before the handle exists (a service
//! that crashes on its first instruction, on another CPU, while `spawn` is
//! still returning). `suit` therefore REGISTERS first and only then reads the
//! process state: a death after registration is delivered by `note_exit`, a
//! death before it is seen by the read. There is no window in which neither
//! path sees it.
//!
//! Reaping. A spawned child is a real child of its launcher (parent pid set),
//! so `wait4` keeps working. When the LAST handle on a terminated child is
//! closed -- including when the launcher itself dies and its handle table is
//! released -- the zombie is collected, so a supervisor that never calls
//! `wait4` does not accumulate zombies.

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crate::kernel::sync::SpinLock;

use super::abi::types::Signals;

pub struct ProcessState {
    pid: u32,
    terminated: AtomicBool,
    exit_code: AtomicI32,
}

pub struct ProcessRef {
    state: Arc<ProcessState>,
}

static WATCHED: SpinLock<Vec<Weak<ProcessState>>> = SpinLock::new(Vec::new());

/// Exit code reported when the process vanished before it could be read.
pub const EXIT_CODE_UNKNOWN: i32 = -1;

impl ProcessState {
    fn mark(&self, code: i32) {
        // Code first, then the flag (Release): a reader that sees TERMINATED
        // (Acquire) sees the code.
        self.exit_code.store(code, Ordering::Relaxed);
        self.terminated.store(true, Ordering::Release);
    }
}

/// Start watching `pid` and return the object to install in a handle table.
pub fn suit(pid: u32) -> ProcessRef {
    let state = Arc::new(ProcessState {
        pid,
        terminated: AtomicBool::new(false),
        exit_code: AtomicI32::new(0),
    });
    {
        let mut watched = WATCHED.lock();
        watched.retain(|weak| weak.strong_count() > 0);
        watched.push(Arc::downgrade(&state));
    }
    // After registration: a death that already happened is read here.
    match crate::kernel::task::process_by_pid(pid) {
        None => state.mark(EXIT_CODE_UNKNOWN),
        Some(process) => {
            let lifecycle = process.lifecycle.lock();
            if lifecycle.zombie {
                state.mark(lifecycle.exit_code);
            }
        }
    }
    ProcessRef { state }
}

/// Called once per process, at the transition to zombie (voluntary exit and
/// forced kill alike). Cost when nobody watches: one lock and an empty scan.
pub fn note_exit(pid: u32, code: i32) {
    let mut hits: Vec<Arc<ProcessState>> = Vec::new();
    {
        let mut watched = WATCHED.lock();
        watched.retain(|weak| match weak.upgrade() {
            Some(state) => {
                if state.pid == pid { hits.push(state); }
                true
            }
            None => false,
        });
    }
    let any = !hits.is_empty();
    for state in hits { state.mark(code); }
    if any { super::activite::publie(); }
}

impl ProcessRef {
    pub fn pid(&self) -> u32 { self.state.pid }

    pub fn terminated(&self) -> bool { self.state.terminated.load(Ordering::Acquire) }

    pub fn exit_code(&self) -> i32 { self.state.exit_code.load(Ordering::Relaxed) }

    pub fn signals(&self) -> Signals {
        if self.terminated() { Signals::TERMINATED } else { Signals::NONE }
    }
}

impl Drop for ProcessRef {
    fn drop(&mut self) {
        // Only the object is dropped here, never a lock-holding caller's
        // state: handle tables release objects outside their own lock.
        if self.terminated() {
            let pid = self.state.pid;
            let zombie = crate::kernel::task::process_by_pid(pid)
                .map(|p| p.lifecycle.lock().zombie)
                .unwrap_or(false);
            if zombie {
                crate::kernel::task::collect_child(pid);
            }
        }
    }
}
