//! Decodeur PNG : de quoi peindre de vraies images, pas des cercles.
//!
//! # Pourquoi le bureau en avait besoin
//!
//! Les icones du bureau etaient DESSINEES : des disques, des rectangles et des
//! degrades empiles a la main, un par application. Cela tenait tant qu'on ne
//! regardait pas de trop pres -- et le premier coup d'oeil sur une capture
//! d'ecran suffisait a voir des cercles empiles.
//!
//! Une vraie image demande un decodeur. PNG en est un petit : un en-tete, des
//! morceaux, un flux zlib, et cinq filtres de ligne. Le noyau a deja `inflate`
//! -- il decompresse les reponses HTTP depuis longtemps -- donc il ne manquait
//! que la couche PNG.
//!
//! # Ce qui est gere
//!
//! Huit bits par composante, entrelacement nul, et les quatre types de couleur
//! utiles a des icones : niveaux de gris, RVB, palette et leurs variantes
//! alpha. Pas d'Adam7, pas de 16 bits : ce sont des formats qu'aucun outil ne
//! produit par defaut pour une icone, et les refuser vaut mieux que les
//! decoder a moitie.
//!
//! # La sortie
//!
//! Un `Image` en `0xAARRGGBB` non premultiplie, c'est-a-dire exactement ce que
//! `framebuffer::blend_rgb` attend. Le compositeur n'a donc aucune conversion
//! a faire par pixel.
//!
//! Le format est PUR : ce fichier ne parle ni au materiel ni au compositeur, et
//! `tools/gui/test_png.rs` l'inclut tel quel pour le comparer, image par image,
//! a ce que le generateur d'assets a encode.

use alloc::vec;
use alloc::vec::Vec;

/// Une image decodee, en `0xAARRGGBB`.
pub struct Image {
    pub largeur: usize,
    pub hauteur: usize,
    pub pixels: Vec<u32>,
}

impl Image {
    /// Pixel `(x, y)`, ou transparent hors bornes.
    pub fn pixel(&self, x: usize, y: usize) -> u32 {
        if x >= self.largeur || y >= self.hauteur {
            return 0;
        }
        self.pixels[y * self.largeur + x]
    }
}

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Decode un PNG. `None` si le fichier est abime ou d'une variante non geree.
///
/// Ne panique jamais : les assets sont embarques a la compilation, mais ce code
/// est aussi le chemin par lequel une image viendrait d'ailleurs un jour.
pub fn decode(octets: &[u8]) -> Option<Image> {
    if octets.len() < 8 || octets[..8] != SIGNATURE {
        return None;
    }

    let mut position = 8usize;
    let mut largeur = 0usize;
    let mut hauteur = 0usize;
    let mut couleur = 0u8;
    let mut palette: Vec<(u8, u8, u8)> = Vec::new();
    let mut transparence: Vec<u8> = Vec::new();
    let mut comprime: Vec<u8> = Vec::new();

    while position + 8 <= octets.len() {
        let taille = lit_u32(&octets[position..position + 4])? as usize;
        let genre = &octets[position + 4..position + 8];
        let debut = position + 8;
        let fin = debut.checked_add(taille)?;
        if fin + 4 > octets.len() {
            return None;
        }
        let contenu = &octets[debut..fin];

        match genre {
            b"IHDR" => {
                if contenu.len() < 13 {
                    return None;
                }
                largeur = lit_u32(&contenu[0..4])? as usize;
                hauteur = lit_u32(&contenu[4..8])? as usize;
                let profondeur = contenu[8];
                couleur = contenu[9];
                // compression 0, filtre 0, entrelacement 0 : tout le reste est
                // refuse plutot que mal decode.
                if contenu[10] != 0 || contenu[11] != 0 || contenu[12] != 0 {
                    return None;
                }
                if profondeur != 8 || largeur == 0 || hauteur == 0 {
                    return None;
                }
                // Une image demesuree ne doit pas faire exploser le tas du
                // noyau sur un fichier corrompu.
                if largeur > 4096 || hauteur > 4096 {
                    return None;
                }
            }
            b"PLTE" => {
                palette = contenu
                    .chunks_exact(3)
                    .map(|c| (c[0], c[1], c[2]))
                    .collect();
            }
            b"tRNS" => {
                transparence = contenu.to_vec();
            }
            b"IDAT" => {
                comprime.extend_from_slice(contenu);
            }
            b"IEND" => break,
            _ => {}
        }
        position = fin + 4;
    }

    if largeur == 0 || comprime.is_empty() {
        return None;
    }

    let composantes = match couleur {
        0 => 1usize, // gris
        2 => 3,      // RVB
        3 => 1,      // index de palette
        4 => 2,      // gris + alpha
        6 => 4,      // RVBA
        _ => return None,
    };

    let brut = crate::net::encoding::inflate::zlib_decode(&comprime).ok()?;
    let par_ligne = largeur.checked_mul(composantes)?;
    if brut.len() < hauteur.checked_mul(par_ligne + 1)? {
        return None;
    }

    let mut lignes = vec![0u8; hauteur * par_ligne];
    defiltre(&brut, &mut lignes, hauteur, par_ligne, composantes)?;

    let mut pixels = Vec::with_capacity(largeur * hauteur);
    for index in 0..largeur * hauteur {
        let source = &lignes[index * composantes..(index + 1) * composantes];
        pixels.push(match couleur {
            0 => argb(255, source[0], source[0], source[0]),
            2 => argb(255, source[0], source[1], source[2]),
            3 => {
                let entree = source[0] as usize;
                let (r, v, b) = *palette.get(entree)?;
                let a = transparence.get(entree).copied().unwrap_or(255);
                argb(a, r, v, b)
            }
            4 => argb(source[1], source[0], source[0], source[0]),
            _ => argb(source[3], source[0], source[1], source[2]),
        });
    }

    Some(Image { largeur, hauteur, pixels })
}

fn argb(a: u8, r: u8, v: u8, b: u8) -> u32 {
    ((a as u32) << 24) | ((r as u32) << 16) | ((v as u32) << 8) | b as u32
}

fn lit_u32(octets: &[u8]) -> Option<u32> {
    if octets.len() < 4 {
        return None;
    }
    Some(u32::from_be_bytes([octets[0], octets[1], octets[2], octets[3]]))
}

/// Defait les cinq filtres de ligne du PNG.
///
/// Chaque ligne du flux decompresse commence par un octet de filtre, suivi de
/// `par_ligne` octets. Le filtre se defait avec l'octet de gauche (`a`), celui
/// du dessus (`b`) et celui du dessus a gauche (`c`) -- deja defiltres, donc
/// l'ordre des lignes compte.
fn defiltre(brut: &[u8], sortie: &mut [u8], hauteur: usize, par_ligne: usize,
    composantes: usize) -> Option<()> {
    for ligne in 0..hauteur {
        let entree = ligne * (par_ligne + 1);
        let filtre = *brut.get(entree)?;
        let source = brut.get(entree + 1..entree + 1 + par_ligne)?;
        let base = ligne * par_ligne;

        for octet in 0..par_ligne {
            let x = source[octet];
            let a = if octet >= composantes {
                sortie[base + octet - composantes]
            } else {
                0
            };
            let b = if ligne > 0 {
                sortie[base - par_ligne + octet]
            } else {
                0
            };
            let c = if ligne > 0 && octet >= composantes {
                sortie[base - par_ligne + octet - composantes]
            } else {
                0
            };
            sortie[base + octet] = match filtre {
                0 => x,
                1 => x.wrapping_add(a),
                2 => x.wrapping_add(b),
                3 => x.wrapping_add((((a as u16) + (b as u16)) / 2) as u8),
                4 => x.wrapping_add(paeth(a, b, c)),
                _ => return None,
            };
        }
    }
    Some(())
}

/// Le predicteur de Paeth : celui des trois voisins dont la somme lineaire est
/// la plus proche, departages dans l'ordre `a`, `b`, `c` -- l'ordre compte, une
/// egalite tranchee autrement decale toute la ligne.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let pa = (p - a as i16).abs();
    let pb = (p - b as i16).abs();
    let pc = (p - c as i16).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}
