//! Fond d'ecran Atelier (BOUCHAUD_ATELIER_FOND_V1).
//!
//! # Calcule une fois, recopie ensuite
//!
//! Un paysage d'aube -- ciel en degrade, halo du soleil, trois chaines de
//! montagnes estompees par la distance, un lac qui les reflete -- est cher a
//! calculer pixel par pixel, et le compositeur repeint le fond sous CHAQUE
//! rectangle de degat. Il est donc calcule UNE fois, a la premiere trame et a
//! la resolution courante, dans un tampon. Une trame ne fait ensuite que
//! recopier les lignes de sa decoupe (`copy_from_slice`) : le cout d'un degat
//! de curseur reste celui de 22 lignes de 14 pixels.
//!
//! # Deterministe et sans flottant transcendant
//!
//! Le relief vient d'un bruit de valeurs a deux octaves, interpole par
//! `smoothstep` : aucune `sin`, aucune `sqrt` (absentes de `core`). Deux
//! demarrages produisent le meme fond au pixel pres.

extern crate alloc;

use alloc::vec::Vec;
use crate::gui::framebuffer as fb;

/// (largeur, hauteur, pixels) du fond calcule.
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
            *cache = Some((largeur, hauteur, calcule(largeur, hauteur)));
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

// ─── Calcul ───────────────────────────────────────────────────────────────

fn melange(a: u32, b: u32, t: f32) -> u32 {
    let t = if t < 0.0 { 0.0 } else if t > 1.0 { 1.0 } else { t };
    let canal = |d: u32| -> u32 {
        let ca = ((a >> d) & 0xff) as f32;
        let cb = ((b >> d) & 0xff) as f32;
        ((ca + (cb - ca) * t) + 0.5) as u32 & 0xff
    };
    (canal(16) << 16) | (canal(8) << 8) | canal(0)
}

/// Degrade a plusieurs arrets : `(position 0..1, couleur)`, positions croissantes.
fn degrade(arrets: &[(f32, u32)], t: f32) -> u32 {
    if t <= arrets[0].0 {
        return arrets[0].1;
    }
    for paire in arrets.windows(2) {
        let (p0, c0) = paire[0];
        let (p1, c1) = paire[1];
        if t <= p1 {
            return melange(c0, c1, (t - p0) / (p1 - p0));
        }
    }
    arrets[arrets.len() - 1].1
}

fn hache(n: i32, graine: u32) -> f32 {
    let mut x = (n as u32).wrapping_mul(0x9e37_79b1) ^ graine.wrapping_mul(0x85eb_ca6b);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    (x & 0xffff) as f32 / 65535.0
}

/// Bruit de valeurs lisse en `x`, periode `periode` pixels, dans 0..1.
fn bruit(x: f32, periode: f32, graine: u32) -> f32 {
    let u = x / periode;
    let i = u as i32;
    let f = u - i as f32;
    let s = f * f * (3.0 - 2.0 * f);
    let a = hache(i, graine);
    let b = hache(i + 1, graine);
    a + (b - a) * s
}

/// Une chaine de montagnes : ligne de crete et matiere.
struct Chaine {
    base: f32,
    amplitude: f32,
    periode: f32,
    graine: u32,
    haut: u32,
    bas: u32,
}

impl Chaine {
    /// Bruit « en aretes » : `1 - |2n - 1|` transforme les bosses d'un bruit
    /// lisse en sommets marques. Trois octaves : la silhouette, les epaulements,
    /// puis un grain fin qui casse la regularite.
    fn crete(&self, x: f32) -> f32 {
        let arete = |n: f32| -> f32 {
            let v = 2.0 * n - 1.0;
            1.0 - if v < 0.0 { -v } else { v }
        };
        let relief = 0.62 * arete(bruit(x, self.periode, self.graine))
            + 0.28 * arete(bruit(x, self.periode / 2.3, self.graine ^ 0x5bd1))
            + 0.10 * bruit(x, self.periode / 7.0, self.graine ^ 0x0c3f);
        self.base - relief * self.amplitude
    }
}

/// Melange entier : `t` en 256es (0 = `a`, 256 = `b`).
#[inline(always)]
fn mix(a: u32, b: u32, t: u32) -> u32 {
    let t = if t > 256 { 256 } else { t };
    let u = 256 - t;
    let r = (((a >> 16) & 0xff) * u + ((b >> 16) & 0xff) * t) >> 8;
    let g = (((a >> 8) & 0xff) * u + ((b >> 8) & 0xff) * t) >> 8;
    let bl = ((a & 0xff) * u + (b & 0xff) * t) >> 8;
    (r << 16) | (g << 8) | bl
}

// BOUCHAUD_ATELIER_FOND_ENTIER_V1
//
// La premiere version faisait tout en flottant, pixel par pixel : sous TCG,
// ou le flottant est emule, la premiere trame du bureau arrivait 2,3 s plus
// tard (bureau-premiere-trame 7170 ms contre 4821). Le flottant ne sert plus
// qu'aux grandeurs PAR LIGNE ou PAR COLONNE (degrade du ciel, lignes de
// crete) ; tout ce qui est par pixel est entier, et le halo n'est calcule que
// dans sa boite.
fn calcule(largeur: usize, hauteur: usize) -> Vec<u32> {
    let debut = crate::kernel::timer::monotonic_ms();
    let mut p = alloc::vec![0u32; largeur * hauteur];
    let lf = largeur as f32;
    let hf = hauteur as f32;
    let horizon = (hf * 0.64) as usize;
    let hz = horizon as f32;

    // Ciel : bleu pervenche en haut, lavande, rose peche, creme a l'horizon.
    let ciel = [
        (0.00, 0x8b9fd6),
        (0.38, 0xb9b2e2),
        (0.72, 0xf0c2c8),
        (1.00, 0xfde4cd),
    ];
    for y in 0..horizon {
        let c = degrade(&ciel, y as f32 / hz);
        p[y * largeur..(y + 1) * largeur].fill(c);
    }
    // Halo du soleil, derriere les montagnes, a droite du centre : une ellipse
    // (1.35 fois plus large que haute), calculee dans sa seule boite.
    let (sx, sy) = ((lf * 0.64) as i64, (hz - hf * 0.05) as i64);
    let ry = (hf * 0.5) as i64;
    let rx = ry * 135 / 100;
    let r2 = ry * ry;
    let y_min = (sy - ry).max(0) as usize;
    let y_max = ((sy + ry) as usize).min(horizon);
    let x_min = (sx - rx).max(0) as usize;
    let x_max = ((sx + rx) as usize).min(largeur);
    for y in y_min..y_max {
        let dy = y as i64 - sy;
        for x in x_min..x_max {
            let dx = (x as i64 - sx) * 100 / 135;
            let d2 = dx * dx + dy * dy;
            if d2 >= r2 {
                continue;
            }
            // a = (1 - d2/r2)^2 * 0.6, en 256es.
            let q = ((r2 - d2) * 256 / r2) as u32;
            let a = q * q * 154 / (256 * 256);
            let i = y * largeur + x;
            p[i] = mix(p[i], 0xfff3de, a);
        }
    }

    // Trois chaines, de la plus lointaine (claire, brumeuse) a la plus proche.
    let chaines = [
        Chaine { base: hz - hf * 0.015, amplitude: hf * 0.26, periode: lf * 0.16,
            graine: 0x1a2b, haut: 0xc3bfe3, bas: 0xd8cfe6 },
        Chaine { base: hz + hf * 0.005, amplitude: hf * 0.19, periode: lf * 0.11,
            graine: 0x77c1, haut: 0x9b9ccc, bas: 0xb6b0d6 },
        Chaine { base: hz + hf * 0.01, amplitude: hf * 0.09, periode: lf * 0.085,
            graine: 0x3e09, haut: 0x6f74a8, bas: 0x8a8cbb },
    ];
    for (rang, chaine) in chaines.iter().enumerate() {
        for x in 0..largeur {
            let crete = chaine.crete(x as f32);
            let sommet = if crete < 0.0 { 0.0 } else { crete };
            let y0 = sommet as usize;
            if y0 >= horizon {
                continue;
            }
            let hauteur_colonne = ((hz - sommet) as u32).max(1);
            // Neige : seulement sur les deux chaines du fond, la ou la crete
            // monte haut -- elle suit le relief, sans motif repete.
            let neige = if rang < 2 {
                let altitude = (chaine.base - crete) / chaine.amplitude;
                ((altitude - 0.62) * 0.55 * chaine.amplitude).max(0.0) as u32
            } else {
                0
            };
            // Couverture fractionnaire de la crete sur sa premiere ligne.
            let bord = (256.0 * (1.0 - (crete - y0 as f32))) as u32;
            for y in y0..horizon {
                let dans = (y - y0) as u32;
                let mut c = mix(chaine.haut, chaine.bas, dans * 256 / hauteur_colonne);
                if dans < neige {
                    let n = 256 - dans * 256 / neige.max(1);
                    c = mix(c, 0xf4f1fb, 90 + n * 128 / 256);
                }
                let i = y * largeur + x;
                p[i] = if y == y0 { mix(p[i], c, bord) } else { c };
            }
        }
    }

    // Lac : reflet comprime du paysage, teinte d'eau qui fonce vers le bas,
    // quelques rides horizontales a peine marquees.
    let profondeur = (hauteur - horizon).max(1) as u32;
    for y in horizon..hauteur {
        let t = ((y - horizon) as u32) * 256 / profondeur;
        let source = horizon as isize - 1 - (((y - horizon) * 135) / 100) as isize;
        let sy = if source < 0 { 0 } else { source as usize };
        let eau = mix(0xc9cdeb, 0x5d6496, t);
        let force = 97 + t * 102 / 256;
        let ride = hache(y as i32, 0xa11e) > 0.82;
        for x in 0..largeur {
            let mut c = mix(p[sy * largeur + x], eau, force);
            if ride {
                c = mix(c, 0xffffff, 15);
            }
            p[y * largeur + x] = c;
        }
    }

    // Assise du dock : un voile tres leger dans le bas de l'ecran, pour que la
    // pastille de verre se detache sans ombre dure.
    let debut_voile = (hf * 0.82) as usize;
    let etendue = (hauteur - debut_voile).max(1) as u32;
    for y in debut_voile..hauteur {
        let a = ((y - debut_voile) as u32) * 56 / etendue;
        for px in &mut p[y * largeur..(y + 1) * largeur] {
            *px = mix(*px, 0x2b2f57, a);
        }
    }
    crate::serial_println!("BOUCHAUD_ATELIER_FOND_CALCULE largeur={} hauteur={} ms={}",
        largeur, hauteur, crate::kernel::timer::monotonic_ms().saturating_sub(debut));
    p
}
