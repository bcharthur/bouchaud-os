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
/// Le pipeline de navigation : a quelle etape une page bloque.
#[path = "navigation.rs"]
pub mod navigation;
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
/// Declare un service en cours de route, sous un parent existant.
///
/// BOUCHAUD_C27_ARBRE_PAR_PROCESSUS
///
/// L'arbre etait entierement statique : `declare_arbre()` le posait au
/// demarrage et plus rien ne bougeait. Cela suffit pour le noyau et le
/// reseau, dont les briques sont connues d'avance -- pas pour un navigateur,
/// qui cree un WebContent par onglet et un WebWorker a la demande.
///
/// Redeclarer le meme identifiant ne le duplique pas : le releve peut donc
/// appeler cette fonction a chaque passe sans tenir de liste.
pub fn declare(id: &str, parent: &str, genre: Genre) -> bool {
    REGISTRE.lock().declare(id, parent, genre)
}

/// Passe a `Arrete` les instances de processus qui n'ont pas ete revues.
///
/// Une instance disparue laisserait sinon sa derniere mesure affichee pour
/// toujours : quatre-vingt-trois pour cent de CPU figes, sur un PID qui
/// n'existe plus. Elle n'est pas RETIREE de l'arbre -- un WebContent qui
/// vient de mourir est exactement ce qu'on cherche apres un plantage --
/// mais ses mesures sont vidées et son etat le dit.
///
/// `vivants` porte les PID vus pendant la passe. Une entree est une instance
/// si son identifiant commence par `browser.` ET porte un PID.
pub fn oublie_instances_absentes(vivants: &[u32]) {
    let maintenant_ns = maintenant();
    let mut disparues: [registre::Id; 32] = [registre::Id::vide(); 32];
    let mut combien = 0usize;
    {
        let r = REGISTRE.lock();
        for entree in r.entrees() {
            if entree.genre != Genre::Processus || entree.etat == Etat::Arrete {
                continue;
            }
            let Some(pid) = entree.kpi.pid else { continue };
            if !entree.id.texte().starts_with("browser.") {
                continue;
            }
            // Une instance porte son PID en suffixe ; le noeud de ROLE, lui,
            // porte un `pid` quand il n'a qu'une instance. Les distinguer par
            // le suffixe evite d'eteindre le role avec ses enfants.
            if !entree.id.texte().ends_with(|c: char| c.is_ascii_digit()) {
                continue;
            }
            if vivants.contains(&pid) || combien == disparues.len() {
                continue;
            }
            disparues[combien] = entree.id;
            combien += 1;
        }
    }
    for id in &disparues[..combien] {
        let texte = id.texte();
        let evenement = {
            let mut r = REGISTRE.lock();
            r.kpi(texte, registre::Kpi::default(), maintenant_ns);
            r.etat(texte, Etat::Arrete, maintenant_ns)
        };
        if evenement {
            emet_evenement(texte, Etat::Arrete, maintenant_ns);
        }
    }
}

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

/// Change l'etat d'un service ET dit pourquoi.
///
/// « Attente » sans raison oblige a deviner ce qui est attendu, et on devine
/// toujours le jour ou l'on n'a pas le temps.
pub fn etat_car(id: &str, nouvel_etat: Etat, raison: &str) {
    let maintenant_ns = maintenant();
    let evenement = {
        let mut r = REGISTRE.lock();
        r.etat_avec_raison(id, nouvel_etat, raison, maintenant_ns)
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
    // LE MODELE VIENT DU BUS PCI, PAS DE L'ETAT DU PILOTE.
    //
    // `using_rtl8168()` rend `rtl8168::is_ready()`. Apres la reprise de degre
    // quatre du 18 septembre, ce booleen est tombe a faux -- et la fenetre
    // Services a bascule d'un coup sur la e1000, annoncant « rtl8168 absente
    // de ce materiel » pour une puce soudee, et « e1000 Erreur, carte non
    // pilotee » pour une carte qui n'a jamais existe sur cette machine.
    //
    // Une carte ne change pas de modele parce que son pilote a renonce.
    if crate::drivers::rtl8168::presente() {
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
    // CINQ COUCHES, CINQ REPONSES DISTINCTES.
    //
    // La photo du 18 septembre montre « Ethernet deconnecte » dans la barre du
    // haut pendant que la fenetre affiche « rtl8168 Actif, 5 Kio/1 Kio ». Les
    // deux disaient vrai et se contredisaient : le cable porte, des octets
    // circulent, ET aucune adresse IPv4 n'a ete obtenue -- sur materiel reel
    // on ne fabrique jamais les adresses SLIRP de QEMU.
    //
    // Le defaut n'etait pas la mesure, c'etait le mot : un seul « connecte »
    // recouvrait cinq questions qui ont chacune leur reponse.
    //
    //   carte detectee   -> net.nic
    //   pilote actif     -> net.nic.<modele>
    //   lien physique    -> net.link
    //   configuration IP -> net.config (dhcp, dns)
    //   connectivite     -> net.ipv4
    //
    // Chacune porte sa RAISON. « Attente » sans raison oblige a deviner.
    let carte_absente = !carte_prete;
    let _ = carte_absente;
    // QUATRE FAITS, QUATRE REPONSES : presence, attachement, service, lien.
    //
    // La photo du 18 septembre melangeait les quatre dans un booleen, et
    // designait le mauvais composant au pire moment.
    if crate::drivers::rtl8168::presente() {
        let pilote = crate::drivers::rtl8168::etat_pilote();
        let (mot, raison) = crate::drivers::etat_pilote::resume(pilote, lien);
        let etat_publie = match mot {
            "actif" => Etat::Actif,
            "attente" => Etat::Attente,
            "demarrage" => Etat::Demarrage,
            "reprise" => Etat::Reprise,
            "panne" => Etat::Panne,
            _ => Etat::Indisponible,
        };
        etat_car("net.nic.rtl8168", etat_publie, raison);
        if pilote.en_service() {
            kpi("net.nic.rtl8168", releve);
        }
    } else {
        etat_car("net.nic.rtl8168", Etat::Indisponible, "absente de ce materiel");
    }
    // LA e1000 DE QEMU N'EST PAS SUR LA TRIGKEY. Elle ne peut donc pas y etre
    // « en panne » : elle est absente, et c'est tout ce qu'on en sait.
    if crate::drivers::rtl8168::presente() {
        etat_car("net.nic.e1000", Etat::Indisponible, "absente de ce materiel");
    } else if carte_prete {
        etat_car("net.nic.e1000", Etat::Actif, "pilote en service");
        kpi("net.nic.e1000", releve);
    } else {
        etat_car("net.nic.e1000", Etat::Indisponible, "absente de ce materiel");
    }

    // Le lien physique, avec ce qu'il vaut.
    let qualite = crate::net::qualite_lien();
    if carte_absente {
        etat_car("net.link", Etat::Indisponible, "aucune carte");
    } else if lien {
        let raison = if qualite.vitesse_mbps == 0 {
            "lien monte"
        } else if qualite.duplex_complet {
            "duplex complet"
        } else {
            "semi-duplex"
        };
        etat_car("net.link", Etat::Actif, raison);
        kpi(
            "net.link",
            Kpi {
                operations: if qualite.vitesse_mbps == 0 {
                    None
                } else {
                    Some(qualite.vitesse_mbps as u64)
                },
                ..Kpi::default()
            },
        );
    } else {
        etat_car("net.link", Etat::Attente, "pas de cable");
    }
    etat_car(
        "net.ethernet",
        if lien { Etat::Actif } else { Etat::Attente },
        if lien { "trames echangees" } else { "lien bas" },
    );

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
    // LA CONNECTIVITE EFFECTIVE, distincte du lien et de la configuration.
    let adresse = crate::net::our_ip();
    if adresse == [0, 0, 0, 0] {
        etat_car("net.ipv4", Etat::Attente, "aucune adresse");
    } else if routees != 0 {
        etat_car("net.ipv4", Etat::Actif, "trafic route");
    } else {
        // Une adresse est posee et la pile repond, mais le routage maison n'a
        // rien route : c'est le cas nominal quand smoltcp porte le trafic.
        etat_car("net.ipv4", Etat::Repos, "adresse posee");
    }
    kpi(
        "net.arp",
        Kpi {
            operations: if arp_vues == 0 { None } else { Some(arp_ok) },
            ..Kpi::default()
        },
    );
    if arp_vues == 0 {
        etat_car(
            "net.arp",
            if lien { Etat::Repos } else { Etat::Indisponible },
            if lien { "aucune resolution" } else { "lien bas" },
        );
    } else if etat_de("net.arp") != Etat::Degrade {
        etat_car("net.arp", Etat::Actif, "voisins resolus");
    }
    // Le bail se lit sur le BAIL, pas sur le nombre de trames DHCP vues par le
    // routage maison : sous QEMU c'est smoltcp qui mene l'echange, le compteur
    // maison reste a zero, et la ligne restait muette avec une adresse
    // affichee dans la barre du haut.
    // LA RAISON VIENT DE DORA, PAS D'UNE SUPPOSITION.
    //
    // « aucun bail » ne disait pas OU l'echange s'arrete. Les trois cas
    // n'appellent pas la meme enquete : aucune offre (le serveur ne repond
    // pas, ou nos trames ne sortent pas), offre sans accuse (le serveur
    // repond mais refuse), bail obtenu.
    let dora = crate::net::application::dhcp::compteurs();
    kpi(
        "net.dhcp",
        Kpi {
            operations: if dora.discover_envoyes == 0 {
                None
            } else {
                Some(dora.discover_envoyes)
            },
            ..Kpi::default()
        },
    );
    if crate::net::bail_obtenu() {
        etat_car("net.dhcp", Etat::Repos, "bail obtenu");
    } else if !lien {
        etat_car("net.dhcp", Etat::Attente, "lien bas");
    } else {
        // LE CAS DE LA TRIGKEY : le cable porte, le serveur ne repond pas.
        etat_car("net.dhcp", Etat::Attente, dora.derniere_etape.nom());
    }

    // --- la resolution de noms, et le transport -------------------------
    // QUATRE ETATS DISTINCTS POUR LE DNS.
    //
    // « resolveur configure » donnait l'impression que la resolution avait ete
    // validee. Le releve du 24 septembre montre le contraire : le resolveur
    // est bien configure -- 192.168.1.254, obtenu par bail --, la requete
    // part, et aucune reponse ne revient. La ligne disait vrai et laissait
    // croire le contraire.
    //
    //   configure     on sait a qui demander, on n'a rien demande
    //   requete emise on a demande, rien n'est encore revenu
    //   operationnel  une reponse est arrivee jusqu'au socket
    //   en erreur     des requetes partent, aucune ne revient
    use crate::net::sonde_dns::{compte, Barreau};
    let resolveur = crate::net::dns_server();
    let emises = compte(Barreau::RxEthernet);
    let recues = compte(Barreau::SocketLivre) + compte(Barreau::RecvSucces);
    kpi(
        "net.dns",
        Kpi {
            operations: if emises == 0 { None } else { Some(emises) },
            ..Kpi::default()
        },
    );
    if resolveur == [0, 0, 0, 0] {
        etat_car("net.dns", Etat::Attente, "aucun resolveur");
    } else if recues != 0 {
        etat_car("net.dns", Etat::Actif, "resolution operationnelle");
    } else if emises != 0 {
        // DES REQUETES PARTENT ET RIEN NE REVIENT. C'est une panne, et elle
        // doit se voir comme telle -- pas comme un service au repos.
        etat_car("net.dns", Etat::Degrade, "aucune reponse recue");
    } else {
        etat_car("net.dns", Etat::Repos, "configure, aucune requete");
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
        etat_car("net.tcp", Etat::Actif, "connexions etablies");
    } else if adresse == [0, 0, 0, 0] {
        etat_car("net.tcp", Etat::Indisponible, "sans adresse IPv4");
    } else {
        etat_car("net.tcp", Etat::Repos, "aucune connexion");
    }
    // UDP, TLS et les protocoles applicatifs suivent le meme prerequis : sans
    // adresse, ils ne peuvent rien faire, et « Indisponible » le dit mieux
    // qu'une colonne vide.
    let sans_ip = adresse == [0, 0, 0, 0];
    for protocole in ["net.udp", "net.icmp"] {
        if etat_de(protocole) == Etat::Inconnu || sans_ip {
            etat_car(
                protocole,
                if sans_ip { Etat::Indisponible } else { Etat::Repos },
                if sans_ip { "sans adresse IPv4" } else { "aucun trafic" },
            );
        }
    }
    for protocole in ["net.tls", "net.http1", "net.http2", "net.hpack", "net.gzip", "net.brotli"] {
        if etat_de(protocole) == Etat::Inconnu {
            etat_car(
                protocole,
                Etat::Indisponible,
                if sans_ip { "sans adresse IPv4" } else { "aucune session" },
            );
        }
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
    let coeurs = crate::arch::x86_64::smp::schedulable_cpus().max(1);
    etat_car("sys.scheduler", Etat::Actif, "charge de la machine");
    kpi(
        "sys.scheduler",
        Kpi {
            cpu_pour_mille: Some(crate::kernel::timer::cpu_load_pct() as u32 * 10),
            operations: Some(coeurs as u64),
            latence_max_us: if pire_pic_us == 0 { None } else { Some(pire_pic_us) },
            ..Kpi::default()
        },
    );

    // LA MEMOIRE. Le tas mesure ; le physique et le compagnon ne sont pas
    // instrumentes, et le disent au lieu de rester muets.
    let (heap_utilise, _libre, heap_total) = crate::kernel::heap::stats();
    kpi("sys.memory.heap", Kpi { rss_octets: Some(heap_utilise as u64), ..Kpi::default() });
    etat_car("sys.memory.heap", Etat::Actif, "tas noyau");
    // LA COLONNE « RAM » VEUT DIRE « CONSOMMEE ».
    //
    // La premiere version y publiait la RAM installee et la reserve du tas :
    // la ligne `sys` affichait alors 1,4 Gio en additionnant une capacite et
    // une consommation. Deux grandeurs differentes dans une meme colonne
    // donnent une somme qui ne veut rien dire -- et qui se lit comme si le
    // systeme mangeait un gigaoctet et demi.
    //
    // Une capacite se dit donc dans la RAISON, ou elle n'est additionnee avec
    // rien.
    let ram = crate::platform::pc::hardware_facts::usable_ram_bytes();
    etat_car("sys.memory.physical", Etat::Actif, &gio(ram));
    etat_car("sys.memory.buddy", Etat::Actif, &mio(heap_total as u64));

    // BOUCHAUD_P13_CACHE_TELEMETRIE
    // Uniquement des compteurs atomiques/lock-free : observer le cache ne doit
    // jamais ajouter une arete de verrou dans le chemin de faute de page.
    let (cache_hits, cache_misses, cache_waits, _cache_shared) =
        crate::kernel::clean_page_cache::stats();
    let (balayages_evites, cache_entrees, cache_reclaimed) =
        crate::kernel::clean_page_cache::balayage_temoins();
    let (_, _, _, _, _, _, _, cache_pire_ns) =
        crate::kernel::clean_page_cache::acquire_timing();
    let cache_ops = cache_hits.saturating_add(cache_misses).saturating_add(cache_waits);
    kpi(
        "sys.memory.page_cache",
        Kpi {
            operations: Some(cache_ops),
            latence_max_us: if cache_pire_ns == 0 { None } else { Some(cache_pire_ns / 1_000) },
            ..Kpi::default()
        },
    );
    let raison_cache = if cache_reclaimed != 0 {
        "reclamation observee"
    } else if balayages_evites != 0 {
        "balayages inutiles evites"
    } else if cache_entrees != 0 {
        "pages propres en cache"
    } else {
        "aucune page en cache"
    };
    etat_car("sys.memory.page_cache", Etat::Actif, raison_cache);

    // LE STOCKAGE.
    let noeuds = crate::fs::ramfs::used_nodes_relaxed();
    kpi("sys.storage.ramfs", Kpi { operations: Some(noeuds as u64), ..Kpi::default() });
    etat_car("sys.storage.ramfs", Etat::Actif, "systeme de fichiers en RAM");
    etat_car("sys.storage.fs", Etat::Actif, "monte");
    // LE DISQUE REEL, ET NON UNE CONSTANTE.
    //
    // Cette ligne annoncait « aucun disque NVMe » en dur. Le releve du
    // 18 septembre contient au meme instant :
    //
    //     BOUCHAUD_NVME_GREEN modele=KINGSTON OM8SEP4512N-A0 mio=488386
    //     NVME_IO_CQE_OK  NVME_IO_COPY_END
    //
    // Le pilote fonctionne ; c'est la publication qui mentait. Un moniteur qui
    // declare absent un disque en service fait chercher la panne ailleurs.
    let (lectures, ecritures, _vidanges, erreurs_nvme, delais) =
        crate::drivers::nvme::stats();
    if crate::drivers::nvme::present() {
        etat_car("sys.storage.nvme", Etat::Actif, "disque interne");
        kpi(
            "sys.storage.nvme",
            Kpi {
                disque_lu: if lectures == 0 { None } else { Some(lectures) },
                disque_ecrit: if ecritures == 0 { None } else { Some(ecritures) },
                operations: Some(lectures.saturating_add(ecritures)),
                ..Kpi::default()
            },
        );
        if erreurs_nvme != 0 || delais != 0 {
            etat_car("sys.storage.nvme", Etat::Degrade, "erreurs d'entree-sortie");
        }
    } else if crate::drivers::nvme::hors_service() {
        etat_car("sys.storage.nvme", Etat::Panne, "retire du service");
    } else {
        etat_car("sys.storage.nvme", Etat::Indisponible, "aucun disque NVMe");
    }

    // L'USB. C'est la que « Indisponible » se distingue d'une panne : sous
    // QEMU il n'y a pas de xHCI, et ce n'est pas un defaut.
    use crate::platform::pc::hardware_facts as materiel;
    if materiel::xhci_active() {
        let ports = materiel::xhci_connected_ports();
        kpi("sys.usb.xhci", Kpi { operations: Some(ports as u64), ..Kpi::default() });
        etat_car("sys.usb.xhci", Etat::Actif, "controleur en service");
        let claviers = crate::drivers::xhci_active::hid_keyboards();
        let souris = crate::drivers::xhci_active::hid_mice();
        etat_car(
            "sys.usb.keyboard",
            if claviers == 0 { Etat::Attente } else { Etat::Actif },
            if claviers == 0 { "aucun clavier" } else { "clavier present" },
        );
        etat_car(
            "sys.usb.mouse",
            if souris == 0 { Etat::Attente } else { Etat::Actif },
            if souris == 0 { "aucune souris" } else { "souris presente" },
        );
        kpi("sys.usb.keyboard", Kpi { operations: Some(claviers as u64), ..Kpi::default() });
        kpi("sys.usb.mouse", Kpi { operations: Some(souris as u64), ..Kpi::default() });
    } else if materiel::xhci_present() {
        etat_car("sys.usb.xhci", Etat::Demarrage, "enumeration en cours");
    } else {
        for usb in ["sys.usb.xhci", "sys.usb.keyboard", "sys.usb.mouse", "sys.usb.bot"] {
            etat_car(usb, Etat::Indisponible, "aucun controleur xHCI");
        }
        etat_car("sys.storage.usb", Etat::Indisponible, "aucun controleur xHCI");
    }

    // --- LE PIPELINE DE NAVIGATION ---------------------------------------
    //
    // Douze etapes, et pour chacune : son etat, sa duree, ses octets. C'est la
    // reponse a « a quelle etape la page bloque ».
    publie_la_navigation();

    // LE GRAPHIQUE ET LE DIAGNOSTIC.
    // BOUCHAUD_P13_GPU_TELEMETRIE
    // Le GPU core ne vaut pas "le bureau" : sur une machine framebuffer sans
    // backend BGA enregistre, le bureau peut fonctionner et le GPU rester
    // indisponible. Les deux etats sont donc publies separement.
    let gpu = crate::drivers::gpu::stats();
    if gpu.active {
        kpi(
            "sys.graphics.gpu",
            Kpi {
                operations: Some(gpu.presents),
                ..Kpi::default()
            },
        );
        etat_car("sys.graphics.gpu", Etat::Actif, gpu.backend.label());
    } else {
        etat_car("sys.graphics.gpu", Etat::Indisponible, "aucun backend GPU enregistre");
    }
    etat_car("sys.diag.blackbox", Etat::Actif, "archive armee");
    etat_car("sys.diag.serial", Etat::Actif, "trace noyau");
    etat_car("sys.graphics.wm", Etat::Actif, "compositeur");
    etat_car("sys.graphics.desktop", Etat::Actif, "bureau");
    etat_car("sys.graphics.present", Etat::Actif, "presentation a l'ecran");
}

/// Reporte l'etat du pipeline de navigation dans le registre.
///
/// # Les six etapes que le noyau ne voit pas
///
/// Le decodage, l'analyse HTML et CSS, la mise en page, la peinture et la
/// presentation sont du code d'anneau 3, dans `WebContent`. Aucun appel
/// systeme ne les traverse, donc aucune mesure honnete n'en sort d'ici.
///
/// Elles se declarent `Indisponible` AVEC LEUR RAISON, et non vides : une
/// case vide laisse croire que l'etape n'a pas eu lieu, ou qu'elle attend.
/// « non instrumente (anneau 3) » dit ou ne pas chercher.
fn publie_la_navigation() {
    use navigation::{Etape, ETAPES};
    use registre::Kpi;
    let Some((url, url_len, etapes, debut_ns, fin_ns, refus, refus_len)) =
        navigation::instantane()
    else {
        // Aucune navigation depuis le demarrage : la chaine attend, elle n'a
        // pas echoue.
        for rang in 0..ETAPES {
            let Some(etape) = Etape::depuis_rang(rang) else { continue };
            if etat_de(etape.service()) == Etat::Inconnu {
                etat_car(etape.service(), Etat::Repos, "aucune navigation");
            }
        }
        return;
    };

    let adresse = core::str::from_utf8(&url[..url_len]).unwrap_or("?");
    let motif = core::str::from_utf8(&refus[..refus_len]).unwrap_or("");
    for rang in 0..ETAPES {
        let Some(etape) = Etape::depuis_rang(rang) else { continue };
        let mesure = etapes[rang];
        if !etape.observable() {
            etat_car(etape.service(), Etat::Indisponible, "non instrumente (anneau 3)");
            continue;
        }
        let raison = match mesure.etat {
            Etat::Panne => "echec",
            Etat::Demarrage => "en cours",
            // UN PREREQUIS ABSENT SE NOMME. « sans objet » n'apprend rien ;
            // « dhcp: sans-offre » dit ou aller regarder.
            Etat::Indisponible if !motif.is_empty() => prerequis_reseau(),
            Etat::Indisponible => "sans objet",
            Etat::Actif => "termine",
            _ => "en attente",
        };
        let etat_publie = match mesure.etat {
            // Une etape reussie n'est pas « active » : elle est FAITE. La
            // montrer active ferait croire a un travail en cours.
            Etat::Actif => Etat::Repos,
            autre => autre,
        };
        etat_car(etape.service(), etat_publie, raison);
        kpi(
            etape.service(),
            Kpi {
                latence_us: if mesure.duree_us == 0 { None } else { Some(mesure.duree_us) },
                rx_octets: if mesure.octets == 0 { None } else { Some(mesure.octets) },
                ..Kpi::default()
            },
        );
    }

    // LE GROUPE PORTE L'URL. C'est la premiere chose qu'on veut lire.
    let duree_us = if fin_ns != 0 && debut_ns != 0 {
        fin_ns.saturating_sub(debut_ns) / 1_000
    } else {
        0
    };
    // LE GROUPE PORTE L'URL, ET LE MOTIF QUAND IL Y EN A UN.
    //
    // Une navigation refusee avant le moindre `connect` -- reseau sans
    // configuration -- doit se lire comme un refus, pas comme un repos.
    if !motif.is_empty() {
        etat_car("browser.navigation", Etat::Degrade, motif);
    } else {
        etat_car(
            "browser.navigation",
            if fin_ns == 0 { Etat::Demarrage } else { Etat::Repos },
            adresse,
        );
    }
    kpi(
        "browser.navigation",
        Kpi {
            latence_us: if duree_us == 0 { None } else { Some(duree_us) },
            operations: Some(navigation::compteur()),
            ..Kpi::default()
        },
    );
}

/// CE QUI MANQUE AU RESEAU, EN DEUX MOTS.
///
/// Sert de raison aux etapes de navigation qui n'ont pas ete tentees : elles
/// n'ont pas echoue, il leur manque un prerequis, et ce prerequis a un nom.
fn prerequis_reseau() -> &'static str {
    if !crate::drivers::e1000::link_up() {
        return "lien bas";
    }
    if crate::net::our_ip() == [0, 0, 0, 0] {
        return "sans adresse IPv4";
    }
    "reseau indisponible"
}

/// Une capacite en gibioctets, pour une raison.
fn gio(octets: u64) -> alloc::string::String {
    alloc::format!("{} Gio installes", octets / (1024 * 1024 * 1024))
}

/// Une reserve en mebioctets, pour une raison.
fn mio(octets: u64) -> alloc::string::String {
    alloc::format!("{} Mio de reserve", octets / (1024 * 1024))
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
