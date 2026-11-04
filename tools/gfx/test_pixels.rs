//! Operations de pixels du tampon arriere (BOUCHAUD_GFX_PIXELS_V1).
//!
//! Le fragment de production `src/drivers/display/pixels.rs` est inclus tel
//! quel. Ce qui est etabli ici : le melange par paires de canaux rend, au bit
//! pres, ce que rendait la formule a trois canaux et trois divisions, et le
//! remplissage par mots de 64 bits ecrit exactement les pixels demandes, quel
//! que soit l'alignement.

#![allow(dead_code)]

#[path = "../../src/drivers/display/pixels.rs"]
mod pixels;

use pixels::*;

/// La formule d'origine de `bochs::melange_pixel`, recopiee telle qu'elle
/// etait avant BOUCHAUD_GFX_PIXELS_V1.
fn reference(dst: u32, rgb: u32, alpha: u8) -> u32 {
    let a = alpha as u32;
    let inv = 255 - a;
    let dr = (dst >> 16) & 0xff; let dg = (dst >> 8) & 0xff; let db = dst & 0xff;
    let sr = (rgb >> 16) & 0xff; let sg = (rgb >> 8) & 0xff; let sb = rgb & 0xff;
    let r = (sr * a + dr * inv) / 255;
    let g = (sg * a + dg * inv) / 255;
    let b = (sb * a + db * inv) / 255;
    (r << 16) | (g << 8) | b
}

#[test]
fn la_division_par_255_est_exacte_sur_toute_valeur_de_voie() {
    // 255 * 255 : la plus grande valeur qu'une voie peut prendre.
    for x in 0u32..=65_025 {
        assert_eq!((x + 1 + (x >> 8)) >> 8, x / 255, "x = {}", x);
    }
}

#[test]
fn chaque_canal_toutes_couvertures_toutes_valeurs() {
    // Un canal a la fois, les deux autres a des valeurs extremes : couvre
    // les 254 couvertures x 256 x 256 combinaisons de chaque voie.
    for a in 1u8..=254 {
        for s in 0u32..=255 {
            for d in 0u32..=255 {
                for decalage in [0u32, 8, 16] {
                    let autre = 0x00a5_5aa5 & !(0xff << decalage);
                    let dst = (d << decalage) | autre;
                    let src = (s << decalage) | (0x005a_a55a & !(0xff << decalage));
                    assert_eq!(melange(dst, src, a as u32), reference(dst, src, a),
                        "a={} s={} d={} canal={}", a, s, d, decalage);
                }
            }
        }
    }
}

#[test]
fn l_octet_haut_est_ignore_et_jamais_ecrit() {
    for a in [1u8, 77, 128, 200, 254] {
        let r = melange(0xff12_3456, 0xee65_4321, a as u32);
        assert_eq!(r >> 24, 0);
        assert_eq!(r, reference(0x0012_3456, 0x0065_4321, a));
    }
}

#[test]
fn melange_ligne_egale_melange_pixel_par_pixel() {
    let mut graine = 0x1234_5678u32;
    let mut alea = || { graine ^= graine << 13; graine ^= graine >> 17; graine ^= graine << 5; graine };
    for a in 1u32..=254 {
        let rgb = alea() & 0x00ff_ffff;
        let mut ligne: Vec<u32> = (0..37).map(|_| alea() & 0x00ff_ffff).collect();
        let attendu: Vec<u32> = ligne.iter().map(|&d| reference(d, rgb, a as u8)).collect();
        melange_ligne(&mut ligne, rgb, a);
        assert_eq!(ligne, attendu, "a={}", a);
    }
}

#[test]
fn remplissage_exact_a_tout_alignement_et_toute_longueur() {
    let mut tampon = vec![0xdead_beefu32; 128];
    for debut in 0..5 {
        for longueur in 0..70 {
            tampon.fill(0xdead_beef);
            remplit_portable(&mut tampon[debut..debut + longueur], 0x0012_3456);
            for (i, &px) in tampon.iter().enumerate() {
                let dedans = i >= debut && i < debut + longueur;
                assert_eq!(px, if dedans { 0x0012_3456 } else { 0xdead_beef },
                    "debut={} longueur={} i={}", debut, longueur, i);
            }
        }
    }
}
