//! Contrats purs du design system Atelier (BOUCHAUD_ATELIER_JETONS_V1,
//! BOUCHAUD_ATELIER_MOUVEMENT_V1, BOUCHAUD_ATELIER_DOCK_V1).
//!
//! Inclut les modules TELS QUELS : ce sont les memes octets que le noyau
//! compile.

#![allow(dead_code)]

extern crate alloc;

#[path = "../../src/gui/atelier/jetons.rs"]
mod jetons;
#[path = "../../src/gui/atelier/mouvement.rs"]
mod mouvement;
#[path = "../../src/gui/atelier/dock.rs"]
mod dock;
#[path = "../../src/gui/protocole.rs"]
mod protocole;
#[path = "../../src/gui/disposition.rs"]
mod disposition;

use mouvement::{applique, melange_couleur, Courbe, Interpolation, PLEIN};

const COURBES: [Courbe; 3] = [Courbe::Lineaire, Courbe::SortieCubique, Courbe::EntreeSortieCubique];

// ─── Mouvement ────────────────────────────────────────────────────────────

#[test]
fn chaque_courbe_part_de_zero_et_arrive_a_plein() {
    for c in COURBES {
        assert_eq!(applique(c, 0), 0, "{c:?}");
        assert_eq!(applique(c, PLEIN), PLEIN, "{c:?}");
        assert_eq!(applique(c, PLEIN + 500), PLEIN, "{c:?} bornee au-dela");
    }
}

#[test]
fn chaque_courbe_est_monotone() {
    for c in COURBES {
        let mut avant = 0;
        for p in 0..=PLEIN {
            let v = applique(c, p);
            assert!(v >= avant, "{c:?} recule a p={p}: {v} < {avant}");
            assert!(v <= PLEIN);
            avant = v;
        }
    }
}

#[test]
fn la_sortie_cubique_part_vite() {
    // Au quart du temps, une transition d'interface a deja fait plus de la
    // moitie du chemin : c'est ce qui la rend vive sans etre brusque.
    assert!(applique(Courbe::SortieCubique, 250) > 500);
    assert!(applique(Courbe::SortieCubique, 500) > applique(Courbe::Lineaire, 500));
}

#[test]
fn une_interpolation_finie_vaut_exactement_l_arrivee() {
    for c in COURBES {
        let i = Interpolation::nouvelle(-37, 211, 1_000, jetons::DUREE_STANDARD_MS, c);
        assert_eq!(i.valeur(0), -37, "avant le debut");
        assert_eq!(i.valeur(1_000), -37, "au debut");
        assert!(!i.finie(1_199));
        assert!(i.finie(1_200));
        assert_eq!(i.valeur(1_200), 211);
        assert_eq!(i.valeur(u64::MAX / 2), 211);
        // Entre les deux, jamais hors de l'intervalle.
        for t in 1_000..=1_200 {
            let v = i.valeur(t);
            assert!((-37..=211).contains(&v), "{c:?} t={t} v={v}");
        }
    }
}

#[test]
fn une_duree_nulle_est_finie_tout_de_suite() {
    let i = Interpolation::nouvelle(0, 10, 50, 0, Courbe::SortieCubique);
    assert!(i.finie(0));
    assert_eq!(i.valeur(0), 10);
}

#[test]
fn aucune_duree_ne_depasse_le_plafond() {
    for d in [jetons::DUREE_RAPIDE_MS, jetons::DUREE_STANDARD_MS, jetons::DUREE_ENTREE_MS] {
        assert!(d <= jetons::DUREE_MAX_MS);
    }
}

#[test]
fn le_melange_de_couleurs_tient_ses_extremites() {
    let (a, b) = (0x112233, 0xddeeff);
    assert_eq!(melange_couleur(a, b, 0), a);
    assert_eq!(melange_couleur(a, b, PLEIN), b);
    let m = melange_couleur(0x000000, 0xffffff, 500);
    for d in [0, 8, 16] {
        let canal = (m >> d) & 0xff;
        assert!((127..=128).contains(&canal), "canal {d}: {canal:#x}");
    }
}

// ─── Dock ─────────────────────────────────────────────────────────────────

const ECRANS: [(u32, u32); 6] = [(640, 480), (800, 600), (1280, 720), (1366, 768),
    (1920, 1080), (3840, 2160)];

#[test]
fn la_zone_du_dock_est_celle_de_la_disposition() {
    assert_eq!(dock::HAUTEUR_ZONE, disposition::HAUTEUR_ZONE_DOCK);
    assert_eq!(dock::OMBRE as u32, disposition::DEBORD_OMBRE);
}

#[test]
fn le_dock_et_son_ombre_tiennent_dans_la_bande_du_bas() {
    for (l, h) in ECRANS {
        let bande = disposition::barre_taches(l, h);
        for n in 0..12 {
            let d = dock::cadre(l, h, n);
            assert!(d.y - dock::OMBRE >= bande.y, "{l}x{h} n={n}: ombre hors bande");
            assert_eq!(d.y + d.h + dock::MARGE_BAS, h as i32, "{l}x{h}: posé a MARGE_BAS");
            assert!(d.x >= 0 && d.x + d.w <= l as i32, "{l}x{h} n={n}: hors ecran");
        }
    }
}

#[test]
fn le_dock_est_centre() {
    for (l, h) in ECRANS {
        for n in 0..8 {
            let d = dock::cadre(l, h, n);
            let gauche = d.x;
            let droite = l as i32 - (d.x + d.w);
            assert!((gauche - droite).abs() <= 1, "{l}x{h} n={n}: {gauche} vs {droite}");
        }
    }
}

#[test]
fn les_cellules_sont_dans_le_dock_et_ne_se_chevauchent_pas() {
    for (l, h) in ECRANS {
        for n in 0..10 {
            let d = dock::cadre(l, h, n);
            let apps = dock::cellule_applications(l, h, n);
            let dedans = |c: dock::Boite| c.x >= d.x && c.y >= d.y
                && c.x + c.w <= d.x + d.w && c.y + c.h <= d.y + d.h;
            assert!(dedans(apps));
            let mut precedente = apps;
            for i in 0..dock::visibles(l, n) {
                let c = dock::cellule_fenetre(l, h, n, i);
                assert!(dedans(c), "{l}x{h} n={n} i={i}");
                assert!(c.x >= precedente.x + precedente.w, "{l}x{h} n={n} i={i} chevauche");
                precedente = c;
            }
            // Le separateur passe entre Applications et la premiere fenetre.
            if dock::visibles(l, n) > 0 {
                let sep = dock::x_separateur(l, h, n);
                assert!(sep >= apps.x + apps.w);
                assert!(sep < dock::cellule_fenetre(l, h, n, 0).x);
            }
        }
    }
}

#[test]
fn une_fenetre_sans_place_n_a_pas_de_cellule() {
    let (l, h) = (640, 480);
    let cap = dock::capacite(l);
    let n = cap + 3;
    for i in cap..n {
        let c = dock::cellule_fenetre(l, h, n, i);
        assert_eq!(c.w, 0);
        assert!(!c.contient(c.x, c.y), "une cellule vide n'est jamais touchee");
    }
}

#[test]
fn le_test_d_impact_suit_la_cellule() {
    let (l, h) = (1280, 720);
    let c = dock::cellule_fenetre(l, h, 3, 1);
    assert!(c.contient(c.x, c.y));
    assert!(c.contient(c.x + c.w - 1, c.y + c.h - 1));
    assert!(!c.contient(c.x + c.w, c.y));
    assert!(!c.contient(c.x - 1, c.y));
}
