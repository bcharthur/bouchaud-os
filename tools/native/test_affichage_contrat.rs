//! BOUCHAUD_COMPOSITORD_PRESENTATION_V1 : bornes de `bo_display_present`.

#[path = "../../src/kernel/native/affichage_contrat.rs"]
mod c;

use c::*;

fn d(largeur: u32, hauteur: u32, x: u32, y: u32, l: u32, h: u32, dx: u32, dy: u32) -> Demande {
    Demande { largeur, hauteur, x, y, l, h, dest_x: dx, dest_y: dy }
}

const EL: u32 = 1280;
const EH: u32 = 720;

#[test]
fn une_demande_exacte_passe() {
    let a = d(320, 200, 0, 0, 320, 200, 960, 520);
    assert_eq!(valide(&a, 320 * 200 * 4, EL, EH), Ok(()));
}

#[test]
fn vide_refuse() {
    for a in [d(0, 1, 0, 0, 1, 1, 0, 0), d(1, 0, 0, 0, 1, 1, 0, 0), d(4, 4, 0, 0, 0, 1, 0, 0), d(4, 4, 0, 0, 1, 0, 0, 0)] {
        assert_eq!(valide(&a, 1 << 20, EL, EH), Err(Refus::Vide));
    }
}

#[test]
fn surface_plus_grande_que_la_region_refusee() {
    let a = d(320, 200, 0, 0, 1, 1, 0, 0);
    assert_eq!(valide(&a, 320 * 200 * 4 - 1, EL, EH), Err(Refus::SurfaceHorsRegion));
}

#[test]
fn rectangle_hors_surface_refuse_au_pixel_pres() {
    assert_eq!(valide(&d(320, 200, 1, 0, 320, 1, 0, 0), 320 * 200 * 4, EL, EH), Err(Refus::HorsSurface));
    assert_eq!(valide(&d(320, 200, 0, 1, 1, 200, 0, 0), 320 * 200 * 4, EL, EH), Err(Refus::HorsSurface));
    assert_eq!(valide(&d(320, 200, 319, 199, 1, 1, 0, 0), 320 * 200 * 4, EL, EH), Ok(()));
}

#[test]
fn destination_hors_ecran_refusee_au_pixel_pres() {
    assert_eq!(valide(&d(8, 8, 0, 0, 8, 8, EL - 7, 0), 256, EL, EH), Err(Refus::HorsEcran));
    assert_eq!(valide(&d(8, 8, 0, 0, 8, 8, 0, EH - 7), 256, EL, EH), Err(Refus::HorsEcran));
    assert_eq!(valide(&d(8, 8, 0, 0, 8, 8, EL - 8, EH - 8), 256, EL, EH), Ok(()));
}

#[test]
fn aucun_debordement_arithmetique_ne_fait_passer_une_demande() {
    let m = u32::MAX;
    // x + l deborderait en u32 et retomberait dans la surface.
    assert_eq!(valide(&d(16, 16, m, 0, 2, 1, 0, 0), 1024, EL, EH), Err(Refus::HorsSurface));
    assert_eq!(valide(&d(16, 16, 0, m, 1, 2, 0, 0), 1024, EL, EH), Err(Refus::HorsSurface));
    // dest + l deborderait en u32.
    assert_eq!(valide(&d(16, 16, 0, 0, 2, 1, m, 0), 1024, EL, EH), Err(Refus::HorsEcran));
    // largeur x hauteur x 4 deborderait en u32 : refuse par la borne.
    assert_eq!(valide(&d(m, m, 0, 0, 1, 1, 0, 0), usize::MAX, EL, EH), Err(Refus::TropGrand));
    assert_eq!(valide(&d(4097, 4096, 0, 0, 1, 1, 0, 0), usize::MAX, EL, EH), Err(Refus::TropGrand));
    assert_eq!(valide(&d(4096, 4096, 0, 0, 1, 1, 0, 0), 4096 * 4096 * 4, EL, EH), Ok(()));
}

#[test]
fn decodage_du_fil_et_decalage_des_lignes() {
    let mut o = [0u8; DEMANDE_OCTETS];
    for (i, v) in [320u32, 200, 10, 20, 30, 40, 50, 60].iter().enumerate() {
        o[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    let a = Demande::depuis_octets(&o);
    assert_eq!(a, d(320, 200, 10, 20, 30, 40, 50, 60));
    assert_eq!(a.decalage_ligne(0), (20 * 320 + 10) * 4);
    assert_eq!(a.decalage_ligne(3), (23 * 320 + 10) * 4);
}

#[test]
fn empreinte_sensible_au_pixel_et_positive() {
    let a: Vec<u32> = (0..1000).collect();
    let mut b = a.clone();
    b[500] ^= 1;
    assert_ne!(empreinte(a.iter().copied()), empreinte(b.iter().copied()));
    // L'octet alpha ne compte pas (l'ecran ne le garde pas).
    let mut c = a.clone();
    c[10] |= 0xff00_0000;
    assert_eq!(empreinte(a.iter().copied()), empreinte(c.iter().copied()));
    assert!(empreinte(a.iter().copied()) <= i64::MAX as u64);
    // Valeur de reference partagee avec le service C (composited-slice.c).
    assert_eq!(empreinte([0x0012_3456u32]), empreinte_c(&[0x0012_3456]));
}

fn empreinte_c(px: &[u32]) -> u64 {
    // Transcription ligne a ligne de `empreinte_fnv` dans composited-slice.c.
    let mut h: u64 = 0xcbf29ce484222325;
    for &p in px {
        let v = p & 0xffffff;
        for k in 0..4 {
            h ^= ((v >> (8 * k)) & 0xff) as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h & 0x3fffffffffffffff
}
