//! Prefixe des lignes du journal serie : heure et charge de la machine.
//!
//! ## Pourquoi
//!
//! Un journal de boot sans horodatage dit *ce qui* s'est passe, jamais *quand*
//! ni *combien ca a coute*. C'est suffisant tant qu'on cherche une erreur ; ca
//! ne l'est plus des qu'on cherche une lenteur — quel programme a mange la
//! seconde qui vient de passer, a quel moment la memoire a double. Le prefixe
//! rend le journal lisible comme un moniteur systeme : moins bavard qu'un
//! gestionnaire des taches, mais qu'on suit en continu.
//!
//!     [18:51:48][ 12%:  5%:  6%] gui: client /bo-navigateur pid=5 ...
//!      \______/  \_/  \_/  \_/
//!       heure    cpu  ram  disque
//!
//! La charge processeur et le remplissage du RAMFS sont ceux de la barre du haut
//! du bureau, aux memes sources. La memoire, non : voir plus bas.
//!
//! ## Ou il s'insere
//!
//! Dans `drivers::serial`, au **debut de chaque ligne** et nulle part ailleurs.
//! Un `serial_print!` qui construit une ligne en plusieurs morceaux n'en recoit
//! donc qu'un seul, au premier morceau — la solution naive, qui prefixe chaque
//! appel, hachait les lignes composees.
//!
//! ## Rien de ce qui est verrouille
//!
//! Le prefixe s'ecrit sur le chemin de **toute** ligne de journal, y compris
//! celle d'une panique et celle d'un echec d'allocation. Il ne peut donc prendre
//! aucun verrou : demander sa taille a l'allocateur depuis un message emis alors
//! que l'allocateur est deja verrouille bloquerait la machine a l'endroit precis
//! ou l'on regarde pour comprendre pourquoi elle va mal — et sans rien afficher.
//!
//! Les trois mesures lisent donc des variables statiques et rien d'autre :
//! charge processeur (compteurs du timer), frames physiques (allocateur de
//! pages), nœuds du RAMFS (tableau fixe). Aucune allocation, aucun verrou,
//! appelable depuis une interruption.
//!
//! La memoire affichee est celle des **frames physiques**, pas celle du tas. Ce
//! n'est pas le meme chiffre que la barre du bureau, et c'est voulu : un
//! navigateur projette des centaines de mebioctets de Qt et de Python par
//! `mmap`, qui ne passent jamais par le tas. Le tas dirait 5 % pendant que la
//! machine se remplit.
//!
//! ## Avant que la machine sache compter
//!
//! Les premieres lignes sortent avant le tas, avant le RAMFS, avant le timer.
//! Leur demander une mesure de charge ferait allouer, ou diviser par zero, dans
//! le journal — c'est-a-dire echouer a l'endroit precis ou l'on regarde pour
//! comprendre pourquoi ca echoue. Tant que [`demarre`] n'a pas ete appele, les
//! trois champs affichent `--`, et seule l'heure — qui ne demande que deux
//! acces au CMOS — est reelle.

use crate::arch::x86_64::rtc;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Les mesures de charge sont-elles disponibles ?
static PRET: AtomicBool = AtomicBool::new(false);
/// Sequences ANSI actives ?
static COULEURS: AtomicBool = AtomicBool::new(true);
/// Derniere mesure, et le tick auquel elle a ete prise.
static CACHE: AtomicU32 = AtomicU32::new(0);
static CACHE_TICK: AtomicU64 = AtomicU64::new(0);

/// Duree de validite d'une mesure, en ticks (1 tick = 1 ms).
///
/// Relire le CMOS et compter les nœuds du RAMFS a chaque ligne couterait plus
/// cher que la ligne elle-meme pendant un boot bavard. Un quart de seconde est
/// bien en dessous de ce qu'un œil distingue et bien au-dessus du cout.
const VALIDITE_TICKS: u64 = 250;

/// Autorise le prefixe a mesurer la charge.
///
/// A appeler quand le tas, le RAMFS et le timer sont prets — pas avant.
pub fn demarre() {
    PRET.store(true, Ordering::Release);
    crate::serial_println!(
        "[journal] prefixe des lignes : [heure][cpu%:memoire physique%:nœuds RAMFS%]"
    );
}

/// Active ou coupe les sequences ANSI.
///
/// Elles supposent un terminal qui les comprend. `run.ps1` sort sur la console
/// de Windows, ou c'est le cas depuis Windows 10 — mais un journal redirige
/// vers un fichier, lui, gagne a rester en texte pur.
pub fn pose_couleurs(actif: bool) {
    COULEURS.store(actif, Ordering::Relaxed);
}

pub fn couleurs() -> bool {
    COULEURS.load(Ordering::Relaxed)
}

// --- Couleurs ---------------------------------------------------------------

const RESET: &str = "\x1b[0m";
const GRIS: &str = "\x1b[90m";
const CYAN: &str = "\x1b[36m";
const VERT: &str = "\x1b[32m";
const JAUNE: &str = "\x1b[33m";
const ROUGE: &str = "\x1b[31m";

/// Couleur d'un pourcentage : verte tant que c'est confortable, jaune quand ca
/// se remplit, rouge quand ca compte. Les seuils sont ceux qu'on regarde.
fn teinte(pourcentage: u8) -> &'static str {
    match pourcentage {
        0..=49 => VERT,
        50..=79 => JAUNE,
        _ => ROUGE,
    }
}

fn ecris(texte: &str) {
    crate::serial_print!("{}", texte);
}

fn ecris_couleur(couleur: &str) {
    if couleurs() {
        ecris(couleur);
    }
}

// --- Mesures ----------------------------------------------------------------

/// (cpu %, ram %, disque %), mise en cache un quart de seconde.
fn charge() -> (u8, u8, u8) {
    if !PRET.load(Ordering::Acquire) {
        return (u8::MAX, u8::MAX, u8::MAX);
    }
    let maintenant = crate::kernel::timer::ticks();
    let cache_tick = CACHE_TICK.load(Ordering::Relaxed);
    if maintenant.wrapping_sub(cache_tick) < VALIDITE_TICKS && cache_tick != 0 {
        let packed = CACHE.load(Ordering::Relaxed);
        return (packed as u8, (packed >> 8) as u8, (packed >> 16) as u8);
    }

    let cpu = crate::kernel::timer::cpu_load_pct();
    let (utilise, total) = crate::kernel::vmm::frame_stats_relaxed();
    let ram = if total > 0 { (utilise * 100 / total) as u8 } else { 0 };
    let nœuds = crate::fs::ramfs::used_nodes_relaxed();
    let disque = if crate::fs::ramfs::MAX_NODES > 0 {
        (nœuds * 100 / crate::fs::ramfs::MAX_NODES) as u8
    } else {
        0
    };

    let packed = cpu as u32 | (ram as u32) << 8 | (disque as u32) << 16;
    CACHE.store(packed, Ordering::Relaxed);
    CACHE_TICK.store(maintenant.max(1), Ordering::Relaxed);
    (cpu, ram, disque)
}

fn ecris_pourcentage(valeur: u8) {
    if valeur == u8::MAX {
        ecris_couleur(GRIS);
        ecris(" --");
        return;
    }
    ecris_couleur(teinte(valeur));
    crate::serial_print!("{:3}", valeur);
}

/// Ecrit le prefixe d'une ligne. Appele par `drivers::serial`.
pub fn ecris_prefixe() {
    let heure = rtc::now();
    ecris_couleur(GRIS);
    ecris("[");
    ecris_couleur(CYAN);
    crate::serial_print!("{:02}:{:02}:{:02}", heure.hour, heure.minute, heure.second);
    ecris_couleur(GRIS);
    ecris("][");

    let (cpu, ram, disque) = charge();
    ecris_pourcentage(cpu);
    ecris_couleur(GRIS);
    ecris("%:");
    ecris_pourcentage(ram);
    ecris_couleur(GRIS);
    ecris("%:");
    ecris_pourcentage(disque);
    ecris_couleur(GRIS);
    ecris("%] ");
    ecris_couleur(RESET);
}
