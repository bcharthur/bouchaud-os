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

    // LE SUSPECT PRINCIPAL de l'arret du run #347 : quand le script autorun
    // se termine, la session s'eteint. Vu de l'exterieur, cela ressemble a une
    // mort spontanee ; c'est une fin nominale qui ne se nommait pas.
    //
    // BOUCHAUD_C63_LE_RUNNER_PEUT_POSSEDER_LA_DUREE_DE_VIE
    //
    // Un banc qui mesure une page asynchrone de plus de cent secondes ne peut
    // pas dependre de la fin de l'autorun. Au run 35907201865, `desktop` est
    // revenu prematurement, l'autorun s'est termine, et la machine s'est
    // eteinte alors que le premier worker n'avait pas franchi son
    // constructeur. Le banc a cru l'experience finie ; elle n'avait pas
    // commence.
    //
    // Le fichier `/garde-vm-vivante` rend la duree de vie a l'hote. Ce n'est
    // PAS une facon de rendre vert un navigateur mort : l'arret prematuré de
    // `desktop` reste visible (`AUTORUN_DESKTOP_RETURN`, `RUN_NOYAU_RETOUR`,
    // `PROCESS_EXIT`), et le banc d'ordre ECHOUE quand il le voit avant son
    // verdict. Le filet sert a OBSERVER la panne, pas a la taire.
    let garde_vivante = {
        let fs = crate::fs::ramfs::fs();
        fs.resolve("/garde-vm-vivante", 0).is_some()
    };
    if garde_vivante {
        crate::serial_println!(
            "AUTORUN_FIN_SANS_EXTINCTION t={} statut={} raison=garde-vm-vivante",
            crate::kernel::timer::monotonic_ms(),
            status,
        );
        // La machine reste debout : c'est l'hote qui decidera de la tuer, apres
        // avoir lu son verdict ou epuise son propre plafond.
        //
        // `hlt` et non `sleep_ticks` : l'autorun s'execute dans le contexte
        // d'amorcage, ou `CURRENT` vaut `NO_TASK`. Dormir y demande une tache
        // et panique -- « task: aucune tache active sur ce CPU », verifie.
        // `hlt` laisse les interruptions reveiller le coeur, donc les autres
        // processeurs et les fils noyau continuent normalement.
        crate::arch::x86_64::cpu::halt_loop()
    }

    power::shutdown_avec_raison(
        if status == 0 { power::EXIT_OK } else { power::EXIT_FAIL },
        "autorun_termine",
    )
}
