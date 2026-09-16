//! Le rectangle arrondi, et la panique qu'il a causee.
//!
//! ```text
//! *** KERNEL PANIC *** cpu=0
//! panicked at src/gui/apps/file_explorer.rs:163:17:
//! min > max. min = 447, max = 446
//! ```
//!
//! Ouvrir le gestionnaire de fichiers tuait le noyau. Ces tests existent pour
//! qu'aucune combinaison de boite et de rayon ne puisse plus le faire, et ils
//! rejouent les constantes REELLES des icones : c'est la seule facon d'etre
//! sur que le cas fautif est couvert et non seulement son voisinage.

#[path = "../../src/gui/geometrie.rs"]
mod geometrie;

use geometrie::{dans_arrondi, rayon_utilisable};

// ---------------------------------------------------------------------------
// Le cas exact du releve
// ---------------------------------------------------------------------------

#[test]
fn la_ligne_interne_de_l_icone_de_fichier_ne_panique_plus() {
    // dans_arrondi(nx, ny, 285, haut, 735, haut + 34, 17), haut = 430.
    // L'ancienne version calculait clamp(447, 446) et paniquait.
    for ligne in 0..4i32 {
        let haut = 430 + ligne * 105;
        let droite = if ligne == 3 { 650 } else { 735 };
        for ny in haut..haut + 34 {
            for nx in (285..droite).step_by(7) {
                let _ = dans_arrondi(nx, ny, 285, haut, droite, haut + 34, 17);
            }
        }
    }
}

#[test]
fn une_boite_dont_la_hauteur_vaut_deux_fois_le_rayon_est_un_stade() {
    // Le cas degenere : plus de partie droite entre les deux arcs. La forme la
    // plus proche qui ait un sens est un stade, et le milieu doit en etre.
    assert!(dans_arrondi(500, 447, 285, 430, 735, 464, 17));
    // Un coin, lui, reste dehors.
    assert!(!dans_arrondi(285, 430, 285, 430, 735, 464, 17));
}

// ---------------------------------------------------------------------------
// Totalite : aucune entree ne panique
// ---------------------------------------------------------------------------

#[test]
fn aucune_boite_ni_aucun_rayon_ne_peut_faire_paniquer() {
    // Le balayage que l'ancienne version n'aurait pas survecu.
    for largeur in 0..40i32 {
        for hauteur in 0..40i32 {
            for rayon in 0..40i32 {
                for py in -1..hauteur + 1 {
                    for px in -1..largeur + 1 {
                        let _ = dans_arrondi(px, py, 0, 0, largeur, hauteur, rayon);
                    }
                }
            }
        }
    }
}

#[test]
fn une_boite_vide_ou_inversee_ne_contient_rien() {
    assert!(!dans_arrondi(0, 0, 0, 0, 0, 0, 5));
    assert!(!dans_arrondi(0, 0, 10, 10, 0, 0, 5));
    assert!(!dans_arrondi(5, 5, 10, 10, 0, 0, 5));
}

#[test]
fn un_rayon_negatif_est_traite_comme_nul() {
    // Le rectangle plein, sans coins retires.
    assert!(dans_arrondi(0, 0, 0, 0, 10, 10, -7));
    assert!(dans_arrondi(9, 9, 0, 0, 10, 10, -7));
}

#[test]
fn un_rayon_enorme_donne_la_forme_la_plus_proche_qui_ait_un_sens() {
    // Pas de panique, et les coins restent retires.
    assert!(dans_arrondi(5, 5, 0, 0, 11, 11, 10_000));
    assert!(!dans_arrondi(0, 0, 0, 0, 11, 11, 10_000));
}

// ---------------------------------------------------------------------------
// Le rayon utilisable
// ---------------------------------------------------------------------------

#[test]
fn le_rayon_ne_depasse_jamais_la_moitie_de_la_boite() {
    for largeur in 1..60i32 {
        for hauteur in 1..60i32 {
            let r = rayon_utilisable(largeur, hauteur, 1000);
            // La borne qui compte : les centres d'arc doivent rester ordonnes.
            assert!(0 + r <= largeur - r - 1, "largeur={} r={}", largeur, r);
            assert!(0 + r <= hauteur - r - 1, "hauteur={} r={}", hauteur, r);
        }
    }
}

#[test]
fn la_borne_est_bien_moitie_moins_un_et_pas_moitie() {
    // `34 / 2` rendrait encore 17, et la panique reviendrait telle quelle.
    assert_eq!(rayon_utilisable(450, 34, 17), 16);
    assert_ne!(rayon_utilisable(450, 34, 17), 17);
}

#[test]
fn un_rayon_qui_tient_deja_n_est_pas_rabote() {
    // Les autres formes des icones doivent garder leur rayon exact.
    assert_eq!(rayon_utilisable(954 - 70, 910 - 260, 72), 72);
    assert_eq!(rayon_utilisable(535 - 90, 420 - 150, 62), 62);
    assert_eq!(rayon_utilisable(914 - 110, 405 - 320, 36), 36);
    assert_eq!(rayon_utilisable(835 - 175, 950 - 70, 58), 58);
    // Et le logo de demarrage, l'autre appelant.
    assert_eq!(rayon_utilisable(345 - 145, 950 - 70, 72), 72);
}

#[test]
fn une_boite_degeneree_n_a_pas_de_coins() {
    assert_eq!(rayon_utilisable(0, 10, 5), 0);
    assert_eq!(rayon_utilisable(10, 0, 5), 0);
    assert_eq!(rayon_utilisable(-3, 10, 5), 0);
}

// ---------------------------------------------------------------------------
// Toutes les icones, a toutes les tailles que le bureau emploie
// ---------------------------------------------------------------------------

/// La transcription exacte de `couverture_icone`, formes comprises.
fn couverture(px: usize, py: usize, size: usize, forme: u8) -> u8 {
    let mut compte = 0u16;
    for sy in 0..2 {
        for sx in 0..2 {
            let nx = ((px * 2 + sx) * 1024 / (size * 2).max(1)) as i32;
            let ny = ((py * 2 + sy) * 1024 / (size * 2).max(1)) as i32;
            let dedans = match forme {
                0 => dans_arrondi(nx, ny, 70, 260, 954, 910, 72)
                    || dans_arrondi(nx, ny, 90, 150, 535, 420, 62),
                1 => dans_arrondi(nx, ny, 110, 320, 914, 405, 36),
                2 => dans_arrondi(nx, ny, 175, 70, 835, 950, 58)
                    && !(nx > 630 && ny < 275 && nx - ny > 545),
                3 => nx >= 630 && nx <= 835 && ny >= 70 && ny <= 275 && nx - 630 >= ny - 70,
                _ => (0..4).any(|ligne| {
                    let haut = 430 + ligne * 105;
                    dans_arrondi(
                        nx, ny, 285, haut,
                        if ligne == 3 { 650 } else { 735 },
                        haut + 34, 17,
                    )
                }),
            };
            if dedans {
                compte += 1;
            }
        }
    }
    (compte * 255 / 4) as u8
}

#[test]
fn chaque_forme_d_icone_se_rasterise_sans_paniquer() {
    // C'est le test qui aurait attrape la panique du 16 septembre : la forme 4
    // -- les lignes internes du fichier -- a toutes les tailles employees.
    for size in [8usize, 12, 16, 24, 32, 48, 64, 96, 128] {
        for forme in 0..5u8 {
            for py in 0..size {
                for px in 0..size {
                    let _ = couverture(px, py, size, forme);
                }
            }
        }
    }
}

#[test]
fn une_icone_de_taille_nulle_ou_minuscule_ne_panique_pas() {
    // `(size * 2).max(1)` protege la division ; le reste doit suivre.
    for size in [0usize, 1, 2, 3] {
        for forme in 0..5u8 {
            for py in 0..size.max(1) {
                for px in 0..size.max(1) {
                    let _ = couverture(px, py, size, forme);
                }
            }
        }
    }
}

#[test]
fn les_lignes_internes_du_fichier_dessinent_bien_quelque_chose() {
    // Ne pas paniquer ne suffit pas : une forme devenue vide passerait ce
    // test-la sans rien dessiner, et l'icone perdrait ses lignes en silence.
    let size = 48usize;
    let mut poses = 0usize;
    for py in 0..size {
        for px in 0..size {
            if couverture(px, py, size, 4) != 0 {
                poses += 1;
            }
        }
    }
    assert!(poses > 20, "les lignes internes ne couvrent que {} pixels", poses);
}
