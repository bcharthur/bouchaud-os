//! Le volume USB et le support qui le porte (BOUCHAUD_USB_LIAISON_SUPPORT_V1),
//! et le traitement d'un refus de vidange (BOUCHAUD_USB_REFUS_SCSI_V1).
//!
//! Le module de production `src/drivers/usb/stockage/liaison.rs` est inclus
//! tel quel. Chaque test nomme l'invariant qu'il etablit ; les numeros sont
//! ceux de la liste de tests de la PR #246.

#[path = "../../src/drivers/usb/stockage/liaison.rs"]
mod liaison;

use liaison::*;

fn pret(l: Liaison, numero: Numero, vidange: bool) -> (Liaison, Decision) {
    l.support_pret(numero, vidange)
}

// (1) Un support qui accepte la vidange : le volume la promet.
#[test]
fn un_support_qui_accepte_la_vidange_la_fait_promettre() {
    let (l, d) = pret(Liaison::NEUVE, 1, true);
    assert_eq!(d, Decision::Lie);
    assert_eq!(l.cible(), Some(1));
    assert!(l.promet_la_vidange());
    assert!(l.vidange_valide(1));
}

// (2) Un support qui refuse la vidange : le volume existe, sans promesse.
#[test]
fn un_support_qui_refuse_la_vidange_reste_utilisable_sans_promesse() {
    let (l, d) = pret(Liaison::NEUVE, 1, false);
    assert_eq!(d, Decision::Lie);
    assert_eq!(l.cible(), Some(1), "lectures et ecritures restent dirigees vers lui");
    assert!(!l.promet_la_vidange());
    assert!(!l.vidange_valide(1));
}

// (3) Deux supports aux capacites differentes : le second ne decide de rien.
#[test]
fn une_seconde_cle_qui_refuse_ne_retire_pas_la_promesse_de_la_premiere() {
    let (l, _) = pret(Liaison::NEUVE, 1, true);
    let (l2, d) = pret(l, 2, false);
    assert_eq!(d, Decision::DejaLie(1));
    assert_eq!(l2, l, "l'etat du volume est inchange");
    assert!(l2.promet_la_vidange());
    // Un refus attribue au second support ne touche pas le volume non plus.
    assert_eq!(l2.vidange_refusee(2), l2);
}

#[test]
fn une_seconde_cle_qui_accepte_n_accorde_pas_de_promesse_a_la_premiere() {
    // Le cas le plus dangereux de l'ancien code : une promesse ACCORDEE a tort.
    let (l, _) = pret(Liaison::NEUVE, 1, false);
    let (l2, d) = pret(l, 2, true);
    assert_eq!(d, Decision::DejaLie(1));
    assert!(!l2.promet_la_vidange());
    assert!(!l2.vidange_valide(2), "une vidange reussie sur la seconde ne vaut rien pour le volume");
    assert_eq!(l2.cible(), Some(1));
}

// (4) Refus de vidange en cours d'utilisation.
#[test]
fn un_refus_en_cours_d_utilisation_retire_la_promesse_et_seulement_elle() {
    let (l, _) = pret(Liaison::NEUVE, 7, true);
    let apres = l.vidange_refusee(7);
    assert!(!apres.promet_la_vidange());
    // (5) le volume designe toujours le meme support : lectures et ecritures
    // ordinaires continuent d'y aller.
    assert_eq!(apres.cible(), Some(7));
    assert!(!apres.orphelin);
    // Le refus est definitif pour ce demarrage : aucun chemin ne la rend.
    let (encore, d) = apres.support_pret(7, true);
    assert_eq!(d, Decision::DejaLie(7));
    assert!(!encore.promet_la_vidange());
}

// (6) Deconnexion, puis (7) reconnexion : aucune capacite perimee reutilisee.
#[test]
fn le_support_lie_retire_rend_le_volume_orphelin_sans_promesse() {
    let (l, _) = pret(Liaison::NEUVE, 3, true);
    let o = l.support_retire(3);
    assert!(o.orphelin);
    assert_eq!(o.cible(), None, "plus aucune entree-sortie ne peut designer un support");
    assert!(!o.promet_la_vidange());
    assert!(!o.vidange_valide(3));
}

#[test]
fn une_cle_rebranchee_n_est_pas_reliee_et_n_herite_de_rien() {
    let (l, _) = pret(Liaison::NEUVE, 3, true);
    let o = l.support_retire(3);
    // La meme cle, rebranchee, recoit un NOUVEAU numero : le compteur ne
    // revient jamais en arriere.
    let (r, d) = o.support_pret(4, true);
    assert_eq!(d, Decision::VolumeOrphelin);
    assert_eq!(r, o);
    assert!(!r.promet_la_vidange());
    assert_eq!(r.cible(), None);
    // Meme l'ancien numero, s'il revenait, ne relierait rien.
    let (r2, d2) = o.support_pret(3, true);
    assert_eq!(d2, Decision::VolumeOrphelin);
    assert!(!r2.promet_la_vidange());
}

#[test]
fn retirer_un_autre_support_ne_touche_pas_le_volume() {
    let (l, _) = pret(Liaison::NEUVE, 1, true);
    let (l, _) = pret(l, 2, false);
    assert_eq!(l.support_retire(2), l);
    assert!(l.support_retire(2).promet_la_vidange());
}

#[test]
fn le_numero_zero_ne_designe_jamais_rien() {
    let (l, d) = Liaison::NEUVE.support_pret(0, true);
    assert_ne!(d, Decision::Lie);
    assert_eq!(l.cible(), None);
    let (l, _) = pret(Liaison::NEUVE, 5, true);
    assert_eq!(l.support_retire(0), l);
    assert_eq!(l.vidange_refusee(0), l);
    assert!(!Liaison::NEUVE.vidange_valide(0));
}

// (8) et (9), cote volume : la promesse publiee au descripteur -- celle que
// `kernel/update::trouve_meta` exige avant un commit A/B -- est exactement
// celle du support lie, quel que soit l'ordre des evenements.
#[test]
fn la_promesse_du_volume_ne_vient_que_du_support_lie_quel_que_soit_l_ordre() {
    // Toutes les sequences de 5 evenements sur 3 supports.
    #[derive(Clone, Copy, Debug)]
    enum E { Pret(Numero, bool), Retire(Numero), Refus(Numero) }
    let evenements = [
        E::Pret(1, true), E::Pret(1, false), E::Pret(2, true), E::Pret(2, false),
        E::Retire(1), E::Retire(2), E::Refus(1), E::Refus(2), E::Pret(3, true),
    ];
    let n = evenements.len();
    let mut sequences = 0u64;
    let mut idx = [0usize; 5];
    loop {
        // Modele de reference, ecrit independamment.
        let mut lie: Option<(Numero, bool)> = None;
        let mut orphelin = false;
        let mut l = Liaison::NEUVE;
        for &i in &idx {
            match evenements[i] {
                E::Pret(num, v) => {
                    if !orphelin && lie.is_none() { lie = Some((num, v)); }
                    l = l.support_pret(num, v).0;
                }
                E::Retire(num) => {
                    if !orphelin && lie.map(|x| x.0) == Some(num) { orphelin = true; }
                    l = l.support_retire(num);
                }
                E::Refus(num) => {
                    if let Some((x, _)) = lie { if x == num { lie = Some((x, false)); } }
                    l = l.vidange_refusee(num);
                }
            }
            let attendu = !orphelin && lie.map(|x| x.1).unwrap_or(false);
            assert_eq!(l.promet_la_vidange(), attendu, "sequence {:?}", idx.map(|i| evenements[i]));
            assert_eq!(l.cible(), if orphelin { None } else { lie.map(|x| x.0) });
            assert_eq!(Liaison::depaquete(l.empaquete()), l, "le mot publie est fidele");
        }
        sequences += 1;
        let mut k = 0;
        while k < 5 { idx[k] += 1; if idx[k] < n { break } idx[k] = 0; k += 1; }
        if k == 5 { break }
    }
    assert_eq!(sequences, (n as u64).pow(5));
}

#[test]
fn le_mot_publie_conserve_les_numeros_sur_32_bits() {
    for lie in [0u32, 1, 2, 0xffff, 0xffff_ffff] {
        for v in [false, true] {
            for o in [false, true] {
                let l = Liaison { lie, vidange_reelle: v, orphelin: o };
                assert_eq!(Liaison::depaquete(l.empaquete()), l);
            }
        }
    }
}

// Refus SCSI (BOUCHAUD_USB_REFUS_SCSI_V1).

#[test]
fn un_csw_command_failed_demande_le_sens_sans_reprise() {
    let t = traitement(classe_refus("bot-commande-echouee"));
    assert!(t.demande_sens, "la condition en attente doit etre lue");
    assert!(!t.reprise, "un refus de commande n'est pas un transport casse");
}

#[test]
fn un_transport_qui_faillit_est_repris_puis_le_sens_demande() {
    for motif in ["bot-erreur-de-phase", "bot-csw-echeance", "bot-csw-invalide",
        "bot-stall-non-recupere", "bot-hors-service", "bulk-echeance"] {
        let t = traitement(classe_refus(motif));
        assert!(t.reprise, "{}", motif);
        assert!(t.demande_sens, "{}", motif);
    }
}

#[test]
fn sans_commande_emise_il_n_y_a_rien_a_lire_ni_a_reprendre() {
    for motif in ["vidange-refus-injecte", "absent", "slot", "peripherique", "pilote-occupe"] {
        assert_eq!(traitement(classe_refus(motif)),
            Traitement { reprise: false, demande_sens: false }, "{}", motif);
    }
}

#[test]
fn le_sens_au_format_fixe_est_decode() {
    // ILLEGAL REQUEST, INVALID COMMAND OPERATION CODE (20h/00h).
    let mut s = [0u8; 18];
    s[0] = 0x70; s[2] = 0x05; s[7] = 10; s[12] = 0x20; s[13] = 0x00;
    let d = decode_sens(&s).unwrap();
    assert_eq!(d, Sens { cle: CLE_REQUETE_ILLEGALE, asc: 0x20, ascq: 0 });
    assert_eq!(d.nom_cle(), "requete-illegale");
    // Bit VALID et bits hauts de l'octet de cle ignores.
    s[0] = 0xf0; s[2] = 0xe3; s[12] = 0x0c; s[13] = 0x00;
    assert_eq!(decode_sens(&s).unwrap(), Sens { cle: 3, asc: 0x0c, ascq: 0 });
}

#[test]
fn le_sens_au_format_descripteur_est_decode() {
    let s = [0x72u8, 0x04, 0x44, 0x00, 0, 0, 0, 0];
    assert_eq!(decode_sens(&s).unwrap(), Sens { cle: 4, asc: 0x44, ascq: 0 });
}

#[test]
fn un_sens_tronque_ou_inconnu_n_est_pas_invente() {
    assert_eq!(decode_sens(&[]), None);
    assert_eq!(decode_sens(&[0x70, 0, 5]), None, "format fixe sans ASC");
    assert_eq!(decode_sens(&[0x72, 5]), None);
    assert_eq!(decode_sens(&[0x00; 18]), None, "code de reponse nul");
    assert_eq!(decode_sens(&[0x7f; 18]), None);
}
