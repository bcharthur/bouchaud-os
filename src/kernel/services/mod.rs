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
/// Le modele VISIBLE de la fenetre Services, pur : voir `services/vue.rs`.
#[path = "vue.rs"]
pub mod vue;

use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::sync::SpinLockIrq;
pub use registre::{Etat, Genre};

static REGISTRE: SpinLockIrq<registre::Registre> =
    SpinLockIrq::new(registre::Registre::neuf());

/// Phase de demarrage courante, pour dater les pics.
static PHASE: SpinLockIrq<registre::Id> = SpinLockIrq::new(registre::Id::vide());

/// L'ATELIER DES VUES : le tampon de travail commun, hors pile.
///
/// # Pourquoi il existe
///
/// Construire une vue demande deux tableaux : l'instantane du registre et les
/// lignes visibles. Chacun porte `SERVICES_MAX` elements. Tant que la borne
/// valait trente-deux, les poser sur la pile passait inapercu ; portee a cent
/// vingt-huit pour loger la topologie reelle, la meme ecriture demandait
/// quatre-vingt-sept kilo-octets de pile a `services::draw` -- pour une pile
/// noyau de trente mille. La garde de profondeur l'a vu avant la machine :
/// c'etait un debordement de pile dans le fil du compositeur, c'est-a-dire un
/// ecran fige sans trace.
///
/// Les trois consommateurs -- la fenetre, le clic, la ligne de commande --
/// partagent donc UN tampon statique au lieu d'en poser un chacun.
///
/// # Ce verrou n'est pas celui du registre
///
/// Il est tenu pendant la peinture, et c'est voulu : il ne serialise que les
/// vues entre elles. Les PUBLICATEURS -- la carte reseau, le navigateur,
/// l'ordonnanceur -- prennent `REGISTRE`, que `instantane` rend avant le
/// premier pixel. Peindre ne bloque jamais une mesure.
///
/// Ordre de prise, sans exception : vue -> atelier -> registre.
pub struct Atelier {
    pub entrees: [registre::Entree; registre::SERVICES_MAX],
    pub lignes: [vue::Ligne; registre::SERVICES_MAX],
    /// Entrees utiles apres le dernier `instantane`.
    pub connues: usize,
}

static ATELIER: SpinLockIrq<Atelier> = SpinLockIrq::new(Atelier {
    entrees: [registre::Entree::vide(); registre::SERVICES_MAX],
    lignes: [vue::Ligne::vide(); registre::SERVICES_MAX],
    connues: 0,
});

/// Emprunte l'atelier et y prend un instantane frais du registre.
///
/// Le verrou du registre est rendu AVANT que `travail` ne commence : c'est la
/// raison d'etre de la fonction, et la seule maniere de ne pas le tenir
/// pendant une peinture.
pub fn avec_atelier<R>(travail: impl FnOnce(&mut Atelier) -> R) -> R {
    let mut atelier = ATELIER.lock();
    // L'instantane ecrit DANS l'atelier. Passer par un tableau intermediaire
    // reposerait sur la pile les quarante kilo-octets qu'on vient d'en sortir.
    let n = {
        let atelier = &mut *atelier;
        instantane(&mut atelier.entrees)
    };
    atelier.connues = n;
    travail(&mut atelier)
}

fn maintenant() -> u64 {
    crate::kernel::timer::monotonic_ns()
}

/// Declare l'arborescence. Appele une fois, tot.
///
/// L'arbre represente la RESPONSABILITE, pas un fil d'execution par ligne :
/// `net.dns` est un protocole observable, `nav.request` est un processus, et
/// les deux ont leur place sans qu'on fabrique un travailleur pour le premier.
pub fn declare_arbre() {
    let refuses = {
        let mut r = REGISTRE.lock();
        registre::declare_topologie(&mut r)
    };
    if refuses != 0 {
        // UN ARBRE TRONQUE NE DOIT PAS SE TAIRE. Le service absent de la vue
        // est toujours celui qu'on y cherche.
        crate::serial_println!(
            "BOUCHAUD_SERVICES_REGISTRE_PLEIN refuses={} borne={}",
            refuses,
            registre::SERVICES_MAX,
        );
    }
}

/// Copie l'etat du registre dans `sortie`. Rend le nombre d'entrees copiees.
///
/// # Pourquoi une copie, et non un emprunt
///
/// Le rendu d'une fenetre prend des millisecondes et touche le tampon video.
/// Tenir le verrou du registre pendant ce temps bloquerait toute publication
/// -- la carte reseau, le navigateur, l'ordonnanceur -- au rythme de
/// l'affichage. La vue travaille donc sur un instantane, pris en une fois.
pub fn instantane(sortie: &mut [registre::Entree]) -> usize {
    let r = REGISTRE.lock();
    let entrees = r.entrees();
    let n = entrees.len().min(sortie.len());
    sortie[..n].copy_from_slice(&entrees[..n]);
    n
}

/// Publie les indicateurs d'un service.
pub fn kpi(id: &str, indicateurs: registre::Kpi) {
    let maintenant_ns = maintenant();
    REGISTRE.lock().kpi(id, indicateurs, maintenant_ns);
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
        r.erreur(id, raison, maintenant_ns)
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
// La publication periodique : ce que les sous-systemes savent d'eux-memes
// ---------------------------------------------------------------------------

static DERNIERE_PUBLICATION_NS: AtomicU64 = AtomicU64::new(0);

/// Periode de publication des indicateurs.
///
/// Une seconde : au-dela, la vue ment d'une seconde ; en deca, on paierait un
/// parcours de sous-systemes pour rien. Le rendu, lui, lit un instantane et ne
/// declenche aucune mesure.
const PUBLICATION_NS: u64 = 1_000_000_000;

/// Va chercher, chez chaque sous-systeme, ce qu'il sait deja de lui-meme.
///
/// # Aucune mesure n'est fabriquee ici
///
/// Tout ce qui est publie existait deja : les compteurs du pilote reseau, ceux
/// du routage, l'etat du lien, la resolution ARP. Cette fonction les RASSEMBLE
/// sous un identifiant commun ; elle n'en invente aucun. Ce qui n'est mesure
/// nulle part reste `None`, et s'affiche « N/A ».
/// L'identifiant de service de la carte reseau REELLEMENT en service.
///
/// La topologie declare les deux cartes que cette machine sait piloter. Une
/// seule est presente a la fois, et c'est celle-la qui doit porter l'etat :
/// annoncer « rtl8168 : Actif » sur une machine equipee d'une e1000 envoie
/// chercher la panne dans le mauvais pilote. Celle qui est absente reste
/// `Inconnu`, donc « N/A » -- ce qui est exactement vrai.
pub fn carte_active() -> &'static str {
    if crate::drivers::e1000::using_rtl8168() {
        "net.nic.rtl8168"
    } else {
        "net.nic.e1000"
    }
}

pub fn publie_les_indicateurs() {
    let maintenant_ns = maintenant();
    let precedent = DERNIERE_PUBLICATION_NS.load(Ordering::Relaxed);
    if precedent != 0 && maintenant_ns.saturating_sub(precedent) < PUBLICATION_NS {
        return;
    }
    DERNIERE_PUBLICATION_NS.store(maintenant_ns, Ordering::Relaxed);

    use registre::Kpi;

    // --- la carte, et le lien qu'elle porte -----------------------------
    //
    // DEUX CARTES, DEUX LIGNES. La premiere version publiait le releve du
    // RTL8168 sans regarder quelle carte tournait : sous QEMU, ou la carte est
    // une e1000, la fenetre affichait « rtl8168 -- Actif -- 0 o/0 o » pendant
    // que la barre du haut montrait un bail DHCP a 10.0.2.15. Des octets ont
    // circule ; la ligne disait zero, et disait le nom d'une carte absente.
    //
    // Un zero mesure et un zero faute de mesure se lisent pareil. Seul le
    // second est un mensonge -- et c'est celui qu'on affichait.
    let lien = crate::drivers::e1000::link_up();
    let carte_prete = crate::drivers::e1000::is_ready();
    let carte = carte_active();
    let releve = if crate::drivers::e1000::using_rtl8168() {
        let nic = crate::drivers::rtl8168::releve();
        Kpi {
            rx_octets: Some(nic.rx_octets),
            tx_octets: Some(nic.tx_octets),
            operations: Some(nic.rx_paquets),
            ..Kpi::default()
        }
    } else {
        // Le pilote e1000 ne compte pas ses octets. On ne les invente pas :
        // `None` s'affiche « N/A », et c'est exactement ce qui est vrai.
        // Les trames, elles, se comptent a l'entree unique.
        let vues = crate::net::compteurs_smoltcp().posees;
        Kpi {
            operations: if vues == 0 { None } else { Some(vues) },
            ..Kpi::default()
        }
    };
    if carte_prete {
        kpi(carte, releve);
        // `Reprise` est pose par le pilote lui-meme et ne doit pas etre efface
        // ici : une reprise en cours est plus grave qu'un lien qui porte.
        if !matches!(etat_de(carte), Etat::Reprise | Etat::Degrade) {
            etat(carte, if lien { Etat::Actif } else { Etat::Attente });
        }
    } else {
        etat(carte, Etat::Panne);
    }
    etat("net.link", if lien { Etat::Actif } else { Etat::Attente });
    etat("net.ethernet", if lien { Etat::Actif } else { Etat::Repos });

    // --- ce que le routage a vu -----------------------------------------
    let (routees, arp_vues, dhcp_vues, arp_ok, _arp_ko, _) =
        crate::net::compteurs_routage();
    kpi(
        "net.ipv4",
        Kpi {
            operations: if routees == 0 { None } else { Some(routees) },
            ..Kpi::default()
        },
    );
    if routees != 0 {
        etat("net.ipv4", Etat::Actif);
    } else if crate::net::our_ip() != [0, 0, 0, 0] {
        // Une adresse est posee et la pile repond, mais le routage maison n'a
        // rien route : c'est le cas nominal quand smoltcp porte le trafic.
        etat("net.ipv4", Etat::Repos);
    }
    kpi(
        "net.arp",
        Kpi {
            operations: if arp_vues == 0 { None } else { Some(arp_ok) },
            ..Kpi::default()
        },
    );
    if arp_vues != 0 && etat_de("net.arp") == Etat::Inconnu {
        etat("net.arp", Etat::Actif);
    }
    kpi(
        "net.dhcp",
        Kpi {
            operations: if dhcp_vues == 0 { None } else { Some(dhcp_vues) },
            ..Kpi::default()
        },
    );
    if crate::net::bail_obtenu() {
        // Le bail est pose ; le client se tait jusqu'au renouvellement.
        //
        // Le bail se lit sur le BAIL, pas sur le nombre de trames DHCP vues
        // par le routage maison : sous QEMU c'est smoltcp qui mene l'echange,
        // le compteur maison reste a zero, et la ligne restait « N/A » avec
        // une adresse affichee dans la barre du haut.
        etat("net.dhcp", Etat::Repos);
    } else if lien {
        etat("net.dhcp", Etat::Attente);
    }

    // --- la resolution de noms, et le transport -------------------------
    let resolveur = crate::net::dns_server();
    if resolveur != [0, 0, 0, 0] && etat_de("net.dns") == Etat::Inconnu {
        etat("net.dns", Etat::Repos);
    }
    let (poignees, _syn_rtx, _rtt_min, rtt_max, rtt_moyen) =
        crate::net::transport::retransmission::stats_poignee();
    kpi(
        "net.tcp",
        Kpi {
            operations: Some(poignees),
            // Aucune poignee, aucune latence : « N/A » et non zero, qui se
            // lirait comme « instantane ».
            latence_us: if poignees == 0 { None } else { Some(rtt_moyen * 1_000) },
            latence_max_us: if poignees == 0 { None } else { Some(rtt_max * 1_000) },
            ..Kpi::default()
        },
    );
    if poignees != 0 {
        etat("net.tcp", Etat::Actif);
    } else if lien {
        etat("net.tcp", Etat::Repos);
    }

    // --- l'ordonnanceur et la memoire ------------------------------------
    let (_, _, pire_pic_us) = compteurs_pics();
    kpi(
        "sys.scheduler",
        Kpi {
            latence_max_us: if pire_pic_us == 0 { None } else { Some(pire_pic_us) },
            ..Kpi::default()
        },
    );
    etat("sys.scheduler", Etat::Actif);
    let (heap_utilise, _libre, _total) = crate::kernel::heap::stats();
    kpi("sys.memory.heap", Kpi { rss_octets: Some(heap_utilise as u64), ..Kpi::default() });
    etat("sys.memory.heap", Etat::Actif);
    etat("sys.diag.blackbox", Etat::Actif);
    etat("sys.graphics.wm", Etat::Actif);
    etat("sys.graphics.desktop", Etat::Actif);
}

/// L'etat courant d'un service, ou `Inconnu`.
fn etat_de(id: &str) -> Etat {
    REGISTRE.lock().lis(id).map(|e| e.etat).unwrap_or(Etat::Inconnu)
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
