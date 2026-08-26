//! Harnais de test hote pour la politique de degat du bureau.
//!
//! Meme principe que `test_protocole.rs` et `test_clavier.rs` : ce qu'un
//! evenement salit est de la geometrie pure -- des rectangles et une union --
//! donc exercable sans QEMU. `src/gui/degats.rs` est inclus tel quel.
//!
//! Ce que ces tests protegent est une regle qui ne se lit dans aucun journal :
//!
//!     cent clics et cent crans de molette dans une page ne doivent produire
//!     AUCUN degat plein ecran.
//!
//! Elle se demontre ; elle ne s'observe pas. Un journal ne montre que la
//! session qu'on a jouee.
//!
//! Lance par `tools/gui/test-degats.sh`.

#![allow(dead_code)]

extern crate alloc;

// `degats.rs` ecrit `crate::gui::protocole::Rect`. Le harnais recree ce chemin
// plutot que de modifier la source : le code teste reste exactement celui qui
// tourne sur la machine.
#[path = "../../src/gui"]
mod gui {
    pub mod protocole;
    pub mod degats;
}

use gui::degats::{degats_plein_ecran, remise_a_zero, Degats, Origine};
use gui::protocole::Rect;

const LARGEUR: u32 = 1280;
const HAUTEUR: u32 = 720;

fn ecran() -> Rect {
    Rect::neuf(0, 0, LARGEUR, HAUTEUR)
}

/// Empreinte du curseur, telle que le bureau la calcule.
fn curseur(x: i32, y: i32) -> Rect {
    Rect::neuf(x, y, 14, 22)
}

/// A + B : une session de cent clics et cent crans dans une page ne produit
/// aucun degat plein ecran.
///
/// # Ce que ce test est, et ce qu'il n'est pas
///
/// C'est un MODELE de la boucle du bureau, pas la boucle elle-meme : celle-ci
/// tient le framebuffer, le RAMFS et l'ordonnanceur, et ne se compile pas sur
/// l'hote. Le modele rejoue les seules contributions que la boucle s'autorise
/// pour ces evenements -- le curseur qui bouge, le degat que le client
/// annonce -- et verifie qu'aucune n'atteint le plein ecran.
///
/// Ce qu'il ne peut pas garantir, c'est qu'un futur `degats.tout()` ne soit pas
/// ajoute dans la branche cliente de `handle_click` ou `handle_wheel`. Cette
/// moitie-la se relit dans le diff, et se mesure au runtime avec
/// `[GUI-DAMAGE] full=`.
#[test]
fn cent_clics_et_cent_crans_dans_un_client_ne_font_aucun_plein_ecran() {
    remise_a_zero();
    let mut degats = Degats::neuf(ecran());
    let mut x = 300i32;

    for tour in 0..100 {
        // Le pointeur se deplace : deux empreintes de curseur.
        degats.ajoute(Origine::Curseur, curseur(x, 200));
        x += 1;
        degats.ajoute(Origine::Curseur, curseur(x, 200));

        // Le clic et le cran partent au client : le bureau n'ajoute rien.

        // De temps en temps le client annonce qu'il a repeint sa page.
        if tour % 10 == 0 {
            degats.ajoute(Origine::Client, Rect::neuf(120, 60, 1000, 560));
        }
    }

    assert_eq!(degats_plein_ecran(), 0, "aucun degat plein ecran");

    // Et la region accumulee reste bornee par la fenetre cliente : elle ne
    // s'etale pas jusqu'a la barre des taches ni jusqu'au bord de l'ecran.
    let region = degats.region();
    assert!(region.bas() <= 620, "region descend jusqu'a {}", region.bas());
}

/// B' : un clic qui REMONTE une fenetre salit cette fenetre et la barre,
/// jamais tout l'ecran.
#[test]
fn remonter_une_fenetre_salit_la_fenetre_et_la_barre_seulement() {
    remise_a_zero();
    let mut degats = Degats::neuf(ecran());
    let fenetre = Rect::neuf(100, 80, 400, 300);
    let barre = Rect::neuf(0, (HAUTEUR - 28) as i32, LARGEUR, 28);

    degats.ajoute(Origine::Fenetre, fenetre);
    degats.ajoute(Origine::BarreTaches, barre);

    assert_eq!(degats_plein_ecran(), 0, "remonter n'est pas un plein ecran");
    let region = degats.region();
    // La region englobe les deux, et rien de plus : elle part du coin de la
    // fenetre et descend jusqu'a la barre.
    assert_eq!(region.x, 0);
    assert_eq!(region.y, 80);
    assert_eq!(region.droite(), LARGEUR as i64);
    assert_eq!(region.bas(), HAUTEUR as i64);
}

/// C : un deplacement de curseur ne salit que l'ancienne et la nouvelle
/// empreinte.
#[test]
fn le_curseur_ne_salit_que_ses_deux_empreintes() {
    remise_a_zero();
    let mut degats = Degats::neuf(ecran());
    degats.ajoute(Origine::Curseur, curseur(200, 200));
    degats.ajoute(Origine::Curseur, curseur(210, 205));

    let region = degats.region();
    assert_eq!(region.x, 200);
    assert_eq!(region.y, 200);
    // 210 + 14 = 224 ; 205 + 22 = 227.
    assert_eq!(region.droite(), 224);
    assert_eq!(region.bas(), 227);
    assert_eq!(degats_plein_ecran(), 0);

    // Et surtout : la region reste minuscule devant l'ecran. C'est la
    // propriete qui compte, pas les bornes exactes.
    let aire = region.largeur as u64 * region.hauteur as u64;
    let aire_ecran = LARGEUR as u64 * HAUTEUR as u64;
    assert!(aire * 100 < aire_ecran, "curseur = {} px sur {}", aire, aire_ecran);
}

/// D : deplacer une fenetre salit l'union de l'ancienne et de la nouvelle
/// position, et rien d'autre.
#[test]
fn deplacer_une_fenetre_salit_l_union_des_deux_positions() {
    remise_a_zero();
    let mut degats = Degats::neuf(ecran());
    let avant = Rect::neuf(100, 100, 200, 150);
    let apres = Rect::neuf(140, 130, 200, 150);
    degats.ajoute(Origine::Fenetre, avant);
    degats.ajoute(Origine::Fenetre, apres);

    let region = degats.region();
    assert_eq!(region.x, 100);
    assert_eq!(region.y, 100);
    assert_eq!(region.droite(), 340);
    assert_eq!(region.bas(), 280);
    assert_eq!(degats_plein_ecran(), 0, "un deplacement n'est pas global");
}

/// Un degat vide n'est pas compte : le compter laisserait croire a une
/// activite qui n'a pas eu lieu, ce qui est le pire sens pour une mesure.
#[test]
fn un_rectangle_vide_ne_compte_pas() {
    remise_a_zero();
    let mut degats = Degats::neuf(ecran());
    degats.ajoute(Origine::PleinEcran, Rect::default());
    degats.ajoute(Origine::Fenetre, Rect::neuf(10, 10, 0, 50));
    assert!(degats.vide());
    assert_eq!(degats_plein_ecran(), 0);
}

/// `tout()` est la seule porte vers le plein ecran, et elle se compte.
#[test]
fn tout_est_compte_et_borne_par_l_ecran() {
    remise_a_zero();
    let mut degats = Degats::neuf(ecran());
    degats.tout();
    assert_eq!(degats_plein_ecran(), 1);
    assert_eq!(degats.region(), ecran());

    // Les bornes viennent du parametre, pas d'un global : la politique ne sait
    // pas sur quel materiel elle tourne.
    let mut minuscule = Degats::neuf(Rect::neuf(0, 0, 10, 10));
    minuscule.tout();
    assert_eq!(minuscule.region(), Rect::neuf(0, 0, 10, 10));
    assert_eq!(degats_plein_ecran(), 2);
}

/// `efface` remet la region a zero sans toucher aux compteurs cumulatifs : ce
/// sont deux questions differentes -- « que faut-il copier maintenant » et
/// « qu'a-t-on demande depuis le demarrage ».
#[test]
fn effacer_la_region_ne_remet_pas_les_compteurs_a_zero() {
    remise_a_zero();
    let mut degats = Degats::neuf(ecran());
    degats.tout();
    degats.efface();
    assert!(degats.vide());
    assert_eq!(degats_plein_ecran(), 1);
}

// ---------------------------------------------------------------------------
// Intersection de decoupe
// ---------------------------------------------------------------------------
//
// Reimplemente ici la formule QUE LE PILOTE APPLIQUE, en bornes exclusives.
// `bochs.rs` ne se compile pas sur l'hote (ports, PCI, memoire physique), mais
// l'intersection est de l'arithmetique pure : la garder identique des deux
// cotes est ce que ces tests protegent.
//
// Le defaut corrige : les bornes droites etaient calculees APRES deplacement
// des bornes gauches, ce qui DECALAIT la fenetre au lieu de l'intersecter.

fn intersection(
    a: (usize, usize, usize, usize),
    b: (usize, usize, usize, usize),
) -> (usize, usize, usize, usize) {
    (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3))
}

/// La formule FAUSSE, telle qu'elle etait, pour montrer ce qui changeait.
fn intersection_fautive(
    demande: (usize, usize, usize, usize),
    global: (usize, usize, usize, usize),
) -> (usize, usize, usize, usize) {
    let (x, y, w, h) = demande;
    let x0 = x.max(global.0);
    let y0 = y.max(global.1);
    let x1 = (x0 + w).min(global.2);
    let y1 = (y0 + h).min(global.3);
    (x0, y0, x1, y1)
}

fn vide(r: (usize, usize, usize, usize)) -> bool {
    r.2 <= r.0 || r.3 <= r.1
}

#[test]
fn l_ancienne_formule_decalait_la_fenetre_au_lieu_de_l_intersecter() {
    // Demande [10, 30) x [0, 10), global commencant a 15.
    let corrige = intersection((10, 0, 30, 10), (15, 0, 100, 100));
    assert_eq!(corrige, (15, 0, 30, 10), "la borne droite ne bouge pas");

    let fautif = intersection_fautive((10, 0, 20, 10), (15, 0, 100, 100));
    assert_eq!(fautif, (15, 0, 35, 10), "l'ancienne gagnait 5 px a droite");
    assert!(fautif.2 > corrige.2, "et depassait la zone demandee");
}

#[test]
fn clip_gauche() {
    let r = intersection((0, 0, 100, 100), (40, 0, 200, 200));
    assert_eq!(r, (40, 0, 100, 100));
}

#[test]
fn clip_droite() {
    let r = intersection((0, 0, 100, 100), (0, 0, 60, 200));
    assert_eq!(r, (0, 0, 60, 100));
}

#[test]
fn clip_haut() {
    let r = intersection((0, 0, 100, 100), (0, 30, 200, 200));
    assert_eq!(r, (0, 30, 100, 100));
}

#[test]
fn clip_bas() {
    let r = intersection((0, 0, 100, 100), (0, 0, 200, 70));
    assert_eq!(r, (0, 0, 100, 70));
}

#[test]
fn clip_des_deux_cotes() {
    let r = intersection((0, 0, 100, 100), (20, 30, 60, 70));
    assert_eq!(r, (20, 30, 60, 70));
    assert!(!vide(r));
}

#[test]
fn clip_entierement_exterieur() {
    // Disjoint a droite, a gauche, en bas, en haut : chaque fois vide.
    assert!(vide(intersection((0, 0, 10, 10), (50, 0, 60, 10))));
    assert!(vide(intersection((50, 0, 60, 10), (0, 0, 10, 10))));
    assert!(vide(intersection((0, 0, 10, 10), (0, 50, 10, 60))));
    assert!(vide(intersection((0, 50, 10, 60), (0, 0, 10, 10))));
}

#[test]
fn une_image_partiellement_hors_ecran_est_rognee_des_deux_cotes() {
    // Image posee a cheval sur le bord droit et le bord bas.
    let ecran = (0usize, 0usize, 1280usize, 720usize);
    let image = (1200usize, 700usize, 1300usize, 800usize);
    let r = intersection(image, ecran);
    assert_eq!(r, (1200, 700, 1280, 720));
    assert!(r.2 <= ecran.2 && r.3 <= ecran.3, "jamais hors ecran");
}

#[test]
fn des_dimensions_extremes_ne_debordent_pas() {
    // C'est `saturating_add` qui tient cette propriete cote pilote : une
    // largeur absurde ne doit pas faire boucler l'addition.
    let x1 = usize::MAX.saturating_add(usize::MAX);
    assert_eq!(x1, usize::MAX);
    let r = intersection((0, 0, x1, x1), (0, 0, 1280, 720));
    assert_eq!(r, (0, 0, 1280, 720));
}

#[test]
fn aucune_ecriture_hors_du_rectangle_original() {
    // Propriete generale, verifiee sur une grille de cas : l'intersection est
    // toujours contenue dans CHACUN des deux rectangles.
    let global = (17usize, 23usize, 91usize, 88usize);
    for x in [0usize, 10, 17, 50, 90, 95] {
        for y in [0usize, 5, 23, 60, 87, 100] {
            for w in [0usize, 1, 13, 200] {
                let demande = (x, y, x.saturating_add(w), y.saturating_add(w));
                let r = intersection(demande, global);
                if vide(r) {
                    continue;
                }
                assert!(r.0 >= demande.0 && r.2 <= demande.2, "hors demande {:?}", r);
                assert!(r.1 >= demande.1 && r.3 <= demande.3, "hors demande {:?}", r);
                assert!(r.0 >= global.0 && r.2 <= global.2, "hors global {:?}", r);
                assert!(r.1 >= global.1 && r.3 <= global.3, "hors global {:?}", r);
            }
        }
    }
}
