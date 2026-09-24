//! BOUCHAUD_C74_CHRONOLOGIE_DU_DEMARRAGE_RESEAU
//!
//! # Pourquoi cette chronologie existe
//!
//! La session physique du Trigkey sur `d131f2a` a ete extraite. Elle donne,
//! pour la premiere fois, un AVANT mesure :
//!
//!     lien=1  vitesse_mbps=1000  duplex=1
//!     discover_sent=9  offer_seen=1  request_sent=1  ack_seen=1
//!     rx_stall=39  repair_req=39  repair_exec=39  recoveries=39
//!     rx_ok_sans_desc=81  rx_ok_without_progress=54
//!     nic_resets=0
//!
//! Le lien etait donc etabli a 1000 Mb/s full duplex, et le meme RTL8168 a
//! ensuite traite des milliers de paquets. La carte n'est pas cassee.
//!
//! Ce que ces compteurs NE disent pas, et qui est toute la question :
//!
//!     pourquoi les DISCOVER #1 a #8 n'obtiennent-ils aucune OFFRE,
//!     alors que le #9 en obtient une ?
//!
//! Des cumuls ne repondent jamais a cela. Il faut une chronologie : QUAND
//! chaque etape a eu lieu, et dans quel etat etait la reception a ce
//! moment-la. C'est ce que ce module pose, et RIEN D'AUTRE -- aucun
//! comportement reseau n'est modifie ici.
//!
//! # Une seule horloge
//!
//! `monotonic_ns()` partout. Les deltas se calculent a la relecture, pas a
//! l'emission : une ligne qui porte sa propre soustraction ment des qu'on la
//! filtre.

use core::sync::atomic::{AtomicU64, Ordering};

/// Les etapes du demarrage reseau, dans l'ordre ou elles doivent survenir.
///
/// `LienBas` et `LienHaut` peuvent se repeter : un cable qu'on debranche et
/// rebranche produit plusieurs montees, et chacune ouvre une negociation
/// neuve. Les autres etapes se repetent aussi -- neuf DISCOVER dans le releve
/// physique -- et c'est precisement ce qu'on veut pouvoir compter.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    CarteDetectee,
    PiloteInit,
    PiloteRret,
    PhyDemarre,
    LienHaut,
    LienBas,
    DhcpDiscover,
    DhcpOffre,
    DhcpRequest,
    DhcpAck,
    Ipv4Prete,
}

impl Phase {
    pub const fn nom(self) -> &'static str {
        match self {
            Phase::CarteDetectee => "NIC_DETECTED",
            Phase::PiloteInit => "DRIVER_INIT",
            Phase::PiloteRret => "DRIVER_READY",
            Phase::PhyDemarre => "PHY_START",
            Phase::LienHaut => "LINK_UP",
            Phase::LienBas => "LINK_DOWN",
            Phase::DhcpDiscover => "DHCP_DISCOVER",
            Phase::DhcpOffre => "DHCP_OFFER",
            Phase::DhcpRequest => "DHCP_REQUEST",
            Phase::DhcpAck => "DHCP_ACK",
            Phase::Ipv4Prete => "IPV4_READY",
        }
    }
}

/// Instant de la phase precedente, pour le `delta_ms` de confort.
static PRECEDENTE_NS: AtomicU64 = AtomicU64::new(0);
/// Instant de `LINK_UP` le plus recent : l'origine qui compte pour DHCP.
static LIEN_HAUT_NS: AtomicU64 = AtomicU64::new(0);

/// Depuis la derniere montee de lien, en ms. Zero si le lien n'est jamais
/// monte -- et c'est une information, pas un defaut.
pub fn depuis_lien_haut_ms() -> u64 {
    let origine = LIEN_HAUT_NS.load(Ordering::Relaxed);
    if origine == 0 {
        return 0;
    }
    crate::kernel::timer::monotonic_ns().saturating_sub(origine) / 1_000_000
}

/// Publie une etape. `xid` vaut 0 hors negociation DHCP.
///
/// Emise sur la console serie ET dans la blackbox : c'est un CHANGEMENT
/// D'ETAT, pas un paquet. La distinction est ce qui garde la trace lisible --
/// on veut la chronologie, pas le trafic.
pub fn phase(etape: Phase, xid: u32) {
    let maintenant = crate::kernel::timer::monotonic_ns();
    let precedente = PRECEDENTE_NS.swap(maintenant, Ordering::Relaxed);
    let delta_ms = if precedente == 0 {
        0
    } else {
        maintenant.saturating_sub(precedente) / 1_000_000
    };
    if etape == Phase::LienHaut {
        LIEN_HAUT_NS.store(maintenant, Ordering::Relaxed);
    }
    if etape == Phase::LienBas {
        LIEN_HAUT_NS.store(0, Ordering::Relaxed);
    }

    crate::kernel::dmesg::log_fmt(format_args!(
        "NET_BOOT boot_id={} phase={} t_ns={} delta_ms={} depuis_lien_ms={} xid={:#x}",
        crate::kernel::blackbox::boot_id_public(),
        etape.nom(),
        maintenant,
        delta_ms,
        depuis_lien_haut_ms(),
        xid,
    ));
}

// ---------------------------------------------------------------------------
// BOUCHAUD_C76_OU_L_OFFRE_DISPARAIT
// ---------------------------------------------------------------------------
//
// La capture du fil a tranche : l'OFFRE ARRIVE.
//
//     OS->reseau  DISCOVER xid=0xb2be5613  0.0.0.0:68 -> 255.255.255.255:67
//     reseau->OS  OFFER    xid=0xb2be5613  10.0.2.2:67 -> 255.255.255.255:68
//
// Meme xid, reponse sous la milliseconde, et le client attend 700 ms. Le
// serveur fait son travail ; le paquet se perd DANS notre pile.
//
// `offer_seen=0` ne dit pas ou. Ces compteurs-ci le disent : chaque etage
// entre la carte et le client DHCP en a un, et la ligne les publie ENSEMBLE
// a la fin de chaque tentative -- pas a chaque paquet.

use core::sync::atomic::AtomicU32;

/// Trames sorties de l'anneau par `draine_anneau`.
pub static RX_TRAMES: AtomicU64 = AtomicU64::new(0);
/// En-tete Ethernet lu.
pub static RX_ETH_OK: AtomicU64 = AtomicU64::new(0);
/// Ethertype IPv4.
pub static RX_IPV4_TYPE: AtomicU64 = AtomicU64::new(0);
/// En-tete IPv4 lu.
pub static RX_IPV4_OK: AtomicU64 = AtomicU64::new(0);
/// En-tete IPv4 REFUSE par l'analyseur.
pub static RX_IPV4_KO: AtomicU64 = AtomicU64::new(0);
/// Bornes de charge utile incoherentes (debut > fin, ou fin > trame).
pub static RX_BORNES_KO: AtomicU64 = AtomicU64::new(0);
/// Protocole UDP.
pub static RX_UDP_PROTO: AtomicU64 = AtomicU64::new(0);
/// En-tete UDP lu.
pub static RX_UDP_OK: AtomicU64 = AtomicU64::new(0);
/// En-tete UDP REFUSE par l'analyseur.
pub static RX_UDP_KO: AtomicU64 = AtomicU64::new(0);
/// Port de destination 68.
pub static RX_PORT68: AtomicU64 = AtomicU64::new(0);
/// Depose dans la boite DHCP.
pub static RX_DEPOSE: AtomicU64 = AtomicU64::new(0);
/// Sorti de la boite par le client.
pub static RX_PRIS: AtomicU64 = AtomicU64::new(0);
/// Trop court pour un BOOTP.
pub static RX_TROP_COURT: AtomicU64 = AtomicU64::new(0);
/// Pas un BOOTREPLY.
pub static RX_PAS_REPLY: AtomicU64 = AtomicU64::new(0);
/// xid different de celui de la tentative.
pub static RX_XID_KO: AtomicU64 = AtomicU64::new(0);
/// Type DHCP different de celui attendu.
pub static RX_TYPE_KO: AtomicU64 = AtomicU64::new(0);
/// Accepte par le client.
pub static RX_ACCEPTE: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn note(compteur: &AtomicU64) {
    compteur.fetch_add(1, Ordering::Relaxed);
}

/// Publie l'etat de tous les etages pour une tentative.
///
/// UNE ligne par tentative. Un `offer_seen=0` seul ne distingue pas « aucune
/// offre n'existe » de « l'offre existe et notre pile la rejette » ; cette
/// ligne-ci nomme l'etage exact ou elle s'arrete.
pub fn trace_rx(attempt: u64, xid: u32, resultat: &str, duree_ms: u64) {
    crate::kernel::dmesg::log_fmt(format_args!(
        "DHCP_RX_TRACE attempt={} xid={:#x} result={} duration_ms={} \
trames={} eth={} ipv4_type={} ipv4_ok={} ipv4_ko={} bornes_ko={} \
udp_proto={} udp_ok={} udp_ko={} port68={} depose={} pris={} \
trop_court={} pas_reply={} xid_ko={} type_ko={} accepte={}",
        attempt, xid, resultat, duree_ms,
        RX_TRAMES.load(Ordering::Relaxed),
        RX_ETH_OK.load(Ordering::Relaxed),
        RX_IPV4_TYPE.load(Ordering::Relaxed),
        RX_IPV4_OK.load(Ordering::Relaxed),
        RX_IPV4_KO.load(Ordering::Relaxed),
        RX_BORNES_KO.load(Ordering::Relaxed),
        RX_UDP_PROTO.load(Ordering::Relaxed),
        RX_UDP_OK.load(Ordering::Relaxed),
        RX_UDP_KO.load(Ordering::Relaxed),
        RX_PORT68.load(Ordering::Relaxed),
        RX_DEPOSE.load(Ordering::Relaxed),
        RX_PRIS.load(Ordering::Relaxed),
        RX_TROP_COURT.load(Ordering::Relaxed),
        RX_PAS_REPLY.load(Ordering::Relaxed),
        RX_XID_KO.load(Ordering::Relaxed),
        RX_TYPE_KO.load(Ordering::Relaxed),
        RX_ACCEPTE.load(Ordering::Relaxed),
    ));

    // L'ETAT MATERIEL, A COTE DES ETAGES LOGICIELS.
    //
    // `trames=0` ne dit pas si la carte a ecrit et que le pilote regarde au
    // mauvais endroit, ou si elle n'a rien ecrit du tout. `rdh` repond : il
    // avance quand le materiel consomme des descripteurs.
    let (rdh, rdt, rx_cur, pret, statuts) = crate::drivers::e1000::etat_rx();
    crate::kernel::dmesg::log_fmt(format_args!(
        "DHCP_RX_ANNEAU attempt={} pret={} rdh={} rdt={} rx_cur={} desc0={:#04x} desc1={:#04x} desc2={:#04x} desc3={:#04x}",
        attempt, pret as u8, rdh, rdt, rx_cur,
        statuts[0], statuts[1], statuts[2], statuts[3],
    ));
    let (status, rctl, ctrl) = crate::drivers::e1000::registres_rx();
    crate::kernel::dmesg::log_fmt(format_args!(
        "DHCP_RX_REGISTRES attempt={} status={:#010x} rctl={:#010x} ctrl={:#010x} lu={} rx_en={}",
        attempt, status, rctl, ctrl,
        (status >> 1) & 1, (rctl >> 1) & 1,
    ));
}
