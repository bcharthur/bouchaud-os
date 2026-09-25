//! La politique de transport : ce qui part, ce qui ne part pas, et ce qui est
//! abandonne pour que la suite passe.
//!
//! # Le defaut que ce module existe pour interdire
//!
//! Le canal de telemetrie lit l'anneau LAB et le reexpedie. La premiere
//! redaction emettait, a chaque envoi reussi, un evenement `TELEMETRIE_ENVOI`
//! DANS CE MEME ANNEAU :
//!
//! ```text
//! anneau -> tour() -> envoi -> TELEMETRIE_ENVOI -> anneau
//!        -> tour() -> envoi -> TELEMETRIE_ENVOI -> anneau
//!        -> ... indefiniment
//! ```
//!
//! Un evenement normal suffisait alors a lancer un trafic perpetuel : le canal
//! se nourrissait de sa propre trace. Sur une machine dont on cherche a mesurer
//! l'ordonnancement et dont le TX est le dernier lien vivant, c'est le pire
//! defaut possible -- il consomme precisement la ressource qu'il doit menager,
//! et il le fait d'autant plus que la machine est calme par ailleurs.
//!
//! `recale` posait le meme piege par un autre chemin : il emet `LAB_PERTE`, et
//! le recalage du transporteur fabriquait donc l'evenement suivant a
//! transporter. Voir `lab::recale_muet`.
//!
//! # Deux garde-fous, et pas un seul
//!
//! 1. le canal ne s'ecrit plus dans l'anneau : ses succes et ses echecs sont
//!    des compteurs internes, lisibles par `compteurs()` ;
//! 2. `telemetrable()` REFUSE quand meme de transporter les evenements du
//!    transport. C'est une ceinture par-dessus les bretelles, et elle est la
//!    parce que le premier garde-fou est une discipline -- quelqu'un
//!    rajoutera un `emets` un jour -- tandis que celui-ci est une regle, et
//!    qu'une regle se contredit en test.
//!
//! # Un tour coute un montant BORNE, et deux bornes le disent
//!
//! Compter seulement ce qui est AJOUTE ne borne pas le travail : un evenement
//! refuse ou trop grand fait avancer le curseur sans rien ajouter, donc mille
//! evenements refuses tenaient dans un seul tour. Pour un agent de
//! diagnostic, c'est un temps de garde arbitraire pris a l'ordonnanceur qu'il
//! est cense observer -- la mesure change ce qu'elle mesure.
//!
//! D'ou DEUX bornes distinctes, parce qu'elles repondent a deux questions
//! differentes :
//!
//!   - `max_ajoutes` borne la TAILLE DU DATAGRAMME : combien d'evenements le
//!     client recevra au plus d'un coup ;
//!   - `max_examines` borne le COUT DU TOUR : combien d'evenements auront ete
//!     regardes, ajoutes ou non.
//!
//! La seconde est celle qui rend la main a l'ordonnanceur. Le curseur avance
//! quand meme de tout ce qui a ete examine, donc rien ne se rejoue au tour
//! suivant : le travail est etale, pas repete.
//!
//! # Pur
//!
//! Pas de reseau, pas d'anneau global, pas d'horloge. Des identifiants et un
//! tampon entrent, des decisions sortent.

use crate::kernel::lab::anneau::Evenement;
use crate::kernel::lab::catalogue::{self, id, Categorie, Style};

use super::tampon::Tampon;

/// Evenements places au plus dans un datagramme.
///
/// La vraie borne d'octets est la taille du tampon ; celle-ci evite de
/// construire un datagramme qu'on jetterait ensuite faute de place.
pub const MAX_AJOUTES: usize = 24;

/// Evenements REGARDES au plus par tour, ajoutes ou non.
///
/// Plus grande que `MAX_AJOUTES`, pour qu'une poignee de refus n'ampute pas
/// un datagramme par ailleurs remplissable ; assez petite pour qu'un tour
/// reste un tour. Soixante-quatre lectures d'anneau sans verrou se comptent
/// en microsecondes.
pub const MAX_EXAMINES: usize = 64;

/// Ce qu'un tour de transport s'autorise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bornes {
    pub max_ajoutes: usize,
    pub max_examines: usize,
}

impl Default for Bornes {
    fn default() -> Self {
        Self::defaut()
    }
}

impl Bornes {
    pub const fn defaut() -> Self {
        Self { max_ajoutes: MAX_AJOUTES, max_examines: MAX_EXAMINES }
    }

    /// Des bornes sur mesure. Pour les epreuves, et pour un appelant qui sait
    /// ce qu'il fait.
    pub const fn neuves(max_ajoutes: usize, max_examines: usize) -> Self {
        Self { max_ajoutes, max_examines }
    }
}

/// Cet evenement peut-il etre transporte par la telemetrie ?
///
/// # Ce qui est refuse, et rien d'autre
///
/// Les evenements que le transport produit LUI-MEME. Tout le reste passe --
/// y compris les autres evenements `Remote`, comme ceux du serveur BRDP :
/// ceux-la sont produits par les commandes d'un operateur, pas par le canal
/// qui les transporte, et ils ne peuvent donc pas s'auto-entretenir.
pub fn telemetrable(categorie: u16, event_id: u32) -> bool {
    if categorie != Categorie::Remote as u16 {
        return true;
    }
    !matches!(event_id, id::TELEMETRIE_ENVOI | id::TELEMETRIE_ABANDON)
}

/// Ce qu'a donne une tentative d'ajout dans un datagramme.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ajout {
    /// L'evenement est dans le tampon.
    Ajoute,
    /// Le tampon est plein, mais cet evenement tiendrait dans un tampon vide.
    /// Il repassera au tour suivant ; le curseur NE DOIT PAS avancer.
    Plein,
    /// L'evenement ne tient pas meme seul dans un tampon vide.
    ///
    /// Le curseur DOIT avancer quand meme : sinon le tour suivant retombe sur
    /// lui, echoue pareil, et tous les evenements qui le suivent sont bloques
    /// pour toujours. C'est un blocage definitif du canal de survie par un
    /// seul evenement mal forme -- exactement le genre de panne qu'on ne peut
    /// pas se permettre dans l'outil qui sert a diagnostiquer les pannes.
    TropGrand,
    /// La politique refuse de transporter cet evenement. Le curseur avance.
    Refuse,
}

/// Ce qu'une source d'evenements rend a un transport.
///
/// # Pourquoi ce n'est pas `Option<Evenement>`
///
/// La premiere redaction passait un `FnMut(u64) -> Option<Evenement>`. Sur un
/// evenement ecrase, l'appelant se recalait CHEZ LUI et rendait `None` :
///
/// ```text
/// tour 1 : lis(10) = Ecrasee -> recale -> nouveau = 100 -> rend None
///          remplis ne voit que None, et laisse son curseur a 10
/// tour 2 : lis(10) = Ecrasee -> recale -> nouveau = 100 -> rend None
///          ... indefiniment
/// ```
///
/// Le nouveau curseur etait calcule puis JETE, et la perte recomptee a chaque
/// tour. Le canal ne repartait jamais : il brulait un tour de boucle pour
/// oublier la meme chose. Rendre le recalage EXPLICITE dans le type rend cette
/// faute impossible a ecrire -- `remplis` recoit `nouveau`, donc il l'applique.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LectureTransport {
    /// L'evenement, entier.
    Evenement(Evenement),
    /// Ce numero est ecrase. Voici ou reprendre, et ce que cela a coute.
    Recale { nouveau: u64, perdues: u64 },
    /// Rien a cette place, et rien apres. Le tour s'arrete.
    PasEncore,
}

/// Tente d'ajouter un evenement au datagramme en construction.
///
/// `charge` doit se terminer par une ligne complete : cette fonction ecrit
/// l'evenement puis son terminateur, et defait tout si l'un des deux ne tient
/// pas.
pub fn ajoute<const N: usize>(charge: &mut Tampon<N>, ev: &Evenement) -> Ajout {
    if !telemetrable(ev.categorie, ev.event_id) {
        return Ajout::Refuse;
    }
    let avant = charge.len();
    let _ = catalogue::rend(charge, ev, Style::Json);
    charge.termine();
    if !charge.tronque() {
        return Ajout::Ajoute;
    }
    // Ca n'a pas tenu. On defait, et on distingue les deux cas : un tampon qui
    // contenait deja quelque chose peut reessayer a vide au tour suivant ; un
    // tampon VIDE qui deborde ne reessaiera jamais avec succes.
    //
    // Le rollback remet le tampon exactement dans l'etat ou l'appelant nous
    // l'a confie, drapeau compris : ce qui suit peut donc y ecrire.
    let _ = charge.tronque_a(avant);
    if avant == 0 {
        return Ajout::TropGrand;
    }
    Ajout::Plein
}

/// Ce qu'un tour de transport a decide pour un evenement.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Bilan {
    /// Evenements places dans le datagramme.
    pub ajoutes: u32,
    /// Evenements REGARDES, ajoutes ou non. C'est le cout du tour.
    pub examines: u32,
    /// Evenements sautes parce que la politique les refuse.
    pub refuses: u32,
    /// Evenements sautes parce qu'ils ne tiennent dans aucun datagramme.
    pub trop_grands: u32,
    /// Recalages subis pendant ce tour.
    pub recalages: u32,
    /// Evenements que l'anneau avait deja ecrases quand on est arrive.
    ///
    /// Comptes UNE FOIS, au moment ou le recalage est applique. L'appelant les
    /// additionne a ses propres pertes sans avoir a les recalculer -- et sans
    /// risquer de les compter a chaque tour, ce qui etait la faute d'avant.
    pub perdues: u64,
    /// Le datagramme est plein : il reste des evenements pour le tour suivant.
    pub plein: bool,
    /// Le tour s'est arrete sur `max_examines`, pas sur la fin des evenements.
    ///
    /// Ce n'est pas une anomalie : c'est la preuve que la borne a servi. Un
    /// appelant qui le voit sait qu'il reste du travail, et qu'il le reprendra
    /// au tour suivant SANS le rejouer, puisque le curseur a avance.
    pub borne_examens: bool,
}

/// Remplit un datagramme a partir d'un curseur, et dit de combien avancer.
///
/// # Ce que cette fonction garantit
///
/// 1. le curseur rendu est TOUJOURS superieur ou egal a celui recu, et il est
///    strictement superieur des qu'un evenement a ete lu -- qu'il ait ete
///    ajoute, refuse, declare trop grand ou saute par un recalage. Un tour ne
///    peut donc pas laisser le canal sur place, et c'est ce qui interdit le
///    blocage definitif ;
/// 2. le nombre de lectures est borne par `bornes.max_examines`, quoi que
///    l'anneau contienne. Un tour rend la main, toujours.
///
/// La seconde garantie tient meme si la source ment : un `Recale` qui ne fait
/// pas avancer le curseur est traite comme une fin de tour plutot que comme
/// une invitation a recommencer.
pub fn remplis<const N: usize>(
    charge: &mut Tampon<N>,
    curseur: u64,
    bornes: Bornes,
    mut lis: impl FnMut(u64) -> LectureTransport,
) -> (u64, Bilan) {
    let mut bilan = Bilan::default();
    let mut seq = curseur;
    loop {
        if bilan.ajoutes as usize >= bornes.max_ajoutes {
            break;
        }
        if bilan.examines as usize >= bornes.max_examines {
            // LA BORNE DE COUT. Le curseur a avance de tout ce qu'on a
            // examine, donc le tour suivant reprend ou celui-ci s'arrete.
            bilan.borne_examens = true;
            break;
        }
        bilan.examines += 1;
        match lis(seq) {
            LectureTransport::PasEncore => break,
            LectureTransport::Recale { nouveau, perdues } => {
                if nouveau <= seq {
                    // La source dit « ecrase » et propose de reprendre ou l'on
                    // est deja. On ne peut rien en faire, et surtout pas
                    // reessayer : on rend la main.
                    break;
                }
                // LE NOUVEAU CURSEUR EST APPLIQUE, PAS JETE. C'est toute la
                // raison d'etre de `LectureTransport`.
                seq = nouveau;
                bilan.recalages += 1;
                bilan.perdues = bilan.perdues.saturating_add(perdues);
            }
            LectureTransport::Evenement(ev) => match ajoute(charge, &ev) {
                Ajout::Ajoute => {
                    bilan.ajoutes += 1;
                    seq += 1;
                }
                Ajout::Refuse => {
                    bilan.refuses += 1;
                    seq += 1;
                }
                Ajout::TropGrand => {
                    bilan.trop_grands += 1;
                    // LE CURSEUR AVANCE. Voir `Ajout::TropGrand`.
                    seq += 1;
                }
                Ajout::Plein => {
                    bilan.plein = true;
                    break;
                }
            },
        }
    }
    (seq, bilan)
}
