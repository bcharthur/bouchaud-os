//! Assets graphiques Atelier embarques (BOUCHAUD_ATELIER_ASSETS_V1).
//!
//! Les fichiers viennent de `tools/assets/fabrique-atelier.py` ; leur
//! provenance, leur licence et leur empreinte sont dans
//! `src/assets/atelier/MANIFEST.json`, verifie par
//! `tools/verifie-assets-atelier.py`. Le noyau n'embarque QUE ce qu'il pose :
//! pas la bibliotheque Lucide, une poignee de glyphes deja rendus.
//!
//! Deux natures d'images :
//!   * les IMAGES (logo) se posent telles quelles, alpha compris ;
//!   * les SYMBOLES (glyphes Lucide blancs) sont des MASQUES : leur alpha est
//!     une couverture, teintee a la pose -- le meme glyphe sert sur verre
//!     clair et sur surface sombre.

use alloc::vec::Vec;
use crate::gui::framebuffer as fb;

/// Marque Bouchaud OS (PROVISOIRE, voir `assets/atelier/sources/logo/`).
pub const LOGO_18: &[u8] = include_bytes!("../../assets/atelier/logo/bouchaud-os-18.png");
pub const LOGO_72: &[u8] = include_bytes!("../../assets/atelier/logo/bouchaud-os-72.png");
/// Glyphe Lucide `layout-grid`, 22 px : la cellule Applications du dock.
pub const SYMBOLE_GRILLE_22: &[u8] =
    include_bytes!("../../assets/atelier/symboles/layout-grid-22.png");

/// Glyphes Lucide de la barre du haut (16 px) et du centre de controle (18 px).
///
/// L'etat du reseau se lit par la FORME : prise Ethernet quand le lien porte
/// une adresse, prise debranchee quand il n'y a rien au bout, cercle d'alerte
/// quand le cable porte mais qu'aucune adresse n'a ete obtenue.
pub const SYMBOLE_ETHERNET_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/ethernet-port-16.png");
pub const SYMBOLE_ETHERNET_18: &[u8] =
    include_bytes!("../../assets/atelier/symboles/ethernet-port-18.png");
pub const SYMBOLE_DEBRANCHE_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/unplug-16.png");
pub const SYMBOLE_DEBRANCHE_18: &[u8] =
    include_bytes!("../../assets/atelier/symboles/unplug-18.png");
pub const SYMBOLE_ALERTE_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/circle-alert-16.png");
pub const SYMBOLE_ALERTE_18: &[u8] =
    include_bytes!("../../assets/atelier/symboles/circle-alert-18.png");
/// Ouvre le centre de controle.
pub const SYMBOLE_REGLAGES_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/sliders-horizontal-16.png");
pub const SYMBOLE_ETEINDRE_18: &[u8] =
    include_bytes!("../../assets/atelier/symboles/power-18.png");
pub const SYMBOLE_REDEMARRER_18: &[u8] =
    include_bytes!("../../assets/atelier/symboles/rotate-ccw-18.png");
pub const SYMBOLE_QUITTER_18: &[u8] =
    include_bytes!("../../assets/atelier/symboles/log-out-18.png");
pub const SYMBOLE_DIAGNOSTIC_18: &[u8] =
    include_bytes!("../../assets/atelier/symboles/activity-18.png");
pub const SYMBOLE_ECRAN_18: &[u8] =
    include_bytes!("../../assets/atelier/symboles/monitor-18.png");

/// Fichiers Atelier (increment D) : glyphes 16 px de la barre laterale, de la
/// navigation et du menu contextuel (masques teintables).
pub const SYMBOLE_ACCUEIL_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/house-16.png");
pub const SYMBOLE_DOCUMENTS_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/file-text-16.png");
pub const SYMBOLE_IMAGES_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/image-16.png");
pub const SYMBOLE_TELECHARGEMENTS_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/download-16.png");
pub const SYMBOLE_RACINE_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/hard-drive-16.png");
pub const SYMBOLE_DOSSIER_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/folder-16.png");
pub const SYMBOLE_GRILLE_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/layout-grid-16.png");
pub const SYMBOLE_LISTE_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/list-16.png");
pub const SYMBOLE_PRECEDENT_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/chevron-left-16.png");
pub const SYMBOLE_SUIVANT_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/chevron-right-16.png");
pub const SYMBOLE_PARENT_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/arrow-up-16.png");
pub const SYMBOLE_COPIER_16: &[u8] =
    include_bytes!("../../assets/atelier/symboles/copy-16.png");

/// Icones de types de fichiers : 48 px (grille, page + badge) et 20 px (liste,
/// glyphe colore). Index : `fichiers::Genre as usize`.
macro_rules! type_fichier {
    ($nom:literal) => {
        [include_bytes!(concat!("../../assets/atelier/fichiers/", $nom, "-20.png")) as &[u8],
         include_bytes!(concat!("../../assets/atelier/fichiers/", $nom, "-48.png")) as &[u8]]
    };
}
pub const TYPES_FICHIERS: [[&[u8]; 2]; 7] = [
    type_fichier!("dossier"),
    type_fichier!("fichier"),
    type_fichier!("texte"),
    type_fichier!("image"),
    type_fichier!("code"),
    type_fichier!("archive"),
    type_fichier!("executable"),
];

/// Fond « summer 1am » (Risto Saukonpää, LGPLv3, KDE), recadre « cover ».
pub const FOND_1280X720: &[u8] =
    include_bytes!("../../assets/atelier/fond/summer_1am-1280x720.png");
pub const FOND_1920X1080: &[u8] =
    include_bytes!("../../assets/atelier/fond/summer_1am-1920x1080.png");

/// Une image decodee, retenue.
struct Retenue {
    cle: usize,
    largeur: usize,
    hauteur: usize,
    pixels: Vec<u32>,
}

static mut RETENUES: Vec<Retenue> = Vec::new();

/// Decode (une fois) et rend l'image `octets`. `None` si elle est abimee.
fn retenue(octets: &'static [u8]) -> Option<&'static Retenue> {
    let cle = octets.as_ptr() as usize;
    // SAFETY : fil unique du gestionnaire de fenetres.
    unsafe {
        let r = &mut *core::ptr::addr_of_mut!(RETENUES);
        if let Some(i) = r.iter().position(|x| x.cle == cle) {
            return r.get(i);
        }
        let image = crate::gui::png::decode(octets)?;
        r.push(Retenue { cle, largeur: image.largeur, hauteur: image.hauteur,
            pixels: image.pixels });
        r.last()
    }
}

/// Dimensions d'une image embarquee.
pub fn dimensions(octets: &'static [u8]) -> Option<(usize, usize)> {
    retenue(octets).map(|r| (r.largeur, r.hauteur))
}

/// Pose une image, alpha compris, coin superieur gauche en `(x, y)`.
pub fn pose(octets: &'static [u8], x: usize, y: usize) {
    let Some((l, h)) = dimensions(octets) else { return };
    if !fb::decoupe_touche(x, y, l, h) {
        return;
    }
    let Some(r) = retenue(octets) else { return };
    for ligne in 0..r.hauteur {
        let debut = ligne * r.largeur;
        fb::blit_argb_span(x, y + ligne, &r.pixels[debut..debut + r.largeur]);
    }
}

/// Pose un MASQUE teinte de `couleur` : l'alpha du symbole devient la
/// couverture, multipliee par `opacite`.
pub fn pose_masque(octets: &'static [u8], x: usize, y: usize, couleur: u32, opacite: u8) {
    let Some((l, h)) = dimensions(octets) else { return };
    if !fb::decoupe_touche(x, y, l, h) {
        return;
    }
    let Some(r) = retenue(octets) else { return };
    for ligne in 0..r.hauteur {
        for colonne in 0..r.largeur {
            let a = (r.pixels[ligne * r.largeur + colonne] >> 24) * opacite as u32 / 255;
            if a != 0 {
                fb::blend_rgb(x + colonne, y + ligne, couleur, a as u8);
            }
        }
    }
}
