//! Les icones du bureau : de vraies images, decodees une fois.
//!
//! # Ce qu'elles etaient
//!
//! Des disques et des rectangles empiles dans `widgets.rs`, un peintre par
//! application, redessines a chaque trame. La premiere capture d'ecran venue
//! le montrait : cinq dessins sans famille, qui ne ressemblaient pas a des
//! icones.
//!
//! # Ce qu'elles sont
//!
//! Quatre PNG fabriques par `tools/assets/fabrique-icones.py` -- du code
//! lisible, revu comme le reste, et qui les refait a l'octet pres -- plus le
//! vrai logo de Ladybird, pris a son depot. L'icone Services est une forme
//! vectorielle rendue par surechantillonnage 4x4 : ses arrondis restent nets
//! a 18 comme a 56 pixels.
//!
//! # Pourquoi un cache
//!
//! Decoder un PNG, c'est un `inflate` et un defiltrage : quelques centaines de
//! micro-secondes. Le compositeur redessine une icone a chaque rectangle de
//! degat qui la touche. Les images sont donc decodees et reduites UNE fois, a
//! la premiere demande, dans la taille exacte ou elles seront posees -- pas de
//! mise a l'echelle par trame, et un `blit_argb_span` par ligne.
//!
//! La reduction se fait par moyenne de blocs sur l'alpha PREMULTIPLIE. Moyenner
//! des couleurs non premultipliees ferait entrer la couleur des pixels
//! transparents -- indefinie -- dans le bord de l'icone : une frange claire
//! tout autour, le defaut le plus courant du redimensionnement d'images a
//! transparence.

use alloc::vec;
use alloc::vec::Vec;

/// Cote d'une icone du bureau, en pixels.
pub const TAILLE_BUREAU: usize = 56;

/// Cote d'une icone dans une barre ou un menu.
pub const TAILLE_PETITE: usize = 18;

/// Les images, dans l'ordre de `window::ICONS`.
const SOURCES: [&[u8]; 5] = [
    include_bytes!("../assets/icons/ladybird.png"),
    include_bytes!("../assets/icons/calculatrice.png"),
    include_bytes!("../assets/icons/terminal.png"),
    include_bytes!("../assets/icons/fichiers.png"),
    include_bytes!("../assets/icons/rustpad.png"),
];

pub const NOMBRE: usize = SOURCES.len();

/// Une icone prete a poser : `TAILLE x TAILLE` pixels `0xAARRGGBB`.
struct Prete {
    cote: usize,
    pixels: Vec<u32>,
}

/// Cache par (image, taille). Deux tailles suffisent : le bureau et les barres.
///
/// `static mut` sur le meme regime que le reste du bureau (`window::ICON_POSITIONS`,
/// `ramfs::fs`) : le gestionnaire de fenetres est un fil unique, et c'est lui
/// seul qui dessine. Aucun chemin d'interruption ne passe ici.
static mut CACHE: [[Option<Prete>; 2]; NOMBRE] = [const { [None, None] }; NOMBRE];

fn rang(cote: usize) -> usize {
    if cote == TAILLE_PETITE { 1 } else { 0 }
}

/// Decode, reduit et retient l'icone `index` a la taille `cote`.
fn prepare(index: usize, cote: usize) -> Option<&'static Prete> {
    if index >= NOMBRE || cote == 0 {
        return None;
    }
    let emplacement = rang(cote);
    // SAFETY : fil unique du gestionnaire de fenetres, voir le commentaire de
    // `CACHE`. La reference rendue est immuable et vit aussi longtemps que le
    // cache, qui n'est jamais vide.
    unsafe {
        let cache = &mut *core::ptr::addr_of_mut!(CACHE);
        if cache[index][emplacement].is_none() {
            let image = crate::gui::png::decode(SOURCES[index])?;
            cache[index][emplacement] = Some(Prete {
                cote,
                pixels: reduit(&image, cote),
            });
        }
        cache[index][emplacement].as_ref()
    }
}

/// Moyenne par blocs, sur l'alpha premultiplie.
fn reduit(image: &crate::gui::png::Image, cote: usize) -> Vec<u32> {
    let mut sortie = vec![0u32; cote * cote];
    for y in 0..cote {
        let y0 = y * image.hauteur / cote;
        let y1 = (((y + 1) * image.hauteur) / cote).max(y0 + 1).min(image.hauteur);
        for x in 0..cote {
            let x0 = x * image.largeur / cote;
            let x1 = (((x + 1) * image.largeur) / cote).max(x0 + 1).min(image.largeur);

            let (mut sr, mut sv, mut sb, mut sa, mut compte) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let pixel = image.pixel(xx, yy);
                    let a = (pixel >> 24) as u64;
                    sr += ((pixel >> 16) & 0xff) as u64 * a;
                    sv += ((pixel >> 8) & 0xff) as u64 * a;
                    sb += (pixel & 0xff) as u64 * a;
                    sa += a;
                    compte += 1;
                }
            }
            if sa == 0 || compte == 0 {
                continue;
            }
            let r = (sr / sa).min(255);
            let v = (sv / sa).min(255);
            let b = (sb / sa).min(255);
            let a = (sa / compte).min(255);
            sortie[y * cote + x] =
                ((a as u32) << 24) | ((r as u32) << 16) | ((v as u32) << 8) | b as u32;
        }
    }
    sortie
}

#[inline]
fn dans_rectangle_arrondi(px: i32, py: i32, x0: i32, y0: i32, x1: i32, y1: i32, rayon: i32) -> bool {
    if px < x0 || py < y0 || px >= x1 || py >= y1 { return false; }
    let cx = px.clamp(x0 + rayon, x1 - rayon - 1);
    let cy = py.clamp(y0 + rayon, y1 - rayon - 1);
    let dx = px - cx;
    let dy = py - cy;
    dx * dx + dy * dy <= rayon * rayon
}

#[inline]
fn couverture_services(px: usize, py: usize, cote: usize, forme: u8) -> u8 {
    let mut touches = 0u16;
    // Seize echantillons donnent des bords reguliers sans conserver de
    // bitmap ni faire dependre l'icone de sa taille finale.
    for sy in 0..4 {
        for sx in 0..4 {
            let nx = ((px * 4 + sx) * 1024 / (cote * 4).max(1)) as i32;
            let ny = ((py * 4 + sy) * 1024 / (cote * 4).max(1)) as i32;
            let dedans = match forme {
                0 => (0..3).any(|rangee| {
                    let haut = 170 + rangee * 245;
                    dans_rectangle_arrondi(nx, ny, 120, haut, 904, haut + 180, 54)
                }),
                1 => (0..3).any(|rangee| {
                    let haut = 170 + rangee * 245;
                    dans_rectangle_arrondi(nx, ny, 330, haut + 69, 745, haut + 111, 18)
                }),
                _ => (0..3).any(|rangee| {
                    let haut = 170 + rangee * 245;
                    let dx = nx - 220;
                    let dy = ny - (haut + 90);
                    dx * dx + dy * dy <= 34 * 34
                }),
            };
            if dedans { touches += 1; }
        }
    }
    (touches * 255 / 16) as u8
}

fn dessine_services_vectoriel(x: usize, y: usize, cote: usize) {
    if cote == 0 { return; }
    for py in 0..cote {
        for px in 0..cote {
            let corps = couverture_services(px, py, cote, 0);
            if corps != 0 {
                crate::gui::framebuffer::blend_rgb(x + px, y + py, 0x2563eb, corps);
            }
            let fente = couverture_services(px, py, cote, 1);
            if fente != 0 {
                crate::gui::framebuffer::blend_rgb(x + px, y + py, 0xdbeafe, fente);
            }
            let voyant = couverture_services(px, py, cote, 2);
            if voyant != 0 {
                crate::gui::framebuffer::blend_rgb(x + px, y + py, 0x5bda8b, voyant);
            }
        }
    }
}

/// Pose l'icone `index`, coin superieur gauche en `(x, y)`.
///
/// Ne dessine rien si l'image manque : une icone absente vaut mieux qu'un
/// carre gris, et le fond du bureau est deja peint dessous.
pub fn dessine(index: usize, x: usize, y: usize, cote: usize) {
    if index == 5 {
        dessine_services_vectoriel(x, y, cote);
        return;
    }
    let Some(prete) = prepare(index, cote) else { return };
    // BOUCHAUD_GFX_CULLING_AMONT_V1 : rien a composer si la trame ne presente
    // pas un pixel de l'icone.
    if !crate::gui::framebuffer::decoupe_touche(x, y, prete.cote, prete.cote) {
        return;
    }
    for ligne in 0..prete.cote {
        let debut = ligne * prete.cote;
        crate::gui::framebuffer::blit_argb_span(
            x,
            y + ligne,
            &prete.pixels[debut..debut + prete.cote],
        );
    }
}

/// L'icone associee a un `kind` d'application, s'il en a une.
///
/// Le `kind` est ce que portent les fenetres et les entrees de menu ; l'indice
/// d'icone est celui de `window::ICONS`. Une seule traduction, ici, pour que la
/// barre des taches et le bureau ne puissent pas montrer deux images
/// differentes pour la meme application.
pub fn pour_kind(kind: usize) -> Option<usize> {
    match kind {
        crate::gui::window::KIND_NAVIGATEUR => Some(0),
        4 => Some(1), // Calculatrice
        0 => Some(2), // Terminal
        1 => Some(3), // Fichiers
        5 => Some(4), // Rustpad
        crate::gui::window::KIND_SERVICES => Some(5),
        _ => None,
    }
}

/// L'icone d'une fenetre, d'apres l'application qu'elle porte.
///
/// `Win` ne retient pas le `kind` qui l'a creee : c'est `App` qui dit ce
/// qu'elle est. Une seule traduction, ici, pour que la barre des taches et le
/// bureau ne puissent pas montrer deux images differentes pour la meme chose.
pub fn pour_app(app: &crate::gui::window::App) -> Option<usize> {
    use crate::gui::window::App;
    Some(match app {
        App::Navigateur { .. } => 0,
        App::Calc { .. } => 1,
        App::Terminal { .. } => 2,
        App::Files { .. } => 3,
        App::Rustpad { .. } => 4,
        App::Services => 5,
        App::Monitor | App::Journal { .. } => return None,
    })
}
