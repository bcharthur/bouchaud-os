//! La telemetrie : parler tant qu'on peut encore parler.
//!
//! # Le pari, et ce qui le fonde
//!
//! Releve TRIGKEY :
//!
//! ```text
//! rx_packets=64  rx_cur=0  desc_nic=64  desc_cpu=0   (pendant 151 s)
//! chip_cmd = RX_ENB | TX_ENB
//! ```
//!
//! La reception est morte. L'EMISSION, elle, continue : le releve montre des
//! DISCOVER qui partent et des requetes ARP qui sortent pendant toute la
//! panne. La machine ne peut plus entendre, mais elle parle encore.
//!
//! BRDP a besoin des deux sens -- un TCP sans RX ne s'etablit jamais. Ce
//! canal-ci n'a besoin que du TX : il pousse les evenements de l'anneau en
//! UDP, sans connexion, sans accuse, sans rien attendre en retour. C'est le
//! seul canal qui survit a la panne qu'on cherche a nommer.
//!
//! # Le format, fige
//!
//! ```text
//! Ethernet  dst = ff:ff:ff:ff:ff:ff
//! IPv4      src = IP LAB (169.254.x.y)   dst = 169.254.255.255
//! UDP       src = 2223                   dst = 2223
//! charge    une ligne JSON par evenement, terminee par \n
//! ```
//!
//! ## Pourquoi la DIFFUSION
//!
//! Emettre vers une adresse unicast demande de connaitre la MAC du PC, donc
//! une resolution ARP, donc une REPONSE -- c'est-a-dire de la reception. Faire
//! dependre le canal de survie de la chose en panne le rendrait inutile
//! exactement quand il sert. La diffusion ne demande rien a personne.
//!
//! ## CE CANAL N'EST NI CONFIDENTIEL NI AUTHENTIFIE
//!
//! Il part en clair, en diffusion, sans signature. Quiconque est sur le
//! segment local le lit, et quiconque est sur le segment local peut forger un
//! datagramme qui lui ressemble. C'est assume, pour trois raisons qui tiennent
//! ENSEMBLE et pas separement :
//!
//!   - il n'existe qu'en LAB MODE, sur une image de laboratoire ;
//!   - il ne sort pas du segment local -- pas de passerelle, pas de route par
//!     defaut, une adresse link-local ;
//!   - **il n'accepte RIEN en entree**. C'est le point qui compte : un
//!     datagramme forge n'a personne a qui parler. Le seul socket qui ecoute
//!     est le BRDP en TCP, et celui-la est authentifie par HMAC.
//!
//! Ce qui fuit est donc l'etat interne d'une machine de laboratoire, a
//! quiconque a deja un acces physique a son commutateur. Ce qui NE peut pas
//! arriver, c'est qu'un tiers s'en serve pour agir sur la machine.
//!
//! # Best effort, au sens strict
//!
//! Pas d'accuse, pas de retransmission, pas de file qui grandit. Ce qui ne
//! part pas est PERDU, et compte comme tel. Un canal de survie qui retiendrait
//! ce qu'il n'arrive pas a emettre finirait par consommer la memoire de la
//! machine qu'il observe -- et l'on aurait remplace une panne reseau par une
//! panne memoire.
//!
//! # Ce canal NE S'ECRIT PAS DANS CE QU'IL TRANSPORTE
//!
//! La premiere redaction emettait `TELEMETRIE_ENVOI` dans l'anneau a chaque
//! envoi reussi. Un evenement normal suffisait alors a lancer un trafic
//! perpetuel : le canal se nourrissait de sa propre trace. Ses succes et ses
//! echecs sont desormais des compteurs internes, et `politique::telemetrable`
//! refuse par-dessus le marche de transporter ces evenements-la. Voir ce
//! module, qui porte la regle et la contredit en test.
//!
//! # Ce fil ne tient aucun verrou pendant qu'il emet
//!
//! Il lit l'anneau LAB, qui est sans verrou, et appelle `e1000::send` -- qui
//! est l'abstraction NIC du depot, pas le pilote Intel : sur cette machine
//! elle aiguille vers `rtl8168::send`. Il ne touche ni a la boite noire ni a
//! `VERROU_RECEPTION` : un envoi reseau lent ne peut donc pas retarder la
//! persistance ni le drainage.

use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::politique::{self, Bornes, LectureTransport};
use super::tampon::Tampon;

/// Un datagramme. Une trame Ethernet sans fragmentation, avec de la marge.
const CHARGE_MAX: usize = 1200;
type Charge = Tampon<CHARGE_MAX>;

/// Periode d'emission. Cinq par seconde : assez pour suivre un auditeur a dix
/// hertz sans faire de l'emission le facteur limitant.
const PERIODE_MS: u64 = 200;
/// Battement de vie : distingue silence normal et canal mort.
const HEARTBEAT_MS: u64 = 5_000;

/// Ce qu'un tour s'autorise : ce qui part, et ce qui est seulement regarde.
///
/// Les deux bornes viennent de `politique`, qui les porte et les contredit en
/// test. Voir l'en-tete de ce module-la : compter seulement ce qui PART ne
/// borne pas le travail, parce qu'un evenement refuse coute une lecture sans
/// rien ajouter.
const BORNES: Bornes = Bornes::defaut();

/// Retard maximal tolere avant de sauter en avant.
///
/// Un client absent ne doit pas faire accumuler du retard : le curseur suit
/// l'anneau, et ce qui est trop vieux est declare perdu plutot que rattrape.
/// Rattraper, ici, voudrait dire emettre des milliers d'evenements d'un coup
/// sur le canal qu'on essaie de menager.
const RETARD_MAX: u64 = 512;

static LANCE: AtomicBool = AtomicBool::new(false);
static DATAGRAMMES: AtomicU64 = AtomicU64::new(0);
static EVENEMENTS: AtomicU64 = AtomicU64::new(0);
static OCTETS: AtomicU64 = AtomicU64::new(0);
static ABANDONS: AtomicU64 = AtomicU64::new(0);
static PERDUS: AtomicU64 = AtomicU64::new(0);
static REFUSES: AtomicU64 = AtomicU64::new(0);
static TROP_GRANDS: AtomicU64 = AtomicU64::new(0);
static RECALAGES: AtomicU64 = AtomicU64::new(0);
static TOURS_BORNES: AtomicU64 = AtomicU64::new(0);
static CURSEUR: AtomicU64 = AtomicU64::new(0);
static HEARTBEATS: AtomicU64 = AtomicU64::new(0);
static DERNIER_HEARTBEAT_MS: AtomicU64 = AtomicU64::new(0);

/// Emet un datagramme de diffusion. Rend faux si la carte n'a pas pris.
///
/// N'EMET AUCUN EVENEMENT, ni en succes ni en echec. Voir l'en-tete du module.
fn emet(charge: &[u8]) -> bool {
    use crate::net::{ethernet, ipv4, udp};

    let mut udp_paquet = [0u8; CHARGE_MAX + 8];
    let Some(n_udp) = udp::build(
        &mut udp_paquet,
        super::PORT_TELEMETRIE,
        super::PORT_TELEMETRIE,
        charge,
    ) else {
        ABANDONS.fetch_add(1, Ordering::Relaxed);
        return false;
    };

    let mut ip_paquet = [0u8; CHARGE_MAX + 32];
    let Some(n_ip) = ipv4::build_packet(
        &mut ip_paquet,
        super::ip(),
        // La diffusion du segment link-local. Aucune resolution ARP, donc
        // aucune dependance a la reception.
        [169, 254, 255, 255],
        ipv4::PROTO_UDP,
        (DATAGRAMMES.load(Ordering::Relaxed) & 0xFFFF) as u16,
        &udp_paquet[..n_udp],
    ) else {
        ABANDONS.fetch_add(1, Ordering::Relaxed);
        return false;
    };

    let mut trame = [0u8; CHARGE_MAX + 64];
    let Some(n) = ethernet::build_frame(
        &mut trame,
        ethernet::BROADCAST,
        crate::drivers::e1000::mac(),
        ethernet::ETHERTYPE_IPV4,
        &ip_paquet[..n_ip],
    ) else {
        ABANDONS.fetch_add(1, Ordering::Relaxed);
        return false;
    };

    if !crate::drivers::e1000::send(&trame[..n]) {
        ABANDONS.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    DATAGRAMMES.fetch_add(1, Ordering::Relaxed);
    OCTETS.fetch_add(n as u64, Ordering::Relaxed);
    true
}

/// Ce que l'anneau LAB rend a un transport, traduit pour `politique`.
///
/// # Le recalage est RENDU, pas applique sur place
///
/// La premiere redaction se recalait ici et rendait `None`. Le nouveau curseur
/// etait donc calcule puis jete, la perte recomptee a chaque tour, et le canal
/// rejouait indefiniment le meme evenement ecrase. C'est `remplis` qui tient le
/// curseur : c'est donc a lui que le recalage doit parvenir.
///
/// `recale_muet` et non `recale` : ce dernier emet `LAB_PERTE` dans l'anneau,
/// et le recalage du transporteur fabriquerait alors l'evenement suivant a
/// transporter. Voir `lab::recale_muet`.
fn lis_pour_transport(seq: u64) -> LectureTransport {
    match crate::kernel::lab::lis(seq) {
        crate::kernel::lab::Lecture::Evenement(ev) => LectureTransport::Evenement(ev),
        crate::kernel::lab::Lecture::Ecrasee => {
            let (nouveau, perdues) = crate::kernel::lab::recale_muet(seq);
            LectureTransport::Recale { nouveau, perdues }
        }
        crate::kernel::lab::Lecture::PasEncore | crate::kernel::lab::Lecture::Dechiree => {
            LectureTransport::PasEncore
        }
    }
}

/// Un tour : ramasse ce qui tient, emet, et n'attend rien.
///
/// Rend le nombre d'evenements partis.
pub fn tour() -> usize {
    let mut curseur = CURSEUR.load(Ordering::Relaxed);
    let (_, plus_ancienne, prochaine, _) = crate::kernel::lab::compteurs();

    // LE RETARD NE SE RATTRAPE PAS, IL SE CONSTATE. Un client absent pendant
    // une minute laisserait sinon des milliers d'evenements a emettre d'un
    // coup, sur le canal meme qu'on essaie de menager.
    if curseur < plus_ancienne || prochaine.saturating_sub(curseur) > RETARD_MAX {
        let neuf = prochaine.saturating_sub(RETARD_MAX).max(plus_ancienne);
        if neuf > curseur {
            PERDUS.fetch_add(neuf - curseur, Ordering::Relaxed);
            curseur = neuf;
        }
    }
    if curseur >= prochaine {
        CURSEUR.store(curseur, Ordering::Relaxed);
        return 0;
    }

    let mut charge = Charge::neuf();
    // `remplis` decide de tout : ce qui entre, ce qui est refuse, ce qui est
    // trop grand pour tenir un jour, ce qui a ete ecrase avant qu'on arrive,
    // et de combien le curseur avance. Ses deux garanties sont qu'il avance
    // des qu'un evenement a ete LU -- c'est elle qui interdit qu'un seul
    // evenement mal forme bloque le canal pour toujours -- et qu'il ne
    // regarde jamais plus de `BORNES.max_examines` evenements par tour.
    let (suivant, bilan) =
        politique::remplis(&mut charge, curseur, BORNES, lis_pour_transport);

    if bilan.refuses > 0 {
        REFUSES.fetch_add(bilan.refuses as u64, Ordering::Relaxed);
    }
    if bilan.trop_grands > 0 {
        TROP_GRANDS.fetch_add(bilan.trop_grands as u64, Ordering::Relaxed);
        PERDUS.fetch_add(bilan.trop_grands as u64, Ordering::Relaxed);
    }
    if bilan.recalages > 0 {
        RECALAGES.fetch_add(bilan.recalages as u64, Ordering::Relaxed);
        // COMPTEES UNE FOIS, ici, parce que le recalage a ete APPLIQUE. Tant
        // que le curseur restait en arriere, la meme perte se recomptait a
        // chaque tour et le chiffre ne voulait plus rien dire.
        PERDUS.fetch_add(bilan.perdues, Ordering::Relaxed);
    }
    if bilan.borne_examens {
        TOURS_BORNES.fetch_add(1, Ordering::Relaxed);
    }

    // LE CURSEUR AVANCE MEME SI L'EMISSION ECHOUE. C'est la definition du best
    // effort : ce qui n'est pas parti est perdu, pas retenu. Le retenir ferait
    // grandir un retard que le tour suivant ne rattraperait pas davantage.
    CURSEUR.store(suivant, Ordering::Relaxed);

    if bilan.ajoutes == 0 {
        return 0;
    }
    if emet(charge.octets()) {
        EVENEMENTS.fetch_add(bilan.ajoutes as u64, Ordering::Relaxed);
        bilan.ajoutes as usize
    } else {
        PERDUS.fetch_add(bilan.ajoutes as u64, Ordering::Relaxed);
        0
    }
}

fn emet_heartbeat_si_du() {
    let maintenant_ms = crate::kernel::timer::monotonic_ms();
    let precedent = DERNIER_HEARTBEAT_MS.load(Ordering::Relaxed);
    if precedent != 0 && maintenant_ms.saturating_sub(precedent) < HEARTBEAT_MS { return; }

    // Ne passe PAS par l'anneau LAB : pas d'auto-alimentation du transport.
    let mut charge = Charge::neuf();
    let _ = write!(charge, "{{\"telemetry\":1,\"heartbeat\":true,\"t_ms\":{},\"seq\":{}}}", maintenant_ms, HEARTBEATS.load(Ordering::Relaxed));
    charge.termine();
    if emet(charge.octets()) {
        HEARTBEATS.fetch_add(1, Ordering::Relaxed);
        DERNIER_HEARTBEAT_MS.store(maintenant_ms, Ordering::Relaxed);
    }
}

fn fil_telemetrie() -> ! {
    loop {
        tour();
        emet_heartbeat_si_du();
        crate::kernel::task::sleep_ticks(crate::kernel::timer::ms_to_ticks(PERIODE_MS));
    }
}

/// Lance la telemetrie. Idempotent. Ne demande aucun jeton : ce canal ne rend
/// que ce que l'anneau contient deja, et il n'accepte RIEN en entree.
pub fn demarre() -> bool {
    if LANCE.load(Ordering::Acquire) {
        return true;
    }
    if !super::actif() {
        return false;
    }
    // On part du present, pas du debut : un canal de survie n'a aucune raison
    // de commencer par rejouer l'amorcage.
    let (_, _, prochaine, _) = crate::kernel::lab::compteurs();
    CURSEUR.store(prochaine, Ordering::Relaxed);
    if crate::kernel::task::spawn_noyau_priorite(
        fil_telemetrie,
        "bouchaud-telemetrie",
        crate::kernel::task::Priorite::Normale,
    ) {
        LANCE.store(true, Ordering::Release);
        return true;
    }
    false
}

pub fn lance() -> bool {
    LANCE.load(Ordering::Relaxed)
}

/// Datagrammes, evenements, octets, abandons.
pub fn compteurs() -> (u64, u64, u64, u64) {
    (
        DATAGRAMMES.load(Ordering::Relaxed),
        EVENEMENTS.load(Ordering::Relaxed),
        OCTETS.load(Ordering::Relaxed),
        ABANDONS.load(Ordering::Relaxed),
    )
}

/// Evenements que ce canal n'a pas transmis, pour une raison ou une autre.
pub fn perdus() -> u64 {
    PERDUS.load(Ordering::Relaxed)
}

/// Evenements que la politique refuse de transporter, et ceux qui ne tiennent
/// dans aucun datagramme.
///
/// Les seconds sont une anomalie : un evenement du catalogue rend au plus
/// quelques centaines d'octets. Un compteur non nul ici veut dire qu'une sonde
/// produit quelque chose d'inattendu, et c'est exactement le genre de fait
/// qu'un canal de diagnostic doit savoir dire de lui-meme.
pub fn ecartes() -> (u64, u64) {
    (
        REFUSES.load(Ordering::Relaxed),
        TROP_GRANDS.load(Ordering::Relaxed),
    )
}

/// Recalages subis, et tours arretes par la borne d'examens.
///
/// Le second compteur n'est pas une alarme : il dit que la borne sert, donc
/// que le canal etale son travail au lieu de le prendre d'un bloc a
/// l'ordonnanceur. Il serait inquietant a zero sous forte charge -- cela
/// voudrait dire que la borne ne protege rien.
pub fn cadence() -> (u64, u64) {
    (
        RECALAGES.load(Ordering::Relaxed),
        TOURS_BORNES.load(Ordering::Relaxed),
    )
}
