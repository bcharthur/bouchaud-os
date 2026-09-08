//! Preuve hote du protocole de stockage de masse USB.
//!
//! Le module de production `src/drivers/usb/stockage/decodage.rs` est inclus
//! tel quel : ce qui est mis a l'epreuve ici est le code qui tournera dans le
//! noyau, et non une copie qui lui ressemble.
//!
//! # Pourquoi ces cas-la
//!
//! Une cle USB qui repond de travers ne se fabrique pas sur commande, et le
//! materiel de reference n'est pas dans cette machine. Les erreurs que ce
//! protocole punit -- les deux boutismes qui cohabitent, l'etiquette du CSW, le
//! residu d'un transfert partiel -- se prouvent en revanche sur des octets, et
//! c'est ici qu'elles doivent l'etre.

#[path = "../../src/drivers/usb/stockage/decodage.rs"]
mod decodage;

use decodage::*;

// ---------------------------------------------------------------------------
// L'enveloppe de commande
// ---------------------------------------------------------------------------

/// La signature du CBW est « USBC », et elle part en PETIT boutiste : c'est de
/// l'USB. L'ecrire a l'endroit fait rejeter chaque commande par la cle.
#[test]
fn la_signature_du_cbw_part_en_petit_boutiste() {
    let mut cbw = [0u8; CBW_OCTETS];
    assert!(encode_cbw(&mut cbw, 1, 512, true, 0, &cdb_test_unite_prete()));
    assert_eq!(&cbw[0..4], &[0x55, 0x53, 0x42, 0x43]); // "USBC"
}

#[test]
fn le_cbw_porte_etiquette_longueur_sens_et_commande() {
    let mut cbw = [0u8; CBW_OCTETS];
    let cdb = cdb_transfert_10(false, 0x1234_5678, 8);
    assert!(encode_cbw(&mut cbw, 0xDEAD_BEEF, 4096, true, 0, &cdb));

    assert_eq!(&cbw[4..8], &0xDEAD_BEEFu32.to_le_bytes());
    assert_eq!(&cbw[8..12], &4096u32.to_le_bytes());
    assert_eq!(cbw[12], VERS_HOTE);
    assert_eq!(cbw[13], 0);
    assert_eq!(cbw[14], 10);
    assert_eq!(&cbw[15..25], &cdb);
}

/// Le sens vit dans le BIT 7, et lui seul. Une ecriture porte un zero.
#[test]
fn le_sens_distingue_lecture_et_ecriture() {
    let mut lecture = [0u8; CBW_OCTETS];
    let mut ecriture = [0u8; CBW_OCTETS];
    encode_cbw(&mut lecture, 1, 512, true, 0, &cdb_lit_capacite());
    encode_cbw(&mut ecriture, 1, 512, false, 0, &cdb_lit_capacite());
    assert_eq!(lecture[12], 0x80);
    assert_eq!(ecriture[12], 0x00);
}

/// Le paquet est remis a zero a chaque encodage. Sans cela, une commande de
/// six octets laisserait derriere elle la queue de la commande de dix qui la
/// precedait, et le contenu du paquet dependrait de son historique.
#[test]
fn le_cbw_ne_garde_rien_de_la_commande_precedente() {
    let mut cbw = [0u8; CBW_OCTETS];
    encode_cbw(&mut cbw, 1, 512, true, 0, &cdb_transfert_10(false, 0xFFFF_FFFF, 0xFFFF));
    encode_cbw(&mut cbw, 2, 0, true, 0, &cdb_test_unite_prete());
    assert_eq!(cbw[14], 6);
    assert!(cbw[21..31].iter().all(|o| *o == 0), "queue non nettoyee : {:?}", &cbw[21..31]);
}

/// Un bloc de commande vide ou de plus de seize octets n'entre pas dans le
/// paquet : le refuser vaut mieux que d'en ecrire une partie.
#[test]
fn une_commande_impossible_est_refusee() {
    let mut cbw = [0u8; CBW_OCTETS];
    assert!(!encode_cbw(&mut cbw, 1, 0, true, 0, &[]));
    assert!(!encode_cbw(&mut cbw, 1, 0, true, 0, &[0u8; 17]));
    assert!(encode_cbw(&mut cbw, 1, 0, true, 0, &[0u8; 16]));
}

// ---------------------------------------------------------------------------
// L'enveloppe de statut
// ---------------------------------------------------------------------------

fn csw_brut(etiquette: u32, residu: u32, statut: u8) -> [u8; CSW_OCTETS] {
    let mut brut = [0u8; CSW_OCTETS];
    brut[0..4].copy_from_slice(&CSW_SIGNATURE.to_le_bytes());
    brut[4..8].copy_from_slice(&etiquette.to_le_bytes());
    brut[8..12].copy_from_slice(&residu.to_le_bytes());
    brut[12] = statut;
    brut
}

#[test]
fn le_csw_se_decode() {
    let csw = decode_csw(&csw_brut(0x1234, 0, 0)).unwrap();
    assert_eq!(csw.etiquette, 0x1234);
    assert_eq!(csw.residu, 0);
    assert_eq!(csw.statut, StatutCsw::Reussi);
}

/// Sans signature, ce n'est pas un CSW abime : c'est autre chose. Le decoder
/// produirait un residu et un statut inventes.
#[test]
fn un_paquet_sans_signature_n_est_pas_un_csw() {
    let mut brut = csw_brut(1, 0, 0);
    brut[0] ^= 0xFF;
    assert!(decode_csw(&brut).is_none());
    assert!(decode_csw(&brut[..CSW_OCTETS - 1]).is_none());
}

#[test]
fn les_trois_statuts_sont_distingues() {
    assert_eq!(decode_csw(&csw_brut(1, 0, 0)).unwrap().statut, StatutCsw::Reussi);
    assert_eq!(decode_csw(&csw_brut(1, 0, 1)).unwrap().statut, StatutCsw::Echec);
    assert_eq!(decode_csw(&csw_brut(1, 0, 2)).unwrap().statut, StatutCsw::ErreurDePhase);
    assert_eq!(decode_csw(&csw_brut(1, 0, 9)).unwrap().statut, StatutCsw::Inconnu(9));
}

/// L'ETIQUETTE est ce qui distingue notre reponse de celle d'une commande
/// precedente. Sans ce controle, un transfert « reussit » en rendant les octets
/// d'un autre secteur.
#[test]
fn une_etiquette_qui_ne_correspond_pas_invalide_le_transfert() {
    let csw = decode_csw(&csw_brut(0xAAAA, 0, 0)).unwrap();
    assert!(transfert_complet(&csw, 0xAAAA));
    assert!(!transfert_complet(&csw, 0xBBBB));
}

/// Un residu non nul est un transfert PARTIEL, et le statut le declare quand
/// meme reussi. Ne lire que le statut fait croire a un secteur entier alors que
/// la moitie du tampon n'a pas ete ecrite.
#[test]
fn un_residu_non_nul_n_est_pas_un_transfert_complet() {
    let csw = decode_csw(&csw_brut(7, 256, 0)).unwrap();
    assert_eq!(csw.statut, StatutCsw::Reussi);
    assert!(!transfert_complet(&csw, 7));
    assert_eq!(octets_transferes(&csw, 512), 256);
}

#[test]
fn un_residu_aberrant_ne_rend_pas_un_compte_negatif() {
    let csw = decode_csw(&csw_brut(7, 99_999, 0)).unwrap();
    assert_eq!(octets_transferes(&csw, 512), 0);
}

// ---------------------------------------------------------------------------
// Les blocs de commande SCSI
// ---------------------------------------------------------------------------

/// Le SCSI est en GRAND boutiste, dans un paquet USB qui est en petit. Les deux
/// cohabitent, et appliquer le meme aux deux lit a la mauvaise adresse.
#[test]
fn l_adresse_scsi_part_en_grand_boutiste() {
    let cdb = cdb_transfert_10(false, 0x0102_0304, 1);
    assert_eq!(&cdb[2..6], &[0x01, 0x02, 0x03, 0x04]);
}

/// Le compte de blocs de `READ (10)` n'est PAS decale de un, contrairement au
/// `NLB` du NVMe. Appliquer le reflexe du NVMe lit un bloc de moins a chaque
/// requete.
#[test]
fn le_compte_de_blocs_n_est_pas_decale_de_un() {
    let cdb = cdb_transfert_10(false, 0, 1);
    assert_eq!(&cdb[7..9], &[0x00, 0x01]);
    let cdb = cdb_transfert_10(false, 0, 8);
    assert_eq!(&cdb[7..9], &[0x00, 0x08]);
}

#[test]
fn lecture_et_ecriture_ont_des_operations_distinctes() {
    assert_eq!(cdb_transfert_10(false, 0, 1)[0], scsi::LIT_10);
    assert_eq!(cdb_transfert_10(true, 0, 1)[0], scsi::ECRIT_10);
}

#[test]
fn les_commandes_courtes_ont_la_bonne_operation() {
    assert_eq!(cdb_test_unite_prete()[0], scsi::TEST_UNITE_PRETE);
    assert_eq!(cdb_interroge(36)[0], scsi::INTERROGE);
    assert_eq!(cdb_interroge(36)[4], 36);
    assert_eq!(cdb_demande_sens(18)[4], 18);
    assert_eq!(cdb_lit_capacite()[0], scsi::LIT_CAPACITE_10);
}

// ---------------------------------------------------------------------------
// Les reponses
// ---------------------------------------------------------------------------

fn capacite_brute(dernier: u32, taille: u32) -> [u8; 8] {
    let mut brut = [0u8; 8];
    brut[0..4].copy_from_slice(&dernier.to_be_bytes());
    brut[4..8].copy_from_slice(&taille.to_be_bytes());
    brut
}

/// Le champ rendu est l'adresse du DERNIER bloc. Le confondre avec un compte
/// perd exactement le bloc de la fin -- celui ou une table de partitions de
/// secours est ecrite.
#[test]
fn la_capacite_rend_le_dernier_bloc_et_non_leur_nombre() {
    let c = decode_capacite(&capacite_brute(0x0000_0F9F, 512)).unwrap();
    assert_eq!(c.dernier_bloc, 0x0F9F);
    assert_eq!(c.blocs(), 0x0FA0);
    assert_eq!(c.taille_bloc, 512);
    assert_eq!(c.octets(), 0x0FA0 * 512);
}

/// Une taille de bloc nulle, non puissance de deux, ou hors plage ne decrit
/// aucun support. La croire ferait diviser par zero, ou calculer des adresses
/// fausses sans jamais se signaler.
#[test]
fn une_taille_de_bloc_impossible_est_refusee() {
    assert!(decode_capacite(&capacite_brute(100, 0)).is_none());
    assert!(decode_capacite(&capacite_brute(100, 500)).is_none());
    assert!(decode_capacite(&capacite_brute(100, 256)).is_none());
    assert!(decode_capacite(&capacite_brute(100, 8192)).is_none());
    assert!(decode_capacite(&capacite_brute(100, 512)).is_some());
    assert!(decode_capacite(&capacite_brute(100, 4096)).is_some());
    assert!(decode_capacite(&[0u8; 7]).is_none());
}

#[test]
fn l_interrogation_dit_le_genre_et_l_amovibilite() {
    let mut brut = [0u8; 36];
    brut[0] = 0x00;
    brut[1] = 0x80;
    let i = decode_interrogation(&brut).unwrap();
    assert_eq!(i.genre, 0);
    assert!(i.amovible);
    assert!(support_utilisable(&i));

    // Un lecteur de bande n'est pas un support de blocs.
    brut[0] = 0x01;
    assert!(!support_utilisable(&decode_interrogation(&brut).unwrap()));
    assert!(decode_interrogation(&brut[..7]).is_none());
}

/// `READ (10)` compte les blocs sur seize bits : le plafond du tampon et celui
/// du champ comptent tous les deux, et le plus petit gagne.
#[test]
fn le_transfert_est_borne_par_le_tampon_et_par_le_champ() {
    assert_eq!(blocs_par_transfert(64 * 1024, 512), 128);
    assert_eq!(blocs_par_transfert(512, 512), 1);
    assert_eq!(blocs_par_transfert(256, 512), 0);
    // Un tampon immense reste borne par les seize bits du champ.
    assert_eq!(blocs_par_transfert(usize::MAX, 512), u16::MAX);
    assert_eq!(blocs_par_transfert(4096, 0), 0);
}
