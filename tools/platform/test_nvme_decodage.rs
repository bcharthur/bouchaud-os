//! Preuve hote du decodage NVM Express.
//!
//! Le module de production `src/drivers/block/nvme/decodage.rs` est inclus tel
//! quel : il ne touche aucun registre, ce qui permet de lui donner ici les
//! valeurs que le materiel ne produira qu'un jour, sur une machine qu'on n'a
//! pas.
//!
//! Ce que ces tests cherchent, ce ne sont pas les cas nominaux -- une lecture
//! d'un secteur depuis un tampon aligne sur un disque en 512 octets marche
//! meme avec un pilote faux. Ce sont les trois familles de fautes qui
//! survivent a toute la mise au point sous QEMU :
//!
//!   * une foulee de sonnette differente de huit ;
//!   * un compte de blocs qui oublie le decalage de un ;
//!   * un transfert dont le DECALAGE, et non la taille, fait basculer dans le
//!     cas « liste PRP ».

#![allow(dead_code)]

#[path = "../../src/drivers/block/nvme/decodage.rs"]
mod decodage;

use decodage::*;

const PAGE: usize = 4096;

// ---------------------------------------------------------------------------
// CAP : les champs decales de un, et la foulee des sonnettes.
// ---------------------------------------------------------------------------

#[test]
fn mqes_est_decale_de_un() {
    // Un controleur qui annonce 0x1F annonce TRENTE-DEUX entrees, pas 31.
    let cap = 0x1Fu64;
    assert_eq!(
        decode_cap(cap).entrees_max,
        32,
        "MQES est un champ decale de un ; le lire brut fait dimensionner \
         toutes les files une entree trop court"
    );
}

#[test]
fn une_foulee_de_sonnette_non_nulle_deplace_toutes_les_sonnettes() {
    // DSTRD = 0 : le cas de QEMU et de presque tout le materiel.
    let plate = decode_cap(0);
    assert_eq!(plate.foulee_sonnette, 4);
    assert_eq!(decalage_sonnette(0, false, plate.foulee_sonnette), 0x1000);
    assert_eq!(decalage_sonnette(0, true, plate.foulee_sonnette), 0x1004);
    assert_eq!(decalage_sonnette(1, false, plate.foulee_sonnette), 0x1008);

    // DSTRD = 2 : foulee de 16 octets. Un pilote qui code 4 ou 8 en dur sonne
    // dans le vide, et la commande reste dans la file pour toujours.
    let large = decode_cap(2u64 << 32);
    assert_eq!(large.foulee_sonnette, 16);
    assert_eq!(decalage_sonnette(0, false, large.foulee_sonnette), 0x1000);
    assert_eq!(decalage_sonnette(0, true, large.foulee_sonnette), 0x1010);
    assert_eq!(
        decalage_sonnette(1, false, large.foulee_sonnette),
        0x1020,
        "la sonnette de soumission de la file 1 est la troisieme, pas la seconde"
    );
}

#[test]
fn un_controleur_qui_annonce_zero_delai_recoit_quand_meme_du_temps() {
    // TO = 0 ne veut pas dire « instantane ». L'attendre zero milliseconde le
    // declarerait mort avant qu'il ait eu le droit de repondre.
    assert_eq!(decode_cap(0).delai_pret_ms, 500);
    // TO = 20 -> 10 secondes.
    assert_eq!(decode_cap(20u64 << 24).delai_pret_ms, 10_000);
}

#[test]
fn cap_expose_le_jeu_de_commandes_et_la_contiguite() {
    let cap = (1u64 << 37) | (1u64 << 16);
    let c = decode_cap(cap);
    assert!(c.jeu_nvm);
    assert!(c.files_contigues);
    assert!(!decode_cap(0).jeu_nvm);
}

#[test]
fn aqa_decale_ses_deux_champs() {
    // 32 entrees de chaque cote -> 31 dans les deux champs.
    assert_eq!(valeur_aqa(32, 32), (31 << 16) | 31);
}

#[test]
fn cc_porte_des_logarithmes_et_non_des_tailles() {
    let cc = valeur_cc_demarrage(0);
    assert_eq!((cc >> 16) & 0xF, 6, "IOSQES = log2(64)");
    assert_eq!((cc >> 20) & 0xF, 4, "IOCQES = log2(16)");
    assert_eq!(cc & 1, 1, "EN");
    assert_eq!((cc >> 7) & 0xF, 0, "MPS");
    let cc4 = valeur_cc_demarrage(1);
    assert_eq!((cc4 >> 7) & 0xF, 1);
}

#[test]
fn csts_distingue_pas_pret_et_panne_fatale() {
    assert!(!pret(0));
    assert!(pret(1));
    assert!(!panne_fatale(1));
    assert!(
        panne_fatale(0x2),
        "sans ce bit, un controleur qui a deja abandonne est attendu le delai entier"
    );
}

// ---------------------------------------------------------------------------
// Les commandes : le decalage de un du compte de blocs.
// ---------------------------------------------------------------------------

#[test]
fn nlb_est_decale_de_un() {
    let un_bloc = commande_transfert(false, 7, 1, 0, 1, 0x1000, 0);
    assert_eq!(
        un_bloc[12] & 0xFFFF,
        0,
        "un bloc s'ecrit zero ; y ecrire un lit DEUX blocs, et sur une \
         ecriture ce bloc de trop ecrase le suivant"
    );
    let huit = commande_transfert(false, 7, 1, 0, 8, 0x1000, 0);
    assert_eq!(huit[12] & 0xFFFF, 7);
}

#[test]
fn une_commande_de_transfert_porte_son_lba_sur_soixante_quatre_bits() {
    let lba = 0x0000_0007_DEAD_BEEFu64;
    let sqe = commande_transfert(true, 3, 1, lba, 4, 0x2000, 0x3000);
    assert_eq!(sqe[10], 0xDEAD_BEEF);
    assert_eq!(sqe[11], 0x0000_0007);
    assert_eq!(sqe[0] & 0xFF, 0x01, "ecriture");
    assert_eq!((sqe[0] >> 16) & 0xFFFF, 3, "identifiant de commande");
    assert_eq!(sqe[1], 1, "NSID");
    assert_eq!(sqe[6], 0x2000);
    assert_eq!(sqe[8], 0x3000);
}

#[test]
fn lecture_et_ecriture_ne_partagent_pas_leur_opcode() {
    let lecture = commande_transfert(false, 0, 1, 0, 1, 0, 0);
    let ecriture = commande_transfert(true, 0, 1, 0, 1, 0, 0);
    assert_eq!(lecture[0] & 0xFF, 0x02);
    assert_eq!(ecriture[0] & 0xFF, 0x01);
}

#[test]
fn prp_se_pose_sur_soixante_quatre_bits() {
    let haut = 0x0000_0003_4000_0000u64;
    let sqe = sqe_prp(sqe_vide(), haut, haut + 4096);
    assert_eq!(sqe[6], 0x4000_0000);
    assert_eq!(sqe[7], 3);
    assert_eq!(sqe[8], 0x4000_1000);
    assert_eq!(sqe[9], 3);
}

#[test]
fn creer_une_file_de_soumission_nomme_sa_file_d_achevement() {
    let sqe = commande_cree_sq(1, 1, 64, 0x5000, 1);
    assert_eq!(sqe[0] & 0xFF, 0x01);
    assert_eq!(sqe[10] & 0xFFFF, 1, "identifiant de file");
    assert_eq!((sqe[10] >> 16) & 0xFFFF, 63, "taille decalee de un");
    assert_eq!(sqe[11] & 1, 1, "physiquement contigue");
    assert_eq!(
        (sqe[11] >> 16) & 0xFFFF,
        1,
        "sans ce champ les achevements partent vers la file admin, ou \
         personne ne les attend"
    );
}

#[test]
fn creer_une_file_d_achevement_n_active_pas_l_interruption() {
    let sqe = commande_cree_cq(1, 1, 64, 0x6000);
    assert_eq!(sqe[0] & 0xFF, 0x05);
    assert_eq!(sqe[11] & 1, 1, "PC");
    assert_eq!(
        sqe[11] & 0x2,
        0,
        "ce pilote scrute ; armer IEN ferait monter un IRQ qu'aucun vecteur \
         ne recoit"
    );
}

#[test]
fn le_nombre_de_files_demande_est_decale_de_un_des_deux_cotes() {
    let sqe = commande_nombre_de_files(2, 4);
    assert_eq!(sqe[10], 0x07, "FID");
    assert_eq!(sqe[11] & 0xFFFF, 3);
    assert_eq!((sqe[11] >> 16) & 0xFFFF, 3);
}

#[test]
fn identify_porte_son_cns() {
    let sqe = commande_identifie(9, 1, cns::NAMESPACE, 0x7000);
    assert_eq!(sqe[0] & 0xFF, 0x06);
    assert_eq!(sqe[10], 0);
    assert_eq!(sqe[1], 1);
    let controleur = commande_identifie(9, 0, cns::CONTROLEUR, 0x7000);
    assert_eq!(controleur[10], 1);
}

// ---------------------------------------------------------------------------
// Les achevements : le bit de phase.
// ---------------------------------------------------------------------------

#[test]
fn un_achevement_expose_sa_phase_et_son_statut() {
    // CID = 0x0042, P = 1, SC = 0x81, SCT = 1.
    let dw3 = 0x0042u32 | (1 << 16) | (0x81 << 17) | (1 << 25);
    let a = decode_achevement([0, 0, (2 << 16) | 5, dw3]);
    assert_eq!(a.identifiant, 0x42);
    assert!(a.phase);
    assert_eq!(a.code_statut, 0x81);
    assert_eq!(a.type_statut, 1);
    assert_eq!(a.tete_sq, 5);
    assert_eq!(a.file_sq, 2);
    assert!(!a.reussi());
}

#[test]
fn seul_un_statut_entierement_nul_est_une_reussite() {
    let ok = decode_achevement([0, 0, 0, 1 << 16]);
    assert!(ok.reussi());
    // Un code de statut non nul avec un type nul reste un echec : c'est le cas
    // des erreurs generiques, les plus frequentes.
    let echec = decode_achevement([0, 0, 0, (1 << 16) | (0x0B << 17)]);
    assert!(!echec.reussi());
}

// ---------------------------------------------------------------------------
// Les listes PRP : le decalage decide, pas la taille.
// ---------------------------------------------------------------------------

#[test]
fn un_transfert_qui_tient_dans_sa_page_n_utilise_pas_prp2() {
    assert_eq!(plan_prp(0x1000, 512, PAGE), Prp::UnePage { prp1: 0x1000 });
    assert_eq!(plan_prp(0x1000, 4096, PAGE), Prp::UnePage { prp1: 0x1000 });
}

#[test]
fn le_decalage_et_non_la_taille_fait_basculer_le_plan() {
    // 8 Kio depuis une adresse ALIGNEE : exactement deux pages.
    assert_eq!(
        plan_prp(0x2000, 8192, PAGE),
        Prp::DeuxPages { prp1: 0x2000, prp2: 0x3000 }
    );
    // Le MEME transfert, decale d'un seul octet, occupe TROIS pages et exige
    // donc une liste. C'est le cas qu'un pilote teste avec des tampons alignes
    // ne rencontre jamais, jusqu'au jour ou un appelant ne l'est pas.
    assert_eq!(
        plan_prp(0x2001, 8192, PAGE),
        Prp::Liste { prp1: 0x2001, entrees: 2 }
    );
}

#[test]
fn une_page_pleine_plus_un_octet_demande_deux_pages() {
    assert_eq!(
        plan_prp(0x4000, 4097, PAGE),
        Prp::DeuxPages { prp1: 0x4000, prp2: 0x5000 }
    );
}

#[test]
fn un_transfert_decale_qui_ne_deborde_que_d_une_page_reste_a_deux() {
    // Decalage 0xF00, 0x200 octets : 0x100 dans la premiere page, 0x100 dans
    // la seconde. Deux pages, pas de liste.
    assert_eq!(
        plan_prp(0x4F00, 0x200, PAGE),
        Prp::DeuxPages { prp1: 0x4F00, prp2: 0x5000 }
    );
}

#[test]
fn une_liste_compte_les_pages_apres_la_premiere() {
    // 64 Kio alignes = 16 pages : la premiere dans PRP1, quinze dans la liste.
    assert_eq!(
        plan_prp(0x10000, 65536, PAGE),
        Prp::Liste { prp1: 0x10000, entrees: 15 }
    );
}

#[test]
fn les_entrees_de_la_liste_commencent_a_la_seconde_page() {
    // La liste ne contient PAS la premiere page : elle est deja dans PRP1.
    assert_eq!(page_de_la_liste(0x10000, 0, PAGE), 0x11000);
    assert_eq!(page_de_la_liste(0x10000, 1, PAGE), 0x12000);
    // Depuis une adresse decalee, la liste part quand meme de la page suivante.
    assert_eq!(page_de_la_liste(0x10ABC, 0, PAGE), 0x11000);
}

#[test]
fn octets_dans_la_premiere_page_ne_deborde_jamais() {
    assert_eq!(octets_dans_la_premiere_page(0, 100, PAGE), 100);
    assert_eq!(octets_dans_la_premiere_page(4000, 100, PAGE), 96);
    assert_eq!(octets_dans_la_premiere_page(0, 99999, PAGE), PAGE);
}

// ---------------------------------------------------------------------------
// Identify Namespace : la taille de bloc n'est pas 512.
// ---------------------------------------------------------------------------

fn identify_namespace(nsze: u64, flbas: u8, formats: &[(u32, u32)]) -> Vec<u8> {
    let mut t = vec![0u8; 4096];
    t[..8].copy_from_slice(&nsze.to_le_bytes());
    t[25] = (formats.len().saturating_sub(1)) as u8;
    t[26] = flbas;
    for (i, (ms, lbads)) in formats.iter().enumerate() {
        let mot = (ms & 0xFFFF) | (lbads << 16);
        t[128 + i * 4..132 + i * 4].copy_from_slice(&mot.to_le_bytes());
    }
    t
}

#[test]
fn un_disque_en_512_est_lu_en_512() {
    let t = identify_namespace(1_000_000, 0, &[(0, 9)]);
    assert_eq!(
        format_bloc(&t),
        Some(FormatBloc { taille_bloc: 512, metadonnees: 0 })
    );
    assert_eq!(blocs_du_namespace(&t), 1_000_000);
}

#[test]
fn un_disque_en_4096_n_est_pas_lu_en_512() {
    // FLBAS designe le format 1, qui est en 4096. Un pilote qui suppose 512
    // lit le bon nombre d'octets HUIT FOIS TROP LOIN, et ne s'en apercoit pas.
    let t = identify_namespace(500_000, 1, &[(0, 9), (0, 12)]);
    assert_eq!(
        format_bloc(&t),
        Some(FormatBloc { taille_bloc: 4096, metadonnees: 0 })
    );
}

#[test]
fn un_format_avec_metadonnees_les_annonce() {
    let t = identify_namespace(10, 0, &[(8, 9)]);
    assert_eq!(
        format_bloc(&t),
        Some(FormatBloc { taille_bloc: 512, metadonnees: 8 })
    );
}

#[test]
fn un_format_inutilisable_rend_none_plutot_que_de_supposer_512() {
    // LBADS = 0 n'est pas un bloc : c'est un champ non initialise. Supposer
    // 512 ferait calculer des adresses fausses sans jamais se signaler.
    let t = identify_namespace(10, 0, &[(0, 0)]);
    assert_eq!(format_bloc(&t), None);
    // Un FLBAS qui designe un format au-dela de NLBAF est tout aussi faux.
    let hors = identify_namespace(10, 5, &[(0, 9)]);
    assert_eq!(format_bloc(&hors), None);
    // Un tampon tronque ne doit pas paniquer.
    assert_eq!(format_bloc(&[0u8; 32]), None);
}

#[test]
fn mdts_est_un_logarithme_et_zero_veut_dire_sans_limite() {
    let mut ctrl = vec![0u8; 4096];
    ctrl[77] = 0;
    assert_eq!(
        transfert_max_octets(&ctrl, 4096),
        None,
        "zero veut dire « pas de limite annoncee », pas « zero octet »"
    );
    ctrl[77] = 5;
    assert_eq!(transfert_max_octets(&ctrl, 4096), Some(4096 << 5));
}

#[test]
fn la_liste_des_namespaces_n_est_pas_supposee_commencer_a_un() {
    let mut liste = vec![0u8; 4096];
    liste[0..4].copy_from_slice(&7u32.to_le_bytes());
    liste[4..8].copy_from_slice(&9u32.to_le_bytes());
    let mut sortie = [0u32; 4];
    assert_eq!(namespaces_actifs(&liste, &mut sortie), 2);
    assert_eq!(&sortie[..2], &[7, 9]);
}

#[test]
fn la_liste_des_namespaces_s_arrete_au_premier_zero() {
    let mut liste = vec![0u8; 64];
    liste[0..4].copy_from_slice(&1u32.to_le_bytes());
    // liste[4..8] reste nul : fin de liste.
    liste[8..12].copy_from_slice(&3u32.to_le_bytes());
    let mut sortie = [0u32; 8];
    assert_eq!(namespaces_actifs(&liste, &mut sortie), 1);
    assert_eq!(sortie[0], 1);
}
