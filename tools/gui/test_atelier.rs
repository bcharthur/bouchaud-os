//! Contrats purs du design system Atelier (BOUCHAUD_ATELIER_JETONS_V1,
//! BOUCHAUD_ATELIER_MOUVEMENT_V1, BOUCHAUD_ATELIER_DOCK_V1,
//! BOUCHAUD_ATELIER_CENTRE_V1, BOUCHAUD_ATELIER_ANIMATIONS_V1).
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
#[path = "../../src/gui/atelier/centre.rs"]
mod centre;
#[path = "../../src/gui/atelier/animations.rs"]
mod animations;
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

// ─── Centre de controle ───────────────────────────────────────────────────

const ECRANS_CENTRE: [(u32, u32); 3] = [(1280, 720), (1920, 1080), (2560, 1600)];

fn dans(petite: dock::Boite, grande: dock::Boite) -> bool {
    petite.x >= grande.x && petite.y >= grande.y
        && petite.x + petite.w <= grande.x + grande.w
        && petite.y + petite.h <= grande.y + grande.h
}

#[test]
fn le_centre_tient_entre_la_barre_et_le_dock() {
    for (l, h) in ECRANS_CENTRE {
        let c = centre::cadre(l, disposition::HAUTEUR_BARRE);
        assert!(c.x >= 0 && c.x + c.w <= l as i32, "{l}x{h}");
        assert!(c.y > disposition::HAUTEUR_BARRE as i32);
        assert!(c.y + c.h + centre::OMBRE < h as i32 - disposition::HAUTEUR_ZONE_DOCK as i32,
            "{l}x{h} : le centre deborde sur le dock");
    }
}

#[test]
fn l_empreinte_contient_le_panneau_et_son_ombre() {
    for (l, _) in ECRANS_CENTRE {
        let c = centre::cadre(l, 30);
        let e = centre::empreinte(l, 30);
        assert!(dans(c, e));
        assert_eq!(e.x, c.x - centre::OMBRE);
        assert_eq!(e.y, c.y - centre::OMBRE);
        assert_eq!(e.w, c.w + 2 * centre::OMBRE);
        assert_eq!(e.h, c.h + 2 * centre::OMBRE);
    }
}

#[test]
fn chaque_action_est_dans_le_panneau_et_aucune_ne_se_chevauche() {
    for (l, _) in ECRANS_CENTRE {
        let c = centre::cadre(l, 30);
        let boites: Vec<dock::Boite> =
            centre::ACTIONS.iter().map(|a| centre::boite_action(l, 30, *a)).collect();
        for (i, b) in boites.iter().enumerate() {
            assert!(b.w > 0 && b.h > 0, "{:?} vide", centre::ACTIONS[i]);
            assert!(dans(*b, c), "{:?} hors du panneau", centre::ACTIONS[i]);
            for (k, o) in boites.iter().enumerate().skip(i + 1) {
                let chevauche = b.x < o.x + o.w && o.x < b.x + b.w
                    && b.y < o.y + o.h && o.y < b.y + b.h;
                assert!(!chevauche, "{:?} et {:?}", centre::ACTIONS[i], centre::ACTIONS[k]);
            }
        }
    }
}

#[test]
fn le_survol_et_le_clic_designent_la_boite_peinte() {
    // Chaque pixel de chaque boite designe SON action, et aucun pixel hors
    // des boites ne designe quoi que ce soit.
    let l = 1280;
    let c = centre::cadre(l, 30);
    for y in c.y - 2..c.y + c.h + 2 {
        for x in c.x - 2..c.x + c.w + 2 {
            let attendu = centre::ACTIONS.iter().copied()
                .find(|a| centre::boite_action(l, 30, *a).contient(x, y));
            assert_eq!(centre::action_sous(l, 30, x, y), attendu, "({x},{y})");
            if attendu.is_some() {
                assert!(centre::dans_panneau(l, 30, x, y));
            }
        }
    }
}

#[test]
fn les_blocs_se_suivent_sans_se_chevaucher() {
    let c = centre::cadre(1280, 30);
    let mut bas = c.y;
    for (bloc, h) in centre::BLOCS {
        let b = centre::bloc(1280, 30, bloc);
        assert_eq!(b.h, h as i32);
        assert!(b.y >= bas, "{bloc:?}");
        bas = b.y + b.h;
    }
    assert_eq!(bas + centre::REMBOURRAGE, c.y + c.h, "le dernier bloc ferme le panneau");
}

#[test]
fn le_bouton_de_la_barre_reste_dans_la_barre_et_a_droite() {
    for (l, _) in ECRANS_CENTRE {
        for grappe in [40u32, 180, 320] {
            let b = centre::bouton_barre(l, 30, grappe);
            assert!(b.y >= 0 && b.y + b.h <= 30);
            assert_eq!(b.x + b.w + centre::BOUTON_MARGE, l as i32);
            assert_eq!(b.w, grappe as i32 + 2 * centre::BOUTON_REMBOURRAGE);
        }
    }
}

// ─── Animations ───────────────────────────────────────────────────────────

use animations::{Animation, Mode, Nature, Registre, CAPACITE};

fn anim(cle: u64, nature: Nature, debut: u64, duree: u32) -> Animation {
    Animation { cle, nature, debut_ms: debut, duree_ms: duree, depart: 255,
        bornes: dock::Boite { x: 10, y: 20, w: 100, h: 80 } }
}

/// Simule la boucle du bureau a 16 ms : rend le nombre de tours ou une trame
/// aurait ete demandee par le registre.
fn trames_demandees(r: &mut Registre, de: u64, a: u64) -> u32 {
    let mut n = 0;
    let mut t = de;
    while t < a {
        let mut finies = 0;
        r.termine(t, |_| finies += 1);
        if r.actif() || finies > 0 {
            n += 1;
        }
        t += 16;
    }
    n
}

#[test]
fn un_bureau_immobile_ne_demande_aucune_trame() {
    let mut r = Registre::neuf();
    assert!(!r.actif());
    assert_eq!(trames_demandees(&mut r, 0, 60_000), 0, "une minute sans geste : zero trame");
}

#[test]
fn une_animation_demande_des_trames_le_temps_de_sa_duree_puis_plus_aucune() {
    let mut r = Registre::neuf();
    r.lance(anim(1, Nature::Ouverture, 1000, 200));
    let pendant = trames_demandees(&mut r, 1000, 1240);
    assert!(pendant >= 200 / 16 && pendant <= 200 / 16 + 2, "{pendant}");
    assert!(!r.actif());
    assert_eq!(trames_demandees(&mut r, 1240, 61_240), 0, "plus rien apres la fin");
}

#[test]
fn une_animation_finie_est_rendue_une_seule_fois_avec_sa_zone() {
    let mut r = Registre::neuf();
    r.lance(anim(7, Nature::Fermeture, 0, 150));
    let mut rendues = Vec::new();
    r.termine(149, |a| rendues.push(a));
    assert!(rendues.is_empty());
    r.termine(150, |a| rendues.push(a));
    r.termine(500, |a| rendues.push(a));
    assert_eq!(rendues.len(), 1);
    assert_eq!(rendues[0].bornes, dock::Boite { x: 10, y: 20, w: 100, h: 80 });
}

#[test]
fn le_registre_plein_evince_la_plus_ancienne_et_la_rend() {
    let mut r = Registre::neuf();
    for i in 0..CAPACITE as u64 {
        assert!(r.lance(anim(i, Nature::Ouverture, 100 + i, 200)).is_none());
    }
    let evincee = r.lance(anim(99, Nature::Ouverture, 500, 200)).expect("evincee");
    assert_eq!(evincee.cle, 0, "la plus ancienne");
    assert_eq!(r.nombre(), CAPACITE, "jamais plus que la capacite");
    for i in 0..1000u64 {
        let _ = r.lance(anim(1000 + i, Nature::Fermeture, 600 + i, 150));
        assert!(r.nombre() <= CAPACITE);
    }
}

#[test]
fn relancer_la_meme_cle_ne_double_pas_et_garde_l_ancienne_zone() {
    let mut r = Registre::neuf();
    r.lance(anim(3, Nature::Ouverture, 0, 200));
    let mut b = anim(3, Nature::Ouverture, 50, 200);
    b.bornes = dock::Boite { x: 300, y: 300, w: 10, h: 10 };
    assert!(r.lance(b).is_none());
    assert_eq!(r.nombre(), 1);
    let z = r.cherche(3, Nature::Ouverture).unwrap().bornes;
    assert!(z.contient(10, 20) && z.contient(309, 309), "union des deux zones");
}

#[test]
fn recouvrir_rend_l_ancienne_et_la_nouvelle_zone() {
    let mut r = Registre::neuf();
    r.lance(anim(4, Nature::Ouverture, 0, 200));
    let nouvelle = dock::Boite { x: 500, y: 400, w: 50, h: 50 };
    let total = r.recouvre(4, Nature::Ouverture, nouvelle).unwrap();
    assert!(total.contient(10, 20), "l'ancienne position est recomposee");
    assert!(total.contient(549, 449), "la nouvelle aussi");
    assert_eq!(r.cherche(4, Nature::Ouverture).unwrap().bornes, nouvelle);
    assert!(r.recouvre(5, Nature::Ouverture, nouvelle).is_none());
}

#[test]
fn annuler_une_ouverture_interrompue_la_retire() {
    let mut r = Registre::neuf();
    r.lance(anim(8, Nature::Ouverture, 0, 200));
    let a = r.annule(8, Nature::Ouverture).expect("annulee");
    assert_eq!(a.cle, 8);
    assert!(!r.actif());
    assert!(r.annule(8, Nature::Ouverture).is_none());
}

#[test]
fn les_courbes_d_animation_partent_et_arrivent_exactement() {
    let a = anim(1, Nature::Ouverture, 1000, 200);
    assert_eq!(a.progression(1000), 0);
    assert_eq!(a.progression(1200), PLEIN);
    assert_eq!(a.progression(5000), PLEIN);
    let mut avant = 0;
    for t in 1000..=1200 {
        let p = a.progression(t);
        assert!(p >= avant && p <= PLEIN);
        avant = p;
    }
    assert_eq!(animations::opacite(Nature::Ouverture, 0, 0), 0);
    assert_eq!(animations::opacite(Nature::Ouverture, 0, PLEIN), 255);
    assert_eq!(animations::opacite(Nature::Fermeture, 200, 0), 200);
    assert_eq!(animations::opacite(Nature::Fermeture, 200, PLEIN), 0);
    assert_eq!(animations::decalage(Mode::Normal, 0), animations::DECALAGE_OUVERTURE);
    assert_eq!(animations::decalage(Mode::Normal, PLEIN), 0);
    assert_eq!(animations::echelle(Mode::Normal, 0), PLEIN);
    assert_eq!(animations::echelle(Mode::Normal, PLEIN), PLEIN - animations::REDUCTION_FERMETURE);
}

#[test]
fn le_mode_reduit_ne_deplace_rien_et_le_mode_aucun_ne_lance_rien() {
    for p in [0, 300, 700, PLEIN] {
        assert_eq!(animations::decalage(Mode::Reduit, p), 0);
        assert_eq!(animations::echelle(Mode::Reduit, p), PLEIN);
        assert_eq!(animations::decalage(Mode::Aucun, p), 0);
    }
    for n in [Nature::Ouverture, Nature::Fermeture] {
        assert_eq!(animations::duree_ms(Mode::Aucun, n), 0);
        assert!(animations::duree_ms(Mode::Reduit, n) < animations::duree_ms(Mode::Normal, n));
        // Les durees de la mission : ouverture 160-220 ms, fermeture 120-180 ms.
    }
    assert!((160..=220).contains(&animations::duree_ms(Mode::Normal, Nature::Ouverture)));
    assert!((120..=180).contains(&animations::duree_ms(Mode::Normal, Nature::Fermeture)));
}

#[test]
fn une_animation_de_duree_nulle_est_finie_tout_de_suite() {
    let a = anim(1, Nature::Ouverture, 1000, 0);
    assert_eq!(a.progression(1000), PLEIN);
    let mut r = Registre::neuf();
    r.lance(a);
    let mut n = 0;
    r.termine(1000, |_| n += 1);
    assert_eq!(n, 1);
}

#[test]
fn les_segments_d_animation_sont_dans_leur_bloc() {
    for (l, _) in ECRANS_CENTRE {
        let bloc = centre::bloc(l, 30, centre::Bloc::Animations);
        for s in centre::SEGMENTS_ANIMATIONS {
            assert!(dans(centre::boite_action(l, 30, s), bloc), "{s:?}");
        }
    }
}

#[test]
fn une_animation_commence_a_sa_premiere_trame_et_non_a_son_lancement() {
    let mut r = Registre::neuf();
    r.lance(anim(5, Nature::Ouverture, animations::EN_ATTENTE, 200));
    // Une trame lente : 300 ms passent avant la premiere composition.
    let mut finies = 0;
    r.termine(300, |_| finies += 1);
    assert_eq!(finies, 0, "une animation jamais montree n'est pas finie");
    assert_eq!(r.cherche(5, Nature::Ouverture).unwrap().progression(300), 0);
    r.demarre(300);
    let a = r.cherche(5, Nature::Ouverture).unwrap();
    assert_eq!(a.debut_ms, 300);
    assert_eq!(a.progression(300), 0);
    assert_eq!(a.progression(500), PLEIN);
    r.demarre(400);
    assert_eq!(r.cherche(5, Nature::Ouverture).unwrap().debut_ms, 300, "datee une seule fois");
    r.termine(500, |_| finies += 1);
    assert_eq!(finies, 1);
}
