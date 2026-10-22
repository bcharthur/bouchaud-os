//! La memoire du scanout n'est jamais ecrite pendant qu'un transfert peut la
//! lire, et chaque copie d'une trame est comptee (BOUCHAUD_VIRTIO_GPU_TAMPONS_V1).

#[path = "../../src/drivers/display/virtio_gpu_tampons.rs"]
mod tampons;

use tampons::{a_rejouer, acheve, mode, octets_copies, peut_ecrire, soumet, Etat, Fin, Mode, Rect};

const ECRAN: Rect = Rect { x: 0, y: 0, largeur: 1280, hauteur: 720 };

fn r(x: u32, y: u32, largeur: u32, hauteur: u32) -> Rect {
    Rect { x, y, largeur, hauteur }
}

#[test]
fn aucune_ecriture_pendant_un_transfert() {
    let e = soumet(Etat::Composition).unwrap();
    assert_eq!(e, Etat::Transfert);
    assert!(!peut_ecrire(e));
    assert_eq!(soumet(e), None, "une seconde soumission en vol est refusee");
    let e = acheve(e, Fin::Achevee);
    assert!(peut_ecrire(e));
}

#[test]
fn un_transfert_sans_reponse_rend_le_tampon_inconnu_pour_toujours() {
    let e = acheve(soumet(Etat::Composition).unwrap(), Fin::SansReponse);
    assert_eq!(e, Etat::Inconnu);
    assert!(!peut_ecrire(e));
    assert_eq!(soumet(e), None);
    // Une reponse tardive ne le rend pas : on ne sait toujours pas si l'hote
    // a fini de lire.
    assert_eq!(acheve(e, Fin::Achevee), Etat::Inconnu);
    assert_eq!(mode(true, true, e), Mode::Copie);
}

#[test]
fn une_fin_sans_transfert_ne_change_rien() {
    assert_eq!(acheve(Etat::Composition, Fin::SansReponse), Etat::Composition);
    assert_eq!(acheve(Etat::Composition, Fin::Achevee), Etat::Composition);
}

#[test]
fn le_mode_direct_doit_etre_voulu_et_possible() {
    assert_eq!(mode(false, true, Etat::Composition), Mode::Copie, "politique par defaut");
    assert_eq!(mode(true, false, Etat::Composition), Mode::Copie, "memoire DMA trop petite");
    assert_eq!(mode(true, true, Etat::Composition), Mode::Direct);
    assert_eq!(mode(true, true, Etat::Transfert), Mode::Direct, "le mode ne donne pas le droit d'ecrire");
    assert!(!peut_ecrire(Etat::Transfert));
}

#[test]
fn union_des_rectangles() {
    assert_eq!(r(10, 10, 10, 10).union(r(30, 5, 5, 5)), r(10, 5, 25, 15));
    assert_eq!(Rect::vide().union(r(1, 2, 3, 4)), r(1, 2, 3, 4));
    assert_eq!(r(1, 2, 3, 4).union(Rect::vide()), r(1, 2, 3, 4));
}

#[test]
fn rejeu_selon_l_age_du_tampon() {
    let historique = [r(0, 0, 1280, 30), r(100, 100, 50, 50), r(0, 690, 1280, 30)];
    assert_eq!(a_rejouer(0, &historique, ECRAN), ECRAN, "jamais presente : tout l'ecran");
    assert_eq!(a_rejouer(1, &historique, ECRAN), Rect::vide(), "il porte la derniere trame");
    assert_eq!(a_rejouer(2, &historique, ECRAN), r(0, 0, 1280, 30), "double tampon : la trame manquee");
    assert_eq!(a_rejouer(3, &historique, ECRAN), r(0, 0, 1280, 150));
    assert_eq!(a_rejouer(5, &historique, ECRAN), ECRAN, "historique trop court : tout l'ecran");
}

#[test]
fn les_copies_d_une_trame_sont_comptees() {
    let barre = r(0, 0, 1280, 30);
    // Livre : le dommage passe par une copie.
    assert_eq!(octets_copies(Mode::Copie, barre, Rect::vide()), 1280 * 30 * 4);
    // Direct a un tampon : aucune copie.
    assert_eq!(octets_copies(Mode::Direct, barre, Rect::vide()), 0);
    // Direct a deux tampons : le rejeu de la trame manquee est une copie.
    assert_eq!(octets_copies(Mode::Direct, barre, barre), 1280 * 30 * 4);
    // Plein ecran : 3,7 Mo par trame en copie.
    assert_eq!(octets_copies(Mode::Copie, ECRAN, Rect::vide()), 3_686_400);
}

/// Rejoue une suite de trames : le bureau ecrit, presente, et un ecrivain
/// concurrent essaie d'ecrire pendant chaque transfert. Aucune ecriture ne
/// passe pendant un transfert, quelle que soit la fin.
#[test]
fn aucun_ecrivain_ne_passe_pendant_un_transfert_quelle_que_soit_la_fin() {
    let fins = [Fin::Achevee, Fin::Achevee, Fin::SansReponse, Fin::Achevee];
    let mut e = Etat::Composition;
    let mut ecritures_pendant_transfert = 0;
    let mut trames = 0;
    for fin in fins {
        if !peut_ecrire(e) {
            break;
        }
        let Some(en_vol) = soumet(e) else { break };
        if peut_ecrire(en_vol) {
            ecritures_pendant_transfert += 1;
        }
        e = acheve(en_vol, fin);
        trames += 1;
    }
    assert_eq!(ecritures_pendant_transfert, 0);
    assert_eq!(trames, 3, "la trame sans reponse arrete la composition directe");
    assert_eq!(e, Etat::Inconnu);
}
