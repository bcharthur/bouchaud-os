// Le protocole du stockage de masse USB : tout ce qui est de l'octet, et rien
// d'autre.
//
// Ce fichier ne touche aucun registre et n'emet aucun transfert. Il est
// `include!` par le pilote et `#[path]`-inclus par
// `tools/platform/test_stockage_decodage.rs`, pour la meme raison que
// `nvme/decodage.rs` : les cas qui font echouer un pilote de stockage ne se
// produisent pas a la demande, et une cle USB qui repond de travers ne se
// fabrique pas sur commande.
//
// # Ce que ce protocole ne pardonne pas
//
// Trois erreurs sont dans presque toute premiere version, et aucune des trois
// ne se voit sur un transfert qui marche :
//
//   * **Les deux boutismes.** Le paquet d'enveloppe (CBW/CSW) est en PETIT
//     boutiste, parce qu'il appartient a USB. Le bloc de commande qu'il PORTE
//     est du SCSI, et le SCSI est en GRAND boutiste. Un pilote qui applique le
//     meme boutisme aux deux lit le bon nombre de blocs a la mauvaise adresse
//     -- ou seize millions de blocs au lieu d'un.
//
//   * **L'etiquette.** Le CSW rend l'etiquette du CBW auquel il repond. Ne pas
//     la verifier revient a accepter la reponse d'une commande precedente
//     comme si elle etait la notre : le transfert « reussit », et les octets
//     rendus sont ceux d'un autre secteur.
//
//   * **Le residu.** Un peripherique qui transfere MOINS que demande le dit
//     dans `residu`, et rend malgre tout un statut nul. Un pilote qui ne lit
//     que le statut croit avoir un secteur entier et en a la moitie, l'autre
//     moitie etant ce que le tampon contenait avant.

/// Taille du paquet d'enveloppe de commande.
pub const CBW_OCTETS: usize = 31;
/// Taille du paquet d'enveloppe de statut.
pub const CSW_OCTETS: usize = 13;

/// Signature du CBW : « USBC » en petit boutiste.
pub const CBW_SIGNATURE: u32 = 0x4342_5355;
/// Signature du CSW : « USBS » en petit boutiste.
pub const CSW_SIGNATURE: u32 = 0x5342_5355;

/// Sens d'un transfert, tel que le bit 7 de `bmCBWFlags` le porte.
pub const VERS_HOTE: u8 = 0x80;
pub const VERS_PERIPHERIQUE: u8 = 0x00;

/// Codes d'operation SCSI utilises.
pub mod scsi {
    pub const TEST_UNITE_PRETE: u8 = 0x00;
    pub const DEMANDE_SENS: u8 = 0x03;
    pub const INTERROGE: u8 = 0x12;
    pub const LIT_CAPACITE_10: u8 = 0x25;
    pub const LIT_10: u8 = 0x28;
    pub const ECRIT_10: u8 = 0x2A;
}

/// Statut rendu par un CSW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatutCsw {
    Reussi,
    Echec,
    /// Le peripherique est perdu : il faut une reinitialisation de la classe.
    ErreurDePhase,
    /// Une valeur que la specification ne definit pas.
    Inconnu(u8),
}

/// Ce qu'un CSW dit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Csw {
    pub etiquette: u32,
    /// Octets DEMANDES et non transferes.
    pub residu: u32,
    pub statut: StatutCsw,
}

/// Ecrit un paquet d'enveloppe de commande.
///
/// `commande` est le bloc SCSI, au plus seize octets. Le reste du champ est
/// laisse a zero : un peripherique lit `longueur_commande` octets et ignore la
/// suite, mais y laisser des restes d'une commande precedente rendrait le
/// contenu du paquet dependant de son historique.
pub fn encode_cbw(
    sortie: &mut [u8; CBW_OCTETS],
    etiquette: u32,
    octets_attendus: u32,
    vers_hote: bool,
    lun: u8,
    commande: &[u8],
) -> bool {
    if commande.is_empty() || commande.len() > 16 {
        return false;
    }
    *sortie = [0u8; CBW_OCTETS];
    // USB : petit boutiste.
    sortie[0..4].copy_from_slice(&CBW_SIGNATURE.to_le_bytes());
    sortie[4..8].copy_from_slice(&etiquette.to_le_bytes());
    sortie[8..12].copy_from_slice(&octets_attendus.to_le_bytes());
    sortie[12] = if vers_hote { VERS_HOTE } else { VERS_PERIPHERIQUE };
    sortie[13] = lun & 0x0F;
    sortie[14] = commande.len() as u8;
    sortie[15..15 + commande.len()].copy_from_slice(commande);
    true
}

/// Lit un paquet d'enveloppe de statut.
///
/// Rend `None` quand la signature n'est pas celle d'un CSW : ce n'est alors
/// pas un CSW en retard ou abime, c'est autre chose, et le decoder produirait
/// un residu et un statut inventes.
pub fn decode_csw(entree: &[u8]) -> Option<Csw> {
    if entree.len() < CSW_OCTETS {
        return None;
    }
    let signature = u32::from_le_bytes([entree[0], entree[1], entree[2], entree[3]]);
    if signature != CSW_SIGNATURE {
        return None;
    }
    Some(Csw {
        etiquette: u32::from_le_bytes([entree[4], entree[5], entree[6], entree[7]]),
        residu: u32::from_le_bytes([entree[8], entree[9], entree[10], entree[11]]),
        statut: match entree[12] {
            0x00 => StatutCsw::Reussi,
            0x01 => StatutCsw::Echec,
            0x02 => StatutCsw::ErreurDePhase,
            autre => StatutCsw::Inconnu(autre),
        },
    })
}

/// Le CSW repond-il bien a CE transfert, et en entier ?
///
/// Les trois conditions sont verifiees ENSEMBLE parce qu'elles echouent
/// separement : une etiquette qui ne correspond pas est la reponse d'une autre
/// commande ; un statut non nul est un refus ; un residu non nul est un
/// transfert partiel que le statut, lui, declare reussi.
///
/// Le nombre d'octets demandes n'est pas un parametre : un residu nul veut
/// dire « rien ne manque », quelle qu'ait ete la demande. Le repasser ici
/// laisserait croire a une verification croisee qui n'existe pas -- pour la
/// quantite reellement transferee, voir [`octets_transferes`].
pub fn transfert_complet(csw: &Csw, etiquette: u32) -> bool {
    csw.etiquette == etiquette && csw.statut == StatutCsw::Reussi && csw.residu == 0
}

/// Octets reellement transferes, d'apres le residu.
pub fn octets_transferes(csw: &Csw, octets_demandes: u32) -> u32 {
    octets_demandes.saturating_sub(csw.residu)
}

// ---------------------------------------------------------------------------
// Les blocs de commande SCSI
// ---------------------------------------------------------------------------

/// `TEST UNIT READY` : six octets, tous nuls sauf l'operation.
pub fn cdb_test_unite_prete() -> [u8; 6] {
    [scsi::TEST_UNITE_PRETE, 0, 0, 0, 0, 0]
}

/// `INQUIRY`, en demandant `octets` de reponse.
///
/// La longueur d'allocation est sur UN octet : demander plus de 255 octets
/// n'est pas possible, et l'y ecrire tronque silencieusement.
pub fn cdb_interroge(octets: u8) -> [u8; 6] {
    [scsi::INTERROGE, 0, 0, 0, octets, 0]
}

/// `REQUEST SENSE`, pour savoir POURQUOI une commande a echoue.
pub fn cdb_demande_sens(octets: u8) -> [u8; 6] {
    [scsi::DEMANDE_SENS, 0, 0, 0, octets, 0]
}

/// `READ CAPACITY (10)` : dix octets, tous nuls sauf l'operation.
pub fn cdb_lit_capacite() -> [u8; 10] {
    [scsi::LIT_CAPACITE_10, 0, 0, 0, 0, 0, 0, 0, 0, 0]
}

/// `READ (10)` ou `WRITE (10)`.
///
/// # Deux pieges dans dix octets
///
/// L'adresse et le compte sont en GRAND boutiste -- c'est du SCSI, pas de
/// l'USB --, alors que l'enveloppe qui porte cette commande est en petit
/// boutiste. Les deux boutismes cohabitent dans le meme paquet.
///
/// Et le compte de blocs n'est PAS decale de un, contrairement au `NLB` du
/// NVMe : ici, zero veut dire « zero bloc », et c'est une commande valide qui
/// ne transfere rien. Appliquer le reflexe du NVMe ferait lire un bloc de
/// moins a chaque requete.
pub fn cdb_transfert_10(ecriture: bool, lba: u32, blocs: u16) -> [u8; 10] {
    let operation = if ecriture { scsi::ECRIT_10 } else { scsi::LIT_10 };
    let a = lba.to_be_bytes();
    let n = blocs.to_be_bytes();
    [operation, 0, a[0], a[1], a[2], a[3], 0, n[0], n[1], 0]
}

// ---------------------------------------------------------------------------
// Les reponses
// ---------------------------------------------------------------------------

/// Ce que `READ CAPACITY (10)` rend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacite {
    /// Adresse du DERNIER bloc, pas leur nombre.
    pub dernier_bloc: u32,
    pub taille_bloc: u32,
}

impl Capacite {
    /// Nombre de blocs. Le champ rendu par le peripherique est l'adresse du
    /// dernier ; le confondre avec un compte perd exactement un bloc, celui de
    /// la fin -- la ou une table de partitions de secours est ecrite.
    pub fn blocs(&self) -> u64 {
        self.dernier_bloc as u64 + 1
    }

    pub fn octets(&self) -> u64 {
        self.blocs().saturating_mul(self.taille_bloc as u64)
    }
}

/// Decode une reponse `READ CAPACITY (10)`. Grand boutiste.
pub fn decode_capacite(entree: &[u8]) -> Option<Capacite> {
    if entree.len() < 8 {
        return None;
    }
    let capacite = Capacite {
        dernier_bloc: u32::from_be_bytes([entree[0], entree[1], entree[2], entree[3]]),
        taille_bloc: u32::from_be_bytes([entree[4], entree[5], entree[6], entree[7]]),
    };
    // Une taille de bloc nulle, ou qui n'est pas une puissance de deux, ne
    // decrit aucun support. La croire ferait diviser par zero plus loin, ou
    // calculer des adresses fausses sans jamais se signaler.
    if capacite.taille_bloc == 0 || !capacite.taille_bloc.is_power_of_two() {
        return None;
    }
    if capacite.taille_bloc < 512 || capacite.taille_bloc > 4096 {
        return None;
    }
    Some(capacite)
}

/// Ce qu'`INQUIRY` dit d'un peripherique.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interrogation {
    /// Zero pour un disque a acces direct. Une cle USB en est un.
    pub genre: u8,
    /// Le support est-il amovible ?
    pub amovible: bool,
}

pub fn decode_interrogation(entree: &[u8]) -> Option<Interrogation> {
    if entree.len() < 8 {
        return None;
    }
    Some(Interrogation {
        genre: entree[0] & 0x1F,
        amovible: entree[1] & 0x80 != 0,
    })
}

/// Le peripherique est-il un support de blocs utilisable ?
///
/// Le genre 0x00 est « acces direct » : disque, cle. Tout le reste -- lecteur
/// de bande, changeur, enceinte de disques -- ne se lit pas par blocs de la
/// meme facon, et le traiter comme tel produirait des transferts refuses.
pub fn support_utilisable(interrogation: &Interrogation) -> bool {
    interrogation.genre == 0x00
}

/// Le nombre de blocs qu'un transfert peut porter d'un coup.
///
/// `READ (10)` compte les blocs sur SEIZE bits : au-dela, il faut `READ (16)`.
/// Le plafond du tampon compte tout autant, et le plus petit des deux gagne.
pub fn blocs_par_transfert(tampon_octets: usize, taille_bloc: u32) -> u16 {
    if taille_bloc == 0 {
        return 0;
    }
    let par_tampon = tampon_octets / taille_bloc as usize;
    par_tampon.min(u16::MAX as usize) as u16
}
