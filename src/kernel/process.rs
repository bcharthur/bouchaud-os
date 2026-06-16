//! Modele de processus.
//!
//! Etape actuelle : table de processus logiques (PID, nom, etat, proprietaire).
//! Les "processus" tournent encore dans le noyau (pas d'isolation user-mode ni
//! de changement de contexte). C'est le socle pour `ps`/`kill` et, plus tard,
//! de vrais processus avec pile, espace memoire et ordonnancement preemptif.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq)]
pub enum State {
    Running,
    Sleeping,
    Zombie,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Running => "running",
            State::Sleeping => "sleeping",
            State::Zombie => "zombie",
        }
    }
}

pub struct Process {
    pub pid: u32,
    pub name: String,
    pub uid: u16,
    pub state: State,
}

static mut TABLE: Option<Vec<Process>> = None;
static mut NEXT_PID: u32 = 1;

fn table() -> &'static mut Vec<Process> {
    unsafe {
        if TABLE.is_none() { TABLE = Some(Vec::new()); }
        TABLE.as_mut().unwrap()
    }
}

/// Cree les processus systeme de base.
pub fn init() {
    table().clear();
    unsafe { NEXT_PID = 1; }
    spawn("init", 0);
    spawn("desktop", 0);
    spawn("shell", 0);
}

/// Cree un processus logique, renvoie son PID.
pub fn spawn(name: &str, uid: u16) -> u32 {
    let pid = unsafe { let p = NEXT_PID; NEXT_PID += 1; p };
    table().push(Process { pid, name: name.to_string(), uid, state: State::Running });
    pid
}

/// Termine un processus (passe en zombie puis le retire).
pub fn kill(pid: u32) -> bool {
    let t = table();
    if pid <= 1 { return false; } // init protege
    let before = t.len();
    t.retain(|p| p.pid != pid);
    t.len() != before
}

/// Change l'etat d'un processus.
pub fn set_state(pid: u32, state: State) {
    for p in table().iter_mut() {
        if p.pid == pid { p.state = state; }
    }
}

/// Nombre de processus.
pub fn count() -> usize {
    table().len()
}

/// Affiche la table des processus (commande `ps`).
pub fn print_table() {
    crate::println!("  PID  UID  ETAT      NOM");
    for p in table().iter() {
        crate::println!("  {:>3}  {:>3}  {:<8}  {}", p.pid, p.uid, p.state.as_str(), p.name);
    }
}
