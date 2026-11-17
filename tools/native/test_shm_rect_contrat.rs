//! BOUCHAUD_SHM_RECT_V1 : bornes de `bo_shm_fill_rect`.
//!
//! Le compositeur choisit decalage, largeur, lignes et pas ; le noyau ne doit
//! jamais ecrire hors de la region, quels que soient les entiers fournis.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/kernel/native/shm_rect_contrat.rs"]
mod c;

use c::*;

const L: u64 = 1280;
const H: u64 = 720;
const REGION: usize = (L * H * 4) as usize;

#[test]
fn ecran_entier_accepte() {
    let r = valide(0, 0x123456, L, H, L * 4, REGION).unwrap();
    assert_eq!((r.decalage, r.ligne, r.lignes, r.pas, r.couleur), (0, 5120, 720, 5120, 0x123456));
}

#[test]
fn dernier_pixel_accepte_un_de_plus_refuse() {
    let dernier = REGION as u64 - 4;
    assert!(valide(dernier, 1, 1, 1, 4, REGION).is_ok());
    assert_eq!(valide(dernier + 1, 1, 1, 1, 4, REGION), Err(Refus::HorsRegion));
    // Derniere ligne d'un rectangle qui deborde d'un octet.
    assert!(valide(L * 4 * 10, 0, L, H - 10, L * 4, REGION).is_ok());
    assert_eq!(valide(L * 4 * 10 + 4, 0, L, H - 10, L * 4, REGION), Err(Refus::HorsRegion));
}

#[test]
fn vide_refuse() {
    assert_eq!(valide(0, 0, 0, 10, 4, REGION), Err(Refus::Vide));
    assert_eq!(valide(0, 0, 10, 0, 40, REGION), Err(Refus::Vide));
}

#[test]
fn couleur_sur_32_bits() {
    assert!(valide(0, u32::MAX as u64, 1, 1, 4, REGION).is_ok());
    assert_eq!(valide(0, 1u64 << 32, 1, 1, 4, REGION), Err(Refus::Couleur));
}

#[test]
fn pas_trop_court_refuse() {
    assert_eq!(valide(0, 0, 10, 2, 39, REGION), Err(Refus::PasTropCourt));
    // Une seule ligne : le pas doit tout de meme couvrir la ligne.
    assert_eq!(valide(0, 0, 10, 1, 0, REGION), Err(Refus::PasTropCourt));
}

#[test]
fn debordements_arithmetiques_refuses() {
    // pixels * 4 deborde.
    assert_eq!(valide(0, 0, u64::MAX / 2, 1, u64::MAX, REGION), Err(Refus::HorsRegion));
    // (lignes - 1) * pas deborde.
    assert_eq!(valide(0, 0, 1, u64::MAX, u64::MAX, REGION), Err(Refus::HorsRegion));
    // decalage + ... deborde.
    assert_eq!(valide(u64::MAX - 2, 0, 1, 1, 4, REGION), Err(Refus::HorsRegion));
    // Produit qui revient « petit » modulo 2^64 : refuse quand meme.
    assert_eq!(valide(0, 0, 1, (1u64 << 62) + 1, 4, REGION), Err(Refus::HorsRegion));
}

#[test]
fn applique_ne_touche_que_le_rectangle() {
    // Region 8x4 pixels ; rectangle 3x2 en (2,1).
    let (rl, rh) = (8usize, 4usize);
    let mut o = vec![0xAAu8; rl * rh * 4];
    let r = valide(((1 * rl + 2) * 4) as u64, 0x11223344, 3, 2, (rl * 4) as u64, o.len()).unwrap();
    applique(&r, &mut o);
    for y in 0..rh {
        for x in 0..rl {
            let p = &o[(y * rl + x) * 4..(y * rl + x) * 4 + 4];
            let dedans = (2..5).contains(&x) && (1..3).contains(&y);
            if dedans {
                assert_eq!(p, &0x11223344u32.to_le_bytes(), "({x},{y})");
            } else {
                assert_eq!(p, &[0xAA; 4], "({x},{y}) touche hors du rectangle");
            }
        }
    }
}
