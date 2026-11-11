//! BOUCHAUD_BUREAU_RING3_2A : bornes de `bo_fenetre_lit`.
//!
//! Le compositeur ring 3 choisit le rectangle, le decalage et le pas ; le
//! noyau ne doit jamais lire hors de ce que le client a peint, ni ecrire hors
//! de la region du compositeur, quels que soient les entiers fournis.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/kernel/native/fenetre_contrat.rs"]
mod c;

use c::*;

fn l(x: u32, y: u32, w: u32, h: u32, decalage: u32, pas: u32) -> Lecture {
    Lecture { x, y, l: w, h, decalage, pas }
}

const SURF: (u32, u32) = (640, 400);
const ECRAN: usize = 1280 * 800 * 4;

#[test]
fn surface_entiere_vers_l_ecran_passe() {
    let pas = 1280 * 4;
    let d = l(0, 0, 640, 400, (144 * 1280 + 200) * 4, pas);
    assert_eq!(valide(&d, SURF, ECRAN), Ok(()));
    assert_eq!(d.destination_ligne(1), ((145 * 1280 + 200) * 4) as usize);
}

#[test]
fn rectangle_vide_refuse() {
    assert_eq!(valide(&l(0, 0, 0, 1, 0, 4), SURF, ECRAN), Err(Refus::Vide));
    assert_eq!(valide(&l(0, 0, 1, 0, 0, 4), SURF, ECRAN), Err(Refus::Vide));
}

#[test]
fn hors_de_ce_que_le_client_a_peint_refuse() {
    assert_eq!(valide(&l(1, 0, 640, 1, 0, 4096), SURF, ECRAN), Err(Refus::HorsSurface));
    assert_eq!(valide(&l(0, 400, 1, 1, 0, 4096), SURF, ECRAN), Err(Refus::HorsSurface));
    // Une surface encore vide (rien de peint) : tout est hors surface.
    assert_eq!(valide(&l(0, 0, 1, 1, 0, 4), (0, 0), ECRAN), Err(Refus::HorsSurface));
}

#[test]
fn debordement_u32_des_coordonnees_refuse() {
    assert_eq!(valide(&l(u32::MAX, 0, 2, 1, 0, 8), SURF, ECRAN), Err(Refus::HorsSurface));
    assert_eq!(valide(&l(0, u32::MAX, 1, 2, 0, 4), SURF, ECRAN), Err(Refus::HorsSurface));
    assert_eq!(valide(&l(0, 0, u32::MAX, 1, 0, u32::MAX), SURF, ECRAN), Err(Refus::HorsSurface));
}

#[test]
fn pas_plus_court_qu_une_ligne_refuse() {
    assert_eq!(valide(&l(0, 0, 640, 2, 0, 640 * 4 - 1), SURF, ECRAN), Err(Refus::PasTropCourt));
    assert_eq!(valide(&l(0, 0, 640, 2, 0, 0), SURF, ECRAN), Err(Refus::PasTropCourt));
}

#[test]
fn destination_hors_region_refusee_au_dernier_octet_pres() {
    let pas = 1280 * 4;
    // Le rectangle tient exactement dans la region : derniere ligne au bout.
    let exact = l(0, 0, 640, 400, (ECRAN as u32) - 399 * pas - 640 * 4, pas);
    assert_eq!(valide(&exact, SURF, ECRAN), Ok(()));
    let un_de_trop = Lecture { decalage: exact.decalage + 1, ..exact };
    assert_eq!(valide(&un_de_trop, SURF, ECRAN), Err(Refus::HorsRegion));
}

#[test]
fn debordement_de_la_destination_ne_passe_jamais() {
    // decalage et pas maximaux : la somme deborderait un u32, pas un u64.
    assert_eq!(valide(&l(0, 0, 1, 400, u32::MAX, u32::MAX), SURF, ECRAN), Err(Refus::HorsRegion));
    assert_eq!(valide(&l(0, 0, 1, 1, u32::MAX - 2, 4), SURF, ECRAN), Err(Refus::HorsRegion));
    // Region vide : rien ne s'y ecrit.
    assert_eq!(valide(&l(0, 0, 1, 1, 0, 4), SURF, 0), Err(Refus::HorsRegion));
}

#[test]
fn surface_annoncee_geante_bornee() {
    // Une geometrie hors norme est rognee a COTE_MAX avant comparaison.
    assert_eq!(valide(&l(4096, 0, 1, 1, 0, 4), (u32::MAX, u32::MAX), ECRAN), Err(Refus::HorsSurface));
}

#[test]
fn decodage_du_fil() {
    let mut o = [0u8; LECTURE_OCTETS];
    for (i, v) in [1u32, 2, 3, 4, 5, 6].iter().enumerate() {
        o[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    assert_eq!(Lecture::depuis_octets(&o), l(1, 2, 3, 4, 5, 6));
}
