//! Fond d'ecran Atelier : une vraie photographie (BOUCHAUD_ATELIER_FOND_IMAGE_V1).
//!
//! « summer 1am », Risto Saukonpää, LGPLv3 (KDE plasma-workspace-wallpapers),
//! recadree « cover » a 1280x720 et 1920x1080 par `tools/assets/fabrique-atelier.py`.
//! Le fond procedural de l'increment 1 (montagnes calculees) n'existe plus.
//!
//! # Decode une fois, recopie ensuite
//!
//! A la premiere trame, a la resolution courante :
//!   1. une variante EXACTE existe : elle est decodee et devient le tampon ;
//!   2. sinon, la plus grande est decodee puis recadree « cover » (centre, sans
//!      deformation) a l'ecran, en echantillonnage bilineaire, une fois ;
//!   3. si le decodage echoue : repli sur un degrade leger dans la meme
//!      palette -- le bureau reste utilisable, et le journal le dit.
//! Une trame ne fait ensuite que recopier les lignes de sa decoupe. Le journal
//! publie la source, le mode, la duree et la memoire retenue
//! (`BOUCHAUD_ATELIER_FOND`).

extern crate alloc;

use alloc::vec::Vec;
use crate::gui::framebuffer as fb;
use super::assets;
use super::echelle::{couvre, mix};

/// (largeur, hauteur, pixels) du fond pret a l'ecran.
static mut CACHE: Option<(usize, usize, Vec<u32>)> = None;

/// Peint le fond sous la decoupe courante.
pub fn dessine() {
    let (largeur, hauteur) = (fb::width(), fb::height());
    if largeur == 0 || hauteur == 0 {
        return;
    }
    // SAFETY : fil unique du gestionnaire de fenetres, comme `icones::CACHE`.
    let pixels = unsafe {
        let cache = &mut *core::ptr::addr_of_mut!(CACHE);
        let perime = !matches!(cache, Some((l, h, _)) if *l == largeur && *h == hauteur);
        if perime {
            *cache = Some((largeur, hauteur, prepare(largeur, hauteur)));
        }
        match cache {
            Some((_, _, p)) => &*p,
            None => return,
        }
    };
    let (cx0, cy0, cx1, cy1) = fb::clip_rect();
    let cx1 = cx1.min(largeur);
    if cx1 <= cx0 {
        return;
    }
    let mut ecrits = 0u64;
    for y in cy0..cy1.min(hauteur) {
        let destination = fb::ligne_mut(cx0, y, cx1 - cx0);
        let n = destination.len();
        if n == 0 {
            continue;
        }
        let debut = y * largeur + cx0;
        destination.copy_from_slice(&pixels[debut..debut + n]);
        ecrits += n as u64;
    }
    fb::note_pixels_dessines(ecrits);
}

/// Variantes embarquees, de la plus petite a la plus grande.
const VARIANTES: [(&str, usize, usize, &[u8]); 2] = [
    ("summer_1am-1280x720", 1280, 720, assets::FOND_1280X720),
    ("summer_1am-1920x1080", 1920, 1080, assets::FOND_1920X1080),
];

fn prepare(largeur: usize, hauteur: usize) -> Vec<u32> {
    let debut = crate::kernel::timer::monotonic_ms();
    let exacte = VARIANTES.iter().find(|v| v.1 == largeur && v.2 == hauteur);
    let (source, mode, resultat) = match exacte {
        Some(v) => (v.0, "exact", crate::gui::png::decode(v.3)
            .filter(|i| i.largeur == largeur && i.hauteur == hauteur)
            .map(|i| opaque(i.pixels))),
        None => {
            let v = VARIANTES[VARIANTES.len() - 1];
            (v.0, "cover", crate::gui::png::decode(v.3)
                .map(|i| couvre(&i.pixels, i.largeur, i.hauteur, largeur, hauteur)))
        }
    };
    let (pixels, mode) = match resultat {
        Some(p) => (p, mode),
        None => (repli(largeur, hauteur), "repli"),
    };
    crate::serial_println!(
        "BOUCHAUD_ATELIER_FOND source={} mode={} ecran={}x{} ms={} ram_kio={}",
        source, mode, largeur, hauteur,
        crate::kernel::timer::monotonic_ms().saturating_sub(debut),
        pixels.len() * 4 / 1024);
    pixels
}

/// Le framebuffer ne lit pas l'alpha : on le met a zero pour qu'une copie
/// brute ne transporte rien d'autre que la couleur.
fn opaque(mut p: Vec<u32>) -> Vec<u32> {
    for px in p.iter_mut() {
        *px &= 0x00ff_ffff;
    }
    p
}

/// Repli : ciel lavande vers rose, la palette de la photographie. Une couleur
/// par ligne, aucun calcul par pixel.
fn repli(largeur: usize, hauteur: usize) -> Vec<u32> {
    let mut p = alloc::vec![0u32; largeur * hauteur];
    for y in 0..hauteur {
        let t = (y * 256 / hauteur.max(1)) as u32;
        let c = if t < 160 { mix(0x6f6ac8, 0xc7a9dd, t * 256 / 160) }
            else { mix(0xc7a9dd, 0xf0c3cf, (t - 160) * 256 / 96) };
        p[y * largeur..(y + 1) * largeur].fill(c);
    }
    p
}
