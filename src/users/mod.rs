//! Base d'utilisateurs dynamique et session courante.
//!
//! Table a taille fixe (aucune allocation) : on peut ajouter, supprimer, lister
//! et modifier des comptes a l'execution (`useradd`, `userdel`, `passwd`...).
//! Par defaut deux comptes existent : `root` (uid 0) et `guest` (uid 1000).

pub mod motdepasse;

use motdepasse::{Empreinte, ITERATIONS_DEFAUT, LONGUEUR_SEL};

const MAX_USERS: usize = 16;
const NAME_LEN: usize = 32;
const HOME_LEN: usize = 48;

#[derive(Copy, Clone)]
struct UserRec {
    used: bool,
    name: [u8; NAME_LEN],
    name_len: usize,
    /// L'empreinte du mot de passe, JAMAIS le mot de passe.
    empreinte: Empreinte,
    home: [u8; HOME_LEN],
    home_len: usize,
    uid: u16,
    gid: u16,
}

impl UserRec {
    const fn empty() -> Self {
        Self {
            used: false,
            name: [0; NAME_LEN], name_len: 0,
            empreinte: Empreinte::verrouille(),
            home: [0; HOME_LEN], home_len: 0,
            uid: 0, gid: 0,
        }
    }

    fn name_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.name[..self.name_len]) }
    }

    fn home_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.home[..self.home_len]) }
    }

    fn mot_de_passe_correct(&self, pass: &str) -> bool {
        self.empreinte.verifie(pass)
    }
}

static mut USERS: [UserRec; MAX_USERS] = [UserRec::empty(); MAX_USERS];

/// Session courante : identifiee par l'uid de l'utilisateur connecte.
pub struct Session {
    uid: u16,
}

static mut SESSION: Session = Session { uid: 0 };

fn copy_into(dst: &mut [u8], src: &str) -> usize {
    let bytes = src.as_bytes();
    let n = if bytes.len() > dst.len() { dst.len() } else { bytes.len() };
    dst[..n].copy_from_slice(&bytes[..n]);
    n
}

fn slot_by_name(name: &str) -> Option<usize> {
    unsafe {
        for i in 0..MAX_USERS {
            if USERS[i].used && USERS[i].name_str() == name {
                return Some(i);
            }
        }
    }
    None
}

fn slot_by_uid(uid: u16) -> Option<usize> {
    unsafe {
        for i in 0..MAX_USERS {
            if USERS[i].used && USERS[i].uid == uid {
                return Some(i);
            }
        }
    }
    None
}

fn free_slot() -> Option<usize> {
    unsafe {
        for i in 0..MAX_USERS {
            if !USERS[i].used { return Some(i); }
        }
    }
    None
}

/// Cree un compte de bas niveau (sans verification de droits).
fn create(name: &str, pass: &str, uid: u16, home: &str) -> Result<u16, &'static str> {
    let empreinte = Empreinte::nouvelle(pass, sel_neuf(), ITERATIONS_DEFAUT);
    installe(name, empreinte, uid, home)
}

/// Cree un compte SANS mot de passe utilisable.
///
/// C'est l'etat des comptes du systeme au demarrage : ils existent, ils ont un
/// uid et un repertoire, et rien ne les ouvre tant que `passwd` n'a pas pose
/// de mot de passe.
fn create_verrouille(name: &str, uid: u16, home: &str) -> Result<u16, &'static str> {
    installe(name, Empreinte::verrouille(), uid, home)
}

fn installe(
    name: &str,
    empreinte: Empreinte,
    uid: u16,
    home: &str,
) -> Result<u16, &'static str> {
    if name.is_empty() { return Err("nom vide"); }
    if name.len() > NAME_LEN { return Err("nom trop long"); }
    if slot_by_name(name).is_some() { return Err("utilisateur deja existant"); }
    let slot = free_slot().ok_or("table utilisateurs pleine")?;
    unsafe {
        let u = &mut USERS[slot];
        *u = UserRec::empty();
        u.used = true;
        u.uid = uid;
        u.gid = uid;
        u.name_len = copy_into(&mut u.name, name);
        u.empreinte = empreinte;
        u.home_len = copy_into(&mut u.home, home);
    }
    Ok(uid)
}

/// Initialise la base avec les comptes par defaut : root et guest.
///
/// # Aucun mot de passe par defaut
///
/// Les deux comptes naissaient avec leur propre nom comme mot de passe --
/// `root:root`, `guest:guest`. Un identifiant par defaut connu n'est pas une
/// commodite, c'est une porte : il est le meme sur toutes les installations, il
/// n'est presque jamais change, et il suffit a lui seul pour prendre l'uid 0.
///
/// Les comptes naissent donc VERROUILLES. Aucun mot de passe ne les ouvre, pas
/// meme la chaine vide, tant que `passwd` n'en a pas pose un. C'est un etat sur
/// par defaut : refuser est toujours recuperable, accepter ne l'est pas.
pub fn init() {
    unsafe { USERS = [UserRec::empty(); MAX_USERS]; }
    let _ = create_verrouille("root", 0, "/");
    let _ = create_verrouille("guest", 1000, "/home/guest");
    unsafe { SESSION.uid = 0; }
}

/// Prochain uid libre (>= 1001) pour un nouvel utilisateur.
fn next_uid() -> u16 {
    let mut max = 1000u16;
    unsafe {
        for i in 0..MAX_USERS {
            if USERS[i].used && USERS[i].uid > max { max = USERS[i].uid; }
        }
    }
    max + 1
}

/// Ajoute un utilisateur (home = /home/<nom>). Renvoie son uid.
pub fn add_user(name: &str, pass: &str) -> Result<u16, &'static str> {
    let uid = next_uid();
    let mut home = [0u8; HOME_LEN];
    let mut len = copy_into(&mut home, "/home/");
    len += copy_into(&mut home[len..], name);
    let home_str = unsafe { core::str::from_utf8_unchecked(&home[..len]) };
    create(name, pass, uid, home_str)
}

/// Supprime un utilisateur par nom. root (uid 0) ne peut pas etre supprime.
pub fn remove_user(name: &str) -> Result<(), &'static str> {
    let slot = slot_by_name(name).ok_or("utilisateur inconnu")?;
    unsafe {
        if USERS[slot].uid == 0 { return Err("impossible de supprimer root"); }
        USERS[slot].used = false;
    }
    Ok(())
}

/// Change le mot de passe d'un utilisateur.
///
/// Un sel NEUF a chaque changement : reutiliser l'ancien laisserait un
/// attaquant qui a vu les deux empreintes savoir que le mot de passe a change
/// sans changer de sel, et reutiliser le travail deja fait sur le premier.
pub fn set_password(name: &str, pass: &str) -> Result<(), &'static str> {
    let slot = slot_by_name(name).ok_or("utilisateur inconnu")?;
    let empreinte = Empreinte::nouvelle(pass, sel_neuf(), ITERATIONS_DEFAUT);
    unsafe {
        USERS[slot].empreinte = empreinte;
    }
    Ok(())
}

/// Verrouille un compte : plus aucun mot de passe ne l'ouvre.
pub fn verrouille(name: &str) -> Result<(), &'static str> {
    let slot = slot_by_name(name).ok_or("utilisateur inconnu")?;
    unsafe {
        USERS[slot].empreinte = Empreinte::verrouille();
    }
    Ok(())
}

/// Ce compte est-il sans mot de passe utilisable ?
pub fn est_verrouille(name: &str) -> bool {
    match slot_by_name(name) {
        Some(slot) => unsafe { USERS[slot].empreinte.est_verrouille() },
        None => true,
    }
}

/// Aucun compte n'a de mot de passe utilisable.
///
/// Vrai au tout premier demarrage. L'ecran de connexion s'en sert pour
/// proposer l'enrolement plutot que de refuser indefiniment : sans mot de
/// passe par defaut ET sans enrolement, la machine serait inaccessible.
pub fn aucun_mot_de_passe_defini() -> bool {
    unsafe {
        for index in 0..MAX_USERS {
            if USERS[index].used && !USERS[index].empreinte.est_verrouille() {
                return false;
            }
        }
    }
    true
}

/// Un sel tire de la source d'aleas du noyau.
fn sel_neuf() -> [u8; LONGUEUR_SEL] {
    let mut sel = [0u8; LONGUEUR_SEL];
    crate::net::security::tls::rng::fill(&mut sel);
    sel
}

/// Verifie un couple (nom, mot de passe). Renvoie l'uid en cas de succes.
pub fn authenticate(name: &str, pass: &str) -> Option<u16> {
    let slot = slot_by_name(name)?;
    unsafe {
        if USERS[slot].mot_de_passe_correct(pass) { Some(USERS[slot].uid) } else { None }
    }
}

/// Resout un nom d'utilisateur en uid.
pub fn uid_of_name(name: &str) -> Option<u16> {
    slot_by_name(name).map(|s| unsafe { USERS[s].uid })
}

/// Renvoie le nom associe a un uid (ou "?" si inconnu).
pub fn name_of_uid(uid: u16) -> &'static str {
    match slot_by_uid(uid) {
        Some(s) => unsafe { USERS[s].name_str() },
        None => "?",
    }
}

/// Renvoie le repertoire d'accueil associe a un uid (ou "/").
pub fn home_of_uid(uid: u16) -> &'static str {
    match slot_by_uid(uid) {
        Some(s) => unsafe { USERS[s].home_str() },
        None => "/",
    }
}

/// Existe-t-il un compte pour cet uid ?
pub fn uid_exists(uid: u16) -> bool {
    slot_by_uid(uid).is_some()
}

/// Liste tous les comptes (commande `users`).
pub fn list() {
    unsafe {
        for i in 0..MAX_USERS {
            if USERS[i].used {
                crate::println!("{}:x:{}:{}:{}", USERS[i].name_str(), USERS[i].uid, USERS[i].gid, USERS[i].home_str());
            }
        }
    }
}

/// Cree dans le RAMFS les repertoires d'accueil manquants (mode 700).
pub fn create_home_dirs() {
    let fs = crate::fs::ramfs::fs();
    unsafe {
        for i in 0..MAX_USERS {
            if !USERS[i].used { continue; }
            let home = USERS[i].home_str();
            let uid = USERS[i].uid;
            if home == "/" { continue; }
            if fs.resolve(home, 0).is_some() { continue; }
            if let Some((parent, name)) = fs.resolve_parent_name(home, 0) {
                if let Ok(idx) = fs.mkdir_at(parent, name) {
                    fs.nodes[idx].uid = uid;
                    fs.nodes[idx].gid = uid;
                    fs.nodes[idx].mode = 0o700;
                }
            }
        }
    }
}

// --- Session ---------------------------------------------------------------

impl Session {
    pub fn uid(&self) -> u16 { self.uid }
    pub fn gid(&self) -> u16 { self.uid }
    pub fn is_root(&self) -> bool { self.uid == 0 }
    pub fn username(&self) -> &'static str { name_of_uid(self.uid) }
    pub fn home(&self) -> &'static str { home_of_uid(self.uid) }
    pub fn set_uid(&mut self, uid: u16) { self.uid = uid; }
}

/// Accede a la session globale courante.
pub fn session() -> &'static mut Session {
    unsafe { &mut SESSION }
}
