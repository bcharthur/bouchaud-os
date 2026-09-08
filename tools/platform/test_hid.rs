//! Preuve hote du decodage HID.
//!
//! La table des touches fait cent lignes de correspondances. Une seule entree
//! fausse ne fait rien planter : elle fait qu'UNE touche ne marche pas, sur
//! une machine physique, ou l'on ne peut la decouvrir qu'en appuyant dessus.
//! Il n'y a pas de pire endroit pour une faute -- invisible a la relecture,
//! invisible a la compilation, et elle se manifeste chez l'utilisateur.
//!
//! Les valeurs attendues ci-dessous ne sont pas recopiees du module : elles
//! viennent du jeu de codes PS/2 numero un, ecrit ici lettre par lettre. Deux
//! transcriptions independantes de la meme table, c'est la seule facon de
//! rendre une faute de frappe visible.

#![allow(dead_code)]

#[path = "../../src/drivers/usb/hid/decodage.rs"]
mod hid;

use hid::*;

fn vide() -> [Evenement; EVENEMENTS_MAX] {
    [Evenement { code: 0, etendu: false, appui: false }; EVENEMENTS_MAX]
}

/// Un rapport de clavier d'amorcage.
fn rapport(modificateurs: u8, touches: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; 8];
    r[0] = modificateurs;
    for (i, t) in touches.iter().take(6).enumerate() {
        r[2 + i] = *t;
    }
    r
}

// ---------------------------------------------------------------------------
// La table, transcrite depuis le jeu PS/2 numero un
// ---------------------------------------------------------------------------

#[test]
fn les_vingt_six_lettres_sont_justes() {
    // HID 0x04..0x1D = a..z. Jeu PS/2 1, dans l'ordre alphabetique.
    let ps2 = [
        0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26,
        0x32, 0x31, 0x18, 0x19, 0x10, 0x13, 0x1F, 0x14, 0x16, 0x2F, 0x11, 0x2D,
        0x15, 0x2C,
    ];
    for (i, attendu) in ps2.iter().enumerate() {
        let usage = 0x04 + i as u8;
        assert_eq!(
            usage_ps2(usage),
            Some((*attendu, false)),
            "la lettre « {} » (usage {usage:#04x}) est mal traduite",
            (b'a' + i as u8) as char
        );
    }
}

#[test]
fn les_dix_chiffres_de_la_rangee_du_haut_sont_justes() {
    // HID 0x1E..0x27 = 1..9 puis 0. PS/2 1 : 0x02..0x0B.
    for i in 0..10u8 {
        assert_eq!(usage_ps2(0x1E + i), Some((0x02 + i, false)));
    }
}

#[test]
fn les_touches_d_edition_sont_justes() {
    for (usage, code) in [
        (0x28u8, 0x1Cu8), // Entree
        (0x29, 0x01),     // Echap
        (0x2A, 0x0E),     // Retour arriere
        (0x2B, 0x0F),     // Tabulation
        (0x2C, 0x39),     // Espace
        (0x2D, 0x0C),     // -
        (0x2E, 0x0D),     // =
        (0x2F, 0x1A),     // [
        (0x30, 0x1B),     // ]
        (0x33, 0x27),     // ;
        (0x34, 0x28),     // '
        (0x35, 0x29),     // `
        (0x36, 0x33),     // ,
        (0x37, 0x34),     // .
        (0x38, 0x35),     // /
        (0x39, 0x3A),     // Verrouillage majuscules
        (0x47, 0x46),     // Arret defilement
    ] {
        assert_eq!(usage_ps2(usage), Some((code, false)), "usage {usage:#04x}");
    }
}

#[test]
fn les_douze_touches_de_fonction_sont_justes() {
    // F1..F10 : 0x3B..0x44. F11 et F12 sont AILLEURS dans le jeu PS/2 : 0x57
    // et 0x58. Les supposer contigus est la faute classique.
    for i in 0..10u8 {
        assert_eq!(usage_ps2(0x3A + i), Some((0x3B + i, false)), "F{}", i + 1);
    }
    assert_eq!(usage_ps2(0x44), Some((0x57, false)), "F11");
    assert_eq!(usage_ps2(0x45), Some((0x58, false)), "F12");
}

#[test]
fn le_pave_de_navigation_est_etendu() {
    // Ces touches partagent leur code avec le pave numerique. C'est le prefixe
    // 0xE0 qui les distingue : sans lui, la fleche « haut » tape un 8.
    for (usage, code) in [
        (0x49u8, 0x52u8), // Inser
        (0x4A, 0x47),     // Origine
        (0x4B, 0x49),     // Page precedente
        (0x4C, 0x53),     // Suppr
        (0x4D, 0x4F),     // Fin
        (0x4E, 0x51),     // Page suivante
        (0x4F, 0x4D),     // Droite
        (0x50, 0x4B),     // Gauche
        (0x51, 0x50),     // Bas
        (0x52, 0x48),     // Haut
    ] {
        assert_eq!(
            usage_ps2(usage),
            Some((code, true)),
            "usage {usage:#04x} : sans le prefixe etendu, la fleche tape un \
             chiffre du pave numerique"
        );
    }
}

#[test]
fn le_pave_numerique_n_est_pas_etendu() {
    for (usage, code) in [
        (0x53u8, 0x45u8), // Verr num
        (0x55, 0x37),     // *
        (0x56, 0x4A),     // -
        (0x57, 0x4E),     // +
        (0x59, 0x4F),     // 1
        (0x5A, 0x50),     // 2
        (0x5B, 0x51),     // 3
        (0x5C, 0x4B),     // 4
        (0x5D, 0x4C),     // 5
        (0x5E, 0x4D),     // 6
        (0x5F, 0x47),     // 7
        (0x60, 0x48),     // 8
        (0x61, 0x49),     // 9
        (0x62, 0x52),     // 0
        (0x63, 0x53),     // .
    ] {
        assert_eq!(usage_ps2(usage), Some((code, false)), "usage {usage:#04x}");
    }
    // Les deux exceptions du pave : la division et l'entree portent le prefixe.
    assert_eq!(usage_ps2(0x54), Some((0x35, true)), "division du pave");
    assert_eq!(usage_ps2(0x58), Some((0x1C, true)), "entree du pave");
}

#[test]
fn la_touche_a_gauche_du_w_d_un_clavier_francais_existe() {
    // Usage 0x64, absente d'un clavier americain et presente sur tous les
    // claviers europeens. Sans elle, la touche « inferieur / superieur » ne
    // fait rien -- un manque qu'on ne decouvre qu'en appuyant dessus, sur la
    // machine, apres l'avoir livree.
    assert_eq!(usage_ps2(0x64), Some((0x56, false)));
}

#[test]
fn la_touche_menu_existe() {
    assert_eq!(usage_ps2(0x65), Some((0x5D, true)));
}

#[test]
fn les_huit_modificateurs_sont_justes() {
    for (bit, code, etendu) in [
        (0u8, 0x1Du8, false), // Ctrl gauche
        (1, 0x2A, false),     // Maj gauche
        (2, 0x38, false),     // Alt gauche
        (3, 0x5B, true),      // Logo gauche
        (4, 0x1D, true),      // Ctrl droit
        (5, 0x36, false),     // Maj droite
        (6, 0x38, true),      // AltGr
        (7, 0x5C, true),      // Logo droit
    ] {
        assert_eq!(modificateur_ps2(bit), Some((code, etendu)), "bit {bit}");
    }
    assert_eq!(modificateur_ps2(8), None);
}

#[test]
fn altgr_se_distingue_de_alt_gauche() {
    // Les deux portent 0x38. Seul le prefixe les separe -- et sur un clavier
    // francais, AltGr sert a taper @, #, [, ], {, } et |.
    assert_eq!(modificateur_ps2(2), Some((0x38, false)));
    assert_eq!(modificateur_ps2(6), Some((0x38, true)));
}

#[test]
fn un_usage_inconnu_ne_produit_rien() {
    // Mieux vaut aucune touche qu'une touche au hasard : un usage inconnu qui
    // retomberait sur un code par defaut ferait taper un caractere que
    // l'utilisateur n'a pas demande.
    assert_eq!(usage_ps2(0x00), None);
    assert_eq!(usage_ps2(0x01), None, "ErrorRollOver n'est pas une touche");
    assert_eq!(usage_ps2(0xFF), None);
    // 0x46 (impression ecran) et 0x48 (pause) demandent des SEQUENCES de
    // codes, pas un code : les traduire par un seul serait faux.
    assert_eq!(usage_ps2(0x46), None);
    assert_eq!(usage_ps2(0x48), None);
}

// ---------------------------------------------------------------------------
// La difference entre deux rapports
// ---------------------------------------------------------------------------

#[test]
fn un_rapport_dit_ce_qui_est_enfonce_pas_ce_qui_vient_d_etre_appuye() {
    let mut etat = EtatClavier::default();
    let mut sortie = vide();
    // « a » enfoncee.
    let n = evenements_clavier(&mut etat, 0, &rapport(0, &[0x04]), &mut sortie).unwrap();
    assert_eq!(n, 1);
    assert_eq!(sortie[0], Evenement { code: 0x1E, etendu: false, appui: true });
    // Le MEME rapport : la touche est toujours enfoncee, rien n'a change.
    let n = evenements_clavier(&mut etat, 0, &rapport(0, &[0x04]), &mut sortie).unwrap();
    assert_eq!(
        n, 0,
        "un rapport identique n'est pas une repetition ; le traiter comme un \
         nouvel appui ferait doubler chaque caractere"
    );
    // Rapport vide : relachement.
    let n = evenements_clavier(&mut etat, 0, &rapport(0, &[]), &mut sortie).unwrap();
    assert_eq!(n, 1);
    assert_eq!(sortie[0], Evenement { code: 0x1E, etendu: false, appui: false });
}

#[test]
fn les_relachements_sortent_avant_les_appuis() {
    let mut etat = EtatClavier::default();
    let mut sortie = vide();
    evenements_clavier(&mut etat, 0, &rapport(0, &[0x04]), &mut sortie).unwrap();
    // « a » remplacee par « b » dans le meme rapport : ce qui arrive des
    // qu'on tape vite.
    let n = evenements_clavier(&mut etat, 0, &rapport(0, &[0x05]), &mut sortie).unwrap();
    assert_eq!(n, 2);
    assert_eq!(
        sortie[0],
        Evenement { code: 0x1E, etendu: false, appui: false },
        "emettre l'appui d'abord donnerait deux touches simultanement \
         enfoncees pour l'application, puis un relachement apres coup"
    );
    assert_eq!(sortie[1], Evenement { code: 0x30, etendu: false, appui: true });
}

#[test]
fn une_touche_qui_disparait_est_toujours_relachee() {
    // Sans cela, un modificateur reste « enfonce » et transforme tout ce qui
    // suit : l'utilisateur tape en majuscules sans avoir touche a Maj.
    let mut etat = EtatClavier::default();
    let mut sortie = vide();
    // Maj gauche seule, sans aucune touche.
    let n = evenements_clavier(&mut etat, 0, &rapport(0x02, &[]), &mut sortie).unwrap();
    assert_eq!(n, 1);
    assert_eq!(sortie[0], Evenement { code: 0x2A, etendu: false, appui: true });
    let n = evenements_clavier(&mut etat, 0, &rapport(0x00, &[]), &mut sortie).unwrap();
    assert_eq!(n, 1);
    assert_eq!(sortie[0], Evenement { code: 0x2A, etendu: false, appui: false });
}

#[test]
fn six_touches_simultanees_tiennent_dans_la_sortie() {
    let mut etat = EtatClavier::default();
    let mut sortie = vide();
    // Le pire cas : huit modificateurs changent, six touches se relachent et
    // six autres s'appuient. Vingt evenements.
    evenements_clavier(&mut etat, 0, &rapport(0, &[4, 5, 6, 7, 8, 9]), &mut sortie).unwrap();
    let n = evenements_clavier(
        &mut etat,
        0,
        &rapport(0xFF, &[0x10, 0x11, 0x12, 0x13, 0x14, 0x15]),
        &mut sortie,
    )
    .unwrap();
    assert_eq!(n, 20);
    assert!(n <= EVENEMENTS_MAX, "la sortie deborderait");
}

#[test]
fn un_rapport_trop_court_est_refuse() {
    let mut etat = EtatClavier::default();
    let mut sortie = vide();
    assert_eq!(evenements_clavier(&mut etat, 0, &[0u8; 7], &mut sortie), None);
    assert_eq!(evenements_clavier(&mut etat, 0, &[], &mut sortie), None);
    assert_eq!(
        etat,
        EtatClavier::default(),
        "un rapport refuse ne doit pas avoir modifie l'etat"
    );
}

#[test]
fn un_code_de_touche_inconnu_n_empeche_pas_les_autres() {
    let mut etat = EtatClavier::default();
    let mut sortie = vide();
    // 0xFF n'est pas une touche ; « a » l'est.
    let n = evenements_clavier(&mut etat, 0, &rapport(0, &[0xFF, 0x04]), &mut sortie).unwrap();
    assert_eq!(n, 1);
    assert_eq!(sortie[0].code, 0x1E);
}

// ---------------------------------------------------------------------------
// L'identifiant de rapport
// ---------------------------------------------------------------------------

#[test]
fn un_identifiant_de_rapport_decale_tout_d_un_octet() {
    // Sans le retirer, les modificateurs deviennent le premier code de
    // touche, et le clavier tape n'importe quoi.
    let mut avec = vec![0x07u8];
    avec.extend_from_slice(&rapport(0x02, &[0x04]));
    let mut etat = EtatClavier::default();
    let mut sortie = vide();
    let n = evenements_clavier(&mut etat, 0x07, &avec, &mut sortie).unwrap();
    assert_eq!(n, 2);
    assert_eq!(sortie[0], Evenement { code: 0x2A, etendu: false, appui: true });
    assert_eq!(sortie[1], Evenement { code: 0x1E, etendu: false, appui: true });
}

#[test]
fn un_rapport_destine_a_un_autre_identifiant_est_ignore() {
    // Il appartient a un autre rapport du meme peripherique. Le decoder
    // ferait taper des touches au hasard des qu'une souris composite envoie
    // ses propres rapports.
    let mut autre = vec![0x09u8];
    autre.extend_from_slice(&rapport(0, &[0x04]));
    let mut etat = EtatClavier::default();
    let mut sortie = vide();
    assert_eq!(evenements_clavier(&mut etat, 0x07, &autre, &mut sortie), None);
    assert_eq!(etat, EtatClavier::default());
}

#[test]
fn sans_identifiant_le_premier_octet_est_deja_la_donnee() {
    assert_eq!(charge_utile(0, &[1, 2, 3]), Some(&[1u8, 2, 3][..]));
    assert_eq!(charge_utile(1, &[1, 2, 3]), Some(&[2u8, 3][..]));
    assert_eq!(charge_utile(1, &[2, 3]), None);
    assert_eq!(charge_utile(1, &[]), None);
}

// ---------------------------------------------------------------------------
// La souris
// ---------------------------------------------------------------------------

#[test]
fn une_souris_a_trois_octets_n_a_pas_de_molette() {
    // Elle n'a pas une molette immobile : elle n'en a pas. Lire un quatrieme
    // octet qui n'existe pas ferait defiler au hasard.
    assert_eq!(
        decode_souris(0, &[0x01, 5, 0xFB]),
        Some(Souris { boutons: 1, dx: 5, dy: -5, roue: 0 })
    );
}

#[test]
fn les_deplacements_sont_signes() {
    // 0xFF est -1, pas 255. Non signe, le pointeur ne partirait que dans un
    // sens, et vite.
    let s = decode_souris(0, &[0, 0xFF, 0xFF, 0xFF]).unwrap();
    assert_eq!((s.dx, s.dy, s.roue), (-1, -1, -1));
    let s = decode_souris(0, &[0, 0x7F, 0x80, 0]).unwrap();
    assert_eq!((s.dx, s.dy), (127, -128));
}

#[test]
fn seuls_les_trois_premiers_boutons_sont_retenus() {
    // Les bits hauts portent les boutons lateraux, que la pile d'entree ne
    // sait pas encore distribuer. Les laisser passer les ferait prendre pour
    // un clic du milieu.
    assert_eq!(decode_souris(0, &[0xFF, 0, 0]).unwrap().boutons, 0x07);
    assert_eq!(decode_souris(0, &[0x04, 0, 0]).unwrap().boutons, 0x04);
}

#[test]
fn un_rapport_de_souris_trop_court_est_refuse() {
    assert_eq!(decode_souris(0, &[0, 0]), None);
    assert_eq!(decode_souris(0, &[]), None);
}

#[test]
fn une_souris_a_identifiant_de_rapport_est_decalee_aussi() {
    assert_eq!(
        decode_souris(3, &[3, 0x02, 10, 0xF6, 1]),
        Some(Souris { boutons: 2, dx: 10, dy: -10, roue: 1 })
    );
    assert_eq!(decode_souris(3, &[4, 0x02, 10, 0xF6]), None);
}

// ---------------------------------------------------------------------------
// Le genre d'une interface
// ---------------------------------------------------------------------------

#[test]
fn seule_la_sous_classe_d_amorcage_designe_un_role() {
    assert_eq!(genre_interface(1, 1), 1, "clavier d'amorcage");
    assert_eq!(genre_interface(1, 2), 2, "souris d'amorcage");
    assert_eq!(
        genre_interface(0, 1),
        0,
        "une interface de sous-classe zero declare son role dans son \
         descripteur de rapport ; deviner « clavier » ferait injecter des \
         touches depuis un volant de course"
    );
    assert_eq!(genre_interface(1, 0), 0);
    assert_eq!(genre_interface(0, 0), 0);
}
