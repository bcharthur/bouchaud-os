//! Preuve hote de l'arithmetique d'un concentrateur USB.
//!
//! Un clavier derriere un concentrateur ne repond pas si UN champ de bits est
//! faux, et il n'y a aucun message d'erreur : le controleur adresse un
//! peripherique qui n'est pas la. On ne le decouvre qu'en branchant un vrai
//! concentrateur -- c'est-a-dire jamais, en integration continue.
//!
//! Les valeurs attendues ci-dessous ne sont pas recopiees du module. Elles
//! viennent des specifications, transcrites ici a la main : xHCI 1.2 §6.2.2
//! pour le contexte de slot, USB 2.0 §11.23.2 et §11.24.2.7 pour le
//! descripteur et l'etat d'un port.

#![allow(dead_code)]

#[path = "../../src/drivers/usb/concentrateur/decodage.rs"]
mod c;

use c::*;

// ---------------------------------------------------------------------------
// La chaine de route.
// ---------------------------------------------------------------------------

#[test]
fn le_premier_etage_occupe_les_quatre_bits_de_poids_faible() {
    assert_eq!(route_enfant(0, 0, 1), Some(0x1));
    assert_eq!(route_enfant(0, 0, 4), Some(0x4));
    assert_eq!(route_enfant(0, 0, 15), Some(0xf));
}

#[test]
fn chaque_etage_decale_de_quatre_bits() {
    // Un concentrateur au port 3 de la racine, un peripherique a son port 2.
    let concentrateur = route_enfant(0, 0, 3).unwrap();
    assert_eq!(concentrateur, 0x3);
    let peripherique = route_enfant(concentrateur, 1, 2).unwrap();
    assert_eq!(peripherique, 0x23, "le port du parent reste en bits 0-3");

    // Et un troisieme etage.
    assert_eq!(route_enfant(peripherique, 2, 7), Some(0x723));
}

#[test]
fn un_port_au_dela_de_quinze_sature_au_lieu_de_reboucler() {
    // Masquer donnerait 1 pour le port 17 -- donc un AUTRE port, sur lequel il
    // y a peut-etre un autre peripherique, et l'adressage reussirait dessus.
    assert_eq!(route_enfant(0, 0, 17), Some(0xf));
    assert_eq!(route_enfant(0, 0, 16), Some(0xf));
    assert_eq!(route_enfant(0, 0, 255), Some(0xf));
}

#[test]
fn le_port_zero_n_existe_pas() {
    // Les ports d'un concentrateur sont numerotes a partir de un. Un zero
    // laisserait la route inchangee, donc designerait le concentrateur.
    assert_eq!(route_enfant(0, 0, 0), None);
}

#[test]
fn au_dela_de_cinq_etages_le_champ_est_plein() {
    let mut route = 0;
    for profondeur in 0..ETAGES_MAX {
        route = route_enfant(route, profondeur, 1).expect("cinq etages tiennent");
    }
    assert_eq!(route, 0x11111);
    assert_eq!(
        route_enfant(route, ETAGES_MAX, 1),
        None,
        "un sixieme concentrateur n'est pas adressable, quel que soit le code"
    );
}

#[test]
fn la_route_tient_dans_vingt_bits() {
    let mut route = 0;
    for profondeur in 0..ETAGES_MAX {
        route = route_enfant(route, profondeur, 15).unwrap();
    }
    assert_eq!(route, 0x000f_ffff);
    assert_eq!(route & !0x000f_ffff, 0);
}

// ---------------------------------------------------------------------------
// Le contexte de slot.
// ---------------------------------------------------------------------------

#[test]
fn le_mot_zero_place_chaque_champ_ou_le_controleur_le_lit() {
    // xHCI 1.2 figure 6-2 : Route 0-19, Speed 20-23, MTT 25, Hub 26,
    // Context Entries 27-31.
    let dw0 = slot_dw0(0x23, VITESSE_BASSE, 1, false, false);
    assert_eq!(dw0 & 0x000f_ffff, 0x23);
    assert_eq!((dw0 >> 20) & 0xf, 2);
    assert_eq!((dw0 >> 27) & 0x1f, 1);
    assert_eq!(dw0 & (1 << 26), 0);
}

#[test]
fn le_bit_hub_est_le_vingt_sixieme() {
    let sans = slot_dw0(0, VITESSE_HAUTE, 1, false, false);
    let avec = slot_dw0(0, VITESSE_HAUTE, 1, true, false);
    assert_eq!(avec ^ sans, 1 << 26);
}

#[test]
fn le_bit_multi_tt_est_le_vingt_cinquieme() {
    let sans = slot_dw0(0, VITESSE_HAUTE, 1, true, false);
    let avec = slot_dw0(0, VITESSE_HAUTE, 1, true, true);
    assert_eq!(avec ^ sans, 1 << 25);
}

#[test]
fn la_route_ne_deborde_pas_sur_la_vitesse() {
    // Une route qui deborderait ecraserait le champ de vitesse : le
    // peripherique serait adresse a la mauvaise vitesse, donc muet.
    let dw0 = slot_dw0(0xffff_ffff, VITESSE_BASSE, 1, false, false);
    assert_eq!((dw0 >> 20) & 0xf, VITESSE_BASSE as u32);
    assert_eq!(dw0 & 0x000f_ffff, 0x000f_ffff);
}

#[test]
fn le_mot_un_porte_le_port_racine_en_bits_seize_a_vingt_trois() {
    let dw1 = slot_dw1(0, 5, 4);
    assert_eq!((dw1 >> 16) & 0xff, 5);
    assert_eq!((dw1 >> 24) & 0xff, 4);
    assert_eq!(dw1 & 0xffff, 0);
}

#[test]
fn le_mot_deux_porte_le_transactionneur() {
    // TT Hub Slot ID 0-7, TT Port Number 8-15, TT Think Time 16-17.
    let dw2 = slot_dw2(6, 3, 2);
    assert_eq!(dw2 & 0xff, 6);
    assert_eq!((dw2 >> 8) & 0xff, 3);
    assert_eq!((dw2 >> 16) & 0x3, 2);
}

#[test]
fn le_temps_de_reflexion_ne_deborde_pas_sur_l_interrupteur() {
    let dw2 = slot_dw2(0, 0, 0xff);
    assert_eq!((dw2 >> 16) & 0x3, 3);
    assert_eq!(dw2 >> 18, 0, "le champ fait deux bits, pas huit");
}

// ---------------------------------------------------------------------------
// Le transactionneur.
// ---------------------------------------------------------------------------

#[test]
fn un_clavier_basse_vitesse_derriere_un_concentrateur_haute_vitesse_en_a_besoin() {
    // C'est le cas COURANT : la quasi-totalite des claviers filaires sont
    // basse vitesse, et tout concentrateur moderne est haute vitesse.
    assert!(requiert_transactionneur(VITESSE_HAUTE, VITESSE_BASSE));
    assert!(requiert_transactionneur(VITESSE_HAUTE, VITESSE_PLEINE));
}

#[test]
fn un_peripherique_a_la_meme_vitesse_que_son_concentrateur_n_en_a_pas_besoin() {
    assert!(!requiert_transactionneur(VITESSE_HAUTE, VITESSE_HAUTE));
    assert!(!requiert_transactionneur(VITESSE_PLEINE, VITESSE_BASSE));
    assert!(!requiert_transactionneur(VITESSE_BASSE, VITESSE_BASSE));
}

#[test]
fn un_peripherique_superspeed_n_a_jamais_de_transactionneur() {
    assert!(!requiert_transactionneur(VITESSE_SUPER, VITESSE_SUPER));
    assert!(!requiert_transactionneur(VITESSE_SUPER, VITESSE_HAUTE));
}

// ---------------------------------------------------------------------------
// Le descripteur du concentrateur.
// ---------------------------------------------------------------------------

fn descripteur_usb2(ports: u8, caracteristiques: u16, alimentation: u8) -> [u8; 9] {
    let c = caracteristiques.to_le_bytes();
    [9, 0x29, ports, c[0], c[1], alimentation, 100, 0x00, 0xff]
}

#[test]
fn le_nombre_de_ports_se_lit_en_position_deux() {
    let d = descripteur(&descripteur_usb2(4, 0, 25)).expect("descripteur valide");
    assert_eq!(d.ports, 4);
}

#[test]
fn le_delai_d_alimentation_se_compte_en_unites_de_deux_millisecondes() {
    // bPwrOn2PwrGood = 50 veut dire 100 ms. Le prendre pour 50 interrogerait
    // le port avant qu'il soit alimente : il se dirait vide.
    let d = descripteur(&descripteur_usb2(4, 0, 50)).unwrap();
    assert_eq!(d.delai_alimentation_ms, 100);
}

#[test]
fn le_delai_d_alimentation_sature_au_lieu_de_deborder() {
    let d = descripteur(&descripteur_usb2(4, 0, 255)).unwrap();
    assert_eq!(d.delai_alimentation_ms, 510);
}

#[test]
fn le_temps_de_reflexion_vient_des_bits_cinq_et_six_des_caracteristiques() {
    for brut in 0u16..4 {
        let d = descripteur(&descripteur_usb2(4, brut << 5, 10)).unwrap();
        assert_eq!(d.temps_reflexion, brut as u8);
    }
    // Les autres bits ne doivent pas y entrer.
    let d = descripteur(&descripteur_usb2(4, 0xff9f, 10)).unwrap();
    assert_eq!(d.temps_reflexion, 0);
}

#[test]
fn un_descripteur_superspeed_se_lit_aux_memes_positions() {
    let d = descripteur(&[12, 0x2a, 4, 0x00, 0x00, 50, 100, 0, 0, 0, 0, 0])
        .expect("le descripteur SuperSpeed partage les six premiers octets");
    assert_eq!(d.ports, 4);
    assert_eq!(d.delai_alimentation_ms, 100);
}

#[test]
fn un_descripteur_qui_n_en_est_pas_un_est_refuse() {
    // Un peripherique etranger peut rendre n'importe quoi. Le lire quand meme
    // ferait boucler la traversee sur un nombre de ports invente.
    assert!(descripteur(&[9, 0x01, 4, 0, 0, 10, 0, 0, 0]).is_none());
    assert!(descripteur(&[]).is_none());
    assert!(descripteur(&[9, 0x29, 4]).is_none(), "tronque");
    assert!(
        descripteur(&descripteur_usb2(0, 0, 10)).is_none(),
        "un concentrateur sans port n'est pas un concentrateur"
    );
    assert!(
        descripteur(&[200, 0x29, 4, 0, 0, 10, 0, 0, 0]).is_none(),
        "une longueur annoncee plus grande que ce qu'on a lu"
    );
}

// ---------------------------------------------------------------------------
// L'etat d'un port.
// ---------------------------------------------------------------------------

fn etat_brut(etat: u16, changements: u16) -> [u8; 4] {
    let e = etat.to_le_bytes();
    let c = changements.to_le_bytes();
    [e[0], e[1], c[0], c[1]]
}

#[test]
fn un_port_vide_se_dit_vide() {
    let p = etat_port(&etat_brut(1 << 8, 0), false).unwrap();
    assert!(!p.connecte);
    assert!(!p.active);
    assert!(p.alimente);
    assert_eq!(p.vitesse, 0);
}

#[test]
fn un_concentrateur_usb2_code_la_vitesse_par_deux_bits_separes() {
    // USB 2.0 tableau 11-21 : bit 9 basse vitesse, bit 10 haute vitesse,
    // aucun des deux = pleine vitesse.
    let actif = (1 << 0) | (1 << 1) | (1 << 8);
    assert_eq!(
        etat_port(&etat_brut(actif, 0), false).unwrap().vitesse,
        VITESSE_PLEINE
    );
    assert_eq!(
        etat_port(&etat_brut(actif | (1 << 9), 0), false).unwrap().vitesse,
        VITESSE_BASSE
    );
    assert_eq!(
        etat_port(&etat_brut(actif | (1 << 10), 0), false).unwrap().vitesse,
        VITESSE_HAUTE
    );
}

#[test]
fn la_vitesse_ne_veut_rien_dire_avant_que_le_port_soit_actif() {
    // Les bits de vitesse ne sont valides qu'apres la reinitialisation. Les
    // lire avant programme une taille de paquet fausse.
    let connecte_non_actif = (1 << 0) | (1 << 8) | (1 << 9);
    assert_eq!(
        etat_port(&etat_brut(connecte_non_actif, 0), false).unwrap().vitesse,
        0
    );
}

#[test]
fn un_concentrateur_superspeed_place_l_alimentation_ailleurs() {
    // Bit 9 est l'alimentation sur un concentrateur SuperSpeed, et la basse
    // vitesse sur un USB 2.0. Lire l'un avec la disposition de l'autre rend
    // une vitesse fausse.
    let actif = (1 << 0) | (1 << 1) | (1 << 9);
    let ss = etat_port(&etat_brut(actif, 0), true).unwrap();
    assert!(ss.alimente);
    assert_eq!(ss.vitesse, VITESSE_SUPER);

    let usb2 = etat_port(&etat_brut(actif, 0), false).unwrap();
    assert!(!usb2.alimente, "bit 9 n'est pas l'alimentation en USB 2.0");
    assert_eq!(usb2.vitesse, VITESSE_BASSE);
}

#[test]
fn la_reinitialisation_en_cours_est_le_bit_quatre() {
    let p = etat_port(&etat_brut((1 << 0) | (1 << 4), 0), false).unwrap();
    assert!(p.en_reinitialisation);
    let q = etat_port(&etat_brut(1 << 0, 0), false).unwrap();
    assert!(!q.en_reinitialisation);
}

#[test]
fn les_changements_sont_rendus_tels_quels() {
    let p = etat_port(&etat_brut(0, (1 << 0) | (1 << 4)), false).unwrap();
    assert_eq!(p.changements, (1 << 0) | (1 << 4));
}

#[test]
fn un_etat_tronque_est_refuse() {
    assert!(etat_port(&[0, 0, 0], false).is_none());
    assert!(etat_port(&[], false).is_none());
}

#[test]
fn chaque_bit_de_changement_a_la_fonctionnalite_qui_l_efface() {
    // Un changement qu'on n'efface pas reste pose : la traversee le relit a
    // chaque tour et rebranche indefiniment le meme peripherique.
    assert_eq!(CHANGEMENTS.len(), 5);
    for (index, (bit, fonctionnalite)) in CHANGEMENTS.iter().enumerate() {
        assert_eq!(*bit, 1 << index);
        assert_eq!(*fonctionnalite, 16 + index as u16);
    }
}

// ---------------------------------------------------------------------------
// Les paquets SETUP.
// ---------------------------------------------------------------------------

#[test]
fn le_paquet_setup_range_chaque_champ_dans_l_ordre_du_fil() {
    let p = paquet_setup(0x23, 3, 8, 2, 0);
    assert_eq!(p & 0xff, 0x23);
    assert_eq!((p >> 8) & 0xff, 3);
    assert_eq!((p >> 16) & 0xffff, 8);
    assert_eq!((p >> 32) & 0xffff, 2);
    assert_eq!((p >> 48) & 0xffff, 0);
}

#[test]
fn une_requete_de_port_est_adressee_au_port_et_non_au_concentrateur() {
    // Destinataire « autre » (3), pas « peripherique » (0). Un
    // SET_FEATURE(RESET) adresse au peripherique reinitialiserait le
    // concentrateur entier, et avec lui tout ce qui est branche dessus.
    assert_eq!(CLASSE_VERS_PORT_ECRITURE & 0x1f, 3);
    assert_eq!(CLASSE_VERS_PORT_LECTURE & 0x1f, 3);
    assert_eq!(CLASSE_VERS_CONCENTRATEUR_LECTURE & 0x1f, 0);
    // Et le sens : bit 7 pose veut dire « du peripherique vers l'hote ».
    assert_eq!(CLASSE_VERS_PORT_ECRITURE & 0x80, 0);
    assert_eq!(CLASSE_VERS_PORT_LECTURE & 0x80, 0x80);
    // Type « classe » (1) pour les trois.
    for genre in [
        CLASSE_VERS_PORT_ECRITURE,
        CLASSE_VERS_PORT_LECTURE,
        CLASSE_VERS_CONCENTRATEUR_LECTURE,
    ] {
        assert_eq!((genre >> 5) & 0x3, 1);
    }
}

#[test]
fn poser_et_effacer_ne_different_que_par_la_requete() {
    let pose = requete_pose_port(PORT_REINITIALISATION, 2);
    let efface = requete_efface_port(PORT_REINITIALISATION, 2);
    assert_eq!((pose >> 8) & 0xff, 3, "SET_FEATURE");
    assert_eq!((efface >> 8) & 0xff, 1, "CLEAR_FEATURE");
    assert_eq!(pose & !(0xffu64 << 8), efface & !(0xffu64 << 8));
}

#[test]
fn l_etat_d_un_port_se_demande_sur_quatre_octets() {
    let r = requete_etat_port(3);
    assert_eq!(r & 0xff, CLASSE_VERS_PORT_LECTURE as u64);
    assert_eq!((r >> 8) & 0xff, 0, "GET_STATUS");
    assert_eq!((r >> 32) & 0xffff, 3, "le port va dans wIndex");
    assert_eq!((r >> 48) & 0xffff, 4);
}

#[test]
fn le_descripteur_demande_depend_du_protocole() {
    let usb2 = requete_descripteur(false, 9);
    let ss = requete_descripteur(true, 12);
    assert_eq!((usb2 >> 24) & 0xff, 0x29);
    assert_eq!((ss >> 24) & 0xff, 0x2a);
    assert_eq!((usb2 >> 48) & 0xffff, 9);
    assert_eq!((ss >> 48) & 0xffff, 12);
}
