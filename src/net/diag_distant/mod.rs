//! La pile de diagnostic : un canal d'enquete qui ne depend pas de l'enquete.
//!
//! # Ce que le releve exige
//!
//! ```text
//! com1=bus-flottant  serial_bytes=0
//! rx_packets=64  rx_cur=0  desc_nic=64  desc_cpu=0   (151 s)
//! blackbox : session 395 s, persistance 193 s, checkpoints 0
//! ```
//!
//! Serie absente, reception morte, persistance arretee. Le canal d'enquete ne
//! peut donc dependre :
//!
//!   - ni de COM1, qui n'existe pas sur cette machine ;
//!   - ni de DHCP, dont l'OFFER est une trame ENTRANTE -- c'est-a-dire la
//!     chose en panne ;
//!   - ni du disque.
//!
//! Il reste l'emission. Le releve montre `chip_cmd = RX_ENB | TX_ENB` et une
//! emission qui continue pendant toute la panne : la machine PARLE encore.
//! C'est sur cela, et sur une adresse link-local derivee de la MAC, que ce
//! module s'appuie.
//!
//! # Deux piles, une carte
//!
//! La pile normale sert le navigateur ; celle-ci sert le debugger. Elles
//! partagent la carte, donc il faut trancher qui recoit quoi AVANT que
//! quiconque voie la trame -- une pile TCP ne jette pas silencieusement, elle
//! repond `RST`. Voir `tri`, qui porte la regle et se contredit en test hote.

pub mod adresse;
pub mod tri;

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::net::file_trames::FileTrames;

/// Port TCP du debugger. Ce n'est PAS du SSH, et cela ne pretend pas l'etre.
pub const PORT_BRDP: u16 = 2222;
/// Port UDP de la telemetrie best-effort.
pub const PORT_TELEMETRIE: u16 = 2223;

static ACTIF: AtomicBool = AtomicBool::new(false);
static IP: AtomicU64 = AtomicU64::new(0);
static MAC: AtomicU64 = AtomicU64::new(0);

// Ce que le tri a decide, compte. Un canal d'enquete qui ne sait pas dire
// combien de trames il a vues ne peut pas distinguer « personne ne me parle »
// de « le tri les envoie ailleurs ».
static TRIEES: AtomicU64 = AtomicU64::new(0);
static POUR_DIAGNOSTIC: AtomicU64 = AtomicU64::new(0);
static POUR_NORMALE: AtomicU64 = AtomicU64::new(0);
static POUR_LES_DEUX: AtomicU64 = AtomicU64::new(0);
static JETEES: AtomicU64 = AtomicU64::new(0);

/// La file de la pile de diagnostic. Bornee, et abonnee seulement si le LAB
/// ecoute : le cas courant ne paie pas une copie par trame.
static mut FILE: FileTrames = FileTrames::neuve();

fn file() -> &'static mut FileTrames {
    unsafe { &mut *core::ptr::addr_of_mut!(FILE) }
}

fn empaquete_ip(ip: [u8; 4]) -> u64 {
    u32::from_be_bytes(ip) as u64
}

fn depaquete_ip(v: u64) -> [u8; 4] {
    (v as u32).to_be_bytes()
}

/// Ouvre le canal de diagnostic sur l'adresse derivee de cette MAC.
///
/// Idempotent. Rend l'adresse retenue.
pub fn ouvre(mac: [u8; 6]) -> [u8; 4] {
    let ip = adresse::depuis_mac(mac);
    IP.store(empaquete_ip(ip), Ordering::Relaxed);
    let mut m = 0u64;
    for octet in mac {
        m = (m << 8) | octet as u64;
    }
    MAC.store(m, Ordering::Relaxed);
    if !ACTIF.swap(true, Ordering::AcqRel) {
        file().abonne();
        crate::kernel::lab::emets(
            crate::kernel::lab::Categorie::Reseau,
            crate::kernel::lab::id::NET_LINK_LOCAL,
            [empaquete_ip(ip), adresse::PREFIXE_BITS as u64, 0, 0],
        );
    }
    ip
}

/// Ferme le canal. Le tri redevient exactement celui d'avant ce module.
pub fn ferme() {
    if ACTIF.swap(false, Ordering::AcqRel) {
        file().desabonne();
    }
}

pub fn actif() -> bool {
    ACTIF.load(Ordering::Relaxed)
}

/// L'adresse de diagnostic. `0.0.0.0` tant que le canal n'est pas ouvert.
pub fn ip() -> [u8; 4] {
    depaquete_ip(IP.load(Ordering::Relaxed))
}

pub fn mac() -> [u8; 6] {
    let m = MAC.load(Ordering::Relaxed);
    [
        (m >> 40) as u8,
        (m >> 32) as u8,
        (m >> 24) as u8,
        (m >> 16) as u8,
        (m >> 8) as u8,
        m as u8,
    ]
}

/// La configuration du tri, telle qu'elle est en ce moment.
pub fn config() -> tri::Config {
    tri::Config {
        ip_diagnostic: ip(),
        mac: mac(),
        port_brdp: PORT_BRDP,
        port_telemetrie: PORT_TELEMETRIE,
        lab_actif: actif(),
    }
}

/// Trie une trame et la depose dans la file de diagnostic si elle lui revient.
///
/// Rend le verdict pour que l'appelant sache s'il doit AUSSI la donner a la
/// pile normale. C'est l'appelant qui fait la copie smoltcp : ce module ne
/// connait pas smoltcp, et n'a pas a le connaitre.
///
/// Appelable depuis le drainage verrouille : pas d'allocation, pas de verrou.
pub fn trie(trame: &[u8]) -> tri::Verdict {
    let verdict = tri::destination(&config(), trame);
    TRIEES.fetch_add(1, Ordering::Relaxed);
    match verdict.pile {
        tri::Pile::Diagnostic => {
            POUR_DIAGNOSTIC.fetch_add(1, Ordering::Relaxed);
        }
        tri::Pile::Normale => {
            POUR_NORMALE.fetch_add(1, Ordering::Relaxed);
        }
        tri::Pile::LesDeux => {
            POUR_LES_DEUX.fetch_add(1, Ordering::Relaxed);
        }
        tri::Pile::Aucune => {
            JETEES.fetch_add(1, Ordering::Relaxed);
        }
    }
    if verdict.pour_diagnostic() {
        file().pose(trame);
        // Une trame detournee de la pile normale est un fait qui merite
        // d'etre dans l'anneau : c'est la preuve que le RST fratricide a ete
        // EVITE, et non pas qu'il ne s'est simplement rien passe.
        if verdict.pile == tri::Pile::Diagnostic {
            crate::kernel::lab::emets(
                crate::kernel::lab::Categorie::Reseau,
                crate::kernel::lab::id::NET_RST_EVITE,
                [verdict.pile as u64, verdict.port as u64, verdict.motif as u64, 0],
            );
        }
    }
    verdict
}

/// Retire une trame pour la pile de diagnostic.
pub fn prends(sortie: &mut [u8]) -> Option<usize> {
    file().retire(sortie)
}

/// Triees, pour le diagnostic, pour la normale, pour les deux.
pub fn compteurs() -> (u64, u64, u64, u64) {
    (
        TRIEES.load(Ordering::Relaxed),
        POUR_DIAGNOSTIC.load(Ordering::Relaxed),
        POUR_NORMALE.load(Ordering::Relaxed),
        POUR_LES_DEUX.load(Ordering::Relaxed),
    )
}

pub fn jetees() -> u64 {
    JETEES.load(Ordering::Relaxed)
}
