//! L'ECHELLE DE LA REPONSE DNS : ou exactement le datagramme se perd.
//!
//! # Ce que l'archive du 24 septembre ne permettait pas de dire
//!
//! `bb(7)` prouve que la carte et DHCP marchent : 533 trames, huit tours
//! d'anneau, 469 descripteurs reutilises, un bail reel -- 192.168.1.97,
//! passerelle et resolveur 192.168.1.254. Ladybird recoit l'adresse du
//! resolveur, et la requete part :
//!
//! ```text
//! M17_UDP_TX dst=192.168.1.254:53 src_port=49985 octets=32 parti=true
//! ```
//!
//! Puis plus rien. Pas de `M17_UDP_LIVRE`, pas de `M17_UDP_PERDU_EMPRUNTE`,
//! pas de `M17_UDP_SANS_DESTINATAIRE`, et `NET-TCP poignees=0`.
//!
//! L'ABSENCE DES TROIS MARQUEURS EST DEJA UNE INFORMATION : la livraison n'a
//! jamais ete tentee. Mais elle ne dit pas lequel des cinq etages en amont a
//! laisse tomber le datagramme, et chacun appelle une correction differente.
//!
//! # L'echelle
//!
//! ```text
//!   carte ---> route_ipv4 ---> udp::parse ---> file IP ---> socket ---> recv
//!     |            |               |              |           |          |
//!   ethernet     ipv4            udp          queued/     match/      success/
//!                                             dequeued    busy        eagain
//! ```
//!
//! Le premier compteur nul dont le predecesseur ne l'est pas NOMME l'etage.
//! C'est tout l'objet de ce module : rendre la prochaine archive capable de
//! trancher entre « la reponse n'est jamais arrivee » et « elle est arrivee
//! et nous l'avons jetee », sans une ligne de trace par paquet.
//!
//! # Pourquoi le port 53 et lui seul
//!
//! Ce n'est pas de la prudence, c'est ce qui rend la sonde utilisable. Un
//! compteur sur tout l'UDP melangerait le trafic mDNS et SSDP d'un reseau
//! domestique -- le releve en montre plusieurs centaines de trames -- au
//! quatre ou six datagrammes qui nous interessent.
//!
//! Ce module est PUR : aucun `use crate::`, aucune horloge, aucune allocation.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Un barreau de l'echelle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Barreau {
    /// Une trame Ethernet porte de l'IPv4 avec un port 53.
    RxEthernet = 0,
    /// Son en-tete IPv4 a ete lu et le datagramme nous est adresse.
    RxIpv4 = 1,
    /// Son en-tete UDP a ete lu.
    RxUdp = 2,
    /// Il a ete mis dans la file des paquets en attente.
    MisEnFile = 3,
    /// Il en a ete ressorti par un appelant.
    SortiDeFile = 4,
    /// Un socket correspond a son port de destination.
    SocketTrouve = 5,
    /// Ce socket etait deja emprunte : le datagramme est PERDU.
    SocketOccupe = 6,
    /// Il a ete depose dans la file du socket.
    SocketLivre = 7,
    /// `poll` a vu le socket pret.
    PollPret = 8,
    /// `recvfrom` a rendu des octets.
    RecvSucces = 9,
    /// `recvfrom` n'avait rien a rendre.
    RecvVide = 10,
}

/// Le nombre de barreaux.
pub const BARREAUX: usize = 11;

impl Barreau {
    pub fn nom(self) -> &'static str {
        match self {
            Barreau::RxEthernet => "dns53_rx_ethernet",
            Barreau::RxIpv4 => "dns53_rx_ipv4",
            Barreau::RxUdp => "dns53_rx_udp",
            Barreau::MisEnFile => "dns53_queued_ip",
            Barreau::SortiDeFile => "dns53_dequeued_ip",
            Barreau::SocketTrouve => "dns53_socket_match",
            Barreau::SocketOccupe => "dns53_socket_busy",
            Barreau::SocketLivre => "dns53_socket_delivered",
            Barreau::PollPret => "dns53_poll_ready",
            Barreau::RecvSucces => "dns53_recv_success",
            Barreau::RecvVide => "dns53_recv_eagain",
        }
    }

    pub fn depuis_rang(rang: usize) -> Option<Barreau> {
        Some(match rang {
            0 => Barreau::RxEthernet,
            1 => Barreau::RxIpv4,
            2 => Barreau::RxUdp,
            3 => Barreau::MisEnFile,
            4 => Barreau::SortiDeFile,
            5 => Barreau::SocketTrouve,
            6 => Barreau::SocketOccupe,
            7 => Barreau::SocketLivre,
            8 => Barreau::PollPret,
            9 => Barreau::RecvSucces,
            10 => Barreau::RecvVide,
            _ => return None,
        })
    }
}

static ECHELLE: [AtomicU64; BARREAUX] = [
    AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
    AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
    AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
];

/// Franchit un barreau.
#[inline]
pub fn note(barreau: Barreau) {
    ECHELLE[barreau as usize].fetch_add(1, Ordering::Relaxed);
}

/// La valeur d'un barreau.
pub fn compte(barreau: Barreau) -> u64 {
    ECHELLE[barreau as usize].load(Ordering::Relaxed)
}

/// Ce datagramme concerne-t-il la sonde ?
///
/// Une requete part vers le port 53 ; une reponse en vient. Les deux sens
/// comptent, sans quoi on ne saurait pas si la requete est seulement sortie.
#[inline]
pub fn concerne(port_source: u16, port_destination: u16) -> bool {
    port_source == 53 || port_destination == 53
}

/// Les ports UDP d'un paquet IPv4, lus SANS rien valider d'autre.
///
/// # Pourquoi une lecture minimale, et pas `parse_header` puis `udp::parse`
///
/// C'est le defaut de la premiere version de cette echelle : ses trois
/// premiers barreaux etaient tous poses DANS la branche
/// `if let Some(u) = udp::parse(...)`, elle-meme placee apres un
/// `parse_header` reussi. Trois verdicts devenaient donc inatteignables --
/// dont « la trame arrive mais route_ipv4 la rejette », precisement celui
/// qu'on cherche.
///
/// Un barreau qui ne peut pas s'allumer ne mesure rien. Celui-ci ne demande
/// que ce qu'il faut pour trouver les ports : la version, la longueur
/// d'en-tete, le protocole, et quatre octets a la bonne place -- avec des
/// bornes, et aucune coherence exigee sur `total_len` ni sur la longueur UDP.
/// Il s'allume donc meme quand les analyseurs suivants refusent le paquet.
pub fn ports_bruts(paquet_ip: &[u8]) -> Option<(u16, u16)> {
    if paquet_ip.len() < 20 {
        return None;
    }
    if paquet_ip[0] >> 4 != 4 {
        return None;
    }
    let ihl = (paquet_ip[0] & 0x0F) as usize * 4;
    if ihl < 20 {
        return None;
    }
    // 17 : UDP. On ne s'interesse qu'a lui.
    if paquet_ip[9] != 17 {
        return None;
    }
    if paquet_ip.len() < ihl + 4 {
        return None;
    }
    let src = ((paquet_ip[ihl] as u16) << 8) | paquet_ip[ihl + 1] as u16;
    let dst = ((paquet_ip[ihl + 2] as u16) << 8) | paquet_ip[ihl + 3] as u16;
    Some((src, dst))
}

// ---------------------------------------------------------------------------
// LA PREMIERE REPONSE, EN DETAIL -- ET ELLE SEULE
// ---------------------------------------------------------------------------

/// Une reponse a-t-elle deja ete decrite ?
static DECRITE: AtomicBool = AtomicBool::new(false);
static DETAIL_SRC: AtomicU32 = AtomicU32::new(0);
static DETAIL_DST: AtomicU32 = AtomicU32::new(0);
static DETAIL_PORTS: AtomicU32 = AtomicU32::new(0);
static DETAIL_LONGUEUR: AtomicU32 = AtomicU32::new(0);
/// Bit 0 : somme IPv4 juste. Bit 1 : somme UDP juste. Bit 2 : somme UDP
/// absente (elle est facultative en IPv4).
static DETAIL_SOMMES: AtomicU32 = AtomicU32::new(0);

pub const SOMME_IPV4_JUSTE: u32 = 1 << 0;
pub const SOMME_UDP_JUSTE: u32 = 1 << 1;
pub const SOMME_UDP_ABSENTE: u32 = 1 << 2;

/// Retient le detail de la PREMIERE reponse vue, et rend vrai si c'est elle.
///
/// Une seule : le but est de pouvoir lire une ligne, pas de remplir l'archive.
/// Si le premier datagramme est le bon, tout est dit ; s'il ne l'est pas, les
/// compteurs suffisent a le savoir.
#[allow(clippy::too_many_arguments)]
pub fn decris_une_fois(
    src: [u8; 4],
    dst: [u8; 4],
    src_port: u16,
    dst_port: u16,
    longueur: u16,
    sommes: u32,
) -> bool {
    if src_port != 53 {
        // Une REPONSE vient du port 53. Une requete sortante n'a rien a
        // apprendre sur le chemin de retour.
        return false;
    }
    if DECRITE.swap(true, Ordering::AcqRel) {
        return false;
    }
    DETAIL_SRC.store(u32::from_be_bytes(src), Ordering::Relaxed);
    DETAIL_DST.store(u32::from_be_bytes(dst), Ordering::Relaxed);
    DETAIL_PORTS.store(((src_port as u32) << 16) | dst_port as u32, Ordering::Relaxed);
    DETAIL_LONGUEUR.store(longueur as u32, Ordering::Relaxed);
    DETAIL_SOMMES.store(sommes, Ordering::Relaxed);
    true
}

/// Le detail retenu : (src, dst, src_port, dst_port, longueur, sommes).
pub fn detail() -> Option<([u8; 4], [u8; 4], u16, u16, u16, u32)> {
    if !DECRITE.load(Ordering::Acquire) {
        return None;
    }
    let ports = DETAIL_PORTS.load(Ordering::Relaxed);
    Some((
        DETAIL_SRC.load(Ordering::Relaxed).to_be_bytes(),
        DETAIL_DST.load(Ordering::Relaxed).to_be_bytes(),
        (ports >> 16) as u16,
        (ports & 0xFFFF) as u16,
        DETAIL_LONGUEUR.load(Ordering::Relaxed) as u16,
        DETAIL_SOMMES.load(Ordering::Relaxed),
    ))
}

// ---------------------------------------------------------------------------
// LE VERDICT
// ---------------------------------------------------------------------------

/// L'etage ou la chaine s'interrompt, en clair.
///
/// Le premier barreau nul dont le predecesseur ne l'est pas. C'est la phrase
/// que la prochaine archive doit pouvoir rendre sans qu'on ait a recoller six
/// compteurs a la main.
pub fn verdict() -> &'static str {
    verdict_de(&releve())
}

/// Les onze compteurs, en une fois.
pub fn releve() -> [u64; BARREAUX] {
    core::array::from_fn(|rang| ECHELLE[rang].load(Ordering::Relaxed))
}

/// Le verdict d'un releve DONNE.
///
/// Separe de `verdict` pour etre testable : les compteurs sont globaux, et un
/// banc qui en depend rend un resultat different selon l'ordre dans lequel le
/// harnais execute ses tests. Une regle de diagnostic qui change avec l'ordre
/// des tests ne se verifie pas.
pub fn verdict_de(c: &[u64; BARREAUX]) -> &'static str {
    let v = |b: Barreau| c[b as usize];
    if v(Barreau::RxEthernet) == 0 {
        return "aucune trame port 53 n'est arrivee sur la carte";
    }
    if v(Barreau::RxIpv4) == 0 {
        return "la trame arrive mais route_ipv4 la rejette";
    }
    if v(Barreau::RxUdp) == 0 {
        return "l'en-tete IPv4 passe mais udp::parse echoue";
    }
    if v(Barreau::MisEnFile) == 0 {
        return "le datagramme n'est jamais mis en file";
    }
    if v(Barreau::SortiDeFile) == 0 {
        return "il est en file mais jamais reclame";
    }
    if v(Barreau::SocketTrouve) == 0 {
        return "il sort de la file mais aucune socket ne correspond";
    }
    if v(Barreau::SocketLivre) == 0 {
        if v(Barreau::SocketOccupe) != 0 {
            return "la socket est trouvee mais occupee : datagramme perdu";
        }
        return "la socket est trouvee mais rien n'est livre";
    }
    if v(Barreau::RecvSucces) == 0 {
        if v(Barreau::PollPret) == 0 {
            return "livre, mais poll ne voit jamais la socket prete";
        }
        return "livre et poll pret, mais recvfrom ne rend rien";
    }
    "la chaine est complete"
}
