//! L'observatoire : UNE source de verite pour le demarrage, l'interface et la
//! boite noire.
//!
//! # Ce qui manquait, et ce que cela a coute
//!
//! Devant le releve physique du 17 septembre, il faut encore deduire a la main
//! si la panne vient du RTL8168, d'ARP, de DNS, de TCP, du `RequestServer` ou
//! de l'ordonnanceur -- en recollant des compteurs qui vivent dans quatre
//! endroits differents et ne se datent pas entre eux.
//!
//! Et l'archive ne porte pas la reponse la plus simple : le premier
//! echantillon encore lisible, a t~287 s, annonce
//! `hid_wake_to_run_max_us = 12 179 860` sans dire QUAND ce pic a eu lieu ni
//! pendant quelle phase. Un maximum cumule ne date rien.
//!
//! # La regle
//!
//! ```text
//!                Registre des services
//!                        |
//!           +------------+-------------+
//!           |            |             |
//!           v            v             v
//!       Interface     Ecran de      Boite noire
//!                     demarrage
//! ```
//!
//! Un evenement reel met a jour UNE metrique centrale ; les consommateurs
//! l'affichent. Pas un compteur dans `netetat`, un autre dans une interface et
//! un troisieme dans l'archive.
//!
//! # Le cout
//!
//! La discipline est dans `registre.rs`, qui est PUR et teste sur machine
//! hote. Ici il n'y a qu'un verrou, une table bornee et l'ecriture des
//! evenements. Aucune allocation, aucune chaine formatee par paquet : seuls
//! les CHANGEMENTS d'etat produisent une ligne.

#[path = "registre.rs"]
pub mod registre;

use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::sync::SpinLockIrq;
pub use registre::{Etat, Genre};

static REGISTRE: SpinLockIrq<registre::Registre> =
    SpinLockIrq::new(registre::Registre::neuf());

/// Phase de demarrage courante, pour dater les pics.
static PHASE: SpinLockIrq<registre::Id> = SpinLockIrq::new(registre::Id::vide());

fn maintenant() -> u64 {
    crate::kernel::timer::monotonic_ns()
}

/// Declare l'arborescence. Appele une fois, tot.
///
/// L'arbre represente la RESPONSABILITE, pas un fil d'execution par ligne :
/// `net.dns` est un protocole observable, `nav.request` est un processus, et
/// les deux ont leur place sans qu'on fabrique un travailleur pour le premier.
pub fn declare_arbre() {
    let mut r = REGISTRE.lock();
    // Systeme
    r.declare("sys", "", Genre::Groupe);
    r.declare("sys.memoire", "sys", Genre::Noyau);
    r.declare("sys.smp", "sys", Genre::Noyau);
    r.declare("sys.usb", "sys", Genre::Pilote);
    r.declare("sys.entrees", "sys", Genre::Pilote);
    r.declare("sys.graphique", "sys", Genre::Service);
    r.declare("sys.stockage", "sys", Genre::Pilote);
    r.declare("sys.blackbox", "sys", Genre::Service);
    // Reseau
    r.declare("net", "", Genre::Groupe);
    r.declare("net.rtl8168", "net", Genre::Pilote);
    r.declare("net.lien", "net", Genre::Service);
    r.declare("net.arp", "net", Genre::Protocole);
    r.declare("net.dhcp", "net", Genre::Protocole);
    r.declare("net.dns", "net", Genre::Protocole);
    r.declare("net.ipv4", "net", Genre::Protocole);
    r.declare("net.tcp", "net", Genre::Protocole);
    r.declare("net.tls", "net", Genre::Protocole);
    r.declare("net.http", "net", Genre::Protocole);
    // Navigateur
    r.declare("nav", "", Genre::Groupe);
    r.declare("nav.hote", "nav", Genre::Processus);
    r.declare("nav.request", "nav", Genre::Processus);
    r.declare("nav.contenu", "nav", Genre::Processus);
    r.declare("nav.compositeur", "nav", Genre::Processus);
}

/// Change l'etat d'un service, et n'ecrit que si cela change quelque chose.
pub fn etat(id: &str, nouvel_etat: Etat) {
    let maintenant_ns = maintenant();
    let evenement = {
        let mut r = REGISTRE.lock();
        r.etat(id, nouvel_etat, maintenant_ns)
    };
    if evenement {
        emet_evenement(id, nouvel_etat, maintenant_ns);
    }
}

/// Note une reussite. Ne produit aucune ligne : elle date, elle ne raconte pas.
pub fn succes(id: &str) {
    let maintenant_ns = maintenant();
    REGISTRE.lock().succes(id, maintenant_ns);
}

/// Note une erreur et sa raison. Passe le service en degrade.
pub fn erreur(id: &str, raison: &str) {
    let maintenant_ns = maintenant();
    let evenement = {
        let mut r = REGISTRE.lock();
        r.erreur(id, raison, maintenant_ns);
        r.etat(id, Etat::Degrade, maintenant_ns)
    };
    if evenement {
        emet_evenement(id, Etat::Degrade, maintenant_ns);
    }
}

/// Pose la phase de demarrage courante. Elle date les pics de latence.
pub fn phase(nom: &str) {
    *PHASE.lock() = registre::Id::depuis(nom);
    etat(nom, Etat::Demarrage);
}

/// La phase courante, ou `demarrage` a defaut.
fn phase_courante() -> registre::Id {
    *PHASE.lock()
}

fn emet_evenement(id: &str, nouvel_etat: Etat, maintenant_ns: u64) {
    let (duree_ms, erreurs, reprises, raison) = {
        let r = REGISTRE.lock();
        match r.lis(id) {
            Some(e) => (
                e.duree_demarrage_ms().unwrap_or(0),
                e.erreurs,
                e.reprises,
                e.raison,
            ),
            None => (0, 0, 0, registre::Id::vide()),
        }
    };
    crate::kernel::blackbox::service_evenement(
        maintenant_ns,
        id,
        nouvel_etat.nom(),
        duree_ms,
        erreurs,
        reprises,
        if raison.est_vide() { "-" } else { raison.texte() },
    );
}

// ---------------------------------------------------------------------------
// Les pics de latence de reveil
// ---------------------------------------------------------------------------

static DERNIER_PIC_NS: AtomicU64 = AtomicU64::new(0);
static PIRE_PIC_US: AtomicU64 = AtomicU64::new(0);
static PICS_VUS: AtomicU64 = AtomicU64::new(0);
static PICS_ECRITS: AtomicU64 = AtomicU64::new(0);

/// Signale un reveil qui a trop attendu son processeur.
///
/// # Ce qui est enregistre, et ce qui ne l'est pas
///
/// Rien en dessous de la borne : un journal par reveil ne mesurerait plus que
/// lui-meme. Au-dessus, un enregistrement -- mais pas mille par seconde : le
/// suivant attend un repos, sauf s'il est nettement pire, auquel cas il porte
/// une information neuve.
///
/// Le releve physique disait `hid_wake_to_run_max_us = 12 179 860` sans dire
/// quand. Cet enregistrement-ci date le pic, nomme la phase de demarrage
/// pendant laquelle il tombe, et decrit le coeur vise.
pub fn pic_reveil(delta_us: u64, echeance_ns: u64, reprise_ns: u64) {
    if delta_us < registre::SEUIL_PIC_REVEIL_US {
        return;
    }
    PICS_VUS.fetch_add(1, Ordering::Relaxed);
    let maintenant_ns = reprise_ns;
    let dernier = DERNIER_PIC_NS.load(Ordering::Relaxed);
    let pire = PIRE_PIC_US.load(Ordering::Relaxed);
    if !registre::pic_a_enregistrer(
        delta_us,
        dernier,
        pire,
        maintenant_ns,
        registre::REPOS_PIC_NS,
    ) {
        PIRE_PIC_US.fetch_max(delta_us, Ordering::Relaxed);
        return;
    }
    DERNIER_PIC_NS.store(maintenant_ns, Ordering::Relaxed);
    PIRE_PIC_US.fetch_max(delta_us, Ordering::Relaxed);
    PICS_ECRITS.fetch_add(1, Ordering::Relaxed);

    let cpu = crate::arch::x86_64::smp::cpu_index();
    let file = crate::arch::x86_64::cpu_local::CpuId::from_index(cpu)
        .map(|id| crate::arch::x86_64::cpu_local::local(id).run_queue_len())
        .unwrap_or(0);
    let bkl = crate::kernel::smp_lock::health_snapshot();
    let phase = phase_courante();

    crate::kernel::blackbox::pic_reveil(
        maintenant_ns,
        echeance_ns,
        delta_us,
        cpu as u32,
        file as u32,
        crate::kernel::task::current_is_kernel_task(),
        bkl.owner_token as u32,
        if phase.est_vide() { "runtime" } else { phase.texte() },
    );

    // L'ETAT COMPLET, UNE FOIS, AU MOMENT OU IL EXPLIQUE QUELQUE CHOSE.
    //
    // La sonde d'ordonnancement n'imprime plus son etat complet a chaque
    // seconde -- c'est ce qui effacait le demarrage de l'archive. Elle le fait
    // sur demande, et un pic de reveil est exactement l'occasion qui le
    // justifie.
    crate::kernel::task::demande_dump_ordonnancement_latence();
}

/// Ce que les pics ont donne : vus, et ecrits.
pub fn compteurs_pics() -> (u64, u64, u64) {
    (
        PICS_VUS.load(Ordering::Relaxed),
        PICS_ECRITS.load(Ordering::Relaxed),
        PIRE_PIC_US.load(Ordering::Relaxed),
    )
}

// ---------------------------------------------------------------------------
// La lecture
// ---------------------------------------------------------------------------

/// Applique `vue` a chaque entree, sous le verrou.
pub fn parcours(mut vue: impl FnMut(&registre::Entree)) {
    let r = REGISTRE.lock();
    for entree in r.entrees() {
        vue(entree);
    }
}

/// Le service le plus grave, s'il y en a un.
pub fn pire() -> Option<(registre::Id, Etat, registre::Id, u64)> {
    let r = REGISTRE.lock();
    r.pire()
        .map(|e| (e.id, e.etat, e.raison, e.derniere_erreur_ns))
}

pub fn compteurs() -> registre::Compteurs {
    REGISTRE.lock().compteurs()
}
