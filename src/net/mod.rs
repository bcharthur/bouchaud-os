//! Pile reseau de Bouchaud OS, organisee par couches du modele OSI.
//!
//! ```text
//!   link/        L2  liaison       ethernet, arp
//!   internet/    L3  reseau        ipv4, icmp
//!   transport/   L4  transport     tcp, udp
//!   security/    L5/6 session+pres tls (1.3 : handshake, record, crypto, x509)
//!   encoding/    L6  presentation  inflate (deflate/gzip), brotli
//!   application/ L7  application   dns, dhcp, http, http2, hpack, html
//!   stack.rs         moteur de pile (loopback) + ce module : interface,
//!                    routage, fetch HTTP(S), commandes (ping, ifconfig...).
//! ```
//!
//! Etat : loopback `lo` (127.0.0.1) actif ; `eth0` via driver e1000 (ARP/IP/
//! UDP/TCP reels) ; DNS/DHCP, HTTP/1.1+2, TLS 1.3 fonctionnels.

// Couches OSI.
/// Choix du resolveur DNS remis au navigateur, pur : voir `resolveur.rs`.
pub mod resolveur;
pub mod link;
pub mod internet;
pub mod transport;
/// L'echelle de la reponse DNS : voir `sonde_dns.rs`.
pub mod sonde_dns;
/// La file de trames d'un consommateur, PURE : voir `net/file_trames.rs`.
pub mod file_trames;
/// `netdiag` et `netetat` : la preuve physique que le reseau tient dans la
/// duree. Voir `net/diagnostic.rs`.
pub mod chronologie;
pub mod rx_recuperation;
pub mod diag_distant;
pub mod diagnostic;
pub mod security;
pub mod encoding;
pub mod application;
pub mod stack;

// Re-exports a plat : conserve les chemins `net::<module>` historiques tout en
// rangeant physiquement les fichiers par couche.
pub use link::{ethernet, arp};
pub use internet::{ipv4, icmp};
pub use transport::{tcp, udp};
pub use security::tls;
pub use encoding::{inflate, brotli};
pub use application::{dns, dhcp, http, http2, hpack, html};

use crate::arch::x86_64::pci;
use crate::drivers::e1000;
use crate::drivers::vga::{self, COLOR_CYAN, COLOR_GREEN, COLOR_YELLOW, COLOR_DEFAULT};
use alloc::format;
use alloc::string::String;
use crate::net::ipv4::Ipv4Addr;
use crate::kernel::sync::SpinLockIrq;
use core::sync::atomic::AtomicBool;

/// Adresse de l'interface loopback.
pub const LO_ADDR: Ipv4Addr = [127, 0, 0, 1];

/// LA PRESOMPTION SLIRP, ET SON SEUL DOMAINE DE VALIDITE.
///
/// Ces trois adresses sont la configuration d'usine du NAT de QEMU. Elles sont
/// justes la, et fausses partout ailleurs. Elles vivent dans un module nomme
/// pour qu'on ne puisse pas les ecrire ailleurs par distraction : le garde-fou
/// `verifie-reseau-sans-triche.py` les interdit dans tout le reste de la pile.
///
/// # Ce qu'elles ont coute
///
/// Sur la machine de reference, `10.0.2.3` a fait repondre « Unable to resolve
/// host » a toutes les pages pendant que le reseau fonctionnait. Une adresse
/// qui ne mene nulle part vaut moins que pas d'adresse du tout : sans
/// resolveur, le navigateur le DIT ; avec un faux, il attend un delai
/// d'attente et accuse le reseau.
mod presomption_slirp {
    use super::Ipv4Addr;
    pub const IP: Ipv4Addr = [10, 0, 2, 15];
    pub const PASSERELLE: Ipv4Addr = [10, 0, 2, 2];
    pub const RESOLVEUR: Ipv4Addr = [10, 0, 2, 3];
}

// Configuration eth0 : presomption SLIRP au depart, que DHCP remplace, et que
// `oublie_la_presomption_slirp` efface sur une carte reelle.
static mut OUR_IP: Ipv4Addr = presomption_slirp::IP;
static mut GW_IP: Ipv4Addr = presomption_slirp::PASSERELLE;
static mut DNS_IP: Ipv4Addr = presomption_slirp::RESOLVEUR;

/// Le resolveur COMPILE, ou RIEN sur une carte reelle.
///
/// `net::resolveur` range le bail avant la passerelle avant la valeur
/// compilee. Ce dernier rang n'a de sens que sous l'emulateur : sur une carte
/// physique il designe une machine qui n'existe pas, et le navigateur passe
/// alors son temps a attendre des delais.
///
/// Rendre zero ici fait descendre `choisis` jusqu'a `Source::Aucun`, qui est
/// la verite : nous n'avons pas de resolveur, et il vaut mieux le dire.
pub fn resolveur_compile() -> Ipv4Addr {
    if e1000::using_rtl8168() {
        [0, 0, 0, 0]
    } else {
        presomption_slirp::RESOLVEUR
    }
}

/// Efface la presomption SLIRP de la configuration vivante.
///
/// Appelee quand la carte est REELLE et qu'aucun bail n'est arrive. Garder
/// `10.0.2.2` comme passerelle ferait resoudre par ARP une adresse qui
/// n'existe pas sur ce reseau -- quatre tentatives, deux secondes, et un echec
/// mis en cache, a chaque paquet sortant.
fn oublie_la_presomption_slirp() {
    unsafe {
        if OUR_IP == presomption_slirp::IP { OUR_IP = [0, 0, 0, 0]; }
        if GW_IP == presomption_slirp::PASSERELLE { GW_IP = [0, 0, 0, 0]; }
        if DNS_IP == presomption_slirp::RESOLVEUR { DNS_IP = [0, 0, 0, 0]; }
    }
}

/// POSE LE VERDICT DE DEMARRAGE, ET LUI SEUL.
///
/// # Pourquoi cela passe par une fonction
///
/// La presomption SLIRP etait effacee a UN endroit : le verdict rendu par
/// `demarre()`. Sur la TRIGKEY, `demarre()` ne rend jamais
/// `SansConfiguration` -- au demarrage le lien est encore bas, donc
/// `LienBas`. C'est le veilleur qui, une fois le cable monte et le DHCP
/// echoue, pose `SansConfiguration` -- et lui n'effacait rien.
///
/// Resultat sur le releve du 18 septembre :
///
/// ```text
/// verdict=sans-configuration  ip=10.0.2.15  gw=10.0.2.2  dns=10.0.2.3
/// ```
///
/// Une adresse de QEMU annoncee sur un cable de bureau : chaque paquet
/// sortant cherchait par ARP une passerelle qui n'existe pas, et la fenetre
/// Services affichait `ipv4 Actif` au-dessus d'un reseau injoignable.
///
/// Deux chemins pour un meme verdict, et un seul des deux tenait la regle.
/// Il n'y en a plus qu'un.
fn pose_le_verdict(nouvel_etat: Demarrage) {
    if matches!(nouvel_etat, Demarrage::SansConfiguration) {
        // CARTE REELLE, AUCUN BAIL : la configuration d'usine de QEMU n'est
        // pas une configuration, c'est une adresse qui n'existe pas sur ce
        // cable.
        oublie_la_presomption_slirp();
    }
    unsafe { DEMARRAGE = nouvel_etat; }
    // AUDIT : CE QUE LE RESTE DU SYSTEME VOIT, A CET INSTANT.
    //
    // `external_enabled()` est vrai pour `Pret` ET pour `SansBail`. Or
    // `SansBail` peut porter la presomption SLIRP compilee -- les memes
    // octets qu'un vrai bail, sans qu'aucun bail n'ait ete obtenu.
    // `bail_obtenu()` est la seule chose qui les distingue, et cette ligne la
    // met a cote du verdict pour qu'on puisse en juger sur mesure.
    crate::kernel::dmesg::log_fmt(format_args!(
        "NET_VERDICT etat={} ip={} gw={} dns={} source={} external_enabled={} connecte={}",
        nom_demarrage(nouvel_etat),
        ipv4::format_addr(&our_ip()),
        ipv4::format_addr(&gateway()),
        ipv4::format_addr(&dns_server()),
        if bail_obtenu() { "bail" } else { "presomption" },
        external_enabled() as u8,
        connecte() as u8,
    ));
}

/// Adresse IPv4 d'eth0.
pub fn our_ip() -> Ipv4Addr { unsafe { OUR_IP } }
/// Passerelle par defaut.
pub fn gateway() -> Ipv4Addr { unsafe { GW_IP } }
/// Serveur DNS configure.
pub fn dns_server() -> Ipv4Addr { unsafe { DNS_IP } }

// BOUCHAUD_BAIL_REELLEMENT_OBTENU_V1
//
// `dns_server()` rend `DNS_IP`, qui vaut la CONSTANTE COMPILEE tant qu'aucun
// bail n'est arrive. Rien ne distinguait donc « le DHCP a rendu 10.0.2.3 » de
// « aucun DHCP n'a repondu, voici la valeur d'usine » -- ce sont les memes
// octets.
//
// Le releve du 16 septembre 18:31 l'a montre en une ligne :
//
//     BOUCHAUD_NAVIGATEUR_RESOLVEUR adresse=10.0.2.3 source=bail-dhcp
//                                   bail=10.0.2.3 passerelle=10.0.2.2
//
// `source=bail-dhcp` sur une machine physique qui n'avait recu aucun bail.
// Le choix bail -> passerelle -> compile etait juste ; l'entree qu'on lui
// donnait ne l'etait pas, et il ne pouvait donc jamais descendre d'un cran.
static BAIL_OBTENU: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Un bail DHCP a-t-il REELLEMENT ete obtenu depuis l'amorcage ?
///
/// C'est la seule question qui distingue une configuration de la valeur
/// d'usine, puisque les deux peuvent porter les memes octets.
pub fn bail_obtenu() -> bool {
    BAIL_OBTENU.load(core::sync::atomic::Ordering::Acquire)
}

/// Applique une configuration reseau (ex. obtenue par DHCP). Invalide le cache ARP.
pub fn set_config(ip: Ipv4Addr, gw: Ipv4Addr, dns: Ipv4Addr) {
    unsafe { OUR_IP = ip; GW_IP = gw; DNS_IP = dns; GW_MAC = None; }
    BAIL_OBTENU.store(true, core::sync::atomic::Ordering::Release);
    // Une adresse materielle apprise avant la configuration ne vaut plus rien,
    // et une entree NEGATIVE posee pendant qu'on etait mal configure ferait
    // echouer les deux premieres secondes d'un reseau desormais correct.
    oublie_voisins();
}

/// Indique si une interface routable vers l'exterieur est active.
///
/// # `SansBail` ne suffit pas, et c'est une mesure qui le dit
///
/// Ce predicat rendait vrai pour `Pret` ET pour `SansBail`, au motif que le
/// repli SLIRP « marche ». Sous un SLIRP hors 10.0.2.x, la ligne d'audit
/// posee au moment du verdict donne :
///
/// ```text
/// NET_VERDICT etat=sans-bail ip=10.0.2.15 gw=10.0.2.2 dns=10.0.2.3
///             source=presomption external_enabled=1 connecte=1
/// ```
///
/// Le serveur offrait 192.168.76.15. Le systeme annoncait donc un reseau
/// exterieur utilisable sur une adresse qu'il avait INVENTEE, hors du
/// sous-reseau reel. `bail_obtenu()` est la seule chose qui distingue une
/// configuration d'une valeur d'usine -- les deux portent les memes octets --
/// et c'est elle qui tranche ici.
///
/// `SansBail` reste vrai quand un bail A ete obtenu : une renegociation qui
/// echoue plus tard ne rend pas fausse l'adresse deja recue.
pub fn external_enabled() -> bool {
    match etat_demarrage() {
        Demarrage::Pret => true,
        Demarrage::SansBail => bail_obtenu(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// eth0 / e1000 : activation et ARP reel
// ---------------------------------------------------------------------------

/// Etat de l'interface exterieure a l'issue du demarrage.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Demarrage {
    /// Aucune carte reseau sur le bus PCI. Une machine sans reseau, ce qui est
    /// une configuration valide et non une panne.
    SansCarte,
    /// Carte presente mais non geree, ou dont l'initialisation a echoue.
    CarteRefusee,
    /// Carte prete, mais le lien n'est pas monte : rien a configurer.
    LienBas,
    /// Carte prete, lien monte, pas de bail DHCP. La configuration statique de
    /// repli s'applique — c'est ce qui fait marcher SLIRP sans serveur DHCP.
    SansBail,
    /// Lien physique monte mais aucune configuration IPv4 n'a ete obtenue.
    /// Sur materiel reel on ne fabrique jamais les adresses SLIRP de QEMU.
    SansConfiguration,
    /// Interface configuree : adresse, passerelle, resolveur.
    Pret,
}

static mut DEMARRAGE: Demarrage = Demarrage::SansCarte;

/// Ce que l'initialisation du demarrage a obtenu.
pub fn etat_demarrage() -> Demarrage { unsafe { DEMARRAGE } }

/// Met le reseau en service au demarrage. **N'echoue jamais.**
///
/// ## Pourquoi cela ne peut pas rester une commande
///
/// Le driver e1000 existait et marchait, mais il fallait taper `ifup` puis
/// `dhcp` avant de pouvoir ouvrir une page. Sur un poste de travail, cela veut
/// dire que le navigateur ne trouve pas le reseau la premiere fois qu'on
/// l'ouvre — et qu'il faut connaitre deux commandes pour reparer quelque chose
/// qui n'est pas casse. Un systeme dont l'interface graphique demarre doit
/// avoir son reseau en service, comme il a son clavier.
///
/// ## Pourquoi chaque echec est benin
///
/// Aucune des cinq issues n'arrete le demarrage. Une machine sans carte reseau
/// doit demarrer ; une machine dont le lien est bas doit demarrer ; une machine
/// sans serveur DHCP doit demarrer avec sa configuration de repli — c'est
/// exactement le cas de SLIRP, ou l'adresse est connue d'avance. Rendre le
/// reseau fatal au demarrage transformerait une machine utilisable en machine
/// morte pour la seule raison qu'elle est hors ligne.
///
/// Le prix est un delai borne : la negociation DHCP attend deux fois quelques
/// secondes avant de renoncer, et seulement si une carte a repondu. Une machine
/// sans carte ne paie rien du tout.
pub fn demarre() -> Demarrage {
    let etat = demarre_interne();
    pose_le_verdict(etat);
    crate::kernel::sysroot::refresh_resolver();
    let ligne = match etat {
        Demarrage::SansCarte => String::from("net: lo 127.0.0.1 actif ; aucune carte reseau"),
        Demarrage::CarteRefusee => String::from("net: lo actif ; carte presente mais non geree"),
        Demarrage::LienBas => String::from("net: lo actif ; eth0 initialisee, lien bas"),
        Demarrage::SansBail => format!(
            "net: eth0 {} (repli QEMU SLIRP) gw {} dns {}",
            ipv4::format_addr(&our_ip()), ipv4::format_addr(&gateway()),
            ipv4::format_addr(&dns_server())),
        Demarrage::SansConfiguration => String::from(
            "net: eth0 lien UP mais DHCP absent ; pas de fausse configuration QEMU"
        ),
        Demarrage::Pret => format!(
            "net: eth0 {} gw {} dns {} — pret",
            ipv4::format_addr(&our_ip()), ipv4::format_addr(&gateway()),
            ipv4::format_addr(&dns_server())),
    };
    crate::kernel::dmesg::log(&ligne);
    etat
}

fn demarre_interne() -> Demarrage {
    use chronologie::Phase;
    if pci::find_network().is_none() {
        return Demarrage::SansCarte;
    }
    chronologie::phase(Phase::CarteDetectee, 0);
    chronologie::phase(Phase::PiloteInit, 0);
    if !e1000::init() {
        return Demarrage::CarteRefusee;
    }
    chronologie::phase(Phase::PiloteRret, 0);
    if !e1000::link_up() {
        // LE VERDICT DU PREMIER INSTANT N'EST PAS LA VERITE DU RESEAU.
        //
        // On le pose comme avant -- ce commit ne change aucun comportement --
        // mais on DATE desormais ce moment, pour pouvoir mesurer plus tard
        // combien de temps separe ce « lien bas » de la montee reelle.
        return Demarrage::LienBas;
    }
    chronologie::phase(Phase::LienHaut, 0);
    match dhcp::negocie() {
        Some(_) => Demarrage::Pret,
        // 10.0.2.x est une convention SLIRP QEMU, pas une configuration
        // universelle. Sur le RTL8168 physique, un DHCP absent signifie
        // simplement "hors ligne" jusqu'a configuration manuelle.
        None if e1000::using_rtl8168() => Demarrage::SansConfiguration,
        None => Demarrage::SansBail,
    }
}

// ---------------------------------------------------------------------------
// BOUCHAUD_NET_IDENTITE_V1 : de QUEL reseau s'agit-il ?
// ---------------------------------------------------------------------------
//
// Un cable n'a pas de SSID. Le seul nom qu'un reseau filaire se donne est
// celui que son serveur DHCP annonce dans l'option 15 -- « fritz.box »,
// « home », « lan ». Il etait DEJA demande dans la liste des parametres
// souhaites ; personne ne lisait la reponse.
//
// A defaut de nom, le sous-reseau en tient lieu : « 192.168.1.0/24 » identifie
// le reseau aussi surement, et c'est ce qu'affichent les outils quand le
// serveur ne nomme rien.

/// Longueur maximale du nom de reseau retenu.
const NOM_RESEAU_MAX: usize = 63;
static mut NOM_RESEAU: [u8; NOM_RESEAU_MAX] = [0; NOM_RESEAU_MAX];
static mut NOM_RESEAU_LEN: usize = 0;
static mut MASQUE: Ipv4Addr = [0, 0, 0, 0];

/// Retient ce que le bail DHCP a appris sur l'identite du reseau.
pub fn pose_identite_reseau(domaine: &[u8], masque: Ipv4Addr) {
    unsafe {
        let n = domaine.len().min(NOM_RESEAU_MAX);
        NOM_RESEAU[..n].copy_from_slice(&domaine[..n]);
        NOM_RESEAU_LEN = n;
        MASQUE = masque;
    }
}

/// Oublie l'identite du reseau : le lien est tombe, elle ne vaut plus rien.
pub fn oublie_identite_reseau() {
    unsafe {
        NOM_RESEAU_LEN = 0;
        MASQUE = [0, 0, 0, 0];
    }
}

/// Le nom du reseau, tel qu'on peut l'afficher.
///
/// Dans l'ordre : le domaine annonce par DHCP, sinon le sous-reseau, sinon la
/// passerelle, sinon rien. On ne FABRIQUE jamais un nom : « hors ligne » se lit
/// a l'etat du lien, pas a une chaine vide.
pub fn nom_reseau() -> String {
    unsafe {
        if NOM_RESEAU_LEN != 0 {
            if let Ok(nom) = core::str::from_utf8(&NOM_RESEAU[..NOM_RESEAU_LEN]) {
                return String::from(nom);
            }
        }
        let ip = our_ip();
        // La longueur de prefixe et l'adresse de reseau viennent du module pur
        // du client DHCP, celui que la suite hote met a l'epreuve. Les
        // recopier ici en ferait deux versions a corriger.
        if let Some(prefixe) = dhcp::options::longueur_prefixe(MASQUE) {
            let reseau = dhcp::options::adresse_reseau(ip, MASQUE);
            return format!("{}/{}", ipv4::format_addr(&reseau), prefixe);
        }
        if ip != [0, 0, 0, 0] {
            return ipv4::format_addr(&ip);
        }
    }
    String::new()
}

/// Ce que le lien vaut, a cet instant.
///
/// Un cable n'a pas de force de signal : sa QUALITE se lit a trois choses --
/// la vitesse negociee, le duplex, et les trames que la carte a laissees
/// tomber faute de tampon. Un lien a l'alternat sur du cuivre moderne signale
/// presque toujours une negociation ratee d'un cote, et les collisions y
/// divisent le debit utile.
pub struct QualiteLien {
    pub vitesse_mbps: u32,
    pub duplex_complet: bool,
    pub trames_perdues: u32,
}

/// Lit la qualite du lien courant.
pub fn qualite_lien() -> QualiteLien {
    QualiteLien {
        vitesse_mbps: e1000::vitesse_mbps(),
        duplex_complet: e1000::duplex_complet(),
        trames_perdues: e1000::trames_perdues(),
    }
}

/// Le reseau est-il utilisable pour joindre l'exterieur, a cet instant ?
///
/// `external_enabled()` repond sur le VERDICT de demarrage ; celle-ci repond
/// sur l'etat courant, lien compris. C'est ce que doit montrer une icone.
pub fn connecte() -> bool {
    // Meme regle que `external_enabled` -- voir son commentaire -- plus le
    // lien. Les deux ne doivent pas pouvoir diverger : un indicateur qui dit
    // « connecte » pendant que la pile dit « pas de reseau exterieur » est
    // pire que les deux reponses prises separement.
    external_enabled() && e1000::link_up()
}

/// L'interface physique est-elle presente et pilotee ?
pub fn carte_presente() -> bool {
    !matches!(etat_demarrage(), Demarrage::SansCarte | Demarrage::CarteRefusee)
}

// ---------------------------------------------------------------------------
// BOUCHAUD_NET_VEILLEUR_DE_LIEN_V1
// ---------------------------------------------------------------------------

/// Periode de relecture de l'etat du lien, en millisecondes.
///
/// Une seconde : un cable qu'on branche est vu dans la seconde, et le cout est
/// une lecture de registre par seconde.
const PERIODE_LIEN_MS: u64 = 1_000;

/// Periode de relance de l'autonegociation quand le lien est bas.
///
/// # Pourquoi il faut la RELANCER, et pas seulement attendre
///
/// Le pilote lisait l'etat du lien et n'ecrivait jamais dans le PHY. Sur la
/// machine de reference, brancher le cable APRES le demarrage ne montait rien :
/// le PHY restait dans l'etat ou la reinitialisation du controleur l'avait
/// laisse, sans negociation en cours, et le bit de lien n'est monte a aucun
/// moment de la session -- le releve du 12 septembre 17:55 ne contient pas une
/// seule ligne `NET_LIEN etat=UP`.
///
/// Quatre secondes : une autonegociation cuivre gigabit dure une a trois
/// secondes, et la relancer avant qu'elle ait fini la recommencerait
/// indefiniment.
const PERIODE_AUTONEGOCIATION_MS: u64 = 4_000;

/// Attente avant la PREMIERE reprise DHCP apres une montee de lien.
///
/// # Pourquoi dix secondes etaient beaucoup trop
///
/// Le releve du 13 septembre 00:00 montre la sequence complete :
///
/// ```text
/// 23:59:02  lien UP 1000 Mb/s duplex complet -> DHCP echoue
/// 23:59:34  lien UP (rebranchement)          -> DHCP echoue
/// 23:59:42  navigateur lance, dns=10.0.2.3   -> about:error
/// 00:00:05  eth0 192.168.1.97 gw 192.168.1.254 dns 192.168.1.254 -- pret
/// ```
///
/// Le reseau FONCTIONNE. Il a simplement mis trente et une secondes a se
/// configurer, parce que les reprises etaient espacees de dix secondes qui
/// doublaient, et que l'utilisateur a clique sur le navigateur entre-temps.
///
/// Une premiere requete perdue juste apres une montee de lien est NORMALE :
/// le commutateur en face vient d'allumer son port et n'apprend les adresses
/// qu'apres une seconde ou deux. Deux secondes, c'est le temps qu'il faut
/// pour retenter une fois que le lien porte vraiment du trafic.
const PERIODE_DHCP_MS: u64 = 2_000;

/// Budget laisse au serveur DHCP par le veilleur, par etape.
///
/// Sept cents millisecondes suffisent AU DEMARRAGE, ou l'enjeu est de ne pas
/// retarder le bureau pour un reseau qui n'existe peut-etre pas. Ici l'enjeu
/// est l'inverse : le lien vient de monter, il y a de bonnes chances qu'un
/// serveur reponde, et ce fil ne retarde rien. Quatre secondes, c'est ce que
/// la commande `dhcp` tapee a la main accorde deja.
const BUDGET_DHCP_VEILLEUR_MS: u64 = 4_000;

/// Plafond de l'attente entre deux reprises DHCP.
///
/// L'attente DOUBLE a chaque echec jusqu'a ce plafond. Un reseau cable sans
/// serveur DHCP -- un commutateur de laboratoire, une liaison directe -- est
/// une situation durable : la retenter toutes les dix secondes pendant des
/// heures est du bruit, et ne trouvera rien de plus qu'a la centieme fois.
/// Une minute reste assez court pour qu'un serveur qui demarre soit vu.
const PLAFOND_DHCP_MS: u64 = 60_000;

static VEILLEUR_LANCE: AtomicBool = AtomicBool::new(false);

/// Relit l'etat du lien et reconfigure quand il change.
///
/// # LE DEFAUT QUE CE FIL CORRIGE
///
/// `demarre()` etait appele UNE fois, et son verdict etait definitif. Le
/// releve physique du 12 septembre finit ainsi :
///
/// ```text
/// BOUCHAUD_TRIGKEY_RTL8168_DRIVER_OK mac=b0:41:6f:09:70:a1
/// BOUCHAUD_TRIGKEY_RTL8168_LINK_DOWN
/// net: lo actif ; eth0 initialisee, lien bas
/// ```
///
/// La carte est reconnue, le pilote fonctionne, et le lien est bas au moment
/// precis ou on regarde -- trois secondes apres la mise sous tension, ce qui
/// est court pour une autonegociation cuivre gigabit, et plus court encore
/// que le temps de brancher un cable. Apres quoi plus personne ne regardait :
/// la machine restait hors ligne pour la duree de la session, et le
/// navigateur repondait « Unable to resolve host » a toutes les pages.
///
/// Le fil ne fabrique aucune configuration : il ne fait que refaire ce que
/// `demarre()` fait, quand l'etat du materiel a change.
fn veilleur_de_lien() -> ! {
    let mut lien_precedent = e1000::link_up();
    let mut prochain_dhcp_ms = 0u64;
    let mut attente_dhcp_ms = PERIODE_DHCP_MS;
    let mut prochaine_negociation_ms = 0u64;
    loop {
        crate::kernel::task::sleep_ticks(
            crate::kernel::timer::ms_to_ticks(PERIODE_LIEN_MS),
        );
        let lien = e1000::link_up();
        verifie_la_reception();
        let etat = etat_demarrage();

        if lien != lien_precedent {
            lien_precedent = lien;
            crate::serial_println!(
                "BOUCHAUD_NET_LIEN etat={} ancien_verdict={}",
                if lien { "UP" } else { "DOWN" },
                nom_demarrage(etat),
            );
            if !lien {
                chronologie::phase(chronologie::Phase::LienBas, 0);
                // Le cable part : on ne garde pas une configuration qui ne
                // mene plus nulle part, sinon chaque requete part dans le vide
                // et attend son echeance.
                pose_le_verdict(Demarrage::LienBas);
                crate::kernel::sysroot::refresh_resolver();
                oublie_identite_reseau();
                oublie_voisins();
                crate::kernel::dmesg::log("net: eth0 lien tombe");
                continue;
            }
            chronologie::phase(chronologie::Phase::LienHaut, 0);
            // Le lien monte : on retente tout de suite, et la montee remet
            // l'attente a son plancher -- c'est un evenement neuf, pas la
            // suite de la serie d'echecs precedente.
            prochain_dhcp_ms = 0;
            attente_dhcp_ms = PERIODE_DHCP_MS;
            crate::kernel::dmesg::log_fmt(format_args!(
                "net: eth0 lien UP {} Mb/s duplex {}",
                e1000::vitesse_mbps(),
                if e1000::duplex_complet() { "complet" } else { "alternat" },
            ));
        }

        if matches!(etat, Demarrage::SansCarte | Demarrage::CarteRefusee) {
            continue;
        }
        if !lien {
            // LE CABLE QU'ON VIENT DE BRANCHER DEMANDE UNE NEGOCIATION.
            //
            // Regarder le bit de lien sans jamais rien demander au PHY, c'est
            // attendre un evenement que personne ne declenche.
            let maintenant = crate::kernel::timer::monotonic_ms();
            if maintenant >= prochaine_negociation_ms {
                prochaine_negociation_ms =
                    maintenant.saturating_add(PERIODE_AUTONEGOCIATION_MS);
                chronologie::phase(chronologie::Phase::PhyDemarre, 0);
                e1000::reveille_le_lien();
            }
            continue;
        }
        // SEUL UN BAIL REELLEMENT OBTENU EST UN ETAT TERMINAL.
        //
        // `SansBail` etait traite ici comme un repos, au motif que le repli
        // SLIRP « marche ». Deux mesures le refutent.
        //
        // La premiere : au demarrage, la reception ne peut pas fonctionner
        // avant environ une seconde -- sous QEMU, le `flush_queue_timer` du
        // modele e1000, arme par l'ecriture de RCTL, refuse toute trame tant
        // qu'il court, et le reecrire repousse la fenetre d'autant. Le DHCP
        // de demarrage part une milliseconde apres l'init et abandonne 700 ms
        // plus tard : il est entierement dedans. `SansBail` n'est donc pas un
        // constat sur le reseau, c'est un constat sur un instant ou l'on ne
        // pouvait rien constater.
        //
        // La seconde, et c'est celle qui tranche : le repli n'est pas une
        // mesure, c'est une constante compilee. Avec un SLIRP hors 10.0.2.x,
        // le serveur offre 192.168.76.15 et la machine pose 10.0.2.15, sur un
        // sous-reseau qui n'existe pas -- pendant que `external_enabled()`
        // annonce au reste du systeme que le reseau est utilisable. Un seul
        // DISCOVER en vingt-cinq secondes, et le verdict ne bougeait plus.
        //
        // On ne touche ni au budget de demarrage, ni a la croissance de
        // l'attente : un reseau vraiment sans serveur DHCP reste espace par
        // le meme plafond qu'avant. On retire seulement le droit de se reposer
        // sur une supposition.
        if matches!(etat, Demarrage::Pret) {
            continue;
        }
        let maintenant = crate::kernel::timer::monotonic_ms();
        if maintenant < prochain_dhcp_ms {
            continue;
        }
        prochain_dhcp_ms = maintenant.saturating_add(attente_dhcp_ms);
        let nouvel_etat = match dhcp::negocie_avant(BUDGET_DHCP_VEILLEUR_MS) {
            Some(_) => Demarrage::Pret,
            None if e1000::using_rtl8168() => Demarrage::SansConfiguration,
            None => Demarrage::SansBail,
        };
        if matches!(nouvel_etat, Demarrage::Pret) {
            attente_dhcp_ms = PERIODE_DHCP_MS;
        } else {
            attente_dhcp_ms = attente_dhcp_ms.saturating_mul(2).min(PLAFOND_DHCP_MS);
        }
        if nouvel_etat as u8 != etat as u8 {
            pose_le_verdict(nouvel_etat);
            crate::kernel::sysroot::refresh_resolver();
            crate::kernel::dmesg::log_fmt(format_args!(
                "net: eth0 {} gw {} dns {} — {}",
                ipv4::format_addr(&our_ip()),
                ipv4::format_addr(&gateway()),
                ipv4::format_addr(&dns_server()),
                nom_demarrage(nouvel_etat),
            ));
            crate::serial_println!(
                "BOUCHAUD_NET_RECONFIGURE verdict={}",
                nom_demarrage(nouvel_etat),
            );
        }
    }
}

/// Le verdict courant, en un mot, pour les releves periodiques.
pub fn nom_verdict() -> &'static str {
    nom_demarrage(etat_demarrage())
}

fn nom_demarrage(etat: Demarrage) -> &'static str {
    match etat {
        Demarrage::SansCarte => "sans-carte",
        Demarrage::CarteRefusee => "carte-refusee",
        Demarrage::LienBas => "lien-bas",
        Demarrage::SansBail => "sans-bail",
        Demarrage::SansConfiguration => "sans-configuration",
        Demarrage::Pret => "pret",
    }
}

/// Lance le veilleur de lien. Sans effet si une carte manque.
pub fn demarre_le_veilleur_de_lien() -> bool {
    if VEILLEUR_LANCE.load(core::sync::atomic::Ordering::Acquire) {
        return true;
    }
    if matches!(etat_demarrage(), Demarrage::SansCarte | Demarrage::CarteRefusee) {
        // Rien a veiller : pas de carte, ou une carte que personne ne pilote.
        return false;
    }
    if crate::kernel::task::spawn_noyau_priorite(
        veilleur_de_lien,
        "net-lien",
        crate::kernel::task::Priorite::Normale,
    ) {
        VEILLEUR_LANCE.store(true, core::sync::atomic::Ordering::Release);
        crate::serial_println!("BOUCHAUD_NET_VEILLEUR_LANCE periode_ms={}", PERIODE_LIEN_MS);
        return true;
    }
    false
}

/// Active l'interface eth0 (initialise le driver e1000).
pub fn ifup() {
    if e1000::init() {
        let m = e1000::mac();
        vga::set_color(COLOR_GREEN);
        println!("eth0 active");
        vga::set_color(COLOR_DEFAULT);
        println!("  MAC : {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5]);
        crate::print!("  inet: "); ipv4::print_addr(&our_ip()); println!("  lien={}", if e1000::link_up() { "UP" } else { "DOWN" });
    } else {
        vga::set_color(COLOR_YELLOW);
        println!("ifup: echec d'initialisation e1000 (lance QEMU avec -device e1000 -netdev user,id=n0)");
        vga::set_color(COLOR_DEFAULT);
    }
}

/// Resout l'adresse MAC d'une IP via ARP. Renvoie None en cas de timeout.
/// Nombre de requetes ARP emises avant d'abandonner.
///
/// ARP se perd : c'est une diffusion, sans accuse de reception et sans
/// retransmission dans le protocole lui-meme. Une implementation qui n'emet
/// **qu'une** requete accepte donc d'echouer chaque fois que cette trame se
/// perd — au demarrage notamment, quand la carte vient d'etre initialisee.
///
/// C'est exactement ce qu'on a observe : `arping 10.0.2.2` a expire dans une
/// execution ou `ping` et HTTP vers **ce meme hote** ont reussi juste apres.
/// Or ICMP et TCP exigent son adresse MAC : la resolution fonctionnait, seule
/// la sonde en un coup avait perdu sa trame.
///
/// Trois emissions, chacune avec sa fenetre d'ecoute. `arping(8)` de Linux fait
/// la meme chose pour la meme raison.
const ARP_TENTATIVES: u32 = 4;

/// Duree d'ecoute apres chaque emission, en millisecondes.
///
/// **Une duree, pas un nombre de tours.** L'ecoute se comptait en 1 500 000
/// iterations, ce qui n'est pas un delai : c'est une quantite de travail, et le
/// temps qu'elle represente depend de la vitesse de la machine. Sur un
/// processeur rapide les trois tentatives s'epuisaient avant qu'un aller-retour
/// ARP ait eu le temps de se faire, et le premier paquet sortant echouait en
/// `ENETUNREACH` — observe en integration continue, jamais sur la machine de
/// developpement, ce qui est la signature meme de ce defaut.
///
/// C'est la meme faute que celle corrigee dans `recvfrom` (cf.
/// `docs/ladybird/M13_DNS.md`) : un delai qui varie avec le processeur n'est
/// pas un delai.
const ARP_ECOUTE_MS: u64 = 500;

/// Cache ARP : ce que l'on sait d'un voisin, y compris qu'il ne repond pas.
///
/// # Pourquoi l'entree NEGATIVE compte autant que la positive
///
/// `arp_resolve` est une attente bornee par l'horloge : quatre tentatives de
/// 500 ms. Sans memoire d'un echec, chaque paquet a destination d'un voisin
/// muet repayait ces deux secondes. Et `TcpConn::pump` emet un accuse par
/// segment recu : une rafale de quarante segments valait quatre-vingts secondes
/// de noyau immobile. C'est exactement la panne observee sur CPU4.
///
/// Retenir l'echec transforme « deux secondes par paquet » en « deux secondes
/// par fenetre de deux secondes ». Ce n'est pas un contournement du defaut de
/// vivacite -- celui-la est corrige separement, en cedant le verrou pendant
/// l'attente et en n'attendant jamais depuis le chemin de disponibilite -- mais
/// c'est ce qui empeche le cout de se repeter sans fin.
///
/// `static mut` sous gros verrou, comme le cache DNS juste en dessous : toute
/// la pile reseau s'execute deja sous ce verrou.
struct EntreeArp {
    ip: Ipv4Addr,
    /// `None` = ce voisin n'a pas repondu (entree negative).
    mac: Option<[u8; 6]>,
    pose_a: u64,
}

static mut CACHE_ARP: Option<alloc::vec::Vec<EntreeArp>> = None;
const ARP_CACHE_MAX: usize = 64;
/// Un voisin qui repond reste valable une minute.
const ARP_TTL_MS: u64 = 60_000;
/// Un voisin muet n'est reinterroge qu'apres ce delai. Assez court pour qu'un
/// voisin qui apparait soit vu vite, assez long pour que l'attente ne se
/// repaie pas a chaque paquet.
const ARP_TTL_NEGATIF_MS: u64 = 2_000;

// ===========================================================================
// BOUCHAUD_NET_RECEPTION_UNIQUE_V1 : une carte, un seul lecteur, rien de jete
// ===========================================================================
//
// ## Le defaut, tel que le releve du 13 septembre le montre
//
// ```text
// M17_UDP_TX dst=192.168.1.254:53 src_port=49185 octets=29 parti=false
// M9_RS_STATE id=0 DNSLookup -> Error
// WebContent: Failed load of "https://example.com/", Unable to resolve host
// ```
//
// Le lien est a 1000 Mb/s duplex complet, le bail DHCP est pose, le resolveur
// est le bon : et AUCUNE requete DNS ne part. `parti=false` vient de
// `hop_mac`, qui vient de `arp_resolve`, qui n'a jamais vu la reponse de la
// passerelle. DHCP fonctionnait parce qu'il est en DIFFUSION -- il ne demande
// aucune resolution ; tout le reste, qui est en unicast, echouait.
//
// ## Pourquoi ARP ne pouvait pas aboutir
//
// La carte avait CINQ lecteurs concurrents -- `arp_resolve`, `poll_ip`,
// `dhcp::recv_avant`, `ping`, le pont smoltcp -- et chacun JETAIT ce qui ne
// l'interessait pas :
//
// * `arp_resolve` jetait toute trame non-ARP : la reponse DNS qu'un autre fil
//   attendait mourait la ;
// * `dhcp::recv_avant` jetait toute trame non-DHCP : la reponse ARP qu'un
//   autre fil attendait mourait la ;
// * `poll_ip` mettait de cote les paquets IP des autres, mais lui non plus ne
//   partageait rien avec les deux precedents.
//
// Une reponse ARP est unique : il n'y en a pas de retransmission de protocole.
// Il suffit donc qu'un autre lecteur la sorte de l'anneau une seule fois pour
// que la resolution echoue -- et qu'elle echoue POUR DE BON, puisque l'echec
// est mis en cache.
//
// ## Ce que cette version fait
//
// Un seul point sort les trames de la carte : `draine_verrouille`. Il ROUTE
// chaque trame vers celui a qui elle appartient -- cache ARP, boite DHCP,
// file IP par protocole -- et n'en jette aucune. Tous les attendeurs lisent
// ensuite leur propre boite. Personne ne mange le courrier d'un autre.
//
// Le verrou masque les interruptions : sans cela une preemption au milieu du
// routage laisserait le verrou pris par une tache qui ne tourne plus. Sa
// section critique est bornee par `TRAMES_PAR_PASSAGE`.

/// Le verrou de la reception : etat partage de la pile ET anneau de la carte.
///
/// Il protege le cache ARP, la file des paquets mis de cote et la boite DHCP.
/// Avant lui, `sendto` (sans gros verrou) et `pump_udp` (sous le domaine
/// Reseau du gros verrou) modifiaient les memes `Vec` depuis deux processeurs.
static VERROU_RECEPTION: SpinLockIrq<()> = SpinLockIrq::new(());

// Ce que le routage a vu, pour que le prochain releve physique puisse DIRE si
// la resolution marche au lieu de laisser deviner. Des compteurs relaches :
// ils ne commandent rien, ils racontent.
use core::sync::atomic::{AtomicU64, Ordering as OrdreCompteur};
// ---------------------------------------------------------------------------
// BOUCHAUD_NET_INGRESS_UNIQUE_V1 : UNE seule fonction lit la carte
// ---------------------------------------------------------------------------
//
// DEUX consommateurs lisaient l'anneau du RTL8168 : le routage maison, ici, et
// le peripherique `smoltcp` (`E1000Device::receive`). Le commentaire de tete de
// `smol_device.rs` le disait deja -- « les deux ne doivent JAMAIS tourner en
// meme temps » -- et rien ne l'empechait.
//
// Ce n'est pas seulement une course memoire. C'est une question de PROPRIETE
// DES PAQUETS. Une trame retiree de la carte par l'un n'existe plus pour
// l'autre, et une reponse ARP ne se retransmet pas : il suffit que le
// peripherique smoltcp la prenne pendant qu'une requete Ladybird tourne pour
// que la resolution maison echoue, et qu'elle echoue POUR DE BON puisque
// l'echec est mis en cache.
//
// Le releve TRIGKEY du 17 septembre en porte la signature exacte : pendant
// trente secondes, Ladybird vivant et le lien a un gigabit,
//
//     [NET-ROUTAGE] trames=100 arp=13 dhcp=2 arp_resolus=1 arp_echoues=0
//     [NET-TCP]     poignees=0 echantillons_rtt=0 syn_retransmis=0
//
// ne bouge plus d'un seul compteur.
//
// Desormais `draine_verrouille` est le SEUL lecteur physique, et `route_trame`
// repartit : cache ARP, boite DHCP, files IPv4 -- et une COPIE dans la file
// smoltcp quand une pile smoltcp est en cours.
//
// UNE COPIE, ET NON UN SOUS-ENSEMBLE. `smoltcp` tient sa propre table de
// voisins : lui cacher les reponses ARP le rendrait muet. Il lui faut le meme
// flux que la pile maison.
static mut FILE_SMOLTCP: file_trames::FileTrames = file_trames::FileTrames::neuve();

/// La file smoltcp, LE VERROU DE RECEPTION ETANT TENU.
#[allow(static_mut_refs)]
fn file_smoltcp() -> &'static mut file_trames::FileTrames {
    unsafe { &mut *core::ptr::addr_of_mut!(FILE_SMOLTCP) }
}

/// Ouvre la file smoltcp. Tant qu'elle est fermee, le routage ne recopie rien.
///
/// L'abonnement EXISTE pour que le cas courant -- aucune pile smoltcp en cours
/// -- ne paie pas une copie de trame par paquet recu.
pub fn abonne_smoltcp() {
    let _garde = VERROU_RECEPTION.lock();
    file_smoltcp().abonne();
}

/// Ferme la file smoltcp et jette ce qu'elle retenait.
pub fn desabonne_smoltcp() {
    let _garde = VERROU_RECEPTION.lock();
    file_smoltcp().desabonne();
}

/// Retire une trame pour smoltcp. Draine la carte si la file est vide.
///
/// C'est le SEUL point d'entree du peripherique smoltcp. Il ne touche plus au
/// materiel : il consomme une file logicielle alimentee par l'ingress unique.
pub fn retire_trame_smoltcp(sortie: &mut [u8]) -> Option<usize> {
    {
        let _garde = VERROU_RECEPTION.lock();
        if let Some(n) = file_smoltcp().retire(sortie) {
            return Some(n);
        }
    }
    // File vide : faire tourner l'ingress une fois, puis relire. Le drainage
    // prend le meme verrou -- on l'a donc rendu avant.
    draine_anneau();
    let _garde = VERROU_RECEPTION.lock();
    file_smoltcp().retire(sortie)
}

/// Une pile smoltcp est-elle en cours ?
pub fn smoltcp_abonnee() -> bool {
    let _garde = VERROU_RECEPTION.lock();
    file_smoltcp().abonnee()
}

/// Ce que la file smoltcp a vu passer.
pub fn compteurs_smoltcp() -> file_trames::Compteurs {
    let _garde = VERROU_RECEPTION.lock();
    file_smoltcp().compteurs()
}

static TRAMES_ROUTEES: AtomicU64 = AtomicU64::new(0);
static TRAMES_ARP: AtomicU64 = AtomicU64::new(0);
static TRAMES_DHCP: AtomicU64 = AtomicU64::new(0);
static ARP_RESOLUS: AtomicU64 = AtomicU64::new(0);
static ARP_ECHOUES: AtomicU64 = AtomicU64::new(0);
static ARP_NON_EMIS: AtomicU64 = AtomicU64::new(0);

/// Ce que le routage de reception a vu depuis le demarrage.
///
/// `(trames, arp, dhcp, arp_resolus, arp_echoues, arp_non_emis)`.
pub fn compteurs_routage() -> (u64, u64, u64, u64, u64, u64) {
    (
        TRAMES_ROUTEES.load(OrdreCompteur::Relaxed),
        TRAMES_ARP.load(OrdreCompteur::Relaxed),
        TRAMES_DHCP.load(OrdreCompteur::Relaxed),
        ARP_RESOLUS.load(OrdreCompteur::Relaxed),
        ARP_ECHOUES.load(OrdreCompteur::Relaxed),
        ARP_NON_EMIS.load(OrdreCompteur::Relaxed),
    )
}

fn cache_arp() -> &'static mut alloc::vec::Vec<EntreeArp> {
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(CACHE_ARP);
        if slot.is_none() {
            *slot = Some(alloc::vec::Vec::new());
        }
        slot.as_mut().unwrap()
    }
}

/// Ce que l'on sait de `ip`, **le verrou de reception etant tenu**.
///
/// `None` : rien (ou l'entree a expire). `Some(None)` : on sait qu'il ne
/// repond pas. `Some(Some(mac))` : on connait son adresse.
fn arp_cache_lit_verrouille(ip: Ipv4Addr) -> Option<Option<[u8; 6]>> {
    let maintenant = crate::kernel::timer::monotonic_ms();
    let file = cache_arp();
    file.retain(|e| {
        let ttl = if e.mac.is_some() { ARP_TTL_MS } else { ARP_TTL_NEGATIF_MS };
        maintenant.wrapping_sub(e.pose_a) < ttl
    });
    file.iter().find(|e| e.ip == ip).map(|e| e.mac)
}

/// Ce que l'on sait de `ip`. Prend le verrou de reception.
fn arp_cache_lit(ip: Ipv4Addr) -> Option<Option<[u8; 6]>> {
    let _garde = VERROU_RECEPTION.lock();
    arp_cache_lit_verrouille(ip)
}

fn arp_cache_pose_verrouille(ip: Ipv4Addr, mac: Option<[u8; 6]>) {
    let file = cache_arp();
    file.retain(|e| e.ip != ip);
    if file.len() >= ARP_CACHE_MAX {
        file.remove(0);
    }
    file.push(EntreeArp { ip, mac, pose_a: crate::kernel::timer::monotonic_ms() });
}

fn arp_cache_pose(ip: Ipv4Addr, mac: Option<[u8; 6]>) {
    let _garde = VERROU_RECEPTION.lock();
    arp_cache_pose_verrouille(ip, mac);
}

/// Retient qu'un voisin n'a pas repondu -- **sans ecraser une reussite**.
///
/// `arp_resolve` pose son echec a la fin de sa fenetre d'ecoute. Entre-temps,
/// le routage peut avoir appris la bonne adresse depuis une trame que ce
/// fil-ci n'a pas regardee. Poser l'echec sans regarder transformerait une
/// resolution reussie en voisin muet pour deux secondes, a chaque fois.
fn arp_cache_pose_echec(ip: Ipv4Addr) {
    let _garde = VERROU_RECEPTION.lock();
    if matches!(arp_cache_lit_verrouille(ip), Some(Some(_))) {
        return;
    }
    arp_cache_pose_verrouille(ip, None);
}

/// Oublie UN voisin, et lui seul.
///
/// # Pourquoi cela existe, et pourquoi ce n'est pas de la triche
///
/// La duree de vie d'une entree ARP positive est d'une minute. Un banc qui
/// resout la passerelle sans rien oublier ne mesurerait donc qu'une seule
/// resolution suivie de cent quatre-vingts lectures de cache : il repondrait
/// « oui » a une question qu'il n'a pas posee.
///
/// Oublier volontairement avant chaque tour est ce qui rend la mesure REELLE :
/// chaque tour part d'un cache vide pour cette adresse-la, emet une vraie
/// requete, et attend une vraie reponse. C'est l'inverse d'allonger la duree
/// de vie -- celle-la cacherait une reception morte, celle-ci l'expose.
pub fn oublie_voisin(ip: Ipv4Addr) {
    let _garde = VERROU_RECEPTION.lock();
    cache_arp().retain(|e| e.ip != ip);
}

/// Ce que le cache sait de `ip`, sans rien demander au reseau.
///
/// `None` : rien en cache. `Some(None)` : un voisin connu pour muet.
pub fn voisin_en_cache(ip: Ipv4Addr) -> Option<Option<[u8; 6]>> {
    arp_cache_lit(ip)
}

/// Resout une adresse materielle, en emettant si necessaire.
pub fn resout_voisin(ip: Ipv4Addr) -> Option<[u8; 6]> {
    arp_resolve(ip)
}

/// Duree de vie d'une entree ARP positive, en millisecondes.
///
/// Publiee pour que le banc puisse l'AFFIRMER plutot que la supposer : un
/// correctif qui la porterait a l'infini pour masquer une reception morte
/// serait vu par un test, et non decouvert six semaines plus tard sur un autre
/// reseau.
pub fn arp_ttl_ms() -> u64 {
    ARP_TTL_MS
}

/// Oublie tous les voisins connus.
///
/// Appele quand la configuration change ou que le lien tombe : une adresse
/// materielle apprise sur un reseau ne vaut rien sur un autre, et une entree
/// NEGATIVE survivrait a la reparation de ce qui l'a causee.
pub fn oublie_voisins() {
    let _garde = VERROU_RECEPTION.lock();
    cache_arp().clear();
}

/// Enregistre ce qu'une trame ARP nous apprend, quelle qu'elle soit.
///
/// Une requete comme une reponse portent `sender_ip`/`sender_mac` : un voisin
/// qui nous parle se presente, et c'est gratuit a retenir. C'est aussi ce qui
/// referme la boucle du chemin non bloquant : la requete partie sans attendre
/// trouvera sa reponse ici, au prochain passage du routage.
fn arp_apprend(paquet: &arp::Packet) {
    if paquet.sender_ip != [0, 0, 0, 0] {
        arp_cache_pose_verrouille(paquet.sender_ip, Some(paquet.sender_mac));
    }
}

/// Une pause qui ne garde pas le gros verrou.
///
/// L'attente ARP etait la plus longue du noyau qui se compte en secondes. La passer en
/// `spin_loop` gardait le verrou global tout du long : tous les autres CPU
/// s'arretaient, l'ordonnanceur ne commutait plus, et un appel systeme trivial
/// sur un autre coeur mettait deux secondes. Mesure : `POLL_BKL_PIRE_US
/// 2016000`.
///
/// `sleep_ticks` suspend la profondeur COMPLETE du verrou externe avant de
/// rendre la main, et ne restaure que celle-la au reveil. On ne l'emploie que
/// depuis une tache utilisateur : ailleurs (initialisation, contexte noyau
/// sans tache) il n'y a personne a qui ceder.
///
/// # Ce que l'appelant doit garantir
///
/// Aucun verrou tournant tenu. C'est pour cela que le chemin de disponibilite
/// (`TcpConn::pump`, appele sous le verrou du socket) n'attend JAMAIS : il
/// passe par [`send_ip_immediat`].
pub(crate) fn attente_cedante() {
    // CEDER DES QU'IL Y A QUELQU'UN A QUI CEDER.
    //
    // La condition exigeait en plus que le gros verrou soit tenu. Elle datait
    // du temps ou `sleep_ticks` l'exigeait lui-meme ; ce n'est plus vrai --
    // `sleep_ticks` releve la profondeur d'entree, quelle qu'elle soit, et
    // « zero est une profondeur comme une autre ». La consequence etait que
    // TOUS les chemins hors gros verrou tournaient a plein processeur pendant
    // leur attente : `sendto` pendant sa resolution ARP (jusqu'a deux
    // secondes), le client DHCP du veilleur de lien (quatre secondes par
    // etape), la commande `ping`. Autant de temps vole a l'interface.
    //
    // `in_user_task` dit exactement ce qu'il faut savoir ici : une tache est
    // ordonnancee sur ce processeur, donc il y a quelqu'un a qui rendre la
    // main. Pendant l'initialisation du demarrage, il n'y a personne, et on
    // tourne -- ce qui est correct, et borne par l'echeance de l'appelant.
    if crate::kernel::task::in_user_task() {
        crate::kernel::task::sleep_ticks(1);
    } else {
        core::hint::spin_loop();
    }
}

/// Resout l'adresse materielle de `target`, en attendant au plus
/// `ARP_TENTATIVES * ARP_ECOUTE_MS`.
///
/// # Ce fil ne lit plus la carte lui-meme
///
/// La version precedente vidait l'anneau a son propre compte et jetait toute
/// trame non-ARP. Deux consequences, toutes deux observees :
///
/// 1. elle detruisait la reponse DNS qu'un autre fil attendait ;
/// 2. elle ratait sa propre reponse ARP des qu'un AUTRE lecteur -- le client
///    DHCP du veilleur de lien, le `pump_udp` d'un `recvfrom` -- l'avait
///    sortie de l'anneau avant elle, ce qui arrive d'autant plus surement que
///    la reponse ARP est unique et jamais retransmise.
///
/// Desormais elle pose sa question, fait tourner le ROUTAGE commun, et
/// surveille le cache : peu importe quel fil a sorti la reponse de l'anneau,
/// elle y sera.
fn arp_resolve(target: Ipv4Addr) -> Option<[u8; 6]> {
    // Site 70 : cette attente est la seule du noyau qui se compte en secondes.
    // La marquer permet a la jauge de tenue maximale du BKL de la NOMMER au
    // lieu de rendre un nombre orphelin.
    crate::kernel::task::stall_site_set(70, u64::from(target[3]));
    for _tentative in 0..ARP_TENTATIVES {
        // ON VERIFIE QUE LA QUESTION EST PARTIE.
        //
        // `e1000::send` rend `false` quand l'anneau d'emission est plein, et
        // personne ne regardait : on attendait alors cinq cents millisecondes
        // la reponse a une question jamais posee. Draîner libere des
        // descripteurs et laisse une seconde chance.
        if !arp_demande_sans_attendre(target) {
            draine_anneau();
            if !arp_demande_sans_attendre(target) {
                ARP_NON_EMIS.fetch_add(1, OrdreCompteur::Relaxed);
                attente_cedante();
                continue;
            }
        }
        let echeance = crate::kernel::timer::monotonic_ms() + ARP_ECOUTE_MS;
        while crate::kernel::timer::monotonic_ms() < echeance {
            // Le routage commun apprend TOUTE trame ARP, requete comme
            // reponse, d'ou qu'elle vienne.
            if draine_anneau() == 0 {
                // Rien sur l'anneau : ceder, et surtout ne pas garder le gros
                // verrou pendant ce temps-la.
                attente_cedante();
            }
            if let Some(Some(mac)) = arp_cache_lit(target) {
                crate::kernel::task::stall_site_clear();
                ARP_RESOLUS.fetch_add(1, OrdreCompteur::Relaxed);
                crate::kernel::services::succes("net.arp");
                crate::kernel::services::etat(
                    "net.arp",
                    crate::kernel::services::Etat::Actif,
                );
                return Some(mac);
            }
        }
    }
    crate::kernel::task::stall_site_clear();
    ARP_ECHOUES.fetch_add(1, OrdreCompteur::Relaxed);
    crate::kernel::services::erreur("net.arp", "pas-de-reponse");
    crate::serial_println!(
        "BOUCHAUD_NET_ARP_ECHEC cible={}.{}.{}.{} tentatives={} ecoute_ms={} \
trames_routees={} trames_arp={}",
        target[0], target[1], target[2], target[3],
        ARP_TENTATIVES,
        ARP_ECOUTE_MS,
        TRAMES_ROUTEES.load(OrdreCompteur::Relaxed),
        TRAMES_ARP.load(OrdreCompteur::Relaxed),
    );
    // Retenir l'echec : c'est ce qui empeche les deux secondes de se repayer au
    // paquet suivant. Jamais au prix d'une reussite concurrente.
    arp_cache_pose_echec(target);
    None
}

/// Envoie une requete ARP et attend une reponse (commande `arping <ip>`).
pub fn arping(argc: usize, argv: &[&str; 12]) {
    if argc < 2 { println!("usage: arping <ip>"); return; }
    let target = match ipv4::parse_addr(argv[1]) {
        Some(a) => a,
        None => { println!("arping: adresse invalide"); return; }
    };
    if !e1000::is_ready() && !e1000::init() {
        println!("arping: carte reseau indisponible (essaie 'ifup')");
        return;
    }
    crate::print!("ARP qui a "); ipv4::print_addr(&target); println!(" ?");
    match arp_resolve(target) {
        Some(m) => {
            vga::set_color(COLOR_GREEN);
            crate::print!("reponse de "); ipv4::print_addr(&target);
            println!(" : {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                m[0], m[1], m[2], m[3], m[4], m[5]);
            vga::set_color(COLOR_DEFAULT);
        }
        None => println!("arping: pas de reponse (timeout)"),
    }
}

/// `ip` est-elle sur NOTRE reseau, au sens du masque que DHCP a donne ?
///
/// # Pourquoi le /24 code en dur etait un defaut
///
/// La fonction supposait que tout reseau tient dans un /24. Le bail DHCP
/// porte pourtant son masque (option 1), il est deja lu et deja retenu dans
/// `MASQUE` -- personne ne s'en servait pour router. Sur un /16 domestique ou
/// un /22 d'entreprise, chaque voisin hors des 254 premieres adresses etait
/// donc envoye a la passerelle, qui le renvoyait sur le meme cable : au mieux
/// un aller-retour inutile, au pire un voisin injoignable si la passerelle ne
/// fait pas de redirection.
///
/// Sans masque connu -- avant le bail, ou sur une configuration statique de
/// repli -- on retombe sur le /24, qui est ce que la version precedente
/// faisait toujours.
fn same_subnet(ip: &Ipv4Addr) -> bool {
    let nous = our_ip();
    let masque = unsafe { MASQUE };
    if masque == [0, 0, 0, 0] {
        return ip[0] == nous[0] && ip[1] == nous[1] && ip[2] == nous[2];
    }
    (0..4).all(|i| ip[i] & masque[i] == nous[i] & masque[i])
}

static mut IP_ID: u16 = 0x4000;
static mut GW_MAC: Option<[u8; 6]> = None;

fn next_ip_id() -> u16 {
    unsafe { IP_ID = IP_ID.wrapping_add(1); IP_ID }
}

/// MAC du prochain saut pour atteindre `dst` (cache la MAC de la passerelle).
/// Adresse materielle du prochain saut, en consultant d'abord le cache.
///
/// `bloquant = false` : on ne repond que depuis le cache, et sur une absence on
/// emet UNE requete sans l'attendre. C'est le mode obligatoire pour tout
/// appelant qui tient un verrou tournant ou qui sert un chemin de
/// disponibilite.
fn hop_mac(dst: &Ipv4Addr, bloquant: bool) -> Option<[u8; 6]> {
    let cible = if same_subnet(dst) { *dst } else { gateway() };

    match arp_cache_lit(cible) {
        Some(Some(mac)) => return Some(mac),
        Some(None) => return None, // connu muet : ne pas repayer l'attente
        None => {}
    }

    if !bloquant {
        // Une seule requete, et on rend la main : la reponse sera apprise par
        // le routage commun, et l'appelant retentera au tour suivant.
        let _ = arp_demande_sans_attendre(cible);
        return None;
    }

    let mac = arp_resolve(cible)?;
    arp_cache_pose(cible, Some(mac));
    Some(mac)
}

/// Emet une requete ARP et rend la main immediatement.
///
/// La reponse sera apprise par `arp_apprend`, appele sur chaque trame ARP que
/// `poll_ip` sort de la carte. L'appelant retentera au tour suivant : un accuse
/// TCP est cumulatif, un datagramme perdu est retransmis.
///
/// Rend `true` si la trame est REELLEMENT partie. L'ancienne version rendait
/// `()` et ignorait le verdict de `e1000::send` : une requete restee dans un
/// anneau plein etait alors indiscernable d'une requete a laquelle personne
/// n'a repondu, et l'appelant attendait une reponse a une question jamais
/// posee.
fn arp_demande_sans_attendre(target: Ipv4Addr) -> bool {
    let mac = e1000::mac();
    let mut arp_buf = [0u8; arp::PACKET_LEN];
    if arp::build(&mut arp_buf, arp::OP_REQUEST, mac, our_ip(), [0; 6], target).is_none() {
        return false;
    }
    let mut frame = [0u8; ethernet::HEADER_LEN + arp::PACKET_LEN];
    match ethernet::build_frame(
        &mut frame, ethernet::BROADCAST, mac, ethernet::ETHERTYPE_ARP, &arp_buf,
    ) {
        Some(flen) => e1000::send(&frame[..flen]),
        None => false,
    }
}

/// Emet un paquet IPv4 (`proto`/`payload`) vers `dst` via e1000.
pub(crate) fn send_ip(dst: Ipv4Addr, proto: u8, payload: &[u8]) -> bool {
    envoie(dst, proto, payload, true)
}

/// Emet sans jamais attendre une resolution ARP.
///
/// A employer partout ou l'appelant ne peut pas dormir : sous un verrou
/// tournant, ou sur un chemin de disponibilite (`poll`). Une absence de cache
/// rend `false` apres avoir emis une requete ; l'appelant retentera.
pub(crate) fn send_ip_immediat(dst: Ipv4Addr, proto: u8, payload: &[u8]) -> bool {
    envoie(dst, proto, payload, false)
}

fn envoie(dst: Ipv4Addr, proto: u8, payload: &[u8], bloquant: bool) -> bool {
    if !e1000::is_ready() && !e1000::init() { return false; }
    let mac = match hop_mac(&dst, bloquant) { Some(m) => m, None => return false };
    let mut ip = [0u8; 1500];
    let ipl = match ipv4::build_packet(&mut ip, our_ip(), dst, proto, next_ip_id(), payload) {
        Some(n) => n, None => return false,
    };
    let mut frame = [0u8; 1514];
    let fl = match ethernet::build_frame(&mut frame, mac, e1000::mac(), ethernet::ETHERTYPE_IPV4, &ip[..ipl]) {
        Some(n) => n, None => return false,
    };
    e1000::send(&frame[..fl])
}

/// Recoit un paquet IPv4 du protocole `proto` (et source optionnelle). Copie la
/// charge utile dans `out`, renvoie (source, longueur). Non bloquant.
/// Repond a une requete ARP qui nous designe.
///
/// Sans cela, un transfert un peu long s'arrete net au milieu. La passerelle
/// revalide periodiquement son entree ARP ; si personne ne repond, elle
/// l'oublie et cesse de nous router les paquets. Un petit transfert se termine
/// avant l'echeance, un gros non — d'ou un defaut qui n'apparaissait que sur
/// les gros fichiers, et jamais sur une page.
fn traite_arp(trame: &[u8]) {
    let paquet = match arp::parse(&trame[ethernet::HEADER_LEN..]) {
        Some(p) => p,
        None => return,
    };
    // Toute trame ARP nous apprend qui est son emetteur, requete comme reponse.
    // C'est ce qui referme la boucle du chemin non bloquant.
    arp_apprend(&paquet);
    if paquet.op != arp::OP_REQUEST || paquet.target_ip != our_ip() {
        return; // c'est de l'ARP, mais pas pour nous
    }

    let mac = e1000::mac();
    let mut reponse = [0u8; arp::PACKET_LEN];
    if arp::build(&mut reponse, arp::OP_REPLY, mac, our_ip(),
                  paquet.sender_mac, paquet.sender_ip).is_none() {
        return;
    }
    let mut sortie = [0u8; ethernet::HEADER_LEN + arp::PACKET_LEN];
    if let Some(longueur) = ethernet::build_frame(
        &mut sortie, paquet.sender_mac, mac, ethernet::ETHERTYPE_ARP, &reponse) {
        e1000::send(&sortie[..longueur]);
    }
}

/// Attente d'une reponse a un echo ICMP, en millisecondes.
///
/// Une duree, et non trois millions de tours de boucle : un nombre de tours
/// vaut une seconde ici et trente seconde ailleurs, ce qui est exactement le
/// defaut deja corrige dans l'ecoute ARP et dans `recvfrom`.
const PING_ATTENTE_MS: u64 = 1_000;

/// Nombre de trames examinees par appel.
///
/// Une seule trame par appel obligeait l'appelant a boucler pour trouver ce
/// qu'il attend, et faisait passer chaque paquet non pertinent pour une absence
/// de trafic. En traiter plusieurs vide l'anneau de reception plus vite, ce qui
/// compte quand l'emetteur envoie par rafales.
const TRAMES_PAR_PASSAGE: usize = 32;

/// Paquets IPv4 sortis de l'anneau mais destines a un autre appelant.
///
/// ## Le defaut que cette file corrige
///
/// M13 a corrige, au niveau des prises UDP, le fait qu'un datagramme sorti de
/// l'anneau pour le compte d'une prise et destine a une autre etait **jete**.
/// La meme faute existait un etage plus bas, dans `poll_ip` lui-meme, et pour
/// tous les protocoles : un appelant qui demandait du TCP depuis une source
/// donnee sortait de l'anneau les trames UDP, ICMP, ou TCP d'une autre
/// connexion — puis les abandonnait par un `continue`.
///
/// Une seule carte alimente toute la machine. Des qu'un navigateur a une
/// connexion TCP ouverte **et** une resolution DNS en cours — c'est-a-dire des
/// la deuxieme page, ou la premiere page qui charge une ressource d'un autre
/// hote — le `poll` de la prise TCP mangeait la reponse DNS. Le resolveur
/// attendait une reponse qui etait deja passee, et la navigation s'arretait
/// sans erreur.
///
/// Ce qui est sorti de l'anneau est donc **mis de cote** au lieu d'etre jete,
/// et le prochain appelant qui le reclame le trouve ici avant de toucher la
/// carte. C'est la meme regle que `livre_datagramme`, appliquee a l'etage IP :
/// on route, on ne jette pas.
///
/// ## Bornes
///
/// La file est bornee en nombre **et** en age. En nombre, parce qu'un paquet
/// que personne ne reclame ne doit pas faire grossir la memoire du noyau ; le
/// plus ancien part en premier. En age, parce qu'un datagramme rendu dix
/// secondes trop tard est pire qu'un datagramme perdu : le protocole a deja
/// retransmis, et la reponse tardive arrive comme un doublon. Deux secondes
/// couvrent largement l'aller-retour d'un resolveur local.
struct PaquetEnAttente {
    proto: u8,
    src: Ipv4Addr,
    charge: alloc::vec::Vec<u8>,
    depose_a: u64,
}

/// Une page reelle ouvre plusieurs connexions vers plusieurs hotes : chaque
/// pompe met alors de cote ce qui appartient aux autres, et la file doit
/// absorber une rafale entiere sans en perdre. Deux cent cinquante-six entrees
/// de 1500 octets plafonnent a 384 Kio — negligeable devant les 2 % de memoire
/// physique que la machine utilise, et bien plus sur qu'une perte silencieuse
/// que TCP paierait en retransmissions.
const ATTENTE_MAX: usize = 256;
const ATTENTE_AGE_MS: u64 = 2_000;

static mut EN_ATTENTE: Option<alloc::vec::Vec<PaquetEnAttente>> = None;

fn file_en_attente() -> &'static mut alloc::vec::Vec<PaquetEnAttente> {
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(EN_ATTENTE);
        slot.get_or_insert_with(alloc::vec::Vec::new)
    }
}

/// Reclame un paquet deja sorti de l'anneau, s'il en existe un qui corresponde.
///
/// Purge au passage ce qui a trop vieilli : c'est le seul endroit ou la file est
/// parcourue, donc le seul ou l'entretien ne coute rien de plus.
fn reclame_en_attente(
    proto: u8,
    src_filter: Option<Ipv4Addr>,
    out: &mut [u8],
) -> Option<(Ipv4Addr, usize)> {
    let file = file_en_attente();
    if file.is_empty() {
        return None;
    }
    let maintenant = crate::kernel::timer::monotonic_ms();
    file.retain(|p| maintenant.wrapping_sub(p.depose_a) < ATTENTE_AGE_MS);

    let position = file.iter().position(|p| {
        p.proto == proto && src_filter.map_or(true, |s| p.src == s)
    })?;
    let paquet = file.remove(position);
    if paquet.proto == ipv4::PROTO_UDP {
        if let Some(u) = udp::parse(&paquet.charge) {
            if sonde_dns::concerne(u.src_port, u.dst_port) {
                sonde_dns::note(sonde_dns::Barreau::SortiDeFile);
            }
        }
    }
    let m = paquet.charge.len().min(out.len());
    out[..m].copy_from_slice(&paquet.charge[..m]);
    Some((paquet.src, m))
}

// ---------------------------------------------------------------------------
// La boite aux lettres du client DHCP
// ---------------------------------------------------------------------------
//
// Une reponse DHCP arrive AVANT qu'on ait une adresse : elle est adressee a
// 255.255.255.255, et `poll_ip` ne saurait a qui la rendre. Elle a donc sa
// propre boite, que seul le client DHCP releve. Sans elle, le client devrait
// relire la carte lui-meme -- et c'est exactement ce qui lui faisait jeter les
// reponses ARP que les autres fils attendaient.

/// Reponses DHCP sorties de l'anneau et pas encore relevees.
static mut BOITE_DHCP: Option<alloc::vec::Vec<(alloc::vec::Vec<u8>, u64)>> = None;
/// Une negociation DORA n'a que deux reponses en vol ; quatre laissent la
/// place a une relance sans jamais faire grossir la memoire.
const BOITE_DHCP_MAX: usize = 4;
/// Au-dela, la reponse concerne une negociation abandonnee depuis longtemps.
const BOITE_DHCP_AGE_MS: u64 = 10_000;

fn boite_dhcp() -> &'static mut alloc::vec::Vec<(alloc::vec::Vec<u8>, u64)> {
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(BOITE_DHCP);
        slot.get_or_insert_with(alloc::vec::Vec::new)
    }
}

fn depose_dhcp_verrouille(charge: &[u8]) {
    let boite = boite_dhcp();
    if boite.len() >= BOITE_DHCP_MAX {
        boite.remove(0);
    }
    boite.push((charge.to_vec(), crate::kernel::timer::monotonic_ms()));
}

/// Releve la plus ancienne reponse DHCP en attente.
///
/// Purge au passage ce qui a trop vieilli : une reponse a une negociation
/// abandonnee ne vaut rien, et son `xid` ne correspondra de toute facon plus.
pub(crate) fn prend_dhcp(out: &mut [u8]) -> Option<usize> {
    let _garde = VERROU_RECEPTION.lock();
    let maintenant = crate::kernel::timer::monotonic_ms();
    let boite = boite_dhcp();
    boite.retain(|(_, pose_a)| maintenant.wrapping_sub(*pose_a) < BOITE_DHCP_AGE_MS);
    if boite.is_empty() {
        return None;
    }
    let (charge, _) = boite.remove(0);
    let n = charge.len().min(out.len());
    out[..n].copy_from_slice(&charge[..n]);
    Some(n)
}

// ---------------------------------------------------------------------------
// Le routage : LE seul endroit qui sort une trame de la carte
// ---------------------------------------------------------------------------

/// Sort les trames de la carte et donne chacune a qui elle appartient.
///
/// Rend le nombre de trames traitees. Zero veut dire que l'anneau est vide a
/// cet instant -- et seulement cela, depuis que le pilote ne confond plus une
/// trame abimee avec une absence de trafic.
/// LA RECEPTION EST UN ETAT DE LIEN, ELLE AUSSI.
///
/// Le veilleur regardait le bit de lien et rien d'autre. Sur le releve du
/// 17 septembre ce bit est reste a UN, a mille megabits, en duplex integral,
/// pendant que plus une seule trame n'entrait. Un lien qui porte bien et ne
/// recoit plus rien est une panne, et personne ne la regardait.
///
/// L'examen est borne en frequence par le pilote, et il ne fait RIEN tant que
/// la reception progresse. Quand il arme une reparation, c'est ce drainage-ci
/// qui l'execute -- sous `VERROU_RECEPTION`, comme tout ce qui touche a
/// l'anneau de la carte.
pub fn verifie_la_reception() {
    if e1000::demande_reparation_si_arretee() {
        draine_anneau();
    }
}

pub(crate) fn draine_anneau() -> usize {
    let _garde = VERROU_RECEPTION.lock();
    draine_verrouille()
}

fn draine_verrouille() -> usize {
    // LA REPARATION A LIEU ICI, ET NULLE PART AILLEURS.
    //
    // C'est le seul chemin qui sorte des trames de la carte en tenant
    // `VERROU_RECEPTION`. Le peripherique smoltcp lit l'anneau sans ce verrou,
    // et le veilleur de lien ne le tient pas non plus : ni l'un ni l'autre ne
    // doit reconstruire un anneau qu'un autre coeur est peut-etre en train de
    // lire. Tous deux se contentent de poser un drapeau ; c'est ici qu'il est
    // servi.
    // B3 : ENCADRER LA REPARATION, PAS SEULEMENT LA DECLENCHER.
    //
    // `repare_si_demande` rend `true` quand elle s'est executee -- une
    // INVOCATION, pas un resultat. Le verdict de reprise se rend plus tard,
    // quand le materiel a eu le temps de montrer qu'il est reparti : la
    // conclusion de la fenetre precedente passe donc AVANT l'ouverture de la
    // suivante, a chaque drainage.
    let instantane = e1000::instantane_rx();
    crate::net::rx_recuperation::conclure_si_du(&instantane);
    if e1000::repare_si_demande() {
        let (req, exec) = e1000::compteurs_reparation();
        crate::net::rx_recuperation::debut(
            "rx-silencieux",
            e1000::nom_pilote(),
            0,
            0,
            &e1000::instantane_rx(),
            req,
            exec,
        );
    }
    let mut buf = [0u8; 2048];
    let mut traitees = 0usize;
    for _ in 0..TRAMES_PAR_PASSAGE {
        let n = match e1000::receive(&mut buf) {
            Some(n) => n,
            None => break, // plus rien a lire
        };
        traitees += 1;
        crate::net::chronologie::note(&crate::net::chronologie::RX_TRAMES);
        TRAMES_ROUTEES.fetch_add(1, OrdreCompteur::Relaxed);
        route_trame(&buf[..n]);
    }
    traitees
}

/// Donne UNE trame a qui de droit. Ne jette que ce que personne n'attend.
fn route_trame(trame: &[u8]) {
    // LE TRI PASSE AVANT TOUT LE MONDE, Y COMPRIS AVANT SMOLTCP.
    //
    // La copie smoltcp se faisait sur le flux BRUT, avant le moindre tri :
    // le routage maison jette ce qu'il ne sait pas traiter -- IPv6, VLAN,
    // controle de flux -- et une trame jetee ici serait perdue pour smoltcp
    // aussi. C'etait juste tant qu'il n'y avait qu'une pile.
    //
    // Des qu'une seconde ecoute sur la meme carte, cela devient un RST
    // FRATRICIDE : smoltcp recoit un segment destine au port 2222, ne trouve
    // aucune chaussette, et repond `RST`. Le PC voit sa connexion au debugger
    // refusee par la machine elle-meme, au moment precis ou l'on cherche a
    // savoir ce qu'elle a. La reciproque est aussi vraie.
    //
    // Une pile TCP ne jette pas silencieusement : elle REPOND. Le tri ne peut
    // donc pas se faire par soustraction, et il ne peut pas se faire apres.
    //
    // LAB eteint, `trie` rend `Normale` pour tout : le comportement est alors
    // exactement celui d'avant ce module.
    let verdict = crate::net::diag_distant::trie(trame);
    if verdict.pour_normale() {
        file_smoltcp().pose(trame);
    }
    if !verdict.pour_normale() {
        // La trame appartient a la pile de diagnostic, qui l'a deja recue.
        // La donner AUSSI au routage maison la ferait traiter deux fois.
        return;
    }

    let entete = match ethernet::parse_header(trame) {
        Some(h) => h,
        None => return,
    };
    crate::net::chronologie::note(&crate::net::chronologie::RX_ETH_OK);
    match entete.ethertype {
        // L'ARP passe avant tout : c'est du service de lien, il ne doit jamais
        // etre jete au motif qu'on attendait autre chose.
        ethernet::ETHERTYPE_ARP => {
            TRAMES_ARP.fetch_add(1, OrdreCompteur::Relaxed);
            traite_arp(trame)
        }
        ethernet::ETHERTYPE_IPV4 => {
            crate::net::chronologie::note(&crate::net::chronologie::RX_IPV4_TYPE);
            route_ipv4(trame)
        }
        // IPv6, VLAN, controle de flux : la pile ne les traite pas, et les
        // retenir ne ferait que remplir la file pour personne.
        _ => {}
    }
}

fn route_ipv4(trame: &[u8]) {
    let n = trame.len();
    // BARREAU 1 : LA TRAME EST LA, ET ELLE PORTE UN PORT 53.
    //
    // Pose AVANT tout analyseur, sur une lecture minimale des ports. La
    // premiere version posait ce barreau -- et les deux suivants -- a
    // l'interieur de `udp::parse`, donc apres un `parse_header` reussi :
    // « la trame arrive mais route_ipv4 la rejette » ne pouvait jamais
    // s'afficher. Un barreau qui ne peut pas s'allumer ne mesure rien.
    let suivi = sonde_dns::ports_bruts(&trame[ethernet::HEADER_LEN..n])
        .map(|(src, dst)| sonde_dns::concerne(src, dst))
        .unwrap_or(false);
    if suivi {
        sonde_dns::note(sonde_dns::Barreau::RxEthernet);
    }

    let iph = match ipv4::parse_header(&trame[ethernet::HEADER_LEN..n]) {
        Some(h) => h,
        None => {
            crate::net::chronologie::note(&crate::net::chronologie::RX_IPV4_KO);
            return;
        }
    };
    crate::net::chronologie::note(&crate::net::chronologie::RX_IPV4_OK);
    let debut = ethernet::HEADER_LEN + iph.header_len;
    let fin = ethernet::HEADER_LEN + iph.total_len;
    if debut > fin || fin > n {
        crate::net::chronologie::note(&crate::net::chronologie::RX_BORNES_KO);
        return;
    }
    let charge = &trame[debut..fin];

    // Le bail DHCP arrive avant qu'on ait une adresse, en diffusion : aucun
    // appelant de `poll_ip` ne le reclamera jamais. Il a sa propre boite.
    if iph.proto == ipv4::PROTO_UDP {
        crate::net::chronologie::note(&crate::net::chronologie::RX_UDP_PROTO);
        match udp::parse(charge) {
            Some(u) => {
                crate::net::chronologie::note(&crate::net::chronologie::RX_UDP_OK);
                if u.dst_port == 68 {
                    crate::net::chronologie::note(&crate::net::chronologie::RX_PORT68);
                    let fin_utile = u.payload_off.saturating_add(u.payload_len);
                    if fin_utile <= charge.len() {
                        crate::net::chronologie::note(&crate::net::chronologie::RX_DEPOSE);
                        TRAMES_DHCP.fetch_add(1, OrdreCompteur::Relaxed);
                        depose_dhcp_verrouille(&charge[u.payload_off..fin_utile]);
                    }
                    return;
                }
            }
            None => { crate::net::chronologie::note(&crate::net::chronologie::RX_UDP_KO); }
        }
    }

    // PAS POUR NOUS : ON NE LE GARDE PAS.
    //
    // La carte accepte le multicast (`MAR0` tout a un) et la diffusion : sur
    // un reseau domestique ordinaire, cela veut dire un flux continu de mDNS,
    // de SSDP et d'annonces diverses. Depuis que TOUT le trafic IP passe par
    // la file d'attente, retenir ces paquets-la remplirait une file bornee de
    // courrier que personne ne reclamera jamais -- et le plus ancien part en
    // premier, donc ce serait la reponse DNS qu'on attend qui sortirait.
    if !pour_nous(&iph.dst) {
        // Le barreau 2 reste eteint : c'est ICI que route_ipv4 rejette.
        return;
    }
    // BARREAU 2 : l'en-tete IPv4 est lu, et le paquet nous est adresse.
    if suivi {
        sonde_dns::note(sonde_dns::Barreau::RxIpv4);
    }

    if suivi && iph.proto == ipv4::PROTO_UDP {
        match udp::parse(charge) {
            Some(u) => {
                // BARREAU 3 : l'en-tete UDP est lu.
                sonde_dns::note(sonde_dns::Barreau::RxUdp);
                let entete_brut =
                    &trame[ethernet::HEADER_LEN..ethernet::HEADER_LEN + iph.header_len];
                decris_la_reponse_dns(&iph, &u, entete_brut, charge);
                // BARREAU 4 : il entre dans la file.
                sonde_dns::note(sonde_dns::Barreau::MisEnFile);
            }
            // Le barreau 3 reste eteint : `udp::parse` a refuse le paquet.
            None => {}
        }
    }
    depose_en_attente_verrouille(iph.proto, iph.src, charge);
}

/// Decrit la PREMIERE reponse DNS vue, avec ses sommes de controle.
///
/// # Pourquoi les sommes comptent ici
///
/// Si la reponse arrive mais qu'une somme est fausse, ce n'est pas la meme
/// panne que si elle n'arrive pas : cela accuse le calcul de somme d'un
/// intermediaire, ou notre lecture de l'en-tete. Les deux verdicts envoient
/// chercher a des endroits opposes, et une seule ligne suffit a trancher.
fn decris_la_reponse_dns(iph: &ipv4::Header, u: &udp::Header, entete_brut: &[u8], charge: &[u8]) {
    let ipv4_juste = ipv4::somme_juste(entete_brut);
    let (udp_juste, udp_absente) = udp::somme_verdict(&iph.src, &iph.dst, charge);
    let mut sommes = 0u32;
    if ipv4_juste {
        sommes |= sonde_dns::SOMME_IPV4_JUSTE;
    }
    if udp_juste {
        sommes |= sonde_dns::SOMME_UDP_JUSTE;
    }
    if udp_absente {
        sommes |= sonde_dns::SOMME_UDP_ABSENTE;
    }
    if sonde_dns::decris_une_fois(
        iph.src,
        iph.dst,
        u.src_port,
        u.dst_port,
        u.payload_len as u16,
        sommes,
    ) {
        crate::serial_println!(
            "BOUCHAUD_DNS53_REPONSE src={}.{}.{}.{}:{} dst={}.{}.{}.{}:{} \
longueur={} somme_ipv4={} somme_udp={}",
            iph.src[0], iph.src[1], iph.src[2], iph.src[3], u.src_port,
            iph.dst[0], iph.dst[1], iph.dst[2], iph.dst[3], u.dst_port,
            u.payload_len,
            if ipv4_juste { "juste" } else { "FAUSSE" },
            if udp_absente {
                "absente"
            } else if udp_juste {
                "juste"
            } else {
                "FAUSSE"
            },
        );
    }
}

/// Ce datagramme nous est-il adresse ?
///
/// Notre adresse, la diffusion generale, la diffusion dirigee de notre
/// sous-reseau. Avant le bail, on ne sait pas encore qui on est : on accepte
/// tout, parce que refuser reviendrait a ne jamais pouvoir se configurer.
fn pour_nous(dst: &Ipv4Addr) -> bool {
    let nous = our_ip();
    if nous == [0, 0, 0, 0] || *dst == nous || *dst == [255, 255, 255, 255] {
        return true;
    }
    let masque = unsafe { MASQUE };
    if masque == [0, 0, 0, 0] {
        // Sans masque connu, on ne sait pas calculer la diffusion dirigee :
        // seul le /24 de repli est sur, et c'est ce que faisait le routage
        // avant qu'un masque soit lu.
        return dst[0] == nous[0] && dst[1] == nous[1] && dst[2] == nous[2];
    }
    (0..4).all(|i| dst[i] | masque[i] == 255 && dst[i] & masque[i] == nous[i] & masque[i])
}

/// Met de cote un paquet destine a un autre appelant.
///
/// Publie au niveau du module : la couche transport en a besoin elle aussi. Un
/// segment TCP venant du bon hote mais adresse a un **autre port local** est
/// bien arrive, il n'est simplement pas pour la connexion qui l'a sorti de
/// l'anneau — le jeter reintroduirait, entre deux connexions vers le meme
/// serveur, exactement le defaut que cette file corrige entre protocoles.
pub(crate) fn depose_en_attente(proto: u8, src: Ipv4Addr, charge: &[u8]) {
    let _garde = VERROU_RECEPTION.lock();
    depose_en_attente_verrouille(proto, src, charge);
}

fn depose_en_attente_verrouille(proto: u8, src: Ipv4Addr, charge: &[u8]) {
    let file = file_en_attente();
    if file.len() >= ATTENTE_MAX {
        file.remove(0);
    }
    file.push(PaquetEnAttente {
        proto,
        src,
        charge: charge.to_vec(),
        depose_a: crate::kernel::timer::monotonic_ms(),
    });
}

/// Rend le prochain paquet `proto` (et source optionnelle) qui nous est arrive.
///
/// Non bloquant. Le tout premier geste est de regarder ce que le routage a
/// deja mis de cote ; ensuite seulement on fait tourner le routage, qui vide
/// l'anneau et range TOUT ce qu'il y trouve.
pub(crate) fn poll_ip(proto: u8, src_filter: Option<Ipv4Addr>, out: &mut [u8]) -> Option<(Ipv4Addr, usize)> {
    let _garde = VERROU_RECEPTION.lock();
    // Ce qu'un autre appelant a sorti de l'anneau pour nous passe avant la
    // carte : c'est deja arrive, le rendre plus tard n'aurait aucun sens.
    if let Some(trouve) = reclame_en_attente(proto, src_filter, out) {
        return Some(trouve);
    }
    if draine_verrouille() == 0 {
        return None;
    }
    reclame_en_attente(proto, src_filter, out)
}

// ── Cache DNS (nom -> IPv4) ───────────────────────────────────────────────────
// Une page moderne charge des dizaines de sous-ressources sur le meme hote ;
// re-resoudre le nom a chaque fois coute un aller-retour UDP (jusqu'a 3 essais
// de 2,5M iterations). On memorise donc les resolutions reussies. Mono-thread
// (boucle GUI), borne en nombre d'entrees (purge FIFO).
static mut DNS_CACHE: Option<alloc::vec::Vec<(String, Ipv4Addr)>> = None;
const DNS_CACHE_MAX: usize = 64;

fn dns_cache_get(name: &str) -> Option<Ipv4Addr> {
    unsafe {
        let slot = &*core::ptr::addr_of!(DNS_CACHE);
        if let Some(c) = slot.as_ref() {
            if let Some((_, ip)) = c.iter().find(|(n, _)| n == name) { return Some(*ip); }
        }
    }
    None
}

fn dns_cache_put(name: &str, ip: Ipv4Addr) {
    use alloc::string::ToString;
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(DNS_CACHE);
        let c = slot.get_or_insert_with(alloc::vec::Vec::new);
        if c.iter().any(|(n, _)| n == name) { return; }
        if c.len() >= DNS_CACHE_MAX { c.remove(0); }
        c.push((name.to_string(), ip));
    }
}

/// Attente d'une reponse DNS par tentative, en millisecondes.
///
/// Un resolveur local repond en quelques millisecondes ; deux secondes
/// couvrent largement un resolveur distant, et trois tentatives donnent six
/// secondes au pire -- ce qui reste sous le delai d'un navigateur.
const DNS_ATTENTE_MS: u64 = 2_000;

/// Resout un nom d'hote en IPv4 via DNS (None en cas d'echec/timeout).
pub fn resolve(name: &str) -> Option<Ipv4Addr> {
    // Deja une IP ?
    if let Some(ip) = ipv4::parse_addr(name) { return Some(ip); }
    // Cache : evite un aller-retour DNS par sous-ressource du meme hote.
    if let Some(ip) = dns_cache_get(name) { return Some(ip); }
    if !e1000::is_ready() && !e1000::init() { return None; }

    let mut payload = [0u8; 1500];
    // Plusieurs essais : la 1re requete echoue souvent (ARP passerelle a chaud,
    // reponse DNS manquee/perdue au demarrage). Chaque essai a un ID frais.
    for _attempt in 0..3u32 {
        let id = (next_ip_id() ^ 0x1234) as u16;
        let mut q = [0u8; 256];
        let qlen = dns::build_query(&mut q, id, name)?;
        let mut udp_buf = [0u8; 300];
        let ulen = udp::build(&mut udp_buf, 0xC000, 53, &q[..qlen])?;
        send_ip(dns_server(), ipv4::PROTO_UDP, &udp_buf[..ulen]);

        // UNE ECHEANCE, ET NON DEUX MILLIONS ET DEMI DE TOURS.
        //
        // Un nombre de tours ne mesure rien : il vaut une seconde ici et
        // trente ailleurs. C'est la meme faute que celle deja corrigee dans
        // l'ecoute ARP et dans `recvfrom` -- et ici elle tournait a plein
        // processeur, sans jamais rendre la main.
        let echeance = crate::kernel::timer::monotonic_ms() + DNS_ATTENTE_MS;
        while crate::kernel::timer::monotonic_ms() < echeance {
            let recu = poll_ip(ipv4::PROTO_UDP, Some(dns_server()), &mut payload);
            let n = match recu {
                Some((_src, n)) => n,
                None => { attente_cedante(); continue; }
            };
            if let Some(u) = udp::parse(&payload[..n]) {
                if u.dst_port == 0xC000 {
                    let off = u.payload_off;
                    let fin = off.saturating_add(u.payload_len);
                    if fin <= n {
                        if let Some(ip) = dns::parse_response(&payload[off..fin], id) {
                            dns_cache_put(name, ip);
                            return Some(ip);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Commande `dns <nom>` / `nslookup`.
pub fn dns_cmd(argc: usize, argv: &[&str; 12]) {
    if argc < 2 { println!("usage: dns <nom>"); return; }
    if !e1000::is_ready() && !e1000::init() {
        println!("dns: carte reseau indisponible (essaie 'ifup')");
        return;
    }
    match resolve(argv[1]) {
        Some(ip) => { crate::print!("{} -> ", argv[1]); ipv4::print_addr(&ip); println!(""); }
        None => println!("dns: pas de reponse pour {}", argv[1]),
    }
}

/// Document brut recupere par le navigateur graphique.
pub struct Document {
    pub banner: alloc::vec::Vec<String>, // lignes de diagnostic (TLS, statut, erreurs)
    pub final_url: String,               // URL apres redirections
    pub content_type: String,
    pub body: alloc::vec::Vec<u8>,       // corps decode (dechunke + decompresse)
    pub is_html: bool,
    pub ok: bool,
}

/// Recupere une URL HTTP(S) et renvoie le document brut (corps decode), en
/// suivant les redirections. Utilise par le moteur de rendu graphique.
pub fn fetch_document(url: &str) -> Document {
    use alloc::string::ToString;
    use crate::diag::Cat;
    let t0 = crate::kernel::timer::cycles_since_boot();
    let mc = || crate::kernel::timer::cycles_since_boot().wrapping_sub(t0) / 1_000_000;
    // Temps reel (ms) calibre sur le PIT, independant de la vitesse d'emulation
    // QEMU — voir kernel::timer::calibrate(). Complementaire au Mc (comparaison
    // relative entre requetes).
    let ms = || crate::kernel::timer::cycles_to_ms(crate::kernel::timer::cycles_since_boot().wrapping_sub(t0));
    let mut banner: alloc::vec::Vec<String> = alloc::vec::Vec::new();

    // LE PIPELINE DE NAVIGATION COMMENCE ICI.
    //
    // Chaque etape se declare en entrant et en sortant : la fenetre Services
    // montre alors laquelle a echoue, au lieu de douze lignes vides.
    use crate::kernel::services::navigation::{self, Etape};
    let horloge = || crate::kernel::timer::monotonic_ns();
    navigation::debute(url, horloge());

    if !e1000::is_ready() && !e1000::init() {
        crate::dlog!(Cat::Err, "reseau indisponible (ifup) pour {}", url);
        banner.push("reseau indisponible (lance 'ifup')".to_string());
        navigation::echoue(Etape::Tcp, horloge());
        return Document { banner, final_url: url.to_string(), content_type: String::new(), body: alloc::vec::Vec::new(), is_html: false, ok: false };
    }

    let mut current = String::from(url);
    for hop in 0..8u32 {
        let (scheme, hostname, port, path) = split_url(&current);
        let (mut b, raw) = if scheme == "https" {
            // La resolution, le raccordement et la poignee vivent dans
            // `https_fetch` : on ne peut pas les separer d'ici sans le
            // reecrire. On date donc l'ensemble sur TLS, et on le dit.
            navigation::entre(Etape::Dns, horloge());
            navigation::entre(Etape::Tcp, horloge());
            navigation::entre(Etape::Tls, horloge());
            let r = tls::https_fetch(&hostname, port, &path);
            if r.raw.is_empty() {
                navigation::echoue(Etape::Tls, horloge());
            } else {
                navigation::reussit(Etape::Dns, 0, horloge());
                navigation::reussit(Etape::Tcp, 0, horloge());
                navigation::reussit(Etape::Tls, 0, horloge());
                navigation::reussit(Etape::Http, r.raw.len() as u64, horloge());
            }
            (r.banner, r.raw)
        } else {
            navigation::entre(Etape::Dns, horloge());
            let ip = match resolve(&hostname) {
                Some(ip) => { navigation::reussit(Etape::Dns, 0, horloge()); ip }
                None => { navigation::echoue(Etape::Dns, horloge()); crate::dlog!(Cat::Err, "DNS echec {} ({}Mc, {}ms)", hostname, mc(), ms()); banner.push(format!("DNS: echec pour {}", hostname)); return Document { banner, final_url: current, content_type: String::new(), body: alloc::vec::Vec::new(), is_html: false, ok: false }; }
            };
            let req = http::build_get(&hostname, &path);
            let mut resp: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
            // HTTP en clair : on passe par la VRAIE pile TCP (smoltcp, RFC 793 :
            // controle de congestion, fast-retransmit, fenetre glissante). La
            // pile maison (tcp::fetch) souffrait de pertes non recuperees sur
            // les gros transferts (retransmissions au RTO -> ~250 s pour 44 Ko) ;
            // smoltcp fait le meme transfert en < 1 s. Repli sur la pile maison
            // si smoltcp echoue (robustesse).
            navigation::entre(Etape::Tcp, horloge());
            if !transport::smol_tcp::fetch(ip, port, req.as_bytes(), &mut resp) {
                resp.clear();
                if !tcp::fetch(ip, port, req.as_bytes(), &mut resp) {
                    navigation::echoue(Etape::Tcp, horloge());
                    crate::dlog!(Cat::Err, "TCP echec {}:{} ({}Mc, {}ms)", hostname, port, mc(), ms());
                    banner.push(format!("connexion TCP echouee vers {}:{}", hostname, port));
                    return Document { banner, final_url: current, content_type: String::new(), body: alloc::vec::Vec::new(), is_html: false, ok: false };
                }
            }
            navigation::reussit(Etape::Tcp, 0, horloge());
            // En clair : pas de poignee TLS a franchir. Le dire, plutot que
            // laisser la case vide -- une etape sautee n'est pas une etape qui
            // a echoue.
            navigation::saute(Etape::Tls, "http en clair", horloge());
            navigation::reussit(Etape::Http, resp.len() as u64, horloge());
            (alloc::vec::Vec::new(), resp)
        };
        // Diagnostic TLS / handshake : remonte les lignes du sous-systeme.
        for line in &b {
            if line.contains("TLS") || line.contains("handshake") || line.contains("echoue") {
                crate::dlog!(Cat::Net, "{}", line);
            }
        }
        banner.append(&mut b);
        if raw.is_empty() {
            crate::dlog!(Cat::Err, "{} {} : reponse vide ({}Mc, {}ms)", scheme, hostname, mc(), ms());
            return Document { banner, final_url: current, content_type: String::new(), body: alloc::vec::Vec::new(), is_html: false, ok: false };
        }
        // Cookies : memorise les Set-Cookie de la reponse (y compris ceux des
        // redirections -- c'est la que Google pose CONSENT/NID).
        application::cookies::store_from_raw(&hostname, &raw);
        match http::parse_response(&raw) {
            Some(r) if r.is_redirect() && hop < 7 => {
                let loc = r.location.clone().unwrap_or_default();
                crate::dlog!(Cat::Info, "redir {} {} -> {}", r.status_code, hostname, loc);
                banner.push(format!("{} -> {}", r.status_code, loc));
                current = http::resolve_location(scheme, &hostname, &loc);
            }
            Some(r) => {
                let is_html = r.is_html();
                let ct = r.content_type.clone().unwrap_or_default();
                crate::dlog!(Cat::Net, "{} {} {} {}o {} {}Mc ({}ms)", scheme, r.status_code, hostname,
                    r.body.len(), if is_html { "html" } else { ct.as_str() }, mc(), ms());
                banner.push(r.status_line.clone());
                // Le corps est la : le telechargement a abouti. Les etapes
                // suivantes -- decodage, HTML, CSS, mise en page, peinture --
                // vivent en anneau 3 et ne traversent aucun appel systeme.
                navigation::reussit(Etape::Telechargement, r.body.len() as u64, horloge());
                navigation::termine(horloge());
                return Document { banner, final_url: current, content_type: ct, body: r.body, is_html, ok: true };
            }
            None => {
                let mut status = String::new();
                for &c in raw.iter().take_while(|&&c| c != b'\r' && c != b'\n') { status.push(c as char); }
                crate::dlog!(Cat::Warn, "reponse HTTP illisible {} : {}", hostname, status);
                navigation::echoue(Etape::Telechargement, horloge());
                banner.push(status);
                return Document { banner, final_url: current, content_type: String::new(), body: alloc::vec::Vec::new(), is_html: false, ok: false };
            }
        }
    }
    crate::dlog!(Cat::Warn, "trop de redirections pour {}", url);
    banner.push("trop de redirections".to_string());
    Document { banner, final_url: current, content_type: String::new(), body: alloc::vec::Vec::new(), is_html: false, ok: false }
}

// ── Cache de ressources (sous-ressources : CSS, JS, images) ───────────────────
// Cache global URL -> octets, partage entre toutes les pages et onglets, pour
// eviter de re-telecharger les feuilles de style / scripts / images. Borne en
// nombre d'entrees et en memoire (purge FIFO). Acces mono-thread (boucle GUI).

static mut RES_CACHE: Option<alloc::vec::Vec<(String, alloc::vec::Vec<u8>)>> = None;
const RES_CACHE_MAX_ENTRIES: usize = 128;
const RES_CACHE_MAX_BYTES: usize = 12_000_000;

// ── Budget temps des sous-ressources ───────────────────────────────────────
// Une page peut referencer des dizaines d'hotes tiers (pub, analytics, CDN de
// polices...) : chacun d'eux, s'il est injoignable ou lent, coute jusqu'a 3s
// (TLS, cf. TcpConn::fill) voire 30s (HTTP simple, cf. tcp::fetch) d'attente
// AVANT d'echouer. Sans plafond global, un budget d'images de 96 (cf. web.rs)
// peut donc a lui seul faire gonfler un chargement de quelques centaines de
// ms a plusieurs MINUTES, sans qu'aucune phase individuelle (DOM/CSS/LAYOUT)
// ne le laisse voir : c'est l'origine du "195004ms" observe en pratique alors
// que la somme des phases loggees ne faisait que ~3s. On borne donc le temps
// TOTAL consacre aux sous-ressources d'une page (CSS externe, JS, images) :
// une fois le budget epuise, les fetch_cached() suivants echouent tout de
// suite au lieu de retenter une connexion complete.
static mut SUBRES_DEADLINE_TSC: Option<u64> = None;
static mut SUBRES_LOGGED: bool = false;

/// (Re)demarre le budget temps des sous-ressources pour une nouvelle page.
/// A appeler une fois au debut du pipeline HTML->layout (voir web.rs).
pub fn start_page_budget(ms: u64) {
    let cycles = crate::kernel::timer::ms_to_cycles(ms);
    unsafe {
        SUBRES_DEADLINE_TSC = if cycles == 0 { None } else {
            Some(crate::kernel::timer::cycles_since_boot().wrapping_add(cycles))
        };
        SUBRES_LOGGED = false;
    }
}

fn subres_budget_exhausted() -> bool {
    unsafe {
        match SUBRES_DEADLINE_TSC {
            Some(deadline) if crate::kernel::timer::cycles_since_boot() >= deadline => {
                if !SUBRES_LOGGED {
                    SUBRES_LOGGED = true;
                    crate::dlog!(crate::diag::Cat::Warn, "budget reseau sous-ressources epuise : fetch restants ignores");
                }
                true
            }
            _ => false,
        }
    }
}

/// Recupere une sous-ressource (CSS/JS/image) avec mise en cache. Renvoie les
/// octets du corps si la requete reussit (ou un hit cache), sinon None.
pub fn fetch_cached(url: &str) -> Option<alloc::vec::Vec<u8>> {
    use alloc::string::ToString;
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(RES_CACHE);
        if slot.is_none() { *slot = Some(alloc::vec::Vec::new()); }
        if let Some(c) = slot.as_ref() {
            if let Some((_, b)) = c.iter().find(|(u, _)| u == url) {
                crate::dlog!(crate::diag::Cat::Cache, "HIT {}o {}", b.len(), url);
                return Some(b.clone());
            }
        }
    }
    if subres_budget_exhausted() { return None; }
    let doc = fetch_document(url);
    if !doc.ok || doc.body.is_empty() { return None; }
    let body = doc.body;
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(RES_CACHE);
        let c = slot.get_or_insert_with(alloc::vec::Vec::new);
        let mut total: usize = c.iter().map(|(_, b)| b.len()).sum();
        // Purge FIFO tant que l'ajout depasse les bornes.
        while (!c.is_empty()) && (c.len() >= RES_CACHE_MAX_ENTRIES || total + body.len() > RES_CACHE_MAX_BYTES) {
            let removed = c.remove(0);
            total = total.saturating_sub(removed.1.len());
        }
        if body.len() <= RES_CACHE_MAX_BYTES {
            c.push((url.to_string(), body.clone()));
        }
    }
    Some(body)
}

/// Vide le cache de ressources (ex. rechargement force).
pub fn clear_cache() {
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(RES_CACHE);
        if let Some(c) = slot.as_mut() { c.clear(); }
    }
    // Abandonne aussi la session HTTPS keep-alive en attente.
    tls::pool_clear();
}

/// Recupere une URL HTTP(S) et renvoie les lignes a afficher (statut + corps),
/// en suivant jusqu'a 5 redirections (301/302/303/307/308 via `Location`).
/// Utilise par la commande `wget` et par le navigateur.
pub fn http_get(url: &str) -> alloc::vec::Vec<String> {
    use alloc::string::ToString;
    let mut out: alloc::vec::Vec<String> = alloc::vec::Vec::new();

    if !e1000::is_ready() && !e1000::init() {
        out.push("reseau indisponible (lance 'ifup')".to_string());
        return out;
    }

    let mut current = String::from(url);
    for hop in 0..6u32 {
        let (scheme, hostname, port, path) = split_url(&current);

        // Recupere la reponse brute (banniere TLS eventuelle + octets HTTP).
        let (mut banner, raw) = if scheme == "https" {
            let r = tls::https_fetch(&hostname, port, &path);
            (r.banner, r.raw)
        } else {
            let ip = match resolve(&hostname) {
                Some(ip) => ip,
                None => { out.push(format!("DNS: echec pour {}", hostname)); return out; }
            };
            let req = http::build_get(&hostname, &path);
            let mut resp: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
            if !tcp::fetch(ip, port, req.as_bytes(), &mut resp) {
                out.push(format!("connexion TCP echouee vers {}:{}", hostname, port));
                return out;
            }
            (alloc::vec::Vec::new(), resp)
        };

        out.append(&mut banner);
        if !raw.is_empty() {
            application::cookies::store_from_raw(&hostname, &raw);
        }
        if raw.is_empty() {
            // La banniere contient deja la trace de diagnostic (canal muet).
            if scheme != "https" { out.push("reponse vide".to_string()); }
            return out;
        }

        match http::parse_response(&raw) {
            Some(r) if r.is_redirect() && hop < 5 => {
                let loc = r.location.clone().unwrap_or_default();
                out.push(format!("{} -> {}", r.status_code, loc));
                current = http::resolve_location(scheme, &hostname, &loc);
            }
            Some(r) => {
                let is_html = r.is_html();
                out.push(r.status_line);
                if is_html {
                    // Rendu type navigateur texte : titre, contenu sans balises,
                    // entites decodees, et liste numerotee des liens.
                    let page = html::render(&r.body, &current);
                    if !page.title.is_empty() { out.push(format!("== {} ==", page.title)); }
                    for l in page.lines {
                        out.push(l);
                        if out.len() > 200 { break; }
                    }
                    if !page.links.is_empty() {
                        out.push(String::new());
                        out.push(format!("--- {} liens ---", page.links.len()));
                        for (n, link) in page.links.iter().enumerate() {
                            out.push(format!("[{}] {}", n + 1, link));
                            if out.len() > 260 { break; }
                        }
                    }
                } else {
                    append_body_lines(&mut out, &r.body);
                }
                return out;
            }
            None => {
                let mut status = String::new();
                for &b in raw.iter().take_while(|&&b| b != b'\r' && b != b'\n') { status.push(b as char); }
                out.push(status);
                return out;
            }
        }
    }
    out.push("trop de redirections".to_string());
    out
}

// Decoupe une URL en (scheme, hostname, port, path).
fn split_url(url: &str) -> (&'static str, String, u16, String) {
    use alloc::string::ToString;
    let (rest, scheme, default_port) = if let Some(r) = url.strip_prefix("https://") {
        (r, "https", 443u16)
    } else if let Some(r) = url.strip_prefix("http://") {
        (r, "http", 80u16)
    } else {
        (url, "http", 80u16)
    };
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (hostname, port) = match host.find(':') {
        Some(i) => (&host[..i], host[i + 1..].parse::<u16>().unwrap_or(default_port)),
        None => (host, default_port),
    };
    (scheme, hostname.to_string(), port, path.to_string())
}

// Ajoute un corps de reponse a `out`, ligne par ligne (non imprimables -> '.').
fn append_body_lines(out: &mut alloc::vec::Vec<String>, body: &[u8]) {
    let mut line = String::new();
    for &b in body {
        match b {
            b'\n' => { out.push(core::mem::take(&mut line)); if out.len() > 200 { break; } }
            b'\r' => {}
            0x20..=0x7e => line.push(b as char),
            _ => line.push('.'),
        }
    }
    if !line.is_empty() { out.push(line); }
}

/// Commande `wget`/`curl`/`http`/`https <url>`.
pub fn wget_cmd(argc: usize, argv: &[&str; 12]) {
    if argc < 2 {
        println!("usage: {} <url>", argv[0]);
        return;
    }
    // La commande `https` force le schema TLS si absent.
    let url = argv[1];
    let prefixed: alloc::string::String;
    let target = if argv[0] == "https" && !url.contains("://") {
        prefixed = alloc::format!("https://{}", url);
        prefixed.as_str()
    } else {
        url
    };
    for l in http_get(target) {
        println!("{}", l);
    }
}

/// Commande `smoltest <hote> [port] [chemin]` : test manuel du backend TCP
/// experimental `transport::smol_tcp` (vraie pile RFC 793 via la crate
/// `smoltcp`, cf. commentaire en tete de ce module). Non branche par defaut
/// (fetch_document continue d'utiliser tcp.rs/TcpConn) : sert a verifier a
/// L'EXECUTION -- ce module n'a ete verifie qu'a la COMPILATION -- avant
/// d'envisager de le faire remplacer la pile maison en production.
pub fn smoltest_cmd(argc: usize, argv: &[&str; 12]) {
    use alloc::string::String;
    if argc < 2 {
        println!("usage: smoltest <hote> [port] [chemin]");
        return;
    }
    let host = argv[1];
    let port: u16 = if argc >= 3 { argv[2].parse().unwrap_or(80) } else { 80 };
    let path = if argc >= 4 { argv[3] } else { "/" };
    let ip = match resolve(host) {
        Some(ip) => ip,
        None => { println!("DNS: echec pour {}", host); return; }
    };
    println!("smoltcp: connexion a {}:{} ({}.{}.{}.{})...", host, port, ip[0], ip[1], ip[2], ip[3]);
    let req = http::build_get(host, path);
    let t0 = crate::kernel::timer::cycles_since_boot();
    let mut out: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    let ok = transport::smol_tcp::fetch(ip, port, req.as_bytes(), &mut out);
    let ms = crate::kernel::timer::cycles_to_ms(crate::kernel::timer::cycles_since_boot().wrapping_sub(t0));
    if !ok {
        println!("smoltcp: echec ({}ms)", ms);
        return;
    }
    println!("smoltcp: {} octets recus en {}ms", out.len(), ms);
    let preview = String::from_utf8_lossy(&out[..out.len().min(300)]);
    for line in preview.lines().take(8) { println!("  {}", line); }
}

/// Commande `tls [hote]` : diagnostics TLS et magasin de CA racines.
pub fn tls_cmd(argc: usize, argv: &[&str; 12]) {
    vga::set_color(COLOR_CYAN);
    println!("TLS : {}", tls::status());
    vga::set_color(COLOR_DEFAULT);
    println!("  magasin de CA racines : {} ancres de confiance", tls::roots::count());
    println!("  suite : TLS_AES_128_GCM_SHA256, groupe x25519");
    println!("  signatures : RSA PKCS#1v1.5 / RSA-PSS / ECDSA P-256/SHA-256 + P-384/SHA-384");
    if argc >= 2 {
        println!("");
        println!("test handshake https://{}/ ...", argv[1]);
        for l in tls::https_get(argv[1], 443, "/") {
            println!("{}", l);
        }
    } else {
        println!("  (astuce : 'tls example.com' teste un vrai handshake)");
        println!("  (astuce : 'tls-selftest' valide la crypto par vecteurs de reference)");
    }
}

/// Ping reel via e1000 : ARP -> ICMP echo sur 4 paquets.
fn ping_remote(target: Ipv4Addr) {
    // Adresse de niveau lien : la cible si locale, sinon la passerelle.
    let next_hop = if same_subnet(&target) { target } else { gateway() };
    let dst_mac = match arp_resolve(next_hop) {
        Some(m) => m,
        None => {
            vga::set_color(COLOR_YELLOW);
            crate::print!("ping: ARP sans reponse pour "); ipv4::print_addr(&next_hop); println!("");
            vga::set_color(COLOR_DEFAULT);
            return;
        }
    };
    let our_mac = e1000::mac();
    let payload = b"bouchaud-os-ping";
    let id = 0x4243u16;
    let mut sent = 0u32;
    let mut recv = 0u32;

    for seq in 0..4u16 {
        let mut icmp_buf = [0u8; 64];
        let il = match icmp::build(&mut icmp_buf, icmp::ECHO_REQUEST, id, seq, payload) {
            Some(n) => n, None => continue,
        };
        let mut ip_buf = [0u8; 128];
        let ipl = match ipv4::build_packet(&mut ip_buf, our_ip(), target, ipv4::PROTO_ICMP, seq, &icmp_buf[..il]) {
            Some(n) => n, None => continue,
        };
        let mut frame = [0u8; ethernet::HEADER_LEN + 128];
        let fl = match ethernet::build_frame(&mut frame, dst_mac, our_mac, ethernet::ETHERTYPE_IPV4, &ip_buf[..ipl]) {
            Some(n) => n, None => continue,
        };
        e1000::send(&frame[..fl]);
        sent += 1;

        // Attend l'echo reply correspondant.
        //
        // Par le routage commun, et non en lisant la carte : une commande de
        // diagnostic ne doit pas detruire la reponse DNS que le navigateur
        // attend au meme instant.
        let mut buf = [0u8; 2048];
        let mut got = false;
        let echeance = crate::kernel::timer::monotonic_ms() + PING_ATTENTE_MS;
        while crate::kernel::timer::monotonic_ms() < echeance {
            let recu = poll_ip(ipv4::PROTO_ICMP, Some(target), &mut buf);
            let n = match recu {
                Some((_, n)) => n,
                None => { attente_cedante(); continue; }
            };
            if let Some(m) = icmp::parse(&buf[..n]) {
                if m.msg_type == icmp::ECHO_REPLY && m.id == id && m.seq == seq {
                    recv += 1;
                    crate::print!("reponse de "); ipv4::print_addr(&target);
                    println!(" : icmp_seq={} ttl=64", seq);
                    got = true;
                    break;
                }
            }
        }
        if !got {
            println!("  delai depasse pour icmp_seq={}", seq);
        }
    }
    let lost = if sent > 0 { (sent - recv) * 100 / sent } else { 100 };
    println!("--- statistiques ping ---");
    println!("{} transmis, {} recus, {}% perdus", sent, recv, lost);
}

// ---------------------------------------------------------------------------
// ping
// ---------------------------------------------------------------------------

pub fn ping(argc: usize, argv: &[&str; 12]) {
    if argc < 2 {
        println!("usage: ping <ip>");
        return;
    }
    let target = match ipv4::parse_addr(argv[1]) {
        Some(a) => a,
        None => { println!("ping: adresse invalide (attendu a.b.c.d)"); return; }
    };

    crate::print!("PING ");
    ipv4::print_addr(&target);
    println!(" : 16 octets de donnees");

    if ipv4::is_loopback(&target) {
        ping_loopback(target);
    } else if e1000::is_ready() || e1000::init() {
        // Ping reel via la carte e1000.
        ping_remote(target);
    } else {
        vga::set_color(COLOR_YELLOW);
        println!("ping: carte reseau indisponible");
        vga::set_color(COLOR_DEFAULT);
        match pci::find_network() {
            Some(d) => println!("  eth0 {:04x}:{:04x} detectee ; lance 'ifup' (QEMU -device e1000 -netdev user,id=n0)", d.vendor, d.device),
            None => println!("  aucune carte reseau PCI; lance QEMU avec -device e1000 -netdev user,id=n0"),
        }
    }
}

/// Envoie 4 echo requests sur loopback en passant par la vraie pile ICMP.
fn ping_loopback(target: Ipv4Addr) {
    let payload = b"bouchaud-os-ping";
    let mut sent = 0u32;
    let mut recv = 0u32;

    for seq in 0..4u16 {
        let mut icmp_buf = [0u8; 64];
        let il = match icmp::build(&mut icmp_buf, icmp::ECHO_REQUEST, 0x4243, seq, payload) {
            Some(n) => n,
            None => continue,
        };
        let mut pkt = [0u8; 128];
        let pl = match ipv4::build_packet(&mut pkt, LO_ADDR, target, ipv4::PROTO_ICMP, seq, &icmp_buf[..il]) {
            Some(n) => n,
            None => continue,
        };
        sent += 1;

        // Le paquet "boucle" : il est traite par notre propre moteur de pile.
        let mut out = [0u8; 128];
        if let Some(rl) = stack::handle_ipv4(&pkt[..pl], &mut out) {
            if let Some(h) = ipv4::parse_header(&out[..rl]) {
                let reply = &out[h.header_len..h.total_len];
                if let Some(m) = icmp::parse(reply) {
                    if m.msg_type == icmp::ECHO_REPLY && m.seq == seq {
                        recv += 1;
                        crate::print!("{} octets de ", h.total_len);
                        ipv4::print_addr(&h.src);
                        println!(": icmp_seq={} ttl=64 temps<1ms (loopback)", seq);
                    }
                }
            }
        }
    }

    let lost = if sent > 0 { (sent - recv) * 100 / sent } else { 100 };
    println!("--- statistiques ping ---");
    println!("{} paquets transmis, {} recus, {}% perdus", sent, recv, lost);
}

// ---------------------------------------------------------------------------
// ifconfig / ip / route / arp
// ---------------------------------------------------------------------------

fn print_eth0_state() {
    if e1000::is_ready() {
        let m = e1000::mac();
        crate::print!("eth0: flags=<UP,RUNNING>  inet ");
        ipv4::print_addr(&our_ip());
        println!("  lien={}", if e1000::link_up() { "UP" } else { "DOWN" });
        println!("      ether {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            m[0], m[1], m[2], m[3], m[4], m[5]);
        return;
    }
    match pci::find_network() {
        Some(d) => {
            crate::print!("eth0: flags=<DOWN>  carte PCI {:04x}:{:04x} (", d.vendor, d.device);
            crate::print!("{}", pci::vendor_name(d.vendor));
            println!(") - driver non charge (lance 'ifup')");
        }
        None => println!("eth0: absente (aucune carte reseau PCI detectee)"),
    }
}

pub fn ifconfig() {
    crate::print!("lo: flags=<UP,LOOPBACK,RUNNING>  inet ");
    ipv4::print_addr(&LO_ADDR);
    println!("  netmask 255.0.0.0");
    print_eth0_state();
}

pub fn ip_cmd() {
    println!("1: lo: <UP,LOOPBACK>");
    crate::print!("   inet ");
    ipv4::print_addr(&LO_ADDR);
    println!("/8 scope host lo");
    println!("2: eth0:");
    print_eth0_state();
}

pub fn route_cmd() {
    println!("Table de routage IPv4:");
    println!("  Destination     Masque          Interface");
    println!("  127.0.0.0       255.0.0.0       lo");
    println!("  (pas de route par defaut: eth0 DOWN, driver NIC non charge)");
}

pub fn arp_cmd() {
    println!("Cache ARP:");
    println!("  Adresse         HWaddr             Iface");
    println!("  (vide: aucune trame Ethernet emise tant que le driver NIC manque)");
}

// ---------------------------------------------------------------------------
// Roadmap + placeholders (couches non encore actives)
// ---------------------------------------------------------------------------

/// Affiche la feuille de route OSI (commande `roadmap`, section reseau).
pub fn print_roadmap() {
    vga::set_color(COLOR_CYAN);
    println!("pile reseau OSI:");
    vga::set_color(COLOR_DEFAULT);
    println!("  L2 Ethernet    encode/decode                   [code OK]");
    println!("  ARP            encode/decode                   [code OK]");
    println!("  L3 IPv4        en-tete + checksum              [code OK]");
    println!("  ICMP           echo (ping loopback)            [actif sur lo]");
    println!("  interface lo   127.0.0.1                       [active]");
    println!("  driver NIC     e1000/virtio-net (RX/TX DMA)    [a ecrire]");
    println!("  UDP/DHCP/DNS                                   [actif]");
    println!("  TCP/HTTP                                       [actif]");
    println!("  TLS 1.3  X25519+AES-GCM+SHA256+HKDF            [actif]");
    println!("  X.509    ASN.1/DER + chaine RSA/ECDSA          [actif]");
}

fn missing_layer(cmd: &str) -> &'static str {
    match cmd {
        "dhcp" => "driver NIC + UDP + client DHCP",
        "dns" => "driver NIC + UDP + resolveur DNS",
        "wget" | "curl" => "driver NIC + TCP + HTTP",
        _ => "driver NIC + couches superieures",
    }
}

/// Message standard d'une commande reseau pas encore active.
pub fn placeholder(cmd: &str) {
    vga::set_color(COLOR_YELLOW);
    println!("{}: non disponible (couches superieures non actives)", cmd);
    vga::set_color(COLOR_DEFAULT);
    println!("  couche manquante: {}", missing_layer(cmd));
    match pci::find_network() {
        Some(d) => println!("  note: carte reseau {:04x}:{:04x} detectee, driver a ecrire", d.vendor, d.device),
        None => println!("  note: aucune carte reseau PCI (essaie QEMU avec -device e1000)"),
    }
}
