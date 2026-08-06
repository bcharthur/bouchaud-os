//! Mode non interactif : execute un script au demarrage, puis eteint.
//!
//! Les trois sondes (`ring3-selftest`, `qpa-probe`, `posix-probe`) verifient
//! plus de cent points de l'ABI et ont trouve la plupart des defauts corriges
//! jusqu'ici — mais elles ne s'executaient que si quelqu'un pensait a les
//! lancer, a la main, en enchainant six commandes. Ce module rend le scenario
//! reproductible : si le disque de donnees apporte un fichier `/autorun`, le
//! noyau le joue au lieu d'ouvrir une session, recopie tout sur COM1, et rend
//! son verdict a l'hote par le code de sortie de l'emulateur.
//!
//! Rien n'est reserve aux tests dans ce mecanisme : c'est l'equivalent d'un
//! `init` scripte, et il sert aussi bien a demarrer une machine sur une tache
//! precise qu'a la faire s'auto-verifier.

use crate::drivers::vga;
use crate::fs::ramfs::{self, NodeKind};
use crate::kernel::power;
use crate::users;
use alloc::string::String;

/// Chemin du script joue au demarrage, s'il existe.
pub const SCRIPT_PATH: &str = "/autorun";

/// Marqueurs encadrant la sortie, pour que l'hote sache ou commence le
/// scenario et ou il finit — le journal du noyau les entoure des deux cotes.
const BEGIN: &str = "=== AUTORUN DEBUT ===";
const END: &str = "=== AUTORUN FIN ===";

/// Le disque a-t-il apporte un script de demarrage ?
pub fn present() -> bool {
    read_script().is_some()
}

/// Lit le script s'il existe et qu'il s'agit bien d'un fichier.
fn read_script() -> Option<String> {
    let fs = ramfs::fs();
    let idx = fs.resolve(SCRIPT_PATH, 0)?;
    if fs.nodes[idx].kind != NodeKind::File {
        return None;
    }
    Some(fs.nodes[idx].content_str())
}

/// Joue le script de demarrage puis eteint la machine. Ne rend la main que
/// s'il n'y a pas de script — auquel cas le noyau poursuit vers la connexion.
pub fn run_if_present() {
    let script = match read_script() {
        Some(s) => s,
        None => return,
    };

    // Pas de connexion en mode non interactif : le script s'execute en root,
    // comme un `init`. C'est aussi ce qui lui permet de lire tout le disque.
    users::session().set_uid(0);

    // La sortie doit atteindre l'hote pour qu'il puisse l'analyser. On recopie
    // au fil de l'eau : si le noyau panique au milieu du scenario, tout ce qui
    // precede la panique est deja parti.
    vga::set_serial_mirror(true);
    crate::println!("{}", BEGIN);

    let status = crate::shell::run_batch(&script);

    crate::println!("{} statut={}", END, status);
    vga::set_serial_mirror(false);

    power::shutdown(if status == 0 { power::EXIT_OK } else { power::EXIT_FAIL })
}
