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
pub mod brdp;
pub mod politique;
pub mod reponses;
pub mod serveur;
pub mod session;
pub mod tampon;
pub mod telemetrie;
pub mod tri;

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

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


// ---------------------------------------------------------------------------
// LE BRANCHEMENT AU DEMARRAGE : UNE SEULE POLITIQUE, DEUX CHEMINS D'AMORCAGE
// ---------------------------------------------------------------------------
//
// `main.rs` et `platform/pc/stage2.rs` amorcent la meme machine par deux
// routes differentes. Ecrire la politique dans les deux ferait deux
// politiques : elles resteraient identiques le jour de la livraison et
// divergeraient la semaine suivante, sans que rien ne le dise -- et l'on
// passerait la campagne d'apres a se demander pourquoi le canal d'enquete
// repond en QEMU et pas sur la machine physique, ou l'inverse.
//
// Les deux appellent donc `ouvre(mac)` puis `demarre_services()`, et la
// politique vit ici.

/// Ce que le serveur BRDP est devenu au demarrage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum EtatBrdp {
    /// `demarre_services()` n'a pas encore ete appele.
    PasEncore = 0,
    /// Le fil ecoute sur `PORT_BRDP`.
    Arme = 1,
    /// AUCUN JETON N'A ETE INJECTE AU BUILD, et rien n'ecoute.
    ///
    /// Ce n'est pas une panne : c'est une image qui n'est pas une image de
    /// laboratoire. La distinction compte, parce qu'une panne appelle une
    /// enquete et celle-ci appelle une option de construction.
    Desactive = 2,
    /// Un jeton existe, mais le fil n'a pas pu demarrer.
    Echec = 3,
}

impl EtatBrdp {
    /// Le mot que porte le releve. C'est le vocabulaire du client PC.
    pub const fn nom(self) -> &'static str {
        match self {
            EtatBrdp::PasEncore => "pending",
            EtatBrdp::Arme => "armed",
            EtatBrdp::Desactive => "disabled",
            EtatBrdp::Echec => "failed",
        }
    }

    const fn depuis(v: u8) -> Self {
        match v {
            1 => EtatBrdp::Arme,
            2 => EtatBrdp::Desactive,
            3 => EtatBrdp::Echec,
            _ => EtatBrdp::PasEncore,
        }
    }
}

/// Ce que la telemetrie est devenue au demarrage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum EtatTelemetrie {
    /// `demarre_services()` n'a pas encore ete appele.
    PasEncore = 0,
    /// Le fil tourne et pousse l'anneau en diffusion.
    EnMarche = 1,
    /// Le fil n'a pas pu demarrer.
    Echec = 2,
}

impl EtatTelemetrie {
    pub const fn nom(self) -> &'static str {
        match self {
            EtatTelemetrie::PasEncore => "pending",
            EtatTelemetrie::EnMarche => "running",
            EtatTelemetrie::Echec => "failed",
        }
    }

    const fn depuis(v: u8) -> Self {
        match v {
            1 => EtatTelemetrie::EnMarche,
            2 => EtatTelemetrie::Echec,
            _ => EtatTelemetrie::PasEncore,
        }
    }
}

/// L'etat du canal d'enquete, en une lecture et sans effet de bord.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Services {
    pub ip: [u8; 4],
    pub brdp: EtatBrdp,
    pub telemetrie: EtatTelemetrie,
}

static ETAT_BRDP: AtomicU8 = AtomicU8::new(0);
static ETAT_TELEMETRIE: AtomicU8 = AtomicU8::new(0);
static SERVICES_DEMARRES: AtomicBool = AtomicBool::new(false);

/// L'etat publie par `demarre_services()`.
///
/// LISIBLE SANS COM1, SANS RESEAU ET SANS VERROU. Le message serie n'est
/// qu'un doublon : sur la machine de reference il n'a jamais produit un seul
/// octet (`com1=bus-flottant  serial_bytes=0`), et une observabilite qui ne
/// vit que dans un port absent n'existe pas.
pub fn services() -> Services {
    Services {
        ip: ip(),
        brdp: EtatBrdp::depuis(ETAT_BRDP.load(Ordering::Acquire)),
        telemetrie: EtatTelemetrie::depuis(ETAT_TELEMETRIE.load(Ordering::Acquire)),
    }
}

/// Lance les deux canaux d'enquete. NE PANIQUE PAS, NE BLOQUE PAS L'AMORCAGE.
///
/// A appeler juste apres `ouvre(mac)`. Idempotent.
///
/// # L'ordre n'est pas arbitraire
///
/// La telemetrie part EN PREMIER, parce qu'elle est le canal qui survit a ce
/// qu'on cherche a nommer. Elle ne demande ni bail DHCP -- dont l'OFFER est
/// une trame ENTRANTE, c'est-a-dire precisement la chose en panne -- ni
/// jeton : elle n'accepte rien en entree, donc il n'y a rien a authentifier.
/// BRDP, lui, a besoin des deux sens ; le faire partir d'abord ferait
/// dependre le canal de survie de la reussite du canal fragile.
///
/// # Aucun des deux ne peut faire echouer l'amorcage
///
/// Les deux `demarre()` rendent un booleen et ne paniquent pas : un fil noyau
/// qu'on ne peut pas creer rend `false`. Un canal d'enquete qui empecherait la
/// machine de demarrer serait la pire des ironies -- on perdrait la machine
/// pour garder l'outil qui sert a l'observer.
pub fn demarre_services() -> Services {
    if SERVICES_DEMARRES.swap(true, Ordering::AcqRel) {
        // Les deux chemins d'amorcage sont exclusifs, mais l'idempotence est
        // gratuite et elle evite qu'un second appel republie un etat.
        return services();
    }

    let ouvert = actif();

    let telemetrie = if telemetrie::demarre() {
        EtatTelemetrie::EnMarche
    } else {
        EtatTelemetrie::Echec
    };

    // `serveur::arme()` ne dit pas « le fil tourne » : il dit qu'un jeton non
    // vide a ete injecte au build. Sans lui on ne TENTE meme pas, et l'on ne
    // parle pas d'echec -- un debugger en lecture seule sans authentification
    // reste un debugger qui publie l'etat interne de la machine a quiconque
    // atteint le segment local.
    let brdp = if !serveur::arme() {
        EtatBrdp::Desactive
    } else if serveur::demarre() {
        EtatBrdp::Arme
    } else {
        EtatBrdp::Echec
    };

    ETAT_TELEMETRIE.store(telemetrie as u8, Ordering::Release);
    ETAT_BRDP.store(brdp as u8, Ordering::Release);

    let ip = ip();
    publie(ip, ouvert, brdp, telemetrie);
    Services { ip, brdp, telemetrie }
}

/// Publie l'etat la ou on ira le chercher : le registre, l'anneau, la serie.
fn publie(ip: [u8; 4], ouvert: bool, brdp: EtatBrdp, telemetrie: EtatTelemetrie) {
    use crate::kernel::services::{etat, etat_car, Etat};

    etat(
        "net.lab",
        if ouvert { Etat::Actif } else { Etat::Indisponible },
    );

    match telemetrie {
        EtatTelemetrie::EnMarche => etat("net.lab.telemetrie", Etat::Actif),
        // LA RAISON EST DITE, PAS DEVINEE. « Panne » sans raison oblige a
        // choisir entre « le LAB n'est pas ouvert » et « l'ordonnanceur a
        // refuse un fil », et l'on choisit toujours mal le jour ou l'on n'a
        // pas le temps.
        EtatTelemetrie::Echec => etat_car(
            "net.lab.telemetrie",
            Etat::Panne,
            if ouvert { "fil-refuse" } else { "lab-ferme" },
        ),
        EtatTelemetrie::PasEncore => {}
    }

    match brdp {
        EtatBrdp::Arme => etat("net.lab.brdp", Etat::Actif),
        // INDISPONIBLE, PAS EN PANNE. Le registre porte deja cette
        // distinction : une panne appelle une enquete, une indisponibilite
        // appelle un prerequis -- ici, une option de construction.
        EtatBrdp::Desactive => etat_car("net.lab.brdp", Etat::Indisponible, "sans-jeton"),
        EtatBrdp::Echec => etat_car(
            "net.lab.brdp",
            Etat::Panne,
            if ouvert { "fil-refuse" } else { "lab-ferme" },
        ),
        EtatBrdp::PasEncore => {}
    }

    crate::kernel::lab::emets(
        crate::kernel::lab::Categorie::Remote,
        crate::kernel::lab::id::LAB_SERVICES,
        [empaquete_ip(ip), brdp as u64, telemetrie as u64, PORT_BRDP as u64],
    );

    // LE MESSAGE SERIE EST UN DOUBLON, ET RIEN D'AUTRE. Tout ce qu'il dit est
    // deja dans le registre des services et dans l'anneau LAB ; sur la machine
    // de reference il n'a jamais produit un octet.
    crate::serial_println!(
        "BOUCHAUD_LAB_READY ip={}.{}.{}.{} brdp={} telemetry={}",
        ip[0], ip[1], ip[2], ip[3],
        brdp.nom(),
        telemetrie.nom(),
    );
}
