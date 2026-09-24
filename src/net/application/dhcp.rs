//! Client DHCP (DORA : Discover/Offer/Request/Ack) sur UDP.
//!
//! Obtient automatiquement l'adresse IP, la passerelle et le DNS aupres du
//! serveur DHCP du reseau (SLIRP sous QEMU), puis applique la configuration.

use crate::arch::x86_64::cpu;
use crate::drivers::e1000;
use crate::net::ipv4::Ipv4Addr;
use crate::net::{self, ethernet, ipv4, udp};

pub mod options;
use options::{build_msg, parse_reply, Lease, LONGUEUR_DOMAINE};

/// Emet un message DHCP en diffusion (broadcast L2 + IP).
fn send(mac: [u8; 6], msg: &[u8]) -> bool {
    let mut udp_buf = [0u8; 600];
    let ulen = match udp::build(&mut udp_buf, 68, 67, msg) { Some(n) => n, None => return false };
    let mut ip = [0u8; 700];
    let ipl = match ipv4::build_packet(&mut ip, [0, 0, 0, 0], [255, 255, 255, 255], ipv4::PROTO_UDP, 0, &udp_buf[..ulen]) {
        Some(n) => n, None => return false,
    };
    let mut frame = [0u8; 760];
    let fl = match ethernet::build_frame(&mut frame, ethernet::BROADCAST, mac, ethernet::ETHERTYPE_IPV4, &ip[..ipl]) {
        Some(n) => n, None => return false,
    };
    e1000::send(&frame[..fl])
}

/// Attend une reponse DHCP du bon xid (et type voulu si != 0).
fn recv(xid: u32, want_type: u8) -> Option<Lease> {
    recv_avant(xid, want_type, ATTENTE_COMMANDE_MS)
}

/// Delai laisse a un serveur DHCP par la commande `dhcp`, tapee par quelqu'un
/// qui attend une reponse et preferera patienter plutot que d'echouer trop tot.
pub const ATTENTE_COMMANDE_MS: u64 = 4_000;

/// Delai laisse par l'initialisation du demarrage.
///
/// Court, et volontairement. Un serveur DHCP repond en quelques millisecondes ;
/// un serveur qui met plusieurs secondes est un serveur qui n'est pas la. La
/// premiere version attendait deux fois huit millions de tours de boucle sans
/// horloge — environ quinze secondes — et les payait **a chaque demarrage** sur
/// un reseau qui n'a pas de serveur DHCP. Sous QEMU/SLIRP, ou l'adressage est
/// fixe et connu d'avance, cela retardait la banniere de quinze secondes pour
/// ne rien apprendre.
pub const ATTENTE_DEMARRAGE_MS: u64 = 700;

/// Attend une reponse, au plus `budget_ms` millisecondes.
///
/// Le temps se lit sur l'horloge du noyau et non sur un compteur de tours : un
/// nombre de tours ne mesure rien, puisqu'il vaut quinze secondes sous
/// emulation et une fraction de seconde sur une vraie machine. C'est la meme
/// boucle qui rendait le demarrage lent ici et rapide ailleurs, sans que le
/// code ne dise laquelle des deux etait voulue.
fn recv_avant(xid: u32, want_type: u8, budget_ms: u64) -> Option<Lease> {
    let issue = recv_avant_interne(xid, want_type, budget_ms);
    let duree = crate::kernel::timer::monotonic_ms()
        .saturating_sub(DEBUT_ATTENTE.load(Ordering::Relaxed));
    crate::net::chronologie::trace_rx(
        TENTATIVES.load(Ordering::Relaxed),
        xid,
        match (&issue, want_type) {
            (Some(_), 2) => "offre",
            (Some(_), 5) => "accuse",
            (Some(_), _) => "recu",
            (None, 2) => "timeout_offre",
            (None, 5) => "timeout_accuse",
            (None, _) => "timeout",
        },
        duree,
    );
    issue
}

fn recv_avant_interne(xid: u32, want_type: u8, budget_ms: u64) -> Option<Lease> {
    let mut buf = [0u8; 1024];
    let debut = crate::kernel::timer::monotonic_ms();
    DEBUT_ATTENTE.store(debut, Ordering::Relaxed);
    let limite = debut.saturating_add(budget_ms);
    let mut fallback_spins = 0usize;

    while crate::kernel::timer::monotonic_ms() < limite
        && fallback_spins < 10_000_000
    {
        fallback_spins = fallback_spins.saturating_add(1);

        // Faire tourner le routage commun, puis relever NOTRE boite.
        //
        // La version precedente lisait la carte elle-meme et jetait toute
        // trame qui n'etait pas une reponse DHCP. Comme le veilleur de lien
        // tient cette boucle quatre secondes d'affilee, elle detruisait les
        // reponses ARP que le reste du systeme attendait au meme instant --
        // et une reponse ARP n'est jamais retransmise. C'est la deuxieme
        // moitie du `parti=false` observe sur la resolution DNS.
        if net::draine_anneau() == 0 {
            net::attente_cedante();
        }
        while let Some(n) = net::prend_dhcp(&mut buf) {
            crate::net::chronologie::note(&crate::net::chronologie::RX_PRIS);
            if n < 8 {
                crate::net::chronologie::note(&crate::net::chronologie::RX_TROP_COURT);
                continue;
            }
            // verifie xid + BOOTREPLY
            if buf[0] != 2 {
                crate::net::chronologie::note(&crate::net::chronologie::RX_PAS_REPLY);
                continue;
            }
            // BANC : falsifier le xid d'une reponse QUI AURAIT ETE ACCEPTEE.
            //
            // La premiere version falsifiait la premiere reponse venue, et le
            // banc ne distinguait plus rien : une OFFRE perimee relachee par
            // l'emulateur -- voir B2 -- produit deja `xid_ko=1` sans aucune
            // injection, si bien que le cas injecte et le cas normal rendaient
            // exactement la meme trace. En ne touchant que ce qui correspond,
            // chaque falsification appliquee TRANSFORME une acceptation en
            // rejet, et son effet devient attribuable.
            //
            // Le tampon est reellement modifie : c'est le client, non le banc,
            // qui constate le desaccord et rejette.
            if u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) == xid
                && banc::fausse_un_xid()
            {
                buf[4] ^= 0xFF;
            }
            let rxid = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
            if rxid != xid {
                crate::net::chronologie::note(&crate::net::chronologie::RX_XID_KO);
                continue;
            }
            // BANC : perdre une OFFRE qui serait sinon acceptee. Elle est
            // jetee ICI, apres le controle de xid, pour que le compteur
            // d'injections appliquees ne compte que des offres qui auraient
            // vraiment abouti.
            if want_type == 2 && banc::jette_une_offre() {
                continue;
            }
            if let Some(l) = parse_reply(&buf[..n]) {
                if want_type == 0 || l.msg_type == want_type {
                    crate::net::chronologie::note(&crate::net::chronologie::RX_ACCEPTE);
                    return Some(l);
                }
                crate::net::chronologie::note(&crate::net::chronologie::RX_TYPE_KO);
            }
        }
    }
    None
}

/// Ce qu'une negociation reussie a rapporte.
#[derive(Clone, Copy)]
pub struct Bail {
    pub ip: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub dns: Ipv4Addr,
}

/// Negocie un bail et applique la configuration. **Sans rien afficher.**
///
/// Separe de `run` parce que les deux appelants n'ont pas le meme besoin : la
/// commande `dhcp` raconte chaque etape a l'utilisateur qui l'a tapee, tandis
/// que l'initialisation du demarrage ne doit produire qu'une ligne de journal.
/// Tant qu'il n'y avait qu'un appelant, la question ne se posait pas ; le
/// demarrage automatique en a fait un second.
// ---------------------------------------------------------------------------
// DORA, COMPTEE
// ---------------------------------------------------------------------------
//
// # Ce que le releve du 18 septembre ne disait pas
//
// L'archive montre onze emissions de 342 octets et `dhcp_vues=0`. Onze
// DISCOVER, aucune reponse -- mais rien ne permettait de dire OU la
// negociation s'arretait : pas d'offre du tout, une offre au mauvais xid, ou
// un REQUEST sans ACK. Trois pannes differentes, trois enquetes differentes,
// et le meme silence.
//
// Quatre compteurs et un etat, sans un seul vidage de paquet.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Debut de l'attente en cours, pour que la trace porte sa duree reelle.
static DEBUT_ATTENTE: AtomicU64 = AtomicU64::new(0);
static DISCOVER_ENVOYES: AtomicU64 = AtomicU64::new(0);
static OFFRES_VUES: AtomicU64 = AtomicU64::new(0);
static REQUESTS_ENVOYES: AtomicU64 = AtomicU64::new(0);
static ACKS_VUS: AtomicU64 = AtomicU64::new(0);
static DERNIER_XID: AtomicU32 = AtomicU32::new(0);
static TENTATIVES: AtomicU64 = AtomicU64::new(0);
static DERNIERE_ETAPE: AtomicU32 = AtomicU32::new(0);

/// Ou la negociation s'est arretee la derniere fois.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum Etape {
    /// Jamais tentee.
    Jamais = 0,
    /// DISCOVER emis, on attend une offre.
    Discover = 1,
    /// Offre recue, REQUEST emis, on attend l'accuse.
    Request = 2,
    /// Bail obtenu.
    Bail = 3,
    /// Aucune offre n'est venue.
    SansOffre = 4,
    /// L'offre est venue, l'accuse non.
    SansAccuse = 5,
}

impl Etape {
    pub fn nom(self) -> &'static str {
        match self {
            Etape::Jamais => "jamais",
            Etape::Discover => "discover",
            Etape::Request => "request",
            Etape::Bail => "bail",
            Etape::SansOffre => "sans-offre",
            Etape::SansAccuse => "sans-accuse",
        }
    }

    fn depuis(valeur: u32) -> Etape {
        match valeur {
            1 => Etape::Discover,
            2 => Etape::Request,
            3 => Etape::Bail,
            4 => Etape::SansOffre,
            5 => Etape::SansAccuse,
            _ => Etape::Jamais,
        }
    }
}

/// L'etat de la negociation, pour le diagnostic et la fenetre Services.
pub struct Compteurs {
    pub discover_envoyes: u64,
    pub offres_vues: u64,
    pub requests_envoyes: u64,
    pub acks_vus: u64,
    pub dernier_xid: u32,
    pub tentatives: u64,
    pub derniere_etape: Etape,
}

pub fn compteurs() -> Compteurs {
    Compteurs {
        discover_envoyes: DISCOVER_ENVOYES.load(Ordering::Relaxed),
        offres_vues: OFFRES_VUES.load(Ordering::Relaxed),
        requests_envoyes: REQUESTS_ENVOYES.load(Ordering::Relaxed),
        acks_vus: ACKS_VUS.load(Ordering::Relaxed),
        dernier_xid: DERNIER_XID.load(Ordering::Relaxed),
        tentatives: TENTATIVES.load(Ordering::Relaxed),
        derniere_etape: Etape::depuis(DERNIERE_ETAPE.load(Ordering::Relaxed)),
    }
}

fn note_etape(etape: Etape) {
    DERNIERE_ETAPE.store(etape as u32, Ordering::Relaxed);
}

pub fn negocie() -> Option<Bail> {
    negocie_avant(ATTENTE_DEMARRAGE_MS)
}

/// `negocie`, avec un budget d'attente explicite par etape.
pub fn negocie_avant(budget_ms: u64) -> Option<Bail> {
    if !e1000::is_ready() && !e1000::init() {
        return None;
    }
    let mac = e1000::mac();
    let xid = cpu::rdtsc() as u32;
    let mut msg = [0u8; 400];

    TENTATIVES.fetch_add(1, Ordering::Relaxed);
    DERNIER_XID.store(xid, Ordering::Relaxed);

    let l = build_msg(&mut msg, xid, mac, 1, None, None);
    send(mac, &msg[..l]);
    DISCOVER_ENVOYES.fetch_add(1, Ordering::Relaxed);
    note_etape(Etape::Discover);
    crate::net::chronologie::phase(crate::net::chronologie::Phase::DhcpDiscover, xid);
    let Some(offer) = recv_avant(xid, 2, budget_ms) else {
        // AUCUNE OFFRE. C'est le cas du releve : onze DISCOVER, pas une
        // reponse. Le dire ici evite de confondre avec un REQUEST sans ACK.
        note_etape(Etape::SansOffre);
        return None;
    };
    OFFRES_VUES.fetch_add(1, Ordering::Relaxed);
    crate::net::chronologie::phase(crate::net::chronologie::Phase::DhcpOffre, xid);

    let l = build_msg(&mut msg, xid, mac, 3, Some(offer.your_ip), Some(offer.server_id));
    send(mac, &msg[..l]);
    REQUESTS_ENVOYES.fetch_add(1, Ordering::Relaxed);
    note_etape(Etape::Request);
    crate::net::chronologie::phase(crate::net::chronologie::Phase::DhcpRequest, xid);
    let Some(ack) = recv_avant(xid, 5, budget_ms) else {
        note_etape(Etape::SansAccuse);
        return None;
    };
    ACKS_VUS.fetch_add(1, Ordering::Relaxed);
    note_etape(Etape::Bail);
    crate::net::chronologie::phase(crate::net::chronologie::Phase::DhcpAck, xid);

    // Valeurs de repli : un serveur qui n'annonce ni routeur ni resolveur
    // laisse la configuration compilee en place plutot que de poser 0.0.0.0,
    // qui donnerait une interface configuree et injoignable.
    let gateway = if ack.router == [0, 0, 0, 0] { net::gateway() } else { ack.router };
    let dns = if ack.dns == [0, 0, 0, 0] { net::dns_server() } else { ack.dns };
    net::set_config(ack.your_ip, gateway, dns);
    net::pose_identite_reseau(&ack.domaine[..ack.domaine_len], ack.masque);
    crate::net::chronologie::phase(crate::net::chronologie::Phase::Ipv4Prete, xid);
    Some(Bail { ip: ack.your_ip, gateway, dns })
}

/// Commande `dhcp` : configuration automatique de l'interface.
pub fn run() {
    if !e1000::is_ready() && !e1000::init() {
        crate::println!("dhcp: carte reseau indisponible (essaie 'ifup')");
        return;
    }
    let mac = e1000::mac();
    let xid = cpu::rdtsc() as u32;
    let mut msg = [0u8; 400];

    // DISCOVER
    let l = build_msg(&mut msg, xid, mac, 1, None, None);
    send(mac, &msg[..l]);
    crate::println!("DHCP: DISCOVER envoye...");
    let offer = match recv(xid, 2) {
        Some(o) => o,
        None => { crate::println!("dhcp: pas d'OFFER (timeout)"); return; }
    };
    crate::print!("DHCP: OFFER "); ipv4::print_addr(&offer.your_ip); crate::println!("");

    // REQUEST
    let l = build_msg(&mut msg, xid, mac, 3, Some(offer.your_ip), Some(offer.server_id));
    send(mac, &msg[..l]);
    let ack = match recv(xid, 5) {
        Some(a) => a,
        None => { crate::println!("dhcp: pas d'ACK (timeout)"); return; }
    };

    // Applique la configuration (avec valeurs de repli).
    let gw = if ack.router == [0, 0, 0, 0] { net::gateway() } else { ack.router };
    let dns = if ack.dns == [0, 0, 0, 0] { net::dns_server() } else { ack.dns };
    net::set_config(ack.your_ip, gw, dns);
    net::pose_identite_reseau(&ack.domaine[..ack.domaine_len], ack.masque);

    crate::print!("DHCP: bail obtenu  inet "); ipv4::print_addr(&ack.your_ip);
    crate::print!("  gw "); ipv4::print_addr(&gw);
    crate::print!("  dns "); ipv4::print_addr(&dns);
    crate::println!("");
}

/// BANC UNIQUEMENT : rendre DETERMINISTES les pannes DHCP.
///
/// # Pourquoi armer un NOMBRE et compter les APPLICATIONS
///
/// Un scenario negatif qui passe parce que la panne n'a pas ete injectee est
/// un vert qui ne defend rien. Ces deux compteurs sont donc separes :
/// `*_ARMEES` dit ce qu'on a demande, `*_APPLIQUEES` dit ce qui a REELLEMENT
/// eu lieu. Le banc compare les deux, et l'ecart est un echec.
///
/// Les deux injections agissent la ou le client lit, pas sur le fil : c'est
/// exactement ce qu'il faut pour eprouver le CLIENT. La panne du fil, elle,
/// est deja couverte par `run_dhcp_wire.sh`.
pub mod banc {
    use core::sync::atomic::{AtomicU32, Ordering};

    static OFFRES_A_JETER: AtomicU32 = AtomicU32::new(0);
    static OFFRES_JETEES: AtomicU32 = AtomicU32::new(0);
    static XID_A_FAUSSER: AtomicU32 = AtomicU32::new(0);
    static XID_FAUSSES: AtomicU32 = AtomicU32::new(0);

    /// Arme la perte des `n` prochaines OFFRES qui auraient ete acceptees.
    pub fn arme_pertes(n: u32) {
        OFFRES_A_JETER.store(n, Ordering::Relaxed);
    }

    /// Arme la falsification du xid des `n` prochaines reponses recues.
    pub fn arme_xid_faux(n: u32) {
        XID_A_FAUSSER.store(n, Ordering::Relaxed);
    }

    pub(super) fn jette_une_offre() -> bool {
        if OFFRES_A_JETER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                if v == 0 { None } else { Some(v - 1) }
            })
            .is_err()
        {
            return false;
        }
        OFFRES_JETEES.fetch_add(1, Ordering::Relaxed);
        true
    }

    pub(super) fn fausse_un_xid() -> bool {
        if XID_A_FAUSSER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                if v == 0 { None } else { Some(v - 1) }
            })
            .is_err()
        {
            return false;
        }
        XID_FAUSSES.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Ce qui restait a injecter, et ce qui l'a ete. Le banc lit cette ligne.
    pub fn rapporte(etiquette: &str) {
        crate::kernel::dmesg::log_fmt(format_args!(
            "DHCP_BANC_INJECTION scenario={} offres_restantes={} offres_jetees={} \
xid_restants={} xid_fausses={}",
            etiquette,
            OFFRES_A_JETER.load(Ordering::Relaxed),
            OFFRES_JETEES.load(Ordering::Relaxed),
            XID_A_FAUSSER.load(Ordering::Relaxed),
            XID_FAUSSES.load(Ordering::Relaxed),
        ));
    }
}
