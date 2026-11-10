//! Les icones applicatives : des ASSETS Atelier, decodes une fois.
//!
//! # D'ou elles viennent (BOUCHAUD_ATELIER_ASSETS_V1)
//!
//! Chaque icone est une tuile Atelier -- carre arrondi en degrade, glyphe
//! Lucide (ISC) -- ecrite en SVG dans `assets/atelier/sources/apps/` par
//! `tools/assets/fabrique-atelier.py`, puis RENDUE par resvg a chaque taille
//! posee (18, 40, 56 px). Rien n'est dessine ici : ce module choisit le PNG de
//! la bonne taille, le decode une fois, et le recopie.
//!
//! Les anciennes icones (disques et rectangles, puis PNG reduits a la volee,
//! icone Services en sur-echantillonnage) n'existent plus. Une taille rendue
//! depuis le vecteur est plus nette qu'une grande image reduite.
//!
//! # Le cache
//!
//! Une entree par (icone, taille), creee a la premiere demande. Une taille non
//! fabriquee est reduite depuis la plus grande (moyenne de blocs sur l'alpha
//! premultiplie), pour ne jamais montrer un trou -- mais aucun appelant du
//! bureau ne la demande : dock 40, bureau 56, barres et menu 18.
//!
//! `static mut` sur le meme regime que le reste du bureau : le gestionnaire de
//! fenetres est un fil unique, et c'est lui seul qui dessine.

use alloc::vec;
use alloc::vec::Vec;

/// Cote d'une icone du bureau, en pixels.
pub const TAILLE_BUREAU: usize = 56;
/// Cote d'une icone dans une barre ou un menu.
pub const TAILLE_PETITE: usize = 18;
/// Cote d'une icone dans le dock Atelier (`atelier::dock::ICONE`).
pub const TAILLE_DOCK: usize = 40;

/// Tailles fabriquees, dans l'ordre des colonnes de `SOURCES`.
const TAILLES: [usize; 3] = [TAILLE_PETITE, TAILLE_DOCK, TAILLE_BUREAU];

macro_rules! tuile {
    ($nom:literal) => {
        [
            include_bytes!(concat!("../assets/atelier/apps/", $nom, "-18.png")).as_slice(),
            include_bytes!(concat!("../assets/atelier/apps/", $nom, "-40.png")).as_slice(),
            include_bytes!(concat!("../assets/atelier/apps/", $nom, "-56.png")).as_slice(),
        ]
    };
}

/// Les tuiles dans leur ordre propre (INDEPENDANT de `window::ICONS`).
/// Le bureau comme le dock doivent utiliser `pour_kind` / `pour_app`, jamais
/// le rang d'une icone de bureau : l'index 8 est Moniteur, 11 est DOOM.
const SOURCES: [[&[u8]; 3]; 12] = [
    tuile!("navigateur"),
    tuile!("calculatrice"),
    tuile!("terminal"),
    tuile!("fichiers"),
    tuile!("edition"),
    tuile!("services"),
    tuile!("notes"),
    tuile!("insights"),
    tuile!("moniteur"),
    tuile!("journal"),
    tuile!("parametres"),
    tuile!("doom"),
];

pub const NOMBRE: usize = SOURCES.len();

/// Indice de la tuile Parametres (pas encore d'application).
pub const PARAMETRES: usize = 10;

/// Une icone prete a poser : `cote x cote` pixels `0xAARRGGBB`.
struct Prete {
    index: usize,
    cote: usize,
    pixels: Vec<u32>,
}

static mut CACHE: Vec<Prete> = Vec::new();

/// Octets d'une icone a la taille exacte, si elle est fabriquee.
pub fn octets(index: usize, cote: usize) -> Option<&'static [u8]> {
    let colonne = TAILLES.iter().position(|&t| t == cote)?;
    SOURCES.get(index).map(|ligne| ligne[colonne])
}

fn prepare(index: usize, cote: usize) -> Option<&'static Prete> {
    if index >= NOMBRE || cote == 0 {
        return None;
    }
    // SAFETY : fil unique du gestionnaire de fenetres (voir l'en-tete).
    unsafe {
        let cache = &mut *core::ptr::addr_of_mut!(CACHE);
        if let Some(i) = cache.iter().position(|p| p.index == index && p.cote == cote) {
            return cache.get(i);
        }
        let pixels = match octets(index, cote) {
            Some(png) => {
                let image = crate::gui::png::decode(png)?;
                if image.largeur != cote || image.hauteur != cote {
                    return None;
                }
                image.pixels
            }
            None => {
                let grande = crate::gui::png::decode(SOURCES[index][TAILLES.len() - 1])?;
                reduit(&grande, cote)
            }
        };
        cache.push(Prete { index, cote, pixels });
        cache.last()
    }
}

/// Moyenne par blocs, sur l'alpha premultiplie (taille non fabriquee).
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

/// Pose l'icone `index`, coin superieur gauche en `(x, y)`.
///
/// Ne dessine rien si l'image manque : une icone absente vaut mieux qu'un
/// carre gris, et le fond du bureau est deja peint dessous.
pub fn dessine(index: usize, x: usize, y: usize, cote: usize) {
    // BOUCHAUD_GFX_CULLING_AMONT_V1 : rien a composer si la trame ne presente
    // pas un pixel de l'icone -- verifie AVANT un eventuel decodage.
    if !crate::gui::framebuffer::decoupe_touche(x, y, cote, cote) {
        return;
    }
    let Some(prete) = prepare(index, cote) else { return };
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
        crate::gui::window::KIND_DOOM => Some(11),
        4 => Some(1), // Calculatrice
        0 => Some(2), // Terminal
        1 => Some(3), // Fichiers
        5 => Some(4), // Rustpad
        crate::gui::window::KIND_SERVICES => Some(5),
        crate::gui::window::KIND_NOTES => Some(6),
        crate::gui::window::KIND_INSIGHTS => Some(7),
        3 => Some(8), // Moniteur
        crate::gui::window::KIND_JOURNAL => Some(9),
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
        App::Application { .. } => return None,
        App::Navigateur { .. } => 0,
        App::Doom { .. } => 11,
        App::Calc { .. } => 1,
        App::Terminal { .. } => 2,
        App::Files { .. } => 3,
        App::Rustpad { .. } => 4,
        App::Services => 5,
        App::Notes { .. } => 6,
        App::Insights => 7,
        App::Monitor => 8,
        App::Journal { .. } => 9,
    })
}
