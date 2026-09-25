//! Le catalogue : UN seul endroit qui sait ce qu'un evenement veut dire.
//!
//! # Pourquoi un catalogue, et non trois formatages
//!
//! Le meme fait doit se lire au shell, dans la boite noire et au bout d'une
//! chaussette TCP. Ecrit trois fois, il derive trois fois : le shell dit
//! `desc63`, la boite noire dit `descripteur_63`, le JSON dit `d63`, et
//! l'enquete croise trois vocabulaires pour un seul evenement. Pire, une sonde
//! ajoutee n'apparait que dans le formatage que son auteur a pense a toucher.
//!
//! Ici, une `Definition` nomme l'evenement et ses arguments UNE fois. Les
//! trois styles ne sont que trois ponctuations de la meme table.
//!
//! # Ce que la table garantit, et que le test verifie
//!
//! - un identifiant n'est jamais donne deux fois ;
//! - un nom n'est jamais donne deux fois ;
//! - l'identifiant porte sa categorie dans ses bits hauts, donc un evenement
//!   mal range se voit sans lire la prose ;
//! - les arguments nommes sont les PREMIERS : un trou entre deux noms ferait
//!   rendre `arg3` sans `arg2`, et personne ne saurait ce que `arg3` compte.

use core::fmt::Write;

use super::anneau::Evenement;

/// Les familles d'evenements. Le nombre est le quartet haut de l'identifiant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u16)]
pub enum Categorie {
    /// L'anneau lui-meme, et le mode LAB.
    Lab = 0,
    /// Le pilote RTL8168 : anneau materiel, descripteurs, registres.
    Rtl8168 = 1,
    /// La boite noire : ecritures, purges, points de reprise.
    Blackbox = 2,
    /// La negociation DHCP, etage par etage.
    Dhcp = 3,
    /// Les verdicts de l'auditeur.
    Audit = 4,
    /// La pile reseau : routage, isolation, sockets.
    Reseau = 5,
    /// Le debugger distant et la telemetrie.
    Remote = 6,
}

impl Categorie {
    pub const fn nom(self) -> &'static str {
        match self {
            Categorie::Lab => "lab",
            Categorie::Rtl8168 => "rtl8168",
            Categorie::Blackbox => "blackbox",
            Categorie::Dhcp => "dhcp",
            Categorie::Audit => "audit",
            Categorie::Reseau => "reseau",
            Categorie::Remote => "remote",
        }
    }

    pub const fn depuis(valeur: u16) -> Option<Categorie> {
        match valeur {
            0 => Some(Categorie::Lab),
            1 => Some(Categorie::Rtl8168),
            2 => Some(Categorie::Blackbox),
            3 => Some(Categorie::Dhcp),
            4 => Some(Categorie::Audit),
            5 => Some(Categorie::Reseau),
            6 => Some(Categorie::Remote),
            _ => None,
        }
    }

    /// La categorie que l'identifiant annonce dans ses bits hauts.
    pub const fn de_l_identifiant(id: u32) -> Option<Categorie> {
        Categorie::depuis((id >> 8) as u16)
    }
}

/// Comment un argument se lit.
///
/// Un registre materiel en decimal est illisible ; un compteur en hexadecimal
/// l'est tout autant. La forme fait partie du sens, donc elle vit dans la
/// table et non dans l'appelant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Forme {
    /// Cet argument n'existe pas pour cet evenement.
    Absent,
    /// Compteur, index, longueur.
    Decimal,
    /// Registre 8 bits : `0x0c`.
    Hexa8,
    /// Registre 16 bits : `0x0041`.
    Hexa16,
    /// Mot de descripteur, registre 32 bits : `0x8000_07ff`.
    Hexa32,
    /// Adresse DMA, masque de soixante-quatre descripteurs.
    Hexa64,
    /// `0` ou `1`, rendu `false`/`true` en JSON.
    Booleen,
    /// Duree en nanosecondes, rendue aussi en microsecondes au shell.
    Nanosecondes,
}

/// Ce qu'un evenement est, dit une seule fois.
pub struct Definition {
    pub id: u32,
    pub nom: &'static str,
    pub args: [&'static str; 4],
    pub formes: [Forme; 4],
}

/// Raccourci de table : un evenement sans argument.
const fn d0(id: u32, nom: &'static str) -> Definition {
    Definition { id, nom, args: ["", "", "", ""], formes: [Forme::Absent; 4] }
}

const fn d(
    id: u32,
    nom: &'static str,
    args: [&'static str; 4],
    formes: [Forme; 4],
) -> Definition {
    Definition { id, nom, args, formes }
}

// ---------------------------------------------------------------------------
// LES IDENTIFIANTS
// ---------------------------------------------------------------------------
//
// `0xCNN` : `C` est la categorie, `NN` le rang dans la famille. Un identifiant
// dit donc sa famille sans consulter la table -- ce qui compte quand on lit un
// vidage brut, ou quand le catalogue lui-meme est en cause.

pub mod id {
    // --- lab ---------------------------------------------------------------
    pub const LAB_DEMARRE: u32 = 0x000;
    pub const LAB_PERTE: u32 = 0x001;

    // --- rtl8168 -----------------------------------------------------------
    pub const RING_WRAP_BEFORE: u32 = 0x100;
    pub const RING_WRAP_AFTER: u32 = 0x101;
    pub const RING_SECOND_LAP_TIMEOUT: u32 = 0x102;
    pub const RX_DESC: u32 = 0x103;
    pub const RX_REGISTRES: u32 = 0x104;
    pub const RX_OWN_MAP: u32 = 0x105;
    pub const RX_RECOVERY_BEGIN: u32 = 0x106;
    pub const RX_RECOVERY_END: u32 = 0x107;
    pub const RX_RECOVERY_DIFFEREE: u32 = 0x108;
    pub const RX_ENERGIE: u32 = 0x109;
    pub const RX_DESC_ADDR_RELU: u32 = 0x10A;
    pub const RX_PROGRES: u32 = 0x10B;

    // --- blackbox ----------------------------------------------------------
    pub const BB_STORAGE_READY: u32 = 0x200;
    pub const BB_CHECKPOINT_DUE: u32 = 0x201;
    pub const BB_CHECKPOINT_BEGIN: u32 = 0x202;
    pub const BB_CHECKPOINT_OK: u32 = 0x203;
    pub const BB_CHECKPOINT_ERREUR: u32 = 0x204;
    pub const BB_CHECKPOINT_REPORTE: u32 = 0x205;
    pub const BB_WRITE_ERREUR: u32 = 0x206;
    pub const BB_FLUSH: u32 = 0x207;
    pub const BB_FIN: u32 = 0x208;
    pub const BB_PARTIEL_CHECKPOINT: u32 = 0x209;

    // --- dhcp --------------------------------------------------------------
    pub const DHCP_DISCOVER: u32 = 0x300;
    pub const DHCP_OFFER: u32 = 0x301;
    pub const DHCP_REQUEST: u32 = 0x302;
    pub const DHCP_ACK: u32 = 0x303;
    pub const DHCP_IPV4_READY: u32 = 0x304;
    pub const DHCP_REJET: u32 = 0x305;
    pub const DHCP_ETAGE: u32 = 0x306;

    // --- audit -------------------------------------------------------------
    pub const AUDIT_TOUR: u32 = 0x400;
    pub const AUDIT_RX_DMA_STALL: u32 = 0x401;
    pub const AUDIT_ANNEAU_INVARIANT: u32 = 0x402;
    pub const AUDIT_SECOND_TOUR_ABSENT: u32 = 0x403;
    pub const AUDIT_BLACKBOX_PERSISTENCE_STALL: u32 = 0x404;
    pub const AUDIT_CADENCE: u32 = 0x405;
    pub const AUDIT_SAIN: u32 = 0x406;

    // --- reseau ------------------------------------------------------------
    pub const NET_TRI: u32 = 0x500;
    pub const NET_RST_EVITE: u32 = 0x501;
    pub const NET_LINK_LOCAL: u32 = 0x502;

    // --- remote ------------------------------------------------------------
    pub const BRDP_ECOUTE: u32 = 0x600;
    pub const BRDP_CONNEXION: u32 = 0x601;
    pub const BRDP_AUTH: u32 = 0x602;
    pub const BRDP_COMMANDE: u32 = 0x603;
    pub const BRDP_DECONNEXION: u32 = 0x604;
    pub const TELEMETRIE_ENVOI: u32 = 0x605;
    pub const TELEMETRIE_ABANDON: u32 = 0x606;
    pub const LAB_SERVICES: u32 = 0x607;
}

use Forme::{Absent as A, Booleen as B, Decimal as N, Hexa16 as H16, Hexa32 as H32,
            Hexa64 as H64, Hexa8 as H8, Nanosecondes as NS};

/// LA TABLE. Une ligne par evenement, et c'est tout ce qui existe.
pub static DEFINITIONS: &[Definition] = &[
    d(id::LAB_DEMARRE, "LAB_DEMARRE",
      ["capacite", "", "", ""], [N, A, A, A]),
    d(id::LAB_PERTE, "LAB_PERTE",
      ["perdues", "curseur", "plus_ancienne", ""], [N, N, N, A]),

    d(id::RING_WRAP_BEFORE, "RING_WRAP_BEFORE",
      ["rx_cur", "rx_paquets", "desc63_opts1", "desc0_opts1"], [N, N, H32, H32]),
    d(id::RING_WRAP_AFTER, "RING_WRAP_AFTER",
      ["rx_cur", "rx_paquets", "desc63_opts1", "desc0_opts1"], [N, N, H32, H32]),
    d(id::RING_SECOND_LAP_TIMEOUT, "RING_SECOND_LAP_TIMEOUT",
      ["attente_ns", "rx_paquets", "rendus_tour2", "isr_rx_ok"], [NS, N, N, N]),
    d(id::RX_DESC, "RX_DESC",
      ["index", "opts1", "opts2", "addr"], [N, H32, H32, H64]),
    d(id::RX_REGISTRES, "RX_REGISTRES",
      ["chip_cmd", "intr_status", "rx_config", "cplus_cmd"], [H8, H16, H32, H16]),
    d(id::RX_OWN_MAP, "RX_OWN_MAP",
      ["map_own", "materiel", "processeur", "rx_cur"], [H64, N, N, N]),
    d(id::RX_RECOVERY_BEGIN, "RX_RECOVERY_BEGIN",
      ["degre", "sans_effet", "rx_paquets", "own_rendus"], [N, N, N, N]),
    d(id::RX_RECOVERY_END, "RX_RECOVERY_END",
      ["effective", "sans_effet", "age_ns", "rx_paquets"], [B, N, NS, N]),
    d(id::RX_RECOVERY_DIFFEREE, "RX_RECOVERY_DIFFEREE",
      ["restant_ns", "sans_effet", "differees", ""], [NS, N, N, A]),
    d(id::RX_ENERGIE, "RX_ENERGIE",
      ["config2", "config5", "misc", "apres"], [H8, H8, H32, B]),
    d(id::RX_DESC_ADDR_RELU, "RX_DESC_ADDR_RELU",
      ["relu", "attendu", "concorde", ""], [H64, H64, B, A]),
    d(id::RX_PROGRES, "RX_PROGRES",
      ["rx_paquets", "rendus_tour1", "rendus_tour2", "tours_cpu"], [N, N, N, N]),

    d(id::BB_STORAGE_READY, "BB_STORAGE_READY",
      ["pret", "depuis_ns", "", ""], [B, NS, A, A]),
    d(id::BB_CHECKPOINT_DUE, "BB_CHECKPOINT_DUE",
      ["echeance_ns", "storage_ready", "", ""], [NS, B, A, A]),
    d(id::BB_CHECKPOINT_BEGIN, "BB_CHECKPOINT_BEGIN",
      ["numero", "records_ram", "records_persistes", ""], [N, N, N, A]),
    d(id::BB_CHECKPOINT_OK, "BB_CHECKPOINT_OK",
      ["numero", "duree_ns", "records_persistes", ""], [N, NS, N, A]),
    d(id::BB_CHECKPOINT_ERREUR, "BB_CHECKPOINT_ERREUR",
      ["numero", "code", "", ""], [N, N, A, A]),
    d(id::BB_CHECKPOINT_REPORTE, "BB_CHECKPOINT_REPORTE",
      ["prochaine_ns", "raison", "essais", ""], [NS, N, N, A]),
    d(id::BB_WRITE_ERREUR, "BB_WRITE_ERREUR",
      ["code", "write_errors", "", ""], [N, N, A, A]),
    d(id::BB_FLUSH, "BB_FLUSH",
      ["ok", "duree_ns", "", ""], [B, NS, A, A]),
    d0(id::BB_FIN, "BB_FIN"),
    d(id::BB_PARTIEL_CHECKPOINT, "BB_PARTIEL_CHECKPOINT",
      ["checkpoint_count", "records_persistes", "", ""], [N, N, A, A]),

    d0(id::DHCP_DISCOVER, "DHCP_DISCOVER"),
    d(id::DHCP_OFFER, "DHCP_OFFER",
      ["xid", "ip", "accepte", ""], [H32, H32, B, A]),
    d(id::DHCP_REQUEST, "DHCP_REQUEST", ["xid", "", "", ""], [H32, A, A, A]),
    d(id::DHCP_ACK, "DHCP_ACK", ["xid", "ip", "", ""], [H32, H32, A, A]),
    d(id::DHCP_IPV4_READY, "DHCP_IPV4_READY",
      ["ip", "masque", "passerelle", ""], [H32, H32, H32, A]),
    d(id::DHCP_REJET, "DHCP_REJET",
      ["etage", "raison", "", ""], [N, N, A, A]),
    d(id::DHCP_ETAGE, "DHCP_ETAGE",
      ["etage", "compteur", "", ""], [N, N, A, A]),

    d(id::AUDIT_TOUR, "AUDIT_TOUR",
      ["numero", "cadence_hz", "verdicts", ""], [N, N, N, A]),
    d(id::AUDIT_RX_DMA_STALL, "AUDIT_RX_DMA_STALL",
      ["rx_paquets", "isr_delta", "desc_cpu", "silence_ns"], [N, N, N, NS]),
    d(id::AUDIT_ANNEAU_INVARIANT, "AUDIT_ANNEAU_INVARIANT",
      ["code", "index", "opts1", ""], [N, N, H32, A]),
    d(id::AUDIT_SECOND_TOUR_ABSENT, "AUDIT_SECOND_TOUR_ABSENT",
      ["rendus_tour1", "rendus_tour2", "attente_ns", ""], [N, N, NS, A]),
    d(id::AUDIT_BLACKBOX_PERSISTENCE_STALL, "AUDIT_BLACKBOX_PERSISTENCE_STALL",
      ["records_ram", "records_persistes", "fige_depuis_ns", ""], [N, N, NS, A]),
    d(id::AUDIT_CADENCE, "AUDIT_CADENCE",
      ["hz", "raison", "", ""], [N, N, A, A]),
    d(id::AUDIT_SAIN, "AUDIT_SAIN", ["tours", "", "", ""], [N, A, A, A]),

    d(id::NET_TRI, "NET_TRI",
      ["pile", "protocole", "port", "accepte"], [N, N, N, B]),
    d(id::NET_RST_EVITE, "NET_RST_EVITE",
      ["pile", "port", "", ""], [N, N, A, A]),
    d(id::NET_LINK_LOCAL, "NET_LINK_LOCAL",
      ["ip", "prefixe", "", ""], [H32, N, A, A]),

    d(id::BRDP_ECOUTE, "BRDP_ECOUTE", ["port", "", "", ""], [N, A, A, A]),
    d(id::BRDP_CONNEXION, "BRDP_CONNEXION",
      ["pair_ip", "pair_port", "", ""], [H32, N, A, A]),
    d(id::BRDP_AUTH, "BRDP_AUTH", ["ok", "raison", "", ""], [B, N, A, A]),
    d(id::BRDP_COMMANDE, "BRDP_COMMANDE",
      ["code", "octets_rendus", "duree_ns", ""], [N, N, NS, A]),
    d(id::BRDP_DECONNEXION, "BRDP_DECONNEXION",
      ["raison", "commandes", "", ""], [N, N, A, A]),
    d(id::TELEMETRIE_ENVOI, "TELEMETRIE_ENVOI",
      ["evenements", "octets", "depuis_seq", ""], [N, N, N, A]),
    d(id::TELEMETRIE_ABANDON, "TELEMETRIE_ABANDON",
      ["raison", "abandons", "", ""], [N, N, A, A]),
    d(id::LAB_SERVICES, "LAB_SERVICES",
      ["ip", "brdp", "telemetrie", "port_brdp"], [H32, N, N, N]),
];

/// La definition d'un identifiant, si le catalogue la connait.
pub fn definition(event_id: u32) -> Option<&'static Definition> {
    DEFINITIONS.iter().find(|d| d.id == event_id)
}

/// Le nom d'un evenement, ou `EVENEMENT_INCONNU`.
///
/// Un identifiant absent de la table ne fait PAS disparaitre l'evenement :
/// c'est justement quand le catalogue est en retard sur les sondes qu'on a
/// besoin de voir passer ce qu'on ne sait pas nommer.
pub fn nom(event_id: u32) -> &'static str {
    match definition(event_id) {
        Some(d) => d.nom,
        None => "EVENEMENT_INCONNU",
    }
}

/// Les trois ponctuations de la meme table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    /// `[  12.345678] rtl8168 RING_WRAP_AFTER rx_cur=0 rx_paquets=64 ...`
    Shell,
    /// `LAB seq=41 t_ns=12345678 cpu=0 rtl8168 RING_WRAP_AFTER rx_cur=0 ...`
    Blackbox,
    /// `{"seq":41,"t_ns":12345678,"cpu":0,"cat":"rtl8168","event":"...",...}`
    Json,
}

fn ecris_valeur(
    sortie: &mut impl Write,
    forme: Forme,
    valeur: u64,
    style: Style,
) -> core::fmt::Result {
    match (forme, style) {
        (Forme::Booleen, Style::Json) => {
            sortie.write_str(if valeur != 0 { "true" } else { "false" })
        }
        (Forme::Booleen, _) => write!(sortie, "{}", (valeur != 0) as u8),
        // EN JSON, UN ENTIER RESTE UN ENTIER. Un `0x8000_07ff` dans un champ
        // numerique casserait tout analyseur ; le client PC formate lui-meme.
        (_, Style::Json) => write!(sortie, "{valeur}"),
        (Forme::Hexa8, _) => write!(sortie, "{valeur:#04x}"),
        (Forme::Hexa16, _) => write!(sortie, "{valeur:#06x}"),
        (Forme::Hexa32, _) => write!(sortie, "{valeur:#010x}"),
        (Forme::Hexa64, _) => write!(sortie, "{valeur:#018x}"),
        (Forme::Nanosecondes, _) => write!(sortie, "{}us", valeur / 1_000),
        (Forme::Decimal, _) | (Forme::Absent, _) => write!(sortie, "{valeur}"),
    }
}

/// Rend un evenement dans le style demande.
///
/// # Un seul chemin, trois ponctuations
///
/// Les noms d'arguments, leur nombre et leur forme viennent de la table, quel
/// que soit le style. Ajouter une sonde la fait donc apparaitre au shell, dans
/// la boite noire et au bout de la chaussette, sans que personne y pense.
pub fn rend(sortie: &mut impl Write, ev: &Evenement, style: Style) -> core::fmt::Result {
    let cat = Categorie::depuis(ev.categorie).map(|c| c.nom()).unwrap_or("?");
    let def = definition(ev.event_id);
    let nom = def.map(|d| d.nom).unwrap_or("EVENEMENT_INCONNU");

    match style {
        Style::Shell => {
            write!(
                sortie,
                "[{:5}.{:06}] {cat} {nom}",
                ev.t_ns / 1_000_000_000,
                (ev.t_ns % 1_000_000_000) / 1_000,
            )?;
        }
        Style::Blackbox => {
            write!(
                sortie,
                "LAB seq={} t_ns={} cpu={} {cat} {nom}",
                ev.seq, ev.t_ns, ev.cpu,
            )?;
        }
        Style::Json => {
            write!(
                sortie,
                "{{\"seq\":{},\"t_ns\":{},\"cpu\":{},\"cat\":\"{cat}\",\"event\":\"{nom}\"",
                ev.seq, ev.t_ns, ev.cpu,
            )?;
        }
    }

    match def {
        Some(def) => {
            for i in 0..4 {
                if def.formes[i] == Forme::Absent {
                    break;
                }
                match style {
                    Style::Json => {
                        write!(sortie, ",\"{}\":", def.args[i])?;
                        ecris_valeur(sortie, def.formes[i], ev.args[i], style)?;
                    }
                    _ => {
                        write!(sortie, " {}=", def.args[i])?;
                        ecris_valeur(sortie, def.formes[i], ev.args[i], style)?;
                    }
                }
            }
        }
        None => {
            // LE CATALOGUE NE SAIT PAS, DONC IL NE PRETEND PAS. Les quatre
            // arguments sortent bruts et numerotes : c'est illisible, et
            // infiniment plus utile qu'un evenement escamote.
            for i in 0..4 {
                match style {
                    Style::Json => write!(sortie, ",\"arg{i}\":{}", ev.args[i])?,
                    _ => write!(sortie, " arg{i}={:#x}", ev.args[i])?,
                }
            }
            match style {
                Style::Json => write!(sortie, ",\"event_id\":{}", ev.event_id)?,
                _ => write!(sortie, " event_id={:#05x}", ev.event_id)?,
            }
        }
    }

    if style == Style::Json {
        sortie.write_char('}')?;
    }
    Ok(())
}
