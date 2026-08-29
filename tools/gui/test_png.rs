//! Le decodeur PNG rend-il exactement l'image que le generateur a encodee ?
//!
//! # Pourquoi ce test existe
//!
//! Les icones du bureau sont des PNG embarques a la compilation. Personne ne
//! les regarde apres coup : si le decodeur se trompe d'un filtre de ligne, le
//! bureau affiche une icone froissee, et rien dans le journal ne le dit.
//!
//! La verification est pourtant facile a poser : le generateur d'assets sait
//! quels pixels il a voulus, et il peut les reencoder avec CHAQUE filtre de
//! ligne. Le decodeur doit rendre les memes octets a partir des cinq.
//!
//! C'est la propriete qui compte : un PNG n'a pas UNE representation, il en a
//! autant que de choix de filtres, et un encodeur reel les melange ligne par
//! ligne. Un decodeur qui ne gere que « None » decode parfaitement les fichiers
//! du generateur et rate tous les autres.
//!
//! # Les vecteurs
//!
//! `tools/gui/vecteurs-png/` contient, pour la meme image de reference, un
//! fichier par filtre, plus les variantes de type de couleur -- gris, RVB,
//! palette, et leurs formes alpha. Ils sont fabriques par
//! `tools/gui/fabrique-vecteurs-png.py`, avec `zlib` et rien d'autre.
//!
//! Lance par `tools/gui/test-png.sh`.

extern crate alloc;

// Le decodeur PNG appelle `crate::net::encoding::inflate` : le harnais recree
// ce chemin plutot que de modifier la source. Le code teste est exactement
// celui qui tourne sur la machine.
#[path = "../../src/net/encoding/inflate.rs"]
pub mod inflate_reel;

/// `inflate.rs` cite `super::brotli` dans `decode_content`, une fonction dont
/// PNG n'a que faire. Un module reduit a cette signature suffit a le faire
/// compiler, et rend explicite ce que ce test n'exerce pas.
pub mod brotli {
    pub fn decode(_body: &[u8]) -> Option<alloc::vec::Vec<u8>> {
        None
    }
}

pub mod net {
    pub mod encoding {
        pub use crate::inflate_reel as inflate;
    }
}

#[path = "../../src/gui/png.rs"]
mod png;

/// L'image de reference : un damier avec un degrade d'alpha, choisi pour que
/// chaque filtre de ligne ait quelque chose a predire -- une image unie serait
/// decodee correctement meme par un defiltrage faux.
fn reference(cote: usize) -> alloc::vec::Vec<u32> {
    let mut pixels = alloc::vec::Vec::with_capacity(cote * cote);
    for y in 0..cote {
        for x in 0..cote {
            let r = (x * 255 / cote.max(1)) as u32;
            let v = (y * 255 / cote.max(1)) as u32;
            let b = if (x / 3 + y / 3) % 2 == 0 { 0xd0u32 } else { 0x20 };
            let a = (((x + y) * 255) / (2 * cote.max(1))) as u32;
            pixels.push((a << 24) | (r << 16) | (v << 8) | b);
        }
    }
    pixels
}

fn charge(nom: &str) -> alloc::vec::Vec<u8> {
    // Les vecteurs sont embarques : le binaire de test n'a pas de repertoire
    // courant garanti.
    match nom {
        "filtre0.png" => include_bytes!("vecteurs-png/filtre0.png").to_vec(),
        "filtre1.png" => include_bytes!("vecteurs-png/filtre1.png").to_vec(),
        "filtre2.png" => include_bytes!("vecteurs-png/filtre2.png").to_vec(),
        "filtre3.png" => include_bytes!("vecteurs-png/filtre3.png").to_vec(),
        "filtre4.png" => include_bytes!("vecteurs-png/filtre4.png").to_vec(),
        "melange.png" => include_bytes!("vecteurs-png/melange.png").to_vec(),
        "rvb.png" => include_bytes!("vecteurs-png/rvb.png").to_vec(),
        "gris.png" => include_bytes!("vecteurs-png/gris.png").to_vec(),
        "gris-alpha.png" => include_bytes!("vecteurs-png/gris-alpha.png").to_vec(),
        "palette.png" => include_bytes!("vecteurs-png/palette.png").to_vec(),
        _ => alloc::vec::Vec::new(),
    }
}

const COTE: usize = 24;

// ─── L'equivalence, filtre par filtre ──────────────────────────────────────

/// LA propriete : les cinq filtres de ligne decrivent la meme image.
#[test]
fn les_cinq_filtres_rendent_la_meme_image() {
    let attendu = reference(COTE);
    for nom in ["filtre0.png", "filtre1.png", "filtre2.png", "filtre3.png",
                "filtre4.png"] {
        let image = png::decode(&charge(nom)).unwrap_or_else(|| {
            panic!("{nom} refuse par le decodeur")
        });
        assert_eq!(image.largeur, COTE, "{nom}");
        assert_eq!(image.hauteur, COTE, "{nom}");
        assert_eq!(image.pixels, attendu, "{nom} : pixels differents");
    }
}

/// LES EGALITES DU PREDICTEUR DE PAETH.
///
/// Le damier de reference n'en produit aucune : departager `pa`, `pb` et `pc`
/// dans le mauvais ordre -- `<` au lieu de `<=` -- le decodait parfaitement.
/// Or c'est exactement le genre d'erreur qui froisse une vraie photo sans rien
/// casser d'autre, et le filtre 4 est celui que tout encodeur choisit le plus
/// souvent.
///
/// Ce vecteur est du bruit pseudo-aleatoire deterministe, encode entierement en
/// Paeth. Il en produit des milliers, et sa reference est le fichier d'octets
/// que le generateur a ecrit a cote.
#[test]
fn les_egalites_de_paeth_sont_departagees_dans_le_bon_ordre() {
    const OCTETS: &[u8] = include_bytes!("vecteurs-png/paeth.txt");
    let image = png::decode(include_bytes!("vecteurs-png/paeth.png"))
        .expect("paeth refuse");
    assert_eq!(image.pixels.len() * 4, OCTETS.len(), "taille du vecteur");
    for (index, pixel) in image.pixels.iter().enumerate() {
        let attendu = &OCTETS[index * 4..index * 4 + 4];
        let obtenu = [
            ((pixel >> 16) & 0xff) as u8,
            ((pixel >> 8) & 0xff) as u8,
            (pixel & 0xff) as u8,
            (pixel >> 24) as u8,
        ];
        assert_eq!(obtenu, attendu, "pixel {index}");
    }
}

/// Un encodeur reel choisit son filtre LIGNE PAR LIGNE. Un decodeur qui n'en
/// gere qu'un passerait les cinq tests precedents et raterait tout fichier
/// produit par un vrai encodeur.
#[test]
fn un_fichier_melangeant_les_filtres_se_decode_aussi() {
    let image = png::decode(&charge("melange.png")).expect("melange refuse");
    assert_eq!(image.pixels, reference(COTE));
}

// ─── Les types de couleur ──────────────────────────────────────────────────

/// RVB sans alpha : chaque pixel doit ressortir completement opaque.
#[test]
fn le_rvb_sans_alpha_est_opaque() {
    let image = png::decode(&charge("rvb.png")).expect("rvb refuse");
    assert_eq!(image.largeur, COTE);
    for (index, pixel) in image.pixels.iter().enumerate() {
        assert_eq!(pixel >> 24, 255, "pixel {index} n'est pas opaque");
    }
    // Et la couleur doit etre celle de la reference, alpha mis a part.
    for (attendu, obtenu) in reference(COTE).iter().zip(image.pixels.iter()) {
        assert_eq!(attendu & 0x00ff_ffff, obtenu & 0x00ff_ffff);
    }
}

/// Niveaux de gris : les trois composantes doivent etre egales.
#[test]
fn le_gris_se_repete_sur_les_trois_composantes() {
    let image = png::decode(&charge("gris.png")).expect("gris refuse");
    for pixel in &image.pixels {
        let r = (pixel >> 16) & 0xff;
        let v = (pixel >> 8) & 0xff;
        let b = pixel & 0xff;
        assert_eq!((r, v), (v, b), "gris non uniforme : {pixel:08x}");
        assert_eq!(pixel >> 24, 255);
    }
}

/// Gris + alpha : deux octets par pixel, le second etant l'opacite.
#[test]
fn le_gris_alpha_garde_son_opacite() {
    let image = png::decode(&charge("gris-alpha.png")).expect("gris-alpha refuse");
    let attendu = reference(COTE);
    for (index, pixel) in image.pixels.iter().enumerate() {
        assert_eq!(pixel >> 24, attendu[index] >> 24, "alpha du pixel {index}");
    }
}

/// Palette + `tRNS` : c'est ce que produisent la plupart des outils pour une
/// image a peu de couleurs, et c'est le seul type ou une entree manquante rend
/// le fichier indecodable plutot que faux.
#[test]
fn la_palette_et_sa_transparence_sont_lues() {
    let image = png::decode(&charge("palette.png")).expect("palette refusee");
    assert_eq!(image.largeur, COTE);
    // La palette de reference alterne opaque et translucide : les deux doivent
    // se retrouver.
    assert!(image.pixels.iter().any(|p| p >> 24 == 255), "aucun pixel opaque");
    assert!(image.pixels.iter().any(|p| p >> 24 < 255), "aucun pixel translucide");
}

// ─── Les refus ─────────────────────────────────────────────────────────────

/// Un fichier tronque, une signature fausse, un en-tete absurde : le decodeur
/// doit rendre `None`, jamais paniquer. Il tourne dans le noyau.
#[test]
fn un_fichier_abime_est_refuse_sans_panique() {
    let bon = charge("filtre0.png");
    assert!(png::decode(&[]).is_none(), "vide");
    assert!(png::decode(b"pas un png du tout").is_none(), "signature");
    for coupe in [8usize, 16, 32, 64, bon.len() / 2, bon.len() - 1] {
        if coupe < bon.len() {
            // Tronque : peut rendre None, ne doit jamais paniquer.
            let _ = png::decode(&bon[..coupe]);
        }
    }
    // Signature bonne, en-tete absurde.
    let mut faux = bon.clone();
    faux[16] = 0xff;
    faux[17] = 0xff;
    let _ = png::decode(&faux);
}

/// Une variante non geree doit etre REFUSEE, pas decodee a moitie : une image
/// entrelacee dont on ignore l'entrelacement est du bruit.
#[test]
fn une_variante_non_geree_est_refusee() {
    let mut entrelace = charge("filtre0.png");
    // Octet 12 de IHDR : l'entrelacement. Le CRC devient faux, ce que ce
    // decodeur ne verifie pas -- c'est justement le champ qu'on teste.
    entrelace[28] = 1;
    assert!(png::decode(&entrelace).is_none(), "Adam7 doit etre refuse");

    let mut seize_bits = charge("filtre0.png");
    seize_bits[24] = 16; // profondeur
    assert!(png::decode(&seize_bits).is_none(), "16 bits doit etre refuse");
}

// ─── Les vraies icones ─────────────────────────────────────────────────────

/// Les cinq images que le noyau embarque doivent se decoder, faire la taille
/// annoncee, et porter de la transparence : une icone sans alpha aurait un
/// carre de fond sur le bureau.
#[test]
fn les_icones_du_bureau_se_decodent() {
    let icones: [(&str, &[u8]); 5] = [
        ("ladybird", include_bytes!("../../src/assets/icons/ladybird.png")),
        ("calculatrice", include_bytes!("../../src/assets/icons/calculatrice.png")),
        ("terminal", include_bytes!("../../src/assets/icons/terminal.png")),
        ("fichiers", include_bytes!("../../src/assets/icons/fichiers.png")),
        ("rustpad", include_bytes!("../../src/assets/icons/rustpad.png")),
    ];
    for (nom, octets) in icones {
        let image = png::decode(octets)
            .unwrap_or_else(|| panic!("{nom} : le noyau ne saurait pas la decoder"));
        assert_eq!((image.largeur, image.hauteur), (128, 128), "{nom}");
        assert_eq!(image.pixels.len(), 128 * 128, "{nom}");
        assert!(
            image.pixels.iter().any(|p| p >> 24 == 0),
            "{nom} : aucun pixel transparent, l'icone aurait un fond carre"
        );
        assert!(
            image.pixels.iter().any(|p| p >> 24 == 255),
            "{nom} : aucun pixel opaque, l'icone serait invisible"
        );
    }
}
