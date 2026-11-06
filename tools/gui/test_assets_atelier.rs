//! Assets Atelier embarques : decodage, tailles, erreurs, memoire, echelle
//! (BOUCHAUD_ATELIER_ASSETS_V1, BOUCHAUD_ATELIER_ECHELLE_V1).
//!
//! Inclut le decodeur PNG et le module d'echelle DU NOYAU, tels quels.

#![allow(dead_code)]
extern crate alloc;

// Meme harnais que `test_png.rs` : le decodeur appelle
// `crate::net::encoding::inflate`, recree ici sans toucher a la source.
#[path = "../../src/net/encoding/inflate.rs"]
pub mod inflate_reel;
pub mod brotli {
    pub fn decode(_body: &[u8]) -> Option<alloc::vec::Vec<u8>> { None }
}
pub mod net {
    pub mod encoding {
        pub use crate::inflate_reel as inflate;
    }
}
#[path = "../../src/gui/png.rs"]
mod png;
#[path = "../../src/gui/atelier/echelle.rs"]
mod echelle;

const FOND_1280: &[u8] = include_bytes!("../../src/assets/atelier/fond/summer_1am-1280x720.png");
const FOND_1920: &[u8] = include_bytes!("../../src/assets/atelier/fond/summer_1am-1920x1080.png");
const LOGO_18: &[u8] = include_bytes!("../../src/assets/atelier/logo/bouchaud-os-18.png");
const LOGO_72: &[u8] = include_bytes!("../../src/assets/atelier/logo/bouchaud-os-72.png");
const GRILLE_22: &[u8] = include_bytes!("../../src/assets/atelier/symboles/layout-grid-22.png");

macro_rules! tuiles {
    ($($nom:literal),*) => {
        [$(
            ($nom, 18, include_bytes!(concat!("../../src/assets/atelier/apps/", $nom, "-18.png")).as_slice()),
            ($nom, 40, include_bytes!(concat!("../../src/assets/atelier/apps/", $nom, "-40.png")).as_slice()),
            ($nom, 56, include_bytes!(concat!("../../src/assets/atelier/apps/", $nom, "-56.png")).as_slice()),
        )*]
    };
}

fn tuiles() -> [(&'static str, usize, &'static [u8]); 36] {
    tuiles!("navigateur", "calculatrice", "terminal", "fichiers", "edition", "services",
        "notes", "insights", "moniteur", "journal", "parametres", "doom")
}

/// Budgets : ce que le bureau garde en memoire une fois les images decodees.
const BUDGET_FOND_1080_OCTETS: usize = 1920 * 1080 * 4;
/// Mesure : 12 tuiles x (18^2 + 40^2 + 56^2) x 4 = 242 880 octets si TOUTES
/// les tailles sont decodees (le bureau ne decode que celles qu'il pose).
const BUDGET_TUILES_OCTETS: usize = 256 * 1024;

#[test]
fn les_fonds_se_decodent_a_leur_taille_et_sont_opaques() {
    for (octets, l, h) in [(FOND_1280, 1280, 720), (FOND_1920, 1920, 1080)] {
        let i = png::decode(octets).expect("fond indecodable par le noyau");
        assert_eq!((i.largeur, i.hauteur), (l, h));
        assert!(i.pixels.iter().all(|p| p >> 24 == 255), "un fond n'a pas d'alpha");
        assert!(i.pixels.len() * 4 <= BUDGET_FOND_1080_OCTETS);
        // Une photographie, pas un aplat : de la variete de couleurs.
        let mut vues = std::collections::HashSet::new();
        for p in i.pixels.iter().step_by(97) { vues.insert(*p & 0xf0f0f0); }
        assert!(vues.len() > 50, "fond trop uniforme : {} teintes", vues.len());
    }
}

#[test]
fn chaque_tuile_a_sa_taille_de_l_alpha_et_tient_le_budget() {
    let mut total = 0;
    for (nom, cote, octets) in tuiles() {
        let i = png::decode(octets).unwrap_or_else(|| panic!("{nom}-{cote} indecodable"));
        assert_eq!((i.largeur, i.hauteur), (cote, cote), "{nom}-{cote}");
        assert!(i.pixels.iter().any(|p| p >> 24 == 0), "{nom}-{cote} : pas de coin transparent");
        assert!(i.pixels.iter().any(|p| p >> 24 == 255), "{nom}-{cote} : rien d'opaque");
        // Les coins d'une tuile arrondie sont transparents.
        assert_eq!(i.pixels[0] >> 24, 0, "{nom}-{cote} : coin haut gauche opaque");
        total += i.pixels.len() * 4;
    }
    assert!(total <= BUDGET_TUILES_OCTETS, "tuiles decodees : {total} octets");
}

/// La position de l'icone sur le bureau ne determine jamais le PNG.
#[test]
fn le_bureau_choisit_son_icone_par_kind_comme_le_menu_et_le_dock() {
    let widgets = include_str!("../../src/gui/widgets.rs");
    let start = widgets.find("fn draw_icon_at(i: usize)").expect("peintre du bureau introuvable");
    let fin = widgets[start..].find("// ─── Fenêtres")
        .expect("fin du peintre du bureau introuvable") + start;
    let bureau = &widgets[start..fin];
    assert!(bureau.contains("icones::pour_kind(kind)"),
        "le bureau doit traduire le type d'application en indice de PNG");
    assert!(!bureau.contains("icones::dessine(i,"),
        "la position du bureau ne doit jamais etre un indice de PNG");
    let mapping = include_str!("../../src/gui/icones.rs");
    assert!(mapping.contains("crate::gui::window::KIND_DOOM => Some(11)"),
        "DOOM doit rester associe a sa tuile 11");
}

#[test]
fn le_logo_et_le_symbole_se_decodent() {
    for (octets, c) in [(LOGO_18, 18), (LOGO_72, 72), (GRILLE_22, 22)] {
        let i = png::decode(octets).unwrap();
        assert_eq!((i.largeur, i.hauteur), (c, c));
        assert!(i.pixels.iter().any(|p| p >> 24 == 0));
    }
    // Un symbole est un MASQUE blanc : sa couleur est uniforme, seul l'alpha varie.
    let g = png::decode(GRILLE_22).unwrap();
    for p in g.pixels.iter().filter(|p| **p >> 24 > 200) {
        assert_eq!(p & 0xffffff, 0xffffff, "masque non blanc : {p:#x}");
    }
}

#[test]
fn un_asset_abime_est_refuse_sans_panique() {
    // Tronque a chaque longueur caracteristique.
    for coupe in [0usize, 7, 8, 33, 100, LOGO_72.len() / 2, LOGO_72.len() - 1] {
        let r = png::decode(&LOGO_72[..coupe]);
        assert!(r.is_none(), "tronque a {coupe} : accepte");
    }
    // Un octet inverse au milieu : refuse, ou decode a la meme taille -- jamais
    // une panique ni une image d'une autre taille.
    for position in [40usize, LOGO_72.len() / 3, LOGO_72.len() / 2] {
        let mut abime = LOGO_72.to_vec();
        abime[position] ^= 0x5a;
        if let Some(i) = png::decode(&abime) {
            assert_eq!(i.pixels.len(), i.largeur * i.hauteur);
            assert!(i.largeur == 72 && i.hauteur == 72, "taille changee par une corruption");
        }
    }
}

#[test]
fn couvre_garde_l_image_a_taille_egale() {
    let src: Vec<u32> = (0..64u32 * 36).map(|i| i.wrapping_mul(2654435761) & 0xffffff).collect();
    let sortie = echelle::couvre(&src, 64, 36, 64, 36);
    assert_eq!(sortie, src);
}

#[test]
fn couvre_remplit_sans_deformer() {
    // Source 16:9 avec une bande verticale rouge au centre ; ecran 16:10.
    let (sl, sh) = (192usize, 108usize);
    let mut src = vec![0x0000ffu32; sl * sh];
    for y in 0..sh { for x in 94..98 { src[y * sl + x] = 0xff0000; } }
    let (l, h) = (256usize, 160usize);
    let sortie = echelle::couvre(&src, sl, sh, l, h);
    assert_eq!(sortie.len(), l * h);
    // La bande reste au centre et verticale : pas d'etirement lateral.
    let rouge = |x: usize, y: usize| (sortie[y * l + x] >> 16) & 0xff > 0x80;
    for y in [0, h / 2, h - 1] {
        assert!(rouge(l / 2, y) || rouge(l / 2 - 1, y), "bande perdue au centre, ligne {y}");
        assert!(!rouge(l / 4, y) && !rouge(3 * l / 4, y));
    }
    // Aucun alpha transporte.
    assert!(sortie.iter().all(|p| p >> 24 == 0));
}

#[test]
fn couvre_ne_panique_pas_sur_des_entrees_degenerees() {
    assert_eq!(echelle::couvre(&[], 0, 0, 10, 10).len(), 100);
    assert_eq!(echelle::couvre(&[1, 2, 3], 2, 2, 4, 4).len(), 16);
    assert!(echelle::couvre(&[1], 1, 1, 0, 0).is_empty());
    let un = echelle::couvre(&[0x123456], 1, 1, 5, 3);
    assert!(un.iter().all(|p| *p == 0x123456));
}
