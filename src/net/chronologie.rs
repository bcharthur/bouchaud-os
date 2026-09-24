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
