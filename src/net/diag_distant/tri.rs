//! Le tri : quelle pile recoit cette trame, et QUI N'EN RECOIT PAS COPIE.
//!
//! # Le piege que ce module existe pour eviter
//!
//! La machine porte deux piles sur une seule carte : la pile normale, qui
//! sert le navigateur, et la pile de diagnostic, qui sert le debugger. Le
//! routage actuel donne a smoltcp une copie de CHAQUE trame brute, avant tout
//! tri :
//!
//! ```rust,ignore
//! fn route_trame(trame: &[u8]) {
//!     file_smoltcp().pose(trame);   // <-- tout, sans exception
//!     ...
//! }
//! ```
//!
//! C'est juste tant qu'il n'y a qu'une pile. Des qu'une seconde ecoute sur la
//! meme carte, cela devient un RST FRATRICIDE : smoltcp recoit un segment TCP
//! destine a une adresse qu'il considere comme locale, ne trouve aucune
//! chaussette sur le port 2222, et repond `RST`. Le PC de developpement voit
//! sa connexion au debugger refusee -- par la machine elle-meme, au moment
//! precis ou l'on cherche a savoir ce qu'elle a.
//!
//! La reciproque est aussi vraie : si la pile de diagnostic voyait le trafic
//! du navigateur, elle repondrait `RST` a des connexions parfaitement
//! legitimes, et le navigateur tomberait a cause du debugger.
//!
//! # La regle
//!
//! Une trame a UN destinataire, choisi avant que quiconque la voie. Le tri ne
//! se fait pas par soustraction -- « donne a tout le monde, chacun jettera ce
//! qui n'est pas pour lui » -- parce qu'une pile TCP ne jette pas
//! silencieusement : elle repond.
//!
//! ```text
//!   TRAME ──▶ TRI ──┬──▶ pile normale   (navigateur, DHCP, DNS)
//!                   ├──▶ pile diagnostic (ARP LAB, ICMP LAB, TCP 2222, UDP 2223)
//!                   ├──▶ LES DEUX       (diffusion : ARP de diffusion)
//!                   └──▶ personne       (IPv6, VLAN, trames pour un tiers)
//! ```
//!
//! # Pur
//!
//! Pas de pile, pas de chaussette, pas d'etat. Des octets et une
//! configuration entrent, un verdict sort. La regle se contredit donc en test
//! hote, sans carte reseau.

/// Qui recoit cette trame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pile {
    /// La pile normale seule : navigateur, DHCP, DNS.
    Normale,
    /// La pile de diagnostic seule. La pile normale N'EN VOIT PAS COPIE.
    Diagnostic,
    /// Les deux. Reserve a ce qui est adresse a tout le monde.
    LesDeux,
    /// Personne. Ni l'une ni l'autre n'a a repondre.
    Aucune,
}

/// Pourquoi cette trame va la. Chiffre, pour tenir dans l'anneau LAB.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u16)]
pub enum Motif {
    /// Trop courte pour porter un en-tete Ethernet.
    TropCourte = 0,
    /// Ni ARP ni IPv4 : IPv6, VLAN, controle de flux.
    ProtocoleInconnu = 1,
    /// ARP de diffusion : les deux piles doivent pouvoir repondre.
    ArpDiffusion = 2,
    /// ARP visant l'adresse de diagnostic.
    ArpDiagnostic = 3,
    /// ARP visant autre chose.
    ArpNormale = 4,
    /// Adressee a l'adresse de diagnostic, et c'est du diagnostic.
    IpDiagnostic = 5,
    /// TCP sur le port du debugger.
    TcpBrdp = 6,
    /// UDP sur le port de telemetrie.
    UdpTelemetrie = 7,
    /// ICMP vers l'adresse de diagnostic.
    IcmpDiagnostic = 8,
    /// Tout le reste du trafic IP.
    IpNormale = 9,
    /// Adressee a l'adresse de diagnostic, mais le LAB est eteint.
    LabEteint = 10,
    /// Diffusion IP : les deux piles la voient.
    IpDiffusion = 11,
}

/// Le verdict complet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Verdict {
    pub pile: Pile,
    pub motif: Motif,
    /// Port de destination, quand il y en a un. Zero sinon.
    pub port: u16,
}

/// Ce que le tri doit savoir de cette machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Config {
    /// L'adresse link-local du canal de diagnostic.
    pub ip_diagnostic: [u8; 4],
    /// La MAC de la carte. Les deux piles la partagent.
    pub mac: [u8; 6],
    /// Port TCP du debugger.
    pub port_brdp: u16,
    /// Port UDP de la telemetrie.
    pub port_telemetrie: u16,
    /// Le LAB ecoute-t-il ?
    ///
    /// LAB eteint, TOUT va a la pile normale -- y compris ce qui vise
    /// l'adresse de diagnostic. Le comportement redevient exactement celui
    /// d'avant ce module, ce qui est la seule facon honnete de dire « le LAB
    /// ne change rien quand il n'est pas la ».
    pub lab_actif: bool,
}

pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

const ETH_LEN: usize = 14;
const DIFFUSION: [u8; 6] = [0xFF; 6];

fn u16_be(o: &[u8]) -> u16 {
    ((o[0] as u16) << 8) | o[1] as u16
}

/// Ou va cette trame ?
///
/// Ne modifie rien, ne retient rien. Appelable depuis le drainage verrouille.
pub fn destination(cfg: &Config, trame: &[u8]) -> Verdict {
    if trame.len() < ETH_LEN {
        return Verdict { pile: Pile::Aucune, motif: Motif::TropCourte, port: 0 };
    }
    let ethertype = u16_be(&trame[12..14]);
    let charge = &trame[ETH_LEN..];

    match ethertype {
        ETHERTYPE_ARP => arp(cfg, trame, charge),
        ETHERTYPE_IPV4 => ipv4(cfg, charge),
        // IPv6, VLAN, controle de flux : aucune des deux piles ne les traite,
        // et les retenir ne ferait que remplir une file pour personne.
        _ => Verdict { pile: Pile::Aucune, motif: Motif::ProtocoleInconnu, port: 0 },
    }
}

fn arp(cfg: &Config, trame: &[u8], charge: &[u8]) -> Verdict {
    // L'ARP EST DU SERVICE DE LIEN : il ne se jette jamais au motif qu'on
    // attendait autre chose. Une reponse ARP est unique et jamais
    // retransmise ; il suffit qu'un lecteur la sorte de l'anneau une fois
    // pour que la resolution echoue, et l'echec est mis en cache.
    if charge.len() < 28 {
        // Trop courte pour etre lue : on la laisse aux deux plutot que de
        // trancher sur une supposition.
        return Verdict { pile: Pile::LesDeux, motif: Motif::ArpDiffusion, port: 0 };
    }
    let cible: [u8; 4] = [charge[24], charge[25], charge[26], charge[27]];

    if cfg.lab_actif && cible == cfg.ip_diagnostic {
        // Un ARP qui demande l'adresse LAB : seule la pile de diagnostic sait
        // qu'elle existe. La pile normale repondrait « inconnue » -- ou pire,
        // ne repondrait pas, et le PC conclurait que la machine est morte.
        return Verdict { pile: Pile::Diagnostic, motif: Motif::ArpDiagnostic, port: 0 };
    }
    if trame[0..6] == DIFFUSION {
        // Une requete de diffusion peut viser n'importe laquelle des deux
        // adresses. Les deux piles la voient, et celle qui est concernee
        // repond. C'est le seul cas ou dupliquer est juste : ARP n'ouvre pas
        // de connexion, donc il n'y a pas de RST a craindre.
        return Verdict { pile: Pile::LesDeux, motif: Motif::ArpDiffusion, port: 0 };
    }
    Verdict { pile: Pile::Normale, motif: Motif::ArpNormale, port: 0 }
}

fn ipv4(cfg: &Config, charge: &[u8]) -> Verdict {
    if charge.len() < 20 {
        return Verdict { pile: Pile::Normale, motif: Motif::IpNormale, port: 0 };
    }
    let ihl = ((charge[0] & 0x0F) as usize) * 4;
    if ihl < 20 || ihl > charge.len() {
        return Verdict { pile: Pile::Normale, motif: Motif::IpNormale, port: 0 };
    }
    let proto = charge[9];
    let dst: [u8; 4] = [charge[16], charge[17], charge[18], charge[19]];

    // LA DIFFUSION VA AUX DEUX. Un DISCOVER DHCP part en diffusion et revient
    // en diffusion ; le detourner vers la pile de diagnostic couperait la
    // configuration reseau normale de la machine.
    if dst == [255, 255, 255, 255] || (dst[0] >= 224 && dst[0] <= 239) {
        return Verdict { pile: Pile::LesDeux, motif: Motif::IpDiffusion, port: 0 };
    }

    if !cfg.lab_actif {
        return Verdict { pile: Pile::Normale, motif: Motif::LabEteint, port: 0 };
    }
    if dst != cfg.ip_diagnostic {
        // Pas pour le LAB. La pile de diagnostic n'en voit RIEN : si elle le
        // voyait, elle repondrait RST a des connexions du navigateur.
        return Verdict { pile: Pile::Normale, motif: Motif::IpNormale, port: 0 };
    }

    // A partir d'ici, la trame vise l'adresse de diagnostic. La pile normale
    // n'en voit rien : c'est cela, et cela seul, qui empeche le RST
    // fratricide sur le port 2222.
    let entete_transport = &charge[ihl..];
    match proto {
        PROTO_ICMP => Verdict {
            pile: Pile::Diagnostic,
            motif: Motif::IcmpDiagnostic,
            port: 0,
        },
        PROTO_TCP if entete_transport.len() >= 4 => {
            let port = u16_be(&entete_transport[2..4]);
            Verdict {
                pile: Pile::Diagnostic,
                motif: if port == cfg.port_brdp { Motif::TcpBrdp } else { Motif::IpDiagnostic },
                port,
            }
        }
        PROTO_UDP if entete_transport.len() >= 4 => {
            let port = u16_be(&entete_transport[2..4]);
            Verdict {
                pile: Pile::Diagnostic,
                motif: if port == cfg.port_telemetrie {
                    Motif::UdpTelemetrie
                } else {
                    Motif::IpDiagnostic
                },
                port,
            }
        }
        _ => Verdict { pile: Pile::Diagnostic, motif: Motif::IpDiagnostic, port: 0 },
    }
}

impl Verdict {
    /// La pile normale doit-elle voir cette trame ?
    pub fn pour_normale(&self) -> bool {
        matches!(self.pile, Pile::Normale | Pile::LesDeux)
    }

    /// La pile de diagnostic doit-elle voir cette trame ?
    pub fn pour_diagnostic(&self) -> bool {
        matches!(self.pile, Pile::Diagnostic | Pile::LesDeux)
    }
}
