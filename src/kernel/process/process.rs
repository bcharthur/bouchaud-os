//! Lightweight kernel process registry.
//!
//! P0-NG1 removes the historical `static mut` table. This registry is not the
//! scheduler's task table; it is the shell/system process catalogue, now with
//! explicit ownership and a ranked lock instead of implicit BKL serialization.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use crate::kernel::sync::{lockdep::LockClass, RankedSpinLock};

#[derive(Clone, Copy, PartialEq)]
pub enum State { Running, Sleeping, Zombie }
impl State {
    pub fn as_str(self) -> &'static str {
        match self { State::Running => "running", State::Sleeping => "sleeping", State::Zombie => "zombie" }
    }
}

#[derive(Clone)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub uid: u16,
    pub state: State,
}

struct Registry { rows: Vec<Process>, next_pid: u32 }
static TABLE: RankedSpinLock<Registry> =
    RankedSpinLock::new(LockClass::ProcessTable, Registry { rows: Vec::new(), next_pid: 1 });

pub fn init() {
    {
        let mut t = TABLE.lock();
        t.rows.clear();
        t.next_pid = 1;
    }
    spawn("init", 0);
    spawn("desktop", 0);
    spawn("shell", 0);
}

pub fn spawn(name: &str, uid: u16) -> u32 {
    let mut t = TABLE.lock();
    let pid = t.next_pid;
    t.next_pid = t.next_pid.wrapping_add(1).max(1);
    t.rows.push(Process { pid, name: name.to_string(), uid, state: State::Running });
    pid
}

pub fn kill(pid: u32) -> bool {
    if pid <= 1 { return false; }
    let mut t = TABLE.lock();
    let before = t.rows.len();
    t.rows.retain(|p| p.pid != pid);
    t.rows.len() != before
}

pub fn set_state(pid: u32, state: State) {
    let mut t = TABLE.lock();
    if let Some(p) = t.rows.iter_mut().find(|p| p.pid == pid) { p.state = state; }
}

pub fn count() -> usize { TABLE.lock().rows.len() }

pub fn print_table() {
    let snapshot = TABLE.lock().rows.clone();
    crate::println!("  PID  UID  ETAT      NOM");
    for p in snapshot.iter() {
        crate::println!("  {:>3}  {:>3}  {:<8}  {}", p.pid, p.uid, p.state.as_str(), p.name);
    }
}
