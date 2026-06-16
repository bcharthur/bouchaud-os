//! Table de handles (descripteurs) — socle pour les futurs syscalls.
//!
//! Un handle est un entier opaque rendu a une application pour designer une
//! ressource noyau (fichier, fenetre, socket...). Modele minimal pour l'instant.

use alloc::vec::Vec;

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

static mut TABLE: Option<Vec<Handle>> = None;
static mut NEXT_ID: u32 = 1;

fn table() -> &'static mut Vec<Handle> {
    unsafe {
        if TABLE.is_none() { TABLE = Some(Vec::new()); }
        TABLE.as_mut().unwrap()
    }
}

/// Ouvre un nouveau handle pour un processus.
pub fn open(kind: HandleKind, owner_pid: u32) -> u32 {
    let id = unsafe { let i = NEXT_ID; NEXT_ID += 1; i };
    table().push(Handle { id, kind, owner_pid });
    id
}

/// Ferme un handle.
pub fn close(id: u32) {
    table().retain(|h| h.id != id);
}

/// Nombre de handles ouverts.
pub fn count() -> usize {
    table().len()
}
